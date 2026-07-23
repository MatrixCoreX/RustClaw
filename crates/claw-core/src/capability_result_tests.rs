use serde_json::json;

use super::{
    ArtifactRef, CapabilityDeliveryIntent, CapabilityResultEnvelope, CapabilityResultStatus,
    CapabilityResultValidationError, Continuation, ContinuationKind, EvidenceRef, RetryDirective,
    StructuredError, CAPABILITY_RESULT_SCHEMA_VERSION,
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

#[test]
fn extended_machine_metadata_is_versioned_and_validated() {
    let mut envelope = CapabilityResultEnvelope::ok(
        "filesystem.search",
        Some("grep_text".to_string()),
        json!({}),
    );
    envelope.page = Some(json!({
        "cursor": 0,
        "next_cursor": 20,
        "snapshot_sha256": "abc123"
    }));
    envelope.truncated = true;
    envelope.provenance = json!({
        "source": "runtime_step",
        "content_trust": "untrusted_tool_output"
    });
    envelope.retry = Some(RetryDirective {
        retryable: true,
        class: Some("rate_limited".to_string()),
        after_ms: Some(1_000),
    });
    envelope.effect = Some("observe".to_string());
    envelope.verification = json!({"status": "passed"});

    envelope.validate().unwrap();
    let encoded = serde_json::to_value(&envelope).unwrap();
    assert_eq!(encoded["page"]["next_cursor"], 20);
    assert_eq!(encoded["truncated"], true);
    assert_eq!(encoded["retry"]["class"], "rate_limited");
    assert_eq!(encoded["effect"], "observe");
    assert_eq!(encoded["verification"]["status"], "passed");
}

#[test]
fn legacy_envelopes_deserialize_with_empty_extended_metadata() {
    let envelope: CapabilityResultEnvelope = serde_json::from_value(json!({
        "schema_version": 1,
        "status": "ok",
        "capability": "filesystem.read",
        "action": "read",
        "data": {"path": "README.md"},
        "artifacts": [],
        "evidence": [],
        "delivery": {"intent": "model_synthesis", "constraints": {}}
    }))
    .unwrap();

    envelope.validate().unwrap();
    assert!(envelope.page.is_none());
    assert!(!envelope.truncated);
    assert_eq!(envelope.provenance, json!({}));
    assert!(envelope.retry.is_none());
    assert!(envelope.effect.is_none());
    assert_eq!(envelope.verification, json!({}));
}
