use super::{replay_bundle_json, replay_diff_summary, replay_run_summary, validate_replay_bundle};

#[test]
fn replay_bundle_redacts_secret_and_private_payload_fields() {
    let task = crate::task::TaskStatusView {
        task_id: "task-replay".to_string(),
        status: "failed".to_string(),
        raw_data: serde_json::json!({
            "task_id": "task-replay",
            "status": "failed",
            "user_key": "sk-test_abcdefghijklmnopqrstuvwxyz123456",
            "payload": {
                "text": "private request content"
            },
            "result_json": {
                "error_code": "provider_rate_limited"
            },
            "task_lifecycle": {
                "state": "failed"
            }
        }),
        result_text: None,
        error_text: None,
        events: vec![crate::events::TaskEventLine {
            event_type: "task_failed".to_string(),
            line: "type=task_failed error_code=provider_rate_limited".to_string(),
            fields: std::collections::BTreeMap::from([
                (
                    "api_key".to_string(),
                    "tp-secret-value-abcdefghijklmnopqrstuvwxyz".to_string(),
                ),
                (
                    "error_code".to_string(),
                    "provider_rate_limited".to_string(),
                ),
            ]),
        }],
    };

    let bundle = replay_bundle_json(&task);
    let bundle_text = serde_json::to_string(&bundle).expect("serialize replay bundle");

    assert_eq!(bundle["task"]["user_key"], "<redacted:secret>");
    assert_eq!(
        bundle["task"]["payload"]["text"],
        "<redacted:private_payload>"
    );
    assert_eq!(
        bundle["events"][0]["fields"]["api_key"],
        "<redacted:secret>"
    );
    assert!(bundle_text.contains("provider_rate_limited"));
    assert!(!bundle_text.contains("sk-test_abcdefghijklmnopqrstuvwxyz123456"));
    assert!(!bundle_text.contains("private request content"));
}

#[test]
fn replay_run_summary_is_recorded_only_machine_result() {
    let bundle = serde_json::json!({
        "schema_version": 1,
        "bundle_kind": "rustclaw_task_replay",
        "task_id": "task-replay-summary",
        "status": "succeeded",
        "lifecycle_state": "succeeded",
        "redaction": {
            "policy": "machine_key_redaction_v1"
        },
        "task": {
            "status": "succeeded"
        },
        "events": [
            {
                "event_type": "task_completed"
            }
        ]
    });

    validate_replay_bundle(&bundle).expect("valid replay bundle");
    let summary = replay_run_summary(&bundle);

    assert_eq!(summary["replay_mode"], "recorded_only");
    assert_eq!(summary["live_provider"], false);
    assert_eq!(summary["task_id"], "task-replay-summary");
    assert_eq!(summary["status"], "succeeded");
    assert_eq!(summary["event_count"], 1);
}

#[test]
fn replay_diff_summary_reports_machine_field_changes() {
    let left = serde_json::json!({
        "schema_version": 1,
        "bundle_kind": "rustclaw_task_replay",
        "task_id": "task-left",
        "status": "succeeded",
        "lifecycle_state": "succeeded",
        "task": {
            "result_json": {
                "artifact_refs": [
                    {
                        "ref": "artifact:left"
                    }
                ]
            }
        },
        "events": [
            {
                "event_type": "task_completed"
            }
        ]
    });
    let right = serde_json::json!({
        "schema_version": 1,
        "bundle_kind": "rustclaw_task_replay",
        "task_id": "task-right",
        "status": "failed",
        "lifecycle_state": "failed",
        "task": {
            "result_json": {
                "artifact_refs": []
            }
        },
        "events": []
    });

    let diff = replay_diff_summary(&left, &right);

    assert_eq!(diff["bundle_kind"], "rustclaw_task_replay_diff");
    assert_eq!(diff["changed"], true);
    assert_eq!(diff["diff"]["status_changed"], true);
    assert_eq!(diff["diff"]["lifecycle_changed"], true);
    assert_eq!(diff["diff"]["event_count_changed"], true);
    assert_eq!(diff["diff"]["artifact_count_changed"], true);
    assert_eq!(diff["left"]["artifact_ref_count"], 1);
    assert_eq!(diff["right"]["artifact_ref_count"], 0);
}
