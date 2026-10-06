use anyhow::Result;
use enderpin::auth::bridge::{AGENT, ASSETS, BOOTSTRAP, EmbeddedAsset, NATIVE};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read},
    path::Path,
};

// Exercise the public API as a downstream crate, including const/static use.
const AUTH_ASSETS: [EmbeddedAsset; 3] = ASSETS;
static AGENT_BYTES: &[u8] = AGENT.bytes;

#[test]
fn assets_are_complete_static_files_with_unique_basenames() {
    assert_eq!(AGENT.name, "auth-bridge.jar");
    assert_eq!(BOOTSTRAP.name, "auth-bootstrap.jar");
    assert_eq!(
        AUTH_ASSETS.map(|asset| asset.name),
        [AGENT.name, BOOTSTRAP.name, NATIVE.name]
    );
    assert_eq!(AGENT_BYTES, AGENT.bytes);
    let mut names = BTreeSet::new();
    for asset in AUTH_ASSETS {
        assert!(!asset.name.is_empty());
        assert!(!asset.name.contains(['/', '\\']));
        assert_eq!(Path::new(asset.name).file_name(), Some(asset.name.as_ref()));
        assert!(names.insert(asset.name), "duplicate embedded filename");
        assert!(!asset.bytes.is_empty(), "empty asset: {}", asset.name);
    }
    assert!(!names.contains("narrator-bridge.jar"));
}

fn jar_files(asset: EmbeddedAsset) -> Result<BTreeMap<String, Vec<u8>>> {
    assert!(asset.bytes.starts_with(b"PK\x03\x04"));
    let mut jar = zip::ZipArchive::new(Cursor::new(asset.bytes))?;
    let mut files = BTreeMap::new();
    for i in 0..jar.len() {
        let mut entry = jar.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        assert!(files.insert(entry.name().to_owned(), bytes).is_none());
    }
    Ok(files)
}

#[test]
fn jars_keep_agent_and_bootstrap_classes_separate() -> Result<()> {
    for (asset, classes, premain) in [
        (
            AGENT,
            &[
                "me/aomona/auth/AuthAgent.class",
                "me/aomona/auth/AuthAgent$1.class",
                "me/aomona/auth/MethodAdapter.class",
            ][..],
            true,
        ),
        (
            BOOTSTRAP,
            &[
                "me/aomona/auth/AuthBridge.class",
                "me/aomona/auth/NativeIO.class",
                "me/aomona/auth/RemotePrivateKey.class",
                "me/aomona/auth/RemoteProvider.class",
                "me/aomona/auth/RemoteSignature.class",
            ][..],
            false,
        ),
    ] {
        let files = jar_files(asset)?;
        let expected: BTreeSet<_> = classes
            .iter()
            .copied()
            .chain(["META-INF/MANIFEST.MF"])
            .collect();
        assert_eq!(
            files.keys().map(String::as_str).collect::<BTreeSet<_>>(),
            expected,
            "incorrect class split in {}",
            asset.name
        );
        let manifest = std::str::from_utf8(&files["META-INF/MANIFEST.MF"])?;
        assert!(manifest.lines().any(|line| line == "Manifest-Version: 1.0"));
        let entry_points: Vec<_> = manifest
            .lines()
            .filter(|line| line.starts_with("Premain-Class:"))
            .collect();
        if premain {
            assert_eq!(entry_points, ["Premain-Class: me.aomona.auth.AuthAgent"]);
        } else {
            assert!(entry_points.is_empty());
        }
        assert!(!manifest.lines().any(|line| {
            line.starts_with("Class-Path:") || line.starts_with("Boot-Class-Path:")
        }));
        for class in classes {
            let bytes = &files[*class];
            // CAFEBABE, no preview bytecode, Java 17 (class-file major version 61).
            assert_eq!(&bytes[..8], b"\xca\xfe\xba\xbe\x00\x00\x00\x3d");
        }
    }
    Ok(())
}

#[test]
fn native_filename_and_magic_match_the_compilation_target() {
    #[cfg(target_os = "linux")]
    {
        assert_eq!(NATIVE.name, "libauth-bridge.so");
        assert!(NATIVE.bytes.starts_with(b"\x7fELF"));
    }
    #[cfg(target_os = "macos")]
    {
        assert_eq!(NATIVE.name, "libauth-bridge.dylib");
        assert!(
            NATIVE.bytes.starts_with(b"\xcf\xfa\xed\xfe")
                || NATIVE.bytes.starts_with(b"\xfe\xed\xfa\xcf"),
            "expected a 64-bit Mach-O library"
        );
    }
    #[cfg(target_os = "windows")]
    {
        assert_eq!(NATIVE.name, "auth-bridge.dll");
        assert!(NATIVE.bytes.starts_with(b"MZ"));
        assert!(NATIVE.bytes.len() >= 64);
        let offset = u32::from_le_bytes([
            NATIVE.bytes[60],
            NATIVE.bytes[61],
            NATIVE.bytes[62],
            NATIVE.bytes[63],
        ]) as usize;
        assert_eq!(NATIVE.bytes.get(offset..offset + 4), Some(&b"PE\0\0"[..]));
    }
}
