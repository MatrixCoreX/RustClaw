use super::TaskStatusView;

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
