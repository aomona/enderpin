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
    output = [] if args[0] == "init" else ["--json"]
    prompts = ["--no-interactive"] if args[0] in {"init", "add", "search"} else []
    result = subprocess.run(
        [str(binary), "-C", str(root), "--cache-dir", str(cache), *args, *output, *prompts],
        capture_output=True, text=True, timeout=1800 if args[0] == "init" else 240,
    )
    if success and result.returncode:
        raise AssertionError(result.stderr)
    if not success:
        assert result.returncode, "expected rejection"
        return result.stderr
    return json.loads(result.stdout) if output else None


def inspect(root, side):
    lock = tomllib.loads((root / "enderpin.lock").read_text())
    packages = lock["targets"][side]["packages"]
    directories = {"mod": "mods", "resourcepack": "resourcepacks", "shader": "shaderpacks", "plugin": "plugins"}
    for p in packages:
        path = root / "run" / side / directories[p["kind"]] / p["filename"]
        assert hashlib.sha512(path.read_bytes()).hexdigest() == p["sha512"]
        assert path.stat().st_size == p["size"]
    return packages


with tempfile.TemporaryDirectory(prefix="enderpin-smoke-") as directory:
    base = Path(directory)
    origin, clone, cache = base / "origin", base / "clone", base / "cache"
    origin.mkdir()
    clone.mkdir()
    run(origin, cache, "init", "--minecraft", "1.21.1", "--loader", "fabric", "--loader-version", "0.16.10", "--target", "all")
    search = run(origin, cache, "search", "sodium")
    assert any(p["slug"] == "sodium" for p in search["hits"])
    run(origin, cache, "add", "sodium", "--target", "all")
    run(origin, cache, "add", "lithium", "--target", "all")
    run(origin, cache, "add", "iris", "--target", "client")
    sodium_pin = next(p for p in inspect(origin, "client") if p["name"] == "sodium")["version"]
    run(origin, cache, "add", "sodium", "--version-id", sodium_pin, "--target", "all")
    run(origin, cache, "update", "--target", "all")
    lock = tomllib.loads((origin / "enderpin.lock").read_text())
    assert all(runtime["loader_version"] == "0.16.10" for runtime in lock["runtimes"].values())
    assert next(p for p in inspect(origin, "client") if p["name"] == "sodium")["version"] == sodium_pin
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
    path = clone / "client/run/mods" / sodium["filename"]
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
    run(urls, cache, "init", "--minecraft", "1.21.1", "--loader", "fabric")
    run(urls, cache, "add", sodium["url"], "--kind", "mod", "--name", "custom-sodium")
    url_package = inspect(urls, "client")[0]
    assert url_package["sha512"] == sodium["sha512"]
    run(urls, cache, "remove", "custom-sodium")
    assert not list((urls / "client/run/mods").iterdir())
    print(json.dumps({
        "search": "passed", "update_preserves_explicit_pins": "passed", "client_artifacts": len(client), "server_artifacts": len(server),
        "offline_clone": "passed", "manual_change_and_restore": "passed", "ignore_dependency_check": "passed",
        "https_url_and_remove": "passed", "game_launch": "not exercised",
        "artifacts": [{"name": p["name"], "version": p.get("version"), "sha512": p["sha512"]} for p in client],
    }, indent=2))
