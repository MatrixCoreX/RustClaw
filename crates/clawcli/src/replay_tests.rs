use super::{
    replay_bundle_json, replay_diff_summary, replay_run_summary, replay_view_json, run_diff,
    run_run, validate_replay_bundle,
};

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
            "status": "succeeded",
            "route_gate_kind": "execute",
            "steps": [
                {
                    "action_type": "call_tool",
                    "skill": "run_cmd",
                    "action": "run",
                    "status": "ok",
                    "tool_result": {
                        "skill": "run_cmd",
                        "status_code": "ok",
                        "exit_code": 0
                    },
                    "answer_verifier": {
                        "status_code": "passed",
                        "verdict": "pass"
                    },
                    "permission_decision": {
                        "decision": "allowed",
                        "risk_level": "low",
                        "action_effect": "read_only"
                    }
                }
            ]
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
    assert_eq!(summary["coverage"]["event_types"][0], "task_completed");
    assert_eq!(summary["coverage"]["has_task_checkpoint"], false);
    assert_eq!(
        summary["execution_replay"]["strategy"],
        "recorded_outputs_first"
    );
    assert_eq!(summary["execution_replay"]["live_provider"], false);
    assert_eq!(summary["execution_replay"]["live_tool_invocations"], false);
    assert_eq!(summary["execution_replay"]["provider_call_count"], 0);
    assert_eq!(summary["execution_replay"]["tool_invocation_count"], 0);
    assert!(summary["execution_replay"]["step_count"]
        .as_u64()
        .is_some_and(|count| count >= 4));
    assert_eq!(
        summary["permission_summary"][0]["permission_decision"]["decision"],
        "allowed"
    );
}

#[test]
fn replay_view_json_filters_llm_tools_and_checkpoints() {
    let bundle = serde_json::json!({
        "schema_version": 1,
        "bundle_kind": "rustclaw_task_replay",
        "task_id": "task-replay-view",
        "status": "running",
        "lifecycle_state": "background",
        "task": {
            "steps": [
                {
                    "step_id": "step_1",
                    "skill": "run_cmd",
                    "status": "ok",
                    "tool_result": {
                        "skill": "run_cmd",
                        "status_code": "ok",
                        "exit_code": 0
                    }
                }
            ]
        },
        "events": [
            {
                "event_type": "provider_call",
                "line": "type=provider_call prompt_label=planner",
                "fields": {
                    "prompt_label": "planner",
                    "llm_call_count": "1"
                }
            },
            {
                "event_type": "checkpoint_created",
                "line": "type=checkpoint_created checkpoint_id=ckpt-1",
                "fields": {
                    "checkpoint_id": "ckpt-1"
                }
            },
            {
                "event_type": "task_progress",
                "fields": {
                    "status": "running"
                }
            }
        ]
    });

    let llm = replay_view_json(&bundle, "llm").expect("llm view");
    let tools = replay_view_json(&bundle, "tools").expect("tools view");
    let checkpoints = replay_view_json(&bundle, "checkpoints").expect("checkpoint view");

    assert_eq!(llm["view"], "llm");
    assert_eq!(llm["item_count"], 1);
    assert_eq!(llm["items"][0]["event_type"], "provider_call");
    assert_eq!(tools["view"], "tools");
    assert!(tools["item_count"].as_u64().is_some_and(|count| count >= 1));
    assert!(tools["items"]
        .as_array()
        .expect("tool items")
        .iter()
        .any(|item| item["skill"] == "run_cmd" || item["tool_result"]["skill"] == "run_cmd"));
    assert_eq!(checkpoints["view"], "checkpoints");
    assert_eq!(checkpoints["item_count"], 1);
    assert_eq!(checkpoints["items"][0]["fields"]["checkpoint_id"], "ckpt-1");
}

