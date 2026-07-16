use super::*;

#[test]
fn sse_parser_handles_comments_multiple_frames_and_terminal_stop() {
    let input = concat!(
        ": heartbeat\n\n",
        "id: 1\nevent: tool_started\ndata: {\"seq\":1,\"event_kind\":\"tool_started\",\"payload\":{}}\n\n",
        "id: 2\nevent: task_final\ndata: {\"seq\":2,\"event_kind\":\"task_final\",\"payload\":{}}\n\n",
        "id: 3\ndata: {\"seq\":3,\"event_kind\":\"after_final\"}\n\n",
    );
    let mut seen = Vec::new();
    consume_sse(std::io::Cursor::new(input), &mut |event| {
        seen.push(event["seq"].as_u64().unwrap());
        Ok(event["event_kind"] != "task_final")
    })
    .unwrap();
    assert_eq!(seen, vec![1, 2]);
}

#[test]
fn event_follow_state_uses_machine_fields() {
    let terminal = serde_json::json!({"event_type": "task_final", "payload": {}});
    assert!(task_event_is_terminal(&terminal));
    assert!(!task_event_is_background(&terminal));

    let background = serde_json::json!({
        "event_type": "task_lifecycle",
        "payload": {"execution_state": "needs_user"}
    });
    assert!(task_event_is_background(&background));
    assert!(!task_event_is_terminal(&background));

    let running = serde_json::json!({
        "event_type": "tool_started",
        "payload": {"state": "running"}
    });
    assert!(!task_event_is_terminal(&running));
    assert!(!task_event_is_background(&running));
    assert_eq!(task_event_seq(&serde_json::json!({"seq": 41})), Some(41));
}

#[test]
fn event_stream_status_classifies_only_unsupported_endpoints_as_fallback() {
    for status in [
        StatusCode::NOT_FOUND,
        StatusCode::METHOD_NOT_ALLOWED,
        StatusCode::NOT_ACCEPTABLE,
        StatusCode::NOT_IMPLEMENTED,
    ] {
        let error: anyhow::Error = TaskEventHttpStatusError {
            status,
            body: String::new(),
        }
        .into();
        assert!(task_event_stream_is_unavailable(&error));
        assert!(task_event_stream_has_http_status(&error));
    }

    let unauthorized: anyhow::Error = TaskEventHttpStatusError {
        status: StatusCode::UNAUTHORIZED,
        body: String::new(),
    }
    .into();
    assert!(!task_event_stream_is_unavailable(&unauthorized));
    assert!(task_event_stream_has_http_status(&unauthorized));
}

#[test]
fn event_stream_read_timeout_is_classified_from_real_transport() {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stream server");
    let address = listener.local_addr().expect("stream server address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept stream request");
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("read request header");
            if line == "\r\n" || line.is_empty() {
                break;
            }
        }
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
        )
        .expect("write stream headers");
        stream.flush().expect("flush stream headers");
        thread::sleep(Duration::from_millis(300));
    });

    let error = follow_task_events_with_timeout(
        &format!("http://{address}"),
        "test-key",
        "task-timeout",
        0,
        Some(Duration::from_millis(100)),
        |_| Ok(true),
    )
    .expect_err("stream should time out");
    server.join().expect("stream server");

    assert!(task_event_stream_timed_out(&error), "{error:#}");
}
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
                                "requires_idempotency_guard": true,
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
    assert_eq!(
        events[0]
            .fields
            .get("requires_idempotency_guard")
            .map(String::as_str),
        Some("true")
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
fn event_lines_include_coding_evidence_machine_fields() {
    let data = json!({
        "result_json": {
            "task_journal": {
                "trace": {
                    "event_stream": [
                        {
                            "seq": 5,
                            "event_type": "coding_evidence",
                            "payload": {
                                "schema_version": 1,
                                "evidence_ref": "coding_evidence:summary",
                                "changed_file_count": 1,
                                "command_count": 2,
                                "verification_command_count": 2,
                                "test_count": 1,
                                "diff_summary_count": 1,
                                "failure_count": 1,
                                "verification_status": "failed",
                                "verification_failure_kind_count": 1,
                                "retry_count": 1,
                                "unverified_risk": "tests_not_observed"
                            }
                        }
                    ]
                }
            }
        }
    });

    let events = task_event_lines(&data);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "coding_evidence");
    assert_eq!(
        events[0].fields.get("evidence_ref").map(String::as_str),
        Some("coding_evidence:summary")
    );
    assert_eq!(
        events[0]
            .fields
            .get("changed_file_count")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        events[0].fields.get("test_count").map(String::as_str),
        Some("1")
    );
    assert_eq!(
        events[0]
            .fields
            .get("verification_command_count")
            .map(String::as_str),
        Some("2")
    );
    assert_eq!(
        events[0].fields.get("retry_count").map(String::as_str),
        Some("1")
    );
    assert_eq!(
        events[0]
            .fields
            .get("verification_status")
            .map(String::as_str),
        Some("failed")
    );
    assert_eq!(
        events[0]
            .fields
            .get("verification_failure_kind_count")
            .map(String::as_str),
        Some("1")
    );
    assert!(events[0]
        .line
        .contains("unverified_risk=tests_not_observed"));
}

