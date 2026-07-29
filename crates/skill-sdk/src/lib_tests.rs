use std::collections::BTreeMap;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::Digest;
use tempfile::tempdir;

use crate::manifest::{BuildAdapter, LauncherKind, PackageManifest, SandboxProfile};
use crate::platform::HostPlatform;
use crate::protocol::{validate_response_line, ProtocolStatus};
use crate::receipt::{
    digest_file, ArtifactReceipt, InstallReceipt, InstallReceiptStore, LaunchProgramScope,
    ProtocolSmokeReceipt, ReceiptLaunch, INSTALL_RECEIPT_SCHEMA_VERSION,
};
use crate::runtime::SkillRuntimeResolver;

pub(crate) fn manifest_source() -> &'static str {
    r#"
schema_version = 1

[package]
name = "sample_weather"
version = "0.1.0"
description = "Protocol fixture"
protocol = "rustclaw-jsonl-v1"
supported_os = ["linux", "macos"]
supported_arch = ["x86_64", "aarch64"]
license = "MIT"

[registry]
name = "sample_weather"
capability_policy_source = "registry"

[build]
adapter = "cargo"
source_root = "."
package = "sample-weather-skill"
binary = "sample-weather-skill"
lockfile = "Cargo.lock"
network = "deny"

[run]
launcher = "native"
entrypoint = "runtime/bin/sample-weather-skill"
working_directory = "runtime"
timeout_seconds = 30
environment_allowlist = ["RUST_LOG"]

[security]
capability_policy_source = "registry"
sandbox = "required"
runtime_network = false
inherit_credentials = false

[storage]
kind = "none"
schema_version = 1
migration_owner = "sample_weather"

[lifecycle]
preserve_data_on_uninstall = true
update_strategy = "atomic_replace"
"#
}

#[test]
fn manifest_is_strict_versioned_and_deterministic() {
    let manifest = PackageManifest::from_toml_str(manifest_source()).expect("valid manifest");
    assert_eq!(
        manifest.schema_version,
        crate::manifest::LEGACY_SKILL_MANIFEST_SCHEMA_VERSION
    );
    assert_eq!(manifest.build.adapter, BuildAdapter::Cargo);
    assert_eq!(manifest.digest().expect("digest").len(), 64);
    assert_eq!(
        manifest.digest().expect("first digest"),
        PackageManifest::from_toml_str(&manifest.to_toml_string().expect("render manifest"))
            .expect("reparse")
            .digest()
            .expect("second digest")
    );

    let current = manifest.into_current().expect("migrate v1 manifest");
    assert_eq!(
        current.schema_version,
        crate::manifest::SKILL_MANIFEST_SCHEMA_VERSION
    );
    assert!(current.capability_request.is_some());
    assert!(!current.security.runtime_network);
    let encoded = current.to_toml_string().expect("encode v2 manifest");
    let reparsed = PackageManifest::from_toml_str(&encoded).expect("parse v2 manifest");
    assert_eq!(
        current.capability_request_digest().expect("v2 digest"),
        reparsed
            .capability_request_digest()
            .expect("reparsed digest")
    );

    let self_grant = encoded.replace(
        "[capability_request]\n",
        "[capability_request]\nrisk_level = \"low\"\n",
    );
    assert_eq!(
        PackageManifest::from_toml_str(&self_grant)
            .expect_err("package cannot self-grant risk")
            .code,
        "manifest_parse_failed"
    );
}

