use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn make_echo_skill_runner(root: &Path) -> PathBuf {
    let path = root.join("echo-skill-runner");
    fs::write(
        &path,
        r#"#!/usr/bin/env python3
import json
import sys

request = json.loads(sys.stdin.readline())
binding = {
    "skill_name": request.get("skill_name"),
    "version": request.get("expected_skill_version"),
    "manifest_digest": request.get("expected_manifest_digest"),
    "receipt_digest": request.get("expected_receipt_digest"),
    "registry_generation": request.get("expected_registry_generation"),
    "registry_generation_digest": request.get("expected_registry_generation_digest"),
    "base_registry_digest": request.get("expected_base_registry_digest"),
    "overlay_generation_digest": request.get("expected_overlay_generation_digest"),
    "policy_digest": request.get("expected_policy_digest"),
    "admission_receipt_digest": request.get("expected_admission_receipt_digest"),
}
print(json.dumps({
    "request_id": request.get("request_id", ""),
    "status": "ok",
    "text": json.dumps(request.get("args", {}), ensure_ascii=False),
    "error_text": None,
    "extra": {"execution_binding": binding},
}, ensure_ascii=False))
"#,
    )
    .expect("write fake skill runner");
    mark_executable(&path, "fake runner");
    path
}

#[cfg(target_os = "linux")]
pub(super) fn make_sandbox_probe_skill_runner(root: &Path) -> PathBuf {
    let path = root.join("sandbox-probe-skill-runner");
    fs::write(
        &path,
        r#"#!/usr/bin/env python3
import json
import os
import sys

request = json.loads(sys.stdin.readline())
args = request.get("args", {})

def try_write(path):
    try:
        with open(path, "w", encoding="utf-8") as handle:
            handle.write("probe")
        return True
    except OSError:
        return False

result = {
    "workspace_write": try_write(os.path.join(os.environ["WORKSPACE_ROOT"], "workspace-probe.txt")),
    "outside_write": try_write(args["outside_path"]),
}
binding = {
    "skill_name": request.get("skill_name"),
    "version": request.get("expected_skill_version"),
    "manifest_digest": request.get("expected_manifest_digest"),
    "receipt_digest": request.get("expected_receipt_digest"),
    "registry_generation": request.get("expected_registry_generation"),
    "registry_generation_digest": request.get("expected_registry_generation_digest"),
    "base_registry_digest": request.get("expected_base_registry_digest"),
    "overlay_generation_digest": request.get("expected_overlay_generation_digest"),
    "policy_digest": request.get("expected_policy_digest"),
    "admission_receipt_digest": request.get("expected_admission_receipt_digest"),
}
print(json.dumps({
    "request_id": request.get("request_id", ""),
    "status": "ok",
    "text": json.dumps(result),
    "error_text": None,
    "extra": {"execution_binding": binding},
}))
"#,
    )
    .expect("write sandbox probe skill runner");
    mark_executable(&path, "sandbox probe");
    path
}

#[cfg(unix)]
fn mark_executable(path: &Path, label: &str) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .unwrap_or_else(|error| panic!("{label} metadata: {error}"))
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap_or_else(|error| panic!("chmod {label}: {error}"));
}

#[cfg(not(unix))]
fn mark_executable(_path: &Path, _label: &str) {}
