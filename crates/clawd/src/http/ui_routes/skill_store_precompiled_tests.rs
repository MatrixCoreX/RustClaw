use super::*;
use skill_sdk::receipt::{LaunchProgramScope, ReceiptLaunch};
use skill_sdk::{
    ArtifactReceipt, HostPlatform, InstallReceipt, InstallReceiptStore, PackageManifest,
    ProtocolSmokeReceipt,
};

struct Fixture {
    workspace: PathBuf,
    manifest: PackageManifest,
    platform: HostPlatform,
}

impl Fixture {
    fn new(target: &str) -> Self {
        let workspace =
            std::env::temp_dir().join(format!("skill-store-candidates-{}", uuid::Uuid::new_v4()));
        let source = workspace.join("source");
        write_external_skill_fixture(&source, "candidate_fixture");
        let mut manifest = PackageManifest::load(&source.join("skill.toml"))
            .unwrap()
            .into_current()
            .unwrap();
        manifest.package.supported_os = vec!["linux".into(), "macos".into()];
        manifest.package.supported_arch = vec!["x86_64".into(), "aarch64".into(), "armv7".into()];
        Self {
            workspace,
            manifest,
            platform: HostPlatform::from_target(target).unwrap(),
        }
    }

    fn packaged(&self) -> PathBuf {
        self.workspace.join("prebuilt/skill-packages")
    }
    fn target_root(&self) -> PathBuf {
        self.workspace
            .join("target/prebuilt-skill-packages")
            .join(self.platform.target.as_ref().unwrap())
    }

    fn publish(&self, root: &Path, manifest: &PackageManifest, platform: HostPlatform) -> PathBuf {
        let store = InstallReceiptStore::new(root);
        let digest = manifest.digest().unwrap();
        let directory = store
            .version_dir(&manifest.package.name, &manifest.package.version, &digest)
            .unwrap();
        std::fs::create_dir_all(directory.join("runtime/src")).unwrap();
        std::fs::write(
            directory.join("skill.toml"),
            manifest.to_toml_string().unwrap(),
        )
        .unwrap();
        let content = "fixture data\n";
        let artifact = directory.join("runtime/src/run.sh");
        std::fs::write(&artifact, content).unwrap();
        let receipt = InstallReceipt {
            schema_version: skill_sdk::INSTALL_RECEIPT_SCHEMA_VERSION,
            skill_name: manifest.package.name.clone(),
            version: manifest.package.version.clone(),
            manifest_digest: digest,
            semantic_contract_digest: Some(manifest.capability_request_digest().unwrap()),
            source_digest: "a".repeat(64),
            lockfile_digests: BTreeMap::new(),
            adapter: manifest.build.adapter,
            adapter_version: "test-v1".into(),
            platform,
            artifacts: vec![ArtifactReceipt {
                path: "runtime/src/run.sh".into(),
                sha256: skill_sdk::digest_file(&artifact).unwrap(),
                size_bytes: content.len() as u64,
                executable: false,
            }],
            launch: ReceiptLaunch {
                launcher: manifest.run.launcher,
                program: "runtime/src/run.sh".into(),
                program_scope: LaunchProgramScope::Package,
                args: vec![],
                working_directory: "runtime/src".into(),
                environment: BTreeMap::new(),
                environment_allowlist: vec![],
                trusted_runtime_sha256: None,
                trusted_runtime_version: None,
                remote_endpoint: None,
            },
            sandbox_profile: manifest.security.sandbox,
            runtime_network: false,
            protocol_smoke: ProtocolSmokeReceipt {
                protocol: "agent-jsonl-v1".into(),
                passed: true,
                request_id: "candidate-test".into(),
                checked_at_unix: 1,
            },
            installed_at_unix: 1,
        };
        store.write_receipt(&directory, &receipt).unwrap();
        store.activate(&directory, &receipt).unwrap();
        directory
    }

    fn select(&self) -> skill_sdk::SkillSdkResult<Option<PathBuf>> {
        precompiled_skill_package_root_for(&self.workspace, &self.platform, &self.manifest)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.workspace);
    }
}

#[test]
fn matching_target_package_is_used_with_missing_empty_or_stale_packaged_cache() {
    let fixture = Fixture::new("aarch64-unknown-linux-gnu");
    fixture.publish(
        &fixture.target_root(),
        &fixture.manifest,
        fixture.platform.clone(),
    );
    assert_eq!(fixture.select().unwrap(), Some(fixture.target_root()));
    std::fs::create_dir_all(fixture.packaged()).unwrap();
    assert_eq!(fixture.select().unwrap(), Some(fixture.target_root()));
    let mut older = fixture.manifest.clone();
    older.package.version = "0.9.0".into();
    fixture.publish(&fixture.packaged(), &older, fixture.platform.clone());
    assert_eq!(fixture.select().unwrap(), Some(fixture.target_root()));
}

