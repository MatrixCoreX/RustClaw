use super::{
    automation_runs_request_payload, coding_review_json, exec_artifact_index_json,
    exec_compact_text_lines, exec_effective_options, exec_exit_class,
    exec_failure_class_from_machine_tokens, exec_summary_json, goal_control_summary_json,
    goal_edit_patch_json, goal_request_payload, goal_status_summary_json, goal_status_text_lines,
    permission_report_json, run_exec, subagent_report_json, task_event_output_lines,
    task_report_json, task_report_text_lines, task_resume_control_summary_json,
    tui_command_from_input, tui_export_json, tui_selected_task_lines, tui_snapshot_json,
    wait_until_matches, watch_progress_json, write_exec_artifacts, ExecExitClass, ExecWaitOutcome,
    TuiCommand,
};

#[test]
fn exec_summary_json_exposes_stable_machine_fields() {
    let task = crate::task::TaskStatusView {
        task_id: "task-exec".to_string(),
        status: "running".to_string(),
        raw_data: serde_json::json!({
            "task_lifecycle": {
                "state": "background",
                "checkpoint_id": "ckpt-exec"
            },
            "changed_files": ["crates/clawcli/src/commands/exec.rs"],
            "result_json": {
                "step_results": [
                    {
                        "step_id": "step_1",
                        "status": "ok",
                        "skill": "run_cmd",
                        "command": "cargo check -p clawcli"
                    }
                ],
                "task_journal": {
                    "trace": {
                        "step_results": [
                            {
                                "artifact_refs": [
                                    {
                                        "ref": "artifact:summary"
                                    }
                                ]
                            }
                        ]
                    }
                }
            }
        }),
        result_text: Some("result-token".to_string()),
        error_text: None,
        events: vec![
            crate::events::TaskEventLine {
                event_type: "checkpoint_created".to_string(),
                line: "type=checkpoint_created checkpoint_id=ckpt-exec".to_string(),
                fields: std::collections::BTreeMap::from([(
                    "checkpoint_id".to_string(),
                    "ckpt-exec".to_string(),
                )]),
            },
            crate::events::TaskEventLine {
                event_type: "provider_call".to_string(),
                line: "type=provider_call prompt_label=planner".to_string(),
                fields: std::collections::BTreeMap::from([
                    ("prompt_label".to_string(), "planner".to_string()),
                    ("llm_call_count".to_string(), "1".to_string()),
                    ("elapsed_ms".to_string(), "120".to_string()),
                    ("prompt_bytes_before_max".to_string(), "4096".to_string()),
                    ("prompt_bytes_after_max".to_string(), "4096".to_string()),
                ]),
            },
        ],
    };

    let summary = exec_summary_json(
        &task,
        ExecWaitOutcome::Background,
        ExecExitClass::Success,
        None,
    );

    assert_eq!(summary["task_id"], "task-exec");
    assert_eq!(summary["status"], "running");
    assert_eq!(summary["lifecycle_state"], "background");
    assert_eq!(summary["outcome"], "background");
    assert_eq!(summary["exit_class"], "success");
    assert_eq!(summary["exit_code"], 0);
    assert_eq!(summary["resume"]["mode"], "new_task");
    assert_eq!(summary["terminal"], false);
    assert_eq!(summary["lifecycle"]["checkpoint_id"], "ckpt-exec");
    assert_eq!(summary["events"][0]["event_type"], "checkpoint_created");
    assert_eq!(summary["events"][0]["fields"]["checkpoint_id"], "ckpt-exec");
    assert_eq!(summary["llm"]["llm_call_count"], 1);
    assert_eq!(summary["llm"]["prompt_bytes_before_max"], 4096);
    assert_eq!(summary["llm"]["budget_health"]["status"], "ok");
    assert_eq!(summary["llm"]["by_prompt"][0]["prompt_label"], "planner");
    assert_eq!(summary["coding"]["changed_file_count"], 1);
    assert_eq!(
        summary["coding"]["state"]["current_phase_hint"],
        "summarize"
    );
    assert_eq!(summary["coding"]["state"]["next_step"], "summarize");
    assert_eq!(
        summary["coding"]["changed_files"][0],
        "crates/clawcli/src/commands/exec.rs"
    );
    assert_eq!(summary["coding"]["verification_command_count"], 1);
    assert_eq!(
        summary["coding"]["verification_commands"][0],
        "cargo check -p clawcli"
    );
    assert_eq!(summary["artifacts"]["ref_count"], 1);
    assert_eq!(summary["artifacts"]["refs"][0]["ref"], "artifact:summary");
}

#[test]
fn task_report_json_exposes_stable_machine_fields() {
    let task = crate::task::TaskStatusView {
        task_id: "task-report".to_string(),
        status: "succeeded".to_string(),
        raw_data: serde_json::json!({
            "execution_state": "completed",
            "user_id": 7,
            "chat_id": 9,
            "task_goal": {
                "goal_id": "goal-report"
            },
            "task_lifecycle": {
                "state": "completed",
                "reason_code": "succeeded"
            },
            "result_json": {
                "task_journal": {
                    "summary": {
                        "task_outcome": {
                            "state": "done",
                            "message_key": "clawd.task.done",
                            "done_conditions": ["tests_pass"],
                            "constraints": [{"scope": "workspace", "writes_allowed": true}],
                            "verification": {"command": "cargo test -p clawd"},
                            "current_progress": ["changed_file_count=1"],
                            "remaining_work": ["summarize"]
                        },
                        "context_budget_report": {
                            "schema_version": 1,
                            "budget_tier": "normal",
                            "included_ref_count": 2,
                            "included_refs": ["goal:goal-report", "file:src/lib.rs"],
                            "excluded_ref_count": 1,
                            "excluded_refs": ["transcript:old"],
                            "char_estimate": 4096,
                            "token_estimate": 1024,
                            "truncation_reason": "budget_limit",
                            "safety_reason": null,
                            "compaction_source": "deterministic"
                        }
                    },
                    "trace": {
                        "contract_matrix": {
                            "final_answer_shape": "generated_file_path_report"
                        },
                        "evidence_coverage": {
                            "missing_evidence": []
                        }
                    }
                },
                "artifact_refs": [
                    {
                        "ref": "artifact:report"
                    }
                ],
                "changed_files": ["src/lib.rs"],
                "final_diff_summary": {
                    "file_count": 1,
                    "summary_code": "update_lib_api",
                    "verification_evidence_refs": ["step:step_1"],
                    "rollback_refs": ["write_file:src/lib.rs"]
                },
                "step_results": [
                    {
                        "step_id": "step_1",
                        "status": "ok",
                        "skill": "run_cmd",
                        "command": "cargo test -p clawd"
                    }
                ]
            }
        }),
        result_text: Some("result-token".to_string()),
        error_text: None,
        events: vec![
            crate::events::TaskEventLine {
                event_type: "task_completed".to_string(),
                line: "seq=1 type=task_completed status=succeeded".to_string(),
                fields: std::collections::BTreeMap::from([(
                    "status".to_string(),
                    "succeeded".to_string(),
                )]),
            },
            crate::events::TaskEventLine {
                event_type: "provider_call".to_string(),
                line: "seq=2 type=provider_call prompt_label=normalizer".to_string(),
                fields: std::collections::BTreeMap::from([
                    ("prompt_label".to_string(), "normalizer".to_string()),
                    ("llm_call_count".to_string(), "1".to_string()),
                    ("elapsed_ms".to_string(), "42".to_string()),
                    ("provider_attempt_count".to_string(), "2".to_string()),
                    ("provider_retry_count".to_string(), "1".to_string()),
                    ("prompt_truncation_count".to_string(), "1".to_string()),
                    ("prompt_bytes_before_max".to_string(), "157037".to_string()),
                    ("prompt_bytes_budget_min".to_string(), "125200".to_string()),
                    ("prompt_bytes_after_max".to_string(), "125180".to_string()),
                    (
                        "prompt_truncated_bytes_total".to_string(),
                        "31857".to_string(),
                    ),
                ]),
            },
        ],
    };

    let report = task_report_json(&task, true);

    assert_eq!(report["report_kind"], "rustclaw_task_report");
    assert_eq!(report["task_id"], "task-report");
    assert_eq!(report["goal_id"], "goal-report");
    assert_eq!(report["session_id"], "user_chat:7:9");
    assert_eq!(report["session"]["user_id"], "7");
    assert_eq!(report["session"]["chat_id"], "9");
    assert_eq!(report["session"]["active_goal_id"], "goal-report");
    assert_eq!(report["status"], "succeeded");
    assert_eq!(report["execution_state"], "completed");
    assert_eq!(report["lifecycle_state"], "completed");
    assert_eq!(report["terminal"], true);
    assert_eq!(report["event_count"], 2);
    assert_eq!(report["events"][0]["event_type"], "task_completed");
    assert_eq!(report["llm"]["provider_call_event_count"], 1);
    assert_eq!(report["llm"]["llm_call_count"], 1);
    assert_eq!(report["llm"]["prompt_truncation_count"], 1);
    assert_eq!(report["llm"]["prompt_bytes_before_max"], 157037);
    assert_eq!(report["llm"]["budget_health"]["status"], "warning");
    assert_eq!(
        report["context_budget"]["source"],
        "task_journal_context_budget_report"
    );
    assert_eq!(report["context_budget"]["included_ref_count"], 2);
    assert_eq!(
        report["context_budget"]["excluded_refs"][0],
        "transcript:old"
    );
    assert_eq!(
        report["llm"]["budget_health"]["warnings"][0],
        "prompt_truncation_count"
    );
    assert_eq!(report["llm"]["by_prompt"][0]["prompt_label"], "normalizer");
    assert_eq!(report["coding"]["changed_file_count"], 1);
    assert_eq!(report["coding"]["changed_files"][0], "src/lib.rs");
    assert_eq!(report["coding"]["verification_command_count"], 1);
    assert_eq!(
        report["coding"]["verification_commands"][0],
        "cargo test -p clawd"
    );
    assert_eq!(report["coding"]["test_count"], 1);
    assert_eq!(report["coding"]["tests"][0], "cargo test -p clawd");
    assert_eq!(report["coding"]["verification_failure_kind_count"], 0);
    assert_eq!(report["coding"]["diff_summary_count"], 1);
    assert_eq!(
        report["coding"]["diff_summaries"][0]["field"],
        "final_diff_summary"
    );
    assert_eq!(
        report["coding"]["diff_summaries"][0]["value"]["summary_code"],
        "update_lib_api"
    );
    assert_eq!(
        report["coding"]["diff_summaries"][0]["normalized"]["file_path"],
        "src/lib.rs"
    );
    assert_eq!(
        report["coding"]["diff_summaries"][0]["normalized"]["change_kind"],
        "modified"
    );
    assert_eq!(
        report["coding"]["diff_summaries"][0]["normalized"]["bounded_hunk_summary"],
        "update_lib_api"
    );
    assert_eq!(
        report["coding"]["diff_summaries"][0]["normalized"]["verification_evidence_refs"][0],
        "step:step_1"
    );
    assert_eq!(
        report["coding"]["diff_summaries"][0]["normalized"]["rollback_refs"][0],
        "write_file:src/lib.rs"
    );
    assert_eq!(report["coding"]["unverified_risk"], serde_json::Value::Null);
    assert_eq!(report["outcome"]["state"], "done");
    assert_eq!(report["outcome"]["message_key"], "clawd.task.done");
    assert_eq!(
        report["outcome"]["final_answer_shape"],
        "generated_file_path_report"
    );
    assert_eq!(report["outcome"]["done_conditions"][0], "tests_pass");
    assert_eq!(report["outcome"]["constraints"][0], "scope=workspace");
    assert_eq!(report["outcome"]["constraints"][1], "writes_allowed=true");
    assert_eq!(
        report["outcome"]["verification"][0],
        "command=cargo test -p clawd"
    );
    assert_eq!(
        report["outcome"]["verification"][1],
        "verification_status=verified"
    );
    assert_eq!(
        report["outcome"]["current_progress"][0],
        "changed_file_count=1"
    );
    assert_eq!(report["outcome"]["remaining_work"][0], "summarize");
    assert_eq!(report["artifacts"]["ref_count"], 1);
    assert_eq!(report["artifacts"]["refs"][0]["ref"], "artifact:report");
}

