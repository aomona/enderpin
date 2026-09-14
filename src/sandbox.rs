//! OS isolation. Unsupported or unavailable backends fail closed.
#[cfg(target_os = "macos")]
use anyhow::Context;
use anyhow::{Result, ensure};
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
}

impl Policy {
    pub fn validate(&self, java: &Path) -> Result<()> {
        for path in [&self.game, &self.temporary, &self.java_home]
            .into_iter()
            .chain(self.readonly.iter())
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
        for writable in [&self.game, &self.temporary] {
            for readonly in self.readonly.iter().chain(std::iter::once(&self.java_home)) {
                ensure!(
                    !readonly.starts_with(writable) && !writable.starts_with(readonly),
                    "sandbox writable and runtime paths overlap"
                );
            }
        }
        Ok(())
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

pub(crate) fn environment(command: &mut Command, policy: &Policy) {
    command
        .current_dir(&policy.game)
        .env("HOME", &policy.game)
        .env("USERPROFILE", &policy.game)
        .env("TMPDIR", &policy.temporary)
        .env("TMP", &policy.temporary)
        .env("TEMP", &policy.temporary)
        .env("JAVA_HOME", &policy.java_home)
        .env("PATH", policy.java_home.join("bin"))
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
    }
    if policy.network {
        profile.push_str(include_str!("sandbox/macos-network.sb"));
    }
    let mut command = Command::new("/usr/bin/sandbox-exec");
    command.env_clear();
    for (index, path) in policy
        .readonly
        .iter()
        .chain(std::iter::once(&policy.java_home))
        .enumerate()
    {
        profile.push_str(&format!(
            "\n(allow file-read* (subpath (param \"RO{index}\")))\n"
        ));
        command.arg("-D").arg(format!(
            "RO{index}={}",
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
