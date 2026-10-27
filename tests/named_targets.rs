use anyhow::Result;
use enderpin::{Manifest, SyncOptions, Workspace, sandbox::permissions};
use std::fs;

const NETWORK: &str = r#"
format = 4
minecraft = "26.3"
[groups.common.packages]
lithium = { modrinth = "lithium", version = "locked-version" }
[profiles.backend]
type = "server"
loader = "fabric"
loader_version = "0.19.5"
groups = ["common"]
jvm_args = ["-XX:+UseZGC"]
memory_mib = 4096
[targets.client]
type = "client"
loader = "fabric"
groups = ["common"]
[targets.survival]
profile = "backend"
[targets.creative]
profile = "backend"
memory_mib = 3072
jvm_args = []
[targets.proxy]
type = "proxy"
loader = "velocity"
loader_version = "4.2.0+30"
"#;

#[test]
fn profiles_groups_overrides_and_independent_fingerprints() -> Result<()> {
    let mut manifest = Manifest::parse(NETWORK)?;
    let survival = manifest.target("survival")?;
    assert_eq!(survival.memory_mib, 4096);
    assert_eq!(survival.jvm_args, ["-XX:+UseZGC"]);
    assert!(manifest.target("creative")?.jvm_args.is_empty());
    assert_eq!(manifest.target("creative")?.memory_mib, 3072);
    assert_eq!(manifest.target("client")?.requests, survival.requests);
    assert!(manifest.target("proxy")?.requests.is_empty());
    let proxy = manifest.fingerprint("proxy")?;
    let client = manifest.fingerprint("client")?;
    manifest
        .groups
        .get_mut("common")
        .expect("group")
        .packages
        .get_mut("lithium")
        .expect("package")
        .version = Some("changed".into());
    assert_ne!(client, manifest.fingerprint("client")?);
    assert_eq!(proxy, manifest.fingerprint("proxy")?);
    manifest.target_mut("creative")?.minecraft = Some("1.21.1".into());
    assert_eq!(manifest.target("creative")?.minecraft, "1.21.1");
    assert_eq!(manifest.target("survival")?.minecraft, "26.3");
    let roundtrip = Manifest::parse(&manifest.to_toml()?)?;
    assert_eq!(roundtrip.target("creative")?.jvm_args, Vec::<String>::new());
    assert_eq!(
        roundtrip.targets["creative"].profile.as_deref(),
        Some("backend")
    );
    Ok(())
}

#[test]
fn invalid_names_unknown_keys_and_ambiguous_inheritance_are_rejected() -> Result<()> {
    for text in [
        NETWORK.replace("[targets.creative]", "[targets.\"../creative\"]"),
        NETWORK.replace("[targets.creative]", "[targets.Creative]"),
        NETWORK.replace("[targets.creative]", "[targets.CON]"),
        NETWORK.replace("[targets.creative]", "[targets.shared]"),
        NETWORK.replace("profile = \"backend\"", "profile = \"missing\""),
        NETWORK.replace("groups = [\"common\"]", "groups = [\"missing\"]"),
        NETWORK.replace(
            "[profiles.backend]",
            "[profiles.backend]\nprofile = 'recursive'",
        ),
        NETWORK.replace(
            "[profiles.backend]",
            "[profiles.backend]\npermissions = { network = true }",
        ),
        NETWORK.replace("memory_mib = 3072", "memory_mib = 0"),
        NETWORK.replace("memory_mib = 3072", "memroy_mib = 3072"),
        NETWORK.replace("type = \"proxy\"", "type = \"client\""),
        NETWORK.replace("format = 4", "format = 3"),
    ] {
        assert!(Manifest::parse(&text).is_err(), "accepted: {text}");
    }
    let mut m = Manifest::parse(NETWORK)?;
    let mut group = m.groups["common"].clone();
    m.groups.insert("same".into(), group.clone());
    m.target_mut("survival")?.groups = Some(vec!["common".into(), "same".into()]);
    m.validate()?;
    group.packages.get_mut("lithium").expect("package").version = None;
    m.groups.insert("same".into(), group);
    assert!(m.validate().is_err());
    Ok(())
}

