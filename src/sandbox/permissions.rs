//! Game-wide settings and local folder grants. Approval is bound to the complete plan.
use std::{fs::File, path::PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::{Side, Workspace, model::hash_bytes, storage};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub files: FilePermissions,
    pub network: bool,
    pub audio: bool,
    /// None means the backend's documented desktop behavior, not an enforced denial.
    pub microphone: Option<bool>,
    pub clipboard: Clipboard,
    pub desktop_integration: bool,
    pub open_links: bool,
    pub open_folders: bool,
    pub import_files: bool,
    pub skin_cache: bool,
    pub graphics_cache: bool,
    pub narrator: bool,
    pub account_authentication: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            files: FilePermissions::default(),
            network: true,
            audio: true,
            microphone: None,
            clipboard: Clipboard::System,
            desktop_integration: true,
            open_links: true,
            open_folders: true,
            import_files: true,
            skin_cache: true,
            graphics_cache: true,
            narrator: true,
            account_authentication: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FilePermissions {
    pub game: FileAccess,
    pub read_only: Vec<GameDirectory>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileAccess {
    ReadOnly,
    #[default]
    ReadWrite,
}
impl FileAccess {
    pub fn writable(self) -> bool {
        self == Self::ReadWrite
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Clipboard {
    /// Use the backend's documented behavior, not an independently enforced grant.
    #[default]
    System,
    None,
    ReadWrite,
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

#[derive(Debug, Clone, Serialize)]
pub struct PackageIdentity {
    pub key: String,
    pub name: String,
    pub version: Option<String>,
    pub sha512: String,
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
    pub target: String,
    pub platform: String,
    pub minecraft: String,
    pub loader: String,
    pub loader_version: Option<String>,
    pub runtime_sha512: Option<String>,
    pub effective: Settings,
    pub packages: Vec<PackageIdentity>,
    pub shared_files: std::collections::BTreeMap<String, String>,
    pub folders: Vec<FolderGrant>,
    /// Shared launch flags change the JVM itself, so they are approved like settings.
    /// Omitted when empty so plans without them keep their existing fingerprint.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub jvm_args: Vec<String>,
    pub limitations: Vec<String>,
}
impl Plan {
    pub fn fingerprint(&self) -> Result<String> {
        // Version the approval contract independently of the package lock format.
        Ok(hash_bytes(&serde_json::to_vec(&(4, self))?))
    }
}

#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Local {
    pub folders: Vec<FolderGrant>,
    pub approved: Option<String>,
}
impl Local {
    fn relative(side: &str) -> String {
        format!(".enderpin/{}/permissions.toml", side)
    }
    pub fn load(ws: &Workspace, side: &str) -> Result<Self> {
        storage::read_optional(&ws.root, &Self::relative(side))?
            .map(|text| toml::from_str(&text).context("invalid local permissions"))
            .transpose()
            .map(Option::unwrap_or_default)
    }
    pub fn save(&self, ws: &Workspace, side: &str) -> Result<()> {
        let _target = storage::target_lock(&ws.root, side)?;
        let dir = storage::directory(&ws.root, &format!(".enderpin/{}", side))?;
        let path = storage::safe_path(&ws.root, &Self::relative(side))?;
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        use std::io::Write;
        file.write_all(toml::to_string_pretty(self)?.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|error| error.error)?;
        Ok(())
    }
}

pub fn plan(ws: &Workspace, side: &str) -> Result<Plan> {
    let _target = storage::target_lock(&ws.root, side)?;
    plan_locked(ws, side)
}

pub(crate) fn plan_locked(ws: &Workspace, side: &str) -> Result<Plan> {
    let target = ws
        .lockfile
        .targets
        .get(side)
        .context("target is not locked; run sync first")?;
    ensure!(
        target.fingerprint == ws.manifest.fingerprint(side)?,
        "package lock is stale; run sync first"
    );
    let ignores = storage::read_optional(&ws.root, ".enderpinignore")?.unwrap_or_default();
    let packages = target.effective(&crate::model::parse_ignores(&ignores, side)?)?;
    let config = ws.manifest.target(side)?;
    let mut plan = Plan {
        workspace: ws.root.clone(),
        side: config.side(),
        target: side.into(),
        platform: std::env::consts::OS.into(),
        minecraft: config.minecraft.clone(),
        loader: config.loader.clone(),
        loader_version: config.loader_version.clone(),
        runtime_sha512: ws
            .lockfile
            .runtimes
            .get(side)
            .map(|runtime| serde_json::to_vec(runtime).map(|bytes| hash_bytes(&bytes)))
            .transpose()?,
        effective: config.permissions.clone(),
        packages: vec![],
        shared_files: ws.shared_file_hashes(side)?,
        folders: Local::load(ws, side)?.folders,
        jvm_args: config.jvm_args.clone(),
        limitations: vec![
            "Grants apply to the entire Minecraft process, including every loaded mod.".into(),
        ],
    };
    validate_folders(ws, &plan.folders)?;
    for package in packages {
        let path = storage::safe_path(&ws.root, &package.relative_path(side))?;
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("{} is not installed; run sync --locked", package.name))?;
        ensure!(
            metadata.is_file() && !storage::is_link(&metadata) && metadata.len() == package.size,
            "{} must be the regular file recorded in the lock",
            package.name
        );
        // Verify the installed bytes, even when a mod asks for no new permissions.
        let mut file = File::open(&path)
            .with_context(|| format!("{} is not installed; run sync --locked", package.name))?;
        ensure!(
            storage::hash_reader(&mut file)? == (package.sha512.clone(), package.size),
            "{} differs from the lock; refusing permission inspection",
            package.name
        );
        plan.packages.push(PackageIdentity {
            key: package.key,
            name: package.name,
            version: package.version_number.or(package.version),
            sha512: package.sha512,
        });
    }
    plan.limitations.extend(
        plan.effective
            .validate_for(std::env::consts::OS, config.side() == Side::Client)?,
    );
    Ok(plan)
}

/// Canonical paths keep a later symlink replacement from changing an approved grant.
pub fn validate_folders(ws: &Workspace, folders: &[FolderGrant]) -> Result<()> {
    if folders.is_empty() {
        return Ok(());
    }
    let private = crate::auth::state_root()?;
    for (index, grant) in folders.iter().enumerate() {
        let path = &grant.path;
        ensure!(
            path.is_absolute() && path.is_dir() && path.canonicalize()? == *path,
            "external folder must be an existing canonical directory"
        );
        ensure!(
            path.parent().is_some() && !path.starts_with(&private) && !private.starts_with(path),
            "folder must not overlap host authentication state or a filesystem root"
        );
        ensure!(
            !ws.root.starts_with(path) && !path.starts_with(&ws.root),
            "external folder must not overlap the workspace"
        );
        ensure!(
            !folders[..index]
                .iter()
                .any(|other| path.starts_with(&other.path) || other.path.starts_with(path)),
            "external folders must not overlap each other"
        );
        super::validate_data_tree(path)?;
    }
    Ok(())
}

impl Settings {
    /// Vanilla clipboard access is enabled unless explicitly denied.
    pub fn clipboard_allowed(&self) -> bool {
        self.clipboard != Clipboard::None
    }

    /// Explicit editor action; never relax a saved denial merely by opening or saving it.
    pub fn reset_unsupported_desktop(&mut self, os: &str) {
        match os {
            "windows" => {
                self.audio = true;
                self.desktop_integration = true;
                self.graphics_cache = true;
                self.microphone = None;
                self.clipboard = Clipboard::System;
            }
            "linux" => {
                self.desktop_integration = true;
                if self.clipboard == Clipboard::None {
                    self.clipboard = Clipboard::System;
                }
                if self.microphone == Some(!self.audio) {
                    self.microphone = None;
                }
            }
            "macos" if !self.audio && self.microphone == Some(true) => {
                self.microphone = None;
            }
            _ => {}
        }
    }

    pub fn validate_for(&self, os: &str, desktop: bool) -> Result<Vec<String>> {
        let mut notes = vec![];
        ensure!(
            matches!(os, "macos" | "linux" | "windows"),
            "unsupported sandbox OS"
        );
        if desktop {
            if self.account_authentication {
                notes.push("Account access includes vanilla multiplayer, Realms, player reports, friends and presence. Realms and social requests also require game network access.".into());
            }
            if self.import_files {
                notes.push("File imports require a host file selection or confirmation. Selected data is copied read-only into the game's import cache (up to 2 GiB per request).".into());
            }
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
                        self.clipboard != Clipboard::None,
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
                        notes.push("Audio grants the selected local PulseAudio/PipeWire service or ALSA devices, including recording; output-only access is not enforced.".into());
                    }
                }
                "windows" => {
                    ensure!(
                        self.audio
                            && self.desktop_integration
                            && self.graphics_cache
                            && self.microphone.is_none()
                            && self.clipboard == Clipboard::System,
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
