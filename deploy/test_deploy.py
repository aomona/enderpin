"""Run with the Python environment from deploy/ansible/requirements.txt."""
import importlib.util
import json
import os
import re
import subprocess
import sys
from pathlib import Path
import tempfile
import tomllib
import unittest
from unittest.mock import patch

from ansible.errors import AnsibleFilterError
from jinja2 import Environment, StrictUndefined
import yaml

BASE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("enderpin_filters", BASE / "ansible/filter_plugins/enderpin.py")
filters = importlib.util.module_from_spec(spec)
spec.loader.exec_module(filters)
spec = importlib.util.spec_from_file_location("enderpin_cd", BASE / "cd.py")
cd = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cd)


class DeploymentTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "servers/backend"
        self.source.mkdir(parents=True)
        self.manifest = 'format=4\nminecraft="1.21.1"\n[targets.server]\ntype="server"\nloader="paper"\n'
        (self.source / "enderpin.toml").write_text(self.manifest)
        (self.source / "enderpin.lock").write_text('format=4\n[runtimes.server]\nloader="paper"\n')
        self.instance = dict(name="backend", workspace="servers/backend", address="10.77.0.2", port=25566, memory_mib=1024)
        self.inventory = yaml.safe_load((BASE / "ansible/inventory.example.yml").read_text())["all"]["vars"]

    def test_workspace_boundaries_and_required_fabric_proxy(self):
        self.assertEqual(filters.workspace(self.root, "servers/backend")["runtime"], "paper")
        for path in ["..", str(self.source)]:
            with self.assertRaises(AnsibleFilterError):
                filters.workspace(self.root, path)
        (self.root / "shortcut").symlink_to(self.source)
        with self.assertRaises(AnsibleFilterError):
            filters.workspace(self.root, "shortcut")
        inputs = self.source / "server/files"
        inputs.mkdir(parents=True)
        (inputs / "server.properties").write_text("online-mode=false")
        with self.assertRaises(AnsibleFilterError):
            filters.workspace(self.root, "servers/backend")
        (inputs / "server.properties").unlink()
        manifest = self.manifest.replace('loader="paper"', 'loader="fabric"') + '[targets.server.packages.proxy]\nmodrinth="fabricproxy-lite"\n'
        (self.source / "enderpin.toml").write_text(manifest)
        with self.assertRaises(AnsibleFilterError):
            filters.workspace(self.root, "servers/backend")
        (self.source / "enderpin.lock").write_text('format=4\n[runtimes.server]\nloader="fabric"\n[[targets.server.packages]]\nname="fabricproxy-lite"\nkind="mod"\nroots=["proxy"]\n')
        self.assertEqual(filters.workspace(self.root, "servers/backend")["runtime"], "fabric")
        (self.source / "enderpin.toml").write_text(manifest.replace('[targets.server]\n', '[targets.server]\ndisabled=["proxy"]\n'))
        with self.assertRaises(AnsibleFilterError):
            filters.workspace(self.root, "servers/backend")

    def test_root_manifest_named_target_profiles_and_groups(self):
        manifest = '''format=4
minecraft="26.3"
[groups.backend.packages.proxy]
modrinth="fabricproxy-lite"
[profiles.backend]
type="server"
loader="fabric"
groups=["backend"]
file_groups=["backend"]
[targets.survival]
profile="backend"
'''
        (self.source / "enderpin.toml").write_text(manifest)
        (self.source / "enderpin.lock").write_text('''format=4
[runtimes.survival]
loader="fabric"
[[targets.survival.packages]]
name="fabricproxy-lite"
kind="mod"
roots=["proxy"]
''')
        shared = self.source / "shared/backend/files"
        shared.mkdir(parents=True)
        result = filters.workspace(self.source, ".", "survival")
        self.assertEqual(result["target"], "survival")
        self.assertEqual(result["inputs"], ["shared/backend/files"])
        with self.assertRaises(AnsibleFilterError):
            filters.workspace(self.source, ".", "../survival")
        (shared / "server.properties").write_text("server-port=25565")
        with self.assertRaises(AnsibleFilterError):
            filters.workspace(self.source, ".", "survival")

    def test_instance_and_vpn_validation(self):
        filters.instances([self.instance])
        for overrides in [dict(port=True), dict(name="../../etc"), dict(properties={"motd":"hi\nonline-mode=false"}),
                          dict(properties={"motd":"hi\\"}), dict(fabric_proxy_hack_early_send="true"), dict(memory_mib=-1)]:
            with self.assertRaises(AnsibleFilterError):
                filters.instances([self.instance | overrides])
        with self.assertRaises(AnsibleFilterError):
            filters.instances([self.instance, self.instance | dict(name="other")])
        network = filters.vpn(self.inventory["enderpin_vpn"], self.inventory["enderpin_vpn_clients"])
        self.assertEqual(network["netmask"], "255.255.255.0")
        for clients in [{"home":"10.77.0.1"}, {"ca":"10.77.0.2"}, {"home":"10.77.0.2","other":"10.77.0.2"}]:
            with self.assertRaises(AnsibleFilterError):
                filters.vpn(self.inventory["enderpin_vpn"], clients)
        filters.backends(self.inventory["enderpin_backends"])
        with self.assertRaises(AnsibleFilterError):
            filters.backends({"try":"10.77.0.2:25566"})

    def test_rendered_configuration_and_secret_consistency(self):
        environment = Environment(undefined=StrictUndefined, keep_trailing_newline=True)
        environment.filters.update(filters.FilterModule().filters())
        environment.filters["to_json"] = json.dumps
        variables = self.inventory | dict(instance=self.instance | dict(motd="サーバー 🐈"),
                       source=dict(runtime="velocity"), enderpin_forwarding_secret="a" * 64,
                       enderpin_instances=[self.instance], enderpin_source_root=str(self.root),
                       inventory_hostname="home", groups=dict(vpn_clients=["home"]),
                       hostvars=dict(localhost=dict(vpn_network=dict(network="10.77.0.0", netmask="255.255.255.0"))))
        def render(name):
            return environment.from_string((BASE / "ansible/templates" / name).read_text()).render(**variables)
        proxy = tomllib.loads(render("velocity.toml.j2"))
        self.assertEqual(proxy["player-info-forwarding-mode"], "modern")
        self.assertEqual(proxy["forced-hosts"], {})
        self.assertEqual(proxy["motd"], "サーバー 🐈")
        fabric = tomllib.loads(render("FabricProxy-Lite.toml.j2"))
        paper = yaml.safe_load(render("paper-global.yml.j2"))
        self.assertEqual(fabric["secret"], paper["proxies"]["velocity"]["secret"])
        self.assertTrue(fabric["hackOnlineMode"])
        variables["source"]["runtime"] = "fabric"
        properties = render("server.properties.j2")
        self.assertIn("online-mode=true", properties)
        variables["source"]["runtime"] = "paper"
        self.assertIn("online-mode=false", render("server.properties.j2"))
        firewall = render("firewall.nft.j2")
        self.assertIn('ip saddr 10.77.0.1 tcp dport 25566 accept', firewall)
        self.assertIn('tcp dport 25566 drop', firewall)
        self.assertNotIn('flush ruleset', firewall)
        service = render("enderpin.service.j2")
        self.assertIn("--service --offline", service)
        self.assertIn("KillMode=mixed", service)
        self.assertIn("Requires=openvpn-client@enderpin.service", service)
        self.assertIn("route-nopull", render("openvpn-client.conf.j2"))
        self.assertNotIn("ca.key", render("openvpn-server.conf.j2"))

    def test_vpn_pki_is_persistent(self):
        inventory = yaml.safe_load((BASE / "ansible/inventory.example.yml").read_text())
        inventory["all"]["vars"]["enderpin_pki_directory"] = str(self.root / "pki")
        (self.root / "inventory.yml").write_text(yaml.safe_dump(inventory))
        command = [str(Path(sys.executable).with_name("ansible-playbook")), "-i", str(self.root / "inventory.yml"),
                   "vpn.yml", "--limit", "localhost"]
        identities = None
        for attempt in range(2):
            result = subprocess.run(command, cwd=BASE / "ansible", text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            self.assertEqual(result.returncode, 0, result.stdout)
            current = {p.name: p.read_bytes() for p in (self.root / "pki").iterdir() if p.suffix in [".key", ".crt"]}
            if attempt:
                self.assertEqual(identities, current)
                self.assertIn("changed=0", result.stdout)
            identities = current
        key = (self.root / "pki/tls-crypt.key").read_text().splitlines()
        self.assertEqual(len(key), 18)
        self.assertTrue(all(re.fullmatch(r"[a-f0-9]{32}", line) for line in key[1:-1]))

    def test_cd_commit_credentials_and_cleanup(self):
        environment = dict(GITHUB_SHA="a" * 40, ENDERPIN_INVENTORY="all: {}", ENDERPIN_VAULT="encrypted",
                           ENDERPIN_VAULT_PASSWORD="secret", ENDERPIN_MANAGE_VPN="true")
        calls = []
        def run(arguments, **kwargs):
            directory = Path(arguments[2]).parent
            self.assertEqual((directory / "password").stat().st_mode & 0o777, 0o600)
            self.assertNotIn("ENDERPIN_VAULT_PASSWORD", kwargs["env"])
            self.assertEqual(json.loads(arguments[-2])["enderpin_revision"], "a" * 40)
            calls.append((arguments[-1], directory))
        with patch.dict(os.environ, environment, clear=True), patch.object(cd.subprocess, "run", side_effect=run):
            cd.main()
        self.assertEqual([call[0] for call in calls], ["vpn.yml", "deploy.yml"])
        self.assertFalse(calls[0][1].exists())
        with patch.dict(os.environ, environment | dict(GITHUB_SHA="branch"), clear=True):
            with self.assertRaises(ValueError):
                cd.main()
        with patch.dict(os.environ, environment, clear=True), patch.object(cd.subprocess, "run", side_effect=RuntimeError("failed")):
            with self.assertRaises(RuntimeError):
                cd.main()


if __name__ == "__main__":
    unittest.main()
