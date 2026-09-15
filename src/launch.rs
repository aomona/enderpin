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
    /// Disable OS isolation only when explicitly requested for a vanilla target.
    pub sandbox: bool,
    pub offline: bool,
    pub network: bool,
    pub accept_eula: bool,
    pub memory_mib: u32,
    pub connect: Option<String>,
    pub port: Option<u16>,
}
impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            sandbox: true,
            offline: false,
            network: true,
            accept_eula: false,
            memory_mib: 2048,
            connect: None,
            port: None,
        }
    }
}

/// Owns the target lock until its JVM exits. Dropping it kills and reaps the child.
pub struct RunningGame {
    child: Child,
    _target: File,
    _temporary: tempfile::TempDir,
    _bridges: Option<tempfile::TempDir>,
    _broker: Option<crate::auth::broker::channel::BrokerGuard>,
    relay_threads: Vec<std::thread::JoinHandle<()>>,
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
        #[cfg(windows)]
        let _ = self.kill();
        // Game stdout/err close after child exit. Draining also drops the narrator
        // sender, stopping host speech instead of leaving it alive after shutdown.
        for thread in self.relay_threads.drain(..) {
            let _ = thread.join();
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
        options.sandbox || (workspace.manifest.target(side).loader == "vanilla" && options.network),
        "unconfined launch is only supported for vanilla targets; network denial requires the sandbox"
    );
    ensure!(
        (256..=1_048_576).contains(&options.memory_mib),
        "memory must be between 256 and 1048576 MiB"
    );
    ensure!(
        options
            .port
            .is_none_or(|port| side == Side::Server && port > 0),
        "port requires a server and must be between 1 and 65535"
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
    let plan = sandbox::permissions::plan_locked(&workspace, side)?;
    ensure!(
        side != Side::Client
            || if demo {
                session.is_none()
            } else {
                !plan.effective.account_authentication || session.is_some()
            },
        "client requires a current account session when authentication is enabled"
    );
    if options.sandbox {
        let approval = sandbox::permissions::Local::load(&workspace, side)?;
        ensure!(
            approval.approved.as_deref() == Some(&plan.fingerprint()?),
            "sandbox permissions changed or are unapproved; run permissions show, then permissions approve <fingerprint>"
        );
    }
    if !options.network {
        ensure!(
            !plan
                .packages
                .iter()
                .flat_map(|p| p.embedded.iter().chain(&p.repository))
                .any(|r| r.permission == sandbox::permissions::Capability::Network),
            "a mod requires network access but --no-network was requested"
        );
    }
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
    for directory in &plan.effective.read_only {
        storage::directory(&game, directory.name())?;
    }
    let mut extra: Vec<_> = plan.folders.values().cloned().collect();
    let bridge_files = if side == Side::Client
        && (workspace.manifest.client.loader != "vanilla"
            || (!demo && plan.effective.account_authentication)
            || plan.effective.narrator)
    {
        let root = storage::directory(&workspace.root, ".enderpin/client/launch")?;
        let files = tempfile::tempdir_in(root)?;
        crate::bridges::write_files(files.path())?;
        Some(files)
    } else {
        None
    };
    let broker = if side == Side::Client && !demo && plan.effective.account_authentication {
        crate::bridges::clear_profile_keys(&game)?;
        Some(crate::bridges::broker(
            &runtime,
            &workspace.lockfile.runtimes[&side],
            session.context("missing session")?,
        )?)
    } else {
        None
    };
    let mut narrator_random = [0; 32];
    getrandom::fill(&mut narrator_random)
        .map_err(|_| anyhow::anyhow!("secure randomness unavailable"))?;
    let narrator_token: String = narrator_random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let narrator = if side == Side::Client && plan.effective.narrator {
        Some(std::sync::Arc::new(
            crate::narrator::NarratorBroker::start(narrator_token.clone())
                .map_err(anyhow::Error::msg)?,
        ))
    } else {
        None
    };
    let assets = if side == Side::Client {
        progress("Preparing local assets and skin cache");
        let caches =
            storage::directory(&workspace.root, ".enderpin/client/cache")?.canonicalize()?;
        let assets = runtime.game_assets(&caches)?.canonicalize()?;
        let skins = storage::directory(&assets, "skins")?.canonicalize()?;
        extra.push(sandbox::permissions::FolderGrant {
            path: assets.clone(),
            write: false,
        });
        extra.push(sandbox::permissions::FolderGrant {
            path: skins,
            write: plan.effective.skin_cache,
        });
        Some(assets)
    } else {
        None
    };
    for grant in &extra {
        ensure!(
            !grant.path.starts_with(&java_home) && !java_home.starts_with(&grant.path),
            "folder grant overlaps the Java runtime"
        );
        for path in &runtime.readonly_roots {
            ensure!(
                !grant.path.starts_with(path) && !path.starts_with(&grant.path),
                "folder grant overlaps the runtime cache"
            );
        }
    }
    let mut policy = Policy {
        game,
        temporary: temporary.path().canonicalize()?,
        java_home,
        readonly: runtime.readonly_roots.clone(),
        network: options.network && plan.effective.network,
        desktop: side == Side::Client,
        permissions: plan.effective,
        extra,
    };
    if let Some(files) = &bridge_files {
        policy.readonly.push(files.path().canonicalize()?);
    }
    ensure!(
        options.connect.is_none() || (policy.network && broker.is_some()),
        "--connect requires approved network and account authentication access"
    );
    let graphics = sandbox::graphics_cache(&workspace.root, &mut policy)?;
    #[cfg(not(windows))]
    let mut command = if options.sandbox {
        sandbox::command(&runtime.java, &policy)?
    } else {
        sandbox::unconfined_command(&runtime.java, &policy)
    };
    #[cfg(windows)]
    let mut command = sandbox::windows::configuration(&runtime.java, &policy)?;
    if let Some(graphics) = graphics {
        command
            .env("XDG_CACHE_HOME", &graphics)
            .env("MESA_SHADER_CACHE_DIR", &graphics)
            .env("__GL_SHADER_DISK_CACHE_PATH", &graphics)
            .env(
                "MESA_SHADER_CACHE_DISABLE",
                if policy.permissions.graphics_cache {
                    "false"
                } else {
                    "true"
                },
            )
            .env(
                "__GL_SHADER_DISK_CACHE",
                if policy.permissions.graphics_cache {
                    "1"
                } else {
                    "0"
                },
            );
    }
    #[cfg(target_os = "macos")]
    inherit_target_lock(&mut command, &target)?;
    let classpath = std::env::join_paths(
        bridge_files
            .iter()
            .map(|files| storage::java_path(&files.path().join("narrator-bridge.jar")))
            .chain(runtime.classpath.iter().map(|p| storage::java_path(p))),
    )?
    .into_string()
    .map_err(|_| anyhow::anyhow!("non-UTF8 classpath"))?;
    let mut args = vec![
        format!("-Xmx{}M", options.memory_mib),
        format!(
            "-Djava.io.tmpdir={}",
            storage::java_path(&policy.temporary).display()
        ),
        format!("-Duser.home={}", storage::java_path(&policy.game).display()),
        "-XX:-UsePerfData".into(),
    ];
    if workspace.manifest.target(side).loader == "fabric" {
        args.push(format!(
            "-Dfabric.gameJarPath={}",
            storage::java_path(&runtime.game_jar).display()
        ));
    }
    if let Some(files) = &bridge_files {
        args.push(format!(
            "-Dmonalauncher.narrator.enabled={}",
            narrator.is_some()
        ));
        args.push(format!("-Dmonalauncher.narrator.token={narrator_token}"));
        if let Some(broker) = &broker {
            // The JVM splits -javaagent at the first '='. Keep user-selected
            // workspace paths out of the agent filename portion.
            args.push(format!(
                "-javaagent:{}={}",
                std::path::Path::new("../launch")
                    .join(
                        files
                            .path()
                            .file_name()
                            .context("bridge directory missing")?
                    )
                    .join("auth-bridge.jar")
                    .display(),
                storage::java_path(&crate::bridges::native(files.path())).display()
            ));
            #[cfg(unix)]
            broker.configure(&mut command)?;
            #[cfg(windows)]
            {
                use std::os::windows::io::AsRawHandle;
                args.push(format!(
                    "-Dmonalauncher.auth.handle={}",
                    broker.child_handle().as_raw_handle() as usize
                ));
            }
        }
    }
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
            if workspace.manifest.server.loader == "vanilla" {
                "net.minecraft.server.Main"
            } else {
                "net.fabricmc.loader.impl.launch.knot.KnotServer"
            }
            .into(),
            "nogui".into(),
        ]);
        if let Some(port) = options.port {
            args.extend(["--port".into(), port.to_string()]);
        }
    } else {
        let game_assets = assets.context("client assets missing")?;
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
                // UUID v3 (MD5) of "OfflinePlayer:Player", as used by Minecraft.
                session.map_or("a01e3843e5213998958af459800e4d11".into(), |s| {
                    s.uuid.clone()
                }),
            ),
            (
                "auth_access_token",
                if broker.is_some() {
                    crate::auth::broker::protocol::GAME_TOKEN.into()
                } else {
                    "0".into()
                },
            ),
            ("user_type", "msa".into()),
            ("user_properties", "{}".into()),
            ("version_name", workspace.manifest.minecraft.clone()),
            ("version_type", "release".into()),
            (
                "game_directory",
                storage::java_path(&policy.game)
                    .to_string_lossy()
                    .into_owned(),
            ),
            (
                "assets_root",
                storage::java_path(&game_assets)
                    .to_string_lossy()
                    .into_owned(),
            ),
            (
                "assets_index_name",
                runtime.asset_index.clone().context("asset index missing")?,
            ),
            (
                "natives_directory",
                storage::java_path(&policy.temporary)
                    .to_string_lossy()
                    .into_owned(),
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
                storage::java_path(&runtime.readonly_roots[0])
                    .to_string_lossy()
                    .into_owned(),
            ),
        ]);
        for argument in crate::runtime::arguments(&runtime.metadata.arguments.jvm, &features)? {
            args.push(expand(&argument, &values)?);
        }
        if workspace.manifest.client.loader == "vanilla" {
            ensure!(
                runtime.metadata.main_class == "net.minecraft.client.main.Main",
                "unsupported vanilla client main class"
            );
            args.push(runtime.metadata.main_class.clone());
        } else {
            args.push("net.fabricmc.loader.impl.launch.knot.KnotClient".into());
        }
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
    // Arguments contain only a placeholder token. Account credentials and signing
    // keys are held by the host broker, never by this file or the game process.
    let mut argument_file = tempfile::NamedTempFile::new_in(&policy.temporary)?;
    for argument in &args {
        writeln!(argument_file, "{}", quote_argument(argument)?)?;
    }
    argument_file.flush()?;
    command
        .arg(format!(
            "@{}",
            storage::java_path(argument_file.path()).display()
        ))
        .stdin(if side == Side::Server {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Retained inside the launch's TempDir until Java has read it and the process exits.
    argument_file.keep().map_err(|error| error.error)?;
    #[cfg(target_os = "linux")]
    let (mut child, parent_lifetime, parent_thread) = {
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
            .context("game could not start")?;
        (child, Some(parent_lifetime), Some(thread))
    };
    #[cfg(not(any(target_os = "linux", windows)))]
    let mut child = command.spawn().context("game could not start")?;
    #[cfg(windows)]
    let mut child = sandbox::windows::spawn_configured(
        &command,
        &policy,
        &broker
            .iter()
            .map(|broker| broker.child_handle())
            .collect::<Vec<_>>(),
        options.sandbox,
    )?;
    let mut relay_threads = vec![];
    if let Some(stdout) = child.stdout.take() {
        relay_threads.push(relay(stdout, false, narrator.clone()));
    }
    if let Some(stderr) = child.stderr.take() {
        relay_threads.push(relay(stderr, true, narrator));
    }
    drop(workspace);
    Ok(RunningGame {
        child,
        _target: target,
        _temporary: temporary,
        _bridges: bridge_files,
        _broker: broker.map(|broker| broker.into_guard()),
        relay_threads,
        #[cfg(target_os = "linux")]
        parent_lifetime,
        #[cfg(target_os = "linux")]
        parent_thread,
    })
}

fn relay(
    reader: impl std::io::Read + Send + 'static,
    stderr: bool,
    narrator: Option<std::sync::Arc<crate::narrator::NarratorBroker>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        use std::io::{BufRead, Read};
        let mut reader = std::io::BufReader::new(reader);
        let mut discarded = false;
        loop {
            let mut bytes = vec![];
            let Ok(count) = reader.by_ref().take(16385).read_until(b'\n', &mut bytes) else {
                break;
            };
            if count == 0 {
                break;
            }
            let ended = bytes.ends_with(b"\n") || count < 16385;
            // ponytail: cap one log line at 16 KiB; stream structured logs if larger lines are needed.
            if discarded || count > 16384 {
                discarded = !ended;
                continue;
            }
            let line = String::from_utf8_lossy(&bytes);
            if narrator
                .as_ref()
                .is_some_and(|broker| broker.handle_line(line.trim_end_matches(['\r', '\n'])))
            {
                continue;
            }
            if line.contains("MONALAUNCHER_NARRATOR\t") {
                continue;
            }
            if stderr {
                let _ = std::io::stderr().lock().write_all(&bytes);
            } else {
                let _ = std::io::stdout().lock().write_all(&bytes);
            }
        }
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
pub(crate) fn inherit_target_lock(
    command: &mut std::process::Command,
    target: &File,
) -> Result<()> {
    use nix::libc;
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    };
    // The broker replaces fd 3 in the child. Never put the retained lock there.
    // SAFETY: fcntl creates a new owned descriptor for the live target file.
    let duplicate = unsafe { libc::fcntl(target.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 4) };
    if duplicate < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let lease = unsafe { File::from_raw_fd(duplicate) };
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
    #[test]
    #[ignore = "requires ENDERPIN_TEST_JAVA_HOME"]
    fn explicit_unconfined_jvm_can_write_outside_game() -> Result<()> {
        let java_home = std::path::PathBuf::from(
            std::env::var_os("ENDERPIN_TEST_JAVA_HOME").context("set ENDERPIN_TEST_JAVA_HOME")?,
        )
        .canonicalize()?;
        let root = tempfile::tempdir()?;
        let game = storage::directory(root.path(), "game")?.canonicalize()?;
        let temporary = storage::directory(root.path(), "tmp")?.canonicalize()?;
        let outside = root.path().join("outside.txt");
        std::fs::write(
            game.join("Unconfined.java"),
            "class Unconfined { public static void main(String[] args) throws Exception { java.nio.file.Files.writeString(java.nio.file.Path.of(args[0]), \"allowed\"); } }",
        )?;
        let policy = Policy {
            game: game.clone(),
            temporary,
            java_home: java_home.clone(),
            readonly: vec![],
            network: true,
            desktop: true,
            permissions: Default::default(),
            extra: vec![],
        };
        let java = java_home.join(if cfg!(windows) {
            "bin/java.exe"
        } else {
            "bin/java"
        });
        #[cfg(not(windows))]
        let mut command = sandbox::unconfined_command(&java, &policy);
        #[cfg(windows)]
        let mut command = sandbox::windows::configuration(&java, &policy)?;
        command
            .arg("-XX:-UsePerfData")
            .arg(storage::java_path(&game.join("Unconfined.java")))
            .arg(storage::java_path(&outside));
        #[cfg(not(windows))]
        let status = command.status()?;
        #[cfg(windows)]
        let status = sandbox::windows::spawn_configured(&command, &policy, &[], false)?.wait()?;
        assert!(status.success());
        assert_eq!(std::fs::read_to_string(outside)?, "allowed");
        Ok(())
    }

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
            permissions: Default::default(),
            extra: vec![],
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
