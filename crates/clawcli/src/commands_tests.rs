use super::{
    exec_exit_class, exec_failure_class_from_machine_tokens, exec_summary_json,
    write_exec_artifacts, ExecExitClass, ExecWaitOutcome,
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
            }
        }),
        result_text: Some("result-token".to_string()),
        error_text: None,
        events: Vec::new(),
    };

    let summary = exec_summary_json(&task, ExecWaitOutcome::Background, ExecExitClass::Success);

    assert_eq!(summary["task_id"], "task-exec");
    assert_eq!(summary["status"], "running");
    assert_eq!(summary["lifecycle_state"], "background");
    assert_eq!(summary["outcome"], "background");
    assert_eq!(summary["exit_class"], "success");
    assert_eq!(summary["exit_code"], 0);
    assert_eq!(summary["terminal"], false);
    assert_eq!(summary["lifecycle"]["checkpoint_id"], "ckpt-exec");
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
    let summary = exec_summary_json(&task, ExecWaitOutcome::Terminal, ExecExitClass::Success);

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

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos()
}
