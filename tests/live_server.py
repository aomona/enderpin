"""Opt-in real server smoke test using a disposable workspace.

Run: python3 tests/live_server.py /path/to/workspace --accept-eula [--offline]
Or: python3 tests/live_server.py --quick --accept-eula [--version 26.2] [--no-sandbox]
--quick sets up a vanilla workspace before explicitly approving and launching the server.
The explicit flag accepts https://aka.ms/MinecraftEULA for the disposable server.
This verifies status/console/shutdown, not authenticated player participation.
"""
import argparse
import json
import os
from pathlib import Path
import queue
import re
import shutil
import signal
import socket
import struct
import subprocess
import tempfile
import threading
import time


def varint(number):
    result = bytearray()
    while number > 127:
        result.append((number & 127) | 128)
        number >>= 7
    result.append(number)
    return bytes(result)


def status(port):
    with socket.create_connection(("127.0.0.1", port), timeout=5) as connection:
        host = b"127.0.0.1"
        handshake = b"\0" + varint(767) + varint(len(host)) + host + struct.pack(">H", port) + b"\1"
        connection.sendall(varint(len(handshake)) + handshake + b"\1\0")

        def read_varint():
            number = 0
            for shift in range(0, 35, 7):
                value = connection.recv(1)
                assert value, "server closed status connection"
                number |= (value[0] & 127) << shift
                if value[0] < 128:
                    return number
            raise AssertionError("invalid status varint")

        assert read_varint() <= 1024 * 1024, "status packet too large"
        assert read_varint() == 0, "unexpected packet type"
        remaining = read_varint()
        assert remaining <= 1024 * 1024, "status JSON too large"
        payload = bytearray()
        while remaining:
            chunk = connection.recv(remaining)
            assert chunk, "truncated status"
            payload.extend(chunk)
            remaining -= len(chunk)
        return json.loads(payload)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("workspace", type=Path, nargs="?")
    parser.add_argument("--quick", action="store_true")
    parser.add_argument("--version", help="quick Minecraft version (otherwise latest stable)")
    parser.add_argument("--no-sandbox", action="store_true", help="quick vanilla server only")
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--cache-dir", type=Path)
    parser.add_argument("--binary", type=Path, default=Path(__file__).resolve().parents[1] / "target/debug" / ("enderpin.exe" if os.name == "nt" else "enderpin"))
    args = parser.parse_args()
    if not args.quick and (args.workspace is None or args.version or args.no_sandbox):
        parser.error("provide a workspace, or use --quick [--version VERSION] [--no-sandbox]")
    if args.quick and args.offline:
        parser.error("quick resolves official metadata online")
    with tempfile.TemporaryDirectory(prefix="enderpin-server-smoke-") as temporary:
        base = Path(temporary)
        version = args.version
        if args.quick and version is None:
            from urllib.request import urlopen
            with urlopen("https://piston-meta.mojang.com/mc/game/version_manifest_v2.json", timeout=30) as response:
                version = json.load(response)["latest"]["release"]
        root = base
        if not args.quick:
            for name in ["enderpin.toml", "enderpin.lock"]:
                shutil.copyfile(args.workspace / name, root / name)
        game = root / "server"
        game.mkdir(parents=True)
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        (game / "server.properties").write_text(
            f"server-ip=127.0.0.1\nserver-port={25565 if args.quick else port}\nonline-mode=true\n"
            "enforce-secure-profile=true\nview-distance=3\nsimulation-distance=3\n"
        )
        command = [str(args.binary.resolve()), "-C", str(root), "--target", "server"]
        if args.cache_dir:
            command += ["--cache-dir", str(args.cache_dir.resolve())]
        offline = ["--offline"] if args.offline else []
        if args.quick:
            setup = [str(args.binary.resolve()), "--no-interactive", "quick", "--server",
                     "--version", version, "--loader", "vanilla"]
            if args.cache_dir:
                setup += ["--cache-dir", str(args.cache_dir.resolve())]
            subprocess.run(setup, cwd=base, check=True, capture_output=True, text=True, timeout=1800)
            assert not (game / "eula.txt").exists(), "setup must not accept EULA or launch"
        subprocess.run(command + ["sync", "--locked"] + offline, check=True, capture_output=True, text=True, timeout=180)
        inspected = subprocess.run(command + ["--json", "permissions", "show"], check=True, capture_output=True, text=True, timeout=30)
        permission_plan = json.loads(inspected.stdout)
        # Only authorize a vanilla server with no managed mods or host folders.
        assert not permission_plan["plan"]["folders"]
        assert not permission_plan["plan"]["packages"]
        subprocess.run(command + ["permissions", "approve", permission_plan["fingerprint"]], check=True, capture_output=True, text=True, timeout=30)
        launch = command + ["launch", "--accept-eula", "--memory", "1024", "--port", str(port)] + offline
        if args.no_sandbox:
            launch += ["--no-sandbox"]
        process = subprocess.Popen(launch, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.STDOUT, text=True, bufsize=1, cwd=base)
        messages = queue.Queue()
        threading.Thread(target=lambda: [messages.put(line) for line in process.stdout], daemon=True).start()
        recent = []
        game_pid = None

        def until(fragment, timeout=180):
            nonlocal game_pid
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                try:
                    line = messages.get(timeout=0.2)
                except queue.Empty:
                    assert process.poll() is None, "server exited: " + "".join(recent[-25:])
                    continue
                recent.append(line)
                match = re.fullmatch(r"(?:Sandboxed|Unconfined) server started \(PID (\d+)\)\. Press Ctrl-C to stop\.\n", line)
                if match and game_pid is None:
                    game_pid = int(match.group(1))
                if fragment in line:
                    return
            raise AssertionError("server timeout: " + "".join(recent[-25:]))

        def console(line):
            process.stdin.write(line + "\n")
            process.stdin.flush()

        try:
            until("Done (", timeout=600)
            # Newer servers publish their status after the initial Done message.
            deadline = time.monotonic() + 10
            while True:
                try:
                    observed = status(port)
                    break
                except (OSError, AssertionError):
                    if time.monotonic() >= deadline:
                        raise
                    time.sleep(0.2)
            assert observed["players"]["online"] == 0
            if args.quick:
                assert (base / "client").is_dir()
                assert (base / "server").is_dir()
                assert (base / "enderpin.toml").is_file()
                assert not (base / "server" / version).exists()
                assert not (base / ".enderpin/server/game").exists()
                assert observed["version"]["name"] == version
                assert "online-mode=true" in (game / "server.properties").read_text()
                assert "eula=true" in (game / "eula.txt").read_text()
            rejected = subprocess.run(command + ["sync", "--locked"] + offline, capture_output=True, text=True, timeout=30)
            assert rejected.returncode and "server is running" in rejected.stderr
            console("list")
            until("players online:", timeout=10)
            console("stop")
            assert process.wait(timeout=40) == 0, "server did not stop cleanly"
            assert (game / "world/level.dat").is_file(), "world was not saved"
            synced = subprocess.run(command + ["sync", "--locked"] + offline, capture_output=True, text=True, timeout=30)
            assert synced.returncode == 0, synced.stderr
            print(json.dumps({"minecraft": observed["version"]["name"], "sandbox": not args.no_sandbox, "quick": args.quick, "port": port,
                              "status": True, "console": True, "running_sync_rejected": True,
                              "world_saved": True, "shutdown": True, "authenticated_join": "not tested by this script"}, indent=2))
        except Exception:
            print("".join(recent[-40:]))
            raise
        finally:
            if process.poll() is None:
                try:
                    console("stop")
                    process.wait(timeout=40)
                except (OSError, subprocess.TimeoutExpired):
                    process.kill()
                    process.wait()
                    if os.name != "nt" and game_pid is not None:
                        try:
                            os.killpg(game_pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass


if __name__ == "__main__":
    main()
