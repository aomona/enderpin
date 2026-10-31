use anyhow::Result;
use enderpin::{Manifest, Workspace};
use std::{fs, process::Command};

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