#[test]
fn task_report_json_prefers_journal_coding_workflow_contract() {
    let task = crate::task::TaskStatusView {
        task_id: "task-coding-workflow-report".to_string(),
        status: "succeeded".to_string(),
        raw_data: serde_json::json!({
            "execution_state": "completed",
            "task_lifecycle": {
                "state": "completed"
            },
            "result_json": {
                "task_journal": {
                    "summary": {
                        "coding_workflow": {
                            "schema_version": 1,
                            "current_phase_hint": "summarize",
                            "next_step": "summarize",
                            "planned_change_count": 1,
                            "planned_changes": ["add library entrypoint"],
                            "diff_ref_count": 1,
                            "diff_refs": ["diff:src/lib.rs:step_1"],
                            "changed_file_count": 1,
                            "changed_files": ["src/lib.rs"],
                            "verification_command_count": 1,
                            "verification_commands": ["cargo test -p clawd"],
                            "verification_status": "verified",
                            "failure_kind_count": 0,
                            "failure_kinds": [],
                            "repair_attempt_count": 0,
                            "repair_attempt_refs": [],
                            "checkpoint_ref_count": 1,
                            "checkpoint_refs": ["coding_checkpoint:verification_command:step_2"],
                            "completed_side_effect_count": 1,
                            "completed_side_effect_refs": ["write_file:src/lib.rs"],
                            "remaining_risks": [],
                            "done_condition_coverage": [
                                {"condition": "changes", "status": "observed"},
                                {"condition": "verification", "status": "verified"}
                            ],
                            "validation_gate": {
                                "schema_version": 1,
                                "gate_status": "satisfied",
                                "can_report_fully_verified": true,
                                "requires_verification": false,
                                "requires_repair": false,
                                "checkpoint_recommended": false,
                                "repair_signal": null
                            }
                        }
                    }
                },
                "changed_files": ["legacy/path.rs"],
                "final_diff_summary": {
                    "summary_code": "legacy_diff"
                },
                "step_results": [
                    {
                        "step_id": "step_2",
                        "status": "ok",
                        "skill": "run_cmd",
                        "command": "cargo test -p clawd"
                    }
                ]
            }
        }),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };

    let report = task_report_json(&task, false);

    assert_eq!(report["coding"]["source"], "task_journal_coding_workflow");
    assert_eq!(report["coding"]["planned_change_count"], 1);
    assert_eq!(
        report["coding"]["planned_changes"][0],
        "add library entrypoint"
    );
    assert_eq!(report["coding"]["diff_ref_count"], 1);
    assert_eq!(report["coding"]["diff_refs"][0], "diff:src/lib.rs:step_1");
    assert_eq!(report["coding"]["changed_file_count"], 1);
    assert_eq!(report["coding"]["changed_files"][0], "src/lib.rs");
    assert_eq!(report["coding"]["verification_command_count"], 1);
    assert_eq!(
        report["coding"]["verification_commands"][0],
        "cargo test -p clawd"
    );
    assert_eq!(report["coding"]["state"]["verification_status"], "verified");
    assert_eq!(report["coding"]["state"]["can_report_fully_verified"], true);
    assert_eq!(
        report["coding"]["validation_gate"]["gate_status"],
        "satisfied"
    );
    assert_eq!(
        report["coding"]["validation_gate"]["can_report_fully_verified"],
        true
    );
    assert_eq!(report["coding"]["state"]["checkpoint_ref_count"], 1);
    assert_eq!(report["coding"]["diff_summary_count"], 1);
    assert_eq!(
        report["coding"]["diff_summaries"][0]["value"]["summary_code"],
        "legacy_diff"
    );
    assert_eq!(
        report["coding"]["done_condition_coverage"][1]["status"],
        "verified"
    );
}

#[test]
fn task_report_json_marks_exceeded_llm_budget_health() {
    let task = crate::task::TaskStatusView {
        task_id: "task-budget-exceeded".to_string(),
        status: "failed".to_string(),
        raw_data: serde_json::json!({
            "execution_state": "completed",
            "task_lifecycle": {
                "state": "completed",
                "reason_code": "provider_final_error"
            }
        }),
        result_text: None,
        error_text: Some("provider_final_error".to_string()),
        events: vec![crate::events::TaskEventLine {
            event_type: "provider_call".to_string(),
            line: "seq=1 type=provider_call prompt_label=planner".to_string(),
            fields: std::collections::BTreeMap::from([
                ("prompt_label".to_string(), "planner".to_string()),
                ("llm_call_count".to_string(), "20".to_string()),
                ("elapsed_ms".to_string(), "950000".to_string()),
                ("provider_retry_count".to_string(), "7".to_string()),
                ("provider_final_error_count".to_string(), "1".to_string()),
                ("prompt_truncation_count".to_string(), "4".to_string()),
                ("prompt_bytes_before_max".to_string(), "800000".to_string()),
                (
                    "prompt_truncated_bytes_total".to_string(),
                    "120000".to_string(),
                ),
            ]),
        }],
    };

    let report = task_report_json(&task, false);
    let exceeded = report["llm"]["budget_health"]["exceeded"]
        .as_array()
        .expect("exceeded tokens");

    assert_eq!(report["llm"]["budget_health"]["status"], "exceeded");
    for token in [
        "llm_call_count",
        "prompt_bytes_before_max",
        "prompt_truncation_count",
        "provider_retry_count",
        "provider_final_error_count",
        "elapsed_ms",
    ] {
        assert!(
            exceeded.iter().any(|value| value == token),
            "missing exceeded token {token}: {exceeded:?}"
        );
    }

    let lines = task_report_text_lines(&task, &report);
    assert!(lines.contains(&"llm_budget_status: exceeded".to_string()));
    assert!(lines.contains(&"llm_budget_exceeded: llm_call_count".to_string()));
    assert!(lines.contains(&"llm_budget_exceeded: provider_final_error_count".to_string()));
}

