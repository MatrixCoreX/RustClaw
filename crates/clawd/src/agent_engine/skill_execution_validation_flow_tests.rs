use super::*;

#[tokio::test]
async fn validation_failure_records_failed_output_and_advances_recipe_repair() {
    let state = test_state();
    let task = test_task();
    let mut loop_state = LoopState::new();
    loop_state.round_no = 1;
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Validate,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        repair_count: 0,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: false,
        ..Default::default()
    };

    let detail = "http_expected_body_marker_missing:marker=ops-repair-ok";
    let output = "status=200\nops-repair-bad\n";
    let outcome = handle_skill_step_success(
        &state,
        &task,
        &mut loop_state,
        "skill:http_basic:{\"action\":\"get\"}",
        &ok_step("step_1", "http_basic", output),
        1,
        1,
        "http_basic",
        "skill",
        "",
        &serde_json::json!({ "action": "get", "url": "http://127.0.0.1:62078/" }),
        output,
        crate::execution_recipe::ActionEffect::validate(),
        crate::execution_recipe::ValidationObservation::Failed(detail.to_string()),
        None,
        None,
        None,
    )
    .await
    .expect("skill step outcome");

    assert!(!outcome.ended_with_user_visible_output);
    assert!(!outcome.continue_in_round);
    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("recoverable_failure_continue_round")
    );
    assert_eq!(
        loop_state.execution_recipe.phase,
        crate::execution_recipe::ExecutionRecipePhase::Repair
    );
    assert_eq!(loop_state.execution_recipe.repair_count, 1);
    assert!(loop_state.has_tool_or_skill_output);
    assert_eq!(
        loop_state
            .output_vars
            .get("failed_step.error")
            .map(String::as_str),
        Some(detail)
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("skill.http_basic.error")
            .map(String::as_str),
        Some(detail)
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("failed_step.action")
            .map(String::as_str),
        Some("skill(http_basic)")
    );
    assert!(loop_state
        .history_compact
        .iter()
        .any(|line| line.contains("validation_failed")
            && line.contains("http_expected_body_marker_missing:marker=ops-repair-ok")));
    assert!(loop_state.successful_action_fingerprints.is_empty());
    assert_eq!(loop_state.executed_step_results.len(), 1);
    assert_eq!(
        loop_state.last_recipe_progress_phase,
        Some(crate::execution_recipe::ExecutionRecipePhase::Repair)
    );
    assert!(loop_state
        .subtask_results
        .iter()
        .any(|line| line.contains("subtask#1 skill(http_basic): success")));
}

#[tokio::test]
async fn successful_skill_user_input_signal_finalizes_as_clarify_delivery() {
    let state = test_state();
    let task = test_task();
    let mut loop_state = LoopState::new();
    loop_state.round_no = 1;

    let output = "Please provide the directory path.";
    let outcome = handle_skill_step_success(
        &state,
        &task,
        &mut loop_state,
        "skill:photo_organize:{\"action\":\"prepare\"}",
        &ok_step("step_1", "photo_organize", output),
        1,
        1,
        "photo_organize",
        "skill",
        "",
        &serde_json::json!({ "action": "prepare" }),
        output,
        crate::execution_recipe::ActionEffect::observe(),
        crate::execution_recipe::ValidationObservation::Passed,
        Some(&serde_json::json!({
            "requires_user_input": true,
            "missing_argument": "source_dir"
        })),
        None,
        None,
    )
    .await
    .expect("skill step outcome");

    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("skill_requires_user_input")
    );
    assert!(outcome.ended_with_user_visible_output);
    assert!(loop_state.pending_user_input_required);
    assert_eq!(loop_state.delivery_messages, vec![output.to_string()]);
}