#[test]
fn per_target_package_replacement_does_not_merge_sources() -> Result<()> {
    let m = Manifest::parse(&format!(
        "{NETWORK}\n[targets.creative.packages.lithium]\nurl = 'https://example.org/lithium.jar'\n"
    ))?;
    let creative = m.target("creative")?;
    assert!(creative.requests["lithium"].modrinth.is_none());
    assert!(creative.requests["lithium"].version.is_none());
    assert!(m.target("survival")?.requests["lithium"].modrinth.is_some());
    Ok(())
}

#[test]
fn named_targets_keep_files_worlds_leases_and_approvals_separate() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cache = tempfile::tempdir()?;
    let mut m = Manifest::parse(NETWORK)?;
    for target in m.targets.values_mut() {
        target.groups = Some(vec![]);
    }
    Workspace::init_manifest(root.path(), m, &["client", "survival", "creative", "proxy"])?;
    fs::write(
        root.path().join("survival/files/server.properties"),
        "server-port=25566\n",
    )?;
    fs::write(
        root.path().join("creative/files/server.properties"),
        "server-port=25567\n",
    )?;
    fs::create_dir_all(root.path().join("survival/run/world"))?;
    fs::write(root.path().join("survival/run/world/level.dat"), b"world")?;
    let mut ws = Workspace::open(root.path(), cache.path())?;
    let options = SyncOptions {
        locked: true,
        offline: true,
        ..Default::default()
    };
    ws.sync(
        ws.manifest.clone(),
        &["survival", "creative"],
        options,
        |_| {},
    )?;
    let survival = permissions::plan(&ws, "survival")?.fingerprint()?;
    let creative = permissions::plan(&ws, "creative")?.fingerprint()?;
    assert_ne!(survival, creative);
    let lease = enderpin::storage::target_lock(root.path(), "survival")?;
    assert!(
        ws.sync(ws.manifest.clone(), &["survival"], options, |_| {})
            .is_err()
    );
    fs::write(
        root.path().join("creative/files/server.properties"),
        "server-port=25568\n",
    )?;
    ws.sync(ws.manifest.clone(), &["creative"], options, |_| {})?;
    drop(lease);
    assert_eq!(survival, permissions::plan(&ws, "survival")?.fingerprint()?);
    assert_ne!(creative, permissions::plan(&ws, "creative")?.fingerprint()?);
    assert_eq!(
        fs::read(root.path().join("survival/run/world/level.dat"))?,
        b"world"
    );
    let mut m = ws.manifest.clone();
    m.targets.remove("survival");
    ws.sync(m, &["creative"], options, |_| {})?;
    assert_eq!(
        fs::read(root.path().join("survival/run/world/level.dat"))?,
        b"world"
    );
    assert!(
        ws.sync(ws.manifest.clone(), &["missing"], options, |_| {})
            .is_err()
    );
    Ok(())
}

#[test]
fn initial_config_preserves_native_inputs_and_refuses_unmanaged_runtime_configs() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cache = tempfile::tempdir()?;
    Workspace::init(root.path(), "26.3".into())?;
    fs::write(
        root.path().join("server/files/server.properties"),
        "motd=mine\n",
    )?;
    let mut ws = Workspace::open(root.path(), cache.path())?;
    assert!(enderpin::initialize::configs(&mut ws, &["server"], true, |_| {})?.is_empty());
    fs::remove_file(root.path().join("server/files/server.properties"))?;
    fs::write(
        root.path().join("server/run/server.properties"),
        "motd=existing\n",
    )?;
    assert!(enderpin::initialize::configs(&mut ws, &["server"], true, |_| {}).is_err());
    assert_eq!(
        fs::read_to_string(root.path().join("server/run/server.properties"))?,
        "motd=existing\n"
    );
    Ok(())
}
