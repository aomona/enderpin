use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};

pub const FORMAT: u32 = 4;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Client,
    Server,
}

impl Side {
    pub const ALL: [Self; 2] = [Self::Client, Self::Server];
    pub fn name(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Server => "server",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Mod,
    Resourcepack,
    Shader,
    Plugin,
}

impl Kind {
    pub fn directory(self) -> &'static str {
        match self {
            Self::Mod => "mods",
            Self::Resourcepack => "resourcepacks",
            Self::Shader => "shaderpacks",
            Self::Plugin => "plugins",
        }
    }
    pub fn api_type(self) -> &'static str {
        match self {
            Self::Mod | Self::Plugin => "mod",
            Self::Resourcepack => "resourcepack",
            Self::Shader => "shader",
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mod | Self::Plugin => "jar",
            _ => "zip",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modrinth: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub kind: Kind,
    /// Exact Modrinth version ID, not a semver expression.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub optional: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

impl Package {
    pub fn modrinth(id: impl Into<String>, kind: Kind) -> Self {
        Self {
            modrinth: Some(id.into()),
            url: None,
            kind,
            version: None,
            prerelease: false,
            optional: vec![],
            filename: None,
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.modrinth.is_some() != self.url.is_some(),
            "specify exactly one of modrinth or url"
        );
        if let Some(id) = &self.modrinth {
            identifier(id)?;
        }
        if let Some(id) = &self.version {
            identifier(id)?;
        }
        if let Some(url) = &self.url {
            https_url(url)?;
            ensure!(
                self.version.is_none() && self.optional.is_empty() && !self.prerelease,
                "URL packages do not support Modrinth version/optional/prerelease settings"
            );
        }
        if let Some(name) = &self.filename {
            filename(name, self.kind)?;
        }
        for id in &self.optional {
            identifier(id)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum TargetType {
    Client,
    Server,
    Proxy,
}
impl TargetType {
    pub fn side(self) -> Side {
        if self == Self::Client {
            Side::Client
        } else {
            Side::Server
        }
    }
}

/// Only Enderpin settings belong here; native configuration stays in files/.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Target {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<TargetType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minecraft: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub loader: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loader_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_mib: Option<u32>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub packages: BTreeMap<String, Package>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_groups: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<crate::sandbox::permissions::Settings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jvm_args: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(flatten)]
    pub settings: Target,
}
impl std::ops::Deref for TargetEntry {
    type Target = Target;
    fn deref(&self) -> &Target {
        &self.settings
    }
}
impl std::ops::DerefMut for TargetEntry {
    fn deref_mut(&mut self) -> &mut Target {
        &mut self.settings
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Group {
    pub packages: BTreeMap<String, Package>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub minecraft: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub groups: BTreeMap<String, Group>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, Target>,
    pub targets: BTreeMap<String, TargetEntry>,
}

/// Fully resolved, target-local input to the existing dependency/runtime resolvers.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedTarget {
    pub kind: TargetType,
    pub minecraft: String,
    pub loader: String,
    pub loader_version: Option<String>,
    pub memory_mib: u32,
    pub packages: BTreeMap<String, Package>,
    pub requests: BTreeMap<String, Package>,
    pub file_groups: Vec<String>,
    pub enabled: Vec<String>,
    pub disabled: Vec<String>,
    pub permissions: crate::sandbox::permissions::Settings,
    pub jvm_args: Vec<String>,
}
impl ResolvedTarget {
    pub fn side(&self) -> Side {
        self.kind.side()
    }
    pub fn fingerprint(&self) -> Result<String> {
        Ok(hash_bytes(&serde_json::to_vec(&(
            FORMAT,
            self.kind,
            &self.minecraft,
            &self.loader,
            &self.requests,
            &self.packages,
            &self.enabled,
            &self.disabled,
        ))?))
    }
}

impl Manifest {
    pub fn new(minecraft: String) -> Result<Self> {
        let mut targets = BTreeMap::new();
        for (name, kind) in [
            ("client", TargetType::Client),
            ("server", TargetType::Server),
        ] {
            targets.insert(
                name.into(),
                TargetEntry {
                    profile: None,
                    settings: Target {
                        kind: Some(kind),
                        loader: "fabric".into(),
                        ..Default::default()
                    },
                },
            );
        }
        let manifest = Self {
            format: FORMAT,
            minecraft,
            groups: BTreeMap::new(),
            profiles: BTreeMap::new(),
            targets,
        };
        manifest.validate()?;
        Ok(manifest)
    }
    pub fn to_toml(&self) -> Result<String> {
        let mut value = toml::Value::try_from(self)?;
        let defaults = toml::Value::try_from(crate::sandbox::permissions::Settings::default())?;
        if let Some(targets) = value.get_mut("targets").and_then(toml::Value::as_table_mut) {
            for (_, target) in targets.iter_mut() {
                if let Some(permissions) = target
                    .get_mut("permissions")
                    .and_then(toml::Value::as_table_mut)
                {
                    omit_default_permissions(permissions, &defaults);
                    if permissions.is_empty() {
                        target
                            .as_table_mut()
                            .context("invalid target")?
                            .remove("permissions");
                    }
                }
            }
        }
        Ok(toml::to_string_pretty(&value)?)
    }
    pub fn parse(text: &str) -> Result<Self> {
        let value: toml::Value = toml::from_str(text).context("invalid enderpin.toml")?;
        ensure!(
            value.get("format").and_then(toml::Value::as_integer) == Some(i64::from(FORMAT)),
            "expected manifest format 4; back up worlds and follow docs/migration.md (no automatic directory migration)"
        );
        let result: Self = value.try_into().context("invalid enderpin.toml")?;
        result.validate()?;
        Ok(result)
    }
    pub fn target_mut(&mut self, name: &str) -> Result<&mut TargetEntry> {
        self.targets
            .get_mut(name)
            .with_context(|| format!("unknown target {name:?}"))
    }
    pub fn target(&self, name: &str) -> Result<ResolvedTarget> {
        target_name(name)?;
        let entry = self
            .targets
            .get(name)
            .with_context(|| format!("unknown target {name:?}"))?;
        let mut merged = if let Some(profile) = &entry.profile {
            toml::Value::try_from(
                self.profiles
                    .get(profile)
                    .with_context(|| format!("unknown profile {profile:?}"))?,
            )?
        } else {
            toml::Value::Table(Default::default())
        };
        let overrides = toml::Value::try_from(&entry.settings)?;
        let table = merged.as_table_mut().context("invalid profile")?;
        for (key, value) in overrides.as_table().context("invalid target")? {
            if key == "packages" {
                let packages = table
                    .entry(key.clone())
                    .or_insert_with(|| toml::Value::Table(Default::default()));
                packages
                    .as_table_mut()
                    .context("invalid packages")?
                    .extend(value.as_table().context("invalid packages")?.clone());
            } else {
                table.insert(key.clone(), value.clone());
            }
        }
        let config: Target = merged.try_into()?;
        let kind = config
            .kind
            .context("target requires type = client, server or proxy")?;
        let minecraft = config.minecraft.unwrap_or_else(|| self.minecraft.clone());
        identifier(&minecraft)?;
        ensure!(
            ["vanilla", "fabric", "paper", "velocity", "neoforge"]
                .contains(&config.loader.as_str()),
            "unsupported loader {}",
            config.loader
        );
        ensure!(
            (kind == TargetType::Proxy) == (config.loader == "velocity"),
            "Velocity requires type = proxy; proxies require loader = velocity"
        );
        ensure!(
            kind != TargetType::Client || config.loader != "paper",
            "Paper is a server runtime"
        );
        if let Some(version) = &config.loader_version {
            identifier(version)?;
        }
        if config.loader == "vanilla" {
            ensure!(
                config.loader_version.is_none(),
                "vanilla has no loader version"
            );
        }
        let mut requests = BTreeMap::new();
        for group in config.groups.unwrap_or_default() {
            let group = self
                .groups
                .get(&group)
                .with_context(|| format!("unknown package group {group:?}"))?;
            for (alias, package) in &group.packages {
                if let Some(previous) = requests.insert(alias.clone(), package.clone()) {
                    ensure!(
                        previous == *package,
                        "conflicting group package {alias}; use distinct groups or one shared declaration"
                    );
                }
            }
        }
        requests.extend(config.packages.clone());
        if config.loader == "vanilla" {
            ensure!(
                requests
                    .values()
                    .all(|p| !matches!(p.kind, Kind::Mod | Kind::Plugin)),
                "vanilla cannot load mods or plugins"
            );
        }
        let enabled = config.enabled.unwrap_or_default();
        let disabled = config.disabled.unwrap_or_default();
        for id in enabled.iter().chain(&disabled) {
            identifier(id)?;
        }
        ensure!(
            enabled.iter().all(|id| !disabled.contains(id)),
            "a package cannot be both enabled and disabled"
        );
        for (name, package) in &requests {
            identifier(name)?;
            package.validate()?;
        }
        let file_groups = config.file_groups.unwrap_or_default();
        for group in &file_groups {
            target_name(group)?;
        }
        let memory_mib = config.memory_mib.unwrap_or(2048);
        ensure!(
            (256..=1_048_576).contains(&memory_mib),
            "memory_mib must be between 256 and 1048576"
        );
        let jvm_args = config.jvm_args.unwrap_or_default();
        jvm_arguments(&jvm_args)?;
        Ok(ResolvedTarget {
            kind,
            minecraft,
            loader: config.loader,
            loader_version: config.loader_version,
            memory_mib,
            packages: config.packages,
            requests,
            file_groups,
            enabled,
            disabled,
            permissions: config.permissions.unwrap_or_default(),
            jvm_args,
        })
    }
    pub fn requests(&self, name: &str) -> Result<BTreeMap<String, Package>> {
        Ok(self.target(name)?.requests)
    }
    pub fn fingerprint(&self, name: &str) -> Result<String> {
        self.target(name)?.fingerprint()
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.format == FORMAT,
            "unsupported manifest format; expected format 4"
        );
        identifier(&self.minecraft)?;
        ensure!(!self.targets.is_empty(), "at least one target is required");
        for (name, profile) in &self.profiles {
            target_name(name)?;
            ensure!(
                profile.permissions.is_none(),
                "permissions must be declared on targets, not profiles"
            );
        }
        for (name, group) in &self.groups {
            target_name(name)?;
            for (alias, package) in &group.packages {
                identifier(alias)?;
                package.validate()?;
            }
        }
        for name in self.targets.keys() {
            ensure!(
                !["shared", "run", "files", "enderpin", "target"].contains(&name.as_str()),
                "reserved target name: {name}"
            );
            self.target(name)
                .with_context(|| format!("invalid target {name}"))?;
        }
        Ok(())
    }
}

pub fn target_name(name: &str) -> Result<()> {
    safe_filename(name)?;
    ensure!(
        !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'),
        "target/group/profile names use lowercase ASCII letters, digits, '-' or '_': {name:?}"
    );
    ensure!(
        !["all"].contains(&name),
        "reserved target/group/profile name: {name}"
    );
    Ok(())
}

pub fn game_directory(name: &str) -> String {
    format!("{name}/run")
}

fn omit_default_permissions(
    table: &mut toml::map::Map<String, toml::Value>,
    defaults: &toml::Value,
) {
    table.retain(|key, value| {
        if let Some(nested) = value.as_table_mut()
            && let Some(default) = defaults.get(key)
        {
            omit_default_permissions(nested, default);
            !nested.is_empty()
        } else {
            defaults.get(key) != Some(value)
        }
    });
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    pub project: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedPackage {
    pub key: String,
    pub name: String,
    pub kind: Kind,
    pub project: Option<String>,
    pub version: Option<String>,
    pub version_number: Option<String>,
    pub url: String,
    pub filename: String,
    pub sha512: String,
    pub size: u64,
    pub default_sides: Vec<Side>,
    #[serde(default)]
    pub roots: Vec<String>,
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub incompatible: Vec<Dependency>,
    #[serde(default)]
    pub optional: Vec<Dependency>,
}

impl LockedPackage {
    pub fn matches(&self, id: &str) -> bool {
        self.key == id
            || self.name == id
            || self.project.as_deref() == Some(id)
            || self.roots.iter().any(|root| root == id)
    }
    pub fn relative_path(&self, side: &str) -> String {
        format!(
            "{}/{}/{}",
            game_directory(side),
            self.kind.directory(),
            self.filename
        )
    }
    pub fn validate(&self) -> Result<()> {
        filename(&self.filename, self.kind)?;
        validate_hash(&self.sha512)?;
        https_url(&self.url)?;
        identifier(&self.name)?;
        ensure!(
            self.size <= crate::registry::MAX_FILE_SIZE,
            "file exceeds supported size limit"
        );
        match (&self.project, &self.version) {
            (Some(project), Some(version)) => {
                identifier(project)?;
                identifier(version)?;
                ensure!(
                    self.key == format!("modrinth:{project}"),
                    "invalid Modrinth package key"
                );
            }
            (None, None) => ensure!(
                self.key == format!("url:{}", self.name),
                "invalid URL package key"
            ),
            _ => bail!("incomplete locked package identity"),
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetLock {
    pub fingerprint: String,
    pub minecraft: String,
    pub loader: String,
    pub requests: BTreeMap<String, Package>,
    pub packages: Vec<LockedPackage>,
}

impl TargetLock {
    pub fn validate(&self) -> Result<()> {
        validate_hash(&self.fingerprint)?;
        let mut keys = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for p in &self.packages {
            p.validate()?;
            ensure!(keys.insert(p.key.clone()), "duplicate package {}", p.key);
            ensure!(
                paths.insert(format!(
                    "{}/{}",
                    p.kind.directory(),
                    p.filename.to_lowercase()
                )),
                "filename collision: {}",
                p.filename
            );
        }
        for p in &self.packages {
            for required in &p.required {
                ensure!(
                    keys.contains(required),
                    "{} requires missing {required}",
                    p.name
                );
            }
        }
        Ok(())
    }
    pub fn effective(&self, ignores: &[String]) -> Result<Vec<LockedPackage>> {
        self.validate()?;
        let mut live = BTreeSet::new();
        let by_key: BTreeMap<_, _> = self.packages.iter().map(|p| (p.key.clone(), p)).collect();
        fn visit(
            key: &str,
            by_key: &BTreeMap<String, &LockedPackage>,
            ignores: &[String],
            live: &mut BTreeSet<String>,
            parent: &str,
        ) -> Result<()> {
            let p = by_key.get(key).context("missing dependency in lock")?;
            ensure!(
                !ignores.iter().any(|i| p.matches(i)),
                "{parent} requires {}, which is ignored",
                p.name
            );
            if live.insert(key.to_owned()) {
                for dependency in &p.required {
                    visit(dependency, by_key, ignores, live, &p.name)?;
                }
            }
            Ok(())
        }
        for p in &self.packages {
            if !p.roots.is_empty() && !ignores.iter().any(|i| p.matches(i)) {
                visit(&p.key, &by_key, ignores, &mut live, &p.name)?;
            }
        }
        Ok(self
            .packages
            .iter()
            .filter(|p| live.contains(&p.key))
            .cloned()
            .collect())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lockfile {
    pub format: u32,
    #[serde(default)]
    pub targets: BTreeMap<String, TargetLock>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub runtimes: BTreeMap<String, crate::runtime::RuntimeLock>,
}
impl Default for Lockfile {
    fn default() -> Self {
        Self {
            format: FORMAT,
            targets: BTreeMap::new(),
            runtimes: BTreeMap::new(),
        }
    }
}
impl Lockfile {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.format == FORMAT, "unsupported lock format");
        for name in self.targets.keys().chain(self.runtimes.keys()) {
            target_name(name)?;
        }
        for target in self.targets.values() {
            target.validate()?;
        }
        for runtime in self.runtimes.values() {
            runtime.validate()?;
        }
        Ok(())
    }
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha512::digest(bytes))
}
pub fn validate_hash(hash: &str) -> Result<()> {
    ensure!(
        hash.len() == 128
            && hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid SHA-512"
    );
    Ok(())
}
pub fn identifier(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 200
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b)),
        "invalid identifier: {value:?}"
    );
    ensure!(value != "." && value != "..", "invalid identifier");
    Ok(())
}
pub fn filename(name: &str, kind: Kind) -> Result<()> {
    safe_filename(name)?;
    ensure!(
        name.to_ascii_lowercase()
            .ends_with(&format!(".{}", kind.extension())),
        "{} packages need a .{} file: {name}",
        kind.api_type(),
        kind.extension()
    );
    Ok(())
}
pub fn safe_filename(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty() && name.len() <= 240 && name != "." && name != "..",
        "invalid filename"
    );
    ensure!(
        !name
            .chars()
            .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c)),
        "unsafe filename: {name:?}"
    );
    ensure!(
        !name.ends_with([' ', '.']),
        "non-portable filename: {name:?}"
    );
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    ensure!(
        ![
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"
        ]
        .contains(&stem.as_str()),
        "reserved filename: {name}"
    );
    Ok(())
}
pub fn https_url(raw: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(raw).context("invalid URL")?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "only public HTTPS file URLs without credentials/fragments are supported"
    );
    Ok(url)
}

