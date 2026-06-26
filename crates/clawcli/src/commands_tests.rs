use super::{
    automation_runs_request_payload, exec_exit_class, exec_failure_class_from_machine_tokens,
    exec_summary_json, run_exec, write_exec_artifacts, ExecExitClass, ExecWaitOutcome,
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
