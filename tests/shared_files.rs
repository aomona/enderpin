use anyhow::Result;
use enderpin::{Side, Workspace, sandbox::permissions, workspace::SyncOptions};
use std::{fs, path::Path};

fn write(root: &Path, path: &str, text: &str) -> Result<()> {
    let path = root.join(path);
    fs::create_dir_all(path.parent().expect("parent"))?;
    fs::write(path, text)?;
    Ok(())
}

fn sync(ws: &mut Workspace, sides: &[Side], restore: bool) -> Result<()> {
    ws.sync(
        ws.manifest.clone(),
        sides,
        SyncOptions {
            locked: true,
            offline: true,
            restore,
            ..Default::default()
        },
        |_| (),
    )?;
    Ok(())
}

#[test]
fn overlays_reproduce_from_git_inputs_and_do_not_touch_personal_data() -> Result<()> {
    let root = tempfile::tempdir()?;
    let clone = tempfile::tempdir()?;
    let cache = tempfile::tempdir()?;
    Workspace::init_quick(root.path(), "26.2".into())?;
    let ignore = fs::read_to_string(root.path().join(".gitignore"))?;
    assert!(ignore.lines().any(|line| line == "/run/"));
    assert!(root.path().join("files/common").is_dir());
    let sources = [
        ("files/common/config/nested/settings.json", "common"),
        ("files/client/config/nested/settings.json", "client"),
        ("files/server/server.properties", "server-port=25566"),
    ];
    for (path, content) in sources {
        write(root.path(), path, content)?;
    }
    write(root.path(), "run/client/saves/keep/level.dat", "world")?;
    write(root.path(), "run/server/world/level.dat", "server world")?;
    let mut ws = Workspace::open(root.path(), cache.path())?;
    sync(&mut ws, &[Side::Client], false)?;
    assert!(!root.path().join("run/server/config").exists());
    sync(&mut ws, &[Side::Server], false)?;
    assert_eq!(
        fs::read_to_string(root.path().join("run/client/config/nested/settings.json"))?,
        "client"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("run/server/config/nested/settings.json"))?,
        "common"
    );
    for name in ["enderpin.toml", "enderpin.lock"] {
        fs::copy(root.path().join(name), clone.path().join(name))?;
    }
    for (path, content) in sources {
        write(clone.path(), path, content)?;
    }
    let mut cloned = Workspace::open(clone.path(), cache.path())?;
    sync(&mut cloned, &Side::ALL, false)?;
    for path in [
        "run/client/config/nested/settings.json",
        "run/server/config/nested/settings.json",
        "run/server/server.properties",
    ] {
        assert_eq!(
            fs::read(root.path().join(path))?,
            fs::read(clone.path().join(path))?
        );
    }
    fs::remove_file(root.path().join("files/client/config/nested/settings.json"))?;
    sync(&mut ws, &[Side::Client], false)?;
    assert_eq!(
        fs::read_to_string(root.path().join("run/client/config/nested/settings.json"))?,
        "common"
    );
    fs::remove_file(root.path().join("files/common/config/nested/settings.json"))?;
    sync(&mut ws, &Side::ALL, false)?;
    assert!(
        !root
            .path()
            .join("run/client/config/nested/settings.json")
            .exists()
    );
    assert!(
        !root
            .path()
            .join("run/server/config/nested/settings.json")
            .exists()
    );
    assert_eq!(
        fs::read_to_string(root.path().join("run/client/saves/keep/level.dat"))?,
        "world"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("run/server/world/level.dat"))?,
        "server world"
    );
    Ok(())
}

#[test]
fn three_way_sync_preserves_local_edits_and_conflicts_atomically() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cache = tempfile::tempdir()?;
    Workspace::init_quick(root.path(), "26.2".into())?;
    let source = "files/common/config/example.json";
    let client = "run/client/config/example.json";
    let server = "run/server/config/example.json";
    write(root.path(), source, "initial")?;
    let mut ws = Workspace::open(root.path(), cache.path())?;
    sync(&mut ws, &Side::ALL, false)?;
    write(root.path(), server, "local")?;
    sync(&mut ws, &Side::ALL, false)?;
    assert_eq!(fs::read_to_string(root.path().join(server))?, "local");
    write(root.path(), source, "upstream")?;
    let state = fs::read(root.path().join(".enderpin/client/state.toml"))?;
    assert!(
        sync(&mut ws, &Side::ALL, false)
            .expect_err("unsafe sync must fail")
            .to_string()
            .contains("shared file conflict")
    );
    assert_eq!(fs::read_to_string(root.path().join(client))?, "initial");
    assert_eq!(
        fs::read(root.path().join(".enderpin/client/state.toml"))?,
        state
    );
    // Copying a local edit back to the source resolves the conflict.
    write(root.path(), source, "local")?;
    sync(&mut ws, &Side::ALL, false)?;
    write(root.path(), server, "another local edit")?;
    fs::remove_file(root.path().join(source))?;
    assert!(sync(&mut ws, &Side::ALL, false).is_err());
    sync(&mut ws, &Side::ALL, true)?;
    assert!(!root.path().join(server).exists());
    assert!(!root.path().join(client).exists());
    Ok(())
}

