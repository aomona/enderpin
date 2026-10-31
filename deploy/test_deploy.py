"""Run with the Python environment from deploy/ansible/requirements.txt."""
import contextlib
import io
import importlib.util
import hashlib
from functools import partial
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path
import tempfile
import tomllib
import unittest
from unittest.mock import patch

from ansible.errors import AnsibleFilterError
from jinja2 import Environment, StrictUndefined
from ansible.plugins.filter.core import FilterModule as CoreFilters
import yaml

BASE = Path(__file__).resolve().parent
BINARY = BASE.parent / "target/debug/enderpin"
spec = importlib.util.spec_from_file_location("enderpin_filters", BASE / "ansible/filter_plugins/enderpin.py")
filters = importlib.util.module_from_spec(spec)
spec.loader.exec_module(filters)
spec = importlib.util.spec_from_file_location("native_settings", BASE / "ansible/filter_plugins/native_settings.py")
native = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native)
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
        self.workspace = partial(filters.workspace, inspector=str(BINARY))
        self.deployment = partial(filters.deployment, inspector=str(BINARY))
        self.prepare_lock()
        self.instance = dict(name="backend", workspace="servers/backend", target="server", address="10.77.0.2", port=25566, integration="velocity")
        self.inventory = yaml.safe_load((BASE / "ansible/inventory.example.yml").read_text())["all"]["vars"]
        self.inventory["enderpin_inspector"] = str(BINARY)
        self.inventory["enderpin_inspector_version"] = "test-build"
        self.inventory["enderpin_inspector_sha256"] = hashlib.sha256(BINARY.read_bytes()).hexdigest()
        self.inventory["enderpin_binary_version"] = "test-build"
        self.inventory["enderpin_binary_sha256"] = hashlib.sha256(b"copied fixture").hexdigest()
        self.inventory["enderpin_forwarding_secret"] = "a" * 64

    def prepare_lock(self, target="server", packages=(), append=False):
        # Build valid fixture identities using the real Rust fingerprints; no downloads.
        previous = tomllib.loads((self.source / "enderpin.lock").read_text()) if append else {}
        (self.source / "enderpin.lock").unlink(missing_ok=True)
        result = subprocess.run([str(BINARY), "-C", str(self.source), "config", "show", "--resolved",
                                 "--target", target, "--json"], text=True, capture_output=True, check=True)
        config = json.loads(result.stdout)["targets"][target]
        runtime = config["runtime"]
        download = dict(url="https://example.org/runtime.jar", algorithm="sha256", hash="a" * 64,
                        filename="runtime.jar", size=1)
        identities = [dict(key="modrinth:proxy", name="fabricproxy-lite", kind="mod", project="proxy", version="v1",
                           url="https://example.org/proxy.jar", filename="proxy.jar", sha512="a" * 128, size=1,
                           default_sides=["server"], roots=list(packages))] if packages else []
        runtime_lock = dict(fingerprint=config["lock"]["runtime"]["expected_fingerprint"],
                            minecraft=runtime["minecraft"], loader=runtime["loader"], loader_version=runtime["loader_version"] or "1",
                            metadata=download, java=dict(release="21", major=21, platforms={"linux-x86_64": download}),
                            fabric_jvm=[], fabric_libraries=[download] if runtime["loader"] == "fabric" else [])
        if runtime["loader"] in ["paper", "velocity"]:
            runtime_lock["server_jar"] = download
        lock = dict(format=4, runtimes={target: runtime_lock}, targets={target: dict(
            fingerprint=config["lock"]["packages"]["expected_fingerprint"], minecraft=runtime["minecraft"],
            loader=runtime["loader"], requests={name: item["declaration"] for name, item in config["packages"].items()},
            packages=identities)})
        for scope in ["runtimes", "targets"]:
            lock[scope] = previous.get(scope, {}) | lock[scope]
        def toml_value(value):
            if isinstance(value, dict):
                return "{" + ", ".join(json.dumps(k) + "=" + toml_value(v) for k, v in value.items()) + "}"
            if isinstance(value, list):
                return "[" + ", ".join(map(toml_value, value)) + "]"
            return json.dumps(value)
        (self.source / "enderpin.lock").write_text("\n".join(k + "=" + toml_value(v) for k, v in lock.items()))

    def test_inspector_contract_and_failures_are_closed_and_redacted(self):
        for response in [subprocess.CompletedProcess([], 1, "CLI_SECRET", "CLI_SECRET"),
                         subprocess.CompletedProcess([], 0, '{"schema_version":2}', ""),
                         subprocess.CompletedProcess([], 0, 'invalid CLI_SECRET', "")]:
            with patch.object(filters.subprocess, "run", return_value=response):
                with self.assertRaises(AnsibleFilterError) as result:
                    self.workspace(self.root, "servers/backend", "server")
                self.assertNotIn("CLI_SECRET", str(result.exception))
        with self.assertRaises(AnsibleFilterError):
            filters.workspace(self.root, "servers/backend", "server", inspector="./enderpinw")
        with patch.object(filters.subprocess, "run", side_effect=FileNotFoundError("CLI_SECRET")):
            with self.assertRaises(AnsibleFilterError) as result:
                self.workspace(self.root, "servers/backend", "server")
            self.assertNotIn("CLI_SECRET", str(result.exception))
        (self.source / "enderpin.toml").write_text(self.manifest.replace("1.21.1", "1.21.2"))
        with self.assertRaisesRegex(AnsibleFilterError, "current package and runtime locks"):
            self.workspace(self.root, "servers/backend", "server")
        (self.source / "enderpin.lock").unlink()
        with self.assertRaisesRegex(AnsibleFilterError, "current package and runtime locks"):
            self.workspace(self.root, "servers/backend", "server")

    def test_workspace_boundaries_and_required_fabric_proxy(self):
        self.assertEqual(self.workspace(self.root, "servers/backend", "server", integration="velocity")["runtime"], "paper")
        for path in ["..", str(self.source)]:
            with self.assertRaises(AnsibleFilterError):
                self.workspace(self.root, path, "server")
        (self.root / "shortcut").symlink_to(self.source)
        with self.assertRaises(AnsibleFilterError):
            self.workspace(self.root, "shortcut", "server")
        inputs = self.source / "server/files"
        inputs.mkdir(parents=True)
        (inputs / "server.properties").write_text("online-mode=false")
        source = self.workspace(self.root, "servers/backend", "server", integration="velocity")
        with self.assertRaises(AnsibleFilterError):
            native.native_settings(source, self.instance)
        (inputs / "server.properties").unlink()
        manifest = self.manifest.replace('loader="paper"', 'loader="fabric"') + '[targets.server.packages.proxy]\nmodrinth="fabricproxy-lite"\n'
        (self.source / "enderpin.toml").write_text(manifest)
        with self.assertRaises(AnsibleFilterError):
            self.workspace(self.root, "servers/backend", "server", integration="velocity")
        self.prepare_lock()
        with self.assertRaisesRegex(AnsibleFilterError, "FabricProxy-Lite must be present"):
            self.workspace(self.root, "servers/backend", "server", integration="velocity")
        self.prepare_lock(packages=["proxy"])
        self.assertEqual(self.workspace(self.root, "servers/backend", "server", integration="velocity")["runtime"], "fabric")
        (self.source / "enderpin.toml").write_text(manifest.replace('[targets.server]\n', '[targets.server]\ndisabled=["proxy"]\n'))
        self.prepare_lock()
        with self.assertRaises(AnsibleFilterError):
            self.workspace(self.root, "servers/backend", "server", integration="velocity")

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
        self.prepare_lock("survival", packages=["proxy"])
        shared = self.source / "shared/backend/files"
        shared.mkdir(parents=True)
        result = self.workspace(self.source, ".", "survival")
        self.assertEqual(result["target"], "survival")
        self.assertEqual(result["inputs"], ["shared/backend/files"])
        with self.assertRaises(AnsibleFilterError):
            self.workspace(self.source, ".", "../survival")
        (shared / "server.properties").write_text("server-port=25565")
        with self.assertRaises(AnsibleFilterError):
            native.native_settings(self.workspace(self.source, ".", "survival"), self.instance)

    def test_inventory_requires_explicit_target(self):
        missing = {key: value for key, value in self.instance.items() if key != "target"}
        for instance in [missing] + [self.instance | dict(target=value) for value in [None, "", 17]]:
            with self.subTest(instance=instance), self.assertRaisesRegex(AnsibleFilterError, "instance backend requires.*target"):
                filters.instances([instance])

    def test_complete_named_target_example(self):
        manifest = (BASE / "example/enderpin.toml").read_text()
        self.assertNotIn("server", tomllib.loads(manifest)["targets"])
        inventory = yaml.safe_load((BASE / "ansible/inventory.example.yml").read_text())["all"]
        hosts = inventory["children"]["minecraft"]["hosts"]
        instances = [instance for host in hosts.values() for instance in host["enderpin_instances"]]
        filters.instances(instances)
        environment = Environment(undefined=StrictUndefined, keep_trailing_newline=True)
        environment.filters.update({key: CoreFilters().filters()[key] for key in ["combine", "to_nice_yaml"]})
        environment.filters.update(filters.FilterModule().filters())
        environment.filters["to_json"] = json.dumps
        templates = {name: environment.from_string((BASE / "ansible/templates" / name).read_text())
                     for name in ["firewall.nft.j2", "enderpin.service.j2"]}
        source_expression = yaml.safe_load((BASE / "ansible/stage.yml").read_text())[0]["ansible.builtin.set_fact"]["source"]
        resolve_source = environment.compile_expression(source_expression.removeprefix("{{").removesuffix("}}").strip())
        variables = self.inventory | dict(enderpin_source_root=str(self.source), enderpin_instances=instances,
                                         inventory_hostname="home", groups=dict(vpn_clients=["home"]))
        for other_runtime in [None, "paper", "velocity"]:
            with self.subTest(other_server_runtime=other_runtime):
                (self.source / "enderpin.toml").write_text(manifest)
                if other_runtime is not None:
                    target_type = "proxy" if other_runtime == "velocity" else "server"
                    extra_manifest = f'\n[targets.server]\ntype="{target_type}"\nloader="{other_runtime}"\n'
                    (self.source / "enderpin.toml").write_text(manifest + extra_manifest)
                for target in ["lobby", "survival", "proxy"]:
                    self.prepare_lock(target, packages=["fabricproxy-lite"] if target == "survival" else [], append=target != "lobby")
                variables["enderpin_protected"] = self.deployment(
                    instances, self.source, self.inventory["enderpin_backend_interface"],
                    self.inventory["enderpin_proxy_address"])
                firewall = templates["firewall.nft.j2"].render(**variables)
                for instance, runtime in zip(instances, ["paper", "fabric", "velocity"], strict=True):
                    source = resolve_source(**variables, instance=instance)
                    self.assertEqual(source["target"], instance["target"])
                    self.assertEqual(source["runtime"], runtime)
                    service = templates["enderpin.service.j2"].render(**variables, instance=instance, source=source)
                    self.assertIn(f"run --target {instance['target']} --no-interactive", service)
                    self.assertEqual(" --accept-eula" in service, runtime != "velocity")
                    if runtime == "velocity":
                        self.assertNotIn(f"tcp dport {instance['port']}", firewall)
                    else:
                        self.assertIn(f"ip saddr 10.77.0.1 tcp dport {instance['port']} accept", firewall)
                        self.assertIn(f"tcp dport {instance['port']} drop", firewall)
        unknown = instances[0] | dict(target="missing")
        with self.assertRaisesRegex(AnsibleFilterError, "unknown deployment target: missing"):
            self.deployment([unknown], self.source, "epvpn", "10.77.0.1")
        with self.assertRaisesRegex(AnsibleFilterError, "unknown deployment target: missing"):
            resolve_source(**variables, instance=unknown)

    def test_instance_and_vpn_validation(self):
        filters.instances([self.instance])
        for overrides in [dict(port=True), dict(name="../../etc"), dict(name="firewall"), dict(properties={"motd":"hi\nonline-mode=false"}),
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
        environment.filters.update({key: CoreFilters().filters()[key] for key in ["combine", "to_nice_yaml"]})
        environment.filters.update(filters.FilterModule().filters())
        environment.filters["to_json"] = json.dumps
        variables = self.inventory | dict(instance=self.instance,
                       source=dict(runtime="velocity", target="server", native=native.native_settings(dict(inputs=[]), self.instance)), enderpin_forwarding_secret="a" * 64,
                       enderpin_instances=[self.instance], enderpin_source_root=str(self.root),
                       inventory_hostname="home", groups=dict(vpn_clients=["home"]),
                       hostvars=dict(localhost=dict(vpn_network=dict(network="10.77.0.0", netmask="255.255.255.0"))))
        variables["enderpin_protected"] = self.deployment(
            [self.instance], self.root, self.inventory["enderpin_backend_interface"],
            self.inventory["enderpin_proxy_address"])
        def render(name):
            return environment.from_string((BASE / "ansible/templates" / name).read_text()).render(**variables)
        variables["source"]["native"]["velocity"] = 'motd = "サーバー 🐈"\n'
        variables["source"]["native"]["velocity_keys"] = ["motd"]
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
        self.assertNotIn("--memory", service)
        self.assertIn("KillMode=mixed", service)
        self.assertIn("Requires=openvpn-client@enderpin.service", service)
        self.assertIn("route-nopull", render("openvpn-client.conf.j2"))
        self.assertNotIn("ca.key", render("openvpn-server.conf.j2"))

    def test_firewall_transition_retains_old_inventory_and_requires_decommission(self):
        old = self.deployment([self.instance], self.root, "epvpn", "10.77.0.1")
        retained = old[0] | dict(name="retained", port=25567)
        previous = old + [retained]
        for desired in [[retained], [retained, old[0] | dict(name="renamed")],
                        [retained, old[0] | dict(port=25568)],
                        [retained, old[0] | dict(runtime="velocity")],
                        [retained, old[0] | dict(proxy_address="10.77.0.3")], []]:
            with self.subTest(desired=desired):
                with self.assertRaises(AnsibleFilterError):
                    filters.transition(desired, previous, [], {})
                retired = ["backend"] if desired else ["backend", "retained"]
                result = filters.transition(desired, previous, retired, {})
                self.assertEqual(result["decommission"], retired)
                self.assertTrue(all(entry in result["protected"] for entry in previous + desired))
        self.assertEqual(filters.transition(previous, previous, [], {})["protected"], previous)
        with self.assertRaises(AnsibleFilterError):
            filters.transition(old, None, [], {"enderpin-backend.service": {}})
        self.assertEqual(filters.transition(old, None, ["backend"], {
            "enderpin-backend.service": {}, "enderpin-firewall.service": {}})["decommission"], ["backend"])
        for invalid in ["backend", ["../bad"], ["firewall"], ["unknown"]]:
            with self.assertRaises(AnsibleFilterError):
                filters.transition(old, old, invalid, {})
        for invalid in [{}, [{}], [old[0] | dict(interface='bad"')]]:
            with self.assertRaises(AnsibleFilterError):
                filters.transition(old, invalid, [], {})

    def test_native_ownership_and_reserved_keys(self):
        inputs = self.source / "server/files"
        (inputs / "config").mkdir(parents=True)
        source = self.workspace(self.root, "servers/backend", "server")
        fixtures = {
            "server.properties": "motd=Hello\nview-distance=6\n",
            "velocity.toml": 'motd="Hello"\n[advanced]\ncompression-threshold=128\n',
            "config/FabricProxy-Lite.toml": "custom=true\n",
            "config/paper-global.yml": "chunks:\n  auto-save-interval: 100\n",
            "spigot.yml": "settings:\n  sample-count: 8\n",
        }
        for name, content in fixtures.items():
            (inputs / name).write_text(content)
        source = self.workspace(self.root, "servers/backend", "server")
        self.assertEqual(native.native_settings(source, {}), {})
        result = native.native_settings(source, self.instance)
        self.assertEqual(result["properties"], fixtures["server.properties"])
        self.assertEqual(result["velocity"], fixtures["velocity.toml"])
        self.assertEqual(result["spigot"]["settings"]["sample-count"], 8)
        conflicts = {
            "server.properties": ["online-mode=true", "server-port: 25565", "server-ip 127.0.0.1", r"online\u002dmode=false", r"online\-mode=false", "# comment\\\nonline-mode=true"],
            "velocity.toml": ['bind="0.0.0.0:25565"', '[servers]\nlobby="x"', '"online-mode"=true'],
            "config/FabricProxy-Lite.toml": ['secret="DO_NOT_LOG_THIS"', "hackOnlineMode=false"],
            "config/paper-global.yml": ['proxies:\n  velocity:\n    secret: DO_NOT_LOG_THIS', "proxies: {}\nproxies: {}", "[]", "false", "[DO_NOT_LOG_THIS]"],
            "spigot.yml": ["settings: {bungeecord: false}"],
            "forwarding.secret": ["DO_NOT_LOG_THIS"],
        }
        for name, values in conflicts.items():
            for content in values:
                with self.subTest(name=name, content=content):
                    (inputs / name).write_text(content)
                    before = {p: p.read_bytes() for p in inputs.rglob("*") if p.is_file()}
                    with self.assertRaises(AnsibleFilterError) as error:
                        native.native_settings(source, self.instance)
                    self.assertNotIn("DO_NOT_LOG_THIS", str(error.exception))
                    self.assertEqual(before, {p: p.read_bytes() for p in before})
            if name in fixtures:
                (inputs / name).write_text(fixtures[name])
            else:
                (inputs / name).unlink()
        shared = self.source / "shared/common/files"
        shared.mkdir(parents=True)
        (shared / "server.properties").write_text("online-mode=false")
        with self.assertRaises(AnsibleFilterError):
            native.native_settings(source | dict(inputs=["server/files", "shared/common/files"]), self.instance)

    def test_normal_mode_preserves_native_files_and_has_no_proxy_requirement(self):
        inputs = self.source / "server/files"
        inputs.mkdir(parents=True)
        properties = inputs / "server.properties"
        properties.write_bytes(b"# initialized\nonline-mode=true\nserver-port=25565\n")
        before = properties.read_bytes()
        (self.source / "enderpin.toml").write_text(self.manifest.replace('loader="paper"', 'loader="fabric"'))
        self.prepare_lock()
        source = self.workspace(self.root, "servers/backend", "server")
        self.assertEqual(native.native_settings(source, {}), {})
        self.assertEqual(properties.read_bytes(), before)
        filters.instances([self.instance | dict(integration="none")])
        for key in ["memory_mib", "properties", "motd", "show_max_players"]:
            with self.assertRaises(AnsibleFilterError):
                filters.instances([self.instance | {key: 1024}])
        environment = Environment(undefined=StrictUndefined)
        environment.filters.update(filters.FilterModule().filters())
        environment.filters["to_json"] = json.dumps
        firewall = environment.from_string((BASE / "ansible/templates/firewall.nft.j2").read_text()).render(
            enderpin_protected=self.deployment([self.instance | dict(integration="none")], self.root, "lo", "127.0.0.1"))
        self.assertNotIn("tcp dport", firewall)

    def test_native_validation_precedes_host_mutation_and_generation_is_explicit(self):
        play = yaml.safe_load((BASE / "ansible/deploy.yml").read_text())[0]
        host_block = next(task["block"] for task in play["tasks"] if "block" in task)
        names = [task["name"] for task in host_block]
        self.assertLess(names.index("Validate and stage every instance"), names.index("Protect both old and candidate backend ports"))
        block = yaml.safe_load((BASE / "ansible/stage.yml").read_text())
        names = [task["name"] for task in block]
        self.assertLess(names.index("Validate reserved native keys before staging"), names.index("Create isolated instance account"))
        for task in block:
            module = task.get("ansible.builtin.template", task.get("ansible.builtin.copy", {}))
            if "/files/" in module.get("dest", ""):
                self.assertIn("instance.integration", task["when"])
                self.assertIn("staged.path", module["dest"])

    def test_ansible_stages_native_settings_without_editing_checkout(self):
        inputs = self.source / "server/files"
        (inputs / "config").mkdir(parents=True)
        (inputs / "server.properties").write_text("motd=Keep me\nview-distance=7\n")
        (inputs / "velocity.toml").write_text('motd="Keep me"\n[advanced]\ncompression-threshold=128\n')
        (inputs / "config/paper-global.yml").write_text("chunks: {auto-save-interval: 100}\n")
        (inputs / "spigot.yml").write_text("settings: {sample-count: 8}\n")
        before = {p: p.read_bytes() for p in self.source.rglob("*") if p.is_file()}
        block = yaml.safe_load((BASE / "ansible/stage.yml").read_text())
        start = next(i for i, task in enumerate(block) if task["name"] == "Copy manifest and lock without worlds or credentials")
        end = next(i for i, task in enumerate(block) if task["name"] == "Stage the reviewed Enderpin binary")
        stage_tasks = block[start:end]
        for task in stage_tasks:
            for module, arguments in task.items():
                if module.startswith("ansible.builtin.") and isinstance(arguments, dict) and "group" in arguments:
                    arguments["group"] = str(os.getgid())
            if "ansible.builtin.template" in task:
                task["ansible.builtin.template"]["src"] = str(BASE / "ansible" / task["ansible.builtin.template"]["src"])
        for mode, runtime in [("none", "fabric"), ("velocity", "paper"), ("velocity", "fabric"), ("velocity", "velocity")]:
            with self.subTest(mode=mode, runtime=runtime):
                stage = self.root / (mode + "-" + runtime)
                stage.mkdir()
                instance = self.instance | dict(integration=mode)
                source = dict(path=str(self.source), inputs=["server/files"], target="server", runtime=runtime)
                source["files"] = {f"server/run/{p.relative_to(inputs)}": f"server/files/{p.relative_to(inputs)}" for p in inputs.rglob("*") if p.is_file()}
                source["native"] = native.native_settings(source, instance)
                variables = self.inventory | dict(instance=instance, source=source, staged=dict(path=str(stage)),
                    instance_user=str(os.getuid()), enderpin_forwarding_secret="a" * 64,
                    ansible_python_interpreter=sys.executable)
                playbook = self.root / "stage.yml"
                playbook.write_text(yaml.safe_dump([dict(hosts="localhost", gather_facts=False, vars=variables, tasks=stage_tasks)]))
                result = subprocess.run([str(Path(sys.executable).with_name("ansible-playbook")), "-i", "localhost,", "-c", "local", str(playbook)],
                                        cwd=BASE / "ansible", capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertNotIn("a" * 64, result.stdout + result.stderr)
                self.assertEqual(before, {p: p.read_bytes() for p in before})
                if mode == "none":
                    self.assertEqual((stage / "server/files/server.properties").read_bytes(), before[inputs / "server.properties"])
                    self.assertEqual((stage / "server/files/velocity.toml").read_bytes(), before[inputs / "velocity.toml"])
                    self.assertFalse((stage / "server/files/forwarding.secret").exists())
                elif runtime == "velocity":
                    proxy = tomllib.loads((stage / "server/files/velocity.toml").read_text())
                    self.assertEqual(proxy["motd"], "Keep me")
                    self.assertEqual(proxy["advanced"]["compression-threshold"], 128)
                    self.assertEqual(proxy["player-info-forwarding-mode"], "modern")
                    self.assertEqual((stage / "server/files/forwarding.secret").read_text(), "a" * 64)
                else:
                    properties = (stage / "server/files/server.properties").read_text()
                    self.assertIn("view-distance=7", properties)
                    self.assertIn("server-port=25566", properties)
                    if runtime == "paper":
                        paper = yaml.safe_load((stage / "server/files/config/paper-global.yml").read_text())
                        spigot = yaml.safe_load((stage / "server/files/spigot.yml").read_text())
                        self.assertEqual(paper["chunks"]["auto-save-interval"], 100)
                        self.assertEqual(paper["proxies"]["velocity"]["secret"], "a" * 64)
                        self.assertEqual(spigot["settings"], {"sample-count": 8, "bungeecord": False})
                    else:
                        fabric = tomllib.loads((stage / "server/files/config/FabricProxy-Lite.toml").read_text())
                        self.assertTrue(fabric["hackOnlineMode"])

    def test_native_preflight_failure_is_actionable_and_redacted(self):
        inputs = self.source / "server/files/config"
        inputs.mkdir(parents=True)
        (inputs / "FabricProxy-Lite.toml").write_text('secret="DO_NOT_LOG_THIS"\n')
        marker = self.root / "cutover"
        playbook = self.root / "preflight.yml"
        playbook.write_text(yaml.safe_dump([dict(hosts="localhost", gather_facts=False,
            vars=dict(source=dict(path=str(self.source), inputs=["server/files"]), instance=self.instance),
            tasks=[{"ansible.builtin.set_fact": {"validated_native": "{{ source | enderpin_native(instance) is mapping }}"}},
                   {"ansible.builtin.copy": {"content": "cutover", "dest": str(marker)}}])]))
        result = subprocess.run([str(Path(sys.executable).with_name("ansible-playbook")), "-i", "localhost,", "-c", "local", str(playbook)],
            cwd=BASE / "ansible", capture_output=True, text=True,
            env=os.environ | {"ANSIBLE_FILTER_PLUGINS": str(BASE / "ansible/filter_plugins")})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Velocity integration conflicts", result.stdout + result.stderr)
        self.assertNotIn("DO_NOT_LOG_THIS", result.stdout + result.stderr)
        self.assertFalse(marker.exists())

    def test_canonical_launch_sources_and_native_file_selection(self):
        manifest = '''format=4
minecraft="1.21.1"
[profiles.backend]
type="server"
loader="paper"
memory_mib=4096
jvm_args=["-XX:+UseZGC", "-Dexample.token=DO_NOT_LOG_THIS"]
file_groups=["common"]
[targets.survival]
profile="backend"
'''
        shared = self.source / "shared/common/files"
        target = self.source / "survival/files"
        shared.mkdir(parents=True)
        target.mkdir(parents=True)
        (shared / "server.properties").write_text("motd=shared\n")
        (target / "server.properties").write_text("motd=target\n")
        for override, expected, origin in [("", 4096, "profiles.backend"), ("memory_mib=6144\n", 6144, "targets.survival")]:
            with self.subTest(origin=origin):
                (self.source / "enderpin.toml").write_text(manifest + override)
                self.prepare_lock("survival")
                source = self.workspace(self.source, ".", "survival")
                self.assertEqual(source["launch"]["memory_mib"], expected)
                self.assertEqual(source["sources"]["memory_mib"], origin)
                self.assertEqual(source["launch"]["jvm_args"], ["-XX:+UseZGC", "-Dexample.token=[REDACTED]"])
                self.assertNotIn("DO_NOT_LOG_THIS", json.dumps(source))
                self.assertEqual(source["files"]["survival/run/server.properties"], "survival/files/server.properties")
                settings = native.native_settings(source, self.instance)
                self.assertEqual(settings["properties"], "motd=target\n")

    def test_legacy_record_and_explicit_integration_transition(self):
        old = self.deployment([self.instance], self.root, "epvpn", "10.77.0.1")
        legacy = [{key: value for key, value in old[0].items() if key != "integration"}]
        self.assertEqual(filters.transition(old, legacy, [], {})["decommission"], [])
        normal = self.instance | dict(integration="none")
        new = self.deployment([normal], self.root, "lo", "127.0.0.1")
        with self.assertRaisesRegex(AnsibleFilterError, "explicit enderpin_decommission"):
            filters.transition(new, legacy, [], {})
        inputs = self.source / "server/files"
        inputs.mkdir(parents=True)
        (inputs / "server.properties").write_text("online-mode=true\n")
        for key in ["enderpin_backend_interface", "enderpin_proxy_address", "enderpin_backends", "enderpin_try", "enderpin_forwarding_secret"]:
            self.inventory.pop(key)
        result, state = self.run_deployment_model([normal], legacy, ["backend"])
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("tcp dport 25566 drop", state["applied"][0])
        self.assertNotIn("tcp dport", state["firewall"])
        self.assertTrue(state["services"]["enderpin-backend"]["active"])
        self.assertEqual(json.loads(state["files"]["/etc/enderpin/instances.json"]), new)

    def test_host_native_conflict_preserves_running_services(self):
        previous = self.deployment([self.instance], self.root, "epvpn", "10.77.0.1")
        inputs = self.source / "server/files"
        inputs.mkdir(parents=True)
        (inputs / "server.properties").write_text("online-mode=false\nrcon.password=DO_NOT_LOG_THIS\n")
        result, state = self.run_deployment_model([self.instance], previous)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Velocity integration conflicts", result.stdout)
        self.assertNotIn("DO_NOT_LOG_THIS", result.stdout)
        self.assertEqual(state["firewall"], "old live rules")
        self.assertTrue(state["services"]["enderpin-backend"]["active"])
        self.assertFalse(any(event["module"] == "systemd_service" for event in state["events"]))

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

    def test_review_manifest_uses_real_canonical_settings_without_secret_values_or_hashes(self):
        root = self.root.resolve()
        inputs = self.source / "server/files"
        inputs.mkdir(parents=True)
        secret = "DO_NOT_PUBLISH_NATIVE_OR_VAULT_123456"
        content = f"password={secret}\n"
        (inputs / "private.properties").write_text(content)
        (self.source / "enderpin.toml").write_text(self.manifest + 'memory_mib=3072\njvm_args=["-Dcredential=' + secret + '"]\n')
        self.prepare_lock()
        shutil.copytree(BASE, root / "deploy", ignore=shutil.ignore_patterns("__pycache__"))
        variables = {key: value for key, value in self.inventory.items() if key != "enderpin_forwarding_secret"}
        variables.update(enderpin_binary=str(BINARY), enderpin_binary_sha256=hashlib.sha256(BINARY.read_bytes()).hexdigest())
        config = {"all": {"vars": variables, "children": {"minecraft": {"hosts": {
            "home": {"enderpin_instances": [self.instance | dict(integration="none")]}}}}}}
        inventory = root / "deploy/ansible/inventory.yml"
        inventory.write_text(yaml.safe_dump(config))
        record = cd.record
        record.run(["git", "init", "-q", str(root)])
        record.run(["git", "add", "."], cwd=root)
        record.run(["git", "-c", "user.name=Deployment Test", "-c", "user.email=test@example.invalid",
                    "-c", "commit.gpgsign=false", "commit", "-qm", "fixture"], cwd=root)
        revision = record.run(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
        with tempfile.TemporaryDirectory() as output_directory:
            output = Path(output_directory) / "manifest.json"
            environment = dict(GITHUB_SHA=revision, ENDERPIN_SECRET_REFERENCE="vault:production",
                               ENDERPIN_SECRET_REVISION="revision-1", ENDERPIN_VAULT=secret,
                               ENDERPIN_VAULT_PASSWORD=secret)
            captured = io.StringIO()
            with patch.dict(os.environ, environment), patch.object(cd, "ROOT", root), contextlib.redirect_stdout(captured):
                cd.main(["plan", "--inventory", str(inventory), "--manifest", str(output)])
            manifest = json.loads(output.read_bytes())
            target = manifest["hosts"]["home"]["targets"][0]
            self.assertEqual(target["launch"]["memory_mib"], 3072)
            self.assertEqual(target["sources"]["memory_mib"], "targets.server")
            self.assertEqual(target["inventory_overrides"], {})
            self.assertIn("server/files/private.properties", target["files"].values())
            self.assertEqual(manifest["source_commit"], revision)
            self.assertEqual(manifest["secrets"]["revision"], "revision-1")
            artifacts = output.read_text() + output.with_suffix(".diff").read_text() + captured.getvalue()
            for value in [secret, hashlib.sha256(secret.encode()).hexdigest(), hashlib.sha256(content.encode()).hexdigest()]:
                self.assertNotIn(value, artifacts)
            self.assertNotIn("fingerprint", artifacts)
            self.assertEqual(record.run(["git", "status", "--porcelain"], cwd=root), b"")
    def test_real_staged_binary_digest_is_checked_before_execution(self):
        marker = self.root / "executed"
        binary = self.root / "binary"
        binary.write_text("#!/bin/sh\ntouch '" + str(marker) + "'\n")
        stage = self.root / "stage"
        stage.mkdir()
        tasks = yaml.safe_load((BASE / "ansible/stage.yml").read_text())
        start = next(i for i, task in enumerate(tasks) if task["name"] == "Stage the reviewed Enderpin binary")
        selected = tasks[start:]
        for task in selected:
            task.pop("become_user", None)
            if "ansible.builtin.copy" in task:
                task["ansible.builtin.copy"]["group"] = str(os.getgid())
        variables = dict(enderpin_binary=str(binary), enderpin_binary_version="test-build",
                         staged=dict(path=str(stage)), instance_user=str(os.getuid()),
                         instance_root=str(self.root), source=dict(target="server"),
                         ansible_python_interpreter=sys.executable)
        for expected, success in [("0" * 64, False), (hashlib.sha256(binary.read_bytes()).hexdigest(), True)]:
            variables["enderpin_binary_sha256"] = expected
            playbook = self.root / "binary-check.yml"
            playbook.write_text(yaml.safe_dump([dict(hosts="localhost", gather_facts=False, vars=variables, tasks=selected)]))
            result = subprocess.run([str(Path(sys.executable).with_name("ansible-playbook")), "-i", "localhost,",
                                     "-c", "local", str(playbook)], capture_output=True, text=True)
            self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
            self.assertEqual(marker.exists(), success)

    def run_deployment_model(self, desired, previous, retired=(), failure=None, refuse_stop=None, missing_units=()):
        """Execute real includes/loops/asserts/rescue with isolated host-operation doubles."""
        scenario = self.root / "playbook"
        scenario.mkdir(exist_ok=True)
        shutil.copytree(BASE / "ansible/filter_plugins", scenario / "filter_plugins", dirs_exist_ok=True)
        shutil.copytree(BASE / "tests/action_plugins", scenario / "action_plugins", dirs_exist_ok=True)
        native = {"assert", "fail", "debug", "set_fact", "include_tasks"}
        def instrument(tasks):
            for task in tasks:
                task.pop("become_user", None)
                for key in ["block", "rescue", "always"]:
                    if key in task:
                        instrument(task[key])
                for key in list(task):
                    if key.startswith("ansible.builtin.") and key.split(".")[-1] not in native:
                        task["host_model"] = dict(module=key.split(".")[-1], args=task.pop(key))
        for name in ["stage.yml", "instance.yml", "firewall.yml", "decommission.yml"]:
            tasks = yaml.safe_load((BASE / "ansible" / name).read_text())
            instrument(tasks)
            (scenario / name).write_text(yaml.safe_dump(tasks, sort_keys=False))
        play = yaml.safe_load((BASE / "ansible/deploy.yml").read_text())[0]
        play.update(hosts="localhost", become=False, gather_facts=False)
        instrument(play["pre_tasks"])
        instrument(play["tasks"])
        play["pre_tasks"].insert(0, dict(host_model=dict(module="setup", args={})))
        play["vars"] = self.inventory | dict(
            enderpin_source_root=str(self.root), enderpin_instances=desired,
            enderpin_decommission=list(retired), enderpin_revision="a" * 40,
            enderpin_accept_eula=True,
            model_path=str(scenario / "state.json"), template_root=str(BASE / "ansible"))
        (scenario / "deploy.yml").write_text(yaml.safe_dump([play], sort_keys=False))
        world = "/srv/enderpin/backend/workspace/server/run/world/level.dat"
        state = dict(files={world: "world must survive"}, services={}, events=[], applied=[],
                     firewall="old live rules", failure=failure or {}, refuse_stop=refuse_stop)
        if previous is not None:
            state["files"]["/etc/enderpin/instances.json"] = json.dumps(previous)
        for entry in previous if previous is not None else [self.instance]:
            state["services"]["enderpin-" + entry["name"]] = dict(active=True, enabled=True)
        for name in missing_units:
            state["services"].pop(name)
        (scenario / "state.json").write_text(json.dumps(state))
        result = subprocess.run([str(Path(sys.executable).with_name("ansible-playbook")),
                                 "-i", "localhost,", "-c", "local", "deploy.yml"],
                                cwd=scenario, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        state = json.loads((scenario / "state.json").read_text())
        self.assertEqual(state["files"][world], "world must survive")
        self.assertFalse(any("/stage-test/" in path for path in state["files"]))
        return result, state

    def test_host_preflight_failures_preserve_services_and_firewall(self):
        other = self.instance | dict(name="other", port=25567)
        previous = self.deployment([self.instance, other], self.root, "epvpn", "10.77.0.1")
        scenarios = [([self.instance, other | dict(target="missing")], None),
                     ([self.instance, other], dict(task="Verify and cache all locked downloads before stopping the live server", instance="other"))]
        for desired, failure in scenarios:
            with self.subTest(failure=failure):
                result, state = self.run_deployment_model(desired, previous, failure=failure)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertEqual(state["firewall"], "old live rules")
                self.assertEqual(json.loads(state["files"]["/etc/enderpin/instances.json"]), previous)
                self.assertTrue(all(service["active"] and service["enabled"] for service in state["services"].values()))
                self.assertFalse(any(event["module"] == "systemd_service" for event in state["events"]))
                if failure:
                    self.assertIn("injected host failure", result.stdout)

    def test_host_decommission_and_cutover_fail_closed(self):
        retained = self.instance | dict(name="retained", port=25567)
        previous = self.deployment([self.instance, retained], self.root, "epvpn", "10.77.0.1")
        for replacement in [None, self.instance | dict(name="renamed"), self.instance | dict(port=25568)]:
            desired = [retained] + ([replacement] if replacement else [])
            with self.subTest(replacement=replacement):
                result, state = self.run_deployment_model(desired, previous)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertEqual(state["firewall"], "old live rules")
                self.assertTrue(state["services"]["enderpin-backend"]["active"])
                result, state = self.run_deployment_model(desired, previous, ["backend"])
                self.assertEqual(result.returncode, 0, result.stdout)
                self.assertIn("worlds, backups and account preserved", result.stdout)
                self.assertIn("tcp dport 25566 drop", state["applied"][0])
                self.assertIn("tcp dport 25567 drop", state["firewall"])
                for entry in desired:
                    self.assertIn(f"tcp dport {entry['port']} drop", state["applied"][0])
                self.assertEqual(json.loads(state["files"]["/etc/enderpin/instances.json"]),
                                 self.deployment(desired, self.root, "epvpn", "10.77.0.1"))
                if replacement is None or replacement["port"] != 25566:
                    self.assertNotIn("tcp dport 25566", state["firewall"])
                if replacement is None or replacement["name"] != "backend":
                    self.assertEqual(state["services"]["enderpin-backend"], dict(active=False, enabled=False))
                self.assertTrue(state["services"]["enderpin-retained"]["active"])
                events = state["events"]
                last_download = max(i for i, event in enumerate(events) if event["task"].startswith("Verify and cache"))
                first_service_change = next(i for i, event in enumerate(events) if event["module"] == "systemd_service")
                self.assertLess(last_download, first_service_change)
                self.assertTrue(all(event["args"]["state"] == "reloaded" for event in events
                                    if event["module"] == "systemd_service" and event["args"]["name"] == "enderpin-firewall"))
        result, state = self.run_deployment_model([retained], previous, ["backend"], refuse_stop="enderpin-backend")
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertEqual(state["firewall"], "old live rules")
        result, state = self.run_deployment_model([retained], previous, ["backend"], failure=dict(task="Verify that recorded old TCP listeners have closed"))
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertEqual(state["firewall"], "old live rules")
        result, state = self.run_deployment_model([retained], previous, ["backend"], failure=dict(task="Wait for the instance TCP listener"))
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn("tcp dport 25566 drop", state["firewall"])
        self.assertIn("tcp dport 25567 drop", state["firewall"])
        self.assertEqual(json.loads(state["files"]["/etc/enderpin/instances.json"]), previous)
        self.assertFalse(state["services"]["enderpin-retained"]["active"])
        changed = self.instance | dict(port=25568)
        result, state = self.run_deployment_model([retained, changed], previous, ["backend"],
                                                  failure=dict(task="Wait for the instance TCP listener", instance="backend"))
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn("tcp dport 25566 drop", state["firewall"])
        self.assertIn("tcp dport 25568 drop", state["firewall"])
        self.assertEqual({entry["port"] for entry in json.loads(state["files"]["/etc/enderpin/instances.json"])},
                         {25566, 25567, 25568})
        result, state = self.run_deployment_model([], previous, ["backend", "retained"])
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertNotIn("tcp dport", state["firewall"])
        result, state = self.run_deployment_model([retained], previous, ["backend"], missing_units=["enderpin-backend"])
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertNotIn("tcp dport 25566", state["firewall"])

    def test_legacy_adoption_requires_explicit_stop(self):
        result, state = self.run_deployment_model([self.instance], None)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertEqual(state["firewall"], "old live rules")
        result, state = self.run_deployment_model([self.instance], None, ["backend"])
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("tcp dport 25566 drop", state["firewall"])



if __name__ == "__main__":
    unittest.main()