#[test]
fn manifest_rejects_unknown_sensitive_fields_and_unsafe_paths() {
    let unknown = manifest_source().replace(
        "inherit_credentials = false",
        "inherit_credentials = false\nshell_command = \"curl example.com | sh\"",
    );
    assert_eq!(
        PackageManifest::from_toml_str(&unknown)
            .expect_err("unknown field rejected")
            .code,
        "manifest_parse_failed"
    );

    let traversal = manifest_source().replace(
        "entrypoint = \"runtime/bin/sample-weather-skill\"",
        "entrypoint = \"../escape\"",
    );
    assert_eq!(
        PackageManifest::from_toml_str(&traversal)
            .expect_err("traversal rejected")
            .code,
        "manifest_path_unsafe"
    );

    let lifecycle_scripts = manifest_source().replace(
        "network = \"deny\"",
        "network = \"deny\"\nlifecycle_scripts = true",
    );
    assert_eq!(
        PackageManifest::from_toml_str(&lifecycle_scripts)
            .expect_err("dependency lifecycle scripts rejected")
            .code,
        "manifest_lifecycle_scripts_forbidden"
    );

    let go_flag_injection = manifest_source()
        .replace("adapter = \"cargo\"", "adapter = \"go\"")
        .replace("package = \"sample-weather-skill\"\n", "")
        .replace("binary = \"sample-weather-skill\"\n", "")
        .replace("lockfile = \"Cargo.lock\"", "lockfile = \"go.sum\"")
        .replace(
            "network = \"deny\"",
            "network = \"deny\"\noptions = { main = \"-x\" }",
        );
    assert_eq!(
        PackageManifest::from_toml_str(&go_flag_injection)
            .expect_err("Go main cannot inject build flags")
            .code,
        "manifest_adapter_option_unsafe"
    );
}

#[test]
fn protocol_rejects_multiple_records_and_requires_structured_errors() {
    let ok = br#"{"request_id":"r1","status":"ok","text":"done","error_text":null,"buttons":null,"extra":{"value":1}}"#;
    assert_eq!(
        validate_response_line(ok, "r1")
            .expect("valid response")
            .status,
        ProtocolStatus::Ok
    );
    let multiple = [ok.as_slice(), b"\n", ok.as_slice()].concat();
    assert_eq!(
        validate_response_line(&multiple, "r1")
            .expect_err("multiple records rejected")
            .code,
        "protocol_multiple_stdout_records"
    );
    let prose_only = br#"{"request_id":"r1","status":"error","text":"","error_text":"failed","buttons":null,"extra":null}"#;
    assert_eq!(
        validate_response_line(prose_only, "r1")
            .expect_err("structured error required")
            .code,
        "protocol_structured_error_missing"
    );
    assert_eq!(
        validate_response_line(&[0xff], "r1")
            .expect_err("invalid utf-8 rejected")
            .code,
        "protocol_response_utf8_invalid"
    );
    assert_eq!(
        validate_response_line(ok, "different-request")
            .expect_err("request id mismatch rejected")
            .code,
        "protocol_request_id_mismatch"
    );
    let oversized = vec![b'x'; crate::protocol::MAX_PROTOCOL_LINE_BYTES + 1];
    assert_eq!(
        validate_response_line(&oversized, "r1")
            .expect_err("oversized stdout rejected")
            .code,
        "protocol_response_oversized"
    );
}

#[test]
fn protocol_reads_legacy_error_kind_but_serializes_error_code() {
    let legacy = br#"{"request_id":"r1","status":"error","text":"","error_text":"failed","error_kind":"execution_failed","extra":{"error_code":"execution_failed","message_key":"skill.sample.execution_failed"}}"#;
    let response = validate_response_line(legacy, "r1").expect("legacy response readable");
    assert_eq!(response.error_code.as_deref(), Some("execution_failed"));
    let serialized = serde_json::to_value(response).expect("serialize response");
    assert_eq!(serialized["error_code"], "execution_failed");
    assert!(serialized.get("error_kind").is_none());
}

