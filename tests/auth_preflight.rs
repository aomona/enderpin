use anyhow::Result;
use enderpin::{
    Manifest, Workspace,
    model::Lockfile,
    runtime::{Algorithm, Download, JavaLock, RuntimeLock},
};
use std::{collections::BTreeMap, fs, process::Command};

#[test]
fn init_rejects_unsupported_authentication_before_mod_resolution_or_writing() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cache = tempfile::tempdir()?;
    let output = Command::new(env!("CARGO_BIN_EXE_enderpin"))
        .arg("-C")
        .arg(root.path())
        .arg("--cache-dir")
        .arg(cache.path())
        .args([
            "init",
            "--minecraft",
            "26.3",
            "--loader",
            "fabric",
            "--loader-version",
            "0.99.0",
            "--mods",
            "nonexistent-preflight-test-mod",
            "--no-interactive",
        ])
        .output()?;
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr)?;
    assert!(error.contains("does not support fabric 0.99.0"), "{error}");
    assert!(!root.path().join("enderpin.toml").exists());
    assert!(!root.path().join("enderpin.lock").exists());
    assert_eq!(fs::read_dir(cache.path())?.count(), 0);
    Ok(())
}

#[test]
fn sync_checks_all_targets_before_package_or_runtime_downloads() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cache = tempfile::tempdir()?;
    let mut manifest = Manifest::new("26.3".into())?;
    manifest.target_mut("client")?.loader_version = Some("0.99.0".into());
    Workspace::init_manifest(root.path(), manifest, &["server", "client"])?;
    let manifest_before = fs::read(root.path().join("enderpin.toml"))?;
    let lock_before = fs::read(root.path().join("enderpin.lock"))?;
    let output = Command::new(env!("CARGO_BIN_EXE_enderpin"))
        .arg("-C")
        .arg(root.path())
        .arg("--cache-dir")
        .arg(cache.path())
        .args(["sync", "--target", "all"])
        .output()?;
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr)?;
    assert!(error.contains("does not support fabric 0.99.0"), "{error}");
    assert_eq!(
        fs::read(root.path().join("enderpin.toml"))?,
        manifest_before
    );
    assert_eq!(fs::read(root.path().join("enderpin.lock"))?, lock_before);
    assert!(!error.contains("Minecraft metadata"), "{error}");
    Ok(())
}

#[test]
fn run_rejects_unsupported_authentication_before_account_work() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cache = tempfile::tempdir()?;
    let mut manifest = Manifest::new("26.3".into())?;
    manifest.target_mut("client")?.loader_version = Some("0.99.0".into());
    Workspace::init_manifest(root.path(), manifest, &["client"])?;
    let output = Command::new(env!("CARGO_BIN_EXE_enderpin"))
        .arg("-C")
        .arg(root.path())
        .arg("--cache-dir")
        .arg(cache.path())
        .args(["run", "--target", "client"])
        .output()?;
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr)?;
    assert!(error.contains("does not support fabric 0.99.0"), "{error}");
    Ok(())
}

#[test]
fn demo_skips_authentication_checks_but_preserves_locks_and_permissions() -> Result<()> {
    for (minecraft, loader, version) in [
        ("1.19.4", "vanilla", None),
        ("1.21.1", "fabric", Some("0.16.10")),
    ] {
        let root = tempfile::tempdir()?;
        let cache = tempfile::tempdir()?;
        let mut manifest = Manifest::new(minecraft.into())?;
        let client = manifest.target_mut("client")?;
        client.loader = loader.into();
        client.loader_version = version.map(str::to_owned);
        Workspace::init_manifest(root.path(), manifest.clone(), &["client"])?;
        let run = |arguments: &[&str]| {
            Command::new(env!("CARGO_BIN_EXE_enderpin"))
                .arg("-C")
                .arg(root.path())
                .arg("--cache-dir")
                .arg(cache.path())
                .args(arguments)
                .output()
        };
        let download = Download {
            url: "https://example.org/fixture".into(),
            algorithm: Algorithm::Sha256,
            hash: "a".repeat(64),
            filename: "fixture.jar".into(),
            size: Some(1),
        };
        let runtime = RuntimeLock {
            fingerprint: RuntimeLock::fingerprint(&manifest, "client")?,
            minecraft: minecraft.into(),
            loader: loader.into(),
            loader_version: version.unwrap_or(minecraft).into(),
            metadata: download.clone(),
            java: JavaLock {
                release: "fixture".into(),
                major: 21,
                platforms: BTreeMap::from([("fixture".into(), download.clone())]),
            },
            fabric_libraries: if loader == "fabric" {
                vec![download]
            } else {
                vec![]
            },
            fabric_jvm: vec![],
            server_jar: None,
        };
        let lock_path = root.path().join("enderpin.lock");
        let original_lock: Lockfile = toml::from_str(&fs::read_to_string(&lock_path)?)?;
        for state in ["missing", "stale", "current"] {
            let mut lock = original_lock.clone();
            if state != "missing" {
                let mut runtime = runtime.clone();
                if state == "stale" {
                    runtime.fingerprint = "b".repeat(128);
                }
                lock.runtimes.insert("client".into(), runtime);
            }
            fs::write(&lock_path, toml::to_string(&lock)?)?;
            let plan = run(&["permissions", "show", "--target", "client", "--json"])?;
            assert!(
                plan.status.success(),
                "{}",
                String::from_utf8_lossy(&plan.stderr)
            );
            let plan: serde_json::Value = serde_json::from_slice(&plan.stdout)?;
            let approved = run(&[
                "permissions",
                "--target",
                "client",
                "approve",
                plan["fingerprint"].as_str().expect("fingerprint"),
            ])?;
            assert!(
                approved.status.success(),
                "{}",
                String::from_utf8_lossy(&approved.stderr)
            );
            let paths = [
                root.path().join("enderpin.toml"),
                lock_path.clone(),
                root.path().join(".enderpin/client/permissions.toml"),
            ];
            let before = paths
                .iter()
                .map(fs::read)
                .collect::<std::io::Result<Vec<_>>>()?;
            let result = run(&[
                "run",
                "--target",
                "client",
                "--demo",
                "--offline",
                "--no-interactive",
            ])?;
            let error = String::from_utf8(result.stderr)?;
            let expected = if state == "current" {
                "runtime file is missing from cache (offline)"
            } else {
                "runtime is not locked"
            };
            assert!(
                !result.status.success() && error.contains(expected),
                "{minecraft}/{loader}/{state}: {error}"
            );
            assert_eq!(
                before,
                paths
                    .iter()
                    .map(fs::read)
                    .collect::<std::io::Result<Vec<_>>>()?
            );
            let result = run(&["run", "--target", "client", "--offline", "--no-interactive"])?;
            let error = String::from_utf8(result.stderr)?;
            assert!(
                !result.status.success() && error.contains("authentication adapter"),
                "{error}"
            );
        }
    }
    Ok(())
}
