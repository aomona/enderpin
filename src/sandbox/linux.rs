use super::Policy;
use anyhow::{Context, Result, ensure};
use nix::libc;
use std::{
    fs,
    io::Write,
    os::{
        fd::AsRawFd,
        unix::{fs::FileTypeExt, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::Command,
};

#[allow(unsafe_code)]
pub(super) fn command(java: &Path, policy: &Policy, ipc: bool) -> Result<Command> {
    ensure!(
        Path::new("/usr/bin/bwrap").is_file(),
        "install bubblewrap (/usr/bin/bwrap); refusing unisolated launch"
    );
    let mut command = Command::new("/usr/bin/bwrap");
    command.env_clear().args([
        "--unshare-all",
        "--unshare-user",
        "--disable-userns",
        "--die-with-parent",
        "--new-session",
        "--cap-drop",
        "ALL",
    ]);
    if policy.network {
        command.arg("--share-net");
        for path in [
            "/etc/resolv.conf",
            "/etc/hosts",
            "/etc/nsswitch.conf",
            "/etc/ssl/certs",
        ] {
            ro_if_present(&mut command, path);
        }
    }
    if ipc {
        command.args(["--preserve-fds", "1"]);
    }
    for path in [
        "/usr",
        "/bin",
        "/lib",
        "/lib64",
        "/etc/ld.so.cache",
        "/etc/alternatives",
        "/etc/fonts",
        "/etc/localtime",
    ] {
        ro_if_present(&mut command, path);
    }
    command.args([
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--perms",
        "0700",
        "--dir",
        "/run/enderpin",
    ]);
    for grant in policy.grants() {
        command
            .arg(if grant.write { "--bind" } else { "--ro-bind" })
            .arg(&grant.path)
            .arg(&grant.path);
    }
    for path in policy.read_only_game()? {
        command.arg("--ro-bind").arg(&path).arg(&path);
    }
    command
        .env("XDG_RUNTIME_DIR", "/run/enderpin")
        .env("XDG_CACHE_HOME", &policy.temporary)
        .env("MESA_SHADER_CACHE_DISABLE", "true")
        .env("__GL_SHADER_DISK_CACHE", "0");
    if policy.desktop {
        desktop(&mut command, policy.permissions.audio)?;
    }
    // bwrap's synthetic root/tmp are otherwise writable. Remount their own mounts
    // read-only; the explicit game/tmp bind mounts remain independently writable.
    for path in ["/tmp", "/dev", "/proc", "/"] {
        command.args(["--remount-ro", path]);
    }
    // Open descriptor is captured by pre_exec, staying alive until spawn even if the
    // temporary filename is removed. CLOEXEC changes only in the forked child.
    let mut filter = tempfile::tempfile()?;
    filter.write_all(&seccomp(policy.network)?)?;
    use std::io::Seek;
    filter.rewind()?;
    let fd = filter.as_raw_fd();
    command
        .arg("--seccomp")
        .arg(fd.to_string())
        .arg("--")
        .arg(java);
    // SAFETY: the closure only calls async-signal-safe fcntl. It owns a live File,
    // whose descriptor cannot be reused before exec. Parent descriptor flags stay unchanged.
    unsafe {
        command.pre_exec(move || {
            let fd = filter.as_raw_fd();
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(command)
}

fn ro_if_present(command: &mut Command, path: &str) {
    if Path::new(path).exists() {
        command.args(["--ro-bind", path, path]);
    }
}
fn bind_socket(command: &mut Command, source: &Path, destination: &Path) -> Result<()> {
    ensure!(
        source.is_absolute() && source.metadata()?.file_type().is_socket(),
        "desktop endpoint must be an absolute local socket"
    );
    command
        .arg("--ro-bind")
        .arg(source.canonicalize()?)
        .arg(destination);
    Ok(())
}
fn desktop(command: &mut Command, audio: bool) -> Result<()> {
    ensure!(
        std::env::var_os("WAYLAND_SOCKET").is_none(),
        "inherited WAYLAND_SOCKET is unsupported; use a named display socket"
    );
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|value| value == "wayland")
    {
        let display = std::env::var_os("WAYLAND_DISPLAY").unwrap_or_else(|| "wayland-0".into());
        let name = PathBuf::from(display);
        let socket = if name.is_absolute() {
            name
        } else {
            ensure!(
                name.components().count() == 1
                    && matches!(
                        name.components().next(),
                        Some(std::path::Component::Normal(_))
                    ),
                "invalid Wayland socket name"
            );
            runtime
                .as_ref()
                .context("XDG_RUNTIME_DIR is required")?
                .join(name)
        };
        bind_socket(command, &socket, Path::new("/run/enderpin/wayland-0"))?;
        command
            .env("WAYLAND_DISPLAY", "wayland-0")
            .env("XDG_SESSION_TYPE", "wayland");
    } else {
        let display =
            std::env::var("DISPLAY").context("client requires a Wayland or local X11 display")?;
        let number = display
            .strip_prefix(':')
            .context("remote X11 is unsupported")?;
        let parts: Vec<_> = number.split('.').collect();
        ensure!(
            parts.len() <= 2
                && parts
                    .iter()
                    .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())),
            "invalid X11 display"
        );
        let socket = PathBuf::from(format!("/tmp/.X11-unix/X{}", parts[0]));
        bind_socket(command, &socket, &socket)?;
        let authority = std::env::var_os("XAUTHORITY")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".Xauthority"))
            })
            .context("XAUTHORITY is required")?;
        ensure!(
            authority.is_absolute() && authority.is_file(),
            "XAUTHORITY must be an absolute file"
        );
        command
            .arg("--ro-bind")
            .arg(authority.canonicalize()?)
            .arg("/run/enderpin/Xauthority")
            .env("DISPLAY", display)
            .env("XAUTHORITY", "/run/enderpin/Xauthority")
            .env("XDG_SESSION_TYPE", "x11");
    }
    let pulse = match std::env::var("PULSE_SERVER") {
        Ok(server) => Some(PathBuf::from(
            server
                .strip_prefix("unix:")
                .context("audio requires a local unix: PULSE_SERVER")?,
        )),
        Err(_) => runtime
            .map(|p| p.join("pulse/native"))
            .filter(|p| p.exists()),
    };
    if audio && let Some(pulse) = pulse {
        bind_socket(command, &pulse, Path::new("/run/enderpin/pulse"))?;
    }
    command
        .env("PULSE_SERVER", "unix:/run/enderpin/pulse")
        .env("ALSOFT_DRIVERS", "pulse");
    if let Ok(entries) = fs::read_dir("/dev/dri") {
        for entry in entries {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with("renderD")
                && entry.metadata()?.file_type().is_char_device()
            {
                command
                    .arg("--dev-bind")
                    .arg(entry.path())
                    .arg(entry.path());
            }
        }
    }
    Ok(())
}

