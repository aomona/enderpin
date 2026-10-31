use std::{collections::BTreeMap, fs, path::Path, process::Command};

use anyhow::Result;
use enderpin::{
    Manifest,
    model::{LockedPackage, Lockfile, TargetLock},
    runtime::{Algorithm, Download, JavaLock, RuntimeLock},
};
use serde_json::{Value, json};

const MANIFEST: &str = r#"
format = 4
minecraft = "1.21.1"
[groups.common.packages]
proxy = { modrinth = "fabricproxy-lite" }
[groups.unused.packages]
unused = { modrinth = "unused" }
[profiles.backend]
type = "server"
loader = "fabric"
groups = ["common", "unused"]
file_groups = ["common"]
memory_mib = 4096
jvm_args = ["-XX:+UseZGC"]
[profiles.backend.packages]
local = { modrinth = "before" }
[targets.survival]
profile = "backend"
groups = ["common"]
memory_mib = 3072
jvm_args = ["-Dinnocent=JVM_SECRET"]
disabled = ["proxy"]
[targets.survival.packages]
local = { url = "https://example.org/URL_SECRET.jar?token=QUERY_SECRET", filename = "local.jar" }
[targets.creative]
profile = "backend"
groups = []
file_groups = []
jvm_args = []
"#;

fn inspect(root: &Path, target: &str, success: bool) -> Result<Value> {
    let result = Command::new(env!("CARGO_BIN_EXE_enderpin"))
        .args(["-C", root.to_str().expect("path"), "--cache-dir"])
        .arg(root.join("unused-cache"))
        .args(["config", "show", "--resolved", "--target", target, "--json"])
        .output()?;
    assert_eq!(
        result.status.success(),
        success,
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = if success {
        assert!(result.stderr.is_empty());
        &result.stdout
    } else {
        assert!(result.stdout.is_empty());
        &result.stderr
    };
    for secret in [
        "JVM_SECRET",
        "URL_SECRET",
        "QUERY_SECRET",
        "NATIVE_SECRET",
        "PARSE_SECRET",
    ] {
        assert!(!String::from_utf8_lossy(output).contains(secret));
    }
    let document: Value = serde_json::from_slice(output)?;
    assert_eq!(document["schema_version"], 1);
    Ok(document)
}

fn snapshot(root: &Path) -> Result<BTreeMap<String, (Vec<u8>, std::time::SystemTime)>> {
    let mut result = BTreeMap::new();
    for item in fs::read_dir(root)? {
        let path = item?.path();
        let name = path
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        result.insert(
            name.clone(),
            (
                if path.is_file() {
                    fs::read(&path)?
                } else {
                    vec![]
                },
                path.metadata()?.modified()?,
            ),
        );
        if path.is_dir() {
            result.extend(
                snapshot(&path)?
                    .into_iter()
                    .map(|(key, value)| (format!("{name}/{key}"), value)),
            );
        }
    }
    Ok(result)
}

#[test]
fn resolves_named_targets_sources_replacements_and_files_without_writes() -> Result<()> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("enderpin.toml"), MANIFEST)?;
    for directory in [
        "shared/common/files",
        "survival/files",
        ".enderpin/transaction",
    ] {
        fs::create_dir_all(root.path().join(directory))?;
    }
    fs::write(root.path().join("shared/common/files/config.txt"), "shared")?;
    fs::write(
        root.path().join("survival/files/config.txt"),
        "NATIVE_SECRET",
    )?;
    fs::write(
        root.path().join(".enderpin/transaction/marker"),
        "untouched",
    )?;
    let before = snapshot(root.path())?;
    let document = inspect(root.path(), "all", true)?;
    let survival = &document["targets"]["survival"];
    assert_eq!(survival["launch"]["memory_mib"], 3072);
    assert_eq!(
        survival["launch"]["jvm_args"],
        json!(["-Dinnocent=[REDACTED]"])
    );
    assert_eq!(survival["sources"]["memory_mib"], "targets.survival");
    assert_eq!(survival["sources"]["loader"], "profiles.backend");
    assert_eq!(survival["sources"]["minecraft"], "manifest");
    assert_eq!(survival["sources"]["permissions"], "default");
    assert_eq!(
        survival["packages"]["proxy"]["sources"],
        json!(["groups.common"])
    );
    assert_eq!(
        survival["packages"]["local"]["sources"],
        json!(["targets.survival"])
    );
    assert!(
        survival["packages"]["local"]["declaration"]
            .get("modrinth")
            .is_none()
    );
    assert!(survival["packages"].get("unused").is_none());
    assert_eq!(survival["disabled"], json!(["proxy"]));
    assert_eq!(
        survival["files"]["survival/run/config.txt"],
        "survival/files/config.txt"
    );
    assert_eq!(
        survival["file_inputs"],
        json!(["shared/common/files", "survival/files"])
    );
    assert_eq!(survival["lock"]["packages"]["status"], "missing");
    let creative = &document["targets"]["creative"];
    assert_eq!(creative["launch"]["memory_mib"], 4096);
    assert_eq!(creative["launch"]["jvm_args"], json!([]));
    assert_eq!(creative["groups"], json!([]));
    assert_eq!(creative["files"], json!({}));
    assert_eq!(
        creative["packages"]["local"]["sources"],
        json!(["profiles.backend"])
    );
    assert!(document["targets"].get("server").is_none());
    assert_eq!(
        inspect(root.path(), "server", false)?["error"]["code"],
        "unknown_target"
    );
    assert_eq!(before, snapshot(root.path())?);
    Ok(())
}

