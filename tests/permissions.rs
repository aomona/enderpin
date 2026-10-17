use anyhow::{Context, Result};
use enderpin::{
    Kind, Package, Side, Workspace,
    model::LockedPackage,
    sandbox::{
        self,
        permissions::{self, Clipboard, FileAccess, FolderGrant, GameDirectory, Local, Settings},
    },
    storage,
};
use std::{
    fs,
    io::{Cursor, Write},
};

#[test]
fn approvals_track_packages_settings_and_local_folders() -> Result<()> {
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
    ws.manifest.client.permissions.network = false;
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
    let game_mods = storage::directory(&ws.root, "run/client/mods")?;
    fs::write(game_mods.join("example.jar"), &bytes)?;
    let original = permissions::plan(&ws, Side::Client)?;
    assert!(
        !original.effective.network,
        "embedded declarations cannot increase game permissions"
    );
    assert_eq!(original.packages.len(), 1);
    let mut local = Local {
        approved: Some(original.fingerprint()?),
        ..Default::default()
    };
    local.save(&ws, Side::Client)?;
    assert_eq!(
        Local::load(&ws, Side::Client)?.approved,
        Some(original.fingerprint()?)
    );

    // New content requires review even though the game's permissions did not increase.
    let updated = vec![b'x'; bytes.len()];
    fs::write(game_mods.join("example.jar"), &updated)?;
    assert!(
        permissions::plan(&ws, Side::Client).is_err(),
        "unlocked content must fail closed"
    );
    ws.lockfile
        .targets
        .get_mut(&Side::Client)
        .context("missing target")?
        .packages[0]
        .sha512 = enderpin::model::hash_bytes(&updated);
    assert_ne!(
        local.approved,
        Some(permissions::plan(&ws, Side::Client)?.fingerprint()?)
    );
    fs::write(game_mods.join("example.jar"), &bytes)?;
    ws.lockfile
        .targets
        .get_mut(&Side::Client)
        .context("missing target")?
        .packages[0]
        .sha512 = enderpin::model::hash_bytes(&bytes);
    assert_eq!(
        local.approved,
        Some(permissions::plan(&ws, Side::Client)?.fingerprint()?)
    );

    let mut second = ws.lockfile.targets[&Side::Client].packages[0].clone();
    second.key = "url:another".into();
    second.name = "another".into();
    second.filename = "another.jar".into();
    fs::write(game_mods.join("another.jar"), &bytes)?;
    ws.lockfile
        .targets
        .get_mut(&Side::Client)
        .context("missing target")?
        .packages
        .push(second);
    assert_ne!(
        local.approved,
        Some(permissions::plan(&ws, Side::Client)?.fingerprint()?)
    );
    ws.lockfile
        .targets
        .get_mut(&Side::Client)
        .context("missing target")?
        .packages
        .pop();

    ws.manifest.client.permissions.network = true;
    assert_ne!(
        local.approved,
        Some(permissions::plan(&ws, Side::Client)?.fingerprint()?)
    );
    ws.manifest.client.permissions.network = false;
    local.folders.push(FolderGrant {
        path: external.path().canonicalize()?,
        write: false,
    });
    local.save(&ws, Side::Client)?;
    let read_only = permissions::plan(&ws, Side::Client)?;
    assert_ne!(local.approved, Some(read_only.fingerprint()?));
    assert_eq!(read_only.folders, local.folders);
    local.folders[0].write = true;
    local.save(&ws, Side::Client)?;
    assert_ne!(
        read_only.fingerprint()?,
        permissions::plan(&ws, Side::Client)?.fingerprint()?
    );
    // Grants apply directly even when there are no mods requesting the folder.
    ws.lockfile
        .targets
        .get_mut(&Side::Client)
        .context("missing target")?
        .packages
        .clear();
    assert!(permissions::plan(&ws, Side::Client)?.folders[0].write);
    for unsafe_path in [
        ws.root.clone(),
        ws.root.parent().context("no parent")?.to_owned(),
        ws.root.join("run/client"),
    ] {
        local.folders[0].path = unsafe_path;
        local.save(&ws, Side::Client)?;
        assert!(permissions::plan(&ws, Side::Client).is_err());
    }
    local.folders[0].path = external.path().canonicalize()?;
    local.folders.push(local.folders[0].clone());
    assert!(
        permissions::validate_folders(&ws, &local.folders).is_err(),
        "overlapping grants are ambiguous"
    );
    local.folders.pop();
    local.save(&ws, Side::Client)?;
    let running = storage::target_lock(&ws.root, Side::Client)?;
    assert!(local.save(&ws, Side::Client).is_err());
    assert!(permissions::plan(&ws, Side::Client).is_err());
    drop(running);
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
    for os in ["windows", "linux", "macos"] {
        let mut settings = Settings {
            audio: false,
            desktop_integration: false,
            graphics_cache: false,
            microphone: Some(true),
            clipboard: Clipboard::None,
            network: false,
            account_authentication: false,
            ..Default::default()
        };
        assert!(settings.validate_for(os, true).is_err());
        settings.reset_unsupported_desktop(os);
        assert!(settings.validate_for(os, true).is_ok());
        assert!(!settings.network && !settings.account_authentication);
    }
    assert!(serde_json::from_str::<Settings>(r#"{"shell":true}"#).is_err());
    let root = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    fs::write(outside.path().join("secret"), b"private")?;
    fs::hard_link(outside.path().join("secret"), root.path().join("alias"))?;
    assert!(sandbox::validate_data_tree(root.path()).is_err());
    Ok(())
}

#[test]
fn compact_settings_roundtrip_and_legacy_requests_fail_closed() -> Result<()> {
    let mut manifest = enderpin::Manifest::new("1.21.1".into())?;
    assert!(!manifest.to_toml()?.contains("permissions"));
    let defaults = &manifest.client.permissions;
    assert!(defaults.account_authentication && defaults.narrator && defaults.clipboard_allowed());
    assert!(defaults.open_links && defaults.open_folders && defaults.import_files);
    manifest.client.permissions.network = false;
    manifest.client.permissions.microphone = Some(false);
    manifest.client.permissions.clipboard = Clipboard::None;
    manifest.client.permissions.narrator = false;
    manifest.client.permissions.open_links = false;
    manifest.client.permissions.open_folders = false;
    manifest.client.permissions.import_files = false;
    let text = manifest.to_toml()?;
    assert!(text.contains("network = false"));
    assert!(text.contains("[client.permissions]"));
    assert!(text.contains("clipboard = \"none\""));
    assert!(!text.contains("game_write"));
    assert!(!text.contains("permissions.files"));
    assert!(
        !enderpin::Manifest::parse(&text)?
            .client
            .permissions
            .clipboard_allowed()
    );
    assert_eq!(
        enderpin::Manifest::parse(&text)?.client.permissions,
        manifest.client.permissions
    );
    assert_eq!(
        enderpin::Manifest::parse(&text)?.server.permissions,
        manifest.server.permissions
    );
    assert!(enderpin::Manifest::parse(&(text + "\n[[client.permissions.example]]\npermission = 'network'\nreason = 'old declaration'\n")).is_err());
    assert!(toml::from_str::<Local>("[bindings]\nexample = '/tmp'\n").is_err());
    Ok(())
}

#[test]
fn file_permissions_roundtrip_omit_nested_defaults() -> Result<()> {
    let mut manifest = enderpin::Manifest::new("1.21.1".into())?;
    manifest.client.permissions.files.read_only = vec![GameDirectory::Mods, GameDirectory::Config];
    manifest.server.permissions.files.game = FileAccess::ReadOnly;
    let text = manifest.to_toml()?;
    assert!(text.contains("[client.permissions.files]"));
    assert!(text.contains("[server.permissions.files]"));
    assert!(text.contains("game = \"read-only\""));
    assert!(!text.contains("game = \"read-write\""));
    let parsed = enderpin::Manifest::parse(&text)?;
    assert_eq!(parsed.client.permissions, manifest.client.permissions);
    assert_eq!(parsed.server.permissions, manifest.server.permissions);
    // Explicit defaults are accepted but not rewritten into every config.
    let defaults = enderpin::Manifest::new("1.21.1".into())?.to_toml()?;
    let explicit = enderpin::Manifest::parse(&format!(
        "{defaults}\n[client.permissions]\nclipboard = 'system'\n[client.permissions.files]\ngame = 'read-write'\nread_only = []\n"
    ))?;
    assert_eq!(explicit.to_toml()?, defaults);
    Ok(())
}

#[test]
fn permission_schema_rejects_legacy_and_misspelled_controls() -> Result<()> {
    let base = enderpin::Manifest::new("1.21.1".into())?.to_toml()?;
    for invalid in [
        "[client.sandbox]\nnetwork = false",
        "[server.sandbox]\ngame_write = false",
        "[client.permissions]\ngame_write = false",
        "[client.permissions]\nread_only = ['mods']",
        "[client.permissions]\nclipboard = false",
        "[client.permissions]\nclipboard = 'read'",
        "[client.permissions.files]\ngame = 'write'",
        "[client.permissions.files]\nread_only = ['../private']",
        "[client.permissions.files]\nwrite = true",
    ] {
        assert!(
            enderpin::Manifest::parse(&format!("{base}\n{invalid}\n")).is_err(),
            "accepted {invalid}"
        );
    }
    for clipboard in [Clipboard::None, Clipboard::ReadWrite] {
        let settings = Settings {
            clipboard,
            ..Default::default()
        };
        assert!(settings.validate_for("windows", true).is_err());
        assert!(settings.validate_for("macos", true).is_ok());
    }
    Ok(())
}
