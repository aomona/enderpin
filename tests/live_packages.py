"""Opt-in real-network package smoke test. Run after cargo build --locked.

Uses disposable workspaces and a private cache; never modifies an existing game.
Python 3.11+. The emitted report distinguishes artifact verification from game launch.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib


binary = Path(__file__).resolve().parents[1] / "target" / "debug" / (
    "enderpin.exe" if os.name == "nt" else "enderpin"
)


def run(root, cache, *args, success=True):
    result = subprocess.run(
        [str(binary), "-C", str(root), "--cache-dir", str(cache), "--no-interactive", "--json", *args],
        capture_output=True, text=True, timeout=240,
    )
    if success and result.returncode:
        raise AssertionError(result.stderr)
    if not success:
        assert result.returncode, "expected rejection"
        return result.stderr
    return json.loads(result.stdout)


def inspect(root, side):
    lock = tomllib.loads((root / "enderpin.lock").read_text())
    packages = lock["targets"][side]["packages"]
    directories = {"mod": "mods", "resourcepack": "resourcepacks", "shader": "shaderpacks", "plugin": "plugins"}
    for p in packages:
        path = root / ".enderpin" / side / "game" / directories[p["kind"]] / p["filename"]
        assert hashlib.sha512(path.read_bytes()).hexdigest() == p["sha512"]
        assert path.stat().st_size == p["size"]
    return packages


with tempfile.TemporaryDirectory(prefix="enderpin-smoke-") as directory:
    base = Path(directory)
    origin, clone, cache = base / "origin", base / "clone", base / "cache"
    origin.mkdir()
    clone.mkdir()
    run(origin, cache, "init", "--minecraft", "1.21.1")
    search = run(origin, cache, "search", "sodium")
    assert any(p["slug"] == "sodium" for p in search["hits"])
    run(origin, cache, "add", "sodium", "--target", "all")
    run(origin, cache, "add", "lithium", "--target", "all")
    run(origin, cache, "add", "iris", "--target", "client")
    client = inspect(origin, "client")
    server = inspect(origin, "server")
    assert any(p["name"] == "sodium" for p in client)
    assert all(p["name"] != "sodium" for p in server)
    assert any(p["name"] == "lithium" for p in server)
    for name in ("enderpin.toml", "enderpin.lock"):
        shutil.copy2(origin / name, clone / name)
    snapshot = (clone / "enderpin.lock").read_bytes()
    run(clone, cache, "sync", "--target", "all", "--locked", "--offline")
    assert (clone / "enderpin.lock").read_bytes() == snapshot
    assert inspect(clone, "client") == client
    assert inspect(clone, "server") == server

    sodium = next(p for p in client if p["name"] == "sodium")
    path = clone / ".enderpin/client/game/mods" / sodium["filename"]
    path.write_bytes(b"manually modified")
    assert "modified" in run(clone, cache, "sync", "--locked", "--offline", success=False)
    run(clone, cache, "restore", "--offline")
    assert hashlib.sha512(path.read_bytes()).hexdigest() == sodium["sha512"]
    (clone / ".enderpinignore").write_text("client:sodium\n")
    assert "requires" in run(clone, cache, "sync", "--locked", "--offline", success=False)
    (clone / ".enderpinignore").write_text("client:iris\nclient:sodium\n")
    run(clone, cache, "sync", "--locked", "--offline")
    assert not path.exists()
    run(clone, cache, "sync", "--locked", "--offline", "--no-ignore")
    assert path.exists()

    # URL mode uses the same public artifact through its explicit file URL.
    urls = base / "urls"
    urls.mkdir()
    run(urls, cache, "init", "--minecraft", "1.21.1")
    run(urls, cache, "add", sodium["url"], "--kind", "mod", "--name", "custom-sodium")
    url_package = inspect(urls, "client")[0]
    assert url_package["sha512"] == sodium["sha512"]
    run(urls, cache, "remove", "custom-sodium")
    assert not list((urls / ".enderpin/client/game/mods").iterdir())
    print(json.dumps({
        "search": "passed", "client_artifacts": len(client), "server_artifacts": len(server),
        "offline_clone": "passed", "manual_change_and_restore": "passed", "ignore_dependency_check": "passed",
        "https_url_and_remove": "passed", "game_launch": "not exercised",
        "artifacts": [{"name": p["name"], "version": p.get("version"), "sha512": p["sha512"]} for p in client],
    }, indent=2))
