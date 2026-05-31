use super::build_attempt_ledger_compact;

#[test]
fn attempt_ledger_renders_failed_step_with_retry_hint() {
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: "s1".to_string(),
            skill: "fs_search".to_string(),
            status: crate::executor::StepExecutionStatus::Error,
            output: None,
            error: Some("__RC_SKILL_ERROR__:{\"error_kind\":\"not_found\"}".to_string()),
            started_at: 1,
            finished_at: 2,
        });
    let ledger = build_attempt_ledger_compact(&loop_state);
    assert!(ledger.contains("\"tool_or_skill\": \"fs_search\""));
    assert!(ledger.contains("\"error_kind\": \"not_found\""));
    assert!(ledger.contains("\"retryable\": true"));
    assert!(ledger.contains("do_not_retry_same_target"));
}

#[test]
fn attempt_ledger_prefers_recorded_args_summary() {
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    super::record_attempt(
        &mut loop_state,
        "run_cmd",
        "command=pwd cwd=/tmp",
        crate::executor::StepExecutionStatus::Ok,
        "/tmp",
        None,
        "completed",
    );
    let ledger = build_attempt_ledger_compact(&loop_state);
    assert!(ledger.contains("\"args_summary\": \"command=pwd cwd=/tmp\""));
    assert!(ledger.contains("\"retryable\": false"));
    assert!(!ledger.contains("not_recorded_in_step_result"));
}

#[test]
fn attempt_ledger_records_verifier_retry_instruction() {
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    super::record_attempt_with_retry_instruction(
        &mut loop_state,
        "answer_verifier",
        "missing_evidence_fields=content_excerpt",
        crate::executor::StepExecutionStatus::Error,
        "only returned step status",
        Some("answer_incomplete"),
        "answer lacks project article content",
        Some("Read README.md and Cargo.toml, then synthesize the requested article."),
    );
    let ledger = build_attempt_ledger_compact(&loop_state);
    assert!(ledger.contains("\"tool_or_skill\": \"answer_verifier\""));
    assert!(ledger.contains("\"retry_instruction\""));
    assert!(ledger.contains("Read README.md and Cargo.toml"));
}

#[test]
fn attempt_ledger_marks_policy_block_non_retryable() {
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    super::record_attempt(
        &mut loop_state,
        "db_basic",
        "sql=DROP TABLE users",
        crate::executor::StepExecutionStatus::Error,
        "",
        Some("unsafe_sql"),
        "unsafe SQL requires refusal or confirmation",
    );
    let ledger = build_attempt_ledger_compact(&loop_state);
    assert!(ledger.contains("\"error_kind\": \"unsafe_sql\""));
    assert!(ledger.contains("\"retryable\": false"));
}

#[test]
fn attempt_ledger_marks_contract_rejections_non_retryable() {
    for (kind, hint) in [
        (
            "contract_action_rejected",
            "do_not_repeat_rejected_action; choose_contract_allowed_action_or_replan",
        ),
        (
            "contract_arg_rejected",
            "do_not_repeat_missing_target_binding; bind_target_or_ask_for_clarification",
        ),
    ] {
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        super::record_attempt(
            &mut loop_state,
            "fs_basic",
            "action=read_text_range",
            crate::executor::StepExecutionStatus::Error,
            "",
            Some(kind),
            "contract preflight rejected the action",
        );
        let ledger = build_attempt_ledger_compact(&loop_state);
        assert!(ledger.contains(&format!("\"error_kind\": \"{kind}\"")));
        assert!(ledger.contains("\"retryable\": false"));
        assert!(ledger.contains(hint));
    }
}

#[test]
fn attempt_ledger_exposes_contract_policy_decision_for_repair_prompt() {
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let err = crate::skills::structured_skill_error_from_parts(
        "run_cmd",
        "contract_action_rejected",
        "action `run_cmd` is rejected by contract `file_names`",
        None,
        Some(serde_json::json!({
            "decision": "rejected_not_allowed",
            "action": "run_cmd",
            "contract_match": "file_names",
            "preferred_actions": ["fs_basic.list_dir"],
            "required_evidence": ["candidates"],
            "final_answer_shape": "name_list",
        })),
    );
    loop_state
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: "s1".to_string(),
            skill: "run_cmd".to_string(),
            status: crate::executor::StepExecutionStatus::Error,
            output: None,
            error: Some(err),
            started_at: 1,
            finished_at: 2,
        });

    let ledger = build_attempt_ledger_compact(&loop_state);

    assert!(ledger.contains("\"contract_policy\""));
    assert!(ledger.contains("\"decision\": \"rejected_not_allowed\""));
    assert!(ledger.contains("\"preferred_actions\""));
    assert!(ledger.contains("fs_basic.list_dir"));
}

#[test]
fn attempt_ledger_marks_terminal_failures_non_retryable() {
    for kind in [
        "confirmed_not_found",
        "invalid_credentials",
        "credential_missing",
        "auth_failed",
        "missing_input",
    ] {
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        super::record_attempt(
            &mut loop_state,
            "tool",
            "target=x",
            crate::executor::StepExecutionStatus::Error,
            "",
            Some(kind),
            "terminal failure",
        );
        let ledger = build_attempt_ledger_compact(&loop_state);
        assert!(ledger.contains(&format!("\"error_kind\": \"{kind}\"")));
        assert!(ledger.contains("\"retryable\": false"));
    }
}
