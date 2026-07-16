use super::{
    async_final_result_value, capability_task_payload, result_text_from_result_json,
    resume_task_payload, threaded_ask_payload, TaskResumeRequest, TaskStatusView,
};

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
                "resume_directive": "run_next_planner_round",
                "heartbeat_at": 1781800000,
                "attempt_id": 2,
                "reason_code": "agent_loop_max_rounds",
                "last_successful_evidence_ref": "step_3:evidence:1",
                "evidence_ref_count": 2,
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
    assert!(tokens
        .iter()
        .any(|token| token == "resume_directive=run_next_planner_round"));
    assert!(tokens
        .iter()
        .any(|token| token == "last_successful_evidence_ref=step_3:evidence:1"));
    assert!(tokens.iter().any(|token| token == "evidence_ref_count=2"));
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
fn needs_user_is_a_background_wait_state() {
    let view = TaskStatusView {
        task_id: "task-needs-user".to_string(),
        status: "running".to_string(),
        raw_data: serde_json::json!({"execution_state": "needs_user"}),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };

    assert!(view.is_background_waiting());
    assert!(!view.is_terminal());
}

#[test]
fn pending_approval_request_id_uses_only_the_machine_resume_contract() {
    let mut view = TaskStatusView {
        task_id: "task-approval".to_string(),
        status: "failed".to_string(),
        raw_data: serde_json::json!({
            "result_json": {
                "resume_context": {
                    "approval_request": {
                        "status": "pending",
                        "request_id": " approval-1 "
                    }
                }
            }
        }),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };

    assert_eq!(view.pending_approval_request_id(), Some("approval-1"));
    view.raw_data["result_json"]["resume_context"]["approval_request"]["status"] =
        serde_json::json!("denied");
    assert_eq!(view.pending_approval_request_id(), None);
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

#[test]
fn resume_payload_only_carries_explicit_approval_grant() {
    let ordinary = resume_task_payload(
        "task-1",
        TaskResumeRequest {
            resume_reason: Some("user_continue"),
            ..Default::default()
        },
    );
    assert_eq!(ordinary["task_id"], "task-1");
    assert!(ordinary.get("approval_decision").is_none());
    assert!(ordinary.get("approval_request_id").is_none());

    let approved = resume_task_payload(
        "task-1",
        TaskResumeRequest {
            approval_request_id: Some(" approval-1 "),
            approval_decision: Some("approve_once"),
            ..Default::default()
        },
    );
    assert_eq!(approved["approval_request_id"], "approval-1");
    assert_eq!(approved["approval_decision"], "approve_once");

    let denied = resume_task_payload(
        "task-1",
        TaskResumeRequest {
            approval_request_id: Some("approval-1"),
            approval_decision: Some("deny"),
            ..Default::default()
        },
    );
    assert_eq!(denied["approval_decision"], "deny");
    assert!(denied.get("approve").is_none());
}

#[test]
fn threaded_ask_payload_binds_thread_and_only_adds_resume_for_followups() {
    let first = threaded_ask_payload("inspect", "thread-1", "session-1", None);
    assert_eq!(first["thread_id"], "thread-1");
    assert_eq!(first["session_id"], "session-1");
    assert_eq!(first["source"], "clawcli_chat");
    assert!(first.get("resume_task_id").is_none());
    assert!(first.get("resume_trigger").is_none());

    let followup = threaded_ask_payload("continue", "thread-1", "session-1", Some("task-previous"));
    assert_eq!(followup["resume_task_id"], "task-previous");
    assert_eq!(followup["resume_trigger"], "user_followup");
}

#[test]
fn capability_task_payload_uses_the_verified_machine_entrypoint() {
    let payload = capability_task_payload(
        "workspace.diff",
        serde_json::json!({"checkpoint_id": "checkpoint-1", "paths": ["src/lib.rs"]}),
    );

    assert_eq!(payload["entrypoint"], "run_capability");
    assert_eq!(payload["capability"], "workspace.diff");
    assert_eq!(payload["source"], "clawcli_machine");
    assert_eq!(payload["args"]["checkpoint_id"], "checkpoint-1");
    assert_eq!(payload["args"]["paths"][0], "src/lib.rs");
    assert!(payload.get("text").is_none());
}
