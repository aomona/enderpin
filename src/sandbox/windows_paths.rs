//! JDK FOLLOW_LINKS path resolution without reading or editing host ancestors.
use super::super::Policy;
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    os::windows::{ffi::OsStrExt, io::AsRawHandle},
    path::Path,
};
use windows_sys::Win32::{
    Foundation::INVALID_HANDLE_VALUE,
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, GetFinalPathNameByHandleW, GetLogicalDrives,
        OPEN_EXISTING, VOLUME_NAME_NT,
    },
};

pub(super) struct Prepared {
    pub policy: Policy,
    pub arguments: Vec<OsString>,
}

pub(super) fn prepare(policy: &Policy) -> Result<Prepared> {
    let assets: [(&str, &[u8]); 3] = [
        (
            "windows-paths-bootstrap.jar",
            include_bytes!(concat!(env!("OUT_DIR"), "/windows-paths-bootstrap.jar")),
        ),
        (
            "windows-paths.jar",
            include_bytes!(concat!(env!("OUT_DIR"), "/windows-paths.jar")),
        ),
        (
            "windows-paths.dll",
            include_bytes!(concat!(env!("OUT_DIR"), "/windows-paths.dll")),
        ),
    ];
    let mut hash = Sha256::new();
    for (_, asset) in assets {
        hash.update(asset);
    }
    let relative = format!(".enderpin-windows/{:x}", hash.finalize());
    let directory = crate::storage::directory(
        policy.game.parent().context("game parent missing")?,
        &relative,
    )?
    .canonicalize()?;
    crate::sandbox::validate_data_tree(&directory)?;
    for (name, bytes) in assets {
        let path = crate::storage::safe_path(&directory, name)?;
        if path.try_exists()? {
            ensure!(
                crate::storage::hash_file(&path)?
                    == (crate::model::hash_bytes(bytes), bytes.len() as u64),
                "Windows path adapter differs from the bundled version: {}",
                path.display()
            );
        } else {
            use std::io::Write;
            let mut file = tempfile::NamedTempFile::new_in(&directory)?;
            file.write_all(bytes)?;
            file.persist_noclobber(path).map_err(|error| error.error)?;
        }
    }
    let mut effective = policy.clone();
    effective.readonly.push(directory);
    // A relative agent filename keeps '=' in user-selected directory names out
    // of the JVM's agent filename/options separator.
    let agent = Path::new("..").join(&relative).join("windows-paths.jar");
    Ok(Prepared {
        policy: effective,
        arguments: vec![
            format!("-javaagent:{}", agent.display()).into(),
            format!("-Denderpin.windows.volumes={}", volume_mappings()?).into(),
        ],
    })
}

fn volume_mappings() -> Result<String> {
    // SAFETY: this system query has no pointer arguments or ownership transfer.
    let drives = unsafe { GetLogicalDrives() };
    ensure!(drives != 0, "cannot enumerate Windows drive mappings");
    let mut mappings = Vec::new();
    for drive in 0..26 {
        if drives & (1 << drive) == 0 {
            continue;
        }
        let letter = char::from(b'A' + drive as u8);
        let root = format!("{letter}:\\");
        let wide: Vec<u16> = std::ffi::OsStr::new(&root)
            .encode_wide()
            .chain([0])
            .collect();
        // Resolve volume names in the host, where the mount manager is accessible.
        // The child still opens and resolves every requested file with its own token.
        // SAFETY: the drive root is NUL terminated; optional pointers are null.
        let file = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                std::ptr::null_mut(),
            )
        };
        if file == INVALID_HANDLE_VALUE {
            continue;
        }
        let file = super::owned(file)?;
        let mut buffer = vec![0u16; 32768];
        // SAFETY: the handle is live and the writable buffer matches its capacity.
        let length = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                VOLUME_NAME_NT,
            )
        };
        if length == 0 || length as usize >= buffer.len() {
            continue;
        }
        let device = String::from_utf16(&buffer[..length as usize])?;
        mappings.push((device.trim_end_matches('\\').to_owned(), letter));
    }
    // SUBST drives can map a subtree; prefer the most specific matching device path.
    mappings.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.1.cmp(&b.1)));
    ensure!(
        !mappings.is_empty(),
        "no accessible Windows volume mappings"
    );
    Ok(mappings
        .into_iter()
        .map(|(device, letter)| format!("{letter}:={device}"))
        .collect::<Vec<_>>()
        .join("|"))
}
