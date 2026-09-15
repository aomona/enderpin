"""Exercise npm/Bun entry-package installs in disposable global directories."""

import functools
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading


def command_line(arguments):
    executable = shutil.which(arguments[0]) or arguments[0]
    args = [executable, *arguments[1:]]
    if os.name == "nt" and executable.lower().endswith(".cmd"):
        # Pass cmd.exe a raw command line: a list would escape its inner quotes
        # a second time, passing literal quotes/backslashes to npm.
        return 'cmd.exe /d /s /c "' + subprocess.list2cmdline(args) + '"'
    return args


def repack(source, destination, metadata):
    with tarfile.open(source) as original, tarfile.open(destination, "w:gz") as output:
        for member in original.getmembers():
            if member.name == "package/package.json":
                data = (json.dumps(metadata) + "\n").encode()
                member.size = len(data)
                output.addfile(member, io.BytesIO(data))
            else:
                output.addfile(member, original.extractfile(member) if member.isfile() else None)


def main():
    entry = Path(sys.argv[1]).resolve(strict=True)
    native = Path(sys.argv[2]).resolve(strict=True) if len(sys.argv) > 2 else None
    with tarfile.open(entry) as package:
        metadata = json.load(package.extractfile("package/package.json"))
        assert not metadata.get("scripts")
        assert len(metadata["optionalDependencies"]) == 5
        assert all(url.startswith(
            f"https://github.com/aomona/enderpin/releases/download/v{metadata['version']}/"
        ) for url in metadata["optionalDependencies"].values())

    with tempfile.TemporaryDirectory(prefix="enderpin npm ") as temp:
        root = Path(temp)
        serving = root / "packages"
        serving.mkdir()
        server = ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(
            SimpleHTTPRequestHandler, directory=str(serving)))
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            archive = entry
            if native:
                with tarfile.open(native) as package:
                    platform_metadata = json.load(package.extractfile("package/package.json"))
                assert platform_metadata["version"] == metadata["version"]
                # Local URLs exist only inside this temporary test package.
                for name in metadata["optionalDependencies"]:
                    _, system, cpu = name.split("-")
                    repack(native, serving / f"{name}.tgz", {
                        **platform_metadata, "os": [system], "cpu": [cpu]})
                    metadata["optionalDependencies"][name] = (
                        f"http://127.0.0.1:{server.server_port}/{name}.tgz")
                archive = serving / "entry.tgz"
                repack(entry, archive, metadata)
            for manager in ("npm", "bun"):
                prefix = root / manager
                prefix.mkdir()
                env = {**os.environ,
                       "BUN_INSTALL_GLOBAL_DIR": str(prefix / "global"),
                       "BUN_INSTALL_BIN": str(prefix / "bin"),
                       "BUN_INSTALL_CACHE_DIR": str(prefix / "cache"),
                       "npm_config_cache": str(prefix / "cache")}
                command = [manager, "install", "-g", "--ignore-scripts"]
                if manager == "npm":
                    command += ["--prefix", str(prefix), "--no-audit", "--no-fund"]
                subprocess.run(command_line(command + [str(archive)]), cwd=prefix, env=env,
                               check=True, timeout=300)
                bins = prefix if manager == "npm" and os.name == "nt" else prefix / "bin"
                executable = shutil.which("enderpin", path=str(bins))
                assert executable, f"{manager} did not link the CLI"
                # .cmd shims need cmd.exe on Windows; quote all generated paths.
                def run(arguments, **kwargs):
                    args = command_line([executable, *arguments])
                    return subprocess.run(args, cwd=prefix, env=env, timeout=60, **kwargs)

                version = run(["--version"], capture_output=True, text=True, check=True)
                assert version.stdout.strip() == f"enderpin {metadata['version']}"
                assert run(["--invalid-option"], capture_output=True).returncode == 2
                workspace = prefix / "world with spaces"
                run(["-C", str(workspace), "init", "--minecraft", "1.21.1"], check=True)
                assert (workspace / "enderpin.toml").is_file()
                assert (workspace / "enderpin.lock").is_file()
                if manager == "bun":
                    installed = json.loads((prefix / "global/package.json").read_text())
                    assert set(installed["dependencies"]) == {"enderpin"}
                    # Bun can execute the entry point without a Node.js runtime.
                    run_args = [shutil.which("bun"), "x", "--bun", "enderpin", "--version"]
                    bun_env = {**env, "PATH": str(bins) + os.pathsep + os.defpath}
                    subprocess.run(run_args, cwd=prefix, env=bun_env, check=True, timeout=60)
                print(f"{manager}: isolated install, version, exit code and workspace checks passed")
        finally:
            server.shutdown()
            server.server_close()
            thread.join()


if __name__ == "__main__":
    main()
