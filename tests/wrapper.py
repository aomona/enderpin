"""Exercise the shipped launchers with native argv, a local release and no network."""
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import shlex
import shutil
import signal
import subprocess
import sys
import tarfile
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
WINDOWS = sys.platform == "win32"
SYSTEM = {"darwin": "darwin", "linux": "linux", "win32": "win32"}[sys.platform]
CPU = "arm64" if platform.machine().lower() in ("arm64", "aarch64") else "x64"
PLATFORM = f"{SYSTEM}-{CPU}"
EXE = "enderpin.exe" if WINDOWS else "enderpin"
PROBE = r'''
use std::{io::{Read, Write}, time::Duration};
fn main() {
    if let Ok(path) = std::env::var("PROBE_READY") {
        std::fs::write(path, std::process::id().to_string()).unwrap();
        loop { std::thread::sleep(Duration::from_millis(50)); }
    }
    for arg in std::env::args().skip(1) {
        println!("arg={}", arg.as_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>());
    }
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    print!("stdin={input}");
    std::io::stdout().flush().unwrap();
    std::process::exit(37);
}
'''

with tempfile.TemporaryDirectory(prefix="enderpin wrapper ") as temporary:
    root = Path(temporary)
    workspace = root / "repo with spaces"
    workspace.mkdir()
    for name in ("enderpinw", "enderpinw.ps1"):
        shutil.copyfile(ROOT / "src" / "wrapper" / name, workspace / name)
    (workspace / "enderpinw").chmod(0o755)
    source = root / "probe.rs"
    source.write_text(PROBE, encoding="utf-8")
    binary = root / EXE
    subprocess.run(["rustc", str(source), "-o", str(binary)], check=True)
    archive = root / "release.tgz"
    with tarfile.open(archive, "w:gz") as package:
        package.add(binary, arcname=f"package/bin/{EXE}")
        # A release archive cannot choose destinations outside the cache.
        entry = tarfile.TarInfo("../escaped")
        entry.size = 4
        package.addfile(entry, io.BytesIO(b"oops"))
    archive_hash = hashlib.sha256(archive.read_bytes()).hexdigest()
    binary_hash = hashlib.sha256(binary.read_bytes()).hexdigest()
    properties = workspace / "enderpin-wrapper.properties"
    original = (f"version=0.1.4\narchive.{PLATFORM}.sha256={archive_hash}\n"
                f"binary.{PLATFORM}.sha256={binary_hash}\n")
    properties.write_text(original, encoding="utf-8", newline="\n")
    env = {**os.environ, "ENDERPIN_WRAPPER_CACHE": str(root / "cache with spaces"),
           "WRAPPER_ARCHIVE": str(archive), "WRAPPER_NETWORK": str(root / "network"),
           "WRAPPER_SCRIPT": str(workspace / "enderpinw.ps1")}
    if WINDOWS:
        # Do not inherit PowerShell 7 module paths when testing Windows PowerShell 5.1.
        env = {key: value for key, value in env.items() if key.upper() != "PSMODULEPATH"}
        harness = root / "launch.ps1"
        harness.write_text(r'''
$ErrorActionPreference = 'Stop'
function Invoke-WebRequest {
    param([switch]$UseBasicParsing, $Uri, $OutFile, $TimeoutSec)
    if ($env:WRAPPER_NETWORK_DISABLED) { throw 'network disabled' }
    [IO.File]::AppendAllText($env:WRAPPER_NETWORK, "$Uri`n")
    [IO.File]::Copy($env:WRAPPER_ARCHIVE, $OutFile)
}
$forwarded = @(ConvertFrom-Json $env:WRAPPER_ARGS)
& $env:WRAPPER_SCRIPT @forwarded
exit $LASTEXITCODE
''', encoding="utf-8")
        command = ["powershell.exe", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", str(harness)]
    else:
        commands = root / "commands"
        commands.mkdir()
        downloader = root / "download.py"
        downloader.write_text('''
import os, pathlib, shutil, sys
if os.environ.get("WRAPPER_NETWORK_DISABLED"): sys.exit(22)
args = sys.argv[1:]
url = next(a for a in args if a.startswith("https://"))
with open(os.environ["WRAPPER_NETWORK"], "a") as log: log.write(url + "\\n")
shutil.copyfile(os.environ["WRAPPER_ARCHIVE"], args[args.index("--output") + 1])
''', encoding="utf-8")
        curl = commands / "curl"
        curl.write_text(f"#!/bin/sh\nexec {shlex.quote(sys.executable)} {shlex.quote(str(downloader))} \"$@\"\n", encoding="utf-8")
        curl.chmod(0o755)
        env["PATH"] = str(commands) + os.pathsep + env["PATH"]
        command = [str(workspace / "enderpinw")]

    arguments = ["sync", "a b", "", 'say "hello"', "日本語", "--target", "server",
                 "C:\\path with spaces\\", 'slash\\"quote', "$HOME;$(false)"]

    def run(args=arguments):
        options = {**env, "WRAPPER_ARGS": json.dumps(args)}
        return subprocess.run(command + ([] if WINDOWS else args), cwd=root,
                              env=options, input="console input\n", text=True,
                              capture_output=True, timeout=20)

    def success():
        result = run()
        expected = "".join(f"arg={arg.encode().hex()}\n" for arg in arguments) + "stdin=console input\n"
        assert result.returncode == 37, (result.returncode, result.stdout, result.stderr)
        assert result.stdout == expected, (result.stdout, expected)

    success()
    cache = Path(env["ENDERPIN_WRAPPER_CACHE"]) / "0.1.4" / PLATFORM / binary_hash / EXE
    assert cache.read_bytes() == binary.read_bytes()
    network = Path(env["WRAPPER_NETWORK"])
    assert network.read_text().splitlines() == [f"https://github.com/aomona/enderpin/releases/download/v0.1.4/enderpin-{PLATFORM}.tgz"]
    assert not list(root.rglob("escaped")), "archive traversal"
    env["WRAPPER_NETWORK_DISABLED"] = "1"
    success()  # Cached execution must not need the network.
    assert len(network.read_text().splitlines()) == 1
    cache.write_bytes(b"corrupt")
    assert run().returncode == 1
    cache.unlink()
    assert run().returncode != 0  # Download failure must never launch anything.
    del env["WRAPPER_NETWORK_DISABLED"]
    properties.write_text(original.replace(archive_hash, "0" * 64), encoding="utf-8")
    assert run().returncode == 1
    assert not cache.exists()
    properties.write_text(original.replace(binary_hash, "0" * 64), encoding="utf-8")
    assert run().returncode == 1
    assert not cache.exists()
    attempts = len(network.read_text().splitlines())
    for invalid in (original + "version=0.1.5\n", original.replace("0.1.4", "../../escape"),
                    original.replace(archive_hash, "invalid")):
        properties.write_text(invalid, encoding="utf-8")
        assert run().returncode == 1
    assert len(network.read_text().splitlines()) == attempts
    properties.write_text(original, encoding="utf-8")
    success()
    properties.write_text(original.replace("0.1.4", "0.1.5"), encoding="utf-8")
    success()
    assert network.read_text().splitlines()[-1] == f"https://github.com/aomona/enderpin/releases/download/v0.1.5/enderpin-{PLATFORM}.tgz"
    properties.write_text(original, encoding="utf-8")
    env["WRAPPER_NETWORK_DISABLED"] = "1"
    success()  # Checking out the previous pin reuses its separate cached executable.
    if not WINDOWS:
        ready = root / "ready"
        process = subprocess.Popen(command, env={**env, "PROBE_READY": str(ready)},
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            deadline = time.monotonic() + 10
            while not ready.exists() and time.monotonic() < deadline:
                time.sleep(0.02)
            assert ready.exists(), "probe did not start"
            assert int(ready.read_text()) == process.pid, "wrapper did not exec its child"
            process.send_signal(signal.SIGTERM)
            assert process.wait(timeout=5) == -signal.SIGTERM
        finally:
            if process.poll() is None:
                process.kill()
            process.communicate()
    assert not list(Path(env["ENDERPIN_WRAPPER_CACHE"]).rglob(".download.*"))
print(f"Wrapper verified on {PLATFORM}: download, cache, hashes, argv, stdin, exit and fail-closed errors")
