"""Review non-secret inputs first; apply only the reviewed deployment manifest."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import sys
import tempfile

import yaml

# Keep inspection read-only even when invoked without Python's -B flag.
sys.dont_write_bytecode = True
# Also works when loaded by the deployment regression tests.
sys.path.insert(0, str(Path(__file__).resolve().parent))
import record


ROOT = Path(__file__).resolve().parent.parent


def manifest(root, inventory_path, revision, config_revision=None, config_sha256=None):
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("GITHUB_SHA must identify one commit")
    if record.run(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip() != revision:
        raise ValueError("checkout does not match GITHUB_SHA")
    if record.run(["git", "status", "--porcelain", "--untracked-files=normal"], cwd=root, text=True):
        raise ValueError("deployment requires a clean committed checkout")
    inventory = record.inventory(inventory_path)
    config = record.configuration_revision(root, inventory_path, revision, config_revision, config_sha256)
    config["effective_sha256"] = record.digest(record.encoded(inventory))
    spec = importlib.util.spec_from_file_location("enderpin_filters", root / "deploy/ansible/filter_plugins/enderpin.py")
    filters = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(filters)
    hosts = {}
    for host in record.members(inventory, "minecraft"):
        variables = inventory["_meta"]["hostvars"][host]
        identity = record.binary(variables["enderpin_binary"], variables["enderpin_binary_version"],
                                 variables["enderpin_binary_sha256"])
        targets = []
        for instance in filters.instances(variables["enderpin_instances"]):
            # The shared filter consumes the canonical Rust configuration interface.
            source = filters.workspace(root, instance["workspace"], instance.get("target", "server"))
            record.tracked_inputs(root, source)
            targets.append({"instance": instance["name"], "workspace": instance["workspace"],
                            "target": source["target"], "runtime": source["runtime"],
                            "file_inputs": source["inputs"]})
        hosts[host] = {"binary": identity, "targets": targets}
    templates = {str(path.relative_to(root)): record.digest(path.read_bytes())
                 for path in sorted((root / "deploy/ansible/templates").glob("*.j2"))}
    return {"schema_version": 1, "source_commit": revision, "configuration": config,
            "inventory": inventory, "hosts": hosts, "generated_configuration": {"templates": templates},
            "manage_vpn": os.environ.get("ENDERPIN_MANAGE_VPN", "false").lower() == "true",
            "secrets": record.secret_reference(os.environ.get("ENDERPIN_SECRET_REFERENCE"),
                                                os.environ.get("ENDERPIN_SECRET_REVISION"))}


def apply(root, inventory, revision, manage_vpn):
    ansible = Path(sys.executable).with_name("ansible-playbook")
    with tempfile.TemporaryDirectory(prefix="enderpin-cd-") as directory:
        temporary = Path(directory)
        for name, variable in [("vault.yml", "ENDERPIN_VAULT"), ("password", "ENDERPIN_VAULT_PASSWORD")]:
            value = os.environ[variable]
            if not value.strip():
                raise ValueError("deployment credentials cannot be empty")
            path = temporary / name
            path.touch(mode=0o600)
            path.write_text(value)
        plaintext = record.run([str(ansible.with_name("ansible-vault")), "view", str(temporary / "vault.yml"),
                                "--vault-password-file", str(temporary / "password")], text=True)
        try:
            secrets = yaml.safe_load(plaintext)
            if not isinstance(secrets, dict) or set(secrets) - record.SECRET_VARIABLES:
                raise ValueError("vault may contain only supported credentials, never deployment configuration")
            if any(not isinstance(value, str) for value in secrets.values()):
                raise ValueError("vault credentials must be strings")
        except yaml.YAMLError:
            raise ValueError("cannot parse deployment credentials") from None
        command = [str(ansible), "-i", str(inventory), "--vault-password-file", str(temporary / "password"),
                   "-e", "@" + str(temporary / "vault.yml"),
                   "-e", json.dumps({"enderpin_revision": revision, "enderpin_source_root": str(root)})]
        for playbook in (["vpn.yml"] if manage_vpn else []) + ["deploy.yml"]:
            record.run(command + [playbook], cwd=root / "deploy/ansible")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["plan", "apply"])
    parser.add_argument("--inventory", type=Path, default=ROOT / "deploy/ansible/inventory.yml")
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--previous", type=Path)
    parser.add_argument("--record", type=Path, help="write the successful manifest after apply")
    parser.add_argument("--config-revision")
    parser.add_argument("--config-sha256")
    args = parser.parse_args(argv)
    revision = os.environ["GITHUB_SHA"]
    previous = json.loads(args.previous.read_bytes()) if args.previous and args.previous.exists() else None
    proposed = manifest(ROOT, args.inventory.resolve(), revision, args.config_revision, args.config_sha256)
    proposed["previous_manifest_sha256"] = record.digest(record.encoded(previous))
    if args.action == "plan":
        record.write(args.manifest, record.encoded(proposed))
        record.write(args.manifest.with_suffix(".diff"), record.difference(previous, proposed).encode())
        print("Deployment manifest and non-secret diff are ready for review.")
    else:
        if not args.record:
            raise ValueError("apply requires --record for the successful deployment inputs")
        reviewed = json.loads(args.manifest.read_bytes())
        if reviewed != proposed:
            raise ValueError("deployment inputs changed after review; generate and review a new plan")
        apply(ROOT, args.inventory.resolve(), revision, proposed["manage_vpn"])
        record.write(args.record, record.encoded(proposed))
        print("Deployment applied; input revisions recorded.")


if __name__ == "__main__":
    try:
        main()
    except Exception:
        # Do not print exception chains: parsers and tools may embed credential values.
        sys.exit("Deployment failed; check the non-secret inputs and controller configuration.")
