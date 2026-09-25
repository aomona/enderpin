//! Explicit host desktop capabilities. Never execute a game-supplied command.
use crate::auth::broker::{
    protocol::{BrokerError, Command},
    service::Operations,
};
use crate::sandbox::permissions::{FolderGrant, Settings};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

enum OpenTarget {
    Web(String),
    Folder(PathBuf),
    Screenshot(PathBuf),
}

pub(crate) struct HostOperations {
    pub account: Option<Box<dyn Operations>>,
    pub settings: Settings,
    pub game: PathBuf,
    pub folders: Vec<FolderGrant>,
    pub imports: PathBuf,
    pub last_open: Option<Instant>,
}
impl Operations for HostOperations {
    fn valid(&self) -> bool {
        self.account.as_ref().is_none_or(|account| account.valid())
    }
    fn execute(&mut self, command: &Command) -> Result<serde_json::Value, BrokerError> {
        if matches!(
            command,
            Command::Realms { .. } | Command::GameServices { .. } | Command::Report { .. }
        ) && !self.settings.network
        {
            return Err(BrokerError::NetworkDenied);
        }
        if let Command::ImportFiles { paths, multiple } = command {
            if !self.settings.import_files || !self.settings.game_write {
                return Err(BrokerError::Forbidden);
            }
            let selected = crate::imports::select(paths.as_deref(), *multiple)?;
            return crate::imports::copy_selected(&selected, &self.imports)
                .map(|paths| serde_json::json!(paths));
        }
        if let Command::DesktopOpen { uri } = command {
            let target = self.open_target(uri)?;
            if self
                .last_open
                .is_some_and(|last| last.elapsed() < Duration::from_secs(1))
            {
                return Err(BrokerError::RateLimited);
            }
            self.last_open = Some(Instant::now());
            let target = if let OpenTarget::Screenshot(path) = target {
                let imported = crate::imports::copy_selected(&[path], &self.imports)?;
                let path = PathBuf::from(imported.first().ok_or(BrokerError::Unavailable)?);
                use std::io::Read;
                let mut signature = [0; 8];
                std::fs::File::open(&path)
                    .and_then(|mut f| f.read_exact(&mut signature))
                    .map_err(|_| BrokerError::InvalidRequest)?;
                if signature != *b"\x89PNG\r\n\x1a\n" {
                    return Err(BrokerError::Forbidden);
                }
                OpenTarget::Screenshot(path)
            } else {
                target
            };
            open(&target)?;
            return Ok(serde_json::Value::Null);
        }
        self.account
            .as_mut()
            .ok_or(BrokerError::Forbidden)?
            .execute(command)
    }
}
impl HostOperations {
    fn open_target(&self, uri: &str) -> Result<OpenTarget, BrokerError> {
        if uri.len() > 8192 || uri.chars().any(char::is_control) {
            return Err(BrokerError::InvalidRequest);
        }
        let url = reqwest::Url::parse(uri).map_err(|_| BrokerError::InvalidRequest)?;
        match url.scheme() {
            "http" | "https" if self.settings.open_links && self.settings.network => {
                if url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                {
                    return Err(BrokerError::InvalidRequest);
                }
                Ok(OpenTarget::Web(url.into()))
            }
            "file" if self.settings.open_folders => {
                let path = url
                    .to_file_path()
                    .map_err(|_| BrokerError::InvalidRequest)?;
                let canonical = path
                    .canonicalize()
                    .map_err(|_| BrokerError::InvalidRequest)?;
                if !canonical.starts_with(&self.game)
                    && !self
                        .folders
                        .iter()
                        .any(|folder| canonical.starts_with(&folder.path))
                {
                    return Err(BrokerError::Forbidden);
                }
                // File opening can launch applications, scripts or shortcuts. Vanilla needs
                // folders and screenshots; do not delegate arbitrary file execution to the OS.
                if !(canonical.is_dir()
                    || (canonical.is_file()
                        && canonical
                            .extension()
                            .is_some_and(|e| e.eq_ignore_ascii_case("png"))))
                {
                    return Err(BrokerError::Forbidden);
                }
                Ok(if canonical.is_dir() {
                    OpenTarget::Folder(canonical)
                } else {
                    OpenTarget::Screenshot(canonical)
                })
            }
            _ => Err(BrokerError::Forbidden),
        }
    }
}
fn open(target: &OpenTarget) -> Result<(), BrokerError> {
    #[cfg(not(windows))]
    {
        let program = if cfg!(target_os = "macos") {
            "/usr/bin/open"
        } else {
            "/usr/bin/xdg-open"
        };
        let mut command = std::process::Command::new(program);
        match target {
            OpenTarget::Web(uri) => {
                command.arg(uri);
            }
            OpenTarget::Folder(path) => {
                if cfg!(target_os = "macos") {
                    command.args(["-a", "Finder"]);
                } else {
                    command = std::process::Command::new("/usr/bin/dbus-send");
                    command.args([
                        "--session",
                        "--type=method_call",
                        "--dest=org.freedesktop.FileManager1",
                        "/org/freedesktop/FileManager1",
                        "org.freedesktop.FileManager1.ShowFolders",
                    ]);
                }
                let uri = reqwest::Url::from_directory_path(path)
                    .map_err(|_| BrokerError::InvalidRequest)?;
                if cfg!(target_os = "macos") {
                    command.arg(uri.as_str());
                } else {
                    command.arg(format!("array:string:{}", uri)).arg("string:");
                }
            }
            OpenTarget::Screenshot(path) => {
                if cfg!(target_os = "macos") {
                    command.args(["-a", "Preview"]);
                }
                command.arg(path);
            }
        }
        let mut child = command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|_| BrokerError::Unavailable)?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
    #[cfg(windows)]
    {
        if let OpenTarget::Folder(path) = target {
            let system = std::env::var_os("SystemRoot")
                .map(PathBuf::from)
                .ok_or(BrokerError::Unavailable)?;
            let mut child = std::process::Command::new(system.join("explorer.exe"))
                .arg(format!(
                    "/select,{}",
                    crate::storage::java_path(path).display()
                ))
                .spawn()
                .map_err(|_| BrokerError::Unavailable)?;
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return Ok(());
        }
        use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
        let value = match target {
            OpenTarget::Web(uri) => uri.clone(),
            OpenTarget::Screenshot(path) => crate::storage::java_path(path)
                .to_string_lossy()
                .into_owned(),
            _ => unreachable!(),
        };
        let operation: Vec<u16> = "open\0".encode_utf16().collect();
        let target: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
        // SAFETY: terminated strings remain valid throughout the SDK call.
        #[allow(unsafe_code)]
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result as usize <= 32 {
            return Err(BrokerError::Unavailable);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_grants_do_not_become_a_command_or_file_execution_proxy() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let game = root.path().canonicalize()?.join("game");
        std::fs::create_dir(&game)?;
        let mut host = HostOperations {
            account: None,
            settings: Settings::default(),
            game: game.clone(),
            folders: vec![],
            imports: game.join("imports"),
            last_open: None,
        };
        assert!(host.open_target("https://minecraft.net/help").is_ok());
        for uri in [
            "javascript:alert(1)",
            "shell:AppsFolder",
            "https://user:password@example.com",
            "https://example.com/\n",
        ] {
            assert!(host.open_target(uri).is_err());
        }
        let folder = reqwest::Url::from_directory_path(&game).expect("file URI");
        assert!(host.open_target(folder.as_str()).is_ok());
        for (name, allowed) in [
            ("screenshot.png", true),
            ("run.command", false),
            ("shortcut.lnk", false),
            ("mod.jar", false),
        ] {
            let path = game.join(name);
            std::fs::write(&path, b"test")?;
            let uri = reqwest::Url::from_file_path(&path).expect("file URI");
            assert_eq!(host.open_target(uri.as_str()).is_ok(), allowed);
        }
        let outside = reqwest::Url::from_directory_path(root.path()).expect("file URI");
        assert!(host.open_target(outside.as_str()).is_err());
        host.settings.network = false;
        assert!(host.open_target("https://minecraft.net").is_err());
        host.settings.open_folders = false;
        assert!(host.open_target(folder.as_str()).is_err());
        Ok(())
    }
}
