use super::{async_final_result_value, result_text_from_result_json, TaskStatusView};

#[test]
fn lifecycle_summary_tokens_include_budget_snapshot() {
    let view = TaskStatusView {
        task_id: "task-budget".to_string(),
        status: "waiting".to_string(),
        raw_data: serde_json::json!({
            "task_lifecycle": {
                "state": "waiting",
                "execution_state": "waiting",
                "checkpoint_id": "ckpt-budget",
                "heartbeat_at": 1781800000,
                "attempt_id": 2,
                "reason_code": "agent_loop_max_rounds",
                "budget": {
                    "round": 2,
                    "step": 3,
                    "llm_calls": 4,
                    "tool_calls": 1,
                    "elapsed_ms": 1200,
                    "llm_elapsed_ms": 900,
                    "tool_elapsed_ms": 300
                }
            }
        }),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };

    let tokens = view.lifecycle_summary_tokens();

    assert_eq!(view.execution_state(), Some("waiting"));
    assert!(tokens
        .iter()
        .any(|token| token == "execution_state=waiting"));
    assert!(tokens
        .iter()
        .any(|token| token == "heartbeat_at=1781800000"));
    assert!(tokens.iter().any(|token| token == "attempt_id=2"));
    assert!(tokens
        .iter()
        .any(|token| token == "reason_code=agent_loop_max_rounds"));
    assert!(tokens.iter().any(|token| token == "budget.round=2"));
    assert!(tokens.iter().any(|token| token == "budget.llm_calls=4"));
    assert!(tokens
        .iter()
        .any(|token| token == "budget.tool_elapsed_ms=300"));
}

#[test]
fn lifecycle_summary_accepts_api_lifecycle_field() {
    let view = TaskStatusView {
        task_id: "task-api-lifecycle".to_string(),
        status: "succeeded".to_string(),
        raw_data: serde_json::json!({
            "lifecycle": {
                "state": "completed",
                "execution_state": "completed",
                "reason_code": "async_poll_completed"
            }
        }),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };

    let tokens = view.lifecycle_summary_tokens();

    assert_eq!(view.lifecycle_state(), Some("completed"));
    assert_eq!(view.execution_state(), Some("completed"));
    assert!(view.is_terminal());
    assert!(tokens.iter().any(|token| token == "state=completed"));
    assert!(tokens
        .iter()
        .any(|token| token == "execution_state=completed"));
    assert!(tokens
        .iter()
        .any(|token| token == "reason_code=async_poll_completed"));
}

#[test]
fn async_final_result_value_extracts_terminal_output() {
    let result_json = serde_json::json!({
        "task_lifecycle": {
            "resume_executor_result_projection": {
                "final_result_json": {
                    "exit_code": 0,
                    "stdout": "ASYNC_STDOUT_TOKEN\n",
                    "output": "ASYNC_OUTPUT_TOKEN\n"
                }
            }
        }
    });

    let final_result = async_final_result_value(&result_json).expect("async final result");

    assert_eq!(final_result["exit_code"], 0);
    assert_eq!(final_result["output"], "ASYNC_OUTPUT_TOKEN\n");
    assert_eq!(
        result_text_from_result_json(&result_json).as_deref(),
        Some("ASYNC_OUTPUT_TOKEN\n")
    );
}
