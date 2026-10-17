//! Embedded Java/JNI authentication bridge assets for launcher integrations.
//!
//! These descriptors expose only filenames and immutable bytes. Reading them does
//! not write files, access credentials, start a broker, or authorize a launch.
//! Callers own storage and launch policy. Keep the bootstrap JAR alongside the
//! native library: the agent resolves it relative to the native library path.
//!
//! Enderpin builds the assets with a JDK 17+ (`java`, `javac`, and `jar`) and a C
//! compiler. Java classes target Java 17. [`NATIVE`] is compiled for the Rust
//! target's OS and architecture, not selected at runtime. Cross-compilation also
//! requires a target C toolchain and target JNI headers via `ENDERPIN_TARGET_JDK`.
//! The build tools run on the host; setting that variable only selects JNI headers.
//! Assets from a different target cannot be substituted for the target's library.
//!
//! ```
//! use enderpin::auth::bridge::{AGENT, ASSETS};
//!
//! assert_eq!(AGENT.name, "auth-bridge.jar");
//! assert_eq!(ASSETS.len(), 3);
//! assert!(ASSETS.iter().all(|asset| !asset.bytes.is_empty()));
//! ```

/// A build-time asset with a basename and immutable, statically embedded bytes.
#[derive(Clone, Copy)]
pub struct EmbeddedAsset {
    /// Filename only, without a directory component.
    pub name: &'static str,
    /// The complete file contents produced when Enderpin was built.
    pub bytes: &'static [u8],
}

/// Java agent and bytecode adapter, with the `Premain-Class` manifest entry.
pub const AGENT: EmbeddedAsset = EmbeddedAsset {
    name: "auth-bridge.jar",
    bytes: include_bytes!(concat!(env!("OUT_DIR"), "/auth-bridge.jar")),
};

/// Bootstrap-classloader bridge, native IPC, and remote-signature classes.
pub const BOOTSTRAP: EmbeddedAsset = EmbeddedAsset {
    name: "auth-bootstrap.jar",
    bytes: include_bytes!(concat!(env!("OUT_DIR"), "/auth-bootstrap.jar")),
};

/// JNI library built for this crate's target OS and architecture.
///
/// The filename is `auth-bridge.dll` on Windows, `libauth-bridge.dylib` on
/// macOS, or `libauth-bridge.so` on Linux.
pub const NATIVE: EmbeddedAsset = EmbeddedAsset {
    name: env!("ENDERPIN_AUTH_NATIVE"),
    bytes: include_bytes!(concat!(env!("OUT_DIR"), "/", env!("ENDERPIN_AUTH_NATIVE"))),
};

/// All authentication assets, in agent, bootstrap, and native-library order.
/// Narrator assets are intentionally not part of this API.
pub const ASSETS: [EmbeddedAsset; 3] = [AGENT, BOOTSTRAP, NATIVE];
