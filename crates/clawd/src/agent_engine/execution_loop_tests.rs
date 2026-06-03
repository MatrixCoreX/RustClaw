use super::{
    action_counts_as_tool_call, action_effect_is_repeatable_for_active_recipe,
    capture_round_progress_snapshot, check_repeat_action_guard, finalize_execute_round_outcome,
};

#[test]
fn observed_output_alone_does_not_mark_plan_exhausted_user_visible() {
    let loop_state = super::LoopState::new(2);
    let snapshot = capture_round_progress_snapshot(&loop_state);
    let outcome = finalize_execute_round_outcome(&loop_state, &snapshot, 1, 1, false, None);
    assert!(outcome.stop_signal.is_none());
}

#[test]
fn explicit_user_visible_output_marks_plan_exhausted() {
    let loop_state = super::LoopState::new(2);
    let snapshot = capture_round_progress_snapshot(&loop_state);
    let outcome = finalize_execute_round_outcome(&loop_state, &snapshot, 1, 1, true, None);
    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("plan_exhausted_user_visible")
    );
}

#[test]
fn max_tool_call_budget_counts_only_external_calls() {
    assert!(action_counts_as_tool_call(&crate::AgentAction::CallTool {
        tool: "system_basic".to_string(),
        args: serde_json::json!({})
    }));
    assert!(action_counts_as_tool_call(&crate::AgentAction::CallSkill {
        skill: "fs_basic".to_string(),
        args: serde_json::json!({})
    }));
    assert!(action_counts_as_tool_call(
        &crate::AgentAction::CallCapability {
            capability: "fs_basic.read_text_range".to_string(),
            args: serde_json::json!({})
        }
    ));
    assert!(!action_counts_as_tool_call(
        &crate::AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()]
        }
    ));
    assert!(!action_counts_as_tool_call(&crate::AgentAction::Respond {
        content: "done".to_string()
    }));
}

#[test]
fn active_recipe_allows_repeating_successful_observe_effect() {
    let recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
        crate::execution_recipe::ExecutionRecipeSpec {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            ..Default::default()
        },
    );
    assert!(action_effect_is_repeatable_for_active_recipe(
        recipe,
        crate::execution_recipe::ActionEffect::observe(),
    ));
    assert!(action_effect_is_repeatable_for_active_recipe(
        recipe,
        crate::execution_recipe::ActionEffect::validate(),
    ));
    assert!(!action_effect_is_repeatable_for_active_recipe(
        recipe,
        crate::execution_recipe::ActionEffect::mutate(),
    ));
}

#[test]
fn done_recipe_does_not_allow_repeating_successful_observe_effect() {
    let mut recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
        crate::execution_recipe::ExecutionRecipeSpec {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            ..Default::default()
        },
    );
    recipe.phase = crate::execution_recipe::ExecutionRecipePhase::Done;
    assert!(!action_effect_is_repeatable_for_active_recipe(
        recipe,
        crate::execution_recipe::ActionEffect::observe(),
    ));
}

#[test]
fn repeat_guard_allows_repeated_respond_delivery() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = crate::ClaimedTask {
        task_id: "task-repeat-respond".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "telegram".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut loop_state = super::LoopState::new(2);
    let action = crate::AgentAction::Respond {
        content: "final answer".to_string(),
    };
    let fingerprint = "respond:final answer".to_string();
    loop_state
        .successful_action_fingerprints
        .insert(fingerprint.clone(), 1);
    let policy = super::AgentLoopGuardPolicy {
        max_steps: 8,
        max_rounds: 2,
        max_tool_calls: 12,
        recoverable_failure_extra_rounds: 0,
        repeat_action_limit: 1,
        no_progress_limit: 1,
        multi_round_enabled: true,
        answer_verifier_retry_limit: 1,
        fast_read: Default::default(),
        grounded_summary: Default::default(),
        multi_step_workspace: Default::default(),
        ops_closed_loop: Default::default(),
    };

    assert_eq!(
        check_repeat_action_guard(
            &state,
            &task,
            &mut loop_state,
            &policy,
            &action,
            &fingerprint,
            1,
        ),
        None
    );
}

#[test]
fn repeat_guard_blocks_identical_non_respond_after_limit() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = crate::ClaimedTask {
        task_id: "task-repeat-run-cmd".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "telegram".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut loop_state = super::LoopState::new(2);
    let action = crate::AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({"command": "pwd"}),
    };
    let fingerprint = "skill:run_cmd:{\"command\":\"pwd\"}".to_string();
    let policy = super::AgentLoopGuardPolicy {
        max_steps: 8,
        max_rounds: 2,
        max_tool_calls: 12,
        recoverable_failure_extra_rounds: 0,
        repeat_action_limit: 1,
        no_progress_limit: 1,
        multi_round_enabled: true,
        answer_verifier_retry_limit: 1,
        fast_read: Default::default(),
        grounded_summary: Default::default(),
        multi_step_workspace: Default::default(),
        ops_closed_loop: Default::default(),
    };

    assert_eq!(
        check_repeat_action_guard(
            &state,
            &task,
            &mut loop_state,
            &policy,
            &action,
            &fingerprint,
            1,
        ),
        None
    );
    assert_eq!(
        check_repeat_action_guard(
            &state,
            &task,
            &mut loop_state,
            &policy,
            &action,
            &fingerprint,
            2,
        )
        .as_deref(),
        Some("repeat_action_limit")
    );
}