#[test]
fn reports_current_stale_and_missing_locks_without_downloads() -> Result<()> {
    let root = tempfile::tempdir()?;
    let manifest_text = "format=4\nminecraft='1.21.1'\n[targets.world]\ntype='server'\nloader='paper'\n[targets.world.packages.plugin]\nkind='plugin'\nurl='https://example.org/QUERY_SECRET.jar'\nfilename='plugin.jar'\n";
    fs::write(root.path().join("enderpin.toml"), manifest_text)?;
    let manifest = Manifest::parse(manifest_text)?;
    let download = Download {
        url: "https://example.org/QUERY_SECRET".into(),
        algorithm: Algorithm::Sha256,
        hash: "a".repeat(64),
        filename: "runtime.jar".into(),
        size: Some(1),
    };
    let mut lock = Lockfile::default();
    lock.targets.insert(
        "world".into(),
        TargetLock {
            fingerprint: manifest.fingerprint("world")?,
            minecraft: "1.21.1".into(),
            loader: "paper".into(),
            requests: manifest.requests("world")?,
            packages: vec![LockedPackage {
                key: "url:plugin".into(),
                name: "plugin".into(),
                kind: enderpin::Kind::Plugin,
                project: None,
                version: None,
                version_number: Some("JVM_SECRET".into()),
                url: "https://example.org/QUERY_SECRET.jar".into(),
                filename: "plugin.jar".into(),
                sha512: "b".repeat(128),
                size: 1,
                default_sides: vec![enderpin::Side::Server],
                roots: vec!["plugin".into()],
                required: vec![],
                incompatible: vec![],
                optional: vec![],
            }],
        },
    );
    lock.runtimes.insert(
        "world".into(),
        RuntimeLock {
            fingerprint: RuntimeLock::fingerprint(&manifest, "world")?,
            minecraft: "1.21.1".into(),
            loader: "paper".into(),
            loader_version: "42".into(),
            metadata: download.clone(),
            java: JavaLock {
                release: "21".into(),
                major: 21,
                platforms: BTreeMap::from([("linux-x86_64".into(), download.clone())]),
            },
            fabric_libraries: vec![],
            fabric_jvm: vec![],
            server_jar: Some(download),
        },
    );
    fs::write(root.path().join("enderpin.lock"), toml::to_string(&lock)?)?;
    let before = snapshot(root.path())?;
    let document = inspect(root.path(), "world", true)?;
    assert_eq!(
        document["targets"]["world"]["lock"]["runtime"]["identity"]["loader_version"],
        "42"
    );
    assert_eq!(document["targets"]["world"]["diagnostics"], json!([]));
    assert_eq!(
        document["targets"]["world"]["lock"]["packages"]["identities"][0]["key"],
        "url:plugin"
    );
    assert_eq!(
        document["targets"]["world"]["lock"]["packages"]["identities"][0]["roots"],
        json!(["plugin"])
    );
    assert_eq!(before, snapshot(root.path())?);
    fs::write(
        root.path().join("enderpin.toml"),
        manifest_text.replace("1.21.1", "1.21.2"),
    )?;
    let document = inspect(root.path(), "world", true)?;
    assert_eq!(
        document["targets"]["world"]["lock"]["runtime"]["status"],
        "stale"
    );
    assert_eq!(
        document["targets"]["world"]["lock"]["packages"]["status"],
        "stale"
    );
    fs::write(root.path().join("enderpin.lock"), "PARSE_SECRET = invalid")?;
    assert_eq!(
        inspect(root.path(), "world", false)?["error"]["code"],
        "invalid_lock"
    );
    Ok(())
}

