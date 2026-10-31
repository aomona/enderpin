"""Validate deployment inputs before touching hosts or rendering secrets."""
import ipaddress
from pathlib import Path
import re
import json
import subprocess

from ansible.errors import AnsibleFilterError


def workspace(source_root, relative, target, inspector):
    root = Path(source_root).resolve(strict=True)
    path = root / relative
    if Path(relative).is_absolute() or ".." in Path(relative).parts:
        raise AnsibleFilterError("workspace must be relative to the source repository")
    if not path.is_dir() or not path.resolve().is_relative_to(root):
        raise AnsibleFilterError("workspace must be a directory inside the source repository")
    for parent in [path, *path.parents]:
        if parent == root:
            break
        if parent.is_symlink():
            raise AnsibleFilterError("workspace cannot contain symbolic links")
    def safe_name(name):
        if not isinstance(name, str) or not re.fullmatch(r"[a-z0-9_-]{1,64}", name) or name in {"all", "con", "prn", "aux", "nul"} or re.fullmatch(r"(?:com|lpt)[1-9]", name):
            raise AnsibleFilterError("invalid target or group name")
    safe_name(target)
    for name in ["enderpin.toml", "enderpin.lock"]:
        if (path / name).is_symlink():
            raise AnsibleFilterError("deployment inputs cannot contain symbolic links")
    if not Path(inspector).is_absolute():
        raise AnsibleFilterError("enderpin_inspector must be an absolute path to a reviewed controller binary")
    try:
        result = subprocess.run([str(inspector), "-C", str(path), "config", "show", "--resolved",
                                 "--target", target, "--json"], capture_output=True, text=True, timeout=60)
        if result.returncode:
            failure = json.loads(result.stderr)
            if failure.get("schema_version") == 1 and failure.get("error", {}).get("code") == "unknown_target":
                raise AnsibleFilterError(f"unknown deployment target: {target}")
            raise AnsibleFilterError("Enderpin rejected the source configuration; inspect it with config show")
        document = json.loads(result.stdout)
        if document.get("schema_version") != 1:
            raise AnsibleFilterError("unsupported Enderpin resolved configuration schema")
        config = document["targets"][target]
        if any(item["severity"] == "error" for item in config["diagnostics"]):
            raise AnsibleFilterError("Enderpin reported an incompatible source configuration")
        if any(config["lock"][scope]["status"] != "current" for scope in ["packages", "runtime"]):
            raise AnsibleFilterError("deployment requires current package and runtime locks")
        runtime = config["runtime"]["loader"]
        if runtime not in ["paper", "fabric", "velocity"] or config["type"] not in ["server", "proxy"]:
            raise AnsibleFilterError("deployment requires a prepared Paper, Fabric or Velocity server lock")
        inputs = config["file_inputs"]
        packages = {alias: item["declaration"] for alias, item in config["packages"].items()}
    except (OSError, subprocess.TimeoutExpired, ValueError, KeyError, TypeError, AttributeError):
        # Do not log subprocess output: even an incompatible binary may print secrets.
        raise AnsibleFilterError("cannot inspect workspace with the reviewed Enderpin binary") from None
    for relative_input in inputs:
        item = path / relative_input
        if any(parent.is_symlink() for parent in [item, *item.parents] if parent != root) or (item.is_dir() and any(p.is_symlink() for p in item.rglob("*"))):
            raise AnsibleFilterError("deployment inputs cannot contain symbolic links")
    if integration == "velocity" and runtime == "fabric":
        disabled = config.get("disabled", [])
        aliases = [name for name, package in packages.items()
                   if package.get("modrinth") == "fabricproxy-lite" and package.get("kind", "mod") == "mod"
                   and name not in disabled and "fabricproxy-lite" not in disabled]
        if not aliases:
            raise AnsibleFilterError("Fabric backends require an enabled direct fabricproxy-lite Modrinth package")
        if not any(set(p.get("roots", [])) & set(aliases) and p.get("kind") == "mod"
                   and p.get("name") not in disabled and p.get("project") not in disabled
                   for p in config["lock"]["packages"]["identities"]):
            raise AnsibleFilterError("FabricProxy-Lite must be present and enabled in the server package lock")
    if integration == "velocity" and runtime == "paper":
        # shortcut: the managed forwarding template uses the Paper 1.19+ configuration layout.
        version = config["runtime"]["minecraft"].split(".")
        if len(version) < 2 or not version[0].isdigit() or not version[1].isdigit() or (int(version[0]), int(version[1])) < (1, 19):
            raise AnsibleFilterError("the Paper deployment template requires Minecraft 1.19 or newer")
    return {"path": str(path), "runtime": runtime, "target": target,
            "inputs": [item for item in inputs if (path / item).is_dir()],
            "files": config["files"], "launch": config["launch"], "sources": config["sources"]}