#[test]
fn coding_review_json_focuses_on_coding_evidence() {
    let task = crate::task::TaskStatusView {
        task_id: "task-review".to_string(),
        status: "succeeded".to_string(),
        raw_data: serde_json::json!({
            "execution_state": "completed",
            "result_json": {
                "changed_files": ["crates/clawcli/src/main.rs"],
                "step_results": [
                    {
                        "step_id": "step_1",
                        "status": "ok",
                        "skill": "run_cmd",
                        "command": "cargo test -p clawcli"
                    }
                ]
            }
        }),
        result_text: Some("visible fallback ignored by review".to_string()),
        error_text: None,
        events: Vec::new(),
    };

    let review = coding_review_json(&task, false);

    assert_eq!(review["report_kind"], "rustclaw_coding_review");
    assert_eq!(review["task_id"], "task-review");
    assert_eq!(review["coding"]["changed_file_count"], 1);
    assert_eq!(review["coding"]["verification_command_count"], 1);
    assert_eq!(review["coding"]["tests"][0], "cargo test -p clawcli");
    assert!(review.get("result_text").is_none());
}

#[test]
fn subagent_report_json_collects_child_results_and_events() {
    let task = crate::task::TaskStatusView {
        task_id: "task-subagents".to_string(),
        status: "running".to_string(),
        raw_data: serde_json::json!({
            "result_json": {
                "child_results": [
                    {
                        "child_run_id": "subagent:1:2:explorer",
                        "subagent_id": "explorer",
                        "status": "succeeded",
                        "result_status": "completed",
                        "role_metadata": {
                            "tool_permission_profile": "read_only"
                        },
                        "timeout_policy": {
                            "timeout_ms": 30000,
                            "source": "agent_guard.subagents.default_timeout_ms"
                        },
                        "outcome_code": "subagent_parallel_readonly_completed",
                        "conflict_count": 1,
                        "failure_isolated": true,
                        "confidence_summary": {
                            "min": 0.72,
                            "max": 0.93
                        },
                        "main_thread_decision": {
                            "decision_status": "needs_conflict_resolution"
                        },
                        "finding_refs": ["finding:1"],
                        "evidence_refs": ["evidence:1"]
                    }
                ]
            }
        }),
        result_text: None,
        error_text: None,
        events: vec![crate::events::TaskEventLine {
            event_type: "subagent".to_string(),
            line: "type=subagent child_run_id=subagent:1:2:verifier".to_string(),
            fields: std::collections::BTreeMap::from([
                (
                    "child_run_id".to_string(),
                    "subagent:1:2:verifier".to_string(),
                ),
                ("subagent_id".to_string(), "verifier".to_string()),
                ("status".to_string(), "succeeded".to_string()),
                (
                    "tool_permission_profile".to_string(),
                    "read_only".to_string(),
                ),
                (
                    "execution_mode".to_string(),
                    "inline_readonly_child_run".to_string(),
                ),
            ]),
        }],
    };

    let report = subagent_report_json(&task);

    assert_eq!(report["report_kind"], "rustclaw_subagent_report");
    assert_eq!(report["task_id"], "task-subagents");
    assert_eq!(report["subagent_count"], 2);
    assert_eq!(
        report["subagents"][0]["child_run_id"],
        "subagent:1:2:explorer"
    );
    assert_eq!(report["subagents"][0]["result_status"], "completed");
    assert_eq!(
        report["subagents"][0]["outcome_code"],
        "subagent_parallel_readonly_completed"
    );
    assert_eq!(report["subagents"][0]["conflict_count"], 1);
    assert_eq!(
        report["subagents"][0]["decision_status"],
        "needs_conflict_resolution"
    );
    assert_eq!(report["subagents"][0]["confidence_min"], 0.72);
    assert_eq!(report["subagents"][0]["confidence_max"], 0.93);
    assert_eq!(report["subagents"][0]["failure_isolated"], true);
    assert_eq!(
        report["subagents"][0]["tool_permission_profile"],
        "read_only"
    );
    assert_eq!(report["subagents"][0]["read_only_enforced"], true);
    assert_eq!(
        report["subagents"][0]["write_isolation_status"],
        "not_supported"
    );
    assert_eq!(report["subagents"][0]["timeout_ms"], 30000);
    assert_eq!(
        report["subagents"][0]["timeout_source"],
        "agent_guard.subagents.default_timeout_ms"
    );
    assert_eq!(report["subagents"][0]["finding_refs"][0], "finding:1");
    assert_eq!(
        report["subagents"][1]["child_run_id"],
        "subagent:1:2:verifier"
    );
    assert_eq!(
        report["subagents"][1]["tool_permission_profile"],
        "read_only"
    );
    assert_eq!(report["subagents"][1]["read_only_enforced"], true);
}

#[test]
fn permission_report_json_collects_structured_decisions() {
    let task = crate::task::TaskStatusView {
        task_id: "task-permission".to_string(),
        status: "failed".to_string(),
        raw_data: serde_json::json!({
            "result_json": {
                "permission_decision": {
                    "decision": "denied_by_policy",
                    "allowed": false,
                    "needs_confirmation": false,
                    "dry_run_required": true,
                    "risk_level": "high",
                    "action_effect": "external_publish",
                    "reason_code": "dry_run_required",
                    "isolation_profile": "local_temp_workspace",
                    "sandbox": {
                        "profile": "workspace_guard",
                        "source": "plan_verifier",
                        "filesystem_write": false
                    }
                },
                "step_results": [
                    {
                        "extra": {
                            "command_policy": {
                                "policy_authority": "contract_matrix",
                                "effect": "filesystem_write",
                                "isolation_profile": "read_only"
                            }
                        }
                    }
                ]
            }
        }),
        result_text: Some("ignored visible fallback".to_string()),
        error_text: None,
        events: Vec::new(),
    };

    let report = permission_report_json(&task);

    assert_eq!(report["report_kind"], "rustclaw_permission_report");
    assert_eq!(report["permission_entry_count"], 2);
    assert_eq!(
        report["permission_entries"][0]["decision"],
        "denied_by_policy"
    );
    assert_eq!(report["permission_entries"][0]["risk_level"], "high");
    assert_eq!(
        report["permission_entries"][0]["isolation_profile"],
        "local_temp_workspace"
    );
    assert_eq!(
        report["permission_entries"][0]["sandbox_profile"],
        "workspace_guard"
    );
    assert_eq!(
        report["permission_entries"][0]["sandbox_source"],
        "plan_verifier"
    );
    assert_eq!(report["permission_entries"][0]["filesystem_write"], false);
    assert_eq!(
        report["permission_entries"][1]["decision"],
        "contract_matrix"
    );
    assert_eq!(
        report["permission_entries"][1]["isolation_profile"],
        "read_only"
    );
    assert!(report.get("result_text").is_none());
}

