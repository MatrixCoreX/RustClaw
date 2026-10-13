use std::collections::BTreeMap;

use super::*;

fn request() -> CapabilityRequestSet {
    CapabilityRequestSet {
        schema_version: CAPABILITY_REQUEST_SCHEMA_VERSION,
        input_schema: serde_json::json!({"type": "object"}),
        output_schema: serde_json::json!({"type": "object"}),
        permissions: RuntimePermissionRequest {
            network: true,
            credential_refs: vec!["weather_api_key".to_string()],
            ..RuntimePermissionRequest::default()
        },
        artifact_contract: ArtifactContractRequest {
            kinds: vec![ArtifactKindRequest::StructuredData],
            output_fields: vec!["extra.artifacts".to_string()],
        },
        evidence_contract: EvidenceContractRequest {
            required: true,
            selectors: vec!["extra.provider".to_string()],
        },
        config_entry_points: vec![ConfigEntryPointRequest {
            kind: ConfigEntryPointKind::Credential,
            reference: "weather_api_key".to_string(),
            required: true,
        }],
        capabilities: vec![CapabilityActionRequest {
            name: "weather.current".to_string(),
            action: Some("current".to_string()),
            description: None,
            effect: RequestedEffect::External,
            execution_mode: RequestedExecutionMode::SyncShort,
            required: vec!["location".to_string()],
            optional: Vec::new(),
            current_request_evidence: Vec::new(),
            input_roles: BTreeMap::new(),
            timeout_seconds: Some(30),
        }],
    }
}

#[test]
fn current_request_evidence_must_reference_a_declared_argument() {
    let mut valid = request();
    valid.capabilities[0].current_request_evidence = vec!["location".to_string()];
    valid.validate().expect("declared evidence field accepted");

    let mut invalid = request();
    invalid.capabilities[0].current_request_evidence = vec!["undeclared".to_string()];
    assert_eq!(
        invalid
            .validate()
            .expect_err("undeclared evidence field rejected")
            .code,
        "capability_request_current_request_evidence_invalid"
    );
}

#[test]
fn empty_current_request_evidence_preserves_legacy_manifest_digest_surface() {
    let legacy_shape = serde_json::to_value(request()).expect("serialize request");
    assert!(legacy_shape["capabilities"][0]
        .get("current_request_evidence")
        .is_none());

    let mut protected = request();
    protected.capabilities[0].current_request_evidence = vec!["location".to_string()];
    let protected_shape = serde_json::to_value(protected).expect("serialize protected request");
    assert_eq!(
        protected_shape["capabilities"][0]["current_request_evidence"],
        serde_json::json!(["location"])
    );
}

#[test]
fn validates_typed_capability_request() {
    request().validate().expect("valid request");
}

#[test]
fn rejects_duplicate_capability_and_secret_shaped_credential_value() {
    let mut duplicate = request();
    duplicate
        .capabilities
        .push(duplicate.capabilities[0].clone());
    assert_eq!(
        duplicate.validate().expect_err("duplicate rejected").code,
        "capability_request_duplicate"
    );

    let mut secret_value = request();
    secret_value.permissions.credential_refs = vec!["sk-live-secret".to_string()];
    assert_eq!(
        secret_value
            .validate()
            .expect_err("credential values are not references")
            .code,
        "capability_request_argument_invalid"
    );
}