def instances(items):
    names, ports = set(), set()
    for item in items:
        name = item["name"]
        if not re.fullmatch(r"[a-z][a-z0-9_]{0,23}", name) or name in names or name == "firewall":
            raise AnsibleFilterError("instance names must be unique lower-case service identifiers")
        if not isinstance(item.get("target"), str) or not item["target"]:
            raise AnsibleFilterError(f"instance {name} requires an explicit non-empty target")
        address = ipaddress.ip_address(item["address"])
        port = item["port"]
        if isinstance(port, bool) or not isinstance(port, int) or not 1 <= port <= 65535 or port in ports:
            raise AnsibleFilterError("each instance on a host needs a distinct TCP port (1..65535)")
        if address.version != 4:
            raise AnsibleFilterError("the initial deployment templates require IPv4 bind addresses")
        if item.get("integration", "none") not in {"none", "velocity"}:
            raise AnsibleFilterError("integration must be none or velocity")
        if any(key in item for key in ["memory_mib", "properties", "motd", "show_max_players"]):
            raise AnsibleFilterError("move memory_mib to enderpin.toml and native game settings to files/; inventory overrides are no longer implicit")
        for flag in ["fabric_proxy_hack_early_send", "fabric_proxy_hack_message_chain"]:
            if flag in item and not isinstance(item[flag], bool):
                raise AnsibleFilterError(f"{flag} must be a boolean")
        names.add(name)
        ports.add(port)
    return items


def backends(items):
    for name, endpoint in items.items():
        if not re.fullmatch(r"[a-z][a-z0-9_]{0,23}", name) or name == "try":
            raise AnsibleFilterError("backend names must be safe server identifiers other than try")
        address, port = endpoint.rsplit(":", 1)
        if ipaddress.ip_address(address).version != 4 or not port.isdigit() or not 1 <= int(port) <= 65535:
            raise AnsibleFilterError("backend endpoints must be IPv4:port")
    return items


def deployment(items, source_root, interface, proxy_address, inspector):
    return [dict(name=item["name"], address=item["address"], port=item["port"],
                 workspace=item["workspace"], target=source["target"], runtime=source["runtime"],
                 integration=item.get("integration", "none"),
                 interface=interface if item.get("integration") == "velocity" else "lo",
                 proxy_address=proxy_address if item.get("integration") == "velocity" else "127.0.0.1")
            for item in instances(items)
            for source in [workspace(source_root, item["workspace"], item["target"], inspector, item.get("integration", "none"))]]


def transition(desired, previous, decommission, services):
    if not isinstance(decommission, list) or any(
            not isinstance(name, str) or not re.fullmatch(r"[a-z][a-z0-9_]{0,23}", name)
            or name == "firewall" for name in decommission):
        raise AnsibleFilterError("enderpin_decommission must list instance names")
    if previous is None:
        # Older installations have no trustworthy port history: stop every old unit before adoption.
        required = {name.removeprefix("enderpin-").removesuffix(".service") for name in services
                    if name.startswith("enderpin-") and name.endswith(".service")
                    and name != "enderpin-firewall.service"}
        previous = []
    else:
        if not isinstance(previous, list):
            raise AnsibleFilterError("invalid deployment record; preserve it and the existing firewall")
        # Pre-integration records described Velocity-managed listeners only.
        previous = [entry | {"integration": entry.get("integration", "velocity")} if isinstance(entry, dict) else entry
                    for entry in previous]
        for entry in previous:
            if (not isinstance(entry, dict) or set(entry) != {
                    "name", "address", "port", "workspace", "target", "runtime", "interface", "proxy_address", "integration"}
                    or entry["runtime"] not in ["paper", "fabric", "velocity"]
                    or not isinstance(entry["interface"], str)
                    or not re.fullmatch(r"[a-zA-Z0-9_-]{1,15}", entry["interface"])):
                raise AnsibleFilterError("invalid deployment record; preserve it and the existing firewall")
            instances([entry])
            ipaddress.IPv4Address(entry["proxy_address"])
        required = {entry["name"] for entry in previous if entry not in desired}
    if required - set(decommission):
        raise AnsibleFilterError("explicit enderpin_decommission required for removed/changed or legacy instances: "
                                 + ", ".join(sorted(required - set(decommission))))
    if set(decommission) - required:
        raise AnsibleFilterError("decommission requests must name removed/changed or legacy instances")
    return {"decommission": sorted(required), "previous": previous,
            "protected": previous + [entry for entry in desired if entry not in previous]}


def vpn(config, clients):
    network = ipaddress.ip_network(config["network"])
    server = ipaddress.ip_address(config["server_address"])
    if network.version != 4 or network.prefixlen > 29 or network.prefixlen < 16 or server != network.network_address + 1:
        raise AnsibleFilterError("VPN needs an IPv4 /16../29 subnet with the first host as server_address")
    used = {server, network.network_address, network.broadcast_address}
    for name, address in clients.items():
        ip = ipaddress.ip_address(address)
        if name in ["ca", "enderpin-gateway"] or not re.fullmatch(r"[a-z][a-z0-9_-]{0,31}", name) or ip not in network or ip in used:
            raise AnsibleFilterError("VPN clients require safe names and unique usable addresses in the VPN subnet")
        used.add(ip)
    port = config["port"]
    if isinstance(port, bool) or not isinstance(port, int) or not 1 <= port <= 65535:
        raise AnsibleFilterError("invalid OpenVPN port")
    if not re.fullmatch(r"[A-Za-z0-9.-]+", config["endpoint"]):
        raise AnsibleFilterError("VPN endpoint must be an IPv4 address or hostname")
    return {"network": str(network.network_address), "netmask": str(network.netmask)}


class FilterModule:
    def filters(self):
        return {"enderpin_workspace": workspace, "enderpin_instances": instances, "enderpin_vpn": vpn,
                "enderpin_deployment": deployment, "enderpin_transition": transition,
                "enderpin_backends": backends, "enderpin_ipv4": lambda value: str(ipaddress.IPv4Address(value))}
