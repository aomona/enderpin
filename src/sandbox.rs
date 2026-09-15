//! OS isolation. Unsupported or unavailable backends fail closed.
#[cfg(target_os = "macos")]
use anyhow::Context;
use anyhow::{Result, ensure};
pub mod permissions;
use permissions::{FolderGrant, Settings};
#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
pub mod windows;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub struct Policy {
    pub game: PathBuf,
    pub temporary: PathBuf,
    pub java_home: PathBuf,
    pub readonly: Vec<PathBuf>,
    pub network: bool,
    pub desktop: bool,
    pub permissions: Settings,
    pub extra: Vec<FolderGrant>,
}

impl Policy {
    pub fn validate(&self, java: &Path) -> Result<()> {
        self.permissions
            .validate_for(std::env::consts::OS, self.desktop)?;
        for path in [&self.game, &self.temporary, &self.java_home]
            .into_iter()
            .chain(self.readonly.iter())
            .chain(self.extra.iter().map(|grant| &grant.path))
        {
            ensure!(
                path.is_absolute() && path.is_dir() && path.canonicalize()? == *path,
                "sandbox paths must be canonical directories"
            );
        }
        ensure!(
            java.canonicalize()?.starts_with(&self.java_home),
            "Java must be inside the pinned runtime"
        );
        for writable in [&self.game, &self.temporary]
            .into_iter()
            .chain(self.extra.iter().filter(|g| g.write).map(|g| &g.path))
        {
            for readonly in self.readonly.iter().chain(std::iter::once(&self.java_home)) {
                ensure!(
                    !readonly.starts_with(writable) && !writable.starts_with(readonly),
                    "sandbox writable and runtime paths overlap"
                );
            }
        }
        ensure!(
            !self.game.starts_with(&self.temporary) && !self.temporary.starts_with(&self.game),
            "temporary directory must be separate from game data"
        );
        validate_data_tree(&self.game)?;
        for grant in &self.extra {
            validate_data_tree(&grant.path)?;
        }
        for path in self.read_only_game()? {
            ensure!(path.is_dir(), "read-only game directory is missing");
        }
        Ok(())
    }

    pub fn read_only_game(&self) -> Result<Vec<PathBuf>> {
        self.permissions
            .read_only
            .iter()
            .map(|dir| crate::storage::safe_path(&self.game, dir.name()))
            .collect()
    }

    pub fn grants(&self) -> Vec<FolderGrant> {
        let mut grants: Vec<_> = self
            .readonly
            .iter()
            .chain([&self.java_home])
            .map(|path| FolderGrant {
                path: path.clone(),
                write: false,
            })
            .collect();
        grants.extend([
            FolderGrant {
                path: self.game.clone(),
                write: self.permissions.game_write,
            },
            FolderGrant {
                path: self.temporary.clone(),
                write: true,
            },
        ]);
        grants.extend(self.extra.clone());
        grants.sort_by(|a, b| a.path.cmp(&b.path));
        grants
    }
}

/// Existing aliases must not turn a scoped permission into access to another tree.
pub fn validate_data_tree(root: &Path) -> Result<()> {
    let mut pending = vec![root.to_owned()];
    let mut count = 0;
    while let Some(path) = pending.pop() {
        count += 1;
        ensure!(
            count <= 1_000_000,
            "sandbox tree exceeds one million entries"
        );
        let metadata = std::fs::symlink_metadata(&path)?;
        ensure!(
            !crate::storage::is_link(&metadata) && (metadata.is_dir() || metadata.is_file()),
            "sandbox data contains an alias or special file: {}",
            path.display()
        );
        if metadata.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                ensure!(
                    metadata.nlink() == 1,
                    "sandbox data contains a hard link: {}",
                    path.display()
                );
            }
            #[cfg(windows)]
            windows::validate_single_link(&path)?;
        } else {
            for entry in std::fs::read_dir(path)? {
                pending.push(entry?.path());
            }
        }
    }
    Ok(())
}

pub(crate) fn graphics_cache(root: &Path, policy: &mut Policy) -> Result<Option<PathBuf>> {
    if !policy.desktop {
        return Ok(None);
    }
    #[cfg(target_os = "linux")]
    {
        let path =
            crate::storage::directory(root, ".enderpin/client/cache/graphics")?.canonicalize()?;
        policy.extra.push(FolderGrant {
            path: path.clone(),
            write: policy.permissions.graphics_cache,
        });
        Ok(Some(path))
    }
    #[cfg(target_os = "macos")]
    if policy.permissions.graphics_cache {
        let output = Command::new("/usr/bin/getconf")
            .env_clear()
            .arg("DARWIN_USER_CACHE_DIR")
            .output()?;
        ensure!(output.status.success(), "cannot resolve macOS user cache");
        let path = PathBuf::from(String::from_utf8(output.stdout)?.trim());
        ensure!(path.is_absolute(), "macOS returned an invalid cache root");
        let path = crate::storage::directory(
            &path.canonicalize()?,
            "net.java.openjdk.java/com.apple.metal",
        )?
        .canonicalize()?;
        policy.extra.push(FolderGrant { path, write: true });
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = root;
        Ok(None)
    }
}

