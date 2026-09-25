"""Bundle native binaries into one dependency-free npm package."""

import argparse
import io
import json
from pathlib import Path
import tarfile
import tomllib


PLATFORMS = ("darwin-arm64", "darwin-x64", "linux-arm64", "linux-x64", "win32-x64")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("target/npm"))
    parser.add_argument("--tag", help="Require a release tag matching Cargo.toml")
    parser.add_argument("--binaries", type=Path, default=Path("target/bun"))
    parser.add_argument("--allow-partial", action="store_true",
                        help="Build a private test package with available platforms only")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    cargo = tomllib.loads((root / "Cargo.toml").read_text())["package"]
    version = cargo["version"]
    if args.tag is not None and args.tag != f"v{version}":
        parser.error(f"release tag must be v{version}")
    metadata = {
        "name": "enderpin",
        "version": version,
        "description": cargo["description"],
        "license": cargo["license"],
        "repository": {"type": "git", "url": "https://github.com/aomona/enderpin.git"},
        "bin": {"enderpin": "enderpin.cjs"},
        "engines": {"node": ">=18"},
        "publishConfig": {"access": "public", "registry": "https://registry.npmjs.org"},
    }
    archives = {platform: args.binaries / f"enderpin-{platform}.tgz" for platform in PLATFORMS}
    if args.allow_partial:
        archives = {platform: path for platform, path in archives.items() if path.is_file()}
        metadata["private"] = True
    if not archives or any(not path.is_file() for path in archives.values()):
        parser.error("all five native archives are required; use --allow-partial only for local tests")

    args.output.mkdir(parents=True, exist_ok=True)
    archive = args.output / f"enderpin-{version}.tgz"
    temporary = archive.with_suffix(".tmp")
    with tarfile.open(temporary, "w:gz") as package:
        data = (json.dumps(metadata, indent=2) + "\n").encode()
        info = tarfile.TarInfo("package/package.json")
        info.size, info.mode = len(data), 0o644
        package.addfile(info, io.BytesIO(data))
        info = package.gettarinfo(root / "npm/enderpin.cjs", "package/enderpin.cjs")
        info.mode = 0o755
        with (root / "npm/enderpin.cjs").open("rb") as source:
            package.addfile(info, source)
        for platform, path in archives.items():
            system, cpu = platform.split("-")
            executable = "enderpin.exe" if system == "win32" else "enderpin"
            with tarfile.open(path) as native:
                manifest = json.load(native.extractfile("package/package.json"))
                if (manifest["name"] != "enderpin" or manifest["version"] != version
                        or manifest["os"] != [system] or manifest["cpu"] != [cpu]):
                    parser.error(f"native package identity mismatch: {path}")
                member = native.getmember(f"package/bin/{executable}")
                if not member.isfile():
                    parser.error(f"native binary must be a regular file: {path}")
                info = tarfile.TarInfo(f"package/native/{platform}/{executable}")
                info.size, info.mode = member.size, 0o755
                package.addfile(info, native.extractfile(member))
        for name in ["LICENSE", "README.md"]:
            package.add(root / name, f"package/{name}")
    temporary.replace(archive)
    print(archive)


if __name__ == "__main__":
    main()
