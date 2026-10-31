"""Native inputs stay user-owned; Velocity reserves only connection/authentication keys."""
from pathlib import Path
import re
import tomllib

from ansible.errors import AnsibleFilterError
import yaml


PROPERTIES = {"server-ip", "server-port", "online-mode", "enable-rcon", "enable-query"}
VELOCITY = {"bind", "online-mode", "player-info-forwarding-mode", "forwarding-secret-file", "servers", "forced-hosts"}
FABRIC = {"secret", "hackOnlineMode", "hackEarlySend", "hackMessageChain"}


class UniqueLoader(yaml.SafeLoader):
    pass


def unique_mapping(loader, node):
    result = {}
    for key, value in node.value:
        key = loader.construct_object(key)
        if key in result:
            raise ValueError("duplicate YAML key")
        result[key] = loader.construct_object(value)
    return result


UniqueLoader.add_constructor(yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, unique_mapping)


def native_settings(source, instance):
    if instance.get("integration", "none") == "none":
        return {}
    result = {"properties": "", "velocity": "", "velocity_keys": [], "fabric": "", "paper": {}, "spigot": {}}
    files = {"server.properties": "properties", "velocity.toml": "velocity",
             "config/FabricProxy-Lite.toml": "fabric", "config/paper-global.yml": "paper", "spigot.yml": "spigot"}
    for scope in source["inputs"]:
        for filename in [*files, "forwarding.secret"]:
            path = Path(source["path"]) / scope / filename
            if not path.exists():
                continue
            try:
                if filename == "forwarding.secret":
                    raise ValueError("reserved file")
                content = path.read_text()
                kind = files[filename]
                if kind == "properties":
                    # Reject nonliteral keys instead of guessing Java properties escape/continuation semantics.
                    continuation = False
                    for line in content.splitlines():
                        line = line.lstrip()
                        if not continuation and line.startswith(("#", "!")):
                            continue
                        if not continuation and line:
                            key = re.split(r"[\s=:]", line, maxsplit=1)[0]
                            if "\\" in key or key in PROPERTIES:
                                raise ValueError("reserved or escaped property key")
                        continuation = (len(line) - len(line.rstrip("\\"))) % 2 == 1
                    if continuation:
                        raise ValueError("unfinished property continuation")
                    value = content
                elif kind in {"velocity", "fabric"}:
                    parsed = tomllib.loads(content)
                    if parsed.keys() & (VELOCITY if kind == "velocity" else FABRIC):
                        raise ValueError("reserved TOML key")
                    value = content
                else:
                    value = yaml.load(content, Loader=UniqueLoader) or {}
                    if not isinstance(value, dict):
                        raise ValueError("expected YAML mapping")
                    parent, key = ("proxies", "velocity") if kind == "paper" else ("settings", "bungeecord")
                    if parent in value and (not isinstance(value[parent], dict) or key in value[parent]):
                        raise ValueError("reserved YAML key")
                if source["files"].get(f"{source['target']}/run/{filename}") == f"{scope}/{filename}":
                    result[kind] = value
                    if kind == "velocity":
                        result["velocity_keys"] = list(parsed)
            except (ValueError, TypeError, OSError, yaml.YAMLError):
                # Parser errors can contain the offending line, including credentials.
                raise AnsibleFilterError(f"Velocity integration conflicts with or cannot safely parse {scope}/{filename}; remove reserved keys (see deployment docs)") from None
    return result


class FilterModule:
    def filters(self):
        return {"enderpin_native": native_settings}
