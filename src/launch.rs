#[cfg(windows)]
use crate::sandbox::windows::Process as Child;
use crate::{
    Side, SyncOptions, Workspace,
    runtime::PreparedRuntime,
    sandbox::{self, Policy},
    storage,
};
use anyhow::{Context, Result, ensure};
#[cfg(not(windows))]
use std::process::Child;
use std::{
    fs::File,
    io::Write,
    process::{ExitStatus, Stdio},
};

pub struct LaunchOptions {
    pub offline: bool,
    pub network: bool,
    pub accept_eula: bool,
    pub memory_mib: u32,
    pub connect: Option<String>,
}
impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            offline: false,
            network: true,
            accept_eula: false,
            memory_mib: 2048,
            connect: None,
        }
    }
}

/// Owns the target lock until its JVM exits. Dropping it kills and reaps the child.
pub struct RunningGame {
    child: Child,
    _target: File,
    _temporary: tempfile::TempDir,
    #[cfg(target_os = "linux")]
    parent_lifetime: Option<std::sync::mpsc::Sender<()>>,
    #[cfg(target_os = "linux")]
    parent_thread: Option<std::thread::JoinHandle<()>>,
}
impl RunningGame {
    pub fn id(&self) -> u32 {
        self.child.id()
    }
    pub fn console(&mut self, line: &str) -> Result<()> {
        ensure!(
            !line.contains(['\n', '\r', '\0']),
            "console input must be one line"
        );
        let input = self
            .child
            .stdin
            .as_mut()
            .context("console input is closed")?;
        writeln!(input, "{line}")?;
        input.flush()?;
        Ok(())
    }
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        Ok(self.child.try_wait()?)
    }
    pub fn kill(&mut self) -> Result<()> {
        #[cfg(unix)]
        {
            use nix::{
                sys::signal::{Signal, killpg},
                unistd::Pid,
            };
            match killpg(Pid::from_raw(self.child.id().try_into()?), Signal::SIGKILL) {
                Ok(()) | Err(nix::errno::Errno::ESRCH) => Ok(()),
                Err(error) => Err(error.into()),
            }
        }
        #[cfg(not(unix))]
        {
            Ok(self.child.kill()?)
        }
    }
    pub fn stop(&mut self, side: Side) -> Result<()> {
        if side == Side::Server {
            return self.console("stop");
        }
        #[cfg(unix)]
        {
            Ok(nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(self.child.id().try_into()?),
                nix::sys::signal::Signal::SIGTERM,
            )?)
        }
        #[cfg(not(unix))]
        {
            self.kill()
        }
    }
}
impl Drop for RunningGame {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.kill();
            let _ = self.child.wait();
        }
        #[cfg(target_os = "linux")]
        {
            self.parent_lifetime.take();
            if let Some(thread) = self.parent_thread.take() {
                let _ = thread.join();
            }
        }
    }
}