pub const MAX_JVM_ARGUMENTS: usize = 64;

/// Conservative allowlist for user JVM options: only `-XX:` flags and `-D`
/// properties. Options that change the heap, classpath, main class, agents or
/// security manager, run commands, or read further options from files are rejected.
pub fn jvm_argument(argument: &str) -> Result<()> {
    ensure!(
        !argument.is_empty() && argument.len() <= 1024,
        "JVM argument must be 1-1024 bytes"
    );
    ensure!(
        !argument.chars().any(char::is_control),
        "JVM argument contains control characters: {argument:?}"
    );
    let heap = || {
        anyhow::anyhow!(
            "{argument:?} is not allowed; set the heap with enderpin run --memory <MIB>"
        )
    };
    if argument.starts_with("-Xmx") || argument.starts_with("-Xms") {
        return Err(heap());
    }
    if let Some(option) = argument.strip_prefix("-XX:") {
        let (name, value) = match option.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (
                option.strip_prefix(['+', '-']).with_context(|| {
                    format!("expected -XX:+Name, -XX:-Name or -XX:Name=value: {argument:?}")
                })?,
                None,
            ),
        };
        ensure!(
            !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "invalid -XX option name: {argument:?}"
        );
        ensure!(
            value.is_none_or(|value| !value.is_empty()),
            "missing -XX option value: {argument:?}"
        );
        if matches!(name, "MaxHeapSize" | "InitialHeapSize" | "MinHeapSize") {
            return Err(heap());
        }
        ensure!(
            !matches!(
                name,
                // Run host commands or read further options from files.
                "OnError"
                    | "OnOutOfMemoryError"
                    | "Flags"
                    | "VMOptionsFile"
                    | "CompileCommandFile"
                    // Runtime agent loading and attach.
                    | "EnableDynamicAgentLoading"
                    | "StartAttachListener"
                    // Set by Enderpin.
                    | "UsePerfData"
            ),
            "JVM option {argument:?} is not allowed"
        );
        return Ok(());
    }
    if let Some(property) = argument.strip_prefix("-D") {
        let name = property.split_once('=').map_or(property, |(name, _)| name);
        ensure!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
            "invalid system property name: {argument:?}"
        );
        const RESERVED: [&str; 13] = [
            "java.class.path",
            "java.library.path",
            "java.home",
            "java.system.class.loader",
            "java.security.manager",
            "java.ext.dirs",
            "java.endorsed.dirs",
            "java.io.tmpdir",
            "java.awt.headless",
            "user.home",
            "user.dir",
            "sun.boot.library.path",
            "sun.java.command",
        ];
        const RESERVED_PREFIXES: [&str; 8] = [
            "java.security.",
            "jdk.module.",
            "jdk.attach.",
            "jdk.internal.",
            // Launcher contract: Enderpin, its bridges and Fabric Loader
            // (fabric.addMods would load jars outside the lock).
            "enderpin.",
            "monalauncher.",
            "fabric.",
            "loader.",
        ];
        ensure!(
            !RESERVED.contains(&name)
                && !RESERVED_PREFIXES
                    .iter()
                    .any(|prefix| name.starts_with(prefix)),
            "system property {name:?} is reserved by Enderpin or the sandbox"
        );
        return Ok(());
    }
    bail!(
        "JVM argument {argument:?} is not allowed; only -XX: options and -D system properties are supported (heap, classpath, agents, argument files and main-class options are reserved)"
    )
}

