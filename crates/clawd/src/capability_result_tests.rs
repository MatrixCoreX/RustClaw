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
fn weather_result_preserves_structured_fields_for_generic_synthesis() {
    let output = json!({
        "text": "provider-localized fallback",
        "extra": {
            "action": "query",
            "mode": "current",
            "location": "Beijing",
            "temperature": 25.2,
            "weather_code": "partly_cloudy",
            "weather_code_raw": 3
        }
    });
    let envelope = super::successful_execution_envelope(
        "weather",
        "step_1",
        &json!({"action": "query", "city": "Beijing"}),
        &output.to_string(),
        output.get("extra"),
    );

    assert_eq!(
        envelope.data.pointer("/extra/temperature"),
        Some(&json!(25.2))
    );
    assert_eq!(
        envelope.data.pointer("/extra/weather_code_raw"),
        Some(&json!(3))
    );
    assert_eq!(
        envelope.data.pointer("/output/extra/location"),
        Some(&json!("Beijing"))
    );
    assert_eq!(
        envelope.delivery.intent,
        CapabilityDeliveryIntent::ModelSynthesis
    );
}

#[test]
fn rss_result_preserves_items_and_sources_for_generic_synthesis() {
    let output = json!({
        "text": "machine fallback",
        "extra": {
            "action": "latest",
            "items": [
                {
                    "title": "Release notes",
                    "source_host": "example.invalid",
                    "date": "2026-07-20"
                }
            ],
            "sources_ok": 1,
            "sources_failed": 0
        }
    });
    let envelope = super::successful_execution_envelope(
        "rss_fetch",
        "step_2",
        &json!({"action": "latest", "limit": 1}),
        &output.to_string(),
        output.get("extra"),
    );

    assert_eq!(
        envelope.data.pointer("/extra/items/0/title"),
        Some(&json!("Release notes"))
    );
    assert_eq!(
        envelope.data.pointer("/extra/items/0/source_host"),
        Some(&json!("example.invalid"))
    );
    assert_eq!(
        envelope.delivery.intent,
        CapabilityDeliveryIntent::ModelSynthesis
    );
}

#[test]
fn web_search_result_preserves_candidates_for_generic_synthesis() {
    let output = json!({
        "text": "machine fallback",
        "extra": {
            "action": "search_extract",
            "candidates": [
                {
                    "title": "Async Rust",
                    "source": "example.invalid",
                    "url": "https://example.invalid/async-rust",
                    "snippet": "A structured search result"
                }
            ]
        }
    });
    let envelope = super::successful_execution_envelope(
        "web_search_extract",
        "step_3",
        &json!({"action": "search_extract", "query": "rust async"}),
        &output.to_string(),
        output.get("extra"),
    );

    assert_eq!(
        envelope.data.pointer("/extra/candidates/0/title"),
        Some(&json!("Async Rust"))
    );
    assert_eq!(
        envelope.data.pointer("/extra/candidates/0/source"),
        Some(&json!("example.invalid"))
    );
    assert_eq!(
        envelope.delivery.intent,
        CapabilityDeliveryIntent::ModelSynthesis
    );
}

#[test]
fn archive_pack_result_exposes_generic_artifact_reference() {
    let extra = json!({
        "action": "pack",
        "archive": "/tmp/reports.zip",
        "field_value": {
            "archive": "/tmp/reports.zip",
            "format": "zip",
            "source": "/tmp/reports"
        },
        "artifacts": [{
            "path": "/tmp/reports.zip",
            "metadata": {
                "action": "pack",
                "format": "zip"
            }
        }]
    });
    let envelope = super::successful_execution_envelope(
        "archive_basic",
        "step_4",
        &json!({"action": "pack"}),
        "untrusted fallback",
        Some(&extra),
    );

    assert_eq!(
        envelope.data.pointer("/extra/field_value/archive"),
        Some(&json!("/tmp/reports.zip"))
    );
    assert_eq!(
        envelope.artifacts[0].path.as_deref(),
        Some("/tmp/reports.zip")
    );
}

#[test]
fn git_result_preserves_structured_state_and_subject_fields() {
    let status_extra = json!({
        "action": "status",
        "current_branch": "main",
        "clean": false,
        "changed_count": 2,
        "paths": ["Cargo.toml", "src/main.rs"],
        "field_value": {
            "current_branch": "main",
            "clean": false,
            "changed_count": 2
        }
    });
    let status = super::successful_execution_envelope(
        "git_basic",
        "step_1",
        &json!({"action": "status"}),
        "untrusted fallback",
        Some(&status_extra),
    );
    assert_eq!(
        status.data.pointer("/extra/field_value/current_branch"),
        Some(&json!("main"))
    );
    assert_eq!(
        status.data.pointer("/extra/field_value/clean"),
        Some(&json!(false))
    );

    let log_extra = json!({
        "action": "log",
        "subject": "refactor: simplify delivery",
        "subjects": ["refactor: simplify delivery"],
        "field_value": {
            "subject": "refactor: simplify delivery",
            "commit_count": 1
        }
    });
    let log = super::successful_execution_envelope(
        "git_basic",
        "step_2",
        &json!({"action": "log", "limit": 1}),
        "untrusted fallback",
        Some(&log_extra),
    );
    assert_eq!(
        log.data.pointer("/extra/field_value/subject"),
        Some(&json!("refactor: simplify delivery"))
    );
    assert_eq!(
        log.delivery.intent,
        CapabilityDeliveryIntent::ModelSynthesis
    );
}

#[test]
fn config_key_result_preserves_generic_listing_fields() {
    let extra = json!({
        "action": "structured_keys",
        "exists": true,
        "container_type": "array",
        "count": 2,
        "keys": ["name", "planner_kind"],
        "identity_values": ["fs_basic", "config_basic"]
    });
    let envelope = super::successful_execution_envelope(
        "config_basic",
        "step_1",
        &json!({"action": "list_keys", "path": "configs/skills_registry.toml"}),
        "untrusted fallback",
        Some(&extra),
    );

    assert_eq!(
        envelope.data.pointer("/extra/keys"),
        Some(&json!(["name", "planner_kind"]))
    );
    assert_eq!(
        envelope.data.pointer("/extra/identity_values"),
        Some(&json!(["fs_basic", "config_basic"]))
    );
    assert_eq!(envelope.data.pointer("/extra/count"), Some(&json!(2)));
    assert_eq!(
        envelope.delivery.intent,
        CapabilityDeliveryIntent::ModelSynthesis
    );
}

#[test]
fn config_field_result_preserves_generic_read_fields() {
    let extra = json!({
        "action": "extract_field",
        "field_path": "llm.selected_vendor",
        "exists": true,
        "field_value": "minimax",
        "value": "minimax",
        "value_text": "minimax",
        "value_type": "string"
    });
    let envelope = super::successful_execution_envelope(
        "config_basic",
        "step_1",
        &json!({
            "action": "read_field",
            "path": "configs/config.toml",
            "field_path": "llm.selected_vendor",
        }),
        "untrusted fallback",
        Some(&extra),
    );

    assert_eq!(
        envelope.data.pointer("/extra/field_path"),
        Some(&json!("llm.selected_vendor"))
    );
    assert_eq!(
        envelope.data.pointer("/extra/field_value"),
        Some(&json!("minimax"))
    );
    assert_eq!(
        envelope.data.pointer("/extra/value_type"),
        Some(&json!("string"))
    );
    assert_eq!(
        envelope.delivery.intent,
        CapabilityDeliveryIntent::ModelSynthesis
    );
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