#[test]
fn unmanaged_files_are_never_adopted_or_overwritten_even_with_restore() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cache = tempfile::tempdir()?;
    Workspace::init_quick(root.path(), "26.2".into())?;
    write(root.path(), "files/client/options.txt", "shared")?;
    write(root.path(), "run/client/options.txt", "personal")?;
    let mut ws = Workspace::open(root.path(), cache.path())?;
    for restore in [false, true] {
        assert!(
            sync(&mut ws, &[Side::Client], restore)
                .expect_err("unsafe sync must fail")
                .to_string()
                .contains("unmanaged filename collision")
        );
        assert_eq!(
            fs::read_to_string(root.path().join("run/client/options.txt"))?,
            "personal"
        );
    }
    Ok(())
}

#[test]
fn local_deletion_is_preserved_until_restore_and_shared_hashes_bind_approval() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cache = tempfile::tempdir()?;
    Workspace::init_quick(root.path(), "26.2".into())?;
    write(root.path(), "files/client/mods/local.jar", "jar1")?;
    let mut ws = Workspace::open(root.path(), cache.path())?;
    sync(&mut ws, &[Side::Client], false)?;
    let fingerprint = permissions::plan(&ws, Side::Client)?.fingerprint()?;
    write(root.path(), "run/client/mods/local.jar", "jar2")?;
    assert_ne!(
        permissions::plan(&ws, Side::Client)?.fingerprint()?,
        fingerprint
    );
    fs::remove_file(root.path().join("run/client/mods/local.jar"))?;
    sync(&mut ws, &[Side::Client], false)?;
    assert!(!root.path().join("run/client/mods/local.jar").exists());
    sync(&mut ws, &[Side::Client], true)?;
    assert_eq!(
        permissions::plan(&ws, Side::Client)?.fingerprint()?,
        fingerprint
    );
    Ok(())
}

#[test]
fn ambiguous_overlay_trees_and_other_target_state_are_rejected() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cache = tempfile::tempdir()?;
    Workspace::init_quick(root.path(), "26.2".into())?;
    write(root.path(), "files/common/config", "file")?;
    write(root.path(), "files/client/config/a.json", "child")?;
    let mut ws = Workspace::open(root.path(), cache.path())?;
    assert!(
        sync(&mut ws, &[Side::Client], false)
            .expect_err("unsafe sync must fail")
            .to_string()
            .contains("file/directory collision")
    );
    fs::remove_file(root.path().join("files/common/config"))?;
    write(root.path(), "files/common/CONFIG/A.JSON", "case alias")?;
    assert!(
        sync(&mut ws, &[Side::Client], false)
            .expect_err("unsafe sync must fail")
            .to_string()
            .contains("differ only by case")
    );
    fs::remove_dir_all(root.path().join("files/common/CONFIG"))?;
    sync(&mut ws, &[Side::Client], false)?;
    let state = root.path().join(".enderpin/client/state.toml");
    let text = fs::read_to_string(&state)?.replace("run/client/", "run/server/");
    fs::write(state, text)?;
    assert!(
        sync(&mut ws, &[Side::Client], false)
            .expect_err("unsafe sync must fail")
            .to_string()
            .contains("different target")
    );
    Ok(())
}

#[test]
#[cfg(unix)]
fn source_and_destination_symlinks_are_rejected() -> Result<()> {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir()?;
    let external = tempfile::tempdir()?;
    let cache = tempfile::tempdir()?;
    Workspace::init_quick(root.path(), "26.2".into())?;
    write(external.path(), "secret", "untouched")?;
    symlink(
        external.path().join("secret"),
        root.path().join("files/common/link"),
    )?;
    let mut ws = Workspace::open(root.path(), cache.path())?;
    assert!(sync(&mut ws, &Side::ALL, false).is_err());
    fs::remove_file(root.path().join("files/common/link"))?;
    write(root.path(), "files/common/config/secret", "replacement")?;
    symlink(external.path(), root.path().join("run/client/config"))?;
    assert!(sync(&mut ws, &Side::ALL, true).is_err());
    assert_eq!(
        fs::read_to_string(external.path().join("secret"))?,
        "untouched"
    );
    Ok(())
}

#[test]
fn old_format_is_rejected_without_moving_game_data() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cache = tempfile::tempdir()?;
    Workspace::init_quick(root.path(), "26.2".into())?;
    let manifest = root.path().join("enderpin.toml");
    let old = fs::read_to_string(&manifest)?.replace("format = 3", "format = 2");
    fs::write(manifest, old)?;
    write(root.path(), "client/saves/keep/level.dat", "world")?;
    assert!(Workspace::open(root.path(), cache.path()).is_err());
    assert_eq!(
        fs::read_to_string(root.path().join("client/saves/keep/level.dat"))?,
        "world"
    );
    Ok(())
}
