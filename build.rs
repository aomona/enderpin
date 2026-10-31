use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn run(command: &mut Command) {
    assert!(
        command
            .status()
            .expect("JDK and native compiler are required to build the sandbox bridges")
            .success(),
        "bridge compilation failed"
    );
}
fn jar(output: &Path, classes: &Path, name: &str, entries: &[&str], manifest: Option<&Path>) {
    let mut cmd = Command::new("jar");
    cmd.arg(if manifest.is_some() { "cfm" } else { "cf" })
        .arg(output.join(name));
    if let Some(manifest) = manifest {
        cmd.arg(manifest);
    }
    for entry in entries {
        cmd.arg("-C").arg(classes).arg(entry);
    }
    run(&mut cmd);
}
fn main() {
    println!("cargo:rerun-if-changed=java");
    println!("cargo:rerun-if-env-changed=JAVA_HOME");
    println!("cargo:rerun-if-env-changed=ENDERPIN_TARGET_JDK");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let classes = output.join("bridge-classes");
    fs::create_dir_all(&classes).expect("create bridge classes");
    let mut javac = Command::new("javac");
    javac.args(["--release", "17", "-d"]).arg(&classes);
    for name in [
        "AuthAgent",
        "AuthBridge",
        "RealmsConnection",
        "MethodAdapter",
        "NativeIO",
        "RemotePrivateKey",
        "RemoteProvider",
        "RemoteSignature",
    ] {
        javac.arg(format!("java/auth-bridge/me/aomona/auth/{name}.java"));
    }
    javac.args([
        "java/narrator-bridge/com/mojang/text2speech/Narrator.java",
        "java/narrator-bridge/com/mojang/text2speech/LauncherNarrator.java",
        "java/windows-paths/me/aomona/sandbox/WindowsPaths.java",
        "java/windows-paths/me/aomona/sandbox/NativePaths.java",
        "java/windows-paths/me/aomona/sandbox/PathAdapter.java",
    ]);
    run(&mut javac);
    let paths_manifest = output.join("windows-paths.mf");
    fs::write(&paths_manifest,
        "Manifest-Version: 1.0\nPremain-Class: me.aomona.sandbox.WindowsPaths\nCan-Retransform-Classes: true\n\n")
        .expect("write Windows path manifest");
    jar(
        &output,
        &classes,
        "windows-paths.jar",
        &[
            "me/aomona/sandbox/WindowsPaths.class",
            "me/aomona/sandbox/WindowsPaths$1.class",
            "me/aomona/sandbox/PathAdapter.class",
        ],
        Some(&paths_manifest),
    );
    jar(
        &output,
        &classes,
        "windows-paths-bootstrap.jar",
        &["me/aomona/sandbox/NativePaths.class"],
        None,
    );
    fs::copy(
        "java/auth-bridge/auth-adapters.properties",
        classes.join("auth-adapters.properties"),
    )
    .expect("copy verified authentication mappings");
    let manifest = output.join("auth-manifest.mf");
    fs::write(
        &manifest,
        "Manifest-Version: 1.0\nPremain-Class: me.aomona.auth.AuthAgent\n\n",
    )
    .expect("write agent manifest");
    jar(
        &output,
        &classes,
        "auth-bridge.jar",
        &[
            "auth-adapters.properties",
            "me/aomona/auth/AuthAgent.class",
            "me/aomona/auth/AuthAgent$1.class",
            "me/aomona/auth/MethodAdapter.class",
        ],
        Some(&manifest),
    );
    jar(
        &output,
        &classes,
        "auth-bootstrap.jar",
        &[
            "me/aomona/auth/AuthBridge.class",
            "me/aomona/auth/RealmsConnection.class",
            "me/aomona/auth/RealmsConnection$1.class",
            "me/aomona/auth/NativeIO.class",
            "me/aomona/auth/RemotePrivateKey.class",
            "me/aomona/auth/RemoteProvider.class",
            "me/aomona/auth/RemoteSignature.class",
        ],
        None,
    );
    jar(
        &output,
        &classes,
        "narrator-bridge.jar",
        &["com/mojang/text2speech"],
        None,
    );
    let java = Command::new("java")
        .args(["-XshowSettings:properties", "-version"])
        .output()
        .expect("JDK required");
    let settings = String::from_utf8_lossy(&java.stderr);
    let home = env::var_os("ENDERPIN_TARGET_JDK")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                settings
                    .lines()
                    .find_map(|line| line.trim().strip_prefix("java.home = "))
                    .expect("Java home"),
            )
        });
    let os = env::var("CARGO_CFG_TARGET_OS").expect("target OS");
    let (headers, native) = match os.as_str() {
        "windows" => ("win32", "auth-bridge.dll"),
        "macos" => ("darwin", "libauth-bridge.dylib"),
        "linux" => ("linux", "libauth-bridge.so"),
        _ => panic!("unsupported sandbox OS"),
    };
    assert!(
        home.join("include").join(headers).is_dir(),
        "target JNI headers missing; set ENDERPIN_TARGET_JDK for cross compilation"
    );
    let mut build = cc::Build::new();
    build
        .include(home.join("include"))
        .include(home.join("include").join(headers));
    let tool = build.get_compiler();
    let mut native_cmd = tool.to_command();
    if tool.is_like_msvc() {
        native_cmd
            .arg("/LD")
            .arg("java/auth-bridge/native.c")
            .arg(format!("/Fe:{}", output.join(native).display()));
    } else {
        native_cmd
            .args([
                "-shared",
                "-fPIC",
                "-Wall",
                "-Wextra",
                "-Werror",
                "java/auth-bridge/native.c",
                "-o",
            ])
            .arg(output.join(native));
    }
    run(&mut native_cmd);
    if os == "windows" {
        let mut paths_native = tool.to_command();
        if tool.is_like_msvc() {
            paths_native
                .arg("/LD")
                .arg("java/windows-paths/native.c")
                .arg(format!("/Fo{}", output.join("windows-paths.obj").display()))
                .arg(format!(
                    "/Fe:{}",
                    output.join("windows-paths.dll").display()
                ));
        } else {
            paths_native
                .args([
                    "-shared",
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    "java/windows-paths/native.c",
                    "-o",
                ])
                .arg(output.join("windows-paths.dll"));
        }
        run(&mut paths_native);
    }
    println!("cargo:rustc-env=ENDERPIN_AUTH_NATIVE={native}");
}
