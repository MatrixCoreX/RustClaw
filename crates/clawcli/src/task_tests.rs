use super::{
    async_final_result_value, capability_task_payload, exec_ask_payload,
    result_text_from_result_json, resume_task_payload, submit_ask, threaded_ask_payload,
    TaskResumeRequest, TaskStatusView, TaskSubmissionOptions, ThreadAskContext,
};

fn capture_submit_headers(options: TaskSubmissionOptions) -> String {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind capture server");
    let address = listener.local_addr().expect("capture address");
    let capture = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set read timeout");
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let count = stream.read(&mut buffer).expect("read request");
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..count]);
            if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let body =
            r#"{"ok":true,"data":{"task_id":"00000000-0000-4000-8000-000000000001"},"error":null}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("write response");
        String::from_utf8(bytes).expect("utf8 request")
    });

    let base_url = format!("http://{address}");
    submit_ask(&base_url, "rk-test", "inspect", options).expect("submit captured task");
    capture.join().expect("join capture server")
}

#[test]
fn task_submission_headers_keep_yolo_explicit() {
    let safe = capture_submit_headers(TaskSubmissionOptions::default()).to_ascii_lowercase();
    assert!(safe.contains("x-agent-client: clawcli"));
    assert!(!safe.contains("x-agent-execution-mode:"));

    let yolo = capture_submit_headers(TaskSubmissionOptions {
        yolo: true,
        permission_mode: None,
    })
    .to_ascii_lowercase();
    assert!(yolo.contains("x-agent-client: clawcli"));
    assert!(yolo.contains("x-agent-execution-mode: yolo"));

    for mode in [
        crate::chat_session::PermissionMode::Safe,
        crate::chat_session::PermissionMode::Ask,
    ] {
        let captured = capture_submit_headers(TaskSubmissionOptions {
            yolo: false,
            permission_mode: Some(mode),
        })
        .to_ascii_lowercase();
        assert!(captured.contains(&format!("x-agent-execution-mode: {}", mode.as_token())));
    }
}

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
                "reason_code": "task_budget_slice_exhausted",
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
        .any(|token| token == "reason_code=task_budget_slice_exhausted"));
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
fn threaded_ask_payload_binds_conversation_and_safe_session_context() {
    let first = threaded_ask_payload(
        "inspect",
        ThreadAskContext {
            conversation_id: "conversation-1",
            session_id: "session-1",
            resume_task_id: None,
            model_override: None,
            compacted_context_ref: None,
            goal_ref: None,
            attachments: &[],
        },
    );
    assert_eq!(first["conversation_id"], "conversation-1");
    assert_eq!(first["thread_id"], "conversation-1");
    assert_eq!(first["session_id"], "session-1");
    assert_eq!(first["source"], "clawcli_chat");
    assert!(first.get("resume_task_id").is_none());
    assert!(first.get("resume_trigger").is_none());

    let model = crate::chat_session::ModelOverride {
        provider: "minimax".to_string(),
        model: "MiniMax-M3".to_string(),
    };
    let attachments = [serde_json::json!({"name": "a.txt"})];
    let followup = threaded_ask_payload(
        "continue",
        ThreadAskContext {
            conversation_id: "conversation-1",
            session_id: "session-1",
            resume_task_id: Some("task-previous"),
            model_override: Some(&model),
            compacted_context_ref: Some("context:1"),
            goal_ref: Some("goal:1"),
            attachments: &attachments,
        },
    );
    assert_eq!(followup["resume_task_id"], "task-previous");
    assert_eq!(followup["resume_trigger"], "user_followup");
    assert_eq!(followup["model_selection"]["provider"], "minimax");
    assert_eq!(followup["model_selection"]["model"], "MiniMax-M3");
    assert_eq!(followup["compacted_context_ref"], "context:1");
    assert_eq!(followup["goal_ref"], "goal:1");
    assert_eq!(followup["attachments"][0]["name"], "a.txt");
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

#[test]
fn coding_exec_payload_carries_a_machine_profile_and_current_directory() {
    let payload = exec_ask_payload(
        "implement the change",
        Some("task-before"),
        Some("coding"),
        Some(std::path::Path::new("/workspace/project/src")),
    );

    assert_eq!(payload["entrypoint"], "exec");
    assert_eq!(payload["source"], "clawcli_machine");
    assert_eq!(payload["execution_profile"], "coding");
    assert_eq!(
        payload["workspace_context"]["current_working_directory"],
        "/workspace/project/src"
    );
    assert_eq!(payload["resume_task_id"], "task-before");
    assert_eq!(payload["resume_trigger"], "user_followup");

    let normalized = exec_ask_payload(
        "implement",
        None,
        Some(" coding "),
        Some(std::path::Path::new("/workspace/project")),
    );
    assert_eq!(normalized["execution_profile"], "coding");
    assert_eq!(
        normalized["workspace_context"]["current_working_directory"],
        "/workspace/project"
    );

    let ordinary = exec_ask_payload("inspect", None, None, None);
    assert!(ordinary.get("execution_profile").is_none());
    assert!(ordinary.get("workspace_context").is_none());
}

#[test]
fn task_plan_snapshot_accepts_only_the_data_only_machine_contract() {
    let mut view = TaskStatusView {
        task_id: "task-plan-cli".to_string(),
        status: "running".to_string(),
        raw_data: serde_json::json!({
            "task_plan": {
                "schema_version": 1,
                "source": "task_plan",
                "status": "ok",
                "data_only": true,
                "render_owner": "ui_cli_channel_projection",
                "plan_revision": 4,
                "steps": [
                    {"step_id": "inspect", "title": "Inspect state", "status": "completed"},
                    {"step_id": "implement", "title": "Implement change", "status": "in_progress"},
                    {"step_id": "verify", "title": "Verify result", "status": "pending"}
                ]
            }
        }),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };

    let plan = view.task_plan_snapshot().expect("valid task plan");
    assert_eq!(plan.plan_revision, 4);
    assert_eq!(plan.completed_count, 1);
    assert_eq!(plan.in_progress_step_id.as_deref(), Some("implement"));
    assert_eq!(plan.steps[2].title, "Verify result");
    assert_eq!(plan.raw["data_only"], true);

    view.raw_data["task_plan"]["data_only"] = serde_json::json!(false);
    assert!(view.task_plan_snapshot().is_none());
}
