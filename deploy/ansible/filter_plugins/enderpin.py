"""Validate deployment inputs before touching hosts or rendering secrets."""
import ipaddress
from pathlib import Path
import re
import tomllib

from ansible.errors import AnsibleFilterError


def workspace(source_root, relative, target):
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
    manifest = tomllib.loads((path / "enderpin.toml").read_text())
    lock = tomllib.loads((path / "enderpin.lock").read_text())
    if manifest.get("format") != 4 or lock.get("format") != 4:
        raise AnsibleFilterError("deployment requires format 4; migrate existing worlds explicitly first")
    entry = manifest.get("targets", {}).get(target)
    if entry is None:
        raise AnsibleFilterError(f"unknown deployment target: {target}")
    profile = manifest.get("profiles", {}).get(entry.get("profile"), {})
    if "profile" in entry and entry["profile"] not in manifest.get("profiles", {}):
        raise AnsibleFilterError("unknown target profile")
    config = {**profile, **entry}
    packages = {}
    for group in config.get("groups", []):
        if group not in manifest.get("groups", {}):
            raise AnsibleFilterError("unknown package group")
        for alias, package in manifest["groups"][group].get("packages", {}).items():
            if alias in packages and packages[alias] != package:
                raise AnsibleFilterError("conflicting package groups")
            packages[alias] = package
    packages.update(profile.get("packages", {}))
    packages.update(entry.get("packages", {}))
    runtime = config["loader"]
    if runtime not in ["paper", "fabric", "velocity"] or target not in lock.get("runtimes", {}) or config.get("type") not in ["server", "proxy"]:
        raise AnsibleFilterError("deployment requires a prepared Paper, Fabric or Velocity server lock")
    inputs = [f"{target}/files"]
    for group in config.get("file_groups", []):
        safe_name(group)
        inputs.append(f"shared/{group}/files")
    for relative_input in inputs:
        item = path / relative_input
        if any(parent.is_symlink() for parent in [item, *item.parents] if parent != root) or (item.is_dir() and any(p.is_symlink() for p in item.rglob("*"))):
            raise AnsibleFilterError("deployment inputs cannot contain symbolic links")
    if runtime == "fabric":
        disabled = config.get("disabled", [])
        aliases = [name for name, package in packages.items()
                   if package.get("modrinth") == "fabricproxy-lite" and package.get("kind", "mod") == "mod"
                   and name not in disabled and "fabricproxy-lite" not in disabled]
        if not aliases:
            raise AnsibleFilterError("Fabric backends require an enabled direct fabricproxy-lite Modrinth package")
        if not any(set(p.get("roots", [])) & set(aliases) and p.get("kind") == "mod"
                   and p.get("name") not in disabled and p.get("project") not in disabled
                   for p in lock.get("targets", {}).get(target, {}).get("packages", [])):
            raise AnsibleFilterError("FabricProxy-Lite must be present and enabled in the server package lock")
    if runtime == "paper":
        # shortcut: the managed forwarding template uses the Paper 1.19+ configuration layout.
        version = config.get("minecraft", manifest["minecraft"]).split(".")
        if len(version) < 2 or not version[0].isdigit() or not version[1].isdigit() or (int(version[0]), int(version[1])) < (1, 19):
            raise AnsibleFilterError("the Paper deployment template requires Minecraft 1.19 or newer")
    managed = ["server.properties", "velocity.toml", "forwarding.secret", "config/FabricProxy-Lite.toml", "config/paper-global.yml", "spigot.yml"]
    for scope in inputs:
        if any((path / scope / name).exists() for name in managed):
            raise AnsibleFilterError("Ansible owns server.properties and forwarding configs; remove duplicate files inputs")
    return {"path": str(path), "runtime": runtime, "target": target, "inputs": [item for item in inputs if (path / item).is_dir()]}


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
        memory = item["memory_mib"]
        if isinstance(memory, bool) or not isinstance(memory, int) or not 256 <= memory <= 1048576:
            raise AnsibleFilterError("memory_mib must be an integer of at least 256")
        for key, value in item.get("properties", {}).items():
            if not re.fullmatch(r"[a-z][a-z0-9-]*", key) or not isinstance(value, (str, int, float, bool)) or any(c in str(value) for c in "\r\n\\"):
                raise AnsibleFilterError("properties require plain keys and single-line values without backslashes")
        for flag in ["fabric_proxy_hack_early_send", "fabric_proxy_hack_message_chain"]:
            if flag in item and not isinstance(item[flag], bool):
                raise AnsibleFilterError(f"{flag} must be a boolean")
        maximum = item.get("show_max_players", 20)
        if isinstance(maximum, bool) or not isinstance(maximum, int) or maximum < 1:
            raise AnsibleFilterError("show_max_players must be a positive integer")
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


def deployment(items, source_root, interface, proxy_address):
    return [dict(name=item["name"], address=item["address"], port=item["port"],
                 workspace=item["workspace"], target=source["target"], runtime=source["runtime"],
                 interface=interface, proxy_address=proxy_address)
            for item in instances(items)
            for source in [workspace(source_root, item["workspace"], item["target"])]]


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
        for entry in previous:
            if (not isinstance(entry, dict) or set(entry) != {
                    "name", "address", "port", "workspace", "target", "runtime", "interface", "proxy_address"}
                    or entry["runtime"] not in ["paper", "fabric", "velocity"]
                    or not isinstance(entry["interface"], str)
                    or not re.fullmatch(r"[a-zA-Z0-9_-]{1,15}", entry["interface"])):
                raise AnsibleFilterError("invalid deployment record; preserve it and the existing firewall")
            instances([entry | {"memory_mib": 256}])
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
