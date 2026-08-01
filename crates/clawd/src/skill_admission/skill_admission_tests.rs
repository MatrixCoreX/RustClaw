use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};
use skill_sdk::receipt::{LaunchProgramScope, ReceiptLaunch};
use skill_sdk::{
    digest_file, scaffold_skill, AdmissionState, ApprovalSource, ArtifactReceipt, BuildAdapter,
    GrantedCapability, HostPlatform, HostPolicyGrant, HostRiskLevel, ImplementationLanguage,
    InstallReceipt, InstallReceiptStore, ProtocolSmokeReceipt, RuntimePermissionRequest,
    ScaffoldRequest, AGENT_JSONL_PROTOCOL, HOST_POLICY_GRANT_SCHEMA_VERSION,
    INSTALL_RECEIPT_SCHEMA_VERSION,
};

use super::{
    AdmissionMutation, ExternalSkillMetadata, SkillAdmissionService, SkillAdmissionSource,
};

#[test]
fn generation_activation_is_atomic_restartable_and_tombstone_preserves_shared_tools() {
    let root = std::env::temp_dir().join(format!("skillctl-admission-{}", uuid::Uuid::new_v4()));
    let skill_name = format!("novel_{}", uuid::Uuid::new_v4().simple());
    fs::create_dir_all(&root).expect("create root");
    let base_registry = root.join("configs/skills_registry.toml");
    fs::create_dir_all(base_registry.parent().expect("registry parent")).expect("create configs");
    fs::write(&base_registry, "").expect("write base registry");
    let manifest = install_fixture(&root, &skill_name);
    let capability = manifest
        .effective_capability_request()
        .expect("capability request")
        .capabilities[0]
        .clone();
    let grant = HostPolicyGrant {
        schema_version: HOST_POLICY_GRANT_SCHEMA_VERSION,
        skill_name: manifest.package.name.clone(),
        version: manifest.package.version.clone(),
        semantic_contract_digest: manifest
            .capability_request_digest()
            .expect("semantic digest"),
        capabilities: vec![GrantedCapability {
            name: capability.name,
            action: capability.action,
        }],
        permissions: RuntimePermissionRequest::default(),
        risk_level: HostRiskLevel::Low,
        auto_invocable: true,
        requires_confirmation: false,
        approval_source: ApprovalSource::AdminApi,
        approved_at_unix: 1,
    };
    let service = SkillAdmissionService::for_test(&root, &base_registry);
    let snapshot = service
        .admit_external(AdmissionMutation {
            metadata: ExternalSkillMetadata {
                name: skill_name.clone(),
                source: SkillAdmissionSource::ExternalOverlay,
                package_manifest_path: format!("data/skills/imports/{skill_name}/skill.toml"),
                description: "Novel probe fixture".to_string(),
                aliases: vec!["probe_novel".to_string()],
                group: "extensions".to_string(),
            },
            prompt: "# Novel probe\n".to_string(),
            state: AdmissionState::Enabled,
            grant: Some(grant.clone()),
        })
        .expect("admit external skill");
    assert_eq!(snapshot.generation, 1);
    assert!(snapshot
        .base_registry_digest
        .as_deref()
        .is_some_and(|digest| digest.len() == 64));
    let binding = snapshot
        .execution_bindings
        .get(&skill_name)
        .expect("execution binding");
    assert_eq!(binding.version, manifest.package.version);
    assert_eq!(
        binding.manifest_digest,
        manifest.digest().expect("manifest digest")
    );
    assert!(binding.policy_digest.is_some());
    assert_eq!(binding.install_receipt_digest.len(), 64);
    assert_eq!(binding.admission_receipt_digest.len(), 64);
    assert_eq!(snapshot.state(&skill_name), Some(AdmissionState::Enabled));
    let merged = claw_core::skill_registry::SkillsRegistry::load_from_base_and_overlay(
        &base_registry,
        snapshot.registry_dir.as_deref(),
    )
    .expect("load merged registry");
    assert!(merged.is_known(&skill_name));
    assert_eq!(
        merged.get(&skill_name).map(|entry| entry.enabled),
        Some(true)
    );

    let pointer_before_failure =
        fs::read(service.root().join("current-generation.json")).expect("read current pointer");
    let mut invalid_grant = grant.clone();
    invalid_grant.permissions.privilege_escalation = true;
    let error = service
        .set_state(&skill_name, AdmissionState::Enabled, Some(invalid_grant))
        .expect_err("over-grant must fail");
    assert_eq!(error.code, "skill_policy_grant_invalid");
    assert_eq!(
        pointer_before_failure,
        fs::read(service.root().join("current-generation.json")).expect("reread pointer")
    );

    let repair_inputs = service
        .current_repair_inputs()
        .expect("read complete generation repair inputs");
    assert_eq!(repair_inputs.len(), 1);
    let repaired = service
        .repair_current_generation(repair_inputs)
        .expect("atomically rewrite complete generation");
    assert_eq!(repaired.generation, snapshot.generation + 1);
    assert_eq!(repaired.state(&skill_name), Some(AdmissionState::Enabled));

    let disabled = service
        .set_state(&skill_name, AdmissionState::InstalledDisabled, None)
        .expect("disable skill");
    assert_eq!(disabled.generation, repaired.generation + 1);
    assert_eq!(
        disabled.state(&skill_name),
        Some(AdmissionState::InstalledDisabled)
    );
    let restarted = SkillAdmissionService::for_test(&root, &base_registry)
        .snapshot()
        .expect("restart snapshot");
    assert_eq!(restarted.generation_digest, disabled.generation_digest);

    let awaiting = service
        .set_state(&skill_name, AdmissionState::AwaitingPolicyApproval, None)
        .expect("revoke host grant");
    assert_eq!(
        awaiting.state(&skill_name),
        Some(AdmissionState::AwaitingPolicyApproval)
    );
    assert!(!awaiting.execution_bindings.contains_key(&skill_name));
    let reenabled = service
        .set_state(&skill_name, AdmissionState::Enabled, Some(grant))
        .expect("grant and enable without restart");
    assert_eq!(reenabled.state(&skill_name), Some(AdmissionState::Enabled));
    assert!(reenabled.execution_bindings.contains_key(&skill_name));

    let shared_toolchain = root.join("data/shared-toolchains/python");
    fs::create_dir_all(&shared_toolchain).expect("create shared toolchain fixture");
    fs::write(shared_toolchain.join("sentinel"), b"keep").expect("write sentinel");
    let tombstoned = service
        .set_state(&skill_name, AdmissionState::Tombstoned, None)
        .expect("tombstone skill");
    InstallReceiptStore::new(root.join("data/skill-packages"))
        .remove_installed_versions(&skill_name)
        .expect("remove skill-owned package");
    let after_removal = service.snapshot().expect("load tombstone after removal");
    assert_eq!(after_removal.generation, tombstoned.generation);
    assert_eq!(
        after_removal.state(&skill_name),
        Some(AdmissionState::Tombstoned)
    );
    assert!(shared_toolchain.join("sentinel").is_file());
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn generation_validation_resolves_its_pinned_receipt_after_current_install_changes() {
    let root = std::env::temp_dir().join(format!(
        "skillctl-admission-pinned-update-{}",
        uuid::Uuid::new_v4()
    ));
    let skill_name = format!("pinned_{}", uuid::Uuid::new_v4().simple());
    fs::create_dir_all(&root).expect("create root");
    let base_registry = root.join("configs/skills_registry.toml");
    fs::create_dir_all(base_registry.parent().expect("registry parent")).expect("create configs");
    fs::write(&base_registry, "").expect("write base registry");
    let manifest = install_fixture(&root, &skill_name);
    let capability = manifest
        .effective_capability_request()
        .expect("capability request")
        .capabilities[0]
        .clone();
    let grant = HostPolicyGrant {
        schema_version: HOST_POLICY_GRANT_SCHEMA_VERSION,
        skill_name: skill_name.clone(),
        version: manifest.package.version.clone(),
        semantic_contract_digest: manifest
            .capability_request_digest()
            .expect("semantic digest"),
        capabilities: vec![GrantedCapability {
            name: capability.name,
            action: capability.action,
        }],
        permissions: RuntimePermissionRequest::default(),
        risk_level: HostRiskLevel::Low,
        auto_invocable: true,
        requires_confirmation: false,
        approval_source: ApprovalSource::AdminApi,
        approved_at_unix: 1,
    };
    let service = SkillAdmissionService::for_test(&root, &base_registry);
    let first = service
        .admit_external(AdmissionMutation {
            metadata: ExternalSkillMetadata {
                name: skill_name.clone(),
                source: SkillAdmissionSource::ExternalOverlay,
                package_manifest_path: format!("data/skills/imports/{skill_name}/skill.toml"),
                description: "Pinned update fixture".to_string(),
                aliases: Vec::new(),
                group: "extensions".to_string(),
            },
            prompt: "# Pinned update fixture\n".to_string(),
            state: AdmissionState::Enabled,
            grant: Some(grant.clone()),
        })
        .expect("admit first install");
    let first_binding = first
        .execution_bindings
        .get(&skill_name)
        .expect("first execution binding")
        .clone();

    let updated_receipt = activate_updated_fixture(&root, &manifest, "updated-program");
    assert_ne!(
        updated_receipt.digest().expect("updated receipt digest"),
        first_binding.install_receipt_digest
    );

    let pinned = service
        .snapshot()
        .expect("existing generation remains valid after current changes");
    assert_eq!(
        pinned
            .execution_bindings
            .get(&skill_name)
            .expect("pinned execution binding")
            .install_receipt_digest,
        first_binding.install_receipt_digest
    );

    let updated = service
        .admit_external(AdmissionMutation {
            metadata: ExternalSkillMetadata {
                name: skill_name.clone(),
                source: SkillAdmissionSource::ExternalOverlay,
                package_manifest_path: format!("data/skills/imports/{skill_name}/skill.toml"),
                description: "Pinned update fixture".to_string(),
                aliases: Vec::new(),
                group: "extensions".to_string(),
            },
            prompt: "# Pinned update fixture\n".to_string(),
            state: AdmissionState::Enabled,
            grant: Some(grant),
        })
        .expect("admit updated install");
    assert_eq!(updated.generation, first.generation + 1);
    assert_eq!(
        updated
            .execution_bindings
            .get(&skill_name)
            .expect("updated execution binding")
            .install_receipt_digest,
        updated_receipt.digest().expect("updated receipt digest")
    );
    fs::remove_dir_all(root).expect("remove fixture");
}

fn install_fixture(root: &Path, skill_name: &str) -> skill_sdk::PackageManifest {
    let source = root.join("sources").join(skill_name);
    scaffold_skill(&ScaffoldRequest {
        destination: source.clone(),
        skill_name: skill_name.to_string(),
        capability_summary: "Run a deterministic admission probe".to_string(),
        actions: vec!["run".to_string()],
        implementation_language: ImplementationLanguage::Rust,
        source_root: ".".to_string(),
    })
    .expect("scaffold fixture");
    let manifest =
        skill_sdk::PackageManifest::load(&source.join("skill.toml")).expect("load manifest");
    let store = InstallReceiptStore::new(root.join("data/skill-packages"));
    let manifest_digest = manifest.digest().expect("manifest digest");
    let install_dir = store
        .version_dir(skill_name, &manifest.package.version, &manifest_digest)
        .expect("version dir");
    let runtime = install_dir.join("runtime");
    fs::create_dir_all(&runtime).expect("create runtime");
    let program = runtime.join("skill");
    fs::write(&program, b"fixture-binary").expect("write program");
    fs::write(
        install_dir.join("skill.toml"),
        manifest.to_toml_string().expect("encode manifest"),
    )
    .expect("write installed manifest");
    let program_digest = digest_file(&program).expect("program digest");
    let receipt = InstallReceipt {
        schema_version: INSTALL_RECEIPT_SCHEMA_VERSION,
        skill_name: skill_name.to_string(),
        version: manifest.package.version.clone(),
        manifest_digest,
        semantic_contract_digest: Some(
            manifest
                .capability_request_digest()
                .expect("semantic digest"),
        ),
        source_digest: program_digest.clone(),
        lockfile_digests: BTreeMap::new(),
        adapter: BuildAdapter::Cargo,
        adapter_version: "fixture-v1".to_string(),
        platform: HostPlatform::current(),
        artifacts: vec![ArtifactReceipt {
            path: "runtime/skill".to_string(),
            sha256: program_digest,
            size_bytes: fs::metadata(&program).expect("program metadata").len(),
            executable: true,
        }],
        launch: ReceiptLaunch {
            launcher: manifest.run.launcher,
            program: "runtime/skill".to_string(),
            program_scope: LaunchProgramScope::Package,
            args: Vec::new(),
            working_directory: ".".to_string(),
            environment: BTreeMap::new(),
            environment_allowlist: Vec::new(),
            trusted_runtime_sha256: None,
            trusted_runtime_version: None,
            remote_endpoint: None,
        },
        sandbox_profile: manifest.security.sandbox,
        runtime_network: false,
        protocol_smoke: ProtocolSmokeReceipt {
            protocol: AGENT_JSONL_PROTOCOL.to_string(),
            passed: true,
            request_id: "fixture-smoke".to_string(),
            checked_at_unix: 1,
        },
        installed_at_unix: 1,
    };
    store
        .write_receipt(&install_dir, &receipt)
        .expect("write receipt");
    store.activate(&install_dir, &receipt).expect("activate");
    manifest
}

fn activate_updated_fixture(
    root: &Path,
    manifest: &skill_sdk::PackageManifest,
    contents: &str,
) -> InstallReceipt {
    let skill_name = &manifest.package.name;
    let store = InstallReceiptStore::new(root.join("data/skill-packages"));
    let source_digest = format!("{:x}", Sha256::digest(contents.as_bytes()));
    let install_identity = format!(
        "{:x}",
        Sha256::digest(format!("updated:{source_digest}").as_bytes())
    );
    let install_dir = store
        .version_dir(skill_name, &manifest.package.version, &install_identity)
        .expect("updated version dir");
    let runtime = install_dir.join("runtime");
    fs::create_dir_all(&runtime).expect("create updated runtime");
    let program = runtime.join("skill");
    fs::write(&program, contents).expect("write updated program");
    fs::write(
        install_dir.join("skill.toml"),
        manifest.to_toml_string().expect("encode updated manifest"),
    )
    .expect("write updated installed manifest");
    let program_digest = digest_file(&program).expect("updated program digest");
    let receipt = InstallReceipt {
        schema_version: INSTALL_RECEIPT_SCHEMA_VERSION,
        skill_name: skill_name.clone(),
        version: manifest.package.version.clone(),
        manifest_digest: manifest.digest().expect("updated manifest digest"),
        semantic_contract_digest: Some(
            manifest
                .capability_request_digest()
                .expect("updated semantic digest"),
        ),
        source_digest,
        lockfile_digests: BTreeMap::new(),
        adapter: BuildAdapter::Cargo,
        adapter_version: "fixture-v2".to_string(),
        platform: HostPlatform::current(),
        artifacts: vec![ArtifactReceipt {
            path: "runtime/skill".to_string(),
            sha256: program_digest,
            size_bytes: fs::metadata(&program)
                .expect("updated program metadata")
                .len(),
            executable: true,
        }],
        launch: ReceiptLaunch {
            launcher: manifest.run.launcher,
            program: "runtime/skill".to_string(),
            program_scope: LaunchProgramScope::Package,
            args: Vec::new(),
            working_directory: ".".to_string(),
            environment: BTreeMap::new(),
            environment_allowlist: Vec::new(),
            trusted_runtime_sha256: None,
            trusted_runtime_version: None,
            remote_endpoint: None,
        },
        sandbox_profile: manifest.security.sandbox,
        runtime_network: false,
        protocol_smoke: ProtocolSmokeReceipt {
            protocol: AGENT_JSONL_PROTOCOL.to_string(),
            passed: true,
            request_id: "fixture-update-smoke".to_string(),
            checked_at_unix: 2,
        },
        installed_at_unix: 2,
    };
    store
        .write_receipt(&install_dir, &receipt)
        .expect("write updated receipt");
    store
        .activate(&install_dir, &receipt)
        .expect("activate updated receipt");
    receipt
}
