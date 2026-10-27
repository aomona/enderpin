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
        .target_mut("client")?
        .packages
        .insert("example".into(), package);
    ws.manifest
        .target_mut("client")?
        .permissions
        .get_or_insert_with(Default::default)
        .network = false;
    let target = ws
        .lockfile
        .targets
        .get_mut("client")
        .context("target missing")?;
    target.fingerprint = ws.manifest.fingerprint("client")?;
    target.requests = ws.manifest.requests("client")?;
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
    let game_mods = storage::directory(&ws.root, "client/run/mods")?;
    fs::write(game_mods.join("example.jar"), &bytes)?;
    let original = permissions::plan(&ws, "client")?;
    assert!(
        !original.effective.network,
        "embedded declarations cannot increase game permissions"
    );
    assert_eq!(original.packages.len(), 1);
    let mut local = Local {
        approved: Some(original.fingerprint()?),
        ..Default::default()
    };
    local.save(&ws, "client")?;
    assert_eq!(
        Local::load(&ws, "client")?.approved,
        Some(original.fingerprint()?)
    );

    // New content requires review even though the game's permissions did not increase.
    let updated = vec![b'x'; bytes.len()];
    fs::write(game_mods.join("example.jar"), &updated)?;
    assert!(
        permissions::plan(&ws, "client").is_err(),
        "unlocked content must fail closed"
    );
    ws.lockfile
        .targets
        .get_mut("client")
        .context("missing target")?
        .packages[0]
        .sha512 = enderpin::model::hash_bytes(&updated);
    assert_ne!(
        local.approved,
        Some(permissions::plan(&ws, "client")?.fingerprint()?)
    );
    fs::write(game_mods.join("example.jar"), &bytes)?;
    ws.lockfile
        .targets
        .get_mut("client")
        .context("missing target")?
        .packages[0]
        .sha512 = enderpin::model::hash_bytes(&bytes);
    assert_eq!(
        local.approved,
        Some(permissions::plan(&ws, "client")?.fingerprint()?)
    );

    let mut second = ws.lockfile.targets["client"].packages[0].clone();
    second.key = "url:another".into();
    second.name = "another".into();
    second.filename = "another.jar".into();
    fs::write(game_mods.join("another.jar"), &bytes)?;
    ws.lockfile
        .targets
        .get_mut("client")
        .context("missing target")?
        .packages
        .push(second);
    assert_ne!(
        local.approved,
        Some(permissions::plan(&ws, "client")?.fingerprint()?)
    );
    ws.lockfile
        .targets
        .get_mut("client")
        .context("missing target")?
        .packages
        .pop();

    ws.manifest
        .target_mut("client")?
        .permissions
        .get_or_insert_with(Default::default)
        .network = true;
    assert_ne!(
        local.approved,
        Some(permissions::plan(&ws, "client")?.fingerprint()?)
    );
    ws.manifest
        .target_mut("client")?
        .permissions
        .get_or_insert_with(Default::default)
        .network = false;

    // Shared launch flags require approval again without invalidating the package lock.
    ws.manifest.target_mut("client")?.jvm_args = Some(vec!["-XX:+UseZGC".into()]);
    let tuned = permissions::plan(&ws, "client")?;
    assert_eq!(tuned.jvm_args, ["-XX:+UseZGC"]);
    assert_ne!(local.approved, Some(tuned.fingerprint()?));
    ws.manifest.target_mut("client")?.jvm_args = None;
    // Plans without JVM arguments keep the fingerprint approved before the key existed.
    assert_eq!(
        local.approved,
        Some(permissions::plan(&ws, "client")?.fingerprint()?)
    );
    assert!(!serde_json::to_string(&permissions::plan(&ws, "client")?)?.contains("jvm_args"));

    local.folders.push(FolderGrant {
        path: external.path().canonicalize()?,
        write: false,
    });
    local.save(&ws, "client")?;
    let read_only = permissions::plan(&ws, "client")?;
    assert_ne!(local.approved, Some(read_only.fingerprint()?));
    assert_eq!(read_only.folders, local.folders);
    local.folders[0].write = true;
    local.save(&ws, "client")?;
    assert_ne!(
        read_only.fingerprint()?,
        permissions::plan(&ws, "client")?.fingerprint()?
    );
    // Grants apply directly even when there are no mods requesting the folder.
    ws.lockfile
        .targets
        .get_mut("client")
        .context("missing target")?
        .packages
        .clear();
    assert!(permissions::plan(&ws, "client")?.folders[0].write);
    for unsafe_path in [
        ws.root.clone(),
        ws.root.parent().context("no parent")?.to_owned(),
        ws.root.join("client/run"),
    ] {
        local.folders[0].path = unsafe_path;
        local.save(&ws, "client")?;
        assert!(permissions::plan(&ws, "client").is_err());
    }
    local.folders[0].path = external.path().canonicalize()?;
    local.folders.push(local.folders[0].clone());
    assert!(
        permissions::validate_folders(&ws, &local.folders).is_err(),
        "overlapping grants are ambiguous"
    );
    local.folders.pop();
    local.save(&ws, "client")?;
    let running = storage::target_lock(&ws.root, "client")?;
    assert!(local.save(&ws, "client").is_err());
    assert!(permissions::plan(&ws, "client").is_err());
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
    let defaults = &manifest.target("client")?.permissions;
    assert!(defaults.account_authentication && defaults.narrator && defaults.clipboard_allowed());
    assert!(defaults.open_links && defaults.open_folders && defaults.import_files);
    manifest
        .target_mut("client")?
        .permissions
        .get_or_insert_with(Default::default)
        .network = false;
    manifest
        .target_mut("client")?
        .permissions
        .get_or_insert_with(Default::default)
        .microphone = Some(false);
    manifest
        .target_mut("client")?
        .permissions
        .get_or_insert_with(Default::default)
        .clipboard = Clipboard::None;
    manifest
        .target_mut("client")?
        .permissions
        .get_or_insert_with(Default::default)
        .narrator = false;
    manifest
        .target_mut("client")?
        .permissions
        .get_or_insert_with(Default::default)
        .open_links = false;
    manifest
        .target_mut("client")?
        .permissions
        .get_or_insert_with(Default::default)
        .open_folders = false;
    manifest
        .target_mut("client")?
        .permissions
        .get_or_insert_with(Default::default)
        .import_files = false;
    let text = manifest.to_toml()?;
    assert!(text.contains("network = false"));
    assert!(text.contains("[targets.client.permissions]"));
    assert!(text.contains("clipboard = \"none\""));
    assert!(!text.contains("game_write"));
    assert!(!text.contains("permissions.files"));
    assert!(
        !enderpin::Manifest::parse(&text)?
            .target("client")?
            .permissions
            .clipboard_allowed()
    );
    assert_eq!(
        enderpin::Manifest::parse(&text)?
            .target("client")?
            .permissions,
        manifest.target("client")?.permissions
    );
    assert_eq!(
        enderpin::Manifest::parse(&text)?
            .target("server")?
            .permissions,
        manifest.target("server")?.permissions
    );
    assert!(enderpin::Manifest::parse(&(text + "\n[[targets.client.permissions.example]]\npermission = 'network'\nreason = 'old declaration'\n")).is_err());
    assert!(toml::from_str::<Local>("[bindings]\nexample = '/tmp'\n").is_err());
    Ok(())
}

