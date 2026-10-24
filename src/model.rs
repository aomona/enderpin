use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};

pub const FORMAT: u32 = 3;

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
    pub fn game_directory(self) -> &'static str {
        match self {
            Self::Client => "run/client",
            Self::Server => "run/server",
        }
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub loader: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader_version: Option<String>,
    #[serde(default)]
    pub packages: BTreeMap<String, Package>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled: Vec<String>,
    #[serde(default)]
    pub permissions: crate::sandbox::permissions::Settings,
    /// Extra JVM options placed after Enderpin's own flags. Part of the permission plan.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub jvm_args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub minecraft: String,
    #[serde(default)]
    pub common: BTreeMap<String, Package>,
    pub client: Target,
    pub server: Target,
}

impl Manifest {
    pub fn new(minecraft: String) -> Result<Self> {
        let target = Target {
            loader: "fabric".into(),
            loader_version: None,
            packages: BTreeMap::new(),
            enabled: vec![],
            disabled: vec![],
            permissions: Default::default(),
            jvm_args: vec![],
        };
        let manifest = Self {
            format: FORMAT,
            minecraft,
            common: BTreeMap::new(),
            client: target.clone(),
            server: target,
        };
        manifest.validate()?;
        Ok(manifest)
    }
    /// Keep shared configuration small; omitted permission keys use Settings::default().
    pub fn to_toml(&self) -> Result<String> {
        let mut value = toml::Value::try_from(self)?;
        let defaults = toml::Value::try_from(crate::sandbox::permissions::Settings::default())?;
        for side in Side::ALL {
            let target = value
                .get_mut(side.name())
                .and_then(toml::Value::as_table_mut)
                .context("missing target settings")?;
            let permissions = target
                .get_mut("permissions")
                .and_then(toml::Value::as_table_mut)
                .context("missing permission settings")?;
            omit_default_permissions(permissions, &defaults);
            if permissions.is_empty() {
                target.remove("permissions");
            }
        }
        Ok(toml::to_string_pretty(&value)?)
    }
    pub fn parse(text: &str) -> Result<Self> {
        let result: Self = toml::from_str(text).context("invalid enderpin.toml")?;
        result.validate()?;
        Ok(result)
    }
    pub fn target(&self, side: Side) -> &Target {
        match side {
            Side::Client => &self.client,
            Side::Server => &self.server,
        }
    }
    pub fn target_mut(&mut self, side: Side) -> &mut Target {
        match side {
            Side::Client => &mut self.client,
            Side::Server => &mut self.server,
        }
    }
    pub fn requests(&self, side: Side) -> BTreeMap<String, Package> {
        let mut result = self.common.clone();
        result.extend(self.target(side).packages.clone());
        result
    }
    pub fn fingerprint(&self, side: Side) -> Result<String> {
        Ok(hash_bytes(&serde_json::to_vec(&(
            FORMAT,
            &self.minecraft,
            side,
            &self.target(side).loader,
            self.requests(side),
            &self.target(side).enabled,
            &self.target(side).disabled,
        ))?))
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.format == FORMAT,
            "unsupported manifest format {}; expected format 3 (see README manual migration)",
            self.format
        );
        identifier(&self.minecraft)?;
        for side in Side::ALL {
            let target = self.target(side);
            if let Some(version) = &target.loader_version {
                identifier(version)?;
            }
            ensure!(
                ["vanilla", "fabric", "paper", "velocity", "neoforge"]
                    .contains(&target.loader.as_str()),
                "unsupported loader {}",
                target.loader
            );
            ensure!(
                side != Side::Client || !matches!(target.loader.as_str(), "paper" | "velocity"),
                "Paper and Velocity are server runtimes"
            );
            if target.loader == "vanilla" {
                ensure!(
                    target.loader_version.is_none(),
                    "vanilla has no loader version"
                );
                ensure!(
                    self.requests(side)
                        .values()
                        .all(|p| !matches!(p.kind, Kind::Mod | Kind::Plugin)),
                    "vanilla cannot load mods or plugins"
                );
            }
            for (name, package) in self.requests(side) {
                identifier(&name)?;
                package
                    .validate()
                    .with_context(|| format!("invalid package {name}"))?;
            }
            for id in target.enabled.iter().chain(&target.disabled) {
                identifier(id)?;
            }
            ensure!(
                target
                    .enabled
                    .iter()
                    .all(|id| !target.disabled.contains(id)),
                "a package cannot be both enabled and disabled"
            );
            jvm_arguments(&target.jvm_args)
                .with_context(|| format!("invalid {}.jvm_args", side.name()))?;
        }
        Ok(())
    }
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
    pub fn relative_path(&self, side: Side) -> String {
        format!(
            "{}/{}/{}",
            side.game_directory(),
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
    pub targets: BTreeMap<Side, TargetLock>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub runtimes: BTreeMap<Side, crate::runtime::RuntimeLock>,
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

pub fn parse_ignores(text: &str, side: Side) -> Result<Vec<String>> {
    let mut result = vec![];
    for (index, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        let (scope, id) = line.split_once(':').unwrap_or(("*", line));
        ensure!(
            ["*", "client", "server"].contains(&scope),
            "invalid ignore scope on line {}",
            index + 1
        );
        identifier(id)?;
        if scope == "*" || scope == side.name() {
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
            server.server.loader = loader.into();
            server.validate()?;
            server.client.loader = loader.into();
            assert!(server.validate().is_err());
        }
        let mut old = m.clone();
        old.format = 1;
        assert!(Manifest::parse(&toml::to_string(&old)?).is_err());
        assert_eq!(
            parse_ignores(
                "# mine\nclient:sodium\nserver:example\ncommon-mod",
                Side::Client
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
            "[server]\n",
            "[server]\njvm_args = [\"-XX:+UseZGC\", \"-Dfile.encoding=UTF-8\"]\n",
        );
        let parsed = Manifest::parse(&text)?;
        assert_eq!(
            parsed.server.jvm_args,
            ["-XX:+UseZGC", "-Dfile.encoding=UTF-8"]
        );
        assert!(parsed.client.jvm_args.is_empty());
        assert_eq!(
            Manifest::parse(&parsed.to_toml()?)?.server.jvm_args,
            parsed.server.jvm_args
        );
        // Launch flags are approved through the permission plan; the package lock stays valid.
        assert_eq!(
            parsed.fingerprint(Side::Server)?,
            m.fingerprint(Side::Server)?
        );
        let error = Manifest::parse(&text.replace("-XX:+UseZGC", "-Xmx8G"))
            .expect_err("heap override")
            .to_string();
        assert!(error.contains("server.jvm_args"), "{error}");
        Ok(())
    }
}