#[test]
fn tui_snapshot_json_wraps_active_and_selected_task() {
    let active = serde_json::json!({
        "data": {
            "tasks": [
                {
                    "task_id": "task-tui",
                    "status": "running",
                    "execution_state": "background"
                }
            ]
        }
    });
    let selected = crate::task::TaskStatusView {
        task_id: "task-tui".to_string(),
        status: "running".to_string(),
        raw_data: serde_json::json!({
            "task_id": "task-tui",
            "status": "running",
            "execution_state": "background",
            "task_lifecycle": {
                "state": "background",
                "checkpoint_id": "ckpt-tui",
                "resume_due": true,
                "resume_wait_seconds": 0,
                "next_action_kind": "resume_checkpoint"
            },
            "result_json": {
                "changed_files": ["src/lib.rs"],
                "task_journal": {
                    "trace": {
                        "step_results": [
                            {
                                "artifact_refs": [
                                    {
                                        "ref": "artifact:tui"
                                    }
                                ]
                            }
                        ]
                    }
                }
            }
        }),
        result_text: None,
        error_text: None,
        events: vec![crate::events::TaskEventLine {
            event_type: "provider_call".to_string(),
            line: "type=provider_call prompt_label=planner llm_call_count=2".to_string(),
            fields: std::collections::BTreeMap::from([
                ("prompt_label".to_string(), "planner".to_string()),
                ("llm_call_count".to_string(), "2".to_string()),
            ]),
        }],
    };

    let snapshot = tui_snapshot_json(&active, Some(&selected));

    assert_eq!(snapshot["snapshot_kind"], "rustclaw_cli_tui");
    assert_eq!(
        snapshot["active"]["data"]["tasks"][0]["task_id"],
        "task-tui"
    );
    assert_eq!(snapshot["selected_task"]["task_id"], "task-tui");
    assert_eq!(
        snapshot["selected_task"]["task_lifecycle"]["checkpoint_id"],
        "ckpt-tui"
    );
    assert_eq!(snapshot["selected_progress"]["checkpoint_id"], "ckpt-tui");
    assert_eq!(snapshot["selected_progress"]["resume_due"], true);
    assert_eq!(
        snapshot["selected_progress"]["next_action_kind"],
        "resume_checkpoint"
    );
    assert_eq!(snapshot["selected_summary"]["llm"]["llm_call_count"], 2);
    assert_eq!(
        snapshot["selected_summary"]["coding"]["changed_file_count"],
        1
    );
    assert_eq!(snapshot["selected_summary"]["artifacts"]["ref_count"], 1);
}

#[test]
fn tui_command_parser_accepts_basic_key_tokens() {
    assert_eq!(tui_command_from_input(""), Some(TuiCommand::Refresh));
    assert_eq!(tui_command_from_input(" r "), Some(TuiCommand::Refresh));
    assert_eq!(tui_command_from_input("W"), Some(TuiCommand::Watch));
    assert_eq!(tui_command_from_input("p"), Some(TuiCommand::Pause));
    assert_eq!(tui_command_from_input("c"), Some(TuiCommand::Cancel));
    assert_eq!(tui_command_from_input("u"), Some(TuiCommand::Resume));
    assert_eq!(tui_command_from_input("n"), Some(TuiCommand::Continue));
    assert_eq!(tui_command_from_input("e"), Some(TuiCommand::Export));
    assert_eq!(tui_command_from_input("1"), Some(TuiCommand::Report));
    assert_eq!(tui_command_from_input("2"), Some(TuiCommand::Review));
    assert_eq!(tui_command_from_input("3"), Some(TuiCommand::Subagents));
    assert_eq!(tui_command_from_input("4"), Some(TuiCommand::Permission));
    assert_eq!(tui_command_from_input("q"), Some(TuiCommand::Quit));
    assert_eq!(tui_command_from_input("watch"), None);
}

#[test]
fn tui_export_json_wraps_snapshot_and_selected_task_id() {
    let active = serde_json::json!({
        "data": {
            "tasks": [
                {
                    "task_id": "task-tui-export",
                    "status": "running"
                }
            ]
        }
    });
    let selected = crate::task::TaskStatusView {
        task_id: "task-tui-export".to_string(),
        status: "running".to_string(),
        raw_data: serde_json::json!({
            "task_id": "task-tui-export",
            "status": "running",
            "task_lifecycle": {
                "state": "background",
                "can_cancel": true
            }
        }),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };

    let export = tui_export_json(&active, Some(&selected));

    assert_eq!(export["export_kind"], "rustclaw_cli_tui_export");
    assert_eq!(export["selected_task_id"], "task-tui-export");
    assert_eq!(
        export["snapshot"]["selected_task"]["task_lifecycle"]["can_cancel"],
        true
    );
    assert_eq!(export["snapshot"]["selected_progress"]["can_cancel"], true);
    assert_eq!(
        export["snapshot"]["selected_summary"]["task_id"],
        "task-tui-export"
    );
}

#[test]
fn tui_selected_task_lines_expose_resume_llm_and_coding_tokens() {
    let selected = crate::task::TaskStatusView {
        task_id: "task-tui-lines".to_string(),
        status: "running".to_string(),
        raw_data: serde_json::json!({
            "execution_state": "background",
            "task_lifecycle": {
                "state": "background",
                "checkpoint_id": "ckpt-lines",
                "resume_due": true,
                "resume_wait_seconds": 9,
                "next_action_kind": "resume_checkpoint",
                "pending_async_job_id": "job-lines",
                "poll_ref": "poll-lines",
                "lease_owner": "worker-lines",
                "heartbeat_at": 1781800000
            },
            "task_goal": {
                "goal_id": "goal-lines",
                "goal_status": "in_progress"
            },
            "result_json": {
                "changed_files": ["src/lib.rs"],
                "task_checkpoint": {
                    "completed_side_effect_refs": ["write_file:src/lib.rs"]
                },
                "task_journal": {
                    "summary": {
                        "task_outcome": {
                            "state": "in_progress",
                            "done_conditions": ["tests_pass"],
                            "current_progress": ["edited_file"],
                            "remaining_work": ["run_tests"]
                        }
                    },
                    "trace": {
                        "step_results": [
                            {
                                "step_id": "step_1",
                                "status": "ok",
                                "skill": "run_cmd",
                                "command": "cargo check -p clawcli"
                            }
                        ]
                    }
                }
            }
        }),
        result_text: None,
        error_text: None,
        events: vec![crate::events::TaskEventLine {
            event_type: "provider_call".to_string(),
            line: "type=provider_call prompt_label=planner llm_call_count=3".to_string(),
            fields: std::collections::BTreeMap::from([
                ("prompt_label".to_string(), "planner".to_string()),
                ("llm_call_count".to_string(), "3".to_string()),
            ]),
        }],
    };

    let lines = tui_selected_task_lines(&selected);

    assert!(lines.contains(&"tui_selected_checkpoint_id: ckpt-lines".to_string()));
    assert!(lines.contains(&"tui_selected_resume_due: true".to_string()));
    assert!(lines.contains(&"tui_selected_resume_wait_seconds: 9".to_string()));
    assert!(lines.contains(&"tui_selected_next_action_kind: resume_checkpoint".to_string()));
    assert!(lines.contains(&"tui_selected_pending_async_job_id: job-lines".to_string()));
    assert!(lines.contains(&"tui_selected_poll_ref: poll-lines".to_string()));
    assert!(lines.contains(&"tui_selected_lease_owner: worker-lines".to_string()));
    assert!(lines.contains(&"tui_selected_heartbeat_at: 1781800000".to_string()));
    assert!(lines.contains(&"tui_selected_llm_call_count: 3".to_string()));
    assert!(lines.contains(&"tui_selected_changed_file_count: 1".to_string()));
    assert!(lines.contains(&"tui_selected_verification_command_count: 1".to_string()));
    assert!(lines.contains(&"tui_selected_verification_status: verified".to_string()));
    assert!(lines.contains(&"tui_selected_completed_side_effect_count: 1".to_string()));
    assert!(lines.contains(&"tui_selected_unverified_risk: tests_not_observed".to_string()));
    assert!(lines.contains(&"tui_selected_goal_id: goal-lines".to_string()));
    assert!(lines.contains(&"tui_selected_goal_status: in_progress".to_string()));
    assert!(lines.contains(&"tui_selected_outcome_state: in_progress".to_string()));
    assert!(lines.contains(&"tui_selected_done_condition_count: 1".to_string()));
    assert!(lines.contains(&"tui_selected_current_progress_count: 7".to_string()));
    assert!(lines.contains(&"tui_selected_remaining_work_count: 2".to_string()));
}

