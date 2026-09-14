use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};

use crate::{
    model::{Dependency, Kind, LockedPackage, Manifest, Package, Side, TargetLock, filename},
    registry::{ApiDependency, Modrinth, Version},
    storage::Cache,
};

const MAX_PACKAGES: usize = 512;

pub fn resolve(
    manifest: &Manifest,
    side: Side,
    old: Option<&TargetLock>,
    update: bool,
    registry: &mut Modrinth,
    cache: &Cache,
) -> Result<TargetLock> {
    manifest.validate()?;
    let old = old.filter(|old| {
        old.minecraft == manifest.minecraft && old.loader == manifest.target(side).loader
    });
    let mut resolver = Resolver {
        manifest,
        side,
        old,
        update,
        registry,
        nodes: BTreeMap::new(),
        visiting: BTreeSet::new(),
    };
    let requests = manifest.requests(side);
    for (alias, package) in &requests {
        let explicitly_enabled = manifest.target(side).packages.contains_key(alias)
            || manifest.target(side).enabled.contains(alias);
        if resolver.disabled(&[alias]) {
            continue;
        }
        let loader = &manifest.target(side).loader;
        let wrong_runtime = (package.kind == Kind::Mod && loader == "paper")
            || (package.kind == Kind::Plugin && loader != "paper");
        if wrong_runtime {
            ensure!(
                !explicitly_enabled,
                "{} cannot load {:?} package {alias}",
                loader,
                package.kind
            );
            continue;
        }
        let previous = old.and_then(|old| old.packages.iter().find(|p| p.roots.contains(alias)));
        if let Some(previous) = previous
            .filter(|_| !update || package.url.is_some())
            .filter(|_| old.is_some_and(|old| old.requests.get(alias) == Some(package)))
        {
            if resolver.disabled(&[
                &previous.name,
                previous.project.as_deref().unwrap_or_default(),
            ]) {
                continue;
            }
            if explicitly_enabled
                || manifest
                    .target(side)
                    .enabled
                    .iter()
                    .any(|id| previous.matches(id))
                || previous.default_sides.contains(&side)
            {
                resolver.reuse(&previous.key)?;
                resolver.add_root(&previous.key, alias)?;
            }
            continue;
        }
        if let Some(url) = &package.url {
            let defaults = match package.kind {
                Kind::Mod => Side::ALL.to_vec(),
                Kind::Plugin => vec![Side::Server],
                _ => vec![Side::Client],
            };
            if !explicitly_enabled && !defaults.contains(&side) {
                continue;
            }
            let parsed = crate::model::https_url(url)?;
            let file_name = package
                .filename
                .clone()
                .or_else(|| {
                    parsed
                        .path_segments()
                        .and_then(|mut s| s.next_back())
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                })
                .context("URL has no filename; specify filename")?;
            filename(&file_name, package.kind)?;
            let reused = previous.filter(|p| p.url == *url);
            let (hash, size) = if let Some(p) = reused {
                (p.sha512.clone(), p.size)
            } else {
                cache.acquire_url(url)?
            };
            let key = format!("url:{alias}");
            resolver.nodes.insert(
                key.clone(),
                LockedPackage {
                    key,
                    name: alias.clone(),
                    kind: package.kind,
                    project: None,
                    version: None,
                    version_number: None,
                    url: url.clone(),
                    filename: file_name,
                    sha512: hash,
                    size,
                    default_sides: defaults,
                    roots: vec![alias.clone()],
                    required: vec![],
                    incompatible: vec![],
                    optional: vec![],
                },
            );
        } else {
            let project_id = package
                .modrinth
                .as_deref()
                .context("missing package source")?;
            resolver.project(
                project_id,
                package,
                Some(alias),
                explicitly_enabled,
                package.version.as_deref(),
            )?;
        }
    }
    for p in resolver.nodes.values() {
        for conflict in &p.incompatible {
            for other in resolver.nodes.values() {
                let matches = if let Some(version) = &conflict.version {
                    other.version.as_ref() == Some(version)
                } else {
                    conflict.project.is_some() && other.project == conflict.project
                };
                ensure!(
                    !matches,
                    "{} is incompatible with {} ({})",
                    p.name,
                    other.name,
                    other.version_number.as_deref().unwrap_or("URL")
                );
            }
        }
    }
    let lock = TargetLock {
        fingerprint: manifest.fingerprint(side)?,
        minecraft: manifest.minecraft.clone(),
        loader: manifest.target(side).loader.clone(),
        requests,
        packages: resolver.nodes.into_values().collect(),
    };
    lock.validate()?;
    Ok(lock)
}

