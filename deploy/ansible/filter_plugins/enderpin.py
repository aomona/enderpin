"""Validate deployment inputs before touching hosts or rendering secrets."""
import ipaddress
from pathlib import Path
import re
import tomllib

from ansible.errors import AnsibleFilterError


def workspace(source_root, relative):
    root = Path(source_root).resolve(strict=True)
    path = root / relative
    if Path(relative).is_absolute() or ".." in Path(relative).parts:
        raise AnsibleFilterError("workspace must be relative to the source repository")
    if not path.is_dir() or path.resolve() == root or not path.resolve().is_relative_to(root):
        raise AnsibleFilterError("workspace must be a directory inside the source repository")
    for parent in [path, *path.parents]:
        if parent == root:
            break
        if parent.is_symlink():
            raise AnsibleFilterError("workspace cannot contain symbolic links")
    for name in ["enderpin.toml", "enderpin.lock", "files"]:
        item = path / name
        if item.is_symlink() or (item.is_dir() and any(p.is_symlink() for p in item.rglob("*"))):
            raise AnsibleFilterError("deployment inputs cannot contain symbolic links")
    manifest = tomllib.loads((path / "enderpin.toml").read_text())
    lock = tomllib.loads((path / "enderpin.lock").read_text())
    runtime = manifest["server"]["loader"]
    if runtime not in ["paper", "fabric", "velocity"] or "server" not in lock.get("runtimes", {}):
        raise AnsibleFilterError("deployment requires a prepared Paper, Fabric or Velocity server lock")
    if runtime == "fabric":
        packages = manifest["server"].get("packages", {})
        disabled = manifest["server"].get("disabled", [])
        aliases = [name for name, package in packages.items()
                   if package.get("modrinth") == "fabricproxy-lite" and package.get("kind", "mod") == "mod"
                   and name not in disabled and "fabricproxy-lite" not in disabled]
        if not aliases:
            raise AnsibleFilterError("Fabric backends require an enabled direct fabricproxy-lite Modrinth package")
        if not any(set(p.get("roots", [])) & set(aliases) and p.get("kind") == "mod"
                   and p.get("name") not in disabled and p.get("project") not in disabled
                   for p in lock.get("targets", {}).get("server", {}).get("packages", [])):
            raise AnsibleFilterError("FabricProxy-Lite must be present and enabled in the server package lock")
    if runtime == "paper":
        # shortcut: the managed forwarding template uses the Paper 1.19+ configuration layout.
        version = manifest["minecraft"].split(".")
        if len(version) < 2 or not version[0].isdigit() or not version[1].isdigit() or (int(version[0]), int(version[1])) < (1, 19):
            raise AnsibleFilterError("the Paper deployment template requires Minecraft 1.19 or newer")
    managed = ["server.properties", "velocity.toml", "forwarding.secret", "config/FabricProxy-Lite.toml", "config/paper-global.yml", "spigot.yml"]
    for scope in ["common", "server"]:
        if any((path / "files" / scope / name).exists() for name in managed):
            raise AnsibleFilterError("Ansible owns server.properties and forwarding configs; remove duplicate files inputs")
    return {"path": str(path), "runtime": runtime, "files": (path / "files").is_dir()}


def instances(items):
    names, ports = set(), set()
    for item in items:
        name = item["name"]
        if not re.fullmatch(r"[a-z][a-z0-9_]{0,23}", name) or name in names:
            raise AnsibleFilterError("instance names must be unique lower-case service identifiers")
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
                "enderpin_backends": backends, "enderpin_ipv4": lambda value: str(ipaddress.IPv4Address(value))}