#[test]
fn file_permissions_roundtrip_omit_nested_defaults() -> Result<()> {
    let mut manifest = enderpin::Manifest::new("1.21.1".into())?;
    manifest
        .target_mut("client")?
        .permissions
        .get_or_insert_with(Default::default)
        .files
        .read_only = vec![GameDirectory::Mods, GameDirectory::Config];
    manifest
        .target_mut("server")?
        .permissions
        .get_or_insert_with(Default::default)
        .files
        .game = FileAccess::ReadOnly;
    let text = manifest.to_toml()?;
    assert!(text.contains("[targets.client.permissions.files]"));
    assert!(text.contains("[targets.server.permissions.files]"));
    assert!(text.contains("game = \"read-only\""));
    assert!(!text.contains("game = \"read-write\""));
    let parsed = enderpin::Manifest::parse(&text)?;
    assert_eq!(
        parsed.target("client")?.permissions,
        manifest.target("client")?.permissions
    );
    assert_eq!(
        parsed.target("server")?.permissions,
        manifest.target("server")?.permissions
    );
    // Explicit defaults are accepted but not rewritten into every config.
    let defaults = enderpin::Manifest::new("1.21.1".into())?.to_toml()?;
    let explicit = enderpin::Manifest::parse(&format!(
        "{defaults}\n[targets.client.permissions]\nclipboard = 'system'\n[targets.client.permissions.files]\ngame = 'read-write'\nread_only = []\n"
    ))?;
    assert_eq!(explicit.to_toml()?, defaults);
    Ok(())
}

#[test]
fn permission_schema_rejects_legacy_and_misspelled_controls() -> Result<()> {
    let base = enderpin::Manifest::new("1.21.1".into())?.to_toml()?;
    for invalid in [
        "[targets.client.sandbox]\nnetwork = false",
        "[targets.server.sandbox]\ngame_write = false",
        "[targets.client.permissions]\ngame_write = false",
        "[targets.client.permissions]\nread_only = ['mods']",
        "[targets.client.permissions]\nclipboard = false",
        "[targets.client.permissions]\nclipboard = 'read'",
        "[targets.client.permissions.files]\ngame = 'write'",
        "[targets.client.permissions.files]\nread_only = ['../private']",
        "[targets.client.permissions.files]\nwrite = true",
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
