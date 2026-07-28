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
        find_program("definitely-not-a-rustclaw-toolchain-7f34")
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