#[tokio::test]
async fn successful_validation_step_records_machine_result_for_closeout() {
    let state = test_state();
    let task = test_task();
    let mut loop_state = LoopState::new();
    loop_state.round_no = 1;
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Validate,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        ..Default::default()
    };

    handle_skill_step_success(
        &state,
        &task,
        &mut loop_state,
        "skill:run_cmd:{\"command\":\"cargo check -p clawd\"}",
        &ok_step("step_3", "run_cmd", "validation ok"),
        3,
        2,
        "run_cmd",
        "skill",
        "command=cargo check -p clawd",
        &serde_json::json!({ "command": "cargo check -p clawd" }),
        "validation ok",
        crate::execution_recipe::ActionEffect::validate(),
        crate::execution_recipe::ValidationObservation::Passed,
        None,
        None,
        None,
    )
    .await
    .expect("skill step outcome");

    let validation = loop_state
        .latest_validation_result
        .as_ref()
        .expect("validation result");
    assert_eq!(
        validation
            .get("status_code")
            .and_then(serde_json::Value::as_str),
        Some("validation_passed")
    );
    assert_eq!(
        validation.get("skill").and_then(serde_json::Value::as_str),
        Some("run_cmd")
    );
    assert_eq!(
        validation
            .get("global_step")
            .and_then(serde_json::Value::as_u64),
        Some(3)
    );
}

#[test]
fn validation_result_is_recipe_independent_and_command_scoped() {
    let mut loop_state = LoopState::new();

    record_latest_validation_result(
        &mut loop_state,
        "run_cmd",
        &serde_json::json!({"command": "cargo test -p clawd"}),
        4,
        1,
        "passed",
        "validation_passed",
        crate::execution_recipe::ActionEffect::validate(),
    );

    let validation = loop_state
        .latest_validation_result
        .as_ref()
        .expect("validation result outside an execution recipe");
    assert_eq!(
        validation
            .get("verification_scope")
            .and_then(serde_json::Value::as_str),
        Some("command")
    );
    assert_eq!(
        validation.get("status").and_then(serde_json::Value::as_str),
        Some("passed")
    );

    invalidate_latest_validation_after_mutation_attempt(
        &mut loop_state,
        crate::execution_recipe::ActionEffect::mutate(),
    );
    assert!(loop_state.latest_validation_result.is_none());
}

#[test]
fn successful_test_process_resolves_inconclusive_text_observation() {
    let args = serde_json::json!({"command": "python3 test_calc_core.py"});
    let observation = validation_observation_with_process_status(
        "run_cmd",
        &args,
        true,
        crate::execution_recipe::ValidationObservation::Inconclusive,
    );
    assert_eq!(
        observation,
        crate::execution_recipe::ValidationObservation::Passed
    );

    let failed_process = validation_observation_with_process_status(
        "run_cmd",
        &args,
        false,
        crate::execution_recipe::ValidationObservation::Inconclusive,
    );
    assert_eq!(
        failed_process,
        crate::execution_recipe::ValidationObservation::Inconclusive
    );
}

#[tokio::test]
async fn run_cmd_validation_failed_marker_advances_recipe_repair_without_success_fingerprint() {
    let state = test_state();
    let task = test_task();
    let mut loop_state = LoopState::new();
    loop_state.round_no = 2;
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Validate,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        repair_count: 0,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: false,
        ..Default::default()
    };

    let output = "VALIDATION_FAILED\n";
    let outcome = handle_skill_step_success(
        &state,
        &task,
        &mut loop_state,
        "skill:run_cmd:{\"command\":\"curl\"}",
        &ok_step("step_2", "run_cmd", output),
        2,
        1,
        "run_cmd",
        "skill",
        "",
        &serde_json::json!({ "command": "curl -s http://127.0.0.1:62078/" }),
        output,
        crate::execution_recipe::ActionEffect::validate(),
        crate::execution_recipe::ValidationObservation::Failed("VALIDATION_FAILED".to_string()),
        None,
        None,
        None,
    )
    .await
    .expect("skill step outcome");

    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("recoverable_failure_continue_round")
    );
    assert_eq!(
        loop_state.execution_recipe.phase,
        crate::execution_recipe::ExecutionRecipePhase::Repair
    );
    assert_eq!(loop_state.execution_recipe.repair_count, 1);
    assert!(loop_state.successful_action_fingerprints.is_empty());
    assert!(loop_state
        .history_compact
        .iter()
        .any(|line| line.contains("skill=run_cmd")
            && line.contains("validation_failed=VALIDATION_FAILED")));
    assert_eq!(
        loop_state
            .output_vars
            .get("failed_step.error")
            .map(String::as_str),
        Some("VALIDATION_FAILED")
    );
    assert!(loop_state
        .subtask_results
        .iter()
        .any(|line| line.contains("subtask#2 skill(run_cmd): success")));
}
