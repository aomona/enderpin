"""Build the npm entry package using the matching GitHub Release binaries."""

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
        "optionalDependencies": {
            f"enderpin-{platform}":
                f"https://github.com/aomona/enderpin/releases/download/v{version}/enderpin-{platform}.tgz"
            for platform in PLATFORMS
        },
    }
    args.output.mkdir(parents=True, exist_ok=True)
    archive = args.output / f"enderpin-{version}.tgz"
    with tarfile.open(archive, "w:gz") as package:
        data = (json.dumps(metadata, indent=2) + "\n").encode()
        info = tarfile.TarInfo("package/package.json")
        info.size, info.mode = len(data), 0o644
        package.addfile(info, io.BytesIO(data))
        info = package.gettarinfo(root / "npm/enderpin.cjs", "package/enderpin.cjs")
        info.mode = 0o755
        with (root / "npm/enderpin.cjs").open("rb") as source:
            package.addfile(info, source)
        for name in ["LICENSE", "README.md"]:
            package.add(root / name, f"package/{name}")
    print(archive)


if __name__ == "__main__":
    main()
