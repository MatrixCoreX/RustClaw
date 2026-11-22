use super::*;

fn fixture() -> (ClaimedTask, LoopState) {
    let task = ClaimedTask {
        claim_attempt: 1,
        task_id: "evidence-repair-child".into(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "ui".into(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".into(),
        payload_json: json!({"task_role":"subagent_child","child_task_contract":{
            "result_contract":{"schema_version":1,"output_format":"machine_json",
            "required_keys":["name","evidence"],"allow_partial":false,
            "require_evidence":false,"evidence_field":"evidence"}}})
        .to_string(),
    };
    let mut state = LoopState::new();
    state.round_no = 2;
    state.tool_calls_total = 1;
    state.total_steps_executed = 1;
    state
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: "step_1".into(),
            skill: "fs_basic".into(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some("observed bytes".into()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });
    state
        .successful_action_fingerprints
        .insert("read-1".into(), 1);
    state.last_user_visible_respond =
        Some(json!({"name":"Panel","evidence":["path: source.rs"]}).to_string());
    (task, state)
}

#[test]
fn reference_repair_preserves_effects_and_budgets_and_uses_only_one_existing_allowance() {
    let (task, mut state) = fixture();
    let receipts = state.executed_step_results.clone();
    let fingerprints = state.successful_action_fingerprints.clone();
    assert!(prepare_child_result_evidence_replan(&task, &mut state));
    assert_eq!(state.round_no, 2);
    assert_eq!(state.tool_calls_total, 1);
    assert_eq!(state.total_steps_executed, 1);
    assert_eq!(state.successful_action_fingerprints, fingerprints);
    assert_eq!(
        serde_json::to_value(&state.executed_step_results).unwrap(),
        serde_json::to_value(receipts).unwrap()
    );
    assert_eq!(
        state.task_observations[0]["allowed_refs"],
        json!(["step:step_1"])
    );
    assert_eq!(state.task_observations[0]["required_dispatches"], json!([]));
    assert_eq!(
        state.task_observations[0]["next_action"],
        "rewrite_result_from_existing_observations"
    );
    assert!(state.last_user_visible_respond.is_none());
    state.last_user_visible_respond =
        Some(json!({"name":"Panel","evidence":["step:missing"]}).to_string());
    assert!(!prepare_child_result_evidence_replan(&task, &mut state));
}

#[test]
fn repaired_reference_passes_the_unchanged_strict_terminal_contract() {
    let (task, mut state) = fixture();
    assert!(prepare_child_result_evidence_replan(&task, &mut state));
    let answer = json!({"name":"Panel","evidence":["step:step_1"]}).to_string();
    let payload: Value = serde_json::from_str(&task.payload_json).unwrap();
    assert!(crate::repo::child_tasks::child_evidence_refs_valid(
        &payload,
        &answer,
        &["step_1"]
    ));
    for refs in [
        json!([]),
        json!(["step:missing"]),
        json!(["path: source.rs"]),
    ] {
        assert!(!crate::repo::child_tasks::child_evidence_refs_valid(
            &payload,
            &json!({"name":"Panel","evidence":refs}).to_string(),
            &["step_1"]
        ));
    }
    state.last_user_visible_respond = Some(answer);
    assert!(!prepare_child_result_evidence_replan(&task, &mut state));
}

#[test]
fn ordinary_results_and_valid_child_references_do_not_start_repair() {
    let (mut task, mut state) = fixture();
    task.payload_json = "{}".into();
    assert!(!prepare_child_result_evidence_replan(&task, &mut state));
    let (task, mut state) = fixture();
    state.last_user_visible_respond =
        Some(json!({"name":"Panel","evidence":["step:step_1"]}).to_string());
    assert!(!prepare_child_result_evidence_replan(&task, &mut state));
}

#[test]
fn missing_fields_failed_steps_and_pending_controls_cannot_be_repaired_into_success() {
    let (task, original) = fixture();
    for case in 0..5 {
        let mut state = original.clone();
        match case {
            0 => {
                state.last_user_visible_respond =
                    Some(json!({"evidence":["path: source.rs"]}).to_string())
            }
            1 => {
                state.executed_step_results[0].status = crate::executor::StepExecutionStatus::Error
            }
            2 => state.pending_user_input_required = true,
            3 => state.pending_resume_failure_user_error = Some("unavailable".into()),
            _ => {
                state
                    .output_vars
                    .insert(ANSWER_VERIFIER_EVIDENCE_REPLAN_USED.into(), "true".into());
            }
        }
        assert!(!prepare_child_result_evidence_replan(&task, &mut state));
        assert!(state.task_observations.is_empty());
    }
}
