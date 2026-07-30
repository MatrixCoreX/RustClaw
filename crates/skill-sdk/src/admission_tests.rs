use crate::admission::{
    AdmissionReceipt, AdmissionState, ApprovalSource, GrantedCapability, HostPolicyGrant,
    HostRiskLevel, HOST_POLICY_GRANT_SCHEMA_VERSION,
};
use crate::capability_request::RuntimePermissionRequest;
use crate::manifest::PackageManifest;

fn legacy_manifest() -> PackageManifest {
    PackageManifest::from_toml_str(super::tests::manifest_source()).expect("legacy manifest")
}

#[test]
fn host_grant_must_be_subset_of_package_request() {
    let manifest = legacy_manifest().into_current().expect("migrate manifest");
    let capability = manifest
        .effective_capability_request()
        .expect("request")
        .capabilities[0]
        .clone();
    let mut grant = HostPolicyGrant {
        schema_version: HOST_POLICY_GRANT_SCHEMA_VERSION,
        skill_name: manifest.package.name.clone(),
        version: manifest.package.version.clone(),
        semantic_contract_digest: manifest.capability_request_digest().expect("digest"),
        capabilities: vec![GrantedCapability {
            name: capability.name,
            action: capability.action,
        }],
        permissions: RuntimePermissionRequest::default(),
        risk_level: HostRiskLevel::High,
        auto_invocable: false,
        requires_confirmation: true,
        approval_source: ApprovalSource::Operator,
        approved_at_unix: 1,
    };
    grant.validate_against(&manifest).expect("valid subset");
    let encoded = serde_json::to_vec(&grant).expect("serialize grant");
    let decoded: HostPolicyGrant = serde_json::from_slice(&encoded).expect("deserialize grant");
    assert_eq!(
        grant.digest(&manifest).expect("grant digest"),
        decoded.digest(&manifest).expect("decoded grant digest")
    );

    let original_semantic_digest = grant.semantic_contract_digest.clone();
    grant.semantic_contract_digest = "0".repeat(64);
    assert_eq!(
        grant
            .validate_against(&manifest)
            .expect_err("semantic drift rejected")
            .code,
        "policy_grant_semantic_digest_mismatch"
    );
    grant.semantic_contract_digest = original_semantic_digest;
    grant.permissions.privilege_escalation = true;
    assert_eq!(
        grant
            .validate_against(&manifest)
            .expect_err("unrequested grant rejected")
            .code,
        "policy_grant_permission_not_requested"
    );
}

#[test]
fn admission_state_requires_matching_policy_evidence() {
    let pending = AdmissionReceipt {
        schema_version: crate::admission::ADMISSION_RECEIPT_SCHEMA_VERSION,
        skill_name: "sample_weather".to_string(),
        version: "0.1.0".to_string(),
        package_digest: "1".repeat(64),
        manifest_digest: "2".repeat(64),
        artifact_set_digest: "3".repeat(64),
        install_receipt_digest: "4".repeat(64),
        semantic_contract_digest: "5".repeat(64),
        granted_policy_digest: None,
        registry_generation: 1,
        platform: crate::HostPlatform::current(),
        state: AdmissionState::AwaitingPolicyApproval,
        approval_source: None,
        approved_at_unix: None,
        admitted_at_unix: 1,
    };
    pending.validate().expect("pending is valid");
    let digest = pending.digest().expect("pending digest");
    let decoded: AdmissionReceipt =
        serde_json::from_slice(&serde_json::to_vec(&pending).expect("encode admission"))
            .expect("decode admission");
    assert_eq!(digest, decoded.digest().expect("decoded admission digest"));
    let mut enabled_without_grant = pending;
    enabled_without_grant.state = AdmissionState::Enabled;
    assert_eq!(
        enabled_without_grant
            .validate()
            .expect_err("enabled requires grant")
            .code,
        "admission_receipt_grant_missing"
    );

    let mut partial_grant = enabled_without_grant;
    partial_grant.granted_policy_digest = Some("6".repeat(64));
    assert_eq!(
        partial_grant
            .validate()
            .expect_err("partial grant evidence is rejected")
            .code,
        "admission_receipt_grant_incomplete"
    );

    let tombstone_without_grant = AdmissionReceipt {
        state: AdmissionState::Tombstoned,
        granted_policy_digest: None,
        approval_source: None,
        approved_at_unix: None,
        ..partial_grant
    };
    tombstone_without_grant
        .validate()
        .expect("an unapproved install can still be tombstoned");
}
