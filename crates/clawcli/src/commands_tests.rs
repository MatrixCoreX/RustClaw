use super::{
    automation_runs_request_payload, coding_review_json, exec_effective_options, exec_exit_class,
    exec_failure_class_from_machine_tokens, exec_summary_json, permission_report_json, run_exec,
    subagent_report_json, task_event_output_lines, task_report_json, task_report_text_lines,
    tui_command_from_input, tui_export_json, tui_snapshot_json, wait_until_matches,
    watch_progress_json, write_exec_artifacts, ExecExitClass, ExecWaitOutcome, TuiCommand,
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
            "result_json": {
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
        events: vec![crate::events::TaskEventLine {
            event_type: "checkpoint_created".to_string(),
            line: "type=checkpoint_created checkpoint_id=ckpt-exec".to_string(),
            fields: std::collections::BTreeMap::from([(
                "checkpoint_id".to_string(),
                "ckpt-exec".to_string(),
            )]),
        }],
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
            "task_lifecycle": {
                "state": "completed",
                "reason_code": "succeeded"
            },
            "result_json": {
                "artifact_refs": [
                    {
                        "ref": "artifact:report"
                    }
                ],
                "changed_files": ["src/lib.rs"],
                "final_diff_summary": {
                    "file_count": 1,
                    "summary_code": "update_lib_api"
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
        events: vec![crate::events::TaskEventLine {
            event_type: "task_completed".to_string(),
            line: "seq=1 type=task_completed status=succeeded".to_string(),
            fields: std::collections::BTreeMap::from([(
                "status".to_string(),
                "succeeded".to_string(),
            )]),
        }],
    };

    let report = task_report_json(&task, true);

    assert_eq!(report["report_kind"], "rustclaw_task_report");
    assert_eq!(report["task_id"], "task-report");
    assert_eq!(report["status"], "succeeded");
    assert_eq!(report["execution_state"], "completed");
    assert_eq!(report["lifecycle_state"], "completed");
    assert_eq!(report["terminal"], true);
    assert_eq!(report["event_count"], 1);
    assert_eq!(report["events"][0]["event_type"], "task_completed");
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
    assert_eq!(report["coding"]["unverified_risk"], serde_json::Value::Null);
    assert_eq!(report["artifacts"]["ref_count"], 1);
    assert_eq!(report["artifacts"]["refs"][0]["ref"], "artifact:report");
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
    assert_eq!(report["subagents"][0]["finding_refs"][0], "finding:1");
    assert_eq!(
        report["subagents"][1]["child_run_id"],
        "subagent:1:2:verifier"
    );
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
                    "reason_code": "dry_run_required"
                },
                "step_results": [
                    {
                        "extra": {
                            "command_policy": {
                                "policy_authority": "contract_matrix",
                                "effect": "filesystem_write"
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
        report["permission_entries"][1]["decision"],
        "contract_matrix"
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
            "task_lifecycle": {
                "state": "background",
                "checkpoint_id": "ckpt-tui"
            }
        }),
        result_text: None,
        error_text: None,
        events: Vec::new(),
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
}

#[test]
fn tui_command_parser_accepts_basic_key_tokens() {
    assert_eq!(tui_command_from_input(""), Some(TuiCommand::Refresh));
    assert_eq!(tui_command_from_input(" r "), Some(TuiCommand::Refresh));
    assert_eq!(tui_command_from_input("W"), Some(TuiCommand::Watch));
    assert_eq!(tui_command_from_input("c"), Some(TuiCommand::Cancel));
    assert_eq!(tui_command_from_input("u"), Some(TuiCommand::Resume));
    assert_eq!(tui_command_from_input("e"), Some(TuiCommand::Export));
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
                "task_journal": {
                    "trace": {
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
    assert_eq!(report["coding"]["unverified_risk"], serde_json::Value::Null);

    let lines = task_report_text_lines(&task, &report);
    assert!(lines.contains(&"coding_verification_failure_kind_count: 1".to_string()));
    assert!(lines.contains(&"verification_failure_kind: test".to_string()));
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
            "result_json": {
                "changed_files": ["crates/clawcli/src/main.rs"],
                "final_diff_summary": {
                    "file_count": 1,
                    "summary_code": "clawcli_exec_artifacts"
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
        events: vec![crate::events::TaskEventLine {
            event_type: "task_completed".to_string(),
            line: "seq=1 type=task_completed status=succeeded".to_string(),
            fields: std::collections::BTreeMap::new(),
        }],
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
    let verification_file = std::fs::read_to_string(artifact_dir.join("verification.json"))
        .expect("read verification artifact");
    let diff_summary_file = std::fs::read_to_string(artifact_dir.join("diff_summary.json"))
        .expect("read diff summary artifact");

    assert!(summary_file.contains("\"exit_class\": \"success\""));
    assert!(task_file.contains("\"task-exec-artifact\""));
    assert!(events_file.contains("type=task_completed"));
    assert!(resume_file.contains("\"task-exec-artifact\""));
    assert!(verification_file.contains("\"artifact_kind\": \"rustclaw_exec_verification\""));
    assert!(verification_file.contains("\"verification_status\": \"verified\""));
    assert!(verification_file.contains("\"cargo test -p clawcli\""));
    assert!(diff_summary_file.contains("\"artifact_kind\": \"rustclaw_exec_diff_summary\""));
    assert!(diff_summary_file.contains("\"summary_code\": \"clawcli_exec_artifacts\""));
    assert!(diff_summary_file.contains("\"crates/clawcli/src/main.rs\""));

    std::fs::remove_dir_all(artifact_dir).ok();
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

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos()
}
