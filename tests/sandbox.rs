//! Opt-in enforcement check with a real JVM, independent of Minecraft login/EULA.
use anyhow::{Context, Result, ensure};
use enderpin::sandbox::{self, Policy};

#[test]
#[ignore = "requires ENDERPIN_TEST_JAVA_HOME and an available OS sandbox"]
fn jvm_file_and_network_isolation() -> Result<()> {
    let home = std::path::PathBuf::from(
        std::env::var_os("ENDERPIN_TEST_JAVA_HOME").context("set ENDERPIN_TEST_JAVA_HOME")?,
    )
    .canonicalize()?;
    let root = tempfile::tempdir()?;
    let root_path = root.path().canonicalize()?;
    let game = root_path.join("game");
    let temporary = root_path.join("tmp");
    let readonly = root_path.join("readonly");
    for path in [&game, &temporary, &readonly] {
        std::fs::create_dir(path)?;
    }
    let secret = root_path.join("secret.txt");
    std::fs::write(&secret, "private-test-data")?;
    std::fs::write(readonly.join("fixed.txt"), "immutable")?;
    std::fs::write(
        game.join("Probe.java"),
        r#"
import java.nio.file.*;
import java.net.*;
public class Probe {
  interface Attempt { void run() throws Exception; }
  static void denied(String label, Attempt operation) throws Exception {
    try { operation.run(); } catch (java.io.IOException | SecurityException expected) {
      System.out.println("denied " + label); return;
    }
    throw new AssertionError("unexpected permission: " + label);
  }
  public static void main(String[] args) throws Exception {
    System.out.println("probe started");
    Path game=Path.of(args[0]), tmp=Path.of(args[1]), ro=Path.of(args[2]), secret=Path.of(args[3]);
    Files.writeString(game.resolve("allowed.txt"), "ok");
    Files.writeString(tmp.resolve("allowed.txt"), "ok");
    if (!Files.readString(ro.resolve("fixed.txt")).equals("immutable")) throw new AssertionError();
    denied("private read", () -> Files.readString(secret));
    denied("private write", () -> Files.writeString(secret, "changed"));
    denied("runtime write", () -> Files.writeString(ro.resolve("fixed.txt"), "changed"));
    if (System.getProperty("os.name").contains("Mac")) {
      denied("child process", () -> new ProcessBuilder(Path.of(System.getProperty("java.home"), "bin/java").toString(), "-version").start());
    }
    if (args[4].equals("allow")) {
      System.out.println("testing allowed loopback");
      try (ServerSocket server=new ServerSocket(0, 1, InetAddress.getLoopbackAddress());
           Socket client=new Socket()) {
        server.setSoTimeout(5000);
        client.connect(new InetSocketAddress(InetAddress.getLoopbackAddress(), server.getLocalPort()), 5000);
        try (Socket accepted=server.accept()) {
          accepted.setSoTimeout(5000);
          client.getOutputStream().write(42);
          if (accepted.getInputStream().read()!=42) throw new AssertionError();
        }
      }
    } else {
      if (!System.getProperty("os.name").startsWith("Windows")) {
        denied("network bind", () -> { try (ServerSocket s=new ServerSocket(0)) {} });
      }
      denied("network connect", () -> { try (Socket s=new Socket()) { s.connect(new InetSocketAddress("127.0.0.1", Integer.parseInt(args[5])), 1000); } });
    }
    System.out.println("PASS");
  }
}
"#,
    )?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let java = home.join(if cfg!(windows) {
        "bin/java.exe"
    } else {
        "bin/java"
    });
    for network in [false, true] {
        println!("starting sandbox probe: network={network}");
        let policy = Policy {
            game: game.clone(),
            temporary: temporary.clone(),
            java_home: home.clone(),
            readonly: vec![readonly.clone()],
            network,
            desktop: false,
        };
        let args: Vec<std::ffi::OsString> = vec![
            "-XX:-UsePerfData".into(),
            format!(
                "-Djava.io.tmpdir={}",
                enderpin::storage::java_path(&temporary).display()
            )
            .into(),
            enderpin::storage::java_path(&game.join("Probe.java")).into(),
            enderpin::storage::java_path(&game).into(),
            enderpin::storage::java_path(&temporary).into(),
            enderpin::storage::java_path(&readonly).into(),
            enderpin::storage::java_path(&secret).into(),
            if network { "allow" } else { "deny" }.into(),
            listener.local_addr()?.port().to_string().into(),
        ];
        #[cfg(not(windows))]
        let output = sandbox::command(&java, &policy)?.args(&args).output()?;
        #[cfg(windows)]
        let output = sandbox::windows::output(&java, &policy, &args)?;
        ensure!(
            output.status.success(),
            "sandbox probe failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        ensure!(
            String::from_utf8_lossy(&output.stdout).contains("PASS"),
            "probe did not finish"
        );
        println!(
            "network={network}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    ensure!(
        std::fs::read_to_string(&secret)? == "private-test-data",
        "host file changed"
    );
    Ok(())
}
