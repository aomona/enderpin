use crate::{
    auth::{
        bridge,
        broker::service::{OfficialOperations, UserAttributesSchema},
    },
    runtime::{PreparedRuntime, RuntimeLock},
    storage,
};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

fn files() -> [(&'static str, &'static [u8]); 4] {
    [
        (bridge::AGENT.name, bridge::AGENT.bytes),
        (bridge::BOOTSTRAP.name, bridge::BOOTSTRAP.bytes),
        (
            "narrator-bridge.jar",
            include_bytes!(concat!(env!("OUT_DIR"), "/narrator-bridge.jar")).as_slice(),
        ),
        (bridge::NATIVE.name, bridge::NATIVE.bytes),
    ]
}

#[cfg(test)]
pub(crate) fn write_files(root: &Path) -> Result<()> {
    for (name, bytes) in files() {
        fs::write(storage::safe_path(root, name)?, bytes)?;
    }
    Ok(())
}

/// Content-addressed launch support keeps AppContainer identities stable across launches.
/// The caller holds the target lock; the game receives read-only access to this directory.
pub(crate) fn prepare(root: &Path, target: &str) -> Result<PathBuf> {
    crate::model::target_name(target)?;
    let mut hash = Sha256::new();
    for (_, bytes) in files() {
        hash.update(bytes);
    }
    let relative = format!(".enderpin/{target}/launch/{:x}", hash.finalize());
    let directory = storage::directory(root, &relative)?.canonicalize()?;
    crate::sandbox::validate_data_tree(&directory)?;
    for (name, bytes) in files() {
        let path = storage::safe_path(&directory, name)?;
        if path.try_exists()? {
            ensure!(
                storage::hash_file(&path)? == (crate::model::hash_bytes(bytes), bytes.len() as u64),
                "launch bridge differs from the bundled version; remove the corrupt bridge directory"
            );
        } else {
            use std::io::Write;
            let mut file = tempfile::NamedTempFile::new_in(&directory)?;
            file.write_all(bytes)?;
            file.persist_noclobber(path).map_err(|error| error.error)?;
        }
    }
    Ok(directory)
}

const FABRIC_ADAPTERS: &[(&str, &str)] = &[
    (
        "0.19.5",
        "93044e4dd46de5d8136701292f05e868da096d2c9fddb4793e4fdbcc63efc695",
    ),
    (
        "0.20.1",
        "d0c5a082e07410aaff9de7ab196aa6b4236002870f9ba3ad5c1260fcce5110ad",
    ),
];

pub(crate) struct AuthAdapter {
    java: u32,
    filename: &'static str,
    hash: &'static str,
    schema: UserAttributesSchema,
}

impl AuthAdapter {
    pub(crate) fn for_game(minecraft: &str) -> Result<Self> {
        let entry = include_str!("../java/auth-bridge/auth-adapters.properties")
            .lines()
            .filter_map(|line| line.split_once('='))
            .find(|(version, _)| *version == minecraft)
            .with_context(|| format!("authentication adapter has no verified mapping for Minecraft {minecraft}; see docs/auth-compatibility.md; refusing a raw-token fallback"))?.1;
        let fields: Vec<_> = entry.split('|').collect();
        ensure!(fields.len() == 8, "invalid bundled authentication adapter");
        let java = fields[0].parse()?;
        let filename = fields[1];
        let hash = fields[2];
        let schema = match fields[3] {
            "legacy" => UserAttributesSchema::Authlib6,
            "social" => UserAttributesSchema::Authlib9,
            _ => anyhow::bail!("invalid bundled authentication attributes schema"),
        };
        Ok(Self {
            java,
            filename,
            hash,
            schema,
        })
    }

    pub(crate) fn validate_loader(&self, loader: &str, version: Option<&str>) -> Result<()> {
        ensure!(
            loader == "vanilla"
                || (loader == "fabric"
                    && version
                        .is_none_or(|v| FABRIC_ADAPTERS.iter().any(|(version, _)| *version == v))),
            "authentication adapter does not support {loader} {}; authenticated clients require Vanilla or Fabric 0.19.5/0.20.1; pin loader_version = \"0.20.1\" for Fabric; refusing a raw-token fallback",
            version.unwrap_or("(unspecified)")
        );
        Ok(())
    }

