"""In-memory host operations for the real Ansible deployment control-flow tests."""
import base64
import json
from pathlib import Path

from ansible.plugins.action import ActionBase
from ansible.template import trust_as_template


class ActionModule(ActionBase):
    def run(self, tmp=None, task_vars=None):
        params = self._task.args
        module, args = params["module"], params["args"]
        path = Path(task_vars["model_path"])
        state = json.loads(path.read_text())
        task = self._task.get_name()
        instance = self._templar.template(trust_as_template("{{ instance.name | default('') }}"))
        state["events"].append(dict(task=task, module=module, args=args, instance=instance))
        result = dict(changed=False)
        failure = state.get("failure", {})
        if task == failure.get("task") and instance == failure.get("instance", instance):
            result.update(failed=True, msg="injected host failure")
        elif module == "setup":
            result["ansible_facts"] = dict(os_family="Debian", distribution="Debian")
        elif module == "stat":
            result["stat"] = dict(exists=args["path"] in state["files"])
        elif module == "slurp":
            result["content"] = base64.b64encode(state["files"][args["src"]].encode()).decode()
        elif module == "service_facts":
            result["ansible_facts"] = dict(services={name + ".service": {} for name in state["services"]})
        elif module == "tempfile":
            result.update(path=args["path"] + "/stage-test", changed=True)
        elif module in ["copy", "template"]:
            if module == "template":
                template = Path(task_vars["template_root"]) / args["src"]
                content = self._templar.template(trust_as_template(template.read_text()))
            else:
                content = args.get("content", "copied fixture")
            result["changed"] = state["files"].get(args["dest"]) != content
            state["files"][args["dest"]] = content
        elif module == "file" and args.get("state") == "absent":
            for key in list(state["files"]):
                if key == args["path"] or key.startswith(args["path"] + "/"):
                    del state["files"][key]
        elif module == "systemd_service":
            name = args["name"]
            service = state["services"].setdefault(name, dict(active=False, enabled=False))
            if name != state.get("refuse_stop"):
                service["active"] = args.get("state") != "stopped"
            if "enabled" in args:
                service["enabled"] = args["enabled"]
            if name == "enderpin-firewall":
                state["firewall"] = state["files"]["/etc/enderpin/firewall.nft"]
                state["applied"].append(state["firewall"])
            result["changed"] = True
        elif module == "command":
            argv = args["argv"]
            result.update(rc=0, stdout="", stdout_lines=[])
            if argv[:2] == ["systemctl", "show"]:
                service = state["services"].get(argv[2].removesuffix(".service"))
                load_state = "loaded" if service else "not-found"
                service = service or dict(active=False, enabled=False)
                lines = ["LoadState=" + load_state,
                         "ActiveState=" + ("active" if service["active"] else "inactive"),
                         "SubState=" + ("running" if service["active"] else "dead"),
                         "MainPID=" + ("123" if service["active"] else "0"),
                         "UnitFileState=" + ("enabled" if service["enabled"] else "disabled")]
                result.update(stdout="\n".join(lines), stdout_lines=lines)
            elif "show" in argv and "--json" in argv:
                result["stdout"] = json.dumps(dict(plan=dict(folders=[]), fingerprint="test"))
        path.write_text(json.dumps(state))
        return result