#[test]
fn rejects_conflicts_and_redacts_errors_and_reports_compatibility() -> Result<()> {
    let root = tempfile::tempdir()?;
    assert_eq!(
        inspect(root.path(), "all", false)?["error"]["code"],
        "manifest_missing"
    );
    for manifest in [
        MANIFEST
            .replace(
                "[groups.unused.packages]\nunused",
                "[groups.unused.packages]\nproxy",
            )
            .replace("groups = [\"common\"]", "groups = [\"common\", \"unused\"]"),
        MANIFEST.replace("type = \"server\"", "type = \"PARSE_SECRET\""),
        "PARSE_SECRET = invalid".into(),
    ] {
        fs::write(root.path().join("enderpin.toml"), manifest)?;
        assert_eq!(
            inspect(root.path(), "all", false)?["error"]["code"],
            "invalid_manifest"
        );
    }
    fs::write(root.path().join("enderpin.toml"), MANIFEST)?;
    fs::create_dir_all(root.path().join("shared/common/files"))?;
    fs::create_dir_all(root.path().join("shared/other/files"))?;
    fs::write(root.path().join("shared/common/files/config"), "one")?;
    fs::write(root.path().join("shared/other/files/config"), "two")?;
    fs::write(
        root.path().join("enderpin.toml"),
        MANIFEST.replace(
            "file_groups = [\"common\"]",
            "file_groups = [\"common\", \"other\"]",
        ),
    )?;
    assert_eq!(
        inspect(root.path(), "all", false)?["error"]["code"],
        "invalid_file_inputs"
    );
    fs::write(
        root.path().join("enderpin.toml"),
        "format=4\nminecraft='1.21.1'\n[targets.world]\ntype='server'\nloader='paper'\n[targets.world.packages.bad]\nmodrinth='bad'\n",
    )?;
    let document = inspect(root.path(), "world", true)?;
    assert!(
        document["targets"]["world"]["diagnostics"]
            .as_array()
            .expect("diagnostics")
            .iter()
            .any(|d| d["code"] == "incompatible_package_kind" && d["severity"] == "error")
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn read_only_directory_works_and_symlinked_inputs_fail_safely() -> Result<()> {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("enderpin.toml"), MANIFEST)?;
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o555))?;
    let result = inspect(root.path(), "all", true);
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755))?;
    result?;
    fs::create_dir_all(root.path().join("survival/files"))?;
    symlink(
        root.path().join("enderpin.toml"),
        root.path().join("survival/files/unsafe"),
    )?;
    assert_eq!(
        inspect(root.path(), "all", false)?["error"]["code"],
        "invalid_file_inputs"
    );
    fs::remove_file(root.path().join("survival/files/unsafe"))?;
    symlink(
        root.path().join("enderpin.toml"),
        root.path().join("enderpin.lock"),
    )?;
    assert_eq!(
        inspect(root.path(), "all", false)?["error"]["code"],
        "lock_unreadable"
    );
    Ok(())
}
