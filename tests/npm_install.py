"""Exercise npm/Bun installs and pnpm dlx without URL subdependencies."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile


def command_line(arguments):
    executable = shutil.which(arguments[0]) or arguments[0]
    args = [executable, *arguments[1:]]
    if os.name == "nt" and executable.lower().endswith(".cmd"):
        # Pass cmd.exe a raw command line: a list would escape its inner quotes
        # a second time, passing literal quotes/backslashes to npm.
        return 'cmd.exe /d /s /c "' + subprocess.list2cmdline(args) + '"'
    return args


def main():
    entry = Path(sys.argv[1]).resolve(strict=True)
    registry_install = len(sys.argv) > 2 and sys.argv[2] == "--registry"
    # Invoke npm itself: version-manager shims may create global links even
    # when --prefix points at our disposable directory (observed with Vite+).
    node = Path(subprocess.check_output(
        ["node", "-p", "process.execPath"], text=True).strip())
    npm_cli = node.parent / ("node_modules/npm/bin/npm-cli.js" if os.name == "nt"
                             else "../lib/node_modules/npm/bin/npm-cli.js")
    if not npm_cli.is_file():
        npm_cli = Path(shutil.which("npm") or "npm").resolve()
        if npm_cli.name != "npm-cli.js" or not npm_cli.is_file():
            raise RuntimeError("Put a standard Node.js/npm installation on PATH for this test")
    with tarfile.open(entry) as package:
        metadata = json.load(package.extractfile("package/package.json"))
        assert not metadata.get("scripts")
        assert not metadata.get("dependencies") and not metadata.get("optionalDependencies")
        binaries = [member for member in package.getmembers() if member.name.startswith("package/native/")]
        assert 1 <= len(binaries) <= 5 if metadata.get("private") else len(binaries) == 5
        assert all(member.isfile() and member.mode & 0o111 for member in binaries)
        assert not (registry_install and metadata.get("private")), "test packages cannot be published"

    with tempfile.TemporaryDirectory(prefix="enderpin npm ") as temp:
        root = Path(temp)
        for manager in ("npm", "bun"):
            prefix = root / manager
            prefix.mkdir()
            env = {**os.environ,
                   "BUN_INSTALL_GLOBAL_DIR": str(prefix / "global"),
                   "BUN_INSTALL_BIN": str(prefix / "bin"),
                   "BUN_INSTALL_CACHE_DIR": str(prefix / "cache"),
                   "npm_config_cache": str(prefix / "cache")}
            command = ([str(node), str(npm_cli)] if manager == "npm" else [manager])
            command += ["install", "-g", "--ignore-scripts"]
            if manager == "npm":
                command += ["--prefix", str(prefix), "--no-audit", "--no-fund"]
            spec = f"enderpin@{metadata['version']}" if registry_install else str(entry)
            subprocess.run(command_line(command + [spec]), cwd=prefix, env=env,
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

        prefix = root / "pnpm"
        prefix.mkdir()
        (prefix / "pnpm-workspace.yaml").write_text(
            "blockExoticSubdeps: true\nignoreScripts: true\n"
            + "storeDir: " + json.dumps(str(prefix / "store")) + "\n"
            + "cacheDir: " + json.dumps(str(prefix / "cache")) + "\n")
        spec = f"enderpin@{metadata['version']}" if registry_install else str(entry)
        command = command_line(["pnpm", "dlx", spec, "--version"])
        result = subprocess.run(command, cwd=prefix, capture_output=True, text=True,
                                check=True, timeout=300)
        assert result.stdout.strip() == f"enderpin {metadata['version']}", result.stdout
        print("pnpm: dlx passed with blockExoticSubdeps enabled")


if __name__ == "__main__":
    main()
