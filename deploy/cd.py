"""Deploy one checked-out commit using a pre-provisioned, trusted controller."""
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile


def main():
    revision = os.environ["GITHUB_SHA"]
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("GITHUB_SHA must identify one commit")
    root = Path(__file__).resolve().parent.parent
    ansible = Path(sys.executable).with_name("ansible-playbook")
    # Keep credentials outside the checkout and remove them even when Ansible fails.
    with tempfile.TemporaryDirectory(prefix="enderpin-cd-") as directory:
        temporary = Path(directory)
        for name, variable in [("inventory.yml", "ENDERPIN_INVENTORY"),
                               ("vault.yml", "ENDERPIN_VAULT"),
                               ("password", "ENDERPIN_VAULT_PASSWORD")]:
            value = os.environ[variable]
            if not value.strip():
                raise ValueError(f"{variable} cannot be empty")
            path = temporary / name
            path.touch(mode=0o600)
            path.write_text(value)
        command = [str(ansible), "-i", str(temporary / "inventory.yml"),
                   "--vault-password-file", str(temporary / "password"),
                   "-e", "@" + str(temporary / "vault.yml"),
                   "-e", json.dumps({"enderpin_revision": revision, "enderpin_source_root": str(root)})]
        environment = {key: value for key, value in os.environ.items()
                       if key not in ["ENDERPIN_INVENTORY", "ENDERPIN_VAULT", "ENDERPIN_VAULT_PASSWORD"]}
        if os.environ.get("ENDERPIN_MANAGE_VPN", "false").lower() == "true":
            subprocess.run(command + ["vpn.yml"], cwd=root / "deploy/ansible", env=environment, check=True)
        subprocess.run(command + ["deploy.yml"], cwd=root / "deploy/ansible", env=environment, check=True)


if __name__ == "__main__":
    main()
