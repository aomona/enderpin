//! Read-only configuration inspection. This deliberately never opens a Workspace.
use std::path::Path;

use serde::Serialize;
use serde_json::{Value, json};

use crate::{Manifest, model::Lockfile, runtime::RuntimeLock, storage};

#[derive(Debug, Serialize)]
pub struct InspectionError {
    pub code: &'static str,
    pub message: &'static str,
}

fn error(code: &'static str, message: &'static str) -> InspectionError {
    // Parser/validation errors can include source lines containing credentials.
    InspectionError { code, message }
}

/// Schema v1 is an inspection contract, never an executable launch plan.
pub fn inspect(root: &Path, selection: &str) -> Result<Value, InspectionError> {
    let root = root
        .canonicalize()
        .map_err(|_| error("workspace_unreadable", "Cannot read workspace"))?;
    let text = storage::read_optional(&root, "enderpin.toml")
        .map_err(|_| error("manifest_unreadable", "Cannot safely read manifest"))?
        .ok_or_else(|| error("manifest_missing", "Manifest is missing"))?;
    let manifest = Manifest::parse(&text).map_err(|_| {
        error(
            "invalid_manifest",
            "Manifest schema or target configuration is invalid",
        )
    })?;
    let names: Vec<_> = if selection == "all" {
        manifest.targets.keys().map(String::as_str).collect()
    } else {
        if !manifest.targets.contains_key(selection) {
            return Err(error("unknown_target", "Selected target is not declared"));
        }
        vec![selection]
    };
    let lock = storage::read_optional(&root, "enderpin.lock")
        .map_err(|_| error("lock_unreadable", "Cannot safely read lock"))?
        .map(|text| {
            let lock: Lockfile = toml::from_str(&text)
                .map_err(|_| error("invalid_lock", "Lock schema or identities are invalid"))?;
            lock.validate()
                .map_err(|_| error("invalid_lock", "Lock schema or identities are invalid"))?;
            Ok(lock)
        })
        .transpose()?
        .unwrap_or_default();
    let mut targets = serde_json::Map::new();
    for name in names {
        let target = manifest
            .target(name)
            .map_err(|_| error("invalid_manifest", "Target configuration is invalid"))?;
        let package_fingerprint = target
            .fingerprint()
            .map_err(|_| error("invalid_manifest", "Cannot fingerprint target"))?;
        let runtime_fingerprint = RuntimeLock::fingerprint(&manifest, name)
            .map_err(|_| error("invalid_manifest", "Cannot fingerprint runtime"))?;
        let package_lock = lock.targets.get(name);
        let runtime_lock = lock.runtimes.get(name);
        let status = |fingerprint: Option<&str>, expected: &str| match fingerprint {
            None => "missing",
            Some(value) if value == expected => "current",
            Some(_) => "stale",
        };
        let package_status = status(
            package_lock.map(|p| p.fingerprint.as_str()),
            &package_fingerprint,
        );
        let runtime_status = status(
            runtime_lock.map(|p| p.fingerprint.as_str()),
            &runtime_fingerprint,
        );
        let mut diagnostics = Vec::new();
        for (scope, status) in [("packages", package_status), ("runtime", runtime_status)] {
            if status != "current" {
                diagnostics
                    .push(json!({"code": format!("{scope}_lock_{status}"), "severity": "warning"}));
            }
        }
        if target.loader == "neoforge" {
            diagnostics.push(json!({"code": "unsupported_runtime", "severity": "error"}));
        }
        if package_status != "current" {
            diagnostics
                .push(json!({"code": "package_compatibility_unverified", "severity": "warning"}));
        }
        for (alias, package) in &target.requests {
            if !target.disabled.contains(alias) && !target.supports_package(package.kind) {
                diagnostics.push(json!({"code": "incompatible_package_kind", "package": alias,
                    "severity": if target.packages.contains_key(alias) || target.enabled.contains(alias) { "error" } else { "warning" }}));
            }
        }
        let files = crate::workspace::shared_file_inputs(&root, name, &target.file_groups)
            .map_err(|_| {
                error(
                    "invalid_file_inputs",
                    "Native file inputs are unsafe, unreadable or conflicting",
                )
            })?;
        let inputs: Vec<_> = target
            .file_groups
            .iter()
            .map(|g| format!("shared/{g}/files"))
            .chain(std::iter::once(format!("{name}/files")))
            .collect();
        let packages: serde_json::Map<_, _> = target
            .requests
            .iter()
            .map(|(alias, package)| {
                let mut declaration = json!(package);
                if package.url.is_some() {
                    declaration["url"] = json!("[REDACTED]");
                }
                (
                    alias.clone(),
                    json!({
                        "declaration": declaration,
                        "sources": target.package_sources[alias],
                        "direct": target.packages.contains_key(alias),
                    }),
                )
            })
            .collect();
        // Arbitrary JVM property/option values can contain credentials, even with innocuous names.
        let jvm_args: Vec<_> = target
            .jvm_args
            .iter()
            .map(|arg| {
                arg.split_once('=')
                    .map_or_else(|| arg.clone(), |(key, _)| format!("{key}=[REDACTED]"))
            })
            .collect();
        let locked_packages: Vec<_> = package_lock
            .into_iter()
            .flat_map(|p| &p.packages)
            .map(|p| {
                json!({
                    "key": p.key, "name": p.name, "kind": p.kind, "project": p.project,
                    "version": p.version, "sha512": p.sha512, "size": p.size,
                    "filename": p.filename, "roots": p.roots, "required": p.required,
                    "default_sides": p.default_sides,
                })
            })
            .collect();
        targets.insert(name.into(), json!({
            "type": target.kind, "side": target.side(),
            "runtime": {"minecraft": target.minecraft, "loader": target.loader, "loader_version": target.loader_version},
            "launch": {"memory_mib": target.memory_mib, "jvm_args": jvm_args, "permissions": target.permissions},
            "sources": target.sources, "profile": manifest.targets[name].profile,
            "groups": target.groups, "file_groups": target.file_groups,
            "file_inputs": inputs, "files": files,
            "packages": packages, "enabled": target.enabled, "disabled": target.disabled,
            "lock": {
                "packages": {"status": package_status, "expected_fingerprint": package_fingerprint,
                    "fingerprint": package_lock.map(|p| &p.fingerprint), "identities": locked_packages},
                "runtime": {"status": runtime_status, "expected_fingerprint": runtime_fingerprint,
                    "identity": runtime_lock.map(|r| json!({"fingerprint": r.fingerprint,
                        "minecraft": r.minecraft, "loader": r.loader, "loader_version": r.loader_version,
                        "java_major": r.java.major}))},
            },
            "diagnostics": diagnostics,
        }));
    }
    Ok(json!({"schema_version": 1, "manifest_format": manifest.format, "targets": targets}))
}
