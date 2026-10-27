//! Seed native configuration without starting a playable world or loading user packages.
use crate::{
    Side, Workspace,
    model::TargetType,
    runtime, sandbox,
    storage::{self, Change, Content},
};
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File},
    io::Read,
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};

const MAX_CONFIG: u64 = 1024 * 1024;

/// Never replaces an existing source, including one supplied through a file group.
pub fn configs(
    ws: &mut Workspace,
    names: &[&str],
    offline: bool,
    mut progress: impl FnMut(&str),
) -> Result<Vec<String>> {
    let mut changes = Vec::new();
    for &name in names {
        let config = ws.manifest.target(name)?;
        if config.kind == TargetType::Client {
            continue;
        }
        let filename = if config.kind == TargetType::Proxy {
            "velocity.toml"
        } else {
            "server.properties"
        };
        let destination = format!("{name}/files/{filename}");
        if storage::safe_path(&ws.root, &destination)?.try_exists()? {
            continue;
        }
        let mut supplied = false;
        for group in &config.file_groups {
            supplied |= storage::safe_path(&ws.root, &format!("shared/{group}/files/{filename}"))?
                .try_exists()?;
        }
        if supplied {
            continue;
        }
        // Do not seed a source on top of a pre-existing, unmanaged game configuration.
        ensure!(
            !storage::safe_path(&ws.root, &format!("{name}/run/{filename}"))?.try_exists()?,
            "{name}/run/{filename} already exists; review and copy it to {destination} instead of initializing it"
        );
        let _lease = storage::target_lock(&ws.root, name)?;
        let lock = ws
            .lockfile
            .runtimes
            .get(name)
            .context("runtime is not locked; run sync first")?;
        ensure!(
            lock.fingerprint == runtime::RuntimeLock::fingerprint(&ws.manifest, name)?,
            "stale runtime lock"
        );
        progress(&format!("{name}: generating {filename}"));
        let bytes = if config.kind == TargetType::Proxy {
            let jar = lock
                .server_jar
                .as_ref()
                .context("Velocity artifact missing")?
                .acquire(&ws.cache, offline)?;
            velocity_defaults(&jar)?
        } else {
            // Use the pinned official Minecraft initializer, not Fabric entrypoints or plugins.
            // Paper-specific YAML and mod configs are generated later by their own software.
            let mut vanilla = lock.clone();
            vanilla.loader = "vanilla".into();
            vanilla.loader_version = vanilla.minecraft.clone();
            vanilla.server_jar = None;
            vanilla.fabric_libraries.clear();
            vanilla.fabric_jvm.clear();
            let runtime =
                runtime::prepare(&vanilla, Side::Server, &ws.cache, offline, &mut progress)?;
            server_defaults(&runtime)?
        };
        changes.push(Change {
            path: destination,
            content: Content::Bytes(bytes),
        });
    }
    let created = changes.iter().map(|c| c.path.clone()).collect();
    if !changes.is_empty() {
        storage::commit(&ws.root, changes)?;
    }
    Ok(created)
}

fn velocity_defaults(jar: &Path) -> Result<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(File::open(jar)?)?;
    let mut entry = zip.by_name("default-velocity.toml").context("this Velocity build has no bundled default-velocity.toml; configure velocity.toml manually")?;
    ensure!(
        entry.size() <= MAX_CONFIG,
        "Velocity default configuration is too large"
    );
    let mut bytes = Vec::new();
    (&mut entry).take(MAX_CONFIG + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_CONFIG,
        "Velocity default configuration is too large"
    );
    let _: toml::Value = toml::from_str(std::str::from_utf8(&bytes)?)?;
    Ok(bytes)
}