#[test]
fn event_lines_include_coding_checkpoint_machine_fields() {
    let data = json!({
        "result_json": {
            "task_journal": {
                "trace": {
                    "event_stream": [
                        {
                            "seq": 6,
                            "event_type": "coding_checkpoint",
                            "payload": {
                                "schema_version": 1,
                                "checkpoint_kind": "verification_command",
                                "checkpoint_ref": "coding_checkpoint:verification_command:1",
                                "evidence_ref": "coding_checkpoint:verification_command:1",
                                "command_index": 1,
                                "verification_command": "cargo test -p clawd",
                                "verification_command_count": 2,
                                "verification_status": "failed",
                                "verification_failure_kind_count": 1
                            }
                        }
                    ]
                }
            }
        }
    });

    let events = task_event_lines(&data);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "coding_checkpoint");
    assert_eq!(
        events[0].fields.get("checkpoint_kind").map(String::as_str),
        Some("verification_command")
    );
    assert_eq!(
        events[0].fields.get("command_index").map(String::as_str),
        Some("1")
    );
    assert_eq!(
        events[0]
            .fields
            .get("verification_command")
            .map(String::as_str),
        Some("cargo test -p clawd")
    );
    assert!(events[0]
        .line
        .contains("checkpoint_ref=coding_checkpoint:verification_command:1"));
}

#[test]
fn event_lines_include_workspace_patch_and_rewind_fields() {
    let data = json!({
        "result_json": {
            "task_journal": {
                "trace": {
                    "event_stream": [{
                        "seq": 8,
                        "event_type": "tool_finished",
                        "payload": {
                            "status": "ok",
                            "checkpoint_id": "patch_checkpoint_1",
                            "patch_id": "sha256:patch-1",
                            "mutation_id": "sha256:mutation-1",
                            "compensates_checkpoint_id": "mutation_checkpoint_1",
                            "compensates_mutation_id": "sha256:mutation-0",
                            "target_path": "src/lib.rs",
                            "isolation_root": "workspace://current",
                            "reversible": true,
                            "additions": 4,
                            "deletions": 2,
                            "changed_hunks": 2
                        }
                    }]
                }
            }
        }
    });

    let events = task_event_lines(&data);

    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].fields.get("patch_id").map(String::as_str),
        Some("sha256:patch-1")
    );
    assert_eq!(
        events[0].fields.get("reversible").map(String::as_str),
        Some("true")
    );
    assert_eq!(
        events[0].fields.get("mutation_id").map(String::as_str),
        Some("sha256:mutation-1")
    );
    assert_eq!(
        events[0]
            .fields
            .get("compensates_mutation_id")
            .map(String::as_str),
        Some("sha256:mutation-0")
    );
    assert!(events[0].line.contains("checkpoint_id=patch_checkpoint_1"));
    assert!(events[0].line.contains("changed_hunks=2"));
    let filters = EventFilters::from_parts(&[], Some("patch_checkpoint_1"), None, None, None);
    assert!(filters.matches(&events[0]));
}

#[test]
fn event_lines_expose_untracked_shell_reversibility() {
    let data = json!({
        "result_json": {"task_journal": {"trace": {"event_stream": [{
            "seq": 9,
            "event_type": "tool_finished",
            "payload": {
                "skill": "run_cmd",
                "status": "ok",
                "reversible": false,
                "reversibility_status": "not_rewindable",
                "reversibility_reason_code": "shell_side_effects_not_tracked"
            }
        }]}}}
    });

    let events = task_event_lines(&data);
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].fields.get("reversible").map(String::as_str),
        Some("false")
    );
    assert_eq!(
        events[0]
            .fields
            .get("reversibility_reason_code")
            .map(String::as_str),
        Some("shell_side_effects_not_tracked")
    );
}

#[test]
fn event_lines_include_coding_task_contract_machine_fields() {
    let data = json!({
        "result_json": {
            "task_journal": {
                "trace": {
                    "event_stream": [
                        {
                            "seq": 7,
                            "event_type": "coding_task_contract",
                            "payload": {
                                "schema_version": 1,
                                "contract_ref": "coding_task_contract:summary",
                                "files_read_count": 1,
                                "files_changed_count": 1,
                                "commands_run_count": 2,
                                "tests_run_count": 1,
                                "verification_command_count": 2,
                                "verification_status": "verified",
                                "retry_count": 1
                            }
                        }
                    ]
                }
            }
        }
    });

    let events = task_event_lines(&data);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "coding_task_contract");
    assert_eq!(
        events[0].fields.get("contract_ref").map(String::as_str),
        Some("coding_task_contract:summary")
    );
    assert_eq!(
        events[0].fields.get("files_read_count").map(String::as_str),
        Some("1")
    );
    assert_eq!(
        events[0]
            .fields
            .get("files_changed_count")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        events[0]
            .fields
            .get("commands_run_count")
            .map(String::as_str),
        Some("2")
    );
    assert_eq!(
        events[0].fields.get("tests_run_count").map(String::as_str),
        Some("1")
    );
    assert!(events[0]
        .line
        .contains("contract_ref=coding_task_contract:summary"));
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
