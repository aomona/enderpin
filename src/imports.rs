//! User-mediated copies into a read-only game-visible staging directory.
use crate::auth::broker::protocol::BrokerError;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub(crate) fn select(
    paths: Option<&[String]>,
    multiple: bool,
) -> Result<Vec<PathBuf>, BrokerError> {
    let selected = paths
        .map(|paths| {
            if paths.is_empty() || paths.len() > 32 {
                return Err(BrokerError::InvalidRequest);
            }
            paths
                .iter()
                .map(|path| {
                    let path = Path::new(path);
                    if !path.is_absolute() {
                        return Err(BrokerError::InvalidRequest);
                    }
                    let path = path
                        .canonicalize()
                        .map_err(|_| BrokerError::InvalidRequest)?;
                    if path.to_string_lossy().chars().any(char::is_control)
                        || path.to_string_lossy().contains('|')
                    {
                        return Err(BrokerError::InvalidRequest);
                    }
                    Ok(path)
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    let prompt = selected
        .as_ref()
        .map(|paths| {
            format!(
                "Allow Minecraft to import copies of these files/folders?\n\n{}",
                paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        })
        .unwrap_or_else(|| "Choose files to import into Minecraft".into());
    if prompt.len() > 8192 {
        return Err(BrokerError::InvalidRequest);
    }
    let mut command = dialog(selected.is_some(), multiple, &prompt);
    command.stdin(Stdio::null()).stderr(Stdio::null());
    // A file avoids a full stdout pipe deadlocking a large multi-selection. Limit reads below.
    let output = tempfile::tempfile().map_err(|_| BrokerError::Unavailable)?;
    command.stdout(output.try_clone().map_err(|_| BrokerError::Unavailable)?);
    let mut child = command.spawn().map_err(|_| BrokerError::Unavailable)?;
    let start = Instant::now();
    let status = loop {
        match child.try_wait().map_err(|_| BrokerError::Unavailable)? {
            Some(status) => break status,
            None if start.elapsed() < Duration::from_secs(120) => {
                std::thread::sleep(Duration::from_millis(100))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(BrokerError::Unavailable);
            }
        }
    };
    if !status.success() {
        return Ok(vec![]);
    }
    if let Some(paths) = selected {
        return Ok(paths);
    }
    use std::io::Seek;
    let mut output = output;
    output.rewind().map_err(|_| BrokerError::Unavailable)?;
    let mut text = String::new();
    output
        .take(65537)
        .read_to_string(&mut text)
        .map_err(|_| BrokerError::InvalidResponse)?;
    if text.len() > 65536 {
        return Err(BrokerError::InvalidResponse);
    }
    let paths: Vec<_> = text
        .lines()
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect();
    if paths.len() > if multiple { 32 } else { 1 } {
        return Err(BrokerError::InvalidResponse);
    }
    let paths = paths
        .into_iter()
        .map(|path| {
            path.canonicalize()
                .map_err(|_| BrokerError::InvalidResponse)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if paths.is_empty() {
        return Ok(paths);
    }
    // Native pickers use line-separated output. Confirm the exact resolved list so a
    // filename containing a separator cannot silently authorize a different host file.
    let paths: Vec<_> = paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    select(Some(&paths), multiple)
}

fn dialog(confirm: bool, multiple: bool, prompt: &str) -> Command {
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("/usr/bin/osascript");
        let script = if confirm {
            "on run argv\ndisplay dialog (item 1 of argv) with title \"Minecraft file access\" buttons {\"Cancel\", \"Import\"} default button \"Cancel\" cancel button \"Cancel\"\nend run"
        } else if multiple {
            "on run argv\nset chosen to choose file with prompt (item 1 of argv) with multiple selections allowed\nset resultText to \"\"\nrepeat with f in chosen\nset resultText to resultText & POSIX path of f & linefeed\nend repeat\nreturn resultText\nend run"
        } else {
            "on run argv\nreturn POSIX path of (choose file with prompt (item 1 of argv))\nend run"
        };
        command.args(["-e", script, prompt]);
        command
    }
    #[cfg(target_os = "linux")]
    {
        let mut command = Command::new("/usr/bin/zenity");
        if confirm {
            command.args([
                "--question",
                "--default-cancel",
                "--no-markup",
                "--text",
                prompt,
            ]);
        } else {
            command.args(["--file-selection", "--title", prompt]);
            if multiple {
                command.args(["--multiple", "--separator=\n"]);
            }
        }
        command
    }
    #[cfg(windows)]
    {
        let system = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let mut command =
            Command::new(system.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"));
        let script = if confirm {
            "Add-Type -AssemblyName System.Windows.Forms; if ([System.Windows.Forms.MessageBox]::Show($env:ENDERPIN_IMPORT_PROMPT, 'Minecraft file access', 'OKCancel', 'Question', 'Button2') -ne 'OK') { exit 1 }"
        } else {
            "Add-Type -AssemblyName System.Windows.Forms; [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); $d = [System.Windows.Forms.OpenFileDialog]::new(); $d.Title = $env:ENDERPIN_IMPORT_PROMPT; $d.Multiselect = $env:ENDERPIN_IMPORT_MULTIPLE -eq 'true'; if ($d.ShowDialog() -ne 'OK') { exit 1 }; $d.FileNames"
        };
        command
            .args(["-NoProfile", "-NonInteractive", "-Sta", "-Command", script])
            .env("ENDERPIN_IMPORT_PROMPT", prompt)
            .env("ENDERPIN_IMPORT_MULTIPLE", multiple.to_string());
        command
    }
}

pub(crate) fn copy_selected(paths: &[PathBuf], root: &Path) -> Result<Vec<String>, BrokerError> {
    if paths.is_empty() {
        return Ok(vec![]);
    }
    if paths.len() > 32 {
        return Err(BrokerError::InvalidRequest);
    }
    let batch = tempfile::tempdir_in(root).map_err(|_| BrokerError::Unavailable)?;
    let mut result = vec![];
    let mut remaining = 2_u64 * 1024 * 1024 * 1024;
    let mut entries = 0;
    for (index, source) in paths.iter().enumerate() {
        if !source.is_absolute() {
            return Err(BrokerError::InvalidRequest);
        }
        let name = source.file_name().ok_or(BrokerError::InvalidRequest)?;
        let parent = batch.path().join(index.to_string());
        fs::create_dir(&parent).map_err(|_| BrokerError::Unavailable)?;
        let destination = parent.join(name);
        let mut queue = vec![(source.clone(), destination.clone())];
        while let Some((source, destination)) = queue.pop() {
            entries += 1;
            if entries > 100_000 {
                return Err(BrokerError::InvalidRequest);
            }
            let metadata =
                fs::symlink_metadata(&source).map_err(|_| BrokerError::InvalidRequest)?;
            if crate::storage::is_link(&metadata) {
                return Err(BrokerError::Forbidden);
            }
            if metadata.is_dir() {
                fs::create_dir(&destination).map_err(|_| BrokerError::Unavailable)?;
                for entry in fs::read_dir(&source).map_err(|_| BrokerError::InvalidRequest)? {
                    let entry = entry.map_err(|_| BrokerError::InvalidRequest)?;
                    queue.push((entry.path(), destination.join(entry.file_name())));
                }
            } else if metadata.is_file() {
                let file = fs::File::open(&source).map_err(|_| BrokerError::InvalidRequest)?;
                // Compare the opened object, not just a pre-open path: the game may race a
                // writable source directory with a symlink/junction replacement.
                if opened_path(&file).map_err(|_| BrokerError::Forbidden)? != source {
                    return Err(BrokerError::Forbidden);
                }
                if !file
                    .metadata()
                    .map_err(|_| BrokerError::InvalidRequest)?
                    .is_file()
                {
                    return Err(BrokerError::Forbidden);
                }
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&destination)
                    .map_err(|_| BrokerError::Unavailable)?;
                let copied = std::io::copy(&mut file.take(remaining + 1), &mut output)
                    .map_err(|_| BrokerError::Unavailable)?;
                remaining = remaining
                    .checked_sub(copied)
                    .ok_or(BrokerError::InvalidRequest)?;
                output.flush().map_err(|_| BrokerError::Unavailable)?;
            } else {
                return Err(BrokerError::Forbidden);
            }
        }
        result.push(
            crate::storage::java_path(&destination)
                .to_string_lossy()
                .into_owned(),
        );
    }
    let _ = batch.keep(); // Read-only copies remain usable until explicitly removed between launches.
    Ok(result)
}

fn opened_path(file: &fs::File) -> std::io::Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::{fd::AsRawFd, unix::ffi::OsStrExt};
        let mut bytes = [0_u8; 1024];
        // SAFETY: F_GETPATH writes at most MAXPATHLEN bytes into the supplied buffer.
        #[allow(unsafe_code)]
        let result =
            unsafe { nix::libc::fcntl(file.as_raw_fd(), nix::libc::F_GETPATH, bytes.as_mut_ptr()) };
        if result < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let end = bytes
            .iter()
            .position(|b| *b == 0)
            .ok_or(std::io::ErrorKind::InvalidData)?;
        Ok(PathBuf::from(std::ffi::OsStr::from_bytes(&bytes[..end])))
    }
    #[cfg(windows)]
    {
        use std::os::{windows::ffi::OsStringExt, windows::io::AsRawHandle};
        let mut bytes = vec![0_u16; 32768];
        // SAFETY: the live file handle and writable buffer remain valid through the call.
        #[allow(unsafe_code)]
        let len = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                0,
            )
        } as usize;
        if len == 0 || len >= bytes.len() {
            return Err(std::io::Error::last_os_error());
        }
        Ok(PathBuf::from(std::ffi::OsString::from_wide(&bytes[..len])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imports_copy_only_selected_regular_data_and_preserve_names() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let root = root.path().canonicalize()?;
        let source = root.join("selected");
        fs::create_dir(&source)?;
        fs::write(source.join("pack.mcmeta"), "data")?;
        let staging = root.join("staging");
        fs::create_dir(&staging)?;
        let result = copy_selected(std::slice::from_ref(&source), &staging)?;
        assert_eq!(
            fs::read_to_string(Path::new(&result[0]).join("pack.mcmeta"))?,
            "data"
        );
        assert_eq!(
            Path::new(&result[0]).file_name(),
            Some(std::ffi::OsStr::new("selected"))
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/passwd", source.join("alias"))?;
            assert!(copy_selected(&[source], &staging).is_err());
        }
        Ok(())
    }
}
