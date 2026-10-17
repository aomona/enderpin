use anyhow::{Context, Result};
use std::{fs, process::Command};

#[test]
fn noninteractive_approval_is_explicit_and_folder_changes_revoke_it() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cache = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let binary = env!("CARGO_BIN_EXE_enderpin");
    let run = |args: &[&str]| {
        Command::new(binary)
            .arg("-C")
            .arg(root.path())
            .arg("--cache-dir")
            .arg(cache.path())
            .args(args)
            .output()
    };
    enderpin::Workspace::init(root.path(), "1.21.1".into())?;
    let show = || -> Result<serde_json::Value> {
        let output = run(&["permissions", "show", "--json"])?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(serde_json::from_slice(&output.stdout)?)
    };
    assert_eq!(show()?["approved"], false);
    assert!(!run(&["permissions", "approve", "--yes"])?.status.success());
    assert!(
        !run(&["permissions", "approve", "wrong-fingerprint"])?
            .status
            .success()
    );
    assert!(
        !root
            .path()
            .join(".enderpin/client/permissions.toml")
            .exists()
    );
    let initial = show()?;
    assert_eq!(initial["plan"]["effective"]["files"]["game"], "read-write");
    assert_eq!(
        initial["plan"]["effective"]["files"]["read_only"],
        serde_json::json!([])
    );
    assert_eq!(initial["plan"]["effective"]["clipboard"], "system");
    let fingerprint = initial["fingerprint"]
        .as_str()
        .context("fingerprint missing")?;
    assert!(
        run(&["permissions", "approve", fingerprint])?
            .status
            .success()
    );
    assert_eq!(show()?["approved"], true);
    assert!(!String::from_utf8(run(&["permissions", "show"])?.stdout)?.contains(fingerprint));

    let folder = outside.path().canonicalize()?;
    let path = folder.to_str().context("non-UTF8 test path")?;
    assert!(
        run(&["permissions", "allow-folder", path])?
            .status
            .success()
    );
    let allowed = show()?;
    assert_eq!(allowed["approved"], false);
    assert_eq!(allowed["plan"]["folders"][0]["write"], false);
    assert!(
        !run(&["permissions", "approve", fingerprint])?
            .status
            .success()
    );
    assert!(
        run(&["permissions", "allow-folder", path, "--write"])?
            .status
            .success()
    );
    assert_eq!(show()?["plan"]["folders"][0]["write"], true);
    assert!(
        !run(&[
            "permissions",
            "allow-folder",
            root.path().to_str().context("path")?
        ])?
        .status
        .success()
    );
    // An unavailable grant must remain removable, using its recorded absolute path.
    fs::remove_dir(&folder)?;
    assert!(
        run(&["permissions", "remove-folder", path])?
            .status
            .success()
    );
    assert!(
        show()?["plan"]["folders"]
            .as_array()
            .context("folders")?
            .is_empty()
    );
    assert!(run(&["permissions", "revoke"])?.status.success());
    assert_eq!(show()?["approved"], false);

    // Changing the shared file policy requires approval, not package resolution.
    assert!(
        run(&["permissions", "approve", fingerprint])?
            .status
            .success()
    );
    assert_eq!(show()?["approved"], true);
    let lock = fs::read(root.path().join("enderpin.lock"))?;
    let manifest_path = root.path().join("enderpin.toml");
    let mut manifest = enderpin::Manifest::parse(&fs::read_to_string(&manifest_path)?)?;
    manifest.client.permissions.files.game = enderpin::sandbox::permissions::FileAccess::ReadOnly;
    manifest.client.permissions.files.read_only =
        vec![enderpin::sandbox::permissions::GameDirectory::Mods];
    fs::write(&manifest_path, manifest.to_toml()?)?;
    let changed = show()?;
    assert_eq!(changed["approved"], false);
    assert_eq!(changed["plan"]["effective"]["files"]["game"], "read-only");
    assert_eq!(
        changed["plan"]["effective"]["files"]["read_only"],
        serde_json::json!(["mods"])
    );
    assert_ne!(changed["fingerprint"], initial["fingerprint"]);
    assert!(
        !run(&["permissions", "approve", fingerprint])?
            .status
            .success()
    );
    let changed_fingerprint = changed["fingerprint"]
        .as_str()
        .context("fingerprint missing")?;
    assert!(
        run(&["permissions", "approve", changed_fingerprint])?
            .status
            .success()
    );
    assert_eq!(show()?["approved"], true);
    assert_eq!(fs::read(root.path().join("enderpin.lock"))?, lock);

    // A stale table must be rejected, not ignored or silently rewritten.
    let invalid = manifest.to_toml()? + "\n[client.sandbox]\nnetwork = false\n";
    fs::write(&manifest_path, &invalid)?;
    let local_path = root.path().join(".enderpin/client/permissions.toml");
    let local = fs::read(&local_path)?;
    assert!(!run(&["permissions", "show", "--json"])?.status.success());
    assert_eq!(fs::read_to_string(&manifest_path)?, invalid);
    assert_eq!(fs::read(&local_path)?, local);
    Ok(())
}
