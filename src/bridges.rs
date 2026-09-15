use crate::{
    auth::broker::{
        channel::PreparedBroker,
        service::{OfficialOperations, UserAttributesSchema},
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
        (
            "auth-bridge.jar",
            include_bytes!(concat!(env!("OUT_DIR"), "/auth-bridge.jar")).as_slice(),
        ),
        (
            "auth-bootstrap.jar",
            include_bytes!(concat!(env!("OUT_DIR"), "/auth-bootstrap.jar")).as_slice(),
        ),
        (
            "narrator-bridge.jar",
            include_bytes!(concat!(env!("OUT_DIR"), "/narrator-bridge.jar")).as_slice(),
        ),
        (
            env!("ENDERPIN_AUTH_NATIVE"),
            include_bytes!(concat!(env!("OUT_DIR"), "/", env!("ENDERPIN_AUTH_NATIVE"))).as_slice(),
        ),
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
    root.join(env!("ENDERPIN_AUTH_NATIVE"))
}