#[test]
fn replay_run_summary_reports_failing_task_fixture_coverage() {
    let fixtures = vec![
        (
            replay_fixture_bundle(
                "task-planner-repair",
                serde_json::json!({
                    "status": "failed",
                    "task_checkpoint": {
                        "checkpoint_id": "ckpt-repair",
                        "repair_signal": {
                            "status_code": "missing_required_evidence",
                            "repair_class": "planner_repair"
                        }
                    }
                }),
                "planner_repair",
            ),
            "has_repair_signal",
            "planner_repair",
        ),
        (
            replay_fixture_bundle(
                "task-async-poll",
                serde_json::json!({
                    "status": "failed",
                    "task_checkpoint": {
                        "checkpoint_id": "ckpt-async",
                        "pending_async_job": {
                            "job_id": "job-async",
                            "status": "expired",
                            "poll_after_seconds": 0,
                            "expires_at": 100,
                            "cancel_ref": "local_process:/tmp/job-async",
                            "message_key": "clawd.task.async_job_expired"
                        }
                    },
                    "task_lifecycle": {
                        "state": "failed",
                        "terminal_reason": "async_job_expired"
                    }
                }),
                "async_poll_failed",
            ),
            "has_pending_async_job",
            "async_poll_failed",
        ),
        (
            replay_fixture_bundle(
                "task-lease-recovery",
                serde_json::json!({
                    "status": "failed",
                    "task_lifecycle": {
                        "state": "failed",
                        "resume_claim": {
                            "checkpoint_id": "ckpt-lease",
                            "owner": "worker-a",
                            "recovery_reason": "worker_lease_expired"
                        }
                    }
                }),
                "lease_recovery_failed",
            ),
            "has_resume_claim",
            "lease_recovery_failed",
        ),
        (
            replay_fixture_bundle(
                "task-subagent-aggregation",
                serde_json::json!({
                    "status": "failed",
                    "result_json": {
                        "child_results": [
                            {
                                "role": "explorer",
                                "status": "failed",
                                "error_code": "child_timeout"
                            }
                        ],
                        "aggregation": {
                            "finding_refs": ["subagent-batch:1"]
                        }
                    }
                }),
                "subagent_aggregation_failed",
            ),
            "has_subagent_results",
            "subagent_aggregation_failed",
        ),
    ];

    for (bundle, coverage_key, event_type) in fixtures {
        validate_replay_bundle(&bundle).expect("valid replay fixture");
        let summary = replay_run_summary(&bundle);

        assert_eq!(summary["replay_mode"], "recorded_only");
        assert_eq!(summary["live_provider"], false);
        assert_eq!(summary["status"], "failed");
        assert_eq!(summary["coverage"][coverage_key], true);
        assert!(summary["coverage"]["event_types"]
            .as_array()
            .expect("event type array")
            .contains(&serde_json::json!(event_type)));
    }
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
            "boundary_context": {
                "route_gate_kind": "execute",
                "decision_envelope": {
                    "semantic_authority": "planner_loop",
                    "decision": "call_capability",
                    "capability_ref": "fs.read"
                }
            },
            "result_json": {
                "task_journal": {
                    "trace": {
                        "step_results": [
                            {
                                "action_type": "call_capability",
                                "capability": "fs.read",
                                "action": "read_text_range",
                                "status": "ok",
                                "permission_decision": {
                                    "decision": "allowed",
                                    "risk_level": "low"
                                }
                            }
                        ]
                    },
                    "summary": {
                        "answer_verifier": {
                            "verdict": "pass",
                            "status_code": "verified"
                        }
                    }
                },
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
            "boundary_context": {
                "route_gate_kind": "execute",
                "decision_envelope": {
                    "semantic_authority": "planner_loop",
                    "decision": "clarify",
                    "capability_ref": "clarify"
                }
            },
            "result_json": {
                "task_journal": {
                    "trace": {
                        "step_results": [
                            {
                                "action_type": "call_capability",
                                "capability": "fs.read",
                                "action": "read_text_range",
                                "status": "error",
                                "error_code": "missing_required_argument",
                                "permission_decision": {
                                    "decision": "denied_by_policy",
                                    "risk_level": "high"
                                }
                            }
                        ]
                    },
                    "summary": {
                        "answer_verifier": {
                            "verdict": "fail",
                            "status_code": "missing_required_evidence"
                        }
                    }
                },
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
    assert_eq!(diff["diff"]["route_changed"], true);
    assert_eq!(diff["diff"]["action_sequence_changed"], true);
    assert_eq!(diff["diff"]["tool_result_changed"], true);
    assert_eq!(diff["diff"]["verifier_changed"], true);
    assert_eq!(diff["diff"]["permission_changed"], true);
    assert!(diff["diff_classes"]
        .as_array()
        .expect("diff classes")
        .contains(&serde_json::json!("final_status_changed")));
    assert!(diff["diff_classes"]
        .as_array()
        .expect("diff classes")
        .contains(&serde_json::json!("plan_changed")));
    assert!(diff["diff_classes"]
        .as_array()
        .expect("diff classes")
        .contains(&serde_json::json!("permission_changed")));
    assert_eq!(diff["left"]["artifact_ref_count"], 1);
    assert_eq!(diff["right"]["artifact_ref_count"], 0);
    assert_eq!(
        diff["left"]["route_fingerprint"][0]["decision_envelope"]["decision"],
        "call_capability"
    );
    assert_eq!(
        diff["right"]["action_sequence"][0]["error_code"],
        "missing_required_argument"
    );
    assert_eq!(
        diff["right"]["verifier_summary"][0]["answer_verifier"]["status_code"],
        "missing_required_evidence"
    );
    assert_eq!(
        diff["right"]["permission_summary"][0]["permission_decision"]["decision"],
        "denied_by_policy"
    );
}

#[test]
fn replay_offline_smoke_runs_bundle_and_diff_without_providers() {
    let base_dir = std::env::temp_dir().join(format!(
        "clawcli_replay_offline_smoke_{}_{}",
        std::process::id(),
        unique_suffix()
    ));
    std::fs::create_dir_all(&base_dir).expect("create replay smoke dir");
    let left_path = base_dir.join("left.json");
    let right_path = base_dir.join("right.json");
    let left = replay_fixture_bundle(
        "task-replay-left",
        serde_json::json!({
            "status": "succeeded",
            "task_lifecycle": {
                "state": "succeeded"
            }
        }),
        "task_completed",
    );
    let right = replay_fixture_bundle(
        "task-replay-right",
        serde_json::json!({
            "status": "failed",
            "task_lifecycle": {
                "state": "failed"
            }
        }),
        "task_failed",
    );
    std::fs::write(
        &left_path,
        serde_json::to_vec_pretty(&left).expect("serialize left bundle"),
    )
    .expect("write left bundle");
    std::fs::write(
        &right_path,
        serde_json::to_vec_pretty(&right).expect("serialize right bundle"),
    )
    .expect("write right bundle");

    run_run(&left_path, true, false, "summary").expect("run replay bundle");
    run_diff(&left_path, &right_path, true).expect("diff replay bundles");

    std::fs::remove_dir_all(base_dir).ok();
}

fn replay_fixture_bundle(
    task_id: &str,
    task: serde_json::Value,
    event_type: &str,
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "bundle_kind": "rustclaw_task_replay",
        "task_id": task_id,
        "status": "failed",
        "lifecycle_state": "failed",
        "redaction": {
            "policy": "machine_key_redaction_v1"
        },
        "task": task,
        "events": [
            {
                "event_type": event_type,
                "fields": {
                    "task_id": task_id,
                    "status": "failed"
                }
            }
        ]
    })
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos()
}