struct Resolver<'a> {
    manifest: &'a Manifest,
    side: Side,
    old: Option<&'a TargetLock>,
    update: bool,
    registry: &'a mut Modrinth,
    nodes: BTreeMap<String, LockedPackage>,
    visiting: BTreeSet<String>,
}

impl Resolver<'_> {
    fn disabled(&self, names: &[&str]) -> bool {
        self.manifest
            .target(self.side)
            .disabled
            .iter()
            .any(|d| names.contains(&d.as_str()))
    }
    fn add_root(&mut self, key: &str, alias: &str) -> Result<()> {
        let node = self
            .nodes
            .get_mut(key)
            .context("resolved root is missing")?;
        if !node.roots.iter().any(|root| root == alias) {
            node.roots.push(alias.into());
            node.roots.sort();
        }
        Ok(())
    }
    fn reuse(&mut self, key: &str) -> Result<()> {
        let mut package = self
            .old
            .and_then(|old| old.packages.iter().find(|p| p.key == key))
            .context("old dependency missing from lock")?
            .clone();
        if let Some(selected) = self.nodes.get(key) {
            ensure!(
                selected.version == package.version && selected.sha512 == package.sha512,
                "version conflict with locked package {}",
                package.name
            );
            return Ok(());
        }
        ensure!(
            self.nodes.len() + self.visiting.len() < MAX_PACKAGES,
            "dependency limit exceeded"
        );
        ensure!(
            self.visiting.insert(key.into()),
            "dependency cycle at {key}"
        );
        ensure!(
            !self.disabled(&[
                &package.name,
                package.project.as_deref().unwrap_or_default()
            ]),
            "required dependency {} is disabled",
            package.name
        );
        let requests = self.manifest.requests(self.side);
        package.required.retain(|key| {
            let dep = self
                .old
                .and_then(|old| old.packages.iter().find(|p| &p.key == key));
            let optional = dep.is_some_and(|dep| {
                package.optional.iter().any(|opt| {
                    opt.project == dep.project
                        || opt.version.is_some() && opt.version == dep.version
                })
            });
            !optional
                || requests.iter().any(|(name, spec)| {
                    (package.roots.contains(name)
                        || spec.modrinth == package.project
                        || spec.modrinth.as_deref() == Some(&package.name))
                        && dep.is_some_and(|dep| {
                            spec.optional
                                .iter()
                                .any(|id| dep.matches(id) || dep.version.as_ref() == Some(id))
                        })
                })
        });
        for dependency in &package.required {
            self.reuse(dependency)?;
        }
        package.roots.clear();
        self.visiting.remove(key);
        self.nodes.insert(key.into(), package);
        Ok(())
    }
    fn project(
        &mut self,
        id: &str,
        spec: &Package,
        root: Option<&str>,
        force: bool,
        exact: Option<&str>,
    ) -> Result<Option<String>> {
        let project = self.registry.project(id)?;
        if self.disabled(&[id, &project.id, &project.slug]) {
            ensure!(
                root.is_some(),
                "required dependency {} is disabled",
                project.slug
            );
            return Ok(None);
        }
        let key = format!("modrinth:{}", project.id);
        let force = force
            || self
                .manifest
                .target(self.side)
                .enabled
                .iter()
                .any(|id| id == &project.id || id == &project.slug);
        if let Some(node) = self.nodes.get(&key) {
            if let Some(exact) = exact {
                ensure!(
                    node.version.as_deref() == Some(exact),
                    "version conflict for {}: {} vs {exact}",
                    project.slug,
                    node.version.as_deref().unwrap_or_default()
                );
            }
            ensure!(
                node.kind == spec.kind,
                "conflicting kinds for {}",
                project.slug
            );
            if let Some(root) = root {
                ensure!(
                    node.roots.is_empty(),
                    "{} is declared more than once; keep one package name per target",
                    project.slug
                );
                let version = node.version.clone().context("missing Modrinth version")?;
                self.nodes.remove(&key);
                return self.project(id, spec, Some(root), force, Some(&version));
            }
            return Ok(Some(key));
        }
        if root.is_none()
            && !self.update
            && let Some(old) = self.old.and_then(|old| {
                old.packages.iter().find(|p| {
                    p.key == key && exact.is_none_or(|id| p.version.as_deref() == Some(id))
                })
            })
        {
            ensure!(old.kind == spec.kind, "conflicting dependency kind");
            self.reuse(&key)?;
            return Ok(Some(key));
        }
        ensure!(
            project.project_type == spec.kind.api_type(),
            "project {} has type {}, not {}",
            project.slug,
            project.project_type,
            spec.kind.api_type()
        );
        let target = self.manifest.target(self.side);
        let preserved = self
            .old
            .and_then(|old| {
                old.packages
                    .iter()
                    .find(|p| p.key == key && p.kind == spec.kind)
            })
            .and_then(|p| p.version.as_deref());
        let exact = exact.or(if self.update { None } else { preserved });
        let version = if let Some(id) = exact {
            self.registry.version(id)?
        } else {
            self.registry
                .versions(&project.id)?
                .into_iter()
                .find(|v| {
                    v.compatible(&self.manifest.minecraft, &target.loader, spec.kind)
                        && (spec.prerelease || v.version_type == "release")
                })
                .with_context(|| {
                    format!(
                        "no compatible {} release for {} on Minecraft {} / {}",
                        spec.kind.api_type(),
                        project.slug,
                        self.manifest.minecraft,
                        target.loader
                    )
                })?
        };
        ensure!(
            version.project_id == project.id,
            "version belongs to a different project"
        );
        ensure!(
            version.compatible(&self.manifest.minecraft, &target.loader, spec.kind),
            "{} {} is incompatible with Minecraft {} / {}",
            project.slug,
            version.version_number,
            self.manifest.minecraft,
            target.loader
        );
        if root.is_some() && !force && !version.default_enabled(&project, self.side, spec.kind) {
            return Ok(None);
        }
        ensure!(
            self.nodes.len() + self.visiting.len() < MAX_PACKAGES,
            "dependency limit exceeded"
        );
        ensure!(
            self.visiting.insert(key.clone()),
            "dependency cycle at {}",
            project.slug
        );
        let file = version.file(spec.kind, spec.filename.as_deref())?;
        let mut node = LockedPackage {
            key: key.clone(),
            name: project.slug.clone(),
            kind: spec.kind,
            project: Some(project.id.clone()),
            version: Some(version.id.clone()),
            version_number: Some(version.version_number.clone()),
            url: file.url.clone(),
            filename: file.filename.clone(),
            sha512: file
                .hashes
                .get("sha512")
                .context("missing SHA-512")?
                .clone(),
            size: file.size,
            default_sides: Side::ALL
                .into_iter()
                .filter(|s| version.default_enabled(&project, *s, spec.kind))
                .collect(),
            roots: root.into_iter().map(str::to_owned).collect(),
            required: vec![],
            incompatible: vec![],
            optional: vec![],
        };
        let mut selected_optional = BTreeSet::new();
        for dependency in &version.dependencies {
            let reference = Dependency {
                project: dependency.project_id.clone(),
                version: dependency.version_id.clone(),
            };
            match dependency.dependency_type.as_str() {
                "embedded" => (),
                "incompatible" => {
                    ensure!(
                        reference.project.is_some() || reference.version.is_some(),
                        "unidentified incompatible dependency for {}",
                        project.slug
                    );
                    node.incompatible.push(reference);
                }
                "required" | "optional" => {
                    let (dep_project, dep_version) =
                        self.dependency_identity(dependency, &version)?;
                    let dep_info = self.registry.project(&dep_project)?;
                    let selected = spec.optional.iter().find(|value| {
                        *value == &dep_project
                            || *value == &dep_info.slug
                            || dep_version.as_ref() == Some(value)
                    });
                    if dependency.dependency_type == "optional" {
                        node.optional.push(Dependency {
                            project: Some(dep_project.clone()),
                            version: dep_version.clone(),
                        });
                        if let Some(selected) = selected {
                            selected_optional.insert(selected.clone());
                        } else {
                            continue;
                        }
                    }
                    let kind = match dep_info.project_type.as_str() {
                        "resourcepack" => Kind::Resourcepack,
                        "shader" => Kind::Shader,
                        "mod" if target.loader == "paper" => Kind::Plugin,
                        "mod" => Kind::Mod,
                        other => bail!("unsupported dependency type {other}"),
                    };
                    let mut dep_spec = Package::modrinth(&dep_project, kind);
                    dep_spec.prerelease = spec.prerelease;
                    let dependency_key = self
                        .project(&dep_project, &dep_spec, None, true, dep_version.as_deref())?
                        .context("required dependency was excluded")?;
                    if !node.required.contains(&dependency_key) {
                        node.required.push(dependency_key);
                    }
                }
                other => bail!("unsupported dependency type {other}"),
            }
        }
        for requested in &spec.optional {
            ensure!(
                selected_optional.contains(requested),
                "{} is not an optional dependency of {} {}",
                requested,
                project.slug,
                version.version_number
            );
        }
        node.required.sort();
        self.visiting.remove(&key);
        self.nodes.insert(key.clone(), node);
        Ok(Some(key))
    }
    fn dependency_identity(
        &mut self,
        dependency: &ApiDependency,
        parent: &Version,
    ) -> Result<(String, Option<String>)> {
        if let Some(version_id) = &dependency.version_id {
            let version = self.registry.version(version_id)?;
            if let Some(project_id) = &dependency.project_id {
                ensure!(
                    &version.project_id == project_id,
                    "dependency project/version mismatch"
                );
            }
            Ok((version.project_id, Some(version_id.clone())))
        } else {
            Ok((dependency.project_id.clone().with_context(|| format!("{} has a dependency without a project/version ID; add a resolvable version instead", parent.version_number))?, None))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::hash_bytes,
        registry::{ApiFile, Project},
    };
    fn fixture(
        registry: &mut Modrinth,
        id: &str,
        dependencies: Vec<ApiDependency>,
        environment: &str,
    ) {
        registry.projects.insert(
            id.into(),
            Project {
                id: id.into(),
                slug: id.into(),
                title: id.into(),
                description: String::new(),
                project_type: "mod".into(),
                client_side: "required".into(),
                server_side: "required".into(),
            },
        );
        let version = Version {
            id: format!("{id}-v1"),
            project_id: id.into(),
            version_number: "1.0".into(),
            version_type: "release".into(),
            date_published: "2026-01-01".into(),
            game_versions: vec!["1.21.1".into()],
            loaders: vec!["fabric".into()],
            environment: environment.into(),
            dependencies,
            files: vec![ApiFile {
                hashes: BTreeMap::from([("sha512".into(), hash_bytes(id.as_bytes()))]),
                url: format!("https://example.org/{id}.jar"),
                filename: format!("{id}.jar"),
                primary: true,
                size: id.len() as u64,
            }],
        };
        registry
            .versions
            .insert(version.id.clone(), version.clone());
        registry.project_versions.insert(id.into(), vec![version]);
    }
    fn dep(id: &str, kind: &str) -> ApiDependency {
        ApiDependency {
            version_id: None,
            project_id: Some(id.into()),
            dependency_type: kind.into(),
        }
    }
    #[test]
    fn sides_overrides_optional_ignore_and_cycles() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let cache = Cache::open(temp.path())?;
        let mut registry = Modrinth::default();
        fixture(&mut registry, "base", vec![], "client_and_server");
        fixture(&mut registry, "extra", vec![], "client_only");
        fixture(
            &mut registry,
            "app",
            vec![dep("base", "required"), dep("extra", "optional")],
            "client_only",
        );
        let mut m = Manifest::new("1.21.1".into())?;
        m.common
            .insert("app".into(), Package::modrinth("app", Kind::Mod));
        let client = resolve(&m, Side::Client, None, false, &mut registry, &cache)?;
        assert_eq!(client.packages.len(), 2);
        assert!(client.effective(&["base".into()]).is_err());
        assert!(client.effective(&["app".into()])?.is_empty());
        assert!(
            resolve(&m, Side::Server, None, false, &mut registry, &cache)?
                .packages
                .is_empty()
        );
        m.server.enabled.push("app".into());
        assert_eq!(
            resolve(&m, Side::Server, None, false, &mut registry, &cache)?
                .packages
                .len(),
            2
        );
        m.common
            .get_mut("app")
            .context("app missing")?
            .optional
            .push("extra".into());
        assert_eq!(
            resolve(
                &m,
                Side::Client,
                Some(&client),
                false,
                &mut registry,
                &cache
            )?
            .packages
            .len(),
            3
        );
        fixture(
            &mut registry,
            "base",
            vec![dep("app", "required")],
            "client_and_server",
        );
        assert!(resolve(&m, Side::Client, None, false, &mut registry, &cache).is_err());
        Ok(())
    }
    #[test]
    fn pinned_versions_and_conflicts_are_enforced() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let cache = Cache::open(temp.path())?;
        let mut registry = Modrinth::default();
        fixture(
            &mut registry,
            "a",
            vec![dep("b", "incompatible")],
            "client_and_server",
        );
        fixture(&mut registry, "b", vec![], "client_and_server");
        let mut m = Manifest::new("1.21.1".into())?;
        m.client
            .packages
            .insert("a".into(), Package::modrinth("a", Kind::Mod));
        m.client
            .packages
            .insert("b".into(), Package::modrinth("b", Kind::Mod));
        assert!(resolve(&m, Side::Client, None, false, &mut registry, &cache).is_err());
        m.client.packages.remove("a");
        let lock = resolve(&m, Side::Client, None, false, &mut registry, &cache)?;
        // A normal sync can preserve this exact graph without contacting the API.
        assert_eq!(
            resolve(
                &m,
                Side::Client,
                Some(&lock),
                false,
                &mut Modrinth::default(),
                &cache
            )?
            .packages[0]
                .version,
            Some("b-v1".into())
        );
        Ok(())
    }

    #[test]
    fn removing_optional_owner_prunes_unused_dependency() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let cache = Cache::open(temp.path())?;
        let mut registry = Modrinth::default();
        fixture(
            &mut registry,
            "a",
            vec![dep("b", "required")],
            "client_and_server",
        );
        fixture(
            &mut registry,
            "b",
            vec![dep("c", "optional")],
            "client_and_server",
        );
        fixture(&mut registry, "c", vec![], "client_and_server");
        let mut m = Manifest::new("1.21.1".into())?;
        m.client
            .packages
            .insert("a".into(), Package::modrinth("a", Kind::Mod));
        let mut b = Package::modrinth("b", Kind::Mod);
        b.optional.push("c".into());
        m.client.packages.insert("b".into(), b);
        let old = resolve(&m, Side::Client, None, false, &mut registry, &cache)?;
        assert_eq!(old.packages.len(), 3);
        m.client.packages.remove("b");
        let next = resolve(
            &m,
            Side::Client,
            Some(&old),
            false,
            &mut Modrinth::default(),
            &cache,
        )?;
        assert_eq!(next.packages.len(), 2);
        assert!(!next.packages.iter().any(|p| p.name == "c"));
        Ok(())
    }
}