    pub(crate) fn validate_java(&self, major: u32) -> Result<()> {
        ensure!(
            major == self.java,
            "authentication adapter requires Java {}, got {major}; refusing a raw-token fallback",
            self.java
        );
        Ok(())
    }

    fn validate_fabric(&self, classpath: &[PathBuf], loader: &str, version: &str) -> Result<()> {
        if loader != "fabric" {
            return Ok(());
        }
        let hash = FABRIC_ADAPTERS
            .iter()
            .find(|(v, _)| *v == version)
            .context("Fabric authentication adapter not verified")?
            .1;
        let jars: Vec<_> = classpath
            .iter()
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("fabric-loader-"))
            })
            .collect();
        let filename = format!("fabric-loader-{version}.jar");
        ensure!(
            jars.len() == 1
                && jars[0]
                    .file_name()
                    .is_some_and(|name| name == filename.as_str()),
            "authentication adapter requires the pinned official Fabric loader"
        );
        use std::io::Read;
        let mut bytes = Vec::new();
        fs::File::open(jars[0])?
            .take(8_388_609)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 8_388_608 && format!("{:x}", Sha256::digest(&bytes)) == hash,
            "Fabric loader does not match the verified authentication adapter"
        );
        Ok(())
    }

    fn validate_authlib(&self, classpath: &[PathBuf]) -> Result<()> {
        let authlibs: Vec<_> = classpath
            .iter()
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("authlib-"))
            })
            .collect();
        ensure!(
            authlibs.len() == 1
                && authlibs[0]
                    .file_name()
                    .is_some_and(|name| name == self.filename),
            "authentication bridge requires the pinned official authlib"
        );
        let file = fs::File::open(authlibs[0])?;
        use std::io::Read;
        let mut bytes = Vec::new();
        file.take(1_048_577).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 1_048_576 && format!("{:x}", Sha256::digest(&bytes)) == self.hash,
            "authlib does not match the verified bridge adapter"
        );
        Ok(())
    }
}

pub(crate) fn broker(
    runtime: &PreparedRuntime,
    lock: &RuntimeLock,
    session: &crate::auth::Session,
) -> Result<OfficialOperations> {
    let adapter = AuthAdapter::for_game(&lock.minecraft)?;
    adapter.validate_loader(&lock.loader, Some(&lock.loader_version))?;
    adapter.validate_java(lock.java.major)?;
    adapter.validate_authlib(&runtime.classpath)?;
    adapter.validate_fabric(&runtime.classpath, &lock.loader, &lock.loader_version)?;
    // Old game-visible private-key caches must not survive switching to host signing.
    let mut operations = OfficialOperations::new(
        Arc::new(session.clone()),
        session.uuid.clone(),
        adapter.schema,
    )?;
    operations.minecraft_version = lock.minecraft.clone();
    // Account authentication is a separate approved capability from the game's IP network.
    Ok(operations)
}

pub(crate) fn clear_profile_keys(game: &Path) -> Result<()> {
    let path = storage::safe_path(game, "profilekeys")?;
    if path.try_exists()? {
        crate::sandbox::validate_data_tree(&path)?;
        ensure!(path.is_dir(), "profilekeys must be a directory");
        fs::remove_dir_all(path).context("cannot remove legacy game-visible profile keys")?;
    }
    Ok(())
}

