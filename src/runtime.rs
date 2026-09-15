//! Preparation of pinned Minecraft, Fabric and Temurin runtimes.
mod archive;

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};

use crate::{
    model::{Manifest, Side, hash_bytes, https_url, identifier, safe_filename},
    registry,
    storage::{self, Cache},
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

impl Algorithm {
    fn len(self) -> usize {
        match self {
            Self::Sha1 => 40,
            Self::Sha256 => 64,
            Self::Sha512 => 128,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Sha1 => "sha1",
            Self::Sha256 => "sha256",
            Self::Sha512 => "sha512",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Download {
    pub url: String,
    pub algorithm: Algorithm,
    pub hash: String,
    pub filename: String,
    pub size: Option<u64>,
}

impl Download {
    pub fn validate(&self) -> Result<()> {
        https_url(&self.url)?;
        safe_filename(&self.filename)?;
        ensure!(
            self.hash.len() == self.algorithm.len()
                && self
                    .hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "invalid runtime digest"
        );
        ensure!(
            self.size
                .is_none_or(|s| s > 0 && s <= registry::MAX_FILE_SIZE),
            "runtime download size is invalid"
        );
        Ok(())
    }
    fn verify(&self, path: &Path) -> Result<()> {
        let mut file = File::open(path)?;
        ensure!(
            self.size
                .is_none_or(|s| file.metadata().is_ok_and(|meta| meta.len() == s)),
            "runtime size mismatch: {}",
            self.filename
        );
        fn digest<D: Digest + Default>(file: &mut File) -> Result<String> {
            let mut digest = D::default();
            let mut buffer = [0u8; 65536];
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                digest.update(&buffer[..count]);
            }
            Ok(digest
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect())
        }
        let hash = match self.algorithm {
            Algorithm::Sha1 => digest::<sha1::Sha1>(&mut file)?,
            Algorithm::Sha256 => digest::<Sha256>(&mut file)?,
            Algorithm::Sha512 => digest::<Sha512>(&mut file)?,
        };
        ensure!(
            hash == self.hash,
            "runtime hash mismatch: {}",
            self.filename
        );
        Ok(())
    }
    pub fn acquire(&self, cache: &Cache, offline: bool) -> Result<PathBuf> {
        self.validate()?;
        let cache_root = cache.runtime_root()?;
        let directory = storage::directory(
            &cache_root,
            &format!("{}-{}", self.algorithm.name(), self.hash),
        )?;
        let destination = storage::safe_path(&directory, &self.filename)?;
        if destination.try_exists()? {
            self.verify(&destination)?;
            return Ok(destination);
        }
        ensure!(
            !offline,
            "runtime file is missing from cache (offline): {}",
            self.filename
        );
        let mut temp = tempfile::NamedTempFile::new_in(&directory)?;
        registry::download(&self.url, temp.as_file_mut())?;
        self.verify(temp.path())?;
        temp.as_file().sync_all()?;
        match temp.persist_noclobber(&destination) {
            Ok(_) => (),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                self.verify(&destination)?
            }
            Err(error) => return Err(error.error.into()),
        }
        Ok(destination)
    }

    fn copy_verified(&self, source: &Path, destination: &Path) -> Result<()> {
        if destination.try_exists()? {
            return self.verify(destination);
        }
        let parent = destination.parent().context("asset has no parent")?;
        let staged = tempfile::NamedTempFile::new_in(parent)?;
        fs::copy(source, staged.path())?;
        self.verify(staged.path())?;
        staged.as_file().sync_all()?;
        match staged.persist_noclobber(destination) {
            Ok(_) => Ok(()),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                self.verify(destination)
            }
            Err(error) => Err(error.error.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JavaLock {
    pub release: String,
    pub major: u32,
    pub platforms: BTreeMap<String, Download>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeLock {
    pub fingerprint: String,
    pub minecraft: String,
    pub loader: String,
    pub loader_version: String,
    pub metadata: Download,
    pub java: JavaLock,
    pub fabric_libraries: Vec<Download>,
    pub fabric_jvm: Vec<String>,
}

impl RuntimeLock {
    pub fn fingerprint(manifest: &Manifest, side: Side) -> Result<String> {
        Ok(hash_bytes(&serde_json::to_vec(&(
            &manifest.minecraft,
            side,
            &manifest.target(side).loader,
            &manifest.target(side).loader_version,
        ))?))
    }
    pub fn validate(&self) -> Result<()> {
        crate::model::validate_hash(&self.fingerprint)?;
        identifier(&self.minecraft)?;
        identifier(&self.loader_version)?;
        ensure!(
            ["vanilla", "fabric"].contains(&self.loader.as_str()),
            "unsupported runtime"
        );
        self.metadata.validate()?;
        ensure!(
            !self.java.platforms.is_empty() && (8..=50).contains(&self.java.major),
            "invalid Java distribution"
        );
        for archive in self.java.platforms.values() {
            archive.validate()?;
        }
        if self.loader == "vanilla" {
            ensure!(
                self.loader_version == self.minecraft
                    && self.fabric_libraries.is_empty()
                    && self.fabric_jvm.is_empty(),
                "vanilla runtime must not contain loader libraries or arguments"
            );
        } else {
            ensure!(
                !self.fabric_libraries.is_empty() && self.fabric_libraries.len() <= 256,
                "invalid Fabric library count"
            );
        }
        for library in &self.fabric_libraries {
            library.validate()?;
        }
        for argument in &self.fabric_jvm {
            ensure!(
                argument.starts_with("-D")
                    && argument.len() < 8192
                    && !argument.contains(['\0', '\n', '\r']),
                "unsupported Fabric JVM argument"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct VersionList {
    latest: LatestVersions,
    versions: Vec<VersionEntry>,
}
#[derive(Debug, Deserialize)]
struct LatestVersions {
    release: String,
}
#[derive(Debug, Deserialize)]
struct VersionEntry {
    id: String,
    url: String,
    sha1: String,
    #[serde(rename = "type")]
    kind: String,
}

const VERSION_MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

pub fn latest_release() -> Result<String> {
    let versions: VersionList = registry::json(&Url::parse(VERSION_MANIFEST)?)?;
    versions.release()
}

impl VersionList {
    fn release(self) -> Result<String> {
        identifier(&self.latest.release)?;
        ensure!(
            self.versions
                .iter()
                .any(|entry| entry.id == self.latest.release && entry.kind == "release"),
            "latest stable Minecraft release is missing from the official manifest"
        );
        Ok(self.latest.release)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameMetadata {
    pub id: String,
    pub main_class: String,
    pub downloads: BTreeMap<String, GameDownload>,
    pub libraries: Vec<Library>,
    pub asset_index: Option<AssetIndex>,
    pub java_version: Option<JavaVersion>,
    #[serde(default)]
    pub arguments: Arguments,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaVersion {
    pub major_version: u32,
}
#[derive(Debug, Clone, Deserialize)]
pub struct GameDownload {
    pub sha1: String,
    pub size: u64,
    pub url: String,
}
impl GameDownload {
    fn download(&self, filename: impl Into<String>) -> Download {
        Download {
            url: self.url.clone(),
            algorithm: Algorithm::Sha1,
            hash: self.sha1.clone(),
            filename: filename.into(),
            size: Some(self.size),
        }
    }
}
#[derive(Debug, Clone, Deserialize)]
pub struct AssetIndex {
    pub id: String,
    pub url: String,
    pub sha1: String,
    pub size: u64,
}
#[derive(Debug, Deserialize)]
struct AssetObjects {
    objects: BTreeMap<String, AssetObject>,
}
#[derive(Debug, Deserialize)]
struct AssetObject {
    hash: String,
    size: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Library {
    pub name: String,
    pub downloads: LibraryDownloads,
    pub rules: Option<Vec<Rule>>,
    pub natives: Option<BTreeMap<String, String>>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct LibraryDownloads {
    pub artifact: Option<GameDownload>,
    #[serde(default)]
    pub classifiers: BTreeMap<String, GameDownload>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub action: String,
    pub os: Option<RuleOs>,
    pub features: Option<BTreeMap<String, bool>>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct RuleOs {
    pub name: Option<String>,
    pub arch: Option<String>,
    pub version: Option<String>,
}
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<Argument>,
    #[serde(default)]
    pub jvm: Vec<Argument>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Argument {
    Plain(String),
    Conditional {
        rules: Vec<Rule>,
        value: ArgumentValue,
    },
}
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ArgumentValue {
    One(String),
    Many(Vec<String>),
}

pub fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}
pub fn os_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "osx"
    } else {
        std::env::consts::OS
    }
}

pub fn rules_allow(rules: Option<&[Rule]>, features: &BTreeMap<String, bool>) -> Result<bool> {
    let Some(rules) = rules else {
        return Ok(true);
    };
    let mut allowed = false;
    for rule in rules {
        ensure!(
            ["allow", "disallow"].contains(&rule.action.as_str()),
            "unknown Minecraft rule action"
        );
        if let Some(os) = &rule.os {
            if os.name.as_deref().is_some_and(|name| name != os_name()) {
                continue;
            }
            if os
                .arch
                .as_deref()
                .is_some_and(|arch| !match std::env::consts::ARCH {
                    "x86_64" => ["x86_64", "amd64", "x64"].contains(&arch),
                    "aarch64" => ["aarch64", "arm64"].contains(&arch),
                    other => arch == other,
                })
            {
                continue;
            }
            ensure!(
                os.version.is_none(),
                "OS-version-specific Minecraft rules are not yet supported"
            );
        }
        if rule.features.as_ref().is_some_and(|required| {
            required
                .iter()
                .any(|(name, value)| features.get(name).copied().unwrap_or(false) != *value)
        }) {
            continue;
        }
        allowed = rule.action == "allow";
    }
    Ok(allowed)
}

pub fn arguments(arguments: &[Argument], features: &BTreeMap<String, bool>) -> Result<Vec<String>> {
    let mut result = vec![];
    for argument in arguments {
        match argument {
            Argument::Plain(value) => result.push(value.clone()),
            Argument::Conditional { rules, value } if rules_allow(Some(rules), features)? => {
                match value {
                    ArgumentValue::One(value) => result.push(value.clone()),
                    ArgumentValue::Many(values) => result.extend(values.clone()),
                }
            }
            _ => (),
        }
    }
    ensure!(
        result.len() <= 4096
            && result
                .iter()
                .all(|value| value.len() < 16384 && !value.contains('\0')),
        "invalid runtime arguments"
    );
    Ok(result)
}

#[derive(Deserialize)]
struct LoaderEntry {
    loader: Loader,
}
#[derive(Deserialize)]
struct Loader {
    version: String,
    stable: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FabricProfile {
    inherits_from: String,
    main_class: String,
    libraries: Vec<MavenLibrary>,
    #[serde(default)]
    arguments: Arguments,
}
#[derive(Deserialize)]
struct MavenLibrary {
    name: String,
    url: String,
    sha1: Option<String>,
    sha512: Option<String>,
    size: Option<u64>,
}

fn maven_path(coordinate: &str) -> Result<String> {
    let parts: Vec<_> = coordinate.split(':').collect();
    ensure!(
        parts.len() == 3 || parts.len() == 4,
        "invalid Maven coordinate"
    );
    for part in &parts {
        identifier(part)?;
    }
    let classifier = parts.get(3).map(|p| format!("-{p}")).unwrap_or_default();
    Ok(format!(
        "{}/{}/{}/{}-{}{}.jar",
        parts[0].replace('.', "/"),
        parts[1],
        parts[2],
        parts[1],
        parts[2],
        classifier
    ))
}

#[derive(Deserialize)]
struct JavaAsset {
    release_name: String,
    binary: JavaBinary,
}
#[derive(Deserialize)]
struct JavaBinary {
    os: String,
    architecture: String,
    package: JavaPackage,
}
#[derive(Deserialize)]
struct JavaPackage {
    link: String,
    checksum: String,
    name: String,
    size: u64,
}
fn java_platform(binary: &JavaBinary) -> Option<String> {
    let os = match binary.os.as_str() {
        "mac" => "macos",
        "linux" => "linux",
        "windows" => "windows",
        _ => return None,
    };
    let arch = match binary.architecture.as_str() {
        "x64" => "x86_64",
        "aarch64" => "aarch64",
        _ => return None,
    };
    Some(format!("{os}-{arch}"))
}

fn java_lock(major: u32) -> Result<JavaLock> {
    let assets: Vec<JavaAsset> = registry::json(&Url::parse(&format!(
        "https://api.adoptium.net/v3/assets/latest/{major}/hotspot?image_type=jdk&heap_size=normal&vendor=eclipse"
    ))?)?;
    let release = assets
        .iter()
        .find(|asset| java_platform(&asset.binary).as_deref() == Some(&platform()))
        .context("Temurin is not available for this Java version and platform")?
        .release_name
        .clone();
    let mut platforms = BTreeMap::new();
    for asset in assets
        .into_iter()
        .filter(|asset| asset.release_name == release)
    {
        if let Some(platform) = java_platform(&asset.binary) {
            let p = asset.binary.package;
            platforms.insert(
                platform,
                Download {
                    url: p.link,
                    algorithm: Algorithm::Sha256,
                    hash: p.checksum,
                    filename: p.name,
                    size: Some(p.size),
                },
            );
        }
    }
    Ok(JavaLock {
        release,
        major,
        platforms,
    })
}

pub fn resolve(
    manifest: &Manifest,
    side: Side,
    previous: Option<&RuntimeLock>,
    update: bool,
    cache: &Cache,
) -> Result<RuntimeLock> {
    let fingerprint = RuntimeLock::fingerprint(manifest, side)?;
    if let Some(previous) = previous.filter(|p| !update && p.fingerprint == fingerprint) {
        previous.validate()?;
        return Ok(previous.clone());
    }
    ensure!(
        ["fabric", "vanilla"].contains(&manifest.target(side).loader.as_str()),
        "runtime preparation supports Fabric and vanilla"
    );
    let versions: VersionList = registry::json(&Url::parse(VERSION_MANIFEST)?)?;
    let entry = versions
        .versions
        .into_iter()
        .find(|v| v.id == manifest.minecraft)
        .context("Minecraft version not found")?;
    let metadata = Download {
        url: entry.url,
        algorithm: Algorithm::Sha1,
        hash: entry.sha1,
        filename: format!("minecraft-{}.json", manifest.minecraft),
        size: None,
    };
    let game: GameMetadata = read_json(&metadata.acquire(cache, false)?)?;
    ensure!(
        game.id == manifest.minecraft,
        "Minecraft metadata identity mismatch"
    );
    let major = game
        .java_version
        .as_ref()
        .map(|j| j.major_version)
        .context("Minecraft metadata does not declare its Java version")?;
    if manifest.target(side).loader == "vanilla" {
        let runtime = RuntimeLock {
            fingerprint,
            minecraft: manifest.minecraft.clone(),
            loader: "vanilla".into(),
            loader_version: manifest.minecraft.clone(),
            metadata,
            java: java_lock(major)?,
            fabric_libraries: vec![],
            fabric_jvm: vec![],
        };
        runtime.validate()?;
        return Ok(runtime);
    }
    let loaders: Vec<LoaderEntry> = registry::json(&Url::parse(&format!(
        "https://meta.fabricmc.net/v2/versions/loader/{}",
        manifest.minecraft
    ))?)?;
    let loader = loaders
        .into_iter()
        .find(|entry| {
            manifest
                .target(side)
                .loader_version
                .as_ref()
                .map(|v| &entry.loader.version == v)
                .unwrap_or(entry.loader.stable)
        })
        .context("compatible Fabric loader not found")?
        .loader
        .version;
    let route = if side == Side::Client {
        "profile"
    } else {
        "server"
    };
    let profile: FabricProfile = registry::json(&Url::parse(&format!(
        "https://meta.fabricmc.net/v2/versions/loader/{}/{loader}/{route}/json",
        manifest.minecraft
    ))?)?;
    ensure!(
        profile.inherits_from == manifest.minecraft,
        "Fabric profile inherits a different Minecraft version"
    );
    let main_class = if side == Side::Client {
        "net.fabricmc.loader.impl.launch.knot.KnotClient"
    } else {
        "net.fabricmc.loader.impl.launch.knot.KnotServer"
    };
    ensure!(
        profile.main_class == main_class,
        "unexpected Fabric main class"
    );
    let mut fabric_libraries = vec![];
    for library in profile.libraries {
        let path = maven_path(&library.name)?;
        let url = https_url(&library.url)?.join(&path)?;
        ensure!(
            url.host_str() == Some("maven.fabricmc.net"),
            "unexpected Fabric Maven repository"
        );
        let (algorithm, hash) = if let Some(hash) = library.sha512 {
            (Algorithm::Sha512, hash)
        } else if let Some(hash) = library.sha1 {
            (Algorithm::Sha1, hash)
        } else {
            let mut text = String::new();
            registry::response(&format!("{url}.sha1"))?
                .take(1025)
                .read_to_string(&mut text)?;
            ensure!(text.len() <= 1024, "Maven checksum is too large");
            (
                Algorithm::Sha1,
                text.split_whitespace()
                    .next()
                    .context("Maven checksum is empty")?
                    .into(),
            )
        };
        fabric_libraries.push(Download {
            url: url.into(),
            algorithm,
            hash,
            filename: Path::new(&path)
                .file_name()
                .context("invalid Maven path")?
                .to_str()
                .context("invalid Maven filename")?
                .into(),
            size: library.size,
        });
    }
    let runtime = RuntimeLock {
        fingerprint,
        minecraft: manifest.minecraft.clone(),
        loader: "fabric".into(),
        loader_version: loader,
        metadata,
        java: java_lock(major)?,
        fabric_libraries,
        fabric_jvm: arguments(&profile.arguments.jvm, &BTreeMap::new())?,
    };
    runtime.validate()?;
    Ok(runtime)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    ensure!(
        path.metadata()?.len() <= 32 * 1024 * 1024,
        "runtime metadata is too large"
    );
    serde_json::from_reader(File::open(path)?).context("invalid runtime metadata")
}

pub struct PreparedRuntime {
    pub java: PathBuf,
    pub java_root: PathBuf,
    pub classpath: Vec<PathBuf>,
    pub game_jar: PathBuf,
    pub assets: Option<PathBuf>,
    pub asset_index: Option<String>,
    pub metadata: GameMetadata,
    pub readonly_roots: Vec<PathBuf>,
}

impl PreparedRuntime {
    /// Minecraft writes downloaded skins below assets_root. Give each game its
    /// own copies so skin writes never require access to the shared runtime cache.
    pub fn game_assets(&self, game: &Path) -> Result<PathBuf> {
        let source = self.assets.as_ref().context("client assets missing")?;
        let index = self
            .metadata
            .asset_index
            .as_ref()
            .context("client asset index missing")?;
        let root = storage::directory(game, "enderpin-assets")?;
        storage::directory(&root, "indexes")?;
        let reference = Download {
            url: index.url.clone(),
            algorithm: Algorithm::Sha1,
            hash: index.sha1.clone(),
            filename: format!("{}.json", index.id),
            size: Some(index.size),
        };
        let relative = format!("indexes/{}.json", index.id);
        let source_index = storage::safe_path(source, &relative)?;
        reference.copy_verified(&source_index, &storage::safe_path(&root, &relative)?)?;
        let objects: AssetObjects = read_json(&source_index)?;
        for object in objects.objects.values() {
            let prefix = object.hash.get(..2).context("invalid asset hash")?;
            storage::directory(&root, &format!("objects/{prefix}"))?;
            let relative = format!("objects/{prefix}/{}", object.hash);
            let download = Download {
                url: format!(
                    "https://resources.download.minecraft.net/{prefix}/{}",
                    object.hash
                ),
                algorithm: Algorithm::Sha1,
                hash: object.hash.clone(),
                filename: object.hash.clone(),
                size: Some(object.size),
            };
            download.copy_verified(
                &storage::safe_path(source, &relative)?,
                &storage::safe_path(&root, &relative)?,
            )?;
        }
        Ok(root)
    }
}

pub fn prepare(
    lock: &RuntimeLock,
    side: Side,
    cache: &Cache,
    offline: bool,
    mut progress: impl FnMut(&str),
) -> Result<PreparedRuntime> {
    lock.validate()?;
    progress("Minecraft metadata");
    let metadata: GameMetadata = read_json(&lock.metadata.acquire(cache, offline)?)?;
    ensure!(
        metadata.id == lock.minecraft,
        "locked Minecraft metadata identity mismatch"
    );
    if side == Side::Client && cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        ensure!(
            !metadata
                .libraries
                .iter()
                .any(|lib| lib.name.starts_with("org.lwjgl:")
                    && lib.name.ends_with(":natives-linux")),
            "this Minecraft profile distributes Linux x86_64 natives only; Linux ARM64 client launch is not supported for this version"
        );
    }
    let runtime_cache = cache.runtime_root()?;
    let java_archive = lock
        .java
        .platforms
        .get(&platform())
        .context("locked Java distribution does not support this platform")?;
    progress("Java");
    let java_download = java_archive.acquire(cache, offline)?;
    let java_root = storage::safe_path(&runtime_cache, &format!("java-{}", java_archive.hash))?;
    if java_root.try_exists()? {
        archive::verify(&java_root)?;
    } else {
        let temporary = tempfile::tempdir_in(&runtime_cache)?;
        archive::extract(
            &java_download,
            java_archive.filename.ends_with(".tar.gz"),
            temporary.path(),
        )?;
        archive::java(temporary.path())?;
        archive::stamp(temporary.path())?;
        let staged = temporary.keep();
        if let Err(error) = fs::rename(&staged, &java_root) {
            if java_root.is_dir() {
                archive::verify(&java_root)?;
                fs::remove_dir_all(staged)?;
            } else {
                return Err(error.into());
            }
        }
    }
    let java = archive::java(&java_root)?;
    let mut classpath = vec![];
    if lock.loader == "fabric" {
        progress("Fabric libraries");
    }
    for library in &lock.fabric_libraries {
        classpath.push(library.acquire(cache, offline)?);
    }
    progress("Minecraft game files");
    let game_download = metadata
        .downloads
        .get(side.name())
        .context("Minecraft distribution does not contain this target")?
        .download(format!("minecraft-{}-{}.jar", lock.minecraft, side.name()));
    let original_game = game_download.acquire(cache, offline)?;
    let game_jar;
    let mut assets = None;
    let mut asset_index = None;
    if side == Side::Server {
        let (jar, libraries) = server_bundle(&original_game, &runtime_cache, &game_download.hash)?;
        game_jar = jar;
        classpath.extend(libraries);
    } else {
        game_jar = original_game;
        for library in &metadata.libraries {
            if !rules_allow(library.rules.as_deref(), &BTreeMap::new())? {
                continue;
            }
            if let Some(artifact) = &library.downloads.artifact {
                let path = maven_path(&library.name)?;
                classpath.push(
                    artifact
                        .download(
                            Path::new(&path)
                                .file_name()
                                .context("invalid library name")?
                                .to_string_lossy()
                                .into_owned(),
                        )
                        .acquire(cache, offline)?,
                );
            }
            ensure!(
                library.natives.is_none(),
                "legacy extracted-native Minecraft profiles are not yet supported"
            );
        }
        if let Some(index) = &metadata.asset_index {
            identifier(&index.id)?;
            progress("Minecraft assets");
            let reference = Download {
                url: index.url.clone(),
                algorithm: Algorithm::Sha1,
                hash: index.sha1.clone(),
                filename: format!("{}.json", index.id),
                size: Some(index.size),
            };
            let path = reference.acquire(cache, offline)?;
            let objects: AssetObjects = read_json(&path)?;
            ensure!(
                objects.objects.len() <= 100_000,
                "too many Minecraft assets"
            );
            let asset_root = storage::directory(&runtime_cache, &format!("assets-{}", index.sha1))?;
            storage::directory(&asset_root, "indexes")?;
            let index_path =
                storage::safe_path(&asset_root, &format!("indexes/{}.json", index.id))?;
            reference.copy_verified(&path, &index_path)?;
            // ponytail: bounded four-worker batches; replace only if asset fetch profiling warrants it.
            let downloads: Vec<_> = objects
                .objects
                .values()
                .map(|object| {
                    ensure!(
                        object.hash.len() == 40
                            && object.hash.bytes().all(|b| b.is_ascii_hexdigit()),
                        "invalid asset digest"
                    );
                    Ok(Download {
                        url: format!(
                            "https://resources.download.minecraft.net/{}/{}",
                            &object.hash[..2],
                            object.hash
                        ),
                        algorithm: Algorithm::Sha1,
                        hash: object.hash.clone(),
                        filename: object.hash.clone(),
                        size: Some(object.size),
                    })
                })
                .collect::<Result<_>>()?;
            for (batch, chunk) in downloads.chunks(4).enumerate() {
                if batch % 32 == 0 {
                    progress(&format!(
                        "Minecraft assets {}/{}",
                        batch * 4,
                        downloads.len()
                    ));
                }
                std::thread::scope(|scope| -> Result<()> {
                    let handles: Vec<_> = chunk
                        .iter()
                        .map(|download| {
                            scope.spawn(|| -> Result<()> {
                                let relative =
                                    format!("objects/{}/{}", &download.hash[..2], download.hash);
                                let destination = storage::safe_path(&asset_root, &relative)?;
                                if destination.exists() {
                                    download.verify(&destination)?;
                                    return Ok(());
                                }
                                let cached = download.acquire(cache, offline)?;
                                storage::directory(
                                    &asset_root,
                                    &format!("objects/{}", &download.hash[..2]),
                                )?;
                                download.copy_verified(&cached, &destination)
                            })
                        })
                        .collect();
                    for handle in handles {
                        handle
                            .join()
                            .map_err(|_| anyhow::anyhow!("asset worker failed"))??;
                    }
                    Ok(())
                })?;
            }
            assets = Some(asset_root);
            asset_index = Some(index.id.clone());
        }
    }
    classpath.push(game_jar.clone());
    Ok(PreparedRuntime {
        java,
        java_root,
        classpath,
        game_jar,
        assets,
        asset_index,
        metadata,
        readonly_roots: vec![runtime_cache],
    })
}

fn server_bundle(
    archive_path: &Path,
    runtime_cache: &Path,
    hash: &str,
) -> Result<(PathBuf, Vec<PathBuf>)> {
    let root = storage::safe_path(runtime_cache, &format!("server-{hash}"))?;
    if !root.exists() {
        let temporary = tempfile::tempdir_in(runtime_cache)?;
        let mut zip = zip::ZipArchive::new(File::open(archive_path)?)?;
        let mut total = 0u64;
        let mut entries = 0u32;
        for group in ["versions", "libraries"] {
            let mut list = String::new();
            zip.by_name(&format!("META-INF/{group}.list"))?
                .take(1024 * 1024 + 1)
                .read_to_string(&mut list)?;
            ensure!(list.len() <= 1024 * 1024, "server bundle list is too large");
            for line in list.lines().filter(|line| !line.is_empty()) {
                let parts: Vec<_> = line.split('\t').collect();
                ensure!(parts.len() == 3, "invalid server bundle entry");
                let mut entry = zip.by_name(&format!("META-INF/{group}/{}", parts[2]))?;
                entries += 1;
                total = total
                    .checked_add(entry.size())
                    .context("bundle size overflow")?;
                ensure!(
                    entries <= 10000 && total <= 3 * 1024 * 1024 * 1024,
                    "expanded server bundle is too large"
                );
                let relative = format!("{group}/{}", parts[2]);
                let path = storage::safe_path(temporary.path(), &relative)?;
                if let Some(parent) = Path::new(&relative).parent().and_then(Path::to_str) {
                    storage::directory(temporary.path(), parent)?;
                }
                ensure!(
                    entry.size() < 512 * 1024 * 1024,
                    "server bundle entry is too large"
                );
                let size = entry.size();
                let mut output = File::options().write(true).create_new(true).open(&path)?;
                ensure!(
                    std::io::copy(&mut entry.by_ref().take(size + 1), &mut output)? == size,
                    "invalid server bundle entry length"
                );
                Download {
                    url: "https://piston-data.mojang.com/".into(),
                    algorithm: Algorithm::Sha256,
                    hash: parts[0].into(),
                    filename: path
                        .file_name()
                        .context("bundle filename missing")?
                        .to_string_lossy()
                        .into_owned(),
                    size: None,
                }
                .verify(&path)?;
            }
        }
        archive::stamp(temporary.path())?;
        let staged = temporary.keep();
        if let Err(error) = fs::rename(&staged, &root) {
            if root.is_dir() {
                archive::verify(&root)?;
                fs::remove_dir_all(staged)?;
            } else {
                return Err(error.into());
            }
        }
    }
    archive::verify(&root)?;
    fn jars(root: &Path, result: &mut Vec<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            ensure!(
                !storage::is_link(&fs::symlink_metadata(entry.path())?),
                "unexpected server bundle link"
            );
            if entry.file_type()?.is_dir() {
                jars(&entry.path(), result)?;
            } else if entry.path().extension().is_some_and(|e| e == "jar") {
                result.push(entry.path());
            }
        }
        Ok(())
    }
    let mut versions = vec![];
    jars(&root.join("versions"), &mut versions)?;
    ensure!(
        versions.len() == 1,
        "expected exactly one bundled server version"
    );
    let mut libraries = vec![];
    jars(&root.join("libraries"), &mut libraries)?;
    libraries.sort();
    Ok((versions.pop().context("missing server JAR")?, libraries))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn latest_uses_stable_release_and_vanilla_rejects_mods() -> Result<()> {
        let list: VersionList = serde_json::from_value(serde_json::json!({
            "latest": {"release": "26.2", "snapshot": "26.3-rc-3"},
            "versions": [
                {"id": "26.3-rc-3", "type": "snapshot", "url": "https://example.org/snapshot", "sha1": "0"},
                {"id": "26.2", "type": "release", "url": "https://example.org/release", "sha1": "1"}
            ]
        }))?;
        assert_eq!(list.release()?, "26.2");
        let bad: VersionList = serde_json::from_value(serde_json::json!({
            "latest": {"release": "26.3-rc-3"},
            "versions": [{"id": "26.3-rc-3", "type": "snapshot", "url": "https://example.org/snapshot", "sha1": "0"}]
        }))?;
        assert!(bad.release().is_err());
        for side in Side::ALL {
            let mut manifest = crate::Workspace::quick_manifest("26.2".into())?;
            manifest.target_mut(side).packages.insert(
                "mod".into(),
                crate::Package::modrinth("mod", crate::Kind::Mod),
            );
            assert!(manifest.validate().is_err());
        }
        Ok(())
    }
    #[test]
    fn validates_coordinates_and_runtime_rules() -> Result<()> {
        assert_eq!(
            maven_path("org.example:library:1.2.3")?,
            "org/example/library/1.2.3/library-1.2.3.jar"
        );
        assert!(maven_path("../evil:x:1").is_err());
        let rules = vec![Rule {
            action: "allow".into(),
            os: None,
            features: Some(BTreeMap::from([("is_demo_user".into(), true)])),
        }];
        assert!(!rules_allow(Some(&rules), &BTreeMap::new())?);
        assert!(rules_allow(
            Some(&rules),
            &BTreeMap::from([("is_demo_user".into(), true)])
        )?);
        Ok(())
    }
}
