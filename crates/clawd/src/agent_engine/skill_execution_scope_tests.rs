use super::*;

#[tokio::test]
async fn successful_external_workspace_step_records_scope_progress() {
    let state = test_state();
    let task = test_task();
    let mut loop_state = LoopState::new();
    loop_state.round_no = 1;
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: false,
        max_repairs: 2,
        saw_inspect: true,
        ..Default::default()
    };

    handle_skill_step_success(
        &state,
        &task,
        &mut loop_state,
        "skill:read_file:{\"path\":\"/opt/other-project/main.rs\"}",
        &ok_step("step_3", "read_file", "fn main() {}\n"),
        3,
        1,
        "read_file",
        "skill",
        "",
        &serde_json::json!({ "path": "/opt/other-project/main.rs" }),
        "fn main() {}\n",
        crate::execution_recipe::ActionEffect::observe(),
        crate::execution_recipe::ValidationObservation::Passed,
        None,
        None,
        Some("/opt/other-project/main.rs"),
    )
    .await
    .expect("skill step outcome");

    assert!(loop_state.execution_recipe.saw_external_target);
}

#[tokio::test]
async fn successful_greenfield_creation_step_records_scope_progress() {
    let state = test_state();
    let task = test_task();
    let mut loop_state = LoopState::new();
    loop_state.round_no = 1;
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        saw_inspect: true,
        ..Default::default()
    };

    handle_skill_step_success(
        &state,
        &task,
        &mut loop_state,
        "skill:write_file:{\"path\":\"tools/demo/main.rs\"}",
        &ok_step("step_4", "write_file", "ok"),
        4,
        1,
        "write_file",
        "skill",
        "",
        &serde_json::json!({ "path": "tools/demo/main.rs", "content": "fn main() {}\n" }),
        "ok",
        crate::execution_recipe::ActionEffect::mutate(),
        crate::execution_recipe::ValidationObservation::Passed,
        None,
        None,
        None,
    )
    .await
    .expect("skill step outcome");

    assert!(loop_state.execution_recipe.saw_greenfield_creation);
}
