use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

pub(super) fn install_verified_test_package(workspace_root: &Path, skill_name: &str) {
    let source_root = workspace_root.join("receipt-fixtures").join(skill_name);
    fs::create_dir_all(&source_root).expect("create receipt fixture source");
    let executable = source_root.join("fixture_skill.py");
    fs::write(
        &executable,
        r#"#!/usr/bin/env python3
import json
import sys

request = json.loads(sys.stdin.readline())
print(json.dumps({
    "request_id": request["request_id"],
    "status": "ok",
    "text": "",
    "error_text": None,
    "extra": {"protocol_smoke": True},
}, separators=(",", ":")))
"#,
    )
    .expect("write receipt fixture executable");
    let digest = format!(
        "{:x}",
        Sha256::digest(fs::read(&executable).expect("read receipt fixture"))
    );
    let platform = skill_sdk::HostPlatform::current();
    let source_relative = format!("receipt-fixtures/{skill_name}");
    let manifest = format!(
        r#"schema_version = 1

[package]
name = "{skill_name}"
version = "1.0.0"
description = "Verified clawd test fixture"
protocol = "agent-jsonl-v1"
supported_os = ["{os}"]
supported_arch = ["{arch}"]
license = "MIT"

[registry]
name = "{skill_name}"
capability_policy_source = "registry"

[build]
adapter = "prebuilt"
source_root = "{source_relative}"
network = "deny"

[[build.artifacts]]
os = "{os}"
arch = "{arch}"
sha256 = "{digest}"
source_path = "fixture_skill.py"
executable = true

[run]
launcher = "native"
entrypoint = "runtime/bin/{skill_name}-skill"
working_directory = "."
timeout_seconds = 5
smoke_args = {{ action = "protocol_smoke" }}

[security]
capability_policy_source = "registry"
sandbox = "required"
runtime_network = false
inherit_credentials = false

[storage]
kind = "none"
schema_version = 1
migration_owner = "{skill_name}"
"#,
        os = platform.os,
        arch = platform.arch,
    );
    let manifest_path = source_root.join("skill.toml");
    fs::write(&manifest_path, manifest).expect("write receipt fixture manifest");
    skill_sdk::SkillInstaller
        .install(&skill_sdk::InstallRequest {
            manifest_path,
            workspace_root: workspace_root.to_path_buf(),
            package_root: workspace_root.join("data/skill-packages"),
            target: None,
            allow_network: false,
            control: None,
        })
        .expect("install verified receipt fixture");
}
