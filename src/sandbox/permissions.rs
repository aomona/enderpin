//! Shared requests are proposals. Only a local approval of the complete plan authorizes launch.
use std::{collections::BTreeMap, fs::File, io::Read, path::PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::{Side, Workspace, model::hash_bytes, storage};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub game_write: bool,
    pub read_only: Vec<GameDirectory>,
    pub network: bool,
    pub audio: bool,
    /// None means the backend's documented desktop behavior, not an enforced denial.
    pub microphone: Option<bool>,
    pub clipboard: Option<bool>,
    pub desktop_integration: bool,
    pub skin_cache: bool,
    pub graphics_cache: bool,
    pub narrator: bool,
    pub account_authentication: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            game_write: true,
            read_only: vec![],
            network: true,
            audio: true,
            microphone: None,
            clipboard: None,
            desktop_integration: true,
            skin_cache: true,
            graphics_cache: true,
            narrator: false,
            account_authentication: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GameDirectory {
    Saves,
    Screenshots,
    Resourcepacks,
    Shaderpacks,
    Mods,
    Config,
    Logs,
    Plugins,
    World,
}
impl GameDirectory {
    pub fn name(self) -> &'static str {
        match self {
            Self::Saves => "saves",
            Self::Screenshots => "screenshots",
            Self::Resourcepacks => "resourcepacks",
            Self::Shaderpacks => "shaderpacks",
            Self::Mods => "mods",
            Self::Config => "config",
            Self::Logs => "logs",
            Self::Plugins => "plugins",
            Self::World => "world",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    Narrator,
    AccountAuthentication,
    Network,
    Audio,
    Microphone,
    Clipboard,
    DesktopIntegration,
    SkinCache,
    GraphicsCache,
    GameWrite,
    DirectoryWrite,
    FolderRead,
    FolderWrite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub permission: Capability,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<GameDirectory>,
    /// A logical slot; shared metadata may never choose an absolute host path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
}
impl Request {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.reason.trim().is_empty()
                && self.reason.len() <= 2048
                && !self.reason.chars().any(char::is_control),
            "permission reason must be nonempty plain text (up to 2048 bytes)"
        );
        ensure!(
            self.directory.is_some() == (self.permission == Capability::DirectoryWrite),
            "directory is required only for directory-write"
        );
        ensure!(
            self.folder.is_some()
                == matches!(
                    self.permission,
                    Capability::FolderRead | Capability::FolderWrite
                ),
            "folder is required only for folder-read/folder-write"
        );
        if let Some(folder) = &self.folder {
            crate::model::identifier(folder)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Declaration {
    format: u32,
    requests: Vec<Request>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageRequests {
    pub key: String,
    pub sha512: String,
    pub embedded: Vec<Request>,
    pub repository: Vec<Request>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FolderGrant {
    pub path: PathBuf,
    pub write: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub workspace: PathBuf,
    pub side: Side,
    pub platform: String,
    pub baseline: Settings,
    pub minecraft: String,
    pub loader: String,
    pub loader_version: Option<String>,
    pub runtime_sha512: Option<String>,
    pub effective: Settings,
    pub packages: Vec<PackageRequests>,
    pub folders: BTreeMap<String, FolderGrant>,
    pub limitations: Vec<String>,
}
impl Plan {
    pub fn fingerprint(&self) -> Result<String> {
        // Version the approval contract independently of the package lock format.
        Ok(hash_bytes(&serde_json::to_vec(&(2, self))?))
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Local {
    pub bindings: BTreeMap<String, PathBuf>,
    pub approved: Option<String>,
}
impl Local {
    fn relative(side: Side) -> String {
        format!(".enderpin/{}/permissions.toml", side.name())
    }
    pub fn load(ws: &Workspace, side: Side) -> Result<Self> {
        storage::read_optional(&ws.root, &Self::relative(side))?
            .map(|text| toml::from_str(&text).context("invalid local permissions"))
            .transpose()
            .map(Option::unwrap_or_default)
    }
    pub fn save(&self, ws: &Workspace, side: Side) -> Result<()> {
        let _target = storage::target_lock(&ws.root, side)?;
        let dir = storage::directory(&ws.root, &format!(".enderpin/{}", side.name()))?;
        let path = storage::safe_path(&ws.root, &Self::relative(side))?;
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        use std::io::Write;
        file.write_all(toml::to_string_pretty(self)?.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|error| error.error)?;
        Ok(())
    }
}

pub fn plan(ws: &Workspace, side: Side) -> Result<Plan> {
    let _target = storage::target_lock(&ws.root, side)?;
    plan_locked(ws, side)
}

pub(crate) fn plan_locked(ws: &Workspace, side: Side) -> Result<Plan> {
    let target = ws
        .lockfile
        .targets
        .get(&side)
        .context("target is not locked; run sync first")?;
    ensure!(
        target.fingerprint == ws.manifest.fingerprint(side)?,
        "package lock is stale; run sync first"
    );
    let ignores = storage::read_optional(&ws.root, ".enderpinignore")?.unwrap_or_default();
    let packages = target.effective(&crate::model::parse_ignores(&ignores, side)?)?;
    let config = ws.manifest.target(side);
    let mut plan = Plan {
        workspace: ws.root.clone(),
        side,
        platform: std::env::consts::OS.into(),
        minecraft: ws.manifest.minecraft.clone(),
        loader: config.loader.clone(),
        loader_version: config.loader_version.clone(),
        runtime_sha512: ws
            .lockfile
            .runtimes
            .get(&side)
            .map(|runtime| serde_json::to_vec(runtime).map(|bytes| hash_bytes(&bytes)))
            .transpose()?,
        baseline: config.sandbox.clone(),
        effective: config.sandbox.clone(),
        packages: vec![],
        folders: BTreeMap::new(),
        limitations: vec![
            "Grants apply to the entire Minecraft process, including every loaded mod.".into(),
        ],
    };
    let local = Local::load(ws, side)?;
    for key in config.permissions.keys() {
        ensure!(
            target.packages.iter().any(|p| p.matches(key)),
            "permission request references unknown package {key}"
        );
    }
    for package in packages {
        let path = storage::safe_path(&ws.root, &package.relative_path(side))?;
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("{} is not installed; run sync --locked", package.name))?;
        ensure!(
            metadata.is_file() && !storage::is_link(&metadata) && metadata.len() == package.size,
            "{} must be the regular file recorded in the lock",
            package.name
        );
        // Read and hash the same descriptor before inspecting untrusted ZIP metadata.
        let mut file = File::open(&path)
            .with_context(|| format!("{} is not installed; run sync --locked", package.name))?;
        ensure!(
            storage::hash_reader(&mut file)? == (package.sha512.clone(), package.size),
            "{} differs from the lock; refusing permission inspection",
            package.name
        );
        use std::io::Seek;
        file.rewind()?;
        let mut archive =
            zip::ZipArchive::new(&mut file).context("package is not a readable ZIP/JAR")?;
        let embedded = if archive
            .index_for_name("enderpin.permissions.json")
            .is_some()
        {
            let entry = archive.by_name("enderpin.permissions.json")?;
            ensure!(
                entry.size() <= 65536,
                "permission declaration exceeds 64 KiB"
            );
            let mut bytes = Vec::new();
            entry.take(65537).read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() <= 65536,
                "permission declaration exceeds 64 KiB"
            );
            let declaration: Declaration =
                serde_json::from_slice(&bytes).context("invalid enderpin.permissions.json")?;
            ensure!(
                declaration.format == 1,
                "unsupported permission declaration format"
            );
            declaration.requests
        } else {
            vec![]
        };
        let repository: Vec<Request> = config
            .permissions
            .iter()
            .filter(|(key, _)| package.matches(key))
            .flat_map(|(_, requests)| requests.clone())
            .collect();
        ensure!(
            embedded.len() + repository.len() <= 128,
            "too many permission requests for {}",
            package.name
        );
        for request in embedded.iter().chain(&repository) {
            request.validate()?;
            if side == Side::Server {
                ensure!(
                    !matches!(
                        request.permission,
                        Capability::Audio
                            | Capability::Microphone
                            | Capability::Clipboard
                            | Capability::DesktopIntegration
                            | Capability::SkinCache
                            | Capability::GraphicsCache
                            | Capability::Narrator
                            | Capability::AccountAuthentication
                    ),
                    "{} requests desktop permissions for a headless server",
                    package.name
                );
            }
            match request.permission {
                Capability::Narrator => plan.effective.narrator = true,
                Capability::AccountAuthentication => plan.effective.account_authentication = true,
                Capability::Network => plan.effective.network = true,
                Capability::Audio => plan.effective.audio = true,
                Capability::Microphone => {
                    plan.effective.microphone = Some(true);
                    plan.effective.audio = true;
                }
                Capability::Clipboard => plan.effective.clipboard = Some(true),
                Capability::DesktopIntegration => plan.effective.desktop_integration = true,
                Capability::SkinCache => plan.effective.skin_cache = true,
                Capability::GraphicsCache => plan.effective.graphics_cache = true,
                Capability::GameWrite => plan.effective.game_write = true,
                Capability::DirectoryWrite => {
                    plan.effective
                        .read_only
                        .retain(|dir| Some(*dir) != request.directory);
                }
                Capability::FolderRead | Capability::FolderWrite => {
                    let slot = request.folder.as_ref().context("folder slot missing")?;
                    let path = local.bindings.get(slot).with_context(|| {
                        format!("{slot} needs a local folder; use permissions bind {slot} <path>")
                    })?;
                    let path = path.canonicalize().context("bound folder is unavailable")?;
                    let private = crate::auth::state_root()?;
                    ensure!(
                        !path.starts_with(&private) && !private.starts_with(&path),
                        "folder must not overlap host authentication state"
                    );
                    ensure!(
                        !ws.root.starts_with(&path) && !path.starts_with(&ws.root),
                        "external folder must not overlap the workspace"
                    );
                    ensure!(
                        path.parent().is_some() && path.is_dir(),
                        "external binding must be a directory below a filesystem root"
                    );
                    super::validate_data_tree(&path)?;
                    let entry = plan
                        .folders
                        .entry(slot.clone())
                        .or_insert(FolderGrant { path, write: false });
                    entry.write |= request.permission == Capability::FolderWrite;
                }
            }
        }
        plan.packages.push(PackageRequests {
            key: package.key,
            sha512: package.sha512,
            embedded,
            repository,
        });
    }
    ensure!(
        plan.effective.game_write
            || !plan
                .packages
                .iter()
                .flat_map(|p| p.embedded.iter().chain(&p.repository))
                .any(|r| r.permission == Capability::DirectoryWrite),
        "directory-write requires game_write; review the baseline or request game-write explicitly"
    );
    plan.limitations.extend(
        plan.effective
            .validate_for(std::env::consts::OS, side == Side::Client)?,
    );
    Ok(plan)
}

impl Settings {
    pub fn validate_for(&self, os: &str, desktop: bool) -> Result<Vec<String>> {
        let mut notes = vec![];
        ensure!(
            matches!(os, "macos" | "linux" | "windows"),
            "unsupported sandbox OS"
        );
        if desktop {
            match os {
                "macos" => {
                    ensure!(
                        self.microphone != Some(true) || self.audio,
                        "microphone requires audio"
                    );
                    if self.microphone == Some(true) {
                        notes.push(
                            "Microphone access additionally requires macOS privacy consent.".into(),
                        );
                    }
                    if self.graphics_cache {
                        notes.push("The macOS Java Metal cache is shared with other Java apps for this OS user.".into());
                    }
                }
                "linux" => {
                    ensure!(
                        self.desktop_integration,
                        "Linux cannot independently disable desktop integration"
                    );
                    ensure!(
                        self.clipboard != Some(false),
                        "Linux display access cannot independently deny clipboard access"
                    );
                    ensure!(
                        self.microphone != Some(false) || !self.audio,
                        "PulseAudio cannot independently deny recording while audio is enabled"
                    );
                    ensure!(
                        self.microphone != Some(true) || self.audio,
                        "microphone requires audio"
                    );
                    notes.push("Linux grants the selected display protocol; X11 can expose other clients, and Wayland access is not protocol-filtered.".into());
                    if self.audio {
                        notes.push("PulseAudio grants the full service, including recording; output-only access is not enforced.".into());
                    }
                }
                "windows" => {
                    ensure!(
                        self.audio
                            && self.desktop_integration
                            && self.graphics_cache
                            && self.microphone.is_none()
                            && self.clipboard.is_none(),
                        "AppContainer cannot independently enforce audio, microphone, clipboard, desktop-integration or graphics-cache switches; leave unsupported controls at their defaults"
                    );
                    notes.push("AppContainer desktop/audio/clipboard and private profile storage are OS-controlled; independent denials are unavailable.".into());
                }
                _ => unreachable!(),
            }
        }
        Ok(notes)
    }
}
