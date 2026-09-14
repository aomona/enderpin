use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::storage::{directory, hash_file, safe_path};

const MAX_EXPANDED: u64 = 3 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;

fn relative(path: &Path) -> Result<String> {
    let mut parts = vec![];
    for part in path.components() {
        match part {
            Component::Normal(p) => parts.push(p.to_str().context("archive path is not UTF-8")?),
            Component::CurDir => (),
            _ => bail!("archive path escapes destination"),
        }
    }
    ensure!(!parts.is_empty(), "empty archive path");
    Ok(parts.join("/"))
}

fn write_entry(root: &Path, path: &str, input: &mut impl Read, size: u64, mode: u32) -> Result<()> {
    if let Some(parent) = Path::new(path)
        .parent()
        .and_then(Path::to_str)
        .filter(|s| !s.is_empty())
    {
        directory(root, parent)?;
    }
    let path = safe_path(root, path)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    ensure!(
        std::io::copy(&mut input.take(size + 1), &mut file)? == size,
        "archive entry size mismatch"
    );
    file.flush()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o755))?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    Ok(())
}

pub fn extract(archive: &Path, tar_gz: bool, root: &Path) -> Result<()> {
    let mut total = 0u64;
    if tar_gz {
        let mut input = tar::Archive::new(flate2::read::GzDecoder::new(File::open(archive)?));
        let mut links = vec![];
        for (index, entry) in input.entries()?.enumerate() {
            ensure!(index < MAX_ENTRIES, "too many archive entries");
            let mut entry = entry?;
            let path = relative(&entry.path()?)?;
            let kind = entry.header().entry_type();
            if kind.is_dir() {
                directory(root, &path)?;
                continue;
            }
            if kind.is_symlink() {
                // Temurin legal-document aliases are materialized as ordinary files.
                let source = entry.link_name()?.context("archive link has no target")?;
                let mut resolved = Path::new(&path)
                    .parent()
                    .context("invalid link parent")?
                    .to_path_buf();
                ensure!(
                    path.split('/').nth(1) == Some("legal"),
                    "unexpected executable/runtime symlink in Java archive"
                );
                for component in source.components() {
                    match component {
                        Component::Normal(p) => resolved.push(p),
                        Component::CurDir => (),
                        Component::ParentDir if resolved.components().count() > 2 => {
                            resolved.pop();
                        }
                        _ => bail!("Java legal link escapes its subtree"),
                    }
                }
                links.push((path, relative(&resolved)?));
                continue;
            }
            ensure!(kind.is_file(), "unsupported archive entry type");
            let size = entry.size();
            total = total.checked_add(size).context("archive size overflow")?;
            ensure!(total <= MAX_EXPANDED, "archive expands too large");
            let mode = entry.header().mode()?;
            write_entry(root, &path, &mut entry, size, mode)?;
        }
        while !links.is_empty() {
            let count = links.len();
            let mut pending = vec![];
            for (destination, source) in links {
                let source_path = safe_path(root, &source)?;
                if !source_path.try_exists()? {
                    pending.push((destination, source));
                    continue;
                }
                let size = source_path.metadata()?.len();
                total = total.checked_add(size).context("archive size overflow")?;
                ensure!(
                    total <= MAX_EXPANDED && source_path.is_file(),
                    "invalid legal document alias"
                );
                write_entry(
                    root,
                    &destination,
                    &mut File::open(source_path)?,
                    size,
                    0o644,
                )?;
            }
            ensure!(pending.len() < count, "missing or cyclic archive links");
            links = pending;
        }
    } else {
        let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
        ensure!(zip.len() <= MAX_ENTRIES, "too many archive entries");
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index)?;
            let name = relative(&entry.enclosed_name().context("unsafe ZIP path")?)?;
            ensure!(
                entry
                    .unix_mode()
                    .is_none_or(|mode| mode & 0o170000 != 0o120000),
                "ZIP symlink is not supported"
            );
            if entry.is_dir() {
                directory(root, &name)?;
                continue;
            }
            let size = entry.size();
            total = total.checked_add(size).context("archive size overflow")?;
            ensure!(total <= MAX_EXPANDED, "archive expands too large");
            let mode = entry.unix_mode().unwrap_or(0o644);
            write_entry(root, &name, &mut entry, size, mode)?;
        }
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileStamp {
    path: String,
    sha512: String,
    size: u64,
}

fn files(root: &Path, relative: &str, result: &mut Vec<FileStamp>) -> Result<()> {
    let path = if relative.is_empty() {
        root.to_path_buf()
    } else {
        safe_path(root, relative)?
    };
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("invalid extracted filename"))?;
        if relative.is_empty() && name == "enderpin-files.json" {
            continue;
        }
        let path = if relative.is_empty() {
            name
        } else {
            format!("{relative}/{name}")
        };
        let full = safe_path(root, &path)?;
        if full.is_dir() {
            files(root, &path, result)?;
        } else {
            let (sha512, size) = hash_file(&full)?;
            result.push(FileStamp { path, sha512, size });
        }
    }
    Ok(())
}

pub fn stamp(root: &Path) -> Result<()> {
    let mut result = vec![];
    files(root, "", &mut result)?;
    result.sort_by(|a, b| a.path.cmp(&b.path));
    fs::write(
        safe_path(root, "enderpin-files.json")?,
        serde_json::to_vec(&result)?,
    )?;
    Ok(())
}

pub fn verify(root: &Path) -> Result<()> {
    let metadata = crate::storage::read_optional(root, "enderpin-files.json")?
        .context("incomplete runtime extraction")?;
    let expected: Vec<FileStamp> = serde_json::from_str(&metadata)?;
    ensure!(
        !expected.is_empty() && expected.len() <= MAX_ENTRIES,
        "invalid runtime file list"
    );
    let mut actual = vec![];
    files(root, "", &mut actual)?;
    actual.sort_by(|a, b| a.path.cmp(&b.path));
    ensure!(
        serde_json::to_vec(&actual)? == serde_json::to_vec(&expected)?,
        "extracted runtime files were modified; remove this runtime directory and prepare again: {}",
        root.display()
    );
    Ok(())
}

pub fn java(root: &Path) -> Result<PathBuf> {
    let mut candidates = vec![];
    for child in fs::read_dir(root)? {
        let child = child?;
        if !child.file_type()?.is_dir() {
            continue;
        }
        let name = child
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("invalid runtime directory"))?;
        for suffix in ["bin/java", "bin/java.exe", "Contents/Home/bin/java"] {
            let path = safe_path(root, &format!("{name}/{suffix}"))?;
            if path.is_file() {
                candidates.push(path);
            }
        }
    }
    ensure!(
        candidates.len() == 1,
        "Java archive does not contain exactly one Java executable"
    );
    candidates.pop().context("Java executable missing")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_archive_escape_and_verify_extracted_changes() -> Result<()> {
        assert!(relative(Path::new("../outside")).is_err());
        let temp = tempfile::tempdir()?;
        fs::write(temp.path().join("sample"), b"original")?;
        stamp(temp.path())?;
        verify(temp.path())?;
        fs::write(temp.path().join("sample"), b"changed")?;
        assert!(verify(temp.path()).is_err());
        Ok(())
    }
}