// Classic BPF inspects the native ABI and denies namespace escape, privileged
// introspection, unusual socket families, and terminal input injection.
fn seccomp(network: bool) -> Result<Vec<u8>> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => 0xc000003e,
        "aarch64" => 0xc00000b7,
        _ => anyhow::bail!("Linux sandbox supports x86_64 and aarch64"),
    };
    let mut instructions: Vec<(u16, u8, u8, u32)> = vec![
        (0x20, 0, 0, 4),
        (0x15, 1, 0, arch),
        (6, 0, 0, 0x80000000),
        (0x20, 0, 0, 0),
        (0x45, 0, 1, 0x40000000),
        (6, 0, 0, 0x80000000),
    ];
    let deny = 0x50000 | libc::EPERM as u32;
    for syscall in [
        libc::SYS_unshare,
        libc::SYS_setns,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_bpf,
        libc::SYS_perf_event_open,
        libc::SYS_keyctl,
        libc::SYS_add_key,
        libc::SYS_request_key,
        libc::SYS_userfaultfd,
        libc::SYS_open_by_handle_at,
        libc::SYS_move_mount,
        libc::SYS_fsopen,
        libc::SYS_fsconfig,
        libc::SYS_fsmount,
        libc::SYS_fspick,
        libc::SYS_mount_setattr,
        libc::SYS_reboot,
        libc::SYS_kexec_load,
    ] {
        instructions.extend([(0x15, 0, 1, syscall as u32), (6, 0, 0, deny)]);
    }
    let namespaces = libc::CLONE_NEWUSER
        | libc::CLONE_NEWNS
        | libc::CLONE_NEWNET
        | libc::CLONE_NEWPID
        | libc::CLONE_NEWIPC
        | libc::CLONE_NEWUTS
        | libc::CLONE_NEWCGROUP;
    instructions.extend([
        (0x15, 0, 1, libc::SYS_clone3 as u32),
        (6, 0, 0, 0x50000 | libc::ENOSYS as u32),
        (0x15, 0, 3, libc::SYS_clone as u32),
        (0x20, 0, 0, 16),
        (0x45, 0, 1, namespaces as u32),
        (6, 0, 0, deny),
        (0x20, 0, 0, 0),
    ]);
    let families: &[i32] = if network {
        &[libc::AF_UNIX, libc::AF_INET, libc::AF_INET6]
    } else {
        &[libc::AF_UNIX]
    };
    for syscall in [libc::SYS_socket, libc::SYS_socketpair] {
        instructions.extend([
            (0x15, 0, (families.len() + 2) as u8, syscall as u32),
            (0x20, 0, 0, 16),
        ]);
        for (index, family) in families.iter().enumerate() {
            instructions.push((0x15, (families.len() - index) as u8, 0, *family as u32));
        }
        instructions.extend([
            (6, 0, 0, 0x50000 | libc::EAFNOSUPPORT as u32),
            (0x20, 0, 0, 0),
        ]);
    }
    instructions.extend([
        (0x15, 0, 5, libc::SYS_ioctl as u32),
        (0x20, 0, 0, 24),
        (0x15, 0, 1, libc::TIOCSTI as u32),
        (6, 0, 0, deny),
        (0x15, 0, 1, 0x541c),
        (6, 0, 0, deny),
        (6, 0, 0, 0x7fff0000),
    ]);
    let mut bytes = vec![];
    for (op, yes, no, value) in instructions {
        bytes.extend(op.to_ne_bytes());
        bytes.extend([yes, no]);
        bytes.extend(value.to_ne_bytes());
    }
    Ok(bytes)
}
