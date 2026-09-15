use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};

pub const FORMAT: u32 = 1;

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
    pub sandbox: crate::sandbox::permissions::Settings,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub permissions: BTreeMap<String, Vec<crate::sandbox::permissions::Request>>,
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
            sandbox: Default::default(),
            permissions: BTreeMap::new(),
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
            "unsupported manifest format {}",
            self.format
        );
        identifier(&self.minecraft)?;
        for side in Side::ALL {
            let target = self.target(side);
            for requests in target.permissions.values() {
                for request in requests {
                    request.validate()?;
                }
            }
            if let Some(version) = &target.loader_version {
                identifier(version)?;
            }
            ensure!(
                ["vanilla", "fabric", "paper", "neoforge"].contains(&target.loader.as_str()),
                "unsupported loader {}",
                target.loader
            );
            ensure!(
                side != Side::Client || target.loader != "paper",
                "Paper is a server runtime"
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
        }
        Ok(())
    }
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
            ".enderpin/{}/game/{}/{}",
            side.name(),
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
        assert_eq!(
            parse_ignores(
                "# mine\nclient:sodium\nserver:example\ncommon-mod",
                Side::Client
            )?,
            vec!["sodium", "common-mod"]
        );
        Ok(())
    }
}
