use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use reqwest::Url;
use serde::Deserialize;

use super::{Algorithm, Download};
use crate::{
    model::{ResolvedTarget, identifier},
    registry,
};

#[derive(Deserialize)]
struct VersionResponse {
    version: ProjectVersion,
}
#[derive(Deserialize)]
struct ProjectVersion {
    id: String,
    java: Java,
}
#[derive(Deserialize)]
struct Java {
    version: JavaVersion,
}
#[derive(Deserialize)]
struct JavaVersion {
    minimum: u32,
}
#[derive(Deserialize)]
struct Build {
    id: u64,
    channel: String,
    downloads: BTreeMap<String, Artifact>,
}
#[derive(Deserialize)]
struct Artifact {
    name: String,
    url: String,
    size: u64,
    checksums: BTreeMap<String, String>,
}

fn selection(manifest: &ResolvedTarget) -> Result<(String, Option<u64>)> {
    let target = manifest;
    let (version, build) = if target.loader == "paper" {
        (
            manifest.minecraft.as_str(),
            target.loader_version.as_deref(),
        )
    } else {
        let configured = target
            .loader_version
            .as_deref()
            .context("Velocity requires loader_version = '<version>' or '<version>+<build>'")?;
        configured
            .split_once('+')
            .map_or((configured, None), |(v, b)| (v, Some(b)))
    };
    identifier(version)?;
    let build = build
        .map(|value| -> Result<u64> {
            let id: u64 = value
                .parse()
                .context("PaperMC build must be a positive integer")?;
            ensure!(
                id > 0 && id.to_string() == value,
                "PaperMC build must be a positive integer"
            );
            Ok(id)
        })
        .transpose()?;
    Ok((version.into(), build))
}

fn artifact(mut builds: Vec<Build>, requested: Option<u64>) -> Result<(u64, Download)> {
    builds.sort_unstable_by_key(|build| std::cmp::Reverse(build.id));
    let build = builds
        .into_iter()
        .find(|build| requested.map_or(build.channel == "STABLE", |id| build.id == id))
        .context("requested PaperMC build is unavailable (no automatic version fallback)")?;
    ensure!(build.id > 0, "invalid PaperMC build id");
    let artifact = build
        .downloads
        .get("server:default")
        .context("PaperMC server artifact is missing")?;
    let url = crate::model::https_url(&artifact.url)?;
    ensure!(
        url.host_str() == Some("fill-data.papermc.io"),
        "unexpected PaperMC download host"
    );
    let download = Download {
        url: artifact.url.clone(),
        algorithm: Algorithm::Sha256,
        hash: artifact
            .checksums
            .get("sha256")
            .context("PaperMC checksum is missing")?
            .clone(),
        filename: artifact.name.clone(),
        size: Some(artifact.size),
    };
    download.validate()?;
    Ok((build.id, download))
}

pub(super) fn resolve(manifest: &ResolvedTarget) -> Result<(String, Download, u32)> {
    let (version, requested) = selection(manifest)?;
    let base = format!(
        "https://fill.papermc.io/v3/projects/{}/versions/{version}",
        manifest.loader
    );
    let info: VersionResponse = registry::json(&Url::parse(&base)?)?;
    ensure!(
        info.version.id == version,
        "PaperMC version identity mismatch"
    );
    let major = info.version.java.version.minimum;
    ensure!(
        (8..=50).contains(&major),
        "invalid PaperMC Java requirement"
    );
    let builds: Vec<Build> = registry::json(&Url::parse(&format!("{base}/builds"))?)?;
    let (id, download) = artifact(builds, requested)?;
    let pinned = if manifest.loader == "paper" {
        id.to_string()
    } else {
        format!("{version}+{id}")
    };
    Ok((pinned, download, major))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builds() -> Result<Vec<Build>> {
        Ok(serde_json::from_value(serde_json::json!([
            {"id": 4, "channel": "STABLE", "downloads": {"server:default": {
                "name":"server.jar", "url":"https://fill-data.papermc.io/server.jar",
                "size":42, "checksums":{"sha256":"a".repeat(64)} }}},
            {"id": 6, "channel": "EXPERIMENTAL", "downloads": {"server:default": {
                "name":"server.jar", "url":"https://fill-data.papermc.io/server.jar",
                "size":42, "checksums":{"sha256":"b".repeat(64)} }}}
        ]))?)
    }
    #[test]
    fn stable_default_exact_pin_and_missing_build() -> Result<()> {
        assert_eq!(artifact(builds()?, None)?.0, 4);
        assert_eq!(artifact(builds()?, Some(6))?.0, 6);
        assert!(artifact(builds()?, Some(5)).is_err());
        let mut invalid = builds()?;
        invalid[0]
            .downloads
            .get_mut("server:default")
            .context("artifact")?
            .url = "https://example.com/server.jar".into();
        assert!(artifact(invalid, Some(4)).is_err());
        Ok(())
    }
    #[test]
    fn independent_proxy_version_and_paper_build() -> Result<()> {
        let mut manifest = crate::Manifest::new("1.21.1".into())?.target("server")?;
        manifest.loader = "velocity".into();
        assert!(selection(&manifest).is_err());
        manifest.loader_version = Some("3.4.0-SNAPSHOT+563".into());
        assert_eq!(selection(&manifest)?, ("3.4.0-SNAPSHOT".into(), Some(563)));
        manifest.loader_version = Some("3.4.0-SNAPSHOT+0".into());
        assert!(selection(&manifest).is_err());
        manifest.loader = "paper".into();
        manifest.loader_version = Some("133".into());
        assert_eq!(selection(&manifest)?, ("1.21.1".into(), Some(133)));
        Ok(())
    }
}
