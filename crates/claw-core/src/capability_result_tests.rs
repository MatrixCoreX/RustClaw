use serde_json::json;

use super::{
    ArtifactRef, CapabilityDeliveryIntent, CapabilityResultEnvelope, CapabilityResultStatus,
    CapabilityResultValidationError, Continuation, ContinuationKind, EvidenceRef, StructuredError,
    CAPABILITY_RESULT_SCHEMA_VERSION,
};

#[test]
fn ok_envelope_uses_model_synthesis_without_domain_tokens() {
    let mut envelope = CapabilityResultEnvelope::ok(
        "filesystem.list",
        Some("list".to_string()),
        json!({"entries": ["README.md"]}),
    );
    envelope.evidence.push(EvidenceRef {
        id: "step_1".to_string(),
        source: "filesystem.list".to_string(),
        locator: Some("workspace".to_string()),
        digest: None,
        metadata: json!({"trusted": true}),
    });

    envelope.validate().unwrap();
    assert_eq!(
        envelope.delivery.intent,
        CapabilityDeliveryIntent::ModelSynthesis
    );
    assert_eq!(envelope.status, CapabilityResultStatus::Ok);
    assert_eq!(envelope.schema_version, CAPABILITY_RESULT_SCHEMA_VERSION);
}

#[test]
fn waiting_and_needs_user_require_machine_continuations() {
    let mut envelope =
        CapabilityResultEnvelope::ok("video.generate", Some("poll".to_string()), json!({}));
    envelope.status = CapabilityResultStatus::Waiting;
    assert_eq!(
        envelope.validate(),
        Err(CapabilityResultValidationError::MissingContinuation)
    );

    envelope.continuation = Some(Continuation {
        kind: ContinuationKind::Poll,
        reference: Some("job:123".to_string()),
        poll_after_ms: Some(1_000),
        state: json!({"status": "pending"}),
    });
    envelope.validate().unwrap();
}

#[test]
fn error_contract_rejects_prose_codes() {
    let envelope = CapabilityResultEnvelope::failed(
        "filesystem.read",
        Some("read".to_string()),
        StructuredError {
            code: "permission denied".to_string(),
            message_key: "capability.permission_denied".to_string(),
            retryable: false,
            details: json!({}),
        },
    );
    assert_eq!(
        envelope.validate(),
        Err(CapabilityResultValidationError::InvalidErrorCode)
    );
}

#[test]
fn artifact_requires_a_stable_address() {
    let mut envelope =
        CapabilityResultEnvelope::ok("document.generate", Some("generate".to_string()), json!({}));
    envelope.artifacts.push(ArtifactRef {
        id: None,
        path: None,
        uri: None,
        media_type: Some("application/pdf".to_string()),
        sha256: None,
        metadata: json!({}),
    });
    assert_eq!(
        envelope.validate(),
        Err(CapabilityResultValidationError::UnaddressableArtifact)
    );
}

#[test]
fn duplicate_evidence_ids_are_rejected() {
    let mut envelope =
        CapabilityResultEnvelope::ok("system.inspect", Some("status".to_string()), json!({}));
    for _ in 0..2 {
        envelope.evidence.push(EvidenceRef {
            id: "step_2".to_string(),
            source: "system.inspect".to_string(),
            locator: None,
            digest: None,
            metadata: json!({}),
        });
    }
    assert_eq!(
        envelope.validate(),
        Err(CapabilityResultValidationError::DuplicateEvidenceRef)
    );
}