#[test]
fn task_report_text_lines_expose_coding_verification_status() {
    let task = crate::task::TaskStatusView {
        task_id: "task-report-text".to_string(),
        status: "succeeded".to_string(),
        raw_data: serde_json::json!({
            "execution_state": "completed",
            "task_lifecycle": {
                "state": "completed",
                "reason_code": "succeeded"
            },
            "result_json": {
                "changed_files": ["src/lib.rs"],
                "step_results": [
                    {
                        "step_id": "step_1",
                        "status": "ok",
                        "skill": "run_cmd",
                        "command": "cargo check -p clawd --all-targets"
                    }
                ]
            }
        }),
        result_text: Some("result-token".to_string()),
        error_text: None,
        events: Vec::new(),
    };
    let report = task_report_json(&task, false);

    let lines = task_report_text_lines(&task, &report);

    assert!(lines.contains(&"coding_changed_file_count: 1".to_string()));
    assert!(lines.contains(&"changed_file: src/lib.rs".to_string()));
    assert!(lines.contains(&"coding_verification_command_count: 1".to_string()));
    assert!(lines.contains(&"verification_command: cargo check -p clawd --all-targets".to_string()));
    assert!(lines.contains(&"coding_test_count: 0".to_string()));
    assert!(lines.contains(&"coding_failure_count: 0".to_string()));
    assert!(lines.contains(&"coding_verification_status: verified".to_string()));
    assert!(lines.contains(&"coding_verification_failure_kind_count: 0".to_string()));
    assert!(!lines.iter().any(|line| line.contains("task_journal")));
}

#[test]
fn task_report_json_exposes_async_final_result() {
    let task = crate::task::TaskStatusView {
        task_id: "task-async-report".to_string(),
        status: "succeeded".to_string(),
        raw_data: serde_json::json!({
            "execution_state": "completed",
            "lifecycle": {
                "state": "completed",
                "execution_state": "completed",
                "reason_code": "async_poll_completed"
            },
            "result_json": {
                "task_lifecycle": {
                    "resume_executor_result_projection": {
                        "final_result_json": {
                            "exit_code": 0,
                            "stdout": "ASYNC_STDOUT_TOKEN\n",
                            "output": "ASYNC_OUTPUT_TOKEN\n"
                        }
                    }
                }
            }
        }),
        result_text: Some("ASYNC_OUTPUT_TOKEN\n".to_string()),
        error_text: None,
        events: Vec::new(),
    };

    let report = task_report_json(&task, false);

    assert_eq!(report["execution_state"], "completed");
    assert_eq!(report["lifecycle_state"], "completed");
    assert_eq!(report["lifecycle"]["reason_code"], "async_poll_completed");
    assert_eq!(report["result_text"], "ASYNC_OUTPUT_TOKEN\n");
    assert_eq!(report["async_result"]["exit_code"], 0);
    assert_eq!(report["async_result"]["output"], "ASYNC_OUTPUT_TOKEN\n");
}

#[test]
fn task_report_json_summarizes_coding_verification_gaps() {
    let task = crate::task::TaskStatusView {
        task_id: "task-coding-report".to_string(),
        status: "failed".to_string(),
        raw_data: serde_json::json!({
            "execution_state": "failed",
            "result_json": {
                "files_changed": [
                    {"path": "crates/clawd/src/main.rs"},
                    {"file_path": "crates/clawd/src/lib.rs"}
                ],
                "repair_count": 2,
                "task_checkpoint": {
                    "checkpoint_id": "ckpt-coding",
                    "resume_entrypoint": "next_planner_round",
                    "completed_side_effect_refs": [
                        "write_file:crates/clawd/src/main.rs"
                    ]
                },
                "task_journal": {
                    "trace": {
                        "event_stream": [
                            {
                                "event_type": "coding_checkpoint",
                                "payload": {
                                    "checkpoint_kind": "verification_command",
                                    "checkpoint_ref": "coding_checkpoint:verification_command:1",
                                    "evidence_ref": "coding_checkpoint:verification_command:1"
                                }
                            }
                        ],
                        "step_results": [
                            {
                                "step_id": "step_1",
                                "status": "ok",
                                "skill": "run_cmd",
                                "command": "cargo fmt --all"
                            },
                            {
                                "step_id": "step_2",
                                "status": "error",
                                "skill": "run_cmd",
                                "requested_action_ref": "run_cmd",
                                "command": "cargo test -p clawd",
                                "error_code": "exit_status"
                            }
                        ]
                    }
                }
            }
        }),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };

    let report = task_report_json(&task, false);

    assert_eq!(report["coding"]["changed_file_count"], 2);
    assert_eq!(report["coding"]["command_count"], 2);
    assert_eq!(report["coding"]["commands"][0], "cargo fmt --all");
    assert_eq!(report["coding"]["commands"][1], "cargo test -p clawd");
    assert_eq!(report["coding"]["verification_command_count"], 2);
    assert_eq!(
        report["coding"]["verification_commands"][0],
        "cargo fmt --all"
    );
    assert_eq!(
        report["coding"]["verification_commands"][1],
        "cargo test -p clawd"
    );
    assert_eq!(report["coding"]["test_count"], 1);
    assert_eq!(report["coding"]["tests"][0], "cargo test -p clawd");
    assert_eq!(report["coding"]["failure_count"], 1);
    assert_eq!(report["coding"]["failures"][0]["step_id"], "step_2");
    assert_eq!(report["coding"]["failures"][0]["error_code"], "exit_status");
    assert_eq!(report["coding"]["verification_failure_kind_count"], 1);
    assert_eq!(report["coding"]["verification_failure_kinds"][0], "test");
    assert_eq!(report["coding"]["retry_count"], 2);
    assert_eq!(report["coding"]["state"]["current_phase_hint"], "repair");
    assert_eq!(
        report["coding"]["state"]["next_step"],
        "repair_failed_verification"
    );
    assert_eq!(report["coding"]["state"]["has_changes"], true);
    assert_eq!(report["coding"]["state"]["has_verification"], true);
    assert_eq!(report["coding"]["state"]["has_failed_verification"], true);
    assert_eq!(report["coding"]["state"]["verification_status"], "failed");
    assert_eq!(
        report["coding"]["validation_gate"]["gate_status"],
        "repair_required"
    );
    assert_eq!(
        report["coding"]["validation_gate"]["can_report_fully_verified"],
        false
    );
    assert_eq!(report["coding"]["state"]["repair_observed"], true);
    assert_eq!(report["coding"]["state"]["checkpointed"], true);
    assert_eq!(report["coding"]["state"]["resumable"], true);
    assert_eq!(
        report["coding"]["state"]["requires_idempotency_guard"],
        true
    );
    assert_eq!(
        report["coding"]["state"]["checkpoint_kinds"][0],
        "verification_command"
    );
    assert_eq!(
        report["coding"]["state"]["completed_side_effect_refs"][0],
        "write_file:crates/clawd/src/main.rs"
    );
    assert!(report["coding"]["state"]["observed_phases"]
        .as_array()
        .expect("observed phases")
        .iter()
        .any(|value| value == "repair"));
    assert_eq!(report["coding"]["unverified_risk"], serde_json::Value::Null);

    let lines = task_report_text_lines(&task, &report);
    assert!(lines.contains(&"coding_verification_failure_kind_count: 1".to_string()));
    assert!(lines.contains(&"verification_failure_kind: test".to_string()));
    assert!(lines.contains(&"coding_current_phase_hint: repair".to_string()));
    assert!(lines.contains(&"coding_next_step: repair_failed_verification".to_string()));
    assert!(lines.contains(&"coding_checkpoint_ref_count: 1".to_string()));
    assert!(lines.contains(&"coding_completed_side_effect_count: 1".to_string()));
}

#[test]
fn task_report_json_extracts_run_cmd_machine_excerpt_verification() {
    let task = crate::task::TaskStatusView {
        task_id: "task-coding-python-test".to_string(),
        status: "succeeded".to_string(),
        raw_data: serde_json::json!({
            "execution_state": "completed",
            "result_json": {
                "changed_files": ["tmp/work/calc_core.py", "tmp/work/test_calc_core.py"],
                "task_journal": {
                    "trace": {
                        "step_results": [
                            {
                                "step_id": "step_4",
                                "status": "ok",
                                "skill": "run_cmd",
                                "requested_action_ref": "run_cmd",
                                "output_excerpt": "exit=0 command=python3 test_calc_core.py"
                            }
                        ]
                    }
                }
            }
        }),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };

    let report = task_report_json(&task, false);

    assert_eq!(report["coding"]["changed_file_count"], 2);
    assert_eq!(report["coding"]["command_count"], 1);
    assert_eq!(report["coding"]["commands"][0], "python3 test_calc_core.py");
    assert_eq!(report["coding"]["verification_command_count"], 1);
    assert_eq!(
        report["coding"]["verification_commands"][0],
        "python3 test_calc_core.py"
    );
    assert_eq!(report["coding"]["test_count"], 1);
    assert_eq!(report["coding"]["tests"][0], "python3 test_calc_core.py");
    assert_eq!(report["coding"]["unverified_risk"], serde_json::Value::Null);
}