fn server_defaults(runtime: &runtime::PreparedRuntime) -> Result<Vec<u8>> {
    let stage = tempfile::tempdir()?;
    let game = fs::create_dir(stage.path().join("game"))
        .map(|_| stage.path().join("game").canonicalize())??;
    let temporary = fs::create_dir(stage.path().join("tmp"))
        .map(|_| stage.path().join("tmp").canonicalize())??;
    let policy = sandbox::Policy {
        game: game.clone(),
        temporary,
        java_home: runtime
            .java
            .parent()
            .and_then(Path::parent)
            .context("invalid Java path")?
            .canonicalize()?,
        readonly: runtime.readonly_roots.clone(),
        network: false,
        desktop: false,
        permissions: sandbox::permissions::Settings {
            network: false,
            audio: false,
            desktop_integration: false,
            open_links: false,
            open_folders: false,
            import_files: false,
            skin_cache: false,
            graphics_cache: false,
            narrator: false,
            account_authentication: false,
            ..Default::default()
        },
        extra: vec![],
    };
    #[cfg(not(windows))]
    let mut command = sandbox::command(&runtime.java, &policy)?;
    #[cfg(windows)]
    let mut command = sandbox::windows::configuration(&runtime.java, &policy)?;
    let classpath = std::env::join_paths(
        runtime
            .classpath
            .iter()
            .chain(std::iter::once(&runtime.game_jar))
            .map(|p| storage::java_path(p)),
    )?;
    command
        .args(["-Xmx256M", "-XX:-UsePerfData", "-Djava.awt.headless=true"])
        .arg(format!(
            "-Djava.io.tmpdir={}",
            storage::java_path(&policy.temporary).display()
        ))
        .arg("-cp")
        .arg(classpath)
        .args(["net.minecraft.server.Main", "--initSettings"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(not(windows))]
    let child = command
        .spawn()
        .context("could not start sandboxed settings initializer")?;
    #[cfg(windows)]
    let child = sandbox::windows::spawn_configured(&command, &policy, &[], true)?;
    let mut child = Initializer(child);
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(status) = child.0.try_wait()? {
            ensure!(
                status.success(),
                "Minecraft settings initializer failed ({status}); no config installed"
            );
            break;
        }
        ensure!(
            Instant::now() < deadline,
            "Minecraft settings initializer timed out; no config installed"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    ensure!(
        !game.join("world").exists(),
        "initializer unexpectedly created a world"
    );
    let file = storage::safe_path(&game, "server.properties")?;
    ensure!(
        fs::metadata(&file)?.len() <= MAX_CONFIG,
        "server.properties is too large"
    );
    let bytes = fs::read(file).context(
        "Minecraft did not generate server.properties; this version needs manual initialization",
    )?;
    ensure!(!bytes.is_empty(), "empty generated server.properties");
    Ok(bytes)
}

#[cfg(not(windows))]
struct Initializer(std::process::Child);
#[cfg(windows)]
struct Initializer(sandbox::windows::Process);
impl Drop for Initializer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn extracts_only_the_native_default_and_rejects_missing_or_invalid_content() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("velocity.jar");
        for content in ["config-version = \\\"2.9\\\"", "invalid"] {
            let mut zip = zip::ZipWriter::new(File::create(&path)?);
            zip.start_file(
                "default-velocity.toml",
                zip::write::SimpleFileOptions::default(),
            )?;
            zip.write_all(content.as_bytes())?;
            zip.finish()?;
            assert!(velocity_defaults(&path).is_err());
        }
        let mut zip = zip::ZipWriter::new(File::create(&path)?);
        zip.start_file(
            "default-velocity.toml",
            zip::write::SimpleFileOptions::default(),
        )?;
        zip.write_all(b"config-version = '2.9'\n")?;
        zip.start_file(
            "forwarding.secret",
            zip::write::SimpleFileOptions::default(),
        )?;
        zip.write_all(b"must not be imported")?;
        zip.finish()?;
        assert_eq!(velocity_defaults(&path)?, b"config-version = '2.9'\n");
        assert!(!dir.path().join("forwarding.secret").exists());
        Ok(())
    }
}