pub fn command(java: &Path, policy: &Policy) -> Result<Command> {
    policy.validate(java)?;
    let mut command = platform_command(java, policy)?;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    environment(&mut command, policy);
    Ok(command)
}

/// Explicit opt-out: a direct JVM process with the same clean launch environment.
#[cfg(not(windows))]
pub(crate) fn unconfined_command(java: &Path, policy: &Policy) -> Command {
    let mut command = Command::new(java);
    command.env_clear();
    for key in [
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "XAUTHORITY",
        "DBUS_SESSION_BUS_ADDRESS",
        "PULSE_SERVER",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    environment(&mut command, policy);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
}

pub(crate) fn environment(command: &mut Command, policy: &Policy) {
    let game = crate::storage::java_path(&policy.game);
    let temporary = crate::storage::java_path(&policy.temporary);
    let java = crate::storage::java_path(&policy.java_home);
    command
        .current_dir(&game)
        .env("HOME", &game)
        .env("USERPROFILE", &game)
        .env("TMPDIR", &temporary)
        .env("TMP", &temporary)
        .env("TEMP", &temporary)
        .env("JAVA_HOME", &java)
        .env("PATH", java.join("bin"))
        .env("LANG", "en_US.UTF-8");
}

#[cfg(target_os = "macos")]
fn platform_command(java: &Path, policy: &Policy) -> Result<Command> {
    ensure!(
        Path::new("/usr/bin/sandbox-exec").is_file(),
        "macOS sandbox-exec is unavailable; refusing unisolated launch"
    );
    let mut profile = String::from(include_str!("sandbox/macos.sb"));
    if policy.desktop {
        profile.push_str(include_str!("sandbox/macos-desktop.sb"));
        if policy.permissions.desktop_integration {
            profile.push_str(include_str!("sandbox/macos-integration.sb"));
        }
        if policy.permissions.audio {
            profile.push_str(include_str!("sandbox/macos-audio.sb"));
        }
        if policy.permissions.microphone == Some(true) {
            profile.push_str("\n(allow device-microphone)\n");
        }
        if policy.permissions.clipboard == Some(true) {
            profile.push_str("\n(allow mach-lookup (global-name \"com.apple.pasteboard.1\"))\n");
        }
    }
    if policy.network {
        profile.push_str(include_str!("sandbox/macos-network.sb"));
    }
    let mut command = Command::new("/usr/bin/sandbox-exec");
    command.env_clear();
    for (index, grant) in policy.grants().iter().enumerate() {
        let path = &grant.path;
        let operations = if grant.write {
            "file-read* file-write*"
        } else {
            "file-read*"
        };
        profile.push_str(&format!(
            "\n(allow {operations} (subpath (param \"FILE{index}\")))\n"
        ));
        command.arg("-D").arg(format!(
            "FILE{index}={}",
            path.to_str().context("non-UTF8 sandbox path")?
        ));
    }
    for (index, path) in policy.read_only_game()?.iter().enumerate() {
        profile.push_str(&format!(
            "\n(deny file-write* (subpath (param \"DENY{index}\")))\n"
        ));
        command.arg("-D").arg(format!(
            "DENY{index}={}",
            path.to_str().context("non-UTF8 sandbox path")?
        ));
    }
    for (name, path) in [
        ("GAME", &policy.game),
        ("TMP", &policy.temporary),
        ("JAVA", &policy.java_home),
    ] {
        command.arg("-D").arg(format!(
            "{name}={}",
            path.to_str().context("non-UTF8 sandbox path")?
        ));
    }
    command.arg("-p").arg(profile).arg(java);
    Ok(command)
}

#[cfg(target_os = "linux")]
fn platform_command(java: &Path, policy: &Policy) -> Result<Command> {
    linux::command(java, policy)
}

#[cfg(windows)]
fn platform_command(_java: &Path, _policy: &Policy) -> Result<Command> {
    anyhow::bail!(
        "Windows needs the native AppContainer process API; use launch::start or sandbox::windows::output"
    )
}
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn platform_command(_java: &Path, _policy: &Policy) -> Result<Command> {
    anyhow::bail!("sandbox backend is not implemented on this OS; refusing unisolated launch")
}
