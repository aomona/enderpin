"""Package a native build for installation with Bun (Python 3.11+)."""

import argparse
import io
import json
import platform
from pathlib import Path
import subprocess
import tarfile
import tomllib


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--output", type=Path, default=Path("target/bun"))
    parser.add_argument("--tag", help="Require a release tag matching Cargo.toml")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    cargo = tomllib.loads((root / "Cargo.toml").read_text())["package"]
    version = cargo["version"]
    if args.tag is not None and args.tag != f"v{version}":
        parser.error(f"release tag must be v{version}")

    system = {"Darwin": "darwin", "Linux": "linux", "Windows": "win32"}.get(
        platform.system()
    )
    cpu = {"x86_64": "x64", "amd64": "x64", "aarch64": "arm64", "arm64": "arm64"}.get(
        platform.machine().lower()
    )
    if system is None or cpu is None:
        parser.error("unsupported OS or CPU")
    binary = args.binary.resolve(strict=True)
    actual = subprocess.check_output([str(binary), "--version"], text=True).strip()
    if actual != f"enderpin {version}":
        parser.error(f"binary version mismatch: {actual}")
    executable = "enderpin.exe" if system == "win32" else "enderpin"
    metadata = {
        "name": "enderpin",
        "version": version,
        "description": cargo["description"],
        "license": cargo["license"],
        "repository": {"type": "git", "url": "https://github.com/aomona/enderpin.git"},
        "bin": {"enderpin": f"bin/{executable}"},
        "os": [system],
        "cpu": [cpu],
    }
    args.output.mkdir(parents=True, exist_ok=True)
    archive = args.output / f"enderpin-{system}-{cpu}.tgz"
    with tarfile.open(archive, "w:gz") as package:
        data = (json.dumps(metadata, indent=2) + "\n").encode()
        info = tarfile.TarInfo("package/package.json")
        info.size = len(data)
        info.mode = 0o644
        package.addfile(info, io.BytesIO(data))
        info = package.gettarinfo(binary, f"package/bin/{executable}")
        info.mode = 0o755
        with binary.open("rb") as source:
            package.addfile(info, source)
        for name in ["LICENSE", "README.md", "PLAN.md", "docs"]:
            package.add(root / name, f"package/{name}")
    print(archive)


if __name__ == "__main__":
    main()
