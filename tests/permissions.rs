use anyhow::{Context, Result};
use enderpin::{
    Kind, Package, Side, Workspace,
    model::LockedPackage,
    sandbox::{
        self,
        permissions::{self, Local, Request, Settings},
    },
    storage,
};
use std::{
    fs,
    io::{Cursor, Write},
};

#[test]
fn requests_are_hash_bound_local_and_rechecked_before_approval() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cache = tempfile::tempdir()?;
    let external = tempfile::tempdir()?;
    Workspace::init(root.path(), "1.21.1".into())?;
    let mut ws = Workspace::open(root.path(), cache.path())?;
    let mut jar = zip::ZipWriter::new(Cursor::new(Vec::new()));
    jar.start_file(
        "enderpin.permissions.json",
        zip::write::SimpleFileOptions::default(),
    )?;
    jar.write_all(br#"{"format":1,"requests":[{"permission":"network","reason":"Connect to the shared server"},{"permission":"folder-read","folder":"schematics","reason":"Read selected schematics"}]}"#)?;
    let bytes = jar.finish()?.into_inner();
    let mut package = Package::modrinth("example", Kind::Mod);
    package.modrinth = None;
    package.url = Some("https://example.org/example.jar".into());
    ws.manifest
        .client
        .packages
        .insert("example".into(), package);
    ws.manifest.client.sandbox.network = false;
    ws.manifest.client.permissions.insert("url:example".into(), vec![serde_json::from_value::<Request>(serde_json::json!({"permission":"folder-write","folder":"schematics","reason":"Save edited schematics"}))?]);
    let target = ws
        .lockfile
        .targets
        .get_mut(&Side::Client)
        .context("target missing")?;
    target.fingerprint = ws.manifest.fingerprint(Side::Client)?;
    target.requests = ws.manifest.requests(Side::Client);
    target.packages.push(LockedPackage {
        key: "url:example".into(),
        name: "example".into(),
        kind: Kind::Mod,
        project: None,
        version: None,
        version_number: None,
        url: "https://example.org/example.jar".into(),
        filename: "example.jar".into(),
        sha512: enderpin::model::hash_bytes(&bytes),
        size: bytes.len() as u64,
        default_sides: vec![Side::Client],
        roots: vec!["example".into()],
        required: vec![],
        incompatible: vec![],
        optional: vec![],
    });
    let game_mods = storage::directory(&ws.root, "client/mods")?;
    fs::write(game_mods.join("example.jar"), &bytes)?;
    assert!(
        permissions::plan(&ws, Side::Client).is_err(),
        "unbound folders must stop inspection"
    );
    let mut local = Local::default();
    local
        .bindings
        .insert("schematics".into(), external.path().canonicalize()?);
    local.save(&ws, Side::Client)?;
    let plan = permissions::plan(&ws, Side::Client)?;
    assert!(!plan.baseline.network && plan.effective.network);
    assert!(plan.folders["schematics"].write);
    assert_eq!(plan.packages[0].embedded.len(), 2);
    assert_eq!(plan.packages[0].repository.len(), 1);
    local.approved = Some(plan.fingerprint()?);
    local.save(&ws, Side::Client)?;
    assert_eq!(
        Local::load(&ws, Side::Client)?.approved,
        Some(plan.fingerprint()?)
    );
    ws.manifest
        .client
        .permissions
        .get_mut("url:example")
        .context("request missing")?[0]
        .reason = "A changed purpose".into();
    assert_ne!(
        local.approved,
        Some(permissions::plan(&ws, Side::Client)?.fingerprint()?)
    );
    let running = storage::target_lock(&ws.root, Side::Client)?;
    assert!(local.save(&ws, Side::Client).is_err());
    assert!(permissions::plan(&ws, Side::Client).is_err());
    drop(running);
    fs::write(game_mods.join("example.jar"), b"replacement")?;
    assert!(
        permissions::plan(&ws, Side::Client).is_err(),
        "changed artifacts must not be inspected as the old mod"
    );
    Ok(())
}

#[test]
fn unsupported_denials_and_aliases_fail_closed() -> Result<()> {
    let settings = Settings {
        microphone: Some(false),
        ..Default::default()
    };
    assert!(settings.validate_for("linux", true).is_err());
    assert!(settings.validate_for("windows", true).is_err());
    assert!(settings.validate_for("macos", true).is_ok());
    assert!(
        serde_json::from_str::<Request>(r#"{"permission":"shell","reason":"run code"}"#).is_err()
    );
    let request: Request = serde_json::from_str(
        r#"{"permission":"folder-read","reason":"Read files","folder":"../private"}"#,
    )?;
    assert!(request.validate().is_err());
    let root = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    fs::write(outside.path().join("secret"), b"private")?;
    fs::hard_link(outside.path().join("secret"), root.path().join("alias"))?;
    assert!(sandbox::validate_data_tree(root.path()).is_err());
    Ok(())
}