#[test]
fn task_report_json_extracts_coding_fields_from_json_string_result_text() {
    let task = crate::task::TaskStatusView {
        task_id: "task-coding-json-text".to_string(),
        status: "succeeded".to_string(),
        raw_data: serde_json::json!({
            "execution_state": "completed",
            "result_json": {
                "text": "{\"changed_files\":[\"tmp/live/calc_core.py\",\"tmp/live/test_calc_core.py\"],\"test_command\":\"python3 test_calc_core.py\",\"test_status\":\"passed\"}"
            }
        }),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };

    let report = task_report_json(&task, false);

    assert_eq!(report["coding"]["changed_file_count"], 2);
    assert_eq!(
        report["coding"]["changed_files"][0],
        "tmp/live/calc_core.py"
    );
    assert_eq!(report["coding"]["verification_command_count"], 1);
    assert_eq!(
        report["coding"]["verification_commands"][0],
        "python3 test_calc_core.py"
    );
    assert_eq!(report["coding"]["test_count"], 1);
    assert_eq!(report["coding"]["tests"][0], "python3 test_calc_core.py");
    assert_eq!(report["coding"]["state"]["verification_status"], "verified");
}

#[test]
fn task_log_event_output_uses_task_events_not_raw_log_files() {
    let task = crate::task::TaskStatusView {
        task_id: "task-logs".to_string(),
        status: "running".to_string(),
        raw_data: serde_json::json!({
            "debug_log_file": "clawd.log",
            "task_lifecycle": {
                "state": "background"
            }
        }),
        result_text: None,
        error_text: None,
        events: vec![crate::events::TaskEventLine {
            event_type: "checkpoint_created".to_string(),
            line: "seq=7 type=checkpoint_created checkpoint_id=ckpt-logs".to_string(),
            fields: std::collections::BTreeMap::from([(
                "checkpoint_id".to_string(),
                "ckpt-logs".to_string(),
            )]),
        }],
    };
    let events = task.events.iter().collect::<Vec<_>>();

    let plain = task_event_output_lines(&task, events.clone(), false).expect("plain event output");
    assert_eq!(
        plain,
        vec!["event: seq=7 type=checkpoint_created checkpoint_id=ckpt-logs"]
    );
    assert!(!plain.join("\n").contains("clawd.log"));

    let jsonl = task_event_output_lines(&task, events, true).expect("jsonl event output");
    let value: serde_json::Value = serde_json::from_str(&jsonl[0]).expect("parse jsonl line");
    assert_eq!(value["task_id"], "task-logs");
    assert_eq!(value["event_type"], "checkpoint_created");
    assert_eq!(value["fields"]["checkpoint_id"], "ckpt-logs");
    assert!(!jsonl[0].contains("clawd.log"));
}

#[test]
fn exec_exit_class_uses_machine_tokens_only() {
    let task = crate::task::TaskStatusView {
        task_id: "task-exec-failed".to_string(),
        status: "failed".to_string(),
        raw_data: serde_json::json!({
            "result_json": {
                "error_code": "provider_rate_limited"
            }
        }),
        result_text: None,
        error_text: Some("ignored visible fallback".to_string()),
        events: Vec::new(),
    };

    assert_eq!(
        exec_failure_class_from_machine_tokens(&task),
        ExecExitClass::ProviderUnavailable
    );
    assert_eq!(
        exec_exit_class(&task, ExecWaitOutcome::Terminal, false),
        ExecExitClass::ProviderUnavailable
    );
}

#[test]
fn exec_artifact_writer_exports_summary_task_and_events() {
    let artifact_dir = std::env::temp_dir().join(format!(
        "clawcli_exec_artifacts_{}_{}",
        std::process::id(),
        unique_suffix()
    ));
    let task = crate::task::TaskStatusView {
        task_id: "task-exec-artifact".to_string(),
        status: "succeeded".to_string(),
        raw_data: serde_json::json!({
            "task_id": "task-exec-artifact",
            "status": "succeeded",
            "task_lifecycle": {
                "state": "background",
                "checkpoint_id": "ckpt-exec-artifact",
                "completed_side_effect_count": 1,
                "requires_idempotency_guard": true
            },
            "result_json": {
                "changed_files": ["crates/clawcli/src/main.rs"],
                "final_diff_summary": {
                    "file_count": 1,
                    "summary_code": "clawcli_exec_artifacts"
                },
                "task_checkpoint": {
                    "checkpoint_id": "ckpt-exec-artifact",
                    "completed_side_effect_refs": ["write_file:crates/clawcli/src/main.rs"]
                },
                "task_journal": {
                    "trace": {
                        "step_results": [
                            {
                                "step_id": "step_1",
                                "status": "ok",
                                "skill": "run_cmd",
                                "command": "cargo test -p clawcli"
                            }
                        ],
                        "event_stream": [
                            {
                                "seq": 1,
                                "event_type": "task_completed",
                                "payload": {
                                    "status": "succeeded"
                                }
                            }
                        ]
                    }
                }
            }
        }),
        result_text: Some("machine-result-token".to_string()),
        error_text: None,
        events: vec![
            crate::events::TaskEventLine {
                event_type: "task_completed".to_string(),
                line: "seq=1 type=task_completed status=succeeded".to_string(),
                fields: std::collections::BTreeMap::new(),
            },
            crate::events::TaskEventLine {
                event_type: "provider_call".to_string(),
                line: "seq=2 type=provider_call prompt_label=planner".to_string(),
                fields: std::collections::BTreeMap::from([
                    ("prompt_label".to_string(), "planner".to_string()),
                    ("llm_call_count".to_string(), "2".to_string()),
                    ("elapsed_ms".to_string(), "250".to_string()),
                    ("prompt_bytes_before_max".to_string(), "8192".to_string()),
                ]),
            },
        ],
    };
    let summary = exec_summary_json(
        &task,
        ExecWaitOutcome::Terminal,
        ExecExitClass::Success,
        None,
    );

    write_exec_artifacts(&artifact_dir, &task, &summary).expect("write exec artifacts");

    let summary_file =
        std::fs::read_to_string(artifact_dir.join("summary.json")).expect("read summary artifact");
    let task_file =
        std::fs::read_to_string(artifact_dir.join("task.json")).expect("read task artifact");
    let events_file =
        std::fs::read_to_string(artifact_dir.join("events.jsonl")).expect("read event artifact");
    let resume_file =
        std::fs::read_to_string(artifact_dir.join("resume.json")).expect("read resume artifact");
    let resume: serde_json::Value =
        serde_json::from_str(&resume_file).expect("parse resume artifact");
    let verification_file = std::fs::read_to_string(artifact_dir.join("verification.json"))
        .expect("read verification artifact");
    let diff_summary_file = std::fs::read_to_string(artifact_dir.join("diff_summary.json"))
        .expect("read diff summary artifact");
    let llm_summary_file = std::fs::read_to_string(artifact_dir.join("llm_summary.json"))
        .expect("read llm summary artifact");
    let llm_summary: serde_json::Value =
        serde_json::from_str(&llm_summary_file).expect("parse llm summary artifact");
    let index_file =
        std::fs::read_to_string(artifact_dir.join("index.json")).expect("read index artifact");
    let index: serde_json::Value = serde_json::from_str(&index_file).expect("parse index artifact");

    assert!(summary_file.contains("\"exit_class\": \"success\""));
    assert!(summary_file.contains("\"llm_call_count\": 2"));
    assert!(task_file.contains("\"task-exec-artifact\""));
    assert!(events_file.contains("type=task_completed"));
    assert!(resume_file.contains("\"task-exec-artifact\""));
    assert_eq!(resume["completed_side_effect_count"], 1);
    assert_eq!(resume["requires_idempotency_guard"], true);
    assert_eq!(
        resume["completed_side_effect_refs"][0],
        "write_file:crates/clawcli/src/main.rs"
    );
    assert_eq!(resume["coding"]["changed_file_count"], 1);
    assert_eq!(
        resume["coding"]["verification_commands"][0],
        "cargo test -p clawcli"
    );
    assert!(verification_file.contains("\"artifact_kind\": \"rustclaw_exec_verification\""));
    assert!(verification_file.contains("\"verification_status\": \"verified\""));
    assert!(verification_file.contains("\"coding_state\""));
    assert!(verification_file.contains("\"completed_side_effect_count\": 1"));
    assert!(verification_file.contains("\"cargo test -p clawcli\""));
    assert!(diff_summary_file.contains("\"artifact_kind\": \"rustclaw_exec_diff_summary\""));
    assert!(diff_summary_file.contains("\"summary_code\": \"clawcli_exec_artifacts\""));
    assert!(diff_summary_file.contains("\"crates/clawcli/src/main.rs\""));
    assert_eq!(llm_summary["llm_call_count"], 2);
    assert_eq!(llm_summary["by_prompt"][0]["prompt_label"], "planner");
    assert_eq!(index["artifact_kind"], "rustclaw_exec_artifact_index");
    assert_eq!(index["task_id"], "task-exec-artifact");
    assert_eq!(index["file_count"], 8);
    assert!(index["files"]
        .as_array()
        .expect("index files")
        .iter()
        .any(|file| file["kind"] == "llm_summary" && file["path"] == "llm_summary.json"));

    std::fs::remove_dir_all(artifact_dir).ok();
}

