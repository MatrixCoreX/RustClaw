use super::*;

#[test]
fn workspace_write_is_required_for_generic_validation_repair() {
    let mut loop_state = LoopState::new();
    let validation = crate::execution_recipe::ActionEffect::validate();

    assert!(!validation_failure_requires_workspace_repair(
        &loop_state,
        validation
    ));

    loop_state.last_written_file_path = Some("src/lib.rs".to_string());
    assert!(validation_failure_requires_workspace_repair(
        &loop_state,
        validation
    ));
    assert!(!validation_failure_requires_workspace_repair(
        &loop_state,
        crate::execution_recipe::ActionEffect::observe()
    ));
}

#[tokio::test]
async fn failed_validation_after_workspace_write_requests_bounded_replan() {
    let state = test_state();
    let task = test_task();
    let mut loop_state = LoopState::new();
    loop_state.last_written_file_path = Some("calc_core.py".to_string());

    let outcome = handle_skill_step_success(
        &state,
        &task,
        &mut loop_state,
        "skill:run_cmd:{\"command\":\"python3 test_calc_core.py\"}",
        &ok_step("step_3", "run_cmd", "---EXIT_CODE=1"),
        3,
        1,
        "run_cmd",
        "skill",
        "",
        &serde_json::json!({"command":"python3 test_calc_core.py"}),
        "---EXIT_CODE=1",
        crate::execution_recipe::ActionEffect::validate(),
        crate::execution_recipe::ValidationObservation::Failed(
            "validation_command_exit_nonzero:exit_code=1".to_string(),
        ),
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
    assert!(loop_state.has_recoverable_failure_context);
    assert_eq!(
        loop_state
            .latest_validation_result
            .as_ref()
            .and_then(|value| value.get("status"))
            .and_then(serde_json::Value::as_str),
        Some("failed")
    );
}
