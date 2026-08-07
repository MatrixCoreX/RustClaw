use serde_json::json;

use super::{
    ArtifactRef, ArtifactVisibility, CapabilityDeliveryIntent, CapabilityResultEnvelope,
    CapabilityResultStatus, CapabilityResultValidationError, Continuation, ContinuationKind,
    EvidenceRef, ResultCompleteness, RetryDirective, StructuredError,
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
        artifact_ref: None,
        id: None,
        path: None,
        uri: None,
        media_type: Some("application/pdf".to_string()),
        filename: None,
        artifact_role: None,
        size_bytes: None,
        sha256: None,
        visibility: None,
        owner_task_id: None,
        producer: None,
        lease: None,
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
    envelope.completeness = Some(ResultCompleteness::partial(
        "page_has_more",
        Some(20),
        None,
        false,
    ));
    envelope.continuation = Some(Continuation {
        kind: ContinuationKind::Opaque,
        reference: Some("cursor:20".to_string()),
        poll_after_ms: None,
        state: json!({"snapshot_sha256": "abc123"}),
    });
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
    assert!(envelope.completeness.is_none());
    assert!(!envelope.truncated);
    assert_eq!(envelope.provenance, json!({}));
    assert!(envelope.retry.is_none());
    assert!(envelope.effect.is_none());
    assert_eq!(envelope.verification, json!({}));
}

#[test]
fn partial_result_requires_a_reason_and_safe_recovery() {
    let mut envelope = CapabilityResultEnvelope::ok(
        "filesystem.search",
        Some("continue".to_string()),
        json!({"entries": []}),
    );
    envelope.completeness = Some(ResultCompleteness::partial(
        "resource_checkpoint",
        Some(200),
        None,
        false,
    ));
    assert_eq!(
        envelope.validate(),
        Err(CapabilityResultValidationError::MissingPartialRecovery)
    );

    envelope.continuation = Some(Continuation {
        kind: ContinuationKind::Opaque,
        reference: Some("snapshot:next".to_string()),
        poll_after_ms: None,
        state: json!({"snapshot_sha256": "abc123"}),
    });
    envelope.validate().unwrap();
}

#[test]
fn complete_result_rejects_partial_reason() {
    let mut envelope =
        CapabilityResultEnvelope::ok("filesystem.read", Some("read".to_string()), json!({}));
    let mut completeness = ResultCompleteness::complete(Some(1));
    completeness.reason_code = Some("partial_deadline".to_string());
    envelope.completeness = Some(completeness);
    assert_eq!(
        envelope.validate(),
        Err(CapabilityResultValidationError::InvalidCompleteness)
    );
}

#[test]
fn status_continuation_delivery_and_artifact_consistency_are_enforced() {
    let mut waiting = CapabilityResultEnvelope::ok("video.generate", None, json!({}));
    waiting.status = CapabilityResultStatus::Waiting;
    waiting.continuation = Some(Continuation {
        kind: ContinuationKind::AwaitUser,
        reference: None,
        poll_after_ms: None,
        state: json!({}),
    });
    assert_eq!(
        waiting.validate(),
        Err(CapabilityResultValidationError::InvalidContinuation)
    );

    let mut artifact_delivery = CapabilityResultEnvelope::ok("document.generate", None, json!({}));
    artifact_delivery.delivery.intent = CapabilityDeliveryIntent::Artifact;
    assert_eq!(
        artifact_delivery.validate(),
        Err(CapabilityResultValidationError::MissingDeliveryArtifact)
    );

    let mut artifact = CapabilityResultEnvelope::ok("document.generate", None, json!({}));
    artifact.artifacts.push(ArtifactRef {
        artifact_ref: None,
        id: Some("not a machine id".to_string()),
        path: Some("report.pdf".to_string()),
        uri: None,
        media_type: Some("application/pdf".to_string()),
        filename: None,
        artifact_role: None,
        size_bytes: None,
        sha256: None,
        visibility: None,
        owner_task_id: None,
        producer: None,
        lease: None,
        metadata: json!({}),
    });
    assert_eq!(
        artifact.validate(),
        Err(CapabilityResultValidationError::InvalidArtifactRef)
    );
}

#[test]
fn canonical_evidence_identity_is_stable_and_content_addressed() {
    let result = CapabilityResultEnvelope::ok(
        "filesystem.read_text",
        Some("read".to_string()),
        json!({"content": "hello", "complete": true}),
    );
    let first = result.canonical_evidence_identity();
    let second = result.canonical_evidence_identity();
    assert_eq!(first, second);
    assert!(first.evidence_id.starts_with("evidence:capability_result:"));
    assert_eq!(first.sha256.len(), 64);
    assert!(first.size_bytes > 0);

    let changed = CapabilityResultEnvelope::ok(
        "filesystem.read_text",
        Some("read".to_string()),
        json!({"content": "changed", "complete": true}),
    );
    assert_ne!(
        first.evidence_id,
        changed.canonical_evidence_identity().evidence_id
    );
}

#[test]
fn canonical_task_artifact_reference_requires_matching_owner_and_digest() {
    let mut envelope = CapabilityResultEnvelope::ok("media.inspect", None, json!({}));
    envelope.artifacts.push(ArtifactRef {
        artifact_ref: Some("artifact:task/task-1/a_deadbeef".to_string()),
        id: Some("a_deadbeef".to_string()),
        path: Some("/workspace/file.bin".to_string()),
        uri: None,
        media_type: Some("application/octet-stream".to_string()),
        filename: Some("file.bin".to_string()),
        artifact_role: Some("input_media".to_string()),
        size_bytes: Some(12),
        sha256: Some("a".repeat(64)),
        visibility: Some(ArtifactVisibility::InternalProcessing),
        owner_task_id: Some("task-2".to_string()),
        producer: Some(json!({"capability": "media.inspect"})),
        lease: Some(json!({"kind": "task_lifecycle"})),
        metadata: json!({}),
    });

    assert_eq!(
        envelope.validate(),
        Err(CapabilityResultValidationError::ArtifactOwnershipMismatch)
    );
    envelope.artifacts[0].owner_task_id = Some("task-1".to_string());
    assert!(envelope.validate().is_ok());
    envelope.artifacts[0].sha256 = Some("bad-digest".to_string());
    assert_eq!(
        envelope.validate(),
        Err(CapabilityResultValidationError::InvalidArtifactRef)
    );
}
