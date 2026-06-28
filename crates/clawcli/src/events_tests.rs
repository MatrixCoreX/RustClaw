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
fn event_lines_include_task_transition_machine_fields() {
    let data = json!({
        "result_json": {
            "task_journal": {
                "trace": {
                    "event_stream": [
                        {
                            "seq": 1,
                            "event_type": "task_transition",
                            "payload": {
                                "task_id": "task-transition",
                                "transition_index": 0,
                                "transition_ref": "task_transition:1",
                                "evidence_ref": "task_transition:1",
                                "state_from": "executing",
                                "state_to": "finalizing",
                                "reason_code": "agent_loop_ready_to_finalize",
                                "round_no": 2,
                                "at_ms": 1781800001000_i64
                            }
                        }
                    ]
                }
            }
        }
    });

    let events = task_event_lines(&data);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "task_transition");
    assert_eq!(
        events[0].fields.get("task_id").map(String::as_str),
        Some("task-transition")
    );
    assert_eq!(
        events[0].fields.get("state_from").map(String::as_str),
        Some("executing")
    );
    assert_eq!(
        events[0].fields.get("state_to").map(String::as_str),
        Some("finalizing")
    );
    assert_eq!(
        events[0].fields.get("reason_code").map(String::as_str),
        Some("agent_loop_ready_to_finalize")
    );
    assert_eq!(
        events[0].fields.get("transition_ref").map(String::as_str),
        Some("task_transition:1")
    );
    assert_eq!(
        events[0].fields.get("evidence_ref").map(String::as_str),
        Some("task_transition:1")
    );
    assert!(events[0]
        .line
        .contains("reason_code=agent_loop_ready_to_finalize"));
    assert!(events[0].line.contains("evidence_ref=task_transition:1"));
}

#[test]
fn event_lines_include_tool_lifecycle_machine_fields() {
    let data = json!({
        "result_json": {
            "task_journal": {
                "trace": {
                    "event_stream": [
                        {
                            "seq": 2,
                            "event_type": "tool_started",
                            "payload": {
                                "phase": "started",
                                "step_id": "step_1",
                                "step_ref": "step_1",
                                "evidence_ref": "step_1",
                                "skill": "run_cmd",
                                "action_kind": "call_capability",
                                "requested_capability": "terminal.run_command",
                                "started_at": 1781800001000_i64
                            }
                        },
                        {
                            "seq": 3,
                            "event_type": "tool_finished",
                            "payload": {
                                "phase": "finished",
                                "step_id": "step_1",
                                "step_ref": "step_1",
                                "evidence_ref": "step_1",
                                "skill": "run_cmd",
                                "status": "ok",
                                "finished_at": 1781800002000_i64
                            }
                        }
                    ]
                }
            }
        }
    });

    let events = task_event_lines(&data);

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, "tool_started");
    assert_eq!(
        events[0].fields.get("phase").map(String::as_str),
        Some("started")
    );
    assert_eq!(
        events[0].fields.get("step_ref").map(String::as_str),
        Some("step_1")
    );
    assert_eq!(
        events[0].fields.get("evidence_ref").map(String::as_str),
        Some("step_1")
    );
    assert!(events[0].line.contains("started_at=1781800001000"));
    assert_eq!(
        events[1].fields.get("status").map(String::as_str),
        Some("ok")
    );
    assert!(events[1].line.contains("finished_at=1781800002000"));
}

#[test]
fn event_lines_include_checkpoint_machine_fields_and_async_filter() {
    let data = json!({
        "result_json": {
            "task_journal": {
                "trace": {
                    "event_stream": [
                        {
                            "seq": 1,
                            "event_type": "checkpoint_created",
                            "payload": {
                                "checkpoint_id": "ckpt-1",
                                "checkpoint_ref": "task_checkpoint:ckpt-1",
                                "evidence_ref": "task_checkpoint:ckpt-1",
                                "resume_entrypoint": "poll_async_job",
                                "completed_side_effect_count": 1,
                                "pending_async_job_id": "job-1",
                                "poll_ref": "local_process:123",
                                "cancel_ref": "local_process:123",
                                "message_key": "async_job_running"
                            }
                        }
                    ]
                }
            }
        }
    });

    let events = task_event_lines(&data);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "checkpoint_created");
    assert_eq!(
        events[0].fields.get("checkpoint_ref").map(String::as_str),
        Some("task_checkpoint:ckpt-1")
    );
    assert_eq!(
        events[0]
            .fields
            .get("pending_async_job_id")
            .map(String::as_str),
        Some("job-1")
    );
    assert!(events[0].line.contains("message_key=async_job_running"));

    let filters = EventFilters::from_parts(&[], None, None, None, Some("job-1"));
    assert!(filters.matches(&events[0]));
}

#[test]
fn event_lines_include_lifecycle_worker_events() {
    let data = json!({
        "result_json": {
            "task_lifecycle": {
                "state": "failed",
                "worker_events": [
                    {
                        "event_type": "heartbeat_missed",
                        "owner_layer": "worker_runtime",
                        "task_id": "task-worker-stale",
                        "state_from": "running",
                        "state_to": "timeout",
                        "reason_code": "worker_heartbeat_stale",
                        "recovered_at": 1781800002_i64
                    }
                ]
            }
        }
    });

    let events = task_event_lines(&data);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "heartbeat_missed");
    assert_eq!(
        events[0].fields.get("task_id").map(String::as_str),
        Some("task-worker-stale")
    );
    assert_eq!(
        events[0].fields.get("state_from").map(String::as_str),
        Some("running")
    );
    assert_eq!(
        events[0].fields.get("state_to").map(String::as_str),
        Some("timeout")
    );
    assert_eq!(
        events[0].fields.get("reason_code").map(String::as_str),
        Some("worker_heartbeat_stale")
    );
    assert!(events[0].line.contains("recovered_at=1781800002"));
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
