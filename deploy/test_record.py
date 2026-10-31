"""Deployment review boundary regression tests; no remote hosts are contacted."""
import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import yaml
from ansible.parsing.vault import VaultLib, VaultSecret

import cd
import record


class RecordTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.inventory = self.root / "inventory.yml"
        self.config = {"all": {"children": {"minecraft": {"hosts": {"home": {
            "ansible_host": "127.0.0.1", "enderpin_instances": []}}}}}}
        self.inventory.write_text(yaml.safe_dump(self.config))
        self.secret = "credential-do-not-publish-123456789"

    def test_static_inventory_and_secret_rejection_before_hashing(self):
        resolved = record.inventory(self.inventory)
        self.assertEqual(record.members(resolved, "minecraft"), ["home"])
        for key in ["ansible_password", "enderpin_forwarding_secret", "rcon.password", "api_token"]:
            self.config["all"]["vars"] = {key: self.secret}
            self.inventory.write_text(yaml.safe_dump(self.config))
            with patch.object(record, "digest") as digest, patch.object(record, "run") as run:
                with self.assertRaises(ValueError) as error:
                    record.inventory(self.inventory)
                self.assertNotIn(self.secret, str(error.exception))
                digest.assert_not_called()
                run.assert_not_called()

    def test_no_inventory_lookup_or_unknown_variables(self):
        self.config["all"]["vars"] = {"enderpin_root": "{{ lookup('env', 'PASSWORD') }}"}
        self.inventory.write_text(yaml.safe_dump(self.config))
        with self.assertRaises(ValueError):
            record.inventory(self.inventory)
        self.config["all"]["vars"] = {"unreviewed_operation": "do something"}
        self.inventory.write_text(yaml.safe_dump(self.config))
        with self.assertRaises(ValueError):
            record.inventory(self.inventory)
        self.config["all"]["vars"] = {"ansible_python_interpreter": "{{ lookup('env', 'PASSWORD') }}"}
        self.inventory.write_text(yaml.safe_dump(self.config))
        with self.assertRaises(ValueError):
            record.inventory(self.inventory)

    def test_frozen_inventory_preserves_nested_groups_without_sidecar_inputs(self):
        self.config["all"]["vars"] = {"enderpin_root": "/srv/enderpin"}
        self.config["all"]["children"]["minecraft"] = {"children": {"backends": {"hosts": {"home": {"ansible_host": "127.0.0.1"}}}}}
        self.inventory.write_text(yaml.safe_dump(self.config))
        before = record.inventory(self.inventory)
        self.inventory.write_text(yaml.safe_dump(record.frozen_inventory(before)))
        after = record.inventory(self.inventory)
        self.assertEqual(before["_meta"], after["_meta"])
        for group in ["all", "minecraft", "backends"]:
            self.assertEqual(record.members(before, group), record.members(after, group))
        (self.root / "host_vars").mkdir()
        with self.assertRaises(ValueError):
            record.inventory(self.inventory)

    def test_binary_and_external_config_changes_without_source_commit_change(self):
        binary = self.root / "enderpin"
        binary.write_bytes(b"reviewed executable")
        expected = record.digest(binary.read_bytes())
        identity = record.binary(binary, "v1", expected)
        self.assertEqual(identity["sha256"], expected)
        binary.write_bytes(b"different executable")
        with self.assertRaises(ValueError):
            record.binary(binary, "v1", expected)
        changed = record.binary(binary, "v1", record.digest(binary.read_bytes()))
        self.assertIn(changed["sha256"], record.difference(identity, changed))
        source = self.root / "checkout"
        source.mkdir()
        expected = record.digest(self.inventory.read_bytes())
        record.configuration_revision(source, self.inventory, "a" * 40, "config-commit-1", expected)
        self.inventory.write_text(self.inventory.read_text() + "# new configuration\n")
        with self.assertRaises(ValueError):
            record.configuration_revision(source, self.inventory, "a" * 40, "config-commit-1", expected)

    def test_failure_output_is_never_logged_and_environment_is_scrubbed(self):
        def fail(command, **kwargs):
            self.assertNotIn("ENDERPIN_VAULT", kwargs["env"])
            self.assertNotIn("ENDERPIN_VAULT_PASSWORD", kwargs["env"])
            return subprocess.CompletedProcess(command, 1, self.secret, self.secret)
        output = io.StringIO()
        with patch.dict(os.environ, ENDERPIN_VAULT=self.secret, ENDERPIN_VAULT_PASSWORD=self.secret), \
             patch.object(record.subprocess, "run", side_effect=fail), contextlib.redirect_stdout(output), \
             contextlib.redirect_stderr(output):
            with self.assertRaises(ValueError) as error:
                record.run(["failing-tool"])
        self.assertNotIn(self.secret, str(error.exception) + output.getvalue())

    def test_vault_can_rotate_but_cannot_override_reviewed_configuration(self):
        calls = []
        def execute(command, **kwargs):
            calls.append(command)
            password = Path(command[command.index("--vault-password-file") + 1])
            self.assertEqual(password.stat().st_mode & 0o777, 0o600)
            if "view" in command:
                return yaml.safe_dump({"enderpin_forwarding_secret": self.secret})
            return b""
        with patch.dict(os.environ, ENDERPIN_VAULT="encrypted", ENDERPIN_VAULT_PASSWORD=self.secret), \
             patch.object(record, "run", side_effect=execute):
            cd.apply(self.root, {"_meta": {"hostvars": {}}}, "a" * 40, True)
        self.assertEqual([command[-1] for command in calls[1:]], ["vpn.yml", "deploy.yml"])
        self.assertFalse(Path(calls[-1][calls[-1].index("--vault-password-file") + 1]).exists())
        with patch.dict(os.environ, ENDERPIN_VAULT="encrypted", ENDERPIN_VAULT_PASSWORD=self.secret), \
             patch.object(record, "run", return_value=yaml.safe_dump({"enderpin_root": self.secret})) as run:
            with self.assertRaises(ValueError) as error:
                cd.apply(self.root, {"_meta": {"hostvars": {}}}, "a" * 40, False)
            self.assertNotIn(self.secret, str(error.exception))
            self.assertEqual(run.call_count, 1)
        before = record.secret_reference("vault:production/forwarding", "revision-1")
        after = record.secret_reference("vault:production/forwarding", "revision-2")
        diff = record.difference(before, after)
        self.assertIn("revision-2", diff)
        self.assertNotIn(self.secret, diff)
        self.assertNotIn(record.digest(self.secret.encode()), diff)

    def test_real_vault_and_failing_ansible_cannot_publish_secret_output(self):
        playbooks = self.root / "deploy/ansible"
        playbooks.mkdir(parents=True)
        (playbooks / "deploy.yml").write_text(yaml.safe_dump([{
            "hosts": "localhost", "gather_facts": False, "tasks": [
                {"ansible.builtin.debug": {"var": "enderpin_forwarding_secret"}},
                {"ansible.builtin.fail": {"msg": "{{ enderpin_forwarding_secret }}"}},
            ]}]))
        password = "vault-password-do-not-publish"
        vault = VaultLib([("default", VaultSecret(password.encode()))])
        encrypted = vault.encrypt(yaml.safe_dump({"enderpin_forwarding_secret": self.secret}).encode()).decode()
        inventory = {"_meta": {"hostvars": {"localhost": {"ansible_connection": "local"}}}}
        output = io.StringIO()
        with patch.dict(os.environ, ENDERPIN_VAULT=encrypted, ENDERPIN_VAULT_PASSWORD=password), \
             contextlib.redirect_stdout(output), contextlib.redirect_stderr(output), patch.object(record, "digest") as digest:
            with self.assertRaises(ValueError) as error:
                cd.apply(self.root, inventory, "a" * 40, False)
            digest.assert_not_called()
        for secret in [password, self.secret, record.digest(self.secret.encode())]:
            self.assertNotIn(secret, output.getvalue() + str(error.exception))

    def test_ignored_native_files_are_not_silently_deployed(self):
        source = self.root / "workspace"
        source.mkdir()
        (source / "enderpin.toml").write_text("format=4")
        (source / "enderpin.lock").write_text("format=4")
        files = source / "server/files"
        files.mkdir(parents=True)
        (files / "server.properties").write_text("view-distance=6")
        (self.root / ".gitignore").write_text("*.local\n")
        record.run(["git", "init", "-q", str(self.root)])
        record.run(["git", "add", "."], cwd=self.root)
        selected = {"path": str(source), "inputs": ["server/files"]}
        record.tracked_inputs(self.root, selected)
        (files / "private.local").write_text(self.secret)
        with self.assertRaises(ValueError) as error:
            record.tracked_inputs(self.root, selected)
        self.assertNotIn(self.secret, str(error.exception))

    def test_record_is_atomic_private_and_failed_apply_preserves_previous(self):
        path = self.root / "last.json"
        record.write(path, b"previous")
        self.assertEqual(path.stat().st_mode & 0o777, 0o600)
        with patch.object(Path, "replace", side_effect=OSError("disk full")):
            with self.assertRaises(OSError):
                record.write(path, b"new")
        self.assertEqual(path.read_bytes(), b"previous")
        self.assertFalse(list(self.root.glob(".enderpin-record-*")))
        proposed = {"manage_vpn": False, "inventory": {}}
        reviewed = self.root / "manifest.json"
        args = ["--inventory", str(self.inventory), "--manifest", str(reviewed)]
        with patch.dict(os.environ, GITHUB_SHA="a" * 40), patch.object(cd, "manifest", return_value=proposed), \
             patch.object(cd, "apply", side_effect=ValueError("apply failed")), contextlib.redirect_stdout(io.StringIO()):
            cd.main(["plan", *args])
            with self.assertRaises(ValueError):
                cd.main(["apply", *args, "--record", str(path)])
        self.assertEqual(path.read_bytes(), b"previous")

    def test_apply_requires_exact_review_and_success_is_recorded(self):
        proposed = {"source_commit": "a" * 40, "manage_vpn": False, "inventory": {}, "hosts": {"memory_mib": 1024}}
        reviewed = self.root / "manifest.json"
        previous = self.root / "last.json"
        args = ["--inventory", str(self.inventory), "--manifest", str(reviewed), "--previous", str(previous)]
        with patch.dict(os.environ, GITHUB_SHA="a" * 40), patch.object(cd, "manifest", return_value=proposed), \
             patch.object(cd, "apply") as apply, contextlib.redirect_stdout(io.StringIO()):
            cd.main(["plan", *args])
            artifact = reviewed.read_text() + reviewed.with_suffix(".diff").read_text()
            self.assertIn("1024", artifact)
            self.assertNotIn(self.secret, artifact)
            proposed["hosts"]["memory_mib"] = 2048
            with self.assertRaises(ValueError):
                cd.main(["apply", *args, "--record", str(previous)])
            apply.assert_not_called()
            proposed["hosts"]["memory_mib"] = 1024
            cd.main(["apply", *args, "--record", str(previous)])
            apply.assert_called_once()
            self.assertEqual(json.loads(previous.read_bytes()), json.loads(reviewed.read_bytes()))
            # Another completed deployment invalidates an old baseline even if inputs match.
            with self.assertRaises(ValueError):
                cd.main(["apply", *args, "--record", str(previous)])


if __name__ == "__main__":
    unittest.main()
