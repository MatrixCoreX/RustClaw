use crate::{
    events::{EventFilters, TaskEventLine},
    output::task_status_lines,
    task::TaskStatusView,
};

#[test]
fn task_status_lines_show_next_action_without_raw_json() {
    let task = TaskStatusView {
        task_id: "task-cli-lines".to_string(),
        status: "running".to_string(),
        raw_data: serde_json::json!({
            "execution_state": "background",
            "task_lifecycle": {
                "state": "background",
                "execution_state": "background",
                "checkpoint_id": "ckpt-cli",
                "next_action_kind": "resume_checkpoint",
                "reason_code": "agent_loop_soft_budget",
                "resume_due": false
            },
            "result_json": {
                "task_journal": {
                    "trace": {
                        "step_results": [
                            {
                                "step_id": "step_1",
                                "status": "ok"
                            }
                        ]
                    }
                }
            }
        }),
        result_text: Some("result-token".to_string()),
        error_text: None,
        events: vec![TaskEventLine {
            event_type: "checkpoint_created".to_string(),
            line: "type=checkpoint_created checkpoint_id=ckpt-cli".to_string(),
            fields: std::collections::BTreeMap::from([(
                "checkpoint_id".to_string(),
                "ckpt-cli".to_string(),
            )]),
        }],
    };

    let lines = task_status_lines(&task, true, &EventFilters::default());

    assert_eq!(lines[0], "task_id: task-cli-lines");
    assert_eq!(lines[1], "status: running");
    assert!(lines.contains(&"execution_state: background".to_string()));
    assert!(lines.contains(&"lifecycle_state: background".to_string()));
    assert!(lines.contains(&"next_action: resume_checkpoint".to_string()));
    assert!(lines
        .iter()
        .any(|line| line.starts_with("lifecycle: ") && line.contains("checkpoint_id=ckpt-cli")));
    assert!(lines.contains(&"result-token".to_string()));
    assert!(lines.contains(&"event: type=checkpoint_created checkpoint_id=ckpt-cli".to_string()));
    assert!(!lines.iter().any(|line| line.contains("task_journal")));
    assert!(!lines.iter().any(|line| line.contains('{')));
}

#[test]
fn task_status_lines_derive_wait_tokens_from_machine_state() {
    let task = TaskStatusView {
        task_id: "task-cli-wait".to_string(),
        status: "running".to_string(),
        raw_data: serde_json::json!({
            "execution_state": "waiting",
            "task_lifecycle": {
                "state": "waiting",
                "execution_state": "waiting",
                "resume_due": true,
                "reason_code": "async_job_running"
            }
        }),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };

    let lines = task_status_lines(&task, false, &EventFilters::default());

    assert!(lines.contains(&"next_action: resume_due".to_string()));
    assert!(lines
        .iter()
        .any(|line| line.starts_with("lifecycle: ") && line.contains("resume_due=true")));
}
