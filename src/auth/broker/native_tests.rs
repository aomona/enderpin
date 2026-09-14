//! Real JNI and OS isolation, with a public synthetic key and no account traffic.
use super::{
    channel::PreparedBroker,
    chat::{
        ChatKeys,
        tests::{ACCOUNT, certificate},
    },
    protocol::{BrokerError, Command},
    service::Operations,
};
use anyhow::{Context, Result, ensure};
use std::{path::PathBuf, time::SystemTime};

struct Fixture(ChatKeys);
impl Operations for Fixture {
    fn valid(&self) -> bool {
        true
    }
    fn execute(&mut self, command: &Command) -> Result<serde_json::Value, BrokerError> {
        match command {
            Command::Certificate {} => self
                .0
                .install(certificate(SystemTime::now()), SystemTime::now()),
            Command::Sign { key_id, message } => {
                self.0.sign(key_id, message, ACCOUNT, SystemTime::now())
            }
            _ => Err(BrokerError::Unsupported),
        }
    }
}

#[test]
#[ignore = "requires ENDERPIN_TEST_JAVA_HOME and an available OS sandbox"]
fn native_bridge_signs_without_exporting_keys_and_enforces_denial() -> Result<()> {
    let home = PathBuf::from(
        std::env::var_os("ENDERPIN_TEST_JAVA_HOME").context("set ENDERPIN_TEST_JAVA_HOME")?,
    )
    .canonicalize()?;
    let root = tempfile::tempdir()?;
    let root = root.path().canonicalize()?;
    let game = crate::storage::directory(&root, "game")?;
    let tmp = crate::storage::directory(&root, "tmp")?;
    let bridge = crate::storage::directory(&root, "bridge")?;
    crate::bridges::write_files(&bridge)?;
    std::fs::write(
        game.join("WindowsIpcSmoke.java"),
        include_str!("../../../java/auth-bridge-smoke/WindowsIpcSmoke.java"),
    )?;
    let value = certificate(SystemTime::now());
    std::fs::write(
        bridge.join("public.pem"),
        value["keyPair"]["publicKey"]
            .as_str()
            .context("public key")?,
    )?;
    let policy = crate::sandbox::Policy {
        game: game.clone(),
        temporary: tmp,
        java_home: home.clone(),
        readonly: vec![bridge.clone()],
        network: false,
        desktop: false,
        permissions: Default::default(),
        extra: vec![],
    };
    let java = home.join(if cfg!(windows) {
        "bin/java.exe"
    } else {
        "bin/java"
    });
    for permitted in [false, true] {
        let broker = PreparedBroker::new(Box::new(Fixture(ChatKeys::default())), permitted)?;
        #[cfg(unix)]
        let handle = 3;
        #[cfg(windows)]
        let handle = {
            use std::os::windows::io::AsRawHandle;
            broker.child_handle().as_raw_handle() as usize
        };
        let arguments: Vec<std::ffi::OsString> = vec![
            format!(
                "-javaagent:{}={}",
                crate::storage::java_path(&bridge.join("auth-bridge.jar")).display(),
                crate::storage::java_path(&crate::bridges::native(&bridge)).display()
            )
            .into(),
            format!("-Dmonalauncher.auth.handle={handle}").into(),
            "-XX:-UsePerfData".into(),
            crate::storage::java_path(&game.join("WindowsIpcSmoke.java")).into(),
            if permitted { "allowed" } else { "denied" }.into(),
            crate::storage::java_path(&bridge.join("public.pem")).into(),
            "f".repeat(64).into(),
        ];
        #[cfg(unix)]
        let output = {
            let mut command = crate::sandbox::command_with_ipc(&java, &policy, true)?;
            broker.configure(&mut command)?;
            command.args(&arguments).output()?
        };
        #[cfg(windows)]
        let output = crate::sandbox::windows::output_inherited(
            &java,
            &policy,
            &arguments,
            &[broker.child_handle()],
        )?;
        ensure!(
            output.status.success(),
            "native broker failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        ensure!(
            String::from_utf8_lossy(&output.stdout).contains(if permitted {
                "JNI_SIGNATURE_AND_FOREIGN_KEY_OK"
            } else {
                "NETWORK_DENIED_OK"
            }),
            "broker probe did not complete"
        );
    }
    std::fs::write(
        game.join("NarratorPolicySmoke.java"),
        include_str!("../../../java/narrator-bridge-smoke/NarratorPolicySmoke.java"),
    )?;
    for permitted in [false, true] {
        let args: Vec<std::ffi::OsString> = vec![
            "-XX:-UsePerfData".into(),
            "-cp".into(),
            crate::storage::java_path(&bridge.join("narrator-bridge.jar")).into(),
            crate::storage::java_path(&game.join("NarratorPolicySmoke.java")).into(),
            permitted.to_string().into(),
        ];
        #[cfg(unix)]
        let output = crate::sandbox::command(&java, &policy)?
            .args(&args)
            .output()?;
        #[cfg(windows)]
        let output = crate::sandbox::windows::output(&java, &policy, &args)?;
        ensure!(
            output.status.success(),
            "narrator bridge policy failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}