#[test]
fn exec_compact_text_lines_include_coding_budget_and_resume_tokens() {
    let summary = serde_json::json!({
        "task_id": "task-compact",
        "status": "succeeded",
        "lifecycle_state": "completed",
        "outcome": "terminal",
        "exit_class": "success",
        "effective_config": {
            "profile": "coding"
        },
        "resume": {
            "mode": "new_task"
        },
        "resume_hint": {
            "checkpoint_id": "ckpt-compact",
            "resume_due": true
        },
        "llm": {
            "budget_health": {
                "status": "warning"
            }
        },
        "coding": {
            "changed_file_count": 1,
            "changed_files": ["crates/clawcli/src/commands/exec.rs"],
            "verification_command_count": 1,
            "verification_commands": ["cargo test -p clawcli exec -- --quiet"],
            "unverified_risk": null,
            "state": {
                "verification_status": "verified",
                "next_step": "summarize",
                "checkpoint_ref_count": 1,
                "completed_side_effect_count": 1
            }
        },
        "artifact_index": {
            "path": "index.json"
        }
    });

    let lines = exec_compact_text_lines(&summary);

    assert!(lines.contains(&"exec_compact_profile: coding".to_string()));
    assert!(lines.contains(&"exec_compact_task_id: task-compact".to_string()));
    assert!(lines.contains(&"exec_compact_budget_status: warning".to_string()));
    assert!(lines.contains(&"exec_compact_checkpoint_id: ckpt-compact".to_string()));
    assert!(lines.contains(&"exec_compact_resume_due: true".to_string()));
    assert!(lines.contains(&"exec_compact_changed_file_count: 1".to_string()));
    assert!(lines.contains(&"exec_compact_verification_status: verified".to_string()));
    assert!(lines.contains(&"exec_compact_artifact_index: index.json".to_string()));
    assert!(lines
        .contains(&"exec_compact_changed_file: crates/clawcli/src/commands/exec.rs".to_string()));
    assert!(lines.contains(
        &"exec_compact_verification_command: cargo test -p clawcli exec -- --quiet".to_string()
    ));

    let index = exec_artifact_index_json(
        &summary,
        std::path::Path::new("/tmp/rustclaw-artifacts"),
        &[("summary", "summary.json"), ("index", "index.json")],
    );
    assert_eq!(index["artifact_kind"], "rustclaw_exec_artifact_index");
    assert_eq!(index["file_count"], 2);
    assert_eq!(
        index["files"][0]["absolute_path"],
        "/tmp/rustclaw-artifacts/summary.json"
    );
}

#[test]
fn exec_summary_json_records_resume_source_task_id() {
    let task = crate::task::TaskStatusView {
        task_id: "task-resume-child".to_string(),
        status: "running".to_string(),
        raw_data: serde_json::json!({
            "task_lifecycle": {
                "state": "background",
                "checkpoint_id": "ckpt-resume"
            }
        }),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };

    let summary = exec_summary_json(
        &task,
        ExecWaitOutcome::Background,
        ExecExitClass::Success,
        Some("task-resume-source"),
    );

    assert_eq!(summary["resume"]["mode"], "resume_task");
    assert_eq!(summary["resume"]["source_task_id"], "task-resume-source");
    assert_eq!(summary["resume"]["resume_trigger"], "user_followup");
}

#[test]
fn exec_offline_smoke_writes_machine_artifact_without_server() {
    let artifact_dir = std::env::temp_dir().join(format!(
        "clawcli_exec_offline_smoke_{}_{}",
        std::process::id(),
        unique_suffix()
    ));

    let exit_code = run_exec(
        "http://127.0.0.1:9",
        "unused-key",
        "unused prompt",
        None,
        None,
        false,
        false,
        false,
        None,
        1000,
        true,
        true,
        Some(&artifact_dir),
        false,
    )
    .expect("offline exec smoke");

    let summary_file =
        std::fs::read_to_string(artifact_dir.join("summary.json")).expect("read summary artifact");
    let summary: serde_json::Value =
        serde_json::from_str(&summary_file).expect("parse summary artifact");

    assert_eq!(exit_code, ExecExitClass::InvalidRequest.code());
    assert_eq!(summary["exit_class"], "invalid_request");
    assert_eq!(summary["exit_code"], ExecExitClass::InvalidRequest.code());
    assert_eq!(summary["error_code"], "exec_background_policy_conflict");

    std::fs::remove_dir_all(artifact_dir).ok();
}

#[test]
fn exec_profile_resolves_machine_options_without_prompt_semantics() {
    let options = exec_effective_options(
        Some("long-tail"),
        false,
        false,
        false,
        None,
        1000,
        false,
        false,
        None,
    )
    .expect("resolve long-tail profile");

    assert_eq!(options.timeout_seconds, Some(3600));
    assert!(options.continue_on_background);
    assert!(!options.fail_on_background);
    assert_eq!(
        options.artifact_dir.as_deref(),
        Some(std::path::Path::new("artifacts/rustclaw-exec/long-tail"))
    );

    let release_gate = exec_effective_options(
        Some("release-gate"),
        false,
        false,
        false,
        Some(42),
        1000,
        false,
        false,
        None,
    )
    .expect("resolve release-gate profile");
    assert_eq!(release_gate.timeout_seconds, Some(42));
    assert!(release_gate.fail_on_background);
}

#[test]
fn wait_until_matches_machine_lifecycle_states() {
    let background = crate::task::TaskStatusView {
        task_id: "task-wait-background".to_string(),
        status: "running".to_string(),
        raw_data: serde_json::json!({
            "execution_state": "background",
            "task_lifecycle": {
                "state": "background"
            }
        }),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };
    assert!(wait_until_matches(&background, "background"));
    assert!(!wait_until_matches(&background, "terminal"));

    let needs_user = crate::task::TaskStatusView {
        task_id: "task-wait-needs-user".to_string(),
        status: "running".to_string(),
        raw_data: serde_json::json!({
            "execution_state": "needs_confirmation"
        }),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };
    assert!(wait_until_matches(&needs_user, "needs_user"));

    let completed = crate::task::TaskStatusView {
        task_id: "task-wait-completed".to_string(),
        status: "succeeded".to_string(),
        raw_data: serde_json::json!({
            "execution_state": "completed"
        }),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };
    assert!(wait_until_matches(&completed, "completed"));
    assert!(wait_until_matches(&completed, "terminal"));
}

#[test]
fn resume_control_summary_projects_checkpoint_lifecycle_fields() {
    let body = serde_json::json!({
        "ok": true,
        "data": {
            "status": "task_resume_requested",
            "task_id": "task-resume",
            "checkpoint_id": "ckpt-1",
            "task_lifecycle": {
                "state": "background",
                "execution_state": "background",
                "resume_due": true,
                "resume_wait_seconds": 0,
                "resume_entrypoint": "next_planner_round",
                "resume_directive": "run_next_planner_round",
                "resume_reason": "agent_loop_soft_budget",
                "resume_claim": {
                    "owner": "worker-a"
                },
                "next_action_kind": "resume_checkpoint",
                "last_successful_evidence_ref": "step_2:evidence:1",
                "evidence_ref_count": 3,
                "budget": {
                    "round": 2,
                    "llm_calls": 5,
                    "tool_calls": 4
                }
            }
        }
    });

    let summary = task_resume_control_summary_json("task-request", "continue", &body);

    assert_eq!(summary["schema_version"], 1);
    assert_eq!(summary["operation"], "continue");
    assert_eq!(summary["task_id"], "task-resume");
    assert_eq!(summary["status"], "task_resume_requested");
    assert_eq!(summary["checkpoint_id"], "ckpt-1");
    assert_eq!(summary["lifecycle_state"], "background");
    assert_eq!(summary["execution_state"], "background");
    assert_eq!(summary["resume_due"], true);
    assert_eq!(summary["resume_wait_seconds"], 0);
    assert_eq!(summary["resume_entrypoint"], "next_planner_round");
    assert_eq!(summary["resume_directive"], "run_next_planner_round");
    assert_eq!(summary["resume_reason"], "agent_loop_soft_budget");
    assert_eq!(summary["resume_owner"], "worker-a");
    assert_eq!(summary["next_action_kind"], "resume_checkpoint");
    assert_eq!(summary["last_successful_evidence_ref"], "step_2:evidence:1");
    assert_eq!(summary["evidence_ref_count"], 3);
    assert_eq!(summary["budget"]["llm_calls"], 5);
}

