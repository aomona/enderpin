"""Opt-in authenticated join on a disposable localhost server, using the OS-stored account.

python3 tests/live_client.py /path/to/locked/workspace --accept-eula --use-account
Opens the real Minecraft client. Does not send player chat or modify the source workspace.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import tempfile
import sys
import tomllib
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("workspace", type=Path)
    parser.add_argument("--cache-dir", type=Path)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--use-account", action="store_true", required=True)
    args = parser.parse_args()
    binary = Path(__file__).resolve().parents[1] / "target/debug" / ("enderpin.exe" if os.name == "nt" else "enderpin")
    with tempfile.TemporaryDirectory(prefix="enderpin-broker-live-") as directory:
        root = Path(directory)
        for name in ["enderpin.toml", "enderpin.lock"]:
            shutil.copyfile(args.workspace / name, root / name)
        base = [str(binary), "-C", str(root)]
        if args.cache_dir:
            base += ["--cache-dir", str(args.cache_dir.resolve())]
        for side in ["server", "client"]:
            cmd = base + ["sync", "--target", side]
            subprocess.run(cmd + ["--locked", "--offline"], check=True, capture_output=True, timeout=240)
            result = subprocess.run(base + ["permissions", "show", "--target", side, "--json"], check=True, capture_output=True, text=True, timeout=30)
            plan = json.loads(result.stdout)
            assert not plan["plan"]["folders"]
            subprocess.run(base + ["permissions", "--target", side, "approve", plan["fingerprint"]], check=True, capture_output=True, timeout=30)
        if args.cache_dir and sys.platform == "darwin":
            # APFS clones save space; the launcher still verifies every asset before use.
            locked = tomllib.loads((root / "enderpin.lock").read_text())["runtimes"]["client"]["metadata"]
            cache = args.cache_dir.resolve() / "runtime"
            metadata = json.loads((cache / (locked["algorithm"] + "-" + locked["hash"]) / locked["filename"]).read_text())
            assets = cache / ("assets-" + metadata["assetIndex"]["sha1"])
            subprocess.run(["cp", "-cR", str(assets), str(root / "client/run/enderpin-assets")], check=True)
        game = root / "server" / "run"
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        (game / "server.properties").write_text(f"server-ip=127.0.0.1\nserver-port={port}\nonline-mode=true\nwhite-list=false\nenforce-secure-profile=true\nview-distance=3\nsimulation-distance=3\n")
        (root / "client/run/options.txt").write_text("onboardAccessibility:false\n")
        server = client = None
        with (root / "server.log").open("w") as server_log, (root / "client.log").open("w") as client_log:
            def until(path, fragment, process, timeout=300, failure_log=None):
                deadline = time.monotonic() + timeout
                while time.monotonic() < deadline:
                    text = path.read_text(errors="replace")
                    if fragment in text:
                        return
                    if process.poll() is not None:
                        detail = failure_log.read_text(errors="replace") if failure_log else text
                        raise RuntimeError("game exited before " + fragment + ":\n" + detail[-10000:])
                    time.sleep(0.25)
                raise RuntimeError("timeout waiting for " + fragment + ":\n" + path.read_text(errors="replace")[-10000:])
            try:
                server = subprocess.Popen(base + ["run", "--target", "server", "--offline", "--accept-eula", "--memory", "1024"], stdin=subprocess.PIPE, stdout=server_log, stderr=subprocess.STDOUT, text=True, start_new_session=os.name != "nt")
                until(root / "server.log", "Done (", server)
                client = subprocess.Popen(base + ["run", "--target", "client", "--offline", "--connect", f"127.0.0.1:{port}", "--memory", "2048"], stdin=subprocess.DEVNULL, stdout=client_log, stderr=subprocess.STDOUT, text=True, start_new_session=os.name != "nt")
                until(root / "server.log", "joined the game", client, failure_log=root / "client.log")
                # Minecraft must not persist a private signing key in its old cache.
                for path in (root / "client/run/profilekeys").glob("**/*"):
                    if path.is_file():
                        assert b"PRIVATE KEY" not in path.read_bytes(), "game-visible private key cache"
                print(json.dumps({"authenticated_join": True, "secure_profile_server": True, "game_private_key_cache": False, "player_chat_sent": False}))
            finally:
                if client is not None and client.poll() is None:
                    if os.name == "nt":
                        # Closing Enderpin also closes its kill-on-close AppContainer job.
                        client.terminate()
                    else:
                        client.send_signal(signal.SIGINT)
                    try:
                        client.wait(timeout=30)
                    except subprocess.TimeoutExpired:
                        client.kill()
                        client.wait()
                if server is not None and server.poll() is None:
                    server.stdin.write("stop\n")
                    server.stdin.flush()
                    try:
                        server.wait(timeout=40)
                    except subprocess.TimeoutExpired:
                        server.kill()
                        server.wait()
                if os.name != "nt":
                    for process in (client, server):
                        if process is not None:
                            try:
                                os.killpg(process.pid, signal.SIGTERM)
                            except ProcessLookupError:
                                pass


if __name__ == "__main__":
    main()
