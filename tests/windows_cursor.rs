//! Real ClipCursor regression; reapplies existing bounds without moving the mouse.
#![cfg(windows)]
use anyhow::{Context, Result, ensure};
use enderpin::sandbox::{self, Policy};

#[test]
#[ignore = "requires interactive Windows desktop, Java 25 and ENDERPIN_TEST_DESKTOP=1"]
fn cursor_confinement_requires_a_scoped_desktop_capability() -> Result<()> {
    if std::env::var_os("ENDERPIN_TEST_DESKTOP").is_none() {
        eprintln!(
            "Cursor probe skipped: set ENDERPIN_TEST_DESKTOP=1 on an interactive desktop with Java 25"
        );
        return Ok(());
    }
    let home = std::path::PathBuf::from(
        std::env::var_os("ENDERPIN_TEST_JAVA_HOME")
            .context("set ENDERPIN_TEST_JAVA_HOME to Java 25")?,
    )
    .canonicalize()?;
    let root = tempfile::tempdir()?;
    let game = root.path().join("game");
    let temporary = root.path().join("tmp");
    std::fs::create_dir(&game)?;
    std::fs::create_dir(&temporary)?;
    std::fs::write(
        game.join("CursorProbe.java"),
        r#"
import java.lang.foreign.*;
import java.lang.invoke.*;
public class CursorProbe {
  public static void main(String[] args) throws Throwable {
    System.loadLibrary("user32");
    Linker linker = Linker.nativeLinker();
    SymbolLookup symbols = SymbolLookup.loaderLookup();
    MethodHandle getClip = linker.downcallHandle(symbols.find("GetClipCursor").orElseThrow(),
      FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS));
    MethodHandle clip = linker.downcallHandle(symbols.find("ClipCursor").orElseThrow(),
      FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS),
      Linker.Option.captureCallState("GetLastError"));
    try (Arena arena = Arena.ofConfined()) {
      MemorySegment rectangle = arena.allocate(16, 4);
      MemorySegment state = arena.allocate(Linker.Option.captureStateLayout());
      int readClip = (int)getClip.invokeExact(rectangle);
      if (readClip == 0) throw new AssertionError("GetClipCursor failed");
      int clipped = (int)clip.invokeExact(state, rectangle);
      int error = state.get(ValueLayout.JAVA_INT, 0);
      System.out.println("ClipCursor existing bounds: result=" + clipped + " error=" + error);
      if (clipped == 0) throw new AssertionError("cursor confinement denied");
    }
  }
}
"#,
    )?;
    let mut policy = Policy {
        game: game.canonicalize()?,
        temporary: temporary.canonicalize()?,
        java_home: home.clone(),
        readonly: vec![],
        network: false,
        desktop: false,
        permissions: Default::default(),
        extra: vec![],
    };
    let java = home.join("bin/java.exe");
    let args: Vec<std::ffi::OsString> = vec![
        "--enable-native-access=ALL-UNNAMED".into(),
        "-XX:-UsePerfData".into(),
        format!(
            "-Djava.io.tmpdir={}",
            enderpin::storage::java_path(&policy.temporary).display()
        )
        .into(),
        enderpin::storage::java_path(&game.join("CursorProbe.java")).into(),
    ];
    let ordinary = std::process::Command::new(&java).args(&args).output()?;
    ensure!(
        ordinary.status.success(),
        "ordinary cursor probe failed: {}",
        String::from_utf8_lossy(&ordinary.stderr)
    );
    eprintln!("ordinary: {}", String::from_utf8_lossy(&ordinary.stdout));
    for desktop in [false, true, false] {
        policy.desktop = desktop;
        let isolated = sandbox::windows::output(&java, &policy, &args)?;
        let stdout = String::from_utf8_lossy(&isolated.stdout);
        eprintln!("AppContainer desktop={desktop}: {stdout}");
        ensure!(
            isolated.status.success() == desktop,
            "unexpected cursor permission result: {}",
            String::from_utf8_lossy(&isolated.stderr)
        );
        ensure!(
            stdout.contains(if desktop {
                "result=1"
            } else {
                "result=0 error=5"
            }),
            "unexpected ClipCursor result: {stdout}"
        );
    }
    Ok(())
}
