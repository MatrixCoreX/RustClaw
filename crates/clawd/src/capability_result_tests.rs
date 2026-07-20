use claw_core::capability_result::{
    CapabilityDeliveryIntent, CapabilityResultStatus, ContinuationKind,
};
use serde_json::json;

#[test]
fn successful_result_wraps_json_and_extra_as_untrusted_data() {
    let envelope = super::successful_execution_envelope(
        "fs_basic",
        "step_3",
        &json!({"action": "inventory_dir"}),
        r#"{"entries":["README.md"]}"#,
        Some(&json!({
            "path": ".",
            "api_key": "secret-value",
            "artifacts": [{"path": "report.json", "media_type": "application/json"}]
        })),
    );

    envelope.validate().unwrap();
    assert_eq!(envelope.status, CapabilityResultStatus::Ok);
    assert_eq!(
        envelope.delivery.intent,
        CapabilityDeliveryIntent::ModelSynthesis
    );
    assert_eq!(envelope.evidence[0].id, "step_3");
    assert_eq!(envelope.evidence[0].locator.as_deref(), Some("."));
    assert_eq!(envelope.artifacts[0].path.as_deref(), Some("report.json"));
    assert!(!envelope.data.to_string().contains("secret-value"));
}

#[test]
fn pending_result_becomes_poll_continuation() {
    let envelope = super::successful_execution_envelope(
        "video_generate",
        "step_1",
        &json!({"action": "generate"}),
        "{}",
        Some(&json!({
            "status": "pending",
            "job_id": "job:42",
            "poll_after_seconds": 3
        })),
    );

    assert_eq!(envelope.status, CapabilityResultStatus::Waiting);
    let continuation = envelope.continuation.unwrap();
    assert_eq!(continuation.kind, ContinuationKind::Poll);
    assert_eq!(continuation.reference.as_deref(), Some("job:42"));
    assert_eq!(continuation.poll_after_ms, Some(3_000));
}

#[test]
fn prose_failure_is_data_not_a_routing_signal() {
    let envelope = super::failed_execution_envelope(
        "fs_basic",
        "step_4",
        &json!({"action": "read_range"}),
        "Permission denied while reading a file",
    );

    let error = envelope.error.unwrap();
    assert_eq!(error.code, "capability_execution_failed");
    assert_eq!(error.message_key, "capability_execution_failed");
    assert!(error.details.to_string().contains("Permission denied"));
}

#[test]
fn signed_artifact_url_is_redacted_before_model_synthesis() {
    let envelope = super::successful_execution_envelope(
        "document_generate",
        "step_8",
        &json!({"action": "generate"}),
        "{}",
        Some(&json!({
            "artifacts": [{
                "uri": "https://example.invalid/report?access_token=secret-token-value"
            }]
        })),
    );

    let uri = envelope.artifacts[0].uri.as_deref().unwrap();
    assert!(!uri.contains("secret-token-value"));
    assert!(uri.contains("[REDACTED]"));
}
