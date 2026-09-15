use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::{
    model::{FORMAT, Lockfile, Manifest, Side, parse_ignores, validate_hash},
    registry::Modrinth,
    resolver,
    storage::{self, Cache, Change, Content},
};

#[derive(Default, Clone, Copy)]
pub struct SyncOptions {
    pub locked: bool,
    pub offline: bool,
    pub update: bool,
    pub restore: bool,
    pub no_ignore: bool,
    pub lock_only: bool,
}

#[derive(Debug, Serialize)]
pub struct Progress {
    pub target: Side,
    pub package: String,
    pub action: &'static str,
}

#[derive(Debug, Default, Serialize)]
pub struct SyncReport {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub unchanged: Vec<String>,
    pub targets: Vec<Side>,
    pub lock_updated: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstalledFile {
    sha512: String,
    size: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstalledState {
    format: u32,
    files: BTreeMap<String, InstalledFile>,
}

/// Owns an exclusive advisory workspace lock through planning and commit.
pub struct Workspace {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub lockfile: Lockfile,
    pub registry: Modrinth,
    cache: Cache,
    original_manifest: String,
    original_lock: Option<String>,
    _guard: File,
}

impl Workspace {
    pub fn prepare_runtime(
        &mut self,
        sides: &[Side],
        options: SyncOptions,
        update: bool,
        mut progress: impl FnMut(Side, &str),
    ) -> Result<BTreeMap<Side, crate::runtime::PreparedRuntime>> {
        let manifest = self.manifest.clone();
        let mut lock = self.plan(&manifest, sides, options)?;
        let mut prepared = BTreeMap::new();
        for &side in sides {
            let previous = lock.runtimes.get(&side);
            if options.locked || options.offline {
                ensure!(!update, "cannot update runtimes with --locked or --offline");
                ensure!(
                    previous.is_some_and(|runtime| crate::runtime::RuntimeLock::fingerprint(
                        &manifest, side
                    )
                    .is_ok_and(|fp| fp == runtime.fingerprint)),
                    "{} runtime is not locked; run prepare first",
                    side.name()
                );
            }
            let runtime = crate::runtime::resolve(&manifest, side, previous, update, &self.cache)?;
            let ready =
                crate::runtime::prepare(&runtime, side, &self.cache, options.offline, |message| {
                    progress(side, message)
                })?;
            lock.runtimes.insert(side, runtime);
            prepared.insert(side, ready);
        }
        self.apply(manifest, lock, sides, options, |event| {
            progress(event.target, &event.package)
        })?;
        Ok(prepared)
    }
    pub fn init(root: &Path, minecraft: String) -> Result<()> {
        Self::init_manifest(root, Manifest::new(minecraft)?, &Side::ALL)
    }

    pub fn init_vanilla_client(root: &Path, minecraft: String) -> Result<()> {
        Self::init_quick(root, minecraft, Side::Client)
    }

    pub fn init_quick(root: &Path, minecraft: String, side: Side) -> Result<()> {
        Self::init_manifest(root, Self::quick_manifest_for(minecraft, side)?, &[side])
    }

    pub fn quick_manifest(minecraft: String) -> Result<Manifest> {
        Self::quick_manifest_for(minecraft, Side::Client)
    }

    pub fn quick_manifest_for(minecraft: String, side: Side) -> Result<Manifest> {
        let mut manifest = Manifest::new(minecraft)?;
        manifest.target_mut(side).loader = "vanilla".into();
        manifest.target_mut(side).sandbox.account_authentication = false;
        manifest.validate()?;
        Ok(manifest)
    }

    fn init_manifest(root: &Path, manifest: Manifest, sides: &[Side]) -> Result<()> {
        fs::create_dir_all(root)?;
        let root = root.canonicalize()?;
        let _guard = storage::operation_lock(&root)?;
        storage::recover(&root)?;
        ensure!(
            storage::read_optional(&root, "enderpin.toml")?.is_none(),
            "enderpin.toml already exists"
        );
        ensure!(
            storage::read_optional(&root, "enderpin.lock")?.is_none(),
            "enderpin.lock already exists"
        );
        let mut lock = Lockfile::default();
        for &side in sides {
            lock.targets.insert(
                side,
                crate::model::TargetLock {
                    fingerprint: manifest.fingerprint(side)?,
                    minecraft: manifest.minecraft.clone(),
                    loader: manifest.target(side).loader.clone(),
                    requests: BTreeMap::new(),
                    packages: vec![],
                },
            );
        }
        let mut gitignore = storage::read_optional(&root, ".gitignore")?.unwrap_or_default();
        for pattern in ["/.enderpin/", "/.enderpinignore"] {
            if !gitignore.lines().any(|line| line == pattern) {
                if !gitignore.is_empty() && !gitignore.ends_with('\n') {
                    gitignore.push('\n');
                }
                gitignore.push_str(pattern);
                gitignore.push('\n');
            }
        }
        storage::commit(
            &root,
            vec![
                Change {
                    path: "enderpin.toml".into(),
                    content: Content::Bytes(toml::to_string_pretty(&manifest)?.into_bytes()),
                },
                Change {
                    path: "enderpin.lock".into(),
                    content: Content::Bytes(toml::to_string_pretty(&lock)?.into_bytes()),
                },
                Change {
                    path: ".gitignore".into(),
                    content: Content::Bytes(gitignore.into_bytes()),
                },
            ],
        )
    }
    pub fn open(root: &Path, cache: &Path) -> Result<Self> {
        let root = root
            .canonicalize()
            .context("workspace does not exist; run enderpin init")?;
        let guard = storage::operation_lock(&root)?;
        storage::recover(&root)?;
        let original_manifest = storage::read_optional(&root, "enderpin.toml")?
            .context("enderpin.toml not found; run enderpin init")?;
        let manifest = Manifest::parse(&original_manifest)?;
        let original_lock = storage::read_optional(&root, "enderpin.lock")?;
        let lockfile: Lockfile = original_lock
            .as_deref()
            .map(toml::from_str)
            .transpose()
            .context("invalid enderpin.lock")?
            .unwrap_or_default();
        lockfile.validate()?;
        Ok(Self {
            root,
            manifest,
            lockfile,
            registry: Modrinth::default(),
            cache: Cache::open(cache)?,
            original_manifest,
            original_lock,
            _guard: guard,
        })
    }
    pub fn plan(
        &mut self,
        manifest: &Manifest,
        sides: &[Side],
        options: SyncOptions,
    ) -> Result<Lockfile> {
        manifest.validate()?;
        ensure!(!sides.is_empty(), "select at least one target");
        ensure!(
            !(options.locked && options.update),
            "locked and update cannot be combined"
        );
        let mut lock = self.lockfile.clone();
        for &side in sides {
            let fingerprint = manifest.fingerprint(side)?;
            let previous = self.lockfile.targets.get(&side);
            let matches = previous.is_some_and(|old| old.fingerprint == fingerprint);
            if options.locked || options.offline {
                ensure!(
                    matches,
                    "{} configuration differs from the lock; run sync or lock without --locked/--offline",
                    side.name()
                );
            }
            if matches && !options.update {
                continue;
            }
            ensure!(!options.offline, "cannot update in offline mode");
            let resolved = resolver::resolve(
                manifest,
                side,
                previous,
                options.update,
                &mut self.registry,
                &self.cache,
            )
            .with_context(|| format!("resolving {}", side.name()))?;
            lock.targets.insert(side, resolved);
        }
        lock.validate()?;
        Ok(lock)
    }
    pub fn sync(
        &mut self,
        manifest: Manifest,
        sides: &[Side],
        options: SyncOptions,
        progress: impl FnMut(Progress),
    ) -> Result<SyncReport> {
        let lock = self.plan(&manifest, sides, options)?;
        self.apply(manifest, lock, sides, options, progress)
    }
    pub fn apply(
        &mut self,
        manifest: Manifest,
        lock: Lockfile,
        sides: &[Side],
        options: SyncOptions,
        mut progress: impl FnMut(Progress),
    ) -> Result<SyncReport> {
        let _targets = sides
            .iter()
            .map(|&side| storage::target_lock(&self.root, side))
            .collect::<Result<Vec<_>>>()?;
        manifest.validate()?;
        lock.validate()?;
        ensure!(
            storage::read_optional(&self.root, "enderpin.toml")?.as_deref()
                == Some(&self.original_manifest),
            "enderpin.toml changed while planning; retry"
        );
        ensure!(
            storage::read_optional(&self.root, "enderpin.lock")? == self.original_lock,
            "enderpin.lock changed while planning; retry"
        );
        let ignores = if options.no_ignore {
            String::new()
        } else {
            storage::read_optional(&self.root, ".enderpinignore")?.unwrap_or_default()
        };
        let mut report = SyncReport {
            targets: sides.to_vec(),
            ..Default::default()
        };
        let mut changes = vec![];
        for &side in sides {
            let target = lock.targets.get(&side).context("target is not locked")?;
            ensure!(
                target.fingerprint == manifest.fingerprint(side)?,
                "plan does not match {} configuration",
                side.name()
            );
            let desired = target.effective(&parse_ignores(&ignores, side)?)?;
            if options.lock_only {
                continue;
            }
            let state_path = format!(".enderpin/{}/state.toml", side.name());
            let old: InstalledState = storage::read_optional(&self.root, &state_path)?
                .map(|s| toml::from_str(&s))
                .transpose()
                .context("invalid installed state")?
                .unwrap_or(InstalledState {
                    format: FORMAT,
                    files: BTreeMap::new(),
                });
            ensure!(old.format == FORMAT, "unsupported installed state format");
            let mut actual = BTreeMap::new();
            for (path, record) in &old.files {
                storage::valid_destination(path)?;
                ensure!(
                    path.starts_with(&format!(".enderpin/{}/game/", side.name())),
                    "installed state contains a different target"
                );
                validate_hash(&record.sha512)?;
                let file = storage::safe_path(&self.root, path)?;
                if file.try_exists()? {
                    let hash = storage::hash_file(&file)?;
                    ensure!(
                        options.restore || hash == (record.sha512.clone(), record.size),
                        "managed file was modified: {path}; use restore explicitly to discard local changes"
                    );
                    actual.insert(path.clone(), hash);
                }
            }
            let mut state = InstalledState {
                format: FORMAT,
                files: BTreeMap::new(),
            };
            // Preflight the complete target before acquiring any files for it.
            for p in &desired {
                let path = p.relative_path(side);
                let exists = storage::safe_path(&self.root, &path)?.try_exists()?;
                ensure!(
                    !exists || old.files.contains_key(&path),
                    "unmanaged filename collision: {path}"
                );
            }
            for p in desired {
                let path = p.relative_path(side);
                if actual.get(&path) == Some(&(p.sha512.clone(), p.size)) {
                    report.unchanged.push(path.clone());
                } else {
                    progress(Progress {
                        target: side,
                        package: p.name.clone(),
                        action: "acquire",
                    });
                    let cached = self
                        .cache
                        .acquire(&p, options.offline)
                        .with_context(|| format!("acquiring {}", p.name))?;
                    changes.push(Change {
                        path: path.clone(),
                        content: Content::File {
                            path: cached,
                            sha512: p.sha512.clone(),
                        },
                    });
                    report.added.push(path.clone());
                }
                state.files.insert(
                    path,
                    InstalledFile {
                        sha512: p.sha512,
                        size: p.size,
                    },
                );
            }
            for path in old.files.keys() {
                if !state.files.contains_key(path) && actual.contains_key(path) {
                    changes.push(Change {
                        path: path.clone(),
                        content: Content::Delete,
                    });
                    report.removed.push(path.clone());
                }
            }
            let new_state = toml::to_string_pretty(&state)?;
            if storage::read_optional(&self.root, &state_path)?.as_deref() != Some(&new_state) {
                changes.push(Change {
                    path: state_path,
                    content: Content::Bytes(new_state.into_bytes()),
                });
            }
        }
        // Preserve a user's TOML formatting/comments unless this operation edits its meaning.
        let manifest_text = if serde_json::to_vec(&manifest)? == serde_json::to_vec(&self.manifest)?
        {
            self.original_manifest.clone()
        } else {
            toml::to_string_pretty(&manifest)?
        };
        let lock_text = toml::to_string_pretty(&lock)?;
        if manifest_text != self.original_manifest {
            changes.push(Change {
                path: "enderpin.toml".into(),
                content: Content::Bytes(manifest_text.clone().into_bytes()),
            });
        }
        if self.original_lock.as_deref() != Some(&lock_text) {
            report.lock_updated = true;
            changes.push(Change {
                path: "enderpin.lock".into(),
                content: Content::Bytes(lock_text.clone().into_bytes()),
            });
        }
        if !changes.is_empty() {
            storage::commit(&self.root, changes)?;
        }
        self.manifest = manifest;
        self.lockfile = lock;
        self.original_manifest = manifest_text;
        self.original_lock = Some(lock_text);
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn running_target_blocks_its_sync_but_not_the_other_target() -> Result<()> {
        let root = tempfile::tempdir()?;
        let cache = tempfile::tempdir()?;
        Workspace::init(root.path(), "1.21.1".into())?;
        let mut ws = Workspace::open(root.path(), cache.path())?;
        let running = storage::target_lock(&ws.root, Side::Server)?;
        let options = SyncOptions {
            locked: true,
            offline: true,
            ..Default::default()
        };
        assert!(
            ws.sync(ws.manifest.clone(), &[Side::Server], options, |_| ())
                .is_err()
        );
        ws.sync(ws.manifest.clone(), &[Side::Client], options, |_| ())?;
        drop(running);
        ws.sync(ws.manifest.clone(), &[Side::Server], options, |_| ())?;
        Ok(())
    }
    use crate::model::{Kind, LockedPackage, Package};
    #[test]
    fn locked_restore_ignore_and_unmanaged_files() -> Result<()> {
        let root = tempfile::tempdir()?;
        let cache_dir = tempfile::tempdir()?;
        Workspace::init(root.path(), "1.21.1".into())?;
        let cache = Cache::open(cache_dir.path())?;
        let hash = cache.seed(b"test artifact")?;
        let mut ws = Workspace::open(root.path(), cache_dir.path())?;
        let mut manifest = ws.manifest.clone();
        let mut package = Package::modrinth("example", Kind::Mod);
        package.modrinth = None;
        package.url = Some("https://example.org/example.jar".into());
        manifest.common.insert("example".into(), package);
        let mut lock = ws.lockfile.clone();
        for side in Side::ALL {
            let target = lock.targets.get_mut(&side).context("missing target")?;
            target.fingerprint = manifest.fingerprint(side)?;
            target.requests = manifest.requests(side);
            target.packages = vec![LockedPackage {
                key: "url:example".into(),
                name: "example".into(),
                kind: Kind::Mod,
                project: None,
                version: None,
                version_number: None,
                url: "https://example.org/example.jar".into(),
                filename: "example.jar".into(),
                sha512: hash.clone(),
                size: 13,
                default_sides: Side::ALL.to_vec(),
                roots: vec!["example".into()],
                required: vec![],
                incompatible: vec![],
                optional: vec![],
            }];
        }
        let options = SyncOptions {
            offline: true,
            ..Default::default()
        };
        ws.apply(manifest.clone(), lock, &Side::ALL, options, |_| ())?;
        let client = root.path().join(".enderpin/client/game/mods/example.jar");
        let server = root.path().join(".enderpin/server/game/mods/example.jar");
        fs::write(&client, b"manual edit")?;
        let locked = SyncOptions {
            locked: true,
            offline: true,
            ..Default::default()
        };
        assert!(
            ws.sync(manifest.clone(), &[Side::Client], locked, |_| ())
                .is_err()
        );
        ws.sync(
            manifest.clone(),
            &[Side::Client],
            SyncOptions {
                restore: true,
                ..locked
            },
            |_| (),
        )?;
        assert_eq!(fs::read(&client)?, b"test artifact");
        fs::write(root.path().join(".enderpinignore"), "client:example\n")?;
        ws.sync(manifest.clone(), &[Side::Client], locked, |_| ())?;
        assert!(!client.exists());
        assert!(server.exists());
        ws.sync(
            manifest,
            &[Side::Client],
            SyncOptions {
                no_ignore: true,
                ..locked
            },
            |_| (),
        )?;
        assert!(client.exists());
        let before = fs::read(root.path().join("enderpin.lock"))?;
        let mut edited = ws.manifest.clone();
        edited.minecraft = "1.20.1".into();
        assert!(ws.sync(edited, &[Side::Client], locked, |_| ()).is_err());
        assert_eq!(fs::read(root.path().join("enderpin.lock"))?, before);
        Ok(())
    }
}