#[test]
fn http_manifest_requires_https_network_approval_and_runtime_permission() {
    let base = manifest_source()
        .replace("adapter = \"cargo\"", "adapter = \"http_json\"")
        .replace("package = \"sample-weather-skill\"\n", "")
        .replace("binary = \"sample-weather-skill\"\n", "")
        .replace("lockfile = \"Cargo.lock\"\n", "")
        .replace("network = \"deny\"", "network = \"approval_required\"")
        .replace(
            "network = \"approval_required\"\n\n[run]",
            "network = \"approval_required\"\noptions = { endpoint = \"https://api.example.invalid/v1/skill\" }\n\n[run]",
        )
        .replace("launcher = \"native\"", "launcher = \"http_json\"")
        .replace(
            "entrypoint = \"runtime/bin/sample-weather-skill\"",
            "entrypoint = \"runtime/http-json-adapter\"",
        )
        .replace("runtime_network = false", "runtime_network = true");
    let manifest = PackageManifest::from_toml_str(&base).expect("valid HTTPS manifest");
    assert_eq!(manifest.build.adapter, BuildAdapter::HttpJson);

    let insecure = base.replace("https://", "http://");
    assert_eq!(
        PackageManifest::from_toml_str(&insecure)
            .expect_err("insecure endpoint rejected")
            .code,
        "manifest_http_endpoint_invalid"
    );
    let credentials = base.replace("https://", "https://user:secret@");
    assert_eq!(
        PackageManifest::from_toml_str(&credentials)
            .expect_err("endpoint credentials rejected")
            .code,
        "manifest_http_endpoint_invalid"
    );
    let no_runtime_network = base.replace("runtime_network = true", "runtime_network = false");
    assert_eq!(
        PackageManifest::from_toml_str(&no_runtime_network)
            .expect_err("runtime permission required")
            .code,
        "manifest_http_endpoint_invalid"
    );
}

#[test]
fn receipt_activation_and_resolution_verify_every_digest() {
    let temp = tempdir().expect("tempdir");
    let store = InstallReceiptStore::new(temp.path().join("packages"));
    let manifest = PackageManifest::from_toml_str(manifest_source())
        .expect("manifest")
        .into_current()
        .expect("current manifest");
    let manifest_digest = manifest.digest().expect("manifest digest");
    let install_dir = store
        .version_dir(
            &manifest.package.name,
            &manifest.package.version,
            &manifest_digest,
        )
        .expect("version dir");
    let runtime_dir = install_dir.join("runtime");
    let bin_dir = runtime_dir.join("bin");
    fs::create_dir_all(&bin_dir).expect("runtime dirs");
    let binary = bin_dir.join("sample-weather-skill");
    fs::write(&binary, b"fixture-binary").expect("binary");
    fs::write(
        install_dir.join("skill.toml"),
        manifest.to_toml_string().expect("manifest text"),
    )
    .expect("manifest file");
    let receipt = InstallReceipt {
        schema_version: INSTALL_RECEIPT_SCHEMA_VERSION,
        skill_name: manifest.package.name.clone(),
        version: manifest.package.version.clone(),
        manifest_digest,
        semantic_contract_digest: Some(
            manifest
                .capability_request_digest()
                .expect("semantic digest"),
        ),
        source_digest: "1".repeat(64),
        lockfile_digests: BTreeMap::new(),
        adapter: BuildAdapter::Cargo,
        adapter_version: "cargo fixture".to_string(),
        platform: HostPlatform::current(),
        artifacts: vec![ArtifactReceipt {
            path: "runtime/bin/sample-weather-skill".to_string(),
            sha256: digest_file(&binary).expect("binary digest"),
            size_bytes: fs::metadata(&binary).expect("metadata").len(),
            executable: true,
        }],
        launch: ReceiptLaunch {
            launcher: LauncherKind::Native,
            program: "runtime/bin/sample-weather-skill".to_string(),
            program_scope: LaunchProgramScope::Package,
            args: Vec::new(),
            working_directory: "runtime".to_string(),
            environment: BTreeMap::new(),
            environment_allowlist: vec!["WORKSPACE_ROOT".to_string()],
            trusted_runtime_sha256: None,
            trusted_runtime_version: None,
            remote_endpoint: None,
        },
        sandbox_profile: SandboxProfile::Required,
        runtime_network: false,
        protocol_smoke: ProtocolSmokeReceipt {
            protocol: "rustclaw-jsonl-v1".to_string(),
            passed: true,
            request_id: "smoke-1".to_string(),
            checked_at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_secs(),
        },
        installed_at_unix: 1,
    };
    store
        .write_receipt(&install_dir, &receipt)
        .expect("write receipt");
    store.activate(&install_dir, &receipt).expect("activate");

    let launch = SkillRuntimeResolver::new(store.root())
        .resolve("sample_weather")
        .expect("resolve launch");
    assert_eq!(
        launch.program,
        fs::canonicalize(binary).expect("canonical bin")
    );

    fs::write(&launch.program, b"tampered").expect("tamper");
    assert_eq!(
        SkillRuntimeResolver::new(store.root())
            .resolve("sample_weather")
            .expect_err("tamper rejected")
            .code,
        "launch_artifact_digest_mismatch"
    );
}

