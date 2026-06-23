use super::*;
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn event_filters_match_machine_fields() {
    let data = json!({
        "result_json": {
            "task_journal": {
                "trace": {
                    "event_stream": [
                        {
                            "seq": 1,
                            "event_type": "policy",
                            "payload": {
                                "decision": "deny",
                                "checkpoint_id": "ckpt-1",
                                "child_run_id": "subagent:1:2:test",
                                "async_job": {
                                    "job_id": "job-1",
                                    "provider_job_id": "provider-job-1"
                                }
                            }
                        }
                    ]
                }
            }
        }
    });
    let events = task_event_lines(&data);
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].fields.get("checkpoint_id").map(String::as_str),
        Some("ckpt-1")
    );
    assert_eq!(
        events[0].fields.get("async_job_id").map(String::as_str),
        Some("job-1")
    );

    let filters = EventFilters::from_parts(
        &[String::from("policy")],
        Some("ckpt-1"),
        Some("DENY"),
        Some("subagent:1:2:test"),
        Some("provider-job-1"),
    );
    assert!(filters.matches(&events[0]));
}

#[test]
fn event_filters_reject_mismatched_machine_fields() {
    let event = TaskEventLine {
        event_type: "checkpoint".to_string(),
        line: "type=checkpoint checkpoint_id=ckpt-1".to_string(),
        fields: BTreeMap::from([("checkpoint_id".to_string(), "ckpt-1".to_string())]),
    };
    let filters = EventFilters::from_parts(
        &[String::from("checkpoint")],
        Some("ckpt-2"),
        None,
        None,
        None,
    );
    assert!(!filters.matches(&event));
}
