use sha2::{Digest, Sha256};

use super::*;

fn artifact(os: &str, arch: &str, contents: &[u8]) -> PlatformArtifact {
    PlatformArtifact {
        os: os.to_string(),
        arch: arch.to_string(),
        sha256: hex::encode(Sha256::digest(contents)),
        source_path: Some("fixture.bin".to_string()),
        url: None,
        executable: true,
        size_bytes: Some(contents.len() as u64),
        archive: None,
    }
}

#[test]
fn prebuilt_selection_requires_an_exact_normalized_platform() {
    let platform = HostPlatform {
        os: "linux".to_string(),
        arch: "aarch64".to_string(),
        target: Some("aarch64-unknown-linux-gnu".to_string()),
    };
    let artifacts = vec![
        artifact("linux", "x86_64", b"x86"),
        artifact("darwin", "arm64", b"mac"),
        artifact("linux", "arm64", b"arm"),
    ];
    let selected = select_prebuilt_artifact(&artifacts, &platform).expect("exact artifact");
    assert_eq!(selected.sha256, hex::encode(Sha256::digest(b"arm")));
    assert_eq!(
        select_prebuilt_artifact(
            &artifacts,
            &HostPlatform {
                os: "linux".to_string(),
                arch: "armv7".to_string(),
                target: Some("armv7-unknown-linux-gnueabihf".to_string()),
            },
        )
        .expect_err("no approximate artifact fallback")
        .code,
        "prebuilt_platform_artifact_missing"
    );
}

#[test]
fn missing_and_unsafe_toolchain_names_are_structured_preflight_errors() {
    assert_eq!(
        find_program("definitely-not-a-agent-runtime-toolchain-7f34")
            .expect_err("missing toolchain")
            .code,
        "toolchain_missing"
    );
    assert_eq!(
        find_program("../bin/python")
            .expect_err("path override rejected")
            .code,
        "toolchain_name_invalid"
    );
}

#[test]
fn http_json_adapter_prepares_a_pinned_https_launch_without_local_toolchains() {
    let source = crate::tests::manifest_source()
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
    let manifest = PackageManifest::from_toml_str(&source).expect("valid HTTP manifest");
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let staging = temp.path().join("staging");
    let cache = temp.path().join("cache");
    fs::create_dir_all(&workspace).expect("workspace");
    let platform = HostPlatform::current();

    let prepared = prepare_package(&AdapterContext {
        manifest: &manifest,
        workspace_root: &workspace,
        manifest_dir: &workspace,
        staging_root: &staging,
        cache_root: &cache,
        platform: &platform,
        target: None,
        allow_network: true,
        control: None,
    })
    .expect("prepare HTTP adapter");

    assert_eq!(prepared.adapter_version, "http-json-v1");
    assert_eq!(prepared.launch.launcher, LauncherKind::HttpJson);
    assert_eq!(
        prepared.launch.remote_endpoint.as_deref(),
        Some("https://api.example.invalid/v1/skill")
    );
    assert_eq!(prepared.artifacts.len(), 1);
    assert!(staging.join("runtime/http-json-adapter").is_file());
}