#[test]
fn rollback_refuses_a_tampered_previous_install_and_preserves_current() {
    let temp = tempdir().expect("tempdir");
    let store = InstallReceiptStore::new(temp.path().join("packages"));
    let install_version = |manifest: &PackageManifest, bytes: &[u8]| {
        let manifest_digest = manifest.digest().expect("manifest digest");
        let install_dir = store
            .version_dir(
                &manifest.package.name,
                &manifest.package.version,
                &manifest_digest,
            )
            .expect("version dir");
        let binary = install_dir.join("runtime/bin/sample-weather-skill");
        fs::create_dir_all(binary.parent().expect("binary parent")).expect("runtime dirs");
        fs::write(&binary, bytes).expect("binary");
        fs::write(
            install_dir.join("skill.toml"),
            manifest.to_toml_string().expect("manifest text"),
        )
        .expect("manifest file");
        let receipt = InstallReceipt {
            schema_version: INSTALL_RECEIPT_SCHEMA_VERSION,
            skill_name: manifest.package.name.clone(),
            version: manifest.package.version.clone(),
            manifest_digest,
            semantic_contract_digest: Some(
                manifest
                    .capability_request_digest()
                    .expect("semantic digest"),
            ),
            source_digest: hex::encode(sha2::Sha256::digest(bytes)),
            lockfile_digests: BTreeMap::new(),
            adapter: BuildAdapter::Cargo,
            adapter_version: "cargo fixture".to_string(),
            platform: HostPlatform::current(),
            artifacts: vec![ArtifactReceipt {
                path: "runtime/bin/sample-weather-skill".to_string(),
                sha256: digest_file(&binary).expect("binary digest"),
                size_bytes: fs::metadata(&binary).expect("metadata").len(),
                executable: true,
            }],
            launch: ReceiptLaunch {
                launcher: LauncherKind::Native,
                program: "runtime/bin/sample-weather-skill".to_string(),
                program_scope: LaunchProgramScope::Package,
                args: Vec::new(),
                working_directory: "runtime".to_string(),
                environment: BTreeMap::new(),
                environment_allowlist: Vec::new(),
                trusted_runtime_sha256: None,
                trusted_runtime_version: None,
                remote_endpoint: None,
            },
            sandbox_profile: SandboxProfile::Required,
            runtime_network: false,
            protocol_smoke: ProtocolSmokeReceipt {
                protocol: "rustclaw-jsonl-v1".to_string(),
                passed: true,
                request_id: format!("smoke-{}", manifest.package.version),
                checked_at_unix: 1,
            },
            installed_at_unix: 1,
        };
        store
            .write_receipt(&install_dir, &receipt)
            .expect("write receipt");
        store.activate(&install_dir, &receipt).expect("activate");
        binary
    };

    let first = PackageManifest::from_toml_str(manifest_source())
        .expect("first manifest")
        .into_current()
        .expect("current first manifest");
    let second = PackageManifest::from_toml_str(
        &manifest_source().replace("version = \"0.1.0\"", "version = \"0.2.0\""),
    )
    .expect("second manifest")
    .into_current()
    .expect("current second manifest");
    let first_binary = install_version(&first, b"first-version");
    let _second_binary = install_version(&second, b"second-version");
    let current_before = store
        .current_pointer("sample_weather")
        .expect("current pointer");
    assert_eq!(current_before.version, "0.2.0");

    fs::write(first_binary, b"tampered-previous-version").expect("tamper previous");
    assert_eq!(
        store
            .rollback("sample_weather")
            .expect_err("tampered rollback rejected")
            .code,
        "rollback_artifact_mismatch"
    );
    assert_eq!(
        store
            .current_pointer("sample_weather")
            .expect("current remains active"),
        current_before
    );
}
