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
    assert!(run(&["init", "--minecraft", "1.21.1"])?.status.success());
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
    assert!(!run(&["--yes", "permissions", "approve"])?.status.success());
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
    Ok(())
}
