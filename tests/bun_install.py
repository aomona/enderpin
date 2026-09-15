"""Install a package URL with Bun in isolation and exercise the native CLI."""

import functools
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading


def main():
    archive = Path(sys.argv[1]).resolve(strict=True)
    with tarfile.open(archive) as package:
        metadata = json.load(package.extractfile("package/package.json"))
        assert not metadata.get("scripts"), "installation must not require scripts"
        assert not metadata.get("dependencies"), "package must be self-contained"
        assert package.getmember("package/" + metadata["bin"]["enderpin"]).mode & 0o111

    with tempfile.TemporaryDirectory(prefix="enderpin-bun-") as temp:
        root = Path(temp)
        bins = root / "bin"
        config = root / "bunfig.toml"
        config.write_text(
            "[install]\n"
            f"globalDir = {json.dumps(str(root / 'global'))}\n"
            f"globalBinDir = {json.dumps(str(bins))}\n"
        )
        handler = functools.partial(SimpleHTTPRequestHandler, directory=str(archive.parent))
        server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            subprocess.run(
                ["bun", "install", "-g", "--ignore-scripts", f"--config={config}",
                 f"--cache-dir={root / 'cache'}",
                 f"http://127.0.0.1:{server.server_port}/{archive.name}"],
                cwd=root, check=True, timeout=120,
            )
        finally:
            server.shutdown()
            server.server_close()
            thread.join()

        executable = shutil.which("enderpin", path=str(bins))
        assert executable, "Bun did not expose the enderpin command"
        env = {**os.environ, "PATH": str(bins) + os.pathsep + os.defpath}
        version = subprocess.check_output([executable, "--version"], env=env, text=True)
        assert version.strip() == f"enderpin {metadata['version']}"
        subprocess.run([executable, "--help"], env=env, check=True, stdout=subprocess.DEVNULL)
        invalid = subprocess.run([executable, "--invalid-option"], env=env, capture_output=True)
        assert invalid.returncode == 2, "CLI error exit code must be preserved"
        workspace = root / "world with spaces"
        subprocess.run(
            [executable, "-C", str(workspace), "init", "--minecraft", "1.21.1"],
            env=env, check=True,
        )
        assert (workspace / "enderpin.toml").is_file()
        assert (workspace / "enderpin.lock").is_file()
        print(f"Bun URL install and native CLI checks passed: {archive.name}")


if __name__ == "__main__":
    main()
