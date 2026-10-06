use crate::{
    auth::{
        bridge,
        broker::{
            channel::PreparedBroker,
            service::{OfficialOperations, UserAttributesSchema},
        },
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

pub(crate) fn write_files(root: &Path) -> Result<()> {
    for (name, bytes) in [
        (bridge::AGENT.name, bridge::AGENT.bytes),
        (bridge::BOOTSTRAP.name, bridge::BOOTSTRAP.bytes),
        (
            "narrator-bridge.jar",
            include_bytes!(concat!(env!("OUT_DIR"), "/narrator-bridge.jar")).as_slice(),
        ),
        (bridge::NATIVE.name, bridge::NATIVE.bytes),
    ] {
        fs::write(storage::safe_path(root, name)?, bytes)?;
    }
    Ok(())
}

pub(crate) fn broker(
    runtime: &PreparedRuntime,
    lock: &RuntimeLock,
    session: &crate::auth::Session,
) -> Result<PreparedBroker> {
    let (java, filename, hash, schema) = match lock.minecraft.as_str() {
        "1.21.1" => (
            21,
            "authlib-6.0.54.jar",
            "319ea7b53b5e52f62ad3e2b81e9db7f0751240edac548bd74f5f19e35dc21a3b",
            UserAttributesSchema::Authlib6,
        ),
        "1.21.8" => (
            21,
            "authlib-6.0.58.jar",
            "7bea5444e83c8d343e11fb9e45939721f0db4321c5056ac846072e4a0bbe1321",
            UserAttributesSchema::Authlib6,
        ),
        "26.2" => (
            25,
            "authlib-9.0.75.jar",
            "1f77e70240548b9cd233da0e12938bbbe5597ac9e94f6c6b07577faf1461a951",
            UserAttributesSchema::Authlib9,
        ),
        _ => anyhow::bail!(
            "authentication broker supports Minecraft 1.21.1, 1.21.8 and 26.2; refusing a raw-token fallback"
        ),
    };
    ensure!(
        lock.java.major == java && lock.loader == "fabric" && lock.loader_version == "0.19.5",
        "authentication bridge requires the supported Java version and Fabric 0.19.5"
    );
    let authlibs: Vec<_> = runtime
        .classpath
        .iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("authlib-"))
        })
        .collect();
    ensure!(
        authlibs.len() == 1 && authlibs[0].file_name().is_some_and(|name| name == filename),
        "authentication bridge requires the pinned official authlib"
    );
    let file = fs::File::open(authlibs[0])?;
    use std::io::Read;
    let mut bytes = Vec::new();
    file.take(1_048_577).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 1_048_576 && format!("{:x}", Sha256::digest(&bytes)) == hash,
        "authlib does not match the verified bridge adapter"
    );
    // Old game-visible private-key caches must not survive switching to host signing.
    let operations =
        OfficialOperations::new(Arc::new(session.clone()), session.uuid.clone(), schema)?;
    // Account authentication is a separate approved capability from the game's IP network.
    Ok(PreparedBroker::new(Box::new(operations), true)?)
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
}