pub fn start(
    mut workspace: Workspace,
    side: Side,
    options: LaunchOptions,
    session: Option<&crate::auth::Session>,
    demo: bool,
    mut progress: impl FnMut(&str),
) -> Result<RunningGame> {
    ensure!(
        (256..=1_048_576).contains(&options.memory_mib),
        "memory must be between 256 and 1048576 MiB"
    );
    ensure!(
        side != Side::Client || demo != session.is_some(),
        "client needs either an authenticated session or explicit demo mode"
    );
    if let Some(server) = &options.connect {
        ensure!(
            side == Side::Client && !demo && options.network,
            "--connect requires an authenticated client with network access"
        );
        ensure!(
            !server.is_empty()
                && server.len() <= 300
                && server
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-_:[]".contains(&b)),
            "invalid server address"
        );
    }
    let game = storage::directory(&workspace.root, &format!(".enderpin/{}/game", side.name()))?
        .canonicalize()?;
    if side == Side::Server {
        let eula = storage::read_optional(&game, "eula.txt")?.unwrap_or_default();
        ensure!(
            options.accept_eula || eula.lines().any(|l| l.trim() == "eula=true"),
            "Minecraft EULA acceptance is required: https://aka.ms/MinecraftEULA ; review it, then run launch --target server --accept-eula"
        );
    }
    let mut runtimes = workspace.prepare_runtime(
        &[side],
        SyncOptions {
            locked: true,
            offline: options.offline,
            ..Default::default()
        },
        false,
        |_, message| progress(message),
    )?;
    let runtime: PreparedRuntime = runtimes.remove(&side).context("target runtime missing")?;
    let target = storage::target_lock(&workspace.root, side)?;
    if side == Side::Server && options.accept_eula {
        let path = storage::safe_path(&game, "eula.txt")?;
        let mut file = File::create(path)?;
        file.write_all(
            b"# Accepted explicitly through enderpin launch --accept-eula\neula=true\n",
        )?;
        file.sync_all()?;
    }
    let temp_root = storage::directory(&workspace.root, &format!(".enderpin/{}/tmp", side.name()))?;
    let temporary = tempfile::tempdir_in(temp_root)?;
    let java_home = runtime
        .java
        .parent()
        .and_then(|p| p.parent())
        .context("invalid Java path")?
        .canonicalize()?;
    let policy = Policy {
        game,
        temporary: temporary.path().canonicalize()?,
        java_home,
        readonly: runtime.readonly_roots.clone(),
        network: options.network,
        desktop: side == Side::Client,
    };
    #[cfg(not(windows))]
    let mut command = sandbox::command(&runtime.java, &policy)?;
    #[cfg(windows)]
    let mut command = sandbox::windows::configuration(&runtime.java, &policy)?;
    #[cfg(target_os = "macos")]
    inherit_target_lock(&mut command, &target)?;
    let classpath = std::env::join_paths(&runtime.classpath)?
        .into_string()
        .map_err(|_| anyhow::anyhow!("non-UTF8 classpath"))?;
    let mut args = vec![
        format!("-Xmx{}M", options.memory_mib),
        format!("-Djava.io.tmpdir={}", policy.temporary.display()),
        format!("-Duser.home={}", policy.game.display()),
        "-XX:-UsePerfData".into(),
        format!("-Dfabric.gameJarPath={}", runtime.game_jar.display()),
    ];
    args.extend(
        workspace.lockfile.runtimes[&side]
            .fabric_jvm
            .iter()
            .cloned(),
    );
    if side == Side::Server {
        args.extend([
            "-Djava.awt.headless=true".into(),
            "-cp".into(),
            classpath,
            "net.fabricmc.loader.impl.launch.knot.KnotServer".into(),
            "nogui".into(),
        ]);
    } else {
        progress("Preparing local assets and skin cache");
        let game_assets = runtime.game_assets(&policy.game)?;
        let features = std::collections::BTreeMap::from([
            ("is_demo_user".into(), demo),
            (
                "is_quick_play_multiplayer".into(),
                options.connect.is_some(),
            ),
        ]);
        let values = std::collections::BTreeMap::from([
            (
                "quickPlayMultiplayer",
                options.connect.clone().unwrap_or_default(),
            ),
            (
                "auth_player_name",
                session.map_or("Player".into(), |s| s.name.clone()),
            ),
            (
                "auth_uuid",
                session.map_or("00000000000000000000000000000000".into(), |s| {
                    s.uuid.clone()
                }),
            ),
            (
                "auth_access_token",
                session.map_or("0".into(), |s| s.access_token.clone()),
            ),
            ("user_type", "msa".into()),
            ("user_properties", "{}".into()),
            ("version_name", workspace.manifest.minecraft.clone()),
            ("version_type", "release".into()),
            ("game_directory", policy.game.to_string_lossy().into_owned()),
            ("assets_root", game_assets.to_string_lossy().into_owned()),
            (
                "assets_index_name",
                runtime.asset_index.clone().context("asset index missing")?,
            ),
            (
                "natives_directory",
                policy.temporary.to_string_lossy().into_owned(),
            ),
            ("launcher_name", "Enderpin".into()),
            ("launcher_version", env!("CARGO_PKG_VERSION").into()),
            ("classpath", classpath),
            ("clientid", "".into()),
            ("auth_xuid", "".into()),
            (
                "classpath_separator",
                if cfg!(windows) { ";" } else { ":" }.into(),
            ),
            (
                "library_directory",
                runtime.readonly_roots[0].to_string_lossy().into_owned(),
            ),
        ]);
        for argument in crate::runtime::arguments(&runtime.metadata.arguments.jvm, &features)? {
            args.push(expand(&argument, &values)?);
        }
        args.push("net.fabricmc.loader.impl.launch.knot.KnotClient".into());
        let game_arguments =
            crate::runtime::arguments(&runtime.metadata.arguments.game, &features)?;
        ensure!(
            options.connect.is_none()
                || game_arguments
                    .iter()
                    .any(|arg| arg == "--quickPlayMultiplayer"),
            "this Minecraft version does not support --connect; join through its multiplayer menu"
        );
        for argument in game_arguments {
            args.push(expand(&argument, &values)?);
        }
    }
    // A private JVM argument file avoids exposing the short-lived game token in process listings.
    // The game and its mods necessarily receive this token; Microsoft refresh credentials do not.
    let mut argument_file = tempfile::NamedTempFile::new_in(&policy.temporary)?;
    for argument in &args {
        writeln!(argument_file, "{}", quote_argument(argument)?)?;
    }
    argument_file.flush()?;
    command
        .arg(format!("@{}", argument_file.path().display()))
        .stdin(if side == Side::Server {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    // Retained inside the launch's TempDir until Java has read it and the process exits.
    argument_file.keep().map_err(|error| error.error)?;
    #[cfg(target_os = "linux")]
    let (child, parent_lifetime, parent_thread) = {
        let (result_sender, result_receiver) = std::sync::mpsc::sync_channel(1);
        let (parent_lifetime, receiver) = std::sync::mpsc::channel();
        // --die-with-parent follows the spawning Linux thread, including GUI workers.
        let thread = std::thread::spawn(move || {
            let result = command.spawn();
            let started = result.is_ok();
            if let Err(error) = result_sender.send(result) {
                if let Ok(mut child) = error.0 {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                return;
            }
            if started {
                let _ = receiver.recv();
            }
        });
        let child = result_receiver
            .recv()
            .context("sandbox parent thread failed")?
            .context("sandboxed game could not start")?;
        (child, Some(parent_lifetime), Some(thread))
    };
    #[cfg(not(any(target_os = "linux", windows)))]
    let child = command.spawn().context("sandboxed game could not start")?;
    #[cfg(windows)]
    let child = sandbox::windows::spawn(&command, &policy)?;
    drop(workspace);
    Ok(RunningGame {
        child,
        _target: target,
        _temporary: temporary,
        #[cfg(target_os = "linux")]
        parent_lifetime,
        #[cfg(target_os = "linux")]
        parent_thread,
    })
}

fn expand(argument: &str, values: &std::collections::BTreeMap<&str, String>) -> Result<String> {
    let mut rest = argument;
    let mut output = String::new();
    while let Some(index) = rest.find("${") {
        output.push_str(&rest[..index]);
        rest = &rest[index + 2..];
        let end = rest
            .find('}')
            .context("invalid Minecraft argument placeholder")?;
        output.push_str(
            values
                .get(&rest[..end])
                .context("unsupported Minecraft argument placeholder")?,
        );
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    Ok(output)
}
fn quote_argument(argument: &str) -> Result<String> {
    ensure!(
        !argument.contains(['\0', '\n', '\r']),
        "invalid JVM argument"
    );
    Ok(format!(
        "\"{}\"",
        argument.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn inherit_target_lock(command: &mut std::process::Command, target: &File) -> Result<()> {
    use nix::libc;
    use std::os::{fd::AsRawFd, unix::process::CommandExt};
    let lease = target.try_clone()?;
    // Keep the target locked in the JVM even if the macOS CLI is forcibly killed.
    // SAFETY: the closure owns the descriptor and uses only async-signal-safe fcntl;
    // flags change in the child, and no unrelated descriptor becomes inheritable.
    unsafe {
        command.pre_exec(move || {
            let fd = lease.as_raw_fd();
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires ENDERPIN_TEST_JAVA_HOME and macOS sandbox-exec"]
    fn jvm_retains_target_lock_if_launcher_releases_it() -> Result<()> {
        use std::io::BufRead;
        let java_home = std::path::PathBuf::from(
            std::env::var_os("ENDERPIN_TEST_JAVA_HOME").context("set ENDERPIN_TEST_JAVA_HOME")?,
        )
        .canonicalize()?;
        let root = tempfile::tempdir()?;
        let game = storage::directory(root.path(), "game")?.canonicalize()?;
        let temporary = storage::directory(root.path(), "tmp")?.canonicalize()?;
        std::fs::write(
            game.join("Hold.java"),
            "class Hold { public static void main(String[] args) throws Exception { System.out.println(\"ready\"); Thread.sleep(10000); } }",
        )?;
        let lease = storage::target_lock(root.path(), Side::Server)?;
        let policy = Policy {
            game: game.clone(),
            temporary,
            java_home: java_home.clone(),
            readonly: vec![],
            network: false,
            desktop: false,
        };
        let mut command = sandbox::command(&java_home.join("bin/java"), &policy)?;
        inherit_target_lock(&mut command, &lease)?;
        command
            .arg("-XX:-UsePerfData")
            .arg(game.join("Hold.java"))
            .stdout(Stdio::piped());
        let mut child = command.spawn()?;
        drop(command);
        drop(lease);
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.take().context("missing stdout")?)
            .read_line(&mut line)?;
        let retained = storage::target_lock(root.path(), Side::Server).is_err();
        let _ = child.kill();
        child.wait()?;
        assert_eq!(line.trim(), "ready");
        assert!(
            retained,
            "JVM must retain the target lock after parent descriptors close"
        );
        storage::target_lock(root.path(), Side::Server)?;
        Ok(())
    }
    #[test]
    fn argfile_quoting_does_not_reinterpret_values_as_templates() -> Result<()> {
        let values = std::collections::BTreeMap::from([("path", "a ${literal} # \\\"b".into())]);
        assert_eq!(expand("${path}/x", &values)?, "a ${literal} # \\\"b/x");
        assert!(expand("${unknown}", &values).is_err());
        assert!(quote_argument("bad\nargument").is_err());
        assert_eq!(quote_argument("a\\b\"c")?, "\"a\\\\b\\\"c\"");
        Ok(())
    }
}