pub(crate) fn native(root: &Path) -> PathBuf {
    root.join(bridge::NATIVE.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authentication_compatibility_and_integrity_fail_closed() -> Result<()> {
        for line in include_str!("../java/auth-bridge/auth-adapters.properties")
            .lines()
            .filter(|line| !line.starts_with('#'))
        {
            let (minecraft, entry) = line.split_once('=').context("adapter row")?;
            let java = entry.split('|').next().context("Java version")?.parse()?;
            let adapter = AuthAdapter::for_game(minecraft)?;
            adapter.validate_loader("vanilla", Some(minecraft))?;
            adapter.validate_loader("fabric", Some("0.19.5"))?;
            adapter.validate_java(java)?;
            adapter.validate_loader("fabric", Some("0.20.1"))?;
            assert!(adapter.validate_loader("fabric", Some("0.99.0")).is_err());
            assert!(adapter.validate_fabric(&[], "fabric", "0.20.1").is_err());
            assert!(adapter.validate_loader("unknown", None).is_err());
            assert!(adapter.validate_java(java + 1).is_err());
            assert!(adapter.validate_authlib(&[]).is_err());
            let root = tempfile::tempdir()?;
            let jar = root.path().join(adapter.filename);
            fs::write(&jar, b"tampered official authlib")?;
            let error = adapter
                .validate_authlib(std::slice::from_ref(&jar))
                .expect_err("tampered authlib");
            assert!(
                error
                    .to_string()
                    .contains("does not match the verified bridge adapter")
            );
            assert!(adapter.validate_authlib(&[jar.clone(), jar]).is_err());
            assert!(
                adapter
                    .validate_authlib(&[root.path().join("authlib-wrong.jar")])
                    .is_err()
            );
        }
        assert!(AuthAdapter::for_game("unsupported").is_err());
        Ok(())
    }

    #[test]
    fn modified_or_duplicate_fabric_loader_fails_closed() -> Result<()> {
        let adapter = AuthAdapter::for_game("1.20.1")?;
        let root = tempfile::tempdir()?;
        for (version, _) in FABRIC_ADAPTERS {
            let jar = root.path().join(format!("fabric-loader-{version}.jar"));
            fs::write(&jar, b"modified loader")?;
            let error = adapter
                .validate_fabric(std::slice::from_ref(&jar), "fabric", version)
                .expect_err("modified loader");
            assert!(
                error
                    .to_string()
                    .contains("does not match the verified authentication adapter")
            );
            assert!(
                adapter
                    .validate_fabric(&[jar.clone(), jar], "fabric", version)
                    .is_err()
            );
        }
        adapter.validate_fabric(&[], "vanilla", "1.20.1")?;
        assert!(AuthAdapter::for_game("1.19.4").is_err());
        Ok(())
    }

    #[test]
    fn writer_uses_public_auth_assets_and_keeps_narrator() -> Result<()> {
        let root = tempfile::tempdir()?;
        write_files(root.path())?;
        for asset in bridge::ASSETS {
            assert_eq!(fs::read(root.path().join(asset.name))?, asset.bytes);
        }
        assert_eq!(
            fs::read(root.path().join("narrator-bridge.jar"))?,
            include_bytes!(concat!(env!("OUT_DIR"), "/narrator-bridge.jar"))
        );
        assert_eq!(fs::read_dir(root.path())?.count(), 4);
        assert_eq!(native(root.path()), root.path().join(bridge::NATIVE.name));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn writer_rejects_symlinks_for_each_asset() -> Result<()> {
        for name in bridge::ASSETS
            .map(|asset| asset.name)
            .into_iter()
            .chain(["narrator-bridge.jar"])
        {
            let root = tempfile::tempdir()?;
            let target = tempfile::NamedTempFile::new()?;
            fs::write(target.path(), b"keep me")?;
            std::os::unix::fs::symlink(target.path(), root.path().join(name))?;
            assert!(write_files(root.path()).is_err());
            assert_eq!(fs::read(target.path())?, b"keep me");
        }
        Ok(())
    }

    #[test]
    fn launch_support_is_stable_and_rejects_tampering() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let first = super::prepare(root.path(), "client")?;
        assert_eq!(first, super::prepare(root.path(), "client")?);
        for asset in bridge::ASSETS {
            assert_eq!(fs::read(first.join(asset.name))?, asset.bytes);
        }
        std::fs::write(first.join("auth-bridge.jar"), b"changed")?;
        assert!(super::prepare(root.path(), "client").is_err());
        Ok(())
    }
}