pub fn jvm_arguments(arguments: &[String]) -> Result<()> {
    ensure!(
        arguments.len() <= MAX_JVM_ARGUMENTS,
        "at most {MAX_JVM_ARGUMENTS} JVM arguments are supported"
    );
    for argument in arguments {
        jvm_argument(argument)?;
    }
    Ok(())
}

pub fn parse_ignores(text: &str, side: &str) -> Result<Vec<String>> {
    let mut result = vec![];
    for (index, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        let (scope, id) = line.split_once(':').unwrap_or(("*", line));
        if scope != "*" {
            target_name(scope)
                .with_context(|| format!("invalid ignore scope on line {}", index + 1))?;
        }
        identifier(id)?;
        if scope == "*" || scope == side {
            result.push(id.to_owned());
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_and_manifest_are_strict() -> Result<()> {
        for bad in ["../evil.jar", "C:evil.jar", "CON.jar", "a\\b.jar", "a.jar "] {
            assert!(filename(bad, Kind::Mod).is_err());
        }
        filename("Sodium+mc1.21.jar", Kind::Mod)?;
        let m = Manifest::new("1.21.1".into())?;
        assert_eq!(Manifest::parse(&toml::to_string(&m)?)?.minecraft, "1.21.1");
        assert!(Manifest::parse(&(toml::to_string(&m)? + "\n[unknown]\na = 1\n")).is_err());
        for loader in ["paper", "velocity"] {
            let mut server = m.clone();
            server.target_mut("server")?.loader = loader.into();
            if loader == "velocity" {
                server.target_mut("server")?.kind = Some(TargetType::Proxy);
            }
            server.validate()?;
            server.target_mut("client")?.loader = loader.into();
            assert!(server.validate().is_err());
        }
        let mut old = m.clone();
        old.format = 1;
        assert!(Manifest::parse(&toml::to_string(&old)?).is_err());
        assert_eq!(
            parse_ignores(
                "# mine\nclient:sodium\nserver:example\ncommon-mod",
                "client"
            )?,
            vec!["sodium", "common-mod"]
        );
        Ok(())
    }

    #[test]
    fn jvm_arguments_allow_only_safe_options() -> Result<()> {
        for good in [
            "-XX:+UseZGC",
            "-XX:-UseCompressedOops",
            "-XX:+UseG1GC",
            "-XX:MaxGCPauseMillis=50",
            "-XX:SoftMaxHeapSize=1g",
            "-Dfile.encoding=UTF-8",
            "-Dlog4j2.formatMsgNoLookups=true",
            "-Dpaper.disableChannelLimit",
            "-Dmotd=hello world",
        ] {
            jvm_argument(good).with_context(|| good.to_owned())?;
        }
        for (bad, message) in [
            ("-Xmx4G", "--memory"),
            ("-Xms1G", "--memory"),
            ("-XX:MaxHeapSize=4g", "--memory"),
            ("-XX:InitialHeapSize=1g", "--memory"),
            ("-cp", "only -XX:"),
            ("-classpath", "only -XX:"),
            ("--class-path=/tmp", "only -XX:"),
            ("-jar", "only -XX:"),
            ("--module-path=/tmp", "only -XX:"),
            ("-p", "only -XX:"),
            ("-javaagent:/tmp/a.jar", "only -XX:"),
            ("-agentlib:jdwp=transport=dt_socket", "only -XX:"),
            ("-agentpath:/tmp/a.so", "only -XX:"),
            ("@/tmp/args", "only -XX:"),
            ("-Xbootclasspath/a:/tmp", "only -XX:"),
            ("--add-opens=java.base/java.lang=ALL-UNNAMED", "only -XX:"),
            ("net.minecraft.server.Main", "only -XX:"),
            ("-Djava.security.manager", "reserved"),
            ("-Djava.security.manager=allow", "reserved"),
            ("-Djava.security.policy==/tmp/p", "reserved"),
            ("-Djava.class.path=/tmp", "reserved"),
            ("-Djava.system.class.loader=Evil", "reserved"),
            ("-Duser.home=/", "reserved"),
            ("-Djava.io.tmpdir=/", "reserved"),
            ("-Dfabric.addMods=/tmp/evil.jar", "reserved"),
            ("-Dmonalauncher.narrator.token=x", "reserved"),
            ("-Denderpin.minecraft=1.0", "reserved"),
            ("-Djdk.attach.allowAttachSelf=true", "reserved"),
            ("-XX:OnError=sh", "not allowed"),
            ("-XX:OnOutOfMemoryError=sh", "not allowed"),
            ("-XX:VMOptionsFile=/tmp/x", "not allowed"),
            ("-XX:Flags=/tmp/x", "not allowed"),
            ("-XX:+EnableDynamicAgentLoading", "not allowed"),
            ("-XX:+UsePerfData", "not allowed"),
            ("-XX:UseZGC", "expected"),
            ("-XX:+Use ZGC", "invalid"),
            ("-XX:Max=", "missing"),
            ("-D", "invalid"),
            ("-D=x", "invalid"),
            ("-Dbad name=x", "invalid"),
            ("-Da=b\n-Xmx1G", "control"),
            ("", "1-1024"),
        ] {
            let error = jvm_argument(bad).expect_err(bad).to_string();
            assert!(error.contains(message), "{bad:?}: {error}");
        }
        assert!(jvm_arguments(&vec!["-XX:+UseZGC".into(); MAX_JVM_ARGUMENTS + 1]).is_err());
        Ok(())
    }

    #[test]
    fn manifest_jvm_args_are_optional_and_validated() -> Result<()> {
        let m = Manifest::new("1.21.1".into())?;
        assert!(!m.to_toml()?.contains("jvm_args"));
        let text = m.to_toml()?.replace(
            "[targets.server]\n",
            "[targets.server]\njvm_args = [\"-XX:+UseZGC\", \"-Dfile.encoding=UTF-8\"]\n",
        );
        let parsed = Manifest::parse(&text)?;
        assert_eq!(
            parsed.target("server")?.jvm_args,
            ["-XX:+UseZGC", "-Dfile.encoding=UTF-8"]
        );
        assert!(parsed.target("client")?.jvm_args.is_empty());
        assert_eq!(
            Manifest::parse(&parsed.to_toml()?)?
                .target("server")?
                .jvm_args,
            parsed.target("server")?.jvm_args
        );
        // Launch flags are approved through the permission plan; the package lock stays valid.
        assert_eq!(parsed.fingerprint("server")?, m.fingerprint("server")?);
        let error = Manifest::parse(&text.replace("-XX:+UseZGC", "-Xmx8G"))
            .expect_err("heap override")
            .to_string();
        assert!(error.contains("invalid target server"), "{error}");
        Ok(())
    }
}
