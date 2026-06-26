use super::{exec_summary_json, ExecWaitOutcome};

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

    let summary = exec_summary_json(&task, ExecWaitOutcome::Background);

    assert_eq!(summary["task_id"], "task-exec");
    assert_eq!(summary["status"], "running");
    assert_eq!(summary["lifecycle_state"], "background");
    assert_eq!(summary["outcome"], "background");
    assert_eq!(summary["terminal"], false);
    assert_eq!(summary["lifecycle"]["checkpoint_id"], "ckpt-exec");
}
