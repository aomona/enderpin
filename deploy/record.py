"""Non-secret deployment inputs. Never read vault values while building a record."""
import difflib
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

import yaml


SECRET_VARIABLES = {"enderpin_forwarding_secret", "ansible_password", "ansible_become_password",
                    "ansible_ssh_pass", "ansible_become_pass"}
INVENTORY_VARIABLES = {
    "ansible_connection", "ansible_python_interpreter", "ansible_host", "ansible_user", "ansible_port",
    "ansible_ssh_common_args", "ansible_ssh_private_key_file",
    "enderpin_root", "enderpin_binary", "enderpin_binary_version", "enderpin_binary_sha256",
    "enderpin_accept_eula", "enderpin_proxy_address", "enderpin_backend_interface",
    "enderpin_backends", "enderpin_try", "enderpin_vpn", "enderpin_vpn_clients",
    "enderpin_pki_directory", "enderpin_instances", "enderpin_decommission",
    "enderpin_inspector", "enderpin_inspector_version", "enderpin_inspector_sha256",
}


def encoded(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode()


def digest(content):
    return hashlib.sha256(content).hexdigest()


def environment():
    return {key: value for key, value in os.environ.items()
            if key not in {"ENDERPIN_INVENTORY", "ENDERPIN_VAULT", "ENDERPIN_VAULT_PASSWORD"}}


def run(arguments, **kwargs):
    # Neither a parser error nor a failing subprocess may echo secret input/output.
    result = subprocess.run(arguments, env=environment(), stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, **kwargs)
    if result.returncode:
        raise ValueError("deployment input inspection or application failed; subprocess output withheld")
    return result.stdout


def non_secret(value):
    """Fail closed on secret-bearing fields and inventory lookups before hashing or logging."""
    if isinstance(value, dict):
        for key, item in value.items():
            if not isinstance(key, str) or re.search(r"secret|password|credential|token|private[-_]?key$", key, re.I):
                raise ValueError("secret-bearing fields are not allowed in deployment configuration")
            if key != "ansible_python_interpreter" or item != "{{ ansible_playbook_python }}":
                non_secret(item)
    elif isinstance(value, list):
        for item in value:
            non_secret(item)
    elif isinstance(value, str):
        if any(marker in value for marker in ["{{", "{%", "{#", "-----BEGIN", "$ANSIBLE_VAULT"]):
            raise ValueError("deployment configuration must be literal non-secret data")
    elif value is not None and not isinstance(value, (bool, int, float)):
        raise ValueError("deployment configuration must contain JSON-compatible values")


def inventory(path, playbook_dir=None):
    try:
        config = yaml.safe_load(path.read_bytes())
        non_secret(config)
    except (OSError, yaml.YAMLError, KeyError, TypeError):
        raise ValueError("cannot read a static non-secret YAML inventory") from None
    for directory in {path.parent, playbook_dir or path.parent}:
        if any((directory / name).exists() for name in ["group_vars", "host_vars"]):
            raise ValueError("put deployment variables in the single versioned inventory, not sidecar directories")
    output = run([str(Path(sys.executable).with_name("ansible-inventory")), "-i", str(path), "--list"],
                 cwd=playbook_dir, text=True)
    try:
        resolved = json.loads(output)
        hosts = resolved["_meta"]["hostvars"]
        for variables in hosts.values():
            if set(variables) - INVENTORY_VARIABLES:
                raise ValueError("inventory contains unsupported variables; put credentials in the vault")
            non_secret(variables)
        if not resolved.get("minecraft"):
            raise ValueError("inventory must select Minecraft hosts")
    except (KeyError, TypeError, json.JSONDecodeError):
        raise ValueError("invalid resolved inventory") from None
    return resolved


def members(inventory, group):
    result = list(inventory.get(group, {}).get("hosts", []))
    for child in inventory.get(group, {}).get("children", []):
        result.extend(members(inventory, child))
    return list(dict.fromkeys(result))


def binary(path, version, expected):
    if not Path(path).is_absolute():
        raise ValueError("binary paths must be absolute controller paths")
    if not isinstance(version, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9.+_-]{0,127}", version):
        raise ValueError("binary release/version must be pinned")
    if not isinstance(expected, str) or not re.fullmatch(r"[0-9a-f]{64}", expected):
        raise ValueError("binary sha256 must be pinned")
    if digest(Path(path).read_bytes()) != expected:
        raise ValueError("deployment binary does not match its pinned sha256")
    return {"version": version, "sha256": expected}


def configuration_revision(root, path, revision, external_revision=None, expected=None):
    if path.is_relative_to(root):
        relative = str(path.relative_to(root))
        run(["git", "ls-files", "--error-unmatch", "--", relative], cwd=root)
        return {"revision": revision, "path": relative}
    if not external_revision or not expected or not re.fullmatch(r"[0-9a-f]{64}", expected):
        raise ValueError("protected inventory requires its immutable revision and sha256")
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._:/@+-]{0,255}", external_revision):
        raise ValueError("invalid protected configuration revision")
    if digest(path.read_bytes()) != expected:
        raise ValueError("protected inventory does not match its pinned sha256")
    return {"revision": external_revision, "sha256": expected}


def secret_reference(reference, revision):
    for value in [reference, revision]:
        if not value or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._:/@+-]{0,255}", value):
            raise ValueError("provide a secret store reference and immutable revision, never a value or hash")
    return {"reference": reference, "revision": revision}


def difference(previous, current):
    return "".join(difflib.unified_diff(encoded(previous).decode().splitlines(True),
                                      encoded(current).decode().splitlines(True),
                                      fromfile="previous deployment", tofile="proposed deployment"))


def write(path, value):
    # Atomic replacement keeps the last successful record intact on interrupted writes.
    with tempfile.NamedTemporaryFile(dir=path.parent, prefix=".enderpin-record-", delete=False) as output:
        temporary = Path(output.name)
        try:
            output.write(value)
            output.flush()
            os.fsync(output.fileno())
            output.close()
            temporary.replace(path)
        finally:
            temporary.unlink(missing_ok=True)


def tracked_inputs(root, source):
    tracked = set(record.decode() for record in run(["git", "ls-files", "-z"], cwd=root).split(b"\0") if record)
    workspace = Path(source["path"])
    paths = [workspace / "enderpin.toml", workspace / "enderpin.lock"]
    for scope in source["inputs"]:
        paths.extend(path for path in (workspace / scope).rglob("*") if path.is_file())
    if any(str(path.relative_to(root)) not in tracked for path in paths):
        raise ValueError("all deployed workspace inputs must be tracked; ignored local files cannot be reproduced")


def frozen_inventory(resolved):
    # Preserve membership and effective host variables without loading sidecar files again.
    return {"all": {"hosts": resolved["_meta"]["hostvars"], "children": {
        group: {"hosts": {host: {} for host in members(resolved, group)}}
        for group in resolved if group not in {"_meta", "all"}
    }}}
