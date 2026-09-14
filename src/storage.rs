use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};

use crate::model::{LockedPackage, hash_bytes, safe_filename, validate_hash};

pub fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

/// Validate every existing component, including Windows junction/reparse points.
pub fn safe_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    ensure!(
        !relative.is_empty() && !relative.contains('\\'),
        "invalid relative path"
    );
    for component in Path::new(relative).components() {
        let Component::Normal(name) = component else {
            bail!("path must stay inside the workspace");
        };
        safe_filename(name.to_str().context("non-UTF8 path")?)?;
        path.push(name);
        match fs::symlink_metadata(&path) {
            Ok(meta) => ensure!(
                !is_link(&meta),
                "symbolic links/reparse points are not allowed: {}",
                path.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(path)
}

pub fn directory(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = safe_path(root, relative)?;
    fs::create_dir_all(&path)?;
    safe_path(root, relative)?;
    ensure!(path.is_dir(), "not a directory: {}", path.display());
    Ok(path)
}

pub fn read_optional(root: &Path, relative: &str) -> Result<Option<String>> {
    let path = safe_path(root, relative)?;
    match File::open(&path) {
        Ok(file) => {
            ensure!(
                file.metadata()?.is_file(),
                "expected a file: {}",
                path.display()
            );
            let mut text = String::new();
            file.take(32 * 1024 * 1024 + 1).read_to_string(&mut text)?;
            ensure!(text.len() <= 32 * 1024 * 1024, "metadata file is too large");
            Ok(Some(text))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn hash_file(path: &Path) -> Result<(String, u64)> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file() && !is_link(&meta),
        "expected a regular file: {}",
        path.display()
    );
    let mut file = File::open(path)?;
    let mut hash = Sha512::new();
    let mut size = 0;
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        size += n as u64;
    }
    Ok((format!("{:x}", hash.finalize()), size))
}

pub fn operation_lock(root: &Path) -> Result<File> {
    directory(root, ".enderpin")?;
    let path = safe_path(root, ".enderpin/operation.lock")?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.try_lock()
        .map_err(|_| anyhow::anyhow!("another Enderpin operation is using this workspace"))?;
    Ok(file)
}

/// Kept by the launcher while a target runs; independent targets can run together.
pub fn target_lock(root: &Path, side: crate::Side) -> Result<File> {
    directory(root, &format!(".enderpin/{}", side.name()))?;
    let path = safe_path(root, &format!(".enderpin/{}/run.lock", side.name()))?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.try_lock()
        .map_err(|_| anyhow::anyhow!("{} is running or being synchronized", side.name()))?;
    Ok(file)
}

pub struct Cache {
    root: PathBuf,
}
impl Cache {
    pub fn runtime_root(&self) -> Result<PathBuf> {
        directory(&self.root, "runtime")
    }
    pub fn open(path: &Path) -> Result<Self> {
        fs::create_dir_all(path)?;
        let root = path.canonicalize()?;
        Ok(Self { root })
    }
    pub fn default_path() -> Result<PathBuf> {
        let dirs = directories::ProjectDirs::from("", "", "enderpin")
            .context("cannot determine cache directory; specify --cache-dir")?;
        Ok(dirs.cache_dir().join("sha512"))
    }
    pub fn path(&self, hash: &str) -> Result<PathBuf> {
        validate_hash(hash)?;
        safe_path(&self.root, hash)
    }
    pub fn find(&self, hash: &str, size: u64) -> Result<Option<PathBuf>> {
        let path = self.path(hash)?;
        if !path.try_exists()? {
            return Ok(None);
        }
        let actual = hash_file(&path)?;
        ensure!(
            actual == (hash.to_owned(), size),
            "corrupt cache entry {}; remove this entry and retry",
            path.display()
        );
        Ok(Some(path))
    }
    pub fn acquire(&self, p: &LockedPackage, offline: bool) -> Result<PathBuf> {
        if let Some(path) = self.find(&p.sha512, p.size)? {
            return Ok(path);
        }
        ensure!(!offline, "{} is not cached (offline)", p.name);
        let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
        let (hash, size) = crate::registry::download(&p.url, temp.as_file_mut())?;
        ensure!(
            hash == p.sha512 && size == p.size,
            "download does not match locked SHA-512/size for {}",
            p.name
        );
        self.persist(temp, &hash, size)
    }
    pub fn acquire_url(&self, url: &str) -> Result<(String, u64)> {
        let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
        let (hash, size) = crate::registry::download(url, temp.as_file_mut())?;
        self.persist(temp, &hash, size)?;
        Ok((hash, size))
    }
    fn persist(&self, temp: tempfile::NamedTempFile, hash: &str, size: u64) -> Result<PathBuf> {
        temp.as_file().sync_all()?;
        let path = self.path(hash)?;
        match temp.persist_noclobber(&path) {
            Ok(_) => Ok(path),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                self.find(hash, size)?.context("cache entry disappeared")
            }
            Err(error) => Err(error.error.into()),
        }
    }
    #[cfg(test)]
    pub fn seed(&self, bytes: &[u8]) -> Result<String> {
        let hash = hash_bytes(bytes);
        let mut file = tempfile::NamedTempFile::new_in(&self.root)?;
        file.write_all(bytes)?;
        self.persist(file, &hash, bytes.len() as u64)?;
        Ok(hash)
    }
}

pub enum Content {
    Bytes(Vec<u8>),
    File { path: PathBuf, sha512: String },
    Delete,
}
pub struct Change {
    pub path: String,
    pub content: Content,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    old: Option<String>,
    new: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    format: u32,
    entries: Vec<Entry>,
}

pub(crate) fn valid_destination(relative: &str) -> Result<()> {
    if [
        "enderpin.toml",
        "enderpin.lock",
        ".gitignore",
        ".enderpin/client/state.toml",
        ".enderpin/server/state.toml",
    ]
    .contains(&relative)
    {
        return Ok(());
    }
    let components: Vec<_> = relative.split('/').collect();
    ensure!(
        components.len() == 5
            && components[0] == ".enderpin"
            && ["client", "server"].contains(&components[1])
            && components[2] == "game"
            && ["mods", "resourcepacks", "shaderpacks", "plugins"].contains(&components[3]),
        "invalid transaction destination: {relative}"
    );
    safe_filename(components[4])
}

fn present_hash(root: &Path, path: &str) -> Result<Option<String>> {
    let path = safe_path(root, path)?;
    if path.try_exists()? {
        Ok(Some(hash_file(&path)?.0))
    } else {
        Ok(None)
    }
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

/// A durable undo journal covers game files, installed state, manifest and lock.
/// Holding the workspace operation lock is required for both commit and recover.
pub fn commit(root: &Path, changes: Vec<Change>) -> Result<()> {
    ensure!(
        !safe_path(root, ".enderpin/transaction")?.try_exists()?,
        "pending transaction must be recovered first"
    );
    let mut seen = BTreeSet::new();
    for change in &changes {
        valid_destination(&change.path)?;
        safe_path(root, &change.path)?;
        ensure!(
            seen.insert(change.path.clone()),
            "duplicate transaction destination"
        );
    }
    directory(root, ".enderpin/transaction/new")?;
    directory(root, ".enderpin/transaction/old")?;
    let staged = (|| -> Result<Journal> {
        let mut entries = vec![];
        for (i, change) in changes.iter().enumerate() {
            let path = safe_path(root, &format!(".enderpin/transaction/new/{i}"))?;
            let new = match &change.content {
                Content::Bytes(bytes) => {
                    write_synced(&path, bytes)?;
                    Some(hash_bytes(bytes))
                }
                Content::File {
                    path: source,
                    sha512,
                } => {
                    fs::copy(source, &path)?;
                    File::open(&path)?.sync_all()?;
                    let actual = hash_file(&path)?.0;
                    ensure!(&actual == sha512, "cached artifact changed while staging");
                    Some(actual)
                }
                Content::Delete => None,
            };
            entries.push(Entry {
                path: change.path.clone(),
                old: present_hash(root, &change.path)?,
                new,
            });
        }
        let journal = Journal { format: 1, entries };
        write_synced(
            &safe_path(root, ".enderpin/transaction/journal.toml")?,
            toml::to_string(&journal)?.as_bytes(),
        )?;
        Ok(journal)
    })();
    let journal = match staged {
        Ok(journal) => journal,
        Err(error) => {
            fs::remove_dir_all(safe_path(root, ".enderpin/transaction")?)?;
            return Err(error);
        }
    };
    let applied = (|| -> Result<()> {
        for (i, entry) in journal.entries.iter().enumerate() {
            ensure!(
                present_hash(root, &entry.path)? == entry.old,
                "file changed during transaction: {}",
                entry.path
            );
            let destination = safe_path(root, &entry.path)?;
            if entry.old.is_some() {
                fs::rename(
                    &destination,
                    safe_path(root, &format!(".enderpin/transaction/old/{i}"))?,
                )?;
            }
            if entry.new.is_some() {
                if let Some(parent) = Path::new(&entry.path)
                    .parent()
                    .and_then(Path::to_str)
                    .filter(|p| !p.is_empty())
                {
                    directory(root, parent)?;
                }
                fs::rename(
                    safe_path(root, &format!(".enderpin/transaction/new/{i}"))?,
                    &destination,
                )?;
            }
        }
        write_synced(
            &safe_path(root, ".enderpin/transaction/committed")?,
            b"committed\n",
        )?;
        Ok(())
    })();
    if let Err(error) = applied {
        recover(root).context(
            "transaction failed and recovery needs attention; backups are in .enderpin/transaction",
        )?;
        return Err(error);
    }
    fs::remove_dir_all(safe_path(root, ".enderpin/transaction")?)
        .context("changes committed; transaction cleanup will be retried next time")?;
    Ok(())
}

pub fn recover(root: &Path) -> Result<bool> {
    let directory = safe_path(root, ".enderpin/transaction")?;
    if !directory.try_exists()? {
        return Ok(false);
    }
    if safe_path(root, ".enderpin/transaction/committed")?.try_exists()? {
        fs::remove_dir_all(directory)?;
        return Ok(true);
    }
    let Some(text) = read_optional(root, ".enderpin/transaction/journal.toml")? else {
        // Before a journal is persisted, no destination has been touched.
        fs::remove_dir_all(directory)?;
        return Ok(true);
    };
    let journal: Journal = toml::from_str(&text)
        .context("invalid recovery journal; preserve .enderpin/transaction for inspection")?;
    ensure!(journal.format == 1, "unsupported recovery journal");
    for entry in &journal.entries {
        valid_destination(&entry.path)?;
        if let Some(hash) = &entry.old {
            validate_hash(hash)?;
        }
        if let Some(hash) = &entry.new {
            validate_hash(hash)?;
        }
    }
    for (i, entry) in journal.entries.iter().enumerate().rev() {
        let backup = safe_path(root, &format!(".enderpin/transaction/old/{i}"))?;
        let current = present_hash(root, &entry.path)?;
        let destination = safe_path(root, &entry.path)?;
        if backup.try_exists()? {
            ensure!(
                Some(hash_file(&backup)?.0) == entry.old,
                "recovery backup changed for {}",
                entry.path
            );
            ensure!(
                current.is_none() || current == entry.new,
                "{} changed after interruption; recovery preserved its backup",
                entry.path
            );
            if current.is_some() {
                fs::remove_file(&destination)?;
            }
            fs::rename(backup, destination)?;
        } else if entry.old.is_none() {
            ensure!(
                current.is_none() || current == entry.new,
                "new file changed after interruption: {}",
                entry.path
            );
            if current.is_some() {
                fs::remove_file(destination)?;
            }
        } else {
            ensure!(
                current == entry.old,
                "cannot recover original file: {}",
                entry.path
            );
        }
    }
    fs::remove_dir_all(directory)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_commit_restores_original_files() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path();
        let _guard = operation_lock(root)?;
        fs::write(root.join("enderpin.toml"), "new")?;
        directory(root, ".enderpin/transaction/old")?;
        fs::write(root.join(".enderpin/transaction/old/0"), "old")?;
        let journal = Journal {
            format: 1,
            entries: vec![Entry {
                path: "enderpin.toml".into(),
                old: Some(hash_bytes(b"old")),
                new: Some(hash_bytes(b"new")),
            }],
        };
        fs::write(
            root.join(".enderpin/transaction/journal.toml"),
            toml::to_string(&journal)?,
        )?;
        assert!(recover(root)?);
        assert_eq!(fs::read_to_string(root.join("enderpin.toml"))?, "old");
        assert!(!recover(root)?);
        Ok(())
    }
    #[test]
    fn commit_and_lock_are_exclusive() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path();
        let _guard = operation_lock(root)?;
        assert!(operation_lock(root).is_err());
        commit(
            root,
            vec![Change {
                path: "enderpin.toml".into(),
                content: Content::Bytes(b"example".to_vec()),
            }],
        )?;
        assert_eq!(fs::read(root.join("enderpin.toml"))?, b"example");
        assert!(
            commit(
                root,
                vec![Change {
                    path: "../evil".into(),
                    content: Content::Delete
                }]
            )
            .is_err()
        );
        Ok(())
    }
    #[test]
    #[cfg(unix)]
    fn symlinked_directory_is_rejected() -> Result<()> {
        let root = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        std::os::unix::fs::symlink(outside.path(), root.path().join(".enderpin"))?;
        assert!(operation_lock(root.path()).is_err());
        Ok(())
    }
}
