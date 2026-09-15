"""Opt-in real Fabric server smoke test, using a disposable copy of a locked workspace.

Run: python3 tests/live_server.py /path/to/workspace --accept-eula [--offline]
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
    parser.add_argument("workspace", type=Path)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--cache-dir", type=Path)
    parser.add_argument("--binary", type=Path, default=Path(__file__).resolve().parents[1] / "target/debug" / ("enderpin.exe" if os.name == "nt" else "enderpin"))
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="enderpin-server-smoke-") as temporary:
        root = Path(temporary)
        for name in ["enderpin.toml", "enderpin.lock"]:
            shutil.copyfile(args.workspace / name, root / name)
        game = root / ".enderpin/server/game"
        game.mkdir(parents=True)
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        (game / "server.properties").write_text(
            f"server-ip=127.0.0.1\nserver-port={port}\nonline-mode=true\n"
            "enforce-secure-profile=true\nview-distance=3\nsimulation-distance=3\n"
        )
        command = [str(args.binary.resolve()), "-C", str(root), "--target", "server"]
        if args.cache_dir:
            command += ["--cache-dir", str(args.cache_dir.resolve())]
        offline = ["--offline"] if args.offline else []
        subprocess.run(command + ["sync", "--locked"] + offline, check=True, capture_output=True, text=True, timeout=180)
        inspected = subprocess.run(command + ["--json", "permissions", "show"], check=True, capture_output=True, text=True, timeout=30)
        permission_plan = json.loads(inspected.stdout)
        # Only authorize the disposable server's baseline: never silently approve
        # mod-supplied requests or host folders as part of this smoke test.
        assert not permission_plan["plan"]["folders"]
        assert all(not p["embedded"] and not p["repository"] for p in permission_plan["plan"]["packages"])
        subprocess.run(command + ["permissions", "approve", permission_plan["fingerprint"]], check=True, capture_output=True, text=True, timeout=30)
        process = subprocess.Popen(command + ["launch", "--accept-eula", "--memory", "1024"] + offline,
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   text=True, bufsize=1)
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
                match = re.fullmatch(r"Sandboxed server started \(PID (\d+)\)\. Press Ctrl-C to stop\.\n", line)
                if match and game_pid is None:
                    game_pid = int(match.group(1))
                if fragment in line:
                    return
            raise AssertionError("server timeout: " + "".join(recent[-25:]))

        def console(line):
            process.stdin.write(line + "\n")
            process.stdin.flush()

        try:
            until("Done (")
            observed = status(port)
            assert observed["players"]["online"] == 0
            rejected = subprocess.run(command + ["sync", "--locked"] + offline, capture_output=True, text=True, timeout=30)
            assert rejected.returncode and "server is running" in rejected.stderr
            console("list")
            until("players online:", timeout=10)
            console("stop")
            assert process.wait(timeout=40) == 0, "server did not stop cleanly"
            assert (game / "world/level.dat").is_file(), "world was not saved"
            synced = subprocess.run(command + ["sync", "--locked"] + offline, capture_output=True, text=True, timeout=30)
            assert synced.returncode == 0, synced.stderr
            print(json.dumps({"minecraft": observed["version"]["name"], "sandbox": True,
                              "status": True, "console": True, "running_sync_rejected": True,
                              "world_saved": True, "shutdown": True, "authenticated_join": "not tested by this script"}, indent=2))
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
