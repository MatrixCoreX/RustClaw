use super::{
    automation_runs_request_payload, exec_exit_class, exec_failure_class_from_machine_tokens,
    exec_summary_json, run_exec, task_event_output_lines, task_report_json, write_exec_artifacts,
    ExecExitClass, ExecWaitOutcome,
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
    assert_eq!(report["coding"]["test_count"], 1);
    assert_eq!(report["coding"]["tests"][0], "cargo test -p clawd");
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
    assert_eq!(report["coding"]["command_count"], 1);
    assert_eq!(report["coding"]["commands"][0], "cargo fmt --all");
    assert_eq!(report["coding"]["test_count"], 0);
    assert_eq!(report["coding"]["failure_count"], 1);
    assert_eq!(report["coding"]["failures"][0]["step_id"], "step_2");
    assert_eq!(report["coding"]["failures"][0]["error_code"], "exit_status");
    assert_eq!(report["coding"]["retry_count"], 2);
    assert_eq!(report["coding"]["unverified_risk"], "tests_not_observed");
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
                "task_journal": {
                    "trace": {
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

    assert!(summary_file.contains("\"exit_class\": \"success\""));
    assert!(task_file.contains("\"task-exec-artifact\""));
    assert!(events_file.contains("type=task_completed"));

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
        false,
        false,
        false,
        None,
        1000,
        true,
        true,
        Some(&artifact_dir),
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