#[test]
fn watch_progress_json_exposes_compact_lifecycle_machine_fields() {
    let task = crate::task::TaskStatusView {
        task_id: "task-watch-progress".to_string(),
        status: "running".to_string(),
        raw_data: serde_json::json!({
            "execution_state": "background",
            "task_lifecycle": {
                "state": "background",
                "db_status": "running",
                "checkpoint_id": "ckpt-watch",
                "can_poll": true,
                "can_cancel": true,
                "resume_entrypoint": "next_planner_round",
                "resume_directive": "run_next_planner_round",
                "resume_reason": "agent_loop_soft_budget",
                "resume_due": false,
                "resume_wait_seconds": 17,
                "next_action_kind": "resume_checkpoint",
                "reason_code": "agent_loop_max_rounds",
                "next_poll_after": "2030-01-01T00:00:00Z",
                "poll_ref": "poll:watch",
                "last_heartbeat_ts": 1781800000,
                "lease_owner": "worker-a",
                "lease_expires_at": 1781800060,
                "claim_attempt": 3,
                "attempt_id": 3,
                "claimed_at": 1781799990
            }
        }),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };

    let progress = watch_progress_json(&task);

    assert_eq!(progress["execution_state"], "background");
    assert_eq!(progress["lifecycle_state"], "background");
    assert_eq!(progress["db_status"], "running");
    assert_eq!(progress["checkpoint_id"], "ckpt-watch");
    assert_eq!(progress["can_poll"], true);
    assert_eq!(progress["can_cancel"], true);
    assert_eq!(progress["resume_entrypoint"], "next_planner_round");
    assert_eq!(progress["resume_directive"], "run_next_planner_round");
    assert_eq!(progress["resume_reason"], "agent_loop_soft_budget");
    assert_eq!(progress["resume_wait_seconds"], 17);
    assert_eq!(progress["next_action_kind"], "resume_checkpoint");
    assert_eq!(progress["reason_code"], "agent_loop_max_rounds");
    assert_eq!(progress["poll_ref"], "poll:watch");
    assert_eq!(progress["last_heartbeat_ts"], 1781800000);
    assert_eq!(progress["lease_owner"], "worker-a");
    assert_eq!(progress["lease_expires_at"], 1781800060);
    assert_eq!(progress["claim_attempt"], 3);
    assert_eq!(progress["attempt_id"], 3);
    assert_eq!(progress["claimed_at"], 1781799990);
}

#[test]
fn automation_runs_payload_clamps_limit_and_trims_job_id() {
    let payload = automation_runs_request_payload(7, 11, Some(" job_abc123 ".to_string()), 250);

    assert_eq!(payload["user_id"], 7);
    assert_eq!(payload["chat_id"], 11);
    assert_eq!(payload["job_id"], "job_abc123");
    assert_eq!(payload["limit"], 100);

    let without_job = automation_runs_request_payload(7, 11, Some("  ".to_string()), 0);
    assert!(without_job["job_id"].is_null());
    assert_eq!(without_job["limit"], 1);
}

#[test]
fn goal_request_payload_preserves_structured_goal_fields() {
    let done_conditions = vec![" tests_pass ".to_string(), String::new()];
    let verification_commands = vec!["cargo test -p clawcli".to_string()];
    let constraints = vec!["scope=workspace".to_string()];

    let payload = goal_request_payload(
        "implement task",
        Some(" ship feature "),
        &done_conditions,
        &verification_commands,
        &constraints,
    );

    assert_eq!(payload["text"], "implement task");
    assert_eq!(payload["goal"]["schema_version"], 1);
    assert_eq!(payload["goal"]["objective"], "ship feature");
    assert_eq!(payload["goal"]["done_conditions"][0], "tests_pass");
    assert_eq!(
        payload["goal"]["verification_commands"][0],
        "cargo test -p clawcli"
    );
    assert_eq!(payload["goal"]["constraints"][0], "scope=workspace");
    assert_eq!(payload["goal"]["goal_status"], "created");
    assert_eq!(
        payload["goal"]["done_conditions"]
            .as_array()
            .expect("done conditions")
            .len(),
        1
    );
}

#[test]
fn goal_edit_patch_json_merges_flags_over_goal_json() {
    let done_conditions = vec!["done_a".to_string()];
    let verification_commands = vec!["cargo test -p clawcli".to_string()];
    let constraints = vec!["scope=workspace".to_string()];
    let allowed_scopes = vec!["crates/clawcli".to_string()];
    let forbidden_actions = vec!["external_publish".to_string()];

    let patch = goal_edit_patch_json(
        Some(r#"{"objective":"from-json","goal_id":"goal-1"}"#),
        Some(" from-flag "),
        &done_conditions,
        &verification_commands,
        &constraints,
        &allowed_scopes,
        &forbidden_actions,
        Some("background"),
    )
    .expect("goal edit patch");

    assert_eq!(patch["goal_id"], "goal-1");
    assert_eq!(patch["objective"], "from-flag");
    assert_eq!(patch["done_conditions"][0], "done_a");
    assert_eq!(patch["verification_commands"][0], "cargo test -p clawcli");
    assert_eq!(patch["constraints"][0], "scope=workspace");
    assert_eq!(patch["allowed_files_or_scopes"][0], "crates/clawcli");
    assert_eq!(patch["forbidden_actions"][0], "external_publish");
    assert_eq!(patch["goal_status"], "background");
}

#[test]
fn goal_status_summary_and_text_lines_use_goal_projection() {
    let task = crate::task::TaskStatusView {
        task_id: "task-goal".to_string(),
        status: "running".to_string(),
        raw_data: serde_json::json!({
            "execution_state": "background",
            "goal": {
                "goal_id": "task:task-goal",
                "goal_status": "background",
                "goal_status_source": "lifecycle",
                "objective": "ship feature",
                "done_conditions": ["tests_pass"],
                "verification_commands": ["cargo test -p clawcli"],
                "constraints": ["scope=workspace"],
                "current_progress": ["changed_file_count=1"]
            },
            "task_lifecycle": {
                "state": "background"
            }
        }),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };

    let summary = goal_status_summary_json(&task);
    assert_eq!(summary["report_kind"], "rustclaw_goal_status");
    assert_eq!(summary["task_id"], "task-goal");
    assert_eq!(summary["goal"]["goal_status"], "background");
    assert_eq!(summary["goal"]["objective"], "ship feature");

    let lines = goal_status_text_lines(&summary);
    assert!(lines.contains(&"goal_task_id: task-goal".to_string()));
    assert!(lines.contains(&"goal_status: background".to_string()));
    assert!(lines.contains(&"goal_done_condition_count: 1".to_string()));
    assert!(lines.contains(&"goal_verification_command_count: 1".to_string()));
    assert!(lines.contains(&"goal_current_progress_count: 1".to_string()));
}

#[test]
fn goal_control_summary_json_extracts_resume_machine_fields() {
    let body = serde_json::json!({
        "data": {
            "task_id": "task-goal-control",
            "status": "task_resume_requested",
            "task_lifecycle": {
                "state": "background",
                "execution_state": "background",
                "checkpoint_id": "ckpt-goal",
                "resume_due": true,
                "resume_wait_seconds": 0,
                "resume_entrypoint": "next_planner_round",
                "resume_directive": "run_next_planner_round",
                "resume_reason": "goal_resume",
                "next_action_kind": "resume_checkpoint"
            }
        }
    });

    let summary = goal_control_summary_json("goal_resume", "task-requested", &body);

    assert_eq!(summary["schema_version"], 1);
    assert_eq!(summary["operation"], "goal_resume");
    assert_eq!(summary["task_id"], "task-goal-control");
    assert_eq!(summary["status"], "task_resume_requested");
    assert_eq!(summary["checkpoint_id"], "ckpt-goal");
    assert_eq!(summary["lifecycle_state"], "background");
    assert_eq!(summary["execution_state"], "background");
    assert_eq!(summary["resume_due"], true);
    assert_eq!(summary["resume_wait_seconds"], 0);
    assert_eq!(summary["resume_entrypoint"], "next_planner_round");
    assert_eq!(summary["resume_directive"], "run_next_planner_round");
    assert_eq!(summary["resume_reason"], "goal_resume");
    assert_eq!(summary["next_action_kind"], "resume_checkpoint");
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos()
}
