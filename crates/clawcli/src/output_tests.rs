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
            "task_budget_slice": {
                "profile": "multi_step_workspace",
                "soft_slice_ms": 900000,
                "continuation_index": 2,
                "cumulative_model_turns": 7,
                "cumulative_tool_calls": 15,
                "cumulative_elapsed_ms": 901000,
                "cumulative_input_tokens": 12000,
                "cumulative_output_tokens": 2400,
                "cumulative_cost_usd_nanos": 42000000,
                "last_decision": "checkpoint_requeue",
                "hard_ceilings": {
                    "model_turns": 256,
                    "tool_calls": 512,
                    "total_tokens": 100000000,
                    "cost_usd_nanos": 100000000000_u64,
                    "elapsed_ms": 86400000,
                    "continuations": 64,
                    "non_resumable_tool_runtime_ms": 3600000
                }
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
    assert!(lines.iter().any(|line| {
        line.starts_with("task_budget: ")
            && line.contains("profile=multi_step_workspace")
            && line.contains("soft_slice_ms=900000")
            && line.contains("model_turns=7")
            && line.contains("input_tokens=12000")
            && line.contains("cost_usd_nanos=42000000")
            && line.contains("hard_model_turns=256")
            && line.contains("hard_tool_calls=512")
            && line.contains("hard_non_resumable_tool_runtime_ms=3600000")
    }));
    assert!(lines
        .iter()
        .any(|line| line.starts_with("lifecycle: ") && line.contains("checkpoint_id=ckpt-cli")));
    assert!(lines.contains(&"result-token".to_string()));
    assert!(lines.contains(&"event: type=checkpoint_created checkpoint_id=ckpt-cli".to_string()));
    assert!(!lines.iter().any(|line| line.contains("task_journal")));
    assert!(!lines.iter().any(|line| line.contains('{')));
}

#[test]
fn task_status_lines_fall_back_to_latest_budget_event() {
    let task = TaskStatusView {
        task_id: "task-cli-budget-event".to_string(),
        status: "running".to_string(),
        raw_data: serde_json::json!({"execution_state": "running"}),
        result_text: None,
        error_text: None,
        events: vec![TaskEventLine {
            event_type: "budget_decision".to_string(),
            line: "type=budget_decision".to_string(),
            fields: std::collections::BTreeMap::from([
                ("decision".to_string(), "continue".to_string()),
                ("continuation_index".to_string(), "1".to_string()),
                ("cumulative_model_turns".to_string(), "8".to_string()),
                ("cumulative_tool_calls".to_string(), "12".to_string()),
                ("hard_model_turns".to_string(), "256".to_string()),
                ("hard_tool_calls".to_string(), "512".to_string()),
                (
                    "next_resumable_action".to_string(),
                    "resume_checkpoint".to_string(),
                ),
            ]),
        }],
    };

    let lines = task_status_lines(&task, false, &EventFilters::default());
    assert!(lines.iter().any(|line| {
        line.starts_with("task_budget: ")
            && line.contains("last_decision=continue")
            && line.contains("continuation_index=1")
            && line.contains("model_turns=8")
            && line.contains("hard_tool_calls=512")
            && line.contains("next_resumable_action=resume_checkpoint")
    }));
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