#[test]
fn same_version_different_manifest_digest_does_not_shadow_the_matching_target() {
    let fixture = Fixture::new("x86_64-unknown-linux-gnu");
    let mut stale = fixture.manifest.clone();
    stale.package.description = "Old source, same version".into();
    fixture.publish(&fixture.packaged(), &stale, fixture.platform.clone());
    fixture.publish(
        &fixture.target_root(),
        &fixture.manifest,
        fixture.platform.clone(),
    );
    assert_eq!(fixture.select().unwrap(), Some(fixture.target_root()));
}

#[test]
fn matching_packaged_cache_keeps_priority_on_linux_pi_and_both_mac_architectures() {
    for target in [
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "armv7-unknown-linux-gnueabihf",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
    ] {
        let fixture = Fixture::new(target);
        fixture.publish(
            &fixture.packaged(),
            &fixture.manifest,
            fixture.platform.clone(),
        );
        fixture.publish(
            &fixture.target_root(),
            &fixture.manifest,
            fixture.platform.clone(),
        );
        assert_eq!(
            fixture.select().unwrap(),
            Some(fixture.packaged()),
            "target={target}"
        );
    }
}

#[test]
fn wrong_os_architecture_or_abi_cannot_be_selected() {
    let fixture = Fixture::new("aarch64-apple-darwin");
    for target in ["aarch64-unknown-linux-gnu", "x86_64-apple-darwin"] {
        fixture.publish(
            &fixture.packaged(),
            &fixture.manifest,
            HostPlatform::from_target(target).unwrap(),
        );
        assert!(fixture.select().is_err(), "target={target}");
        fixture.publish(
            &fixture.target_root(),
            &fixture.manifest,
            fixture.platform.clone(),
        );
        assert_eq!(fixture.select().unwrap(), Some(fixture.target_root()));
        std::fs::remove_dir_all(fixture.target_root()).unwrap();
    }
    let fixture = Fixture::new("x86_64-unknown-linux-gnu");
    fixture.publish(
        &fixture.packaged(),
        &fixture.manifest,
        HostPlatform::from_target("x86_64-unknown-linux-musl").unwrap(),
    );
    assert!(fixture.select().is_err());
}

#[test]
fn invalid_pointer_is_not_an_installable_candidate_or_a_source_build_fallback() {
    let fixture = Fixture::new("x86_64-unknown-linux-gnu");
    assert!(fixture.select().unwrap().is_none());
    let pointer = fixture
        .packaged()
        .join(&fixture.manifest.package.name)
        .join("current.json");
    std::fs::create_dir_all(pointer.parent().unwrap()).unwrap();
    std::fs::write(pointer, "not json").unwrap();
    assert_eq!(
        fixture.select().unwrap_err().code,
        "precompiled_package_unavailable"
    );
    fixture.publish(
        &fixture.target_root(),
        &fixture.manifest,
        fixture.platform.clone(),
    );
    assert_eq!(fixture.select().unwrap(), Some(fixture.target_root()));
}

#[test]
fn invalid_receipt_is_rejected_and_final_installer_still_checks_artifact_hashes() {
    let fixture = Fixture::new("x86_64-unknown-linux-gnu");
    let installed = fixture.publish(
        &fixture.packaged(),
        &fixture.manifest,
        fixture.platform.clone(),
    );
    std::fs::write(installed.join("runtime/src/run.sh"), "tampered data").unwrap();
    let root = fixture
        .select()
        .unwrap()
        .expect("metadata selects candidate only");
    let manifest_path = fixture.workspace.join("source/skill.toml");
    std::fs::write(&manifest_path, fixture.manifest.to_toml_string().unwrap()).unwrap();
    let error = skill_sdk::SkillInstaller
        .install_precompiled(&skill_sdk::PrecompiledInstallRequest {
            manifest_path,
            workspace_root: fixture.workspace.clone(),
            package_root: fixture.workspace.join("installed"),
            precompiled_root: root,
            target: fixture.platform.target.clone(),
            control: None,
        })
        .expect_err("artifact verification must not be bypassed");
    assert_eq!(error.code, "precompiled_artifact_mismatch");
    std::fs::write(installed.join("install-receipt.json"), "{}").unwrap();
    assert!(fixture.select().is_err());
}
