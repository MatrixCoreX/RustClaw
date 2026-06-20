use super::{ok_step, test_state_with_registry};
use crate::agent_engine::support::load_agent_loop_guard_policy;
use crate::agent_engine::LoopState;
use crate::AgentAction;

fn test_task(task_id: &str) -> crate::ClaimedTask {
    crate::ClaimedTask {
        task_id: task_id.to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: String::new(),
    }
}

fn active_apply_recipe_loop_state() -> LoopState {
    let mut loop_state = LoopState::new(4);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        max_repairs: 3,
        saw_inspect: true,
        ..Default::default()
    };
    loop_state
}

#[test]
fn active_recipe_terminal_synthesis_after_observation_replans_before_llm() {
    let state = test_state_with_registry();
    let policy = load_agent_loop_guard_policy(&state);
    let mut loop_state = active_apply_recipe_loop_state();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "http_basic",
        "status=200\nops-repair-bad\n",
    ));
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(
        super::super::active_recipe_terminal_discussion_should_replan(
            &actions,
            &loop_state,
            &policy,
            0,
        )
    );
}

#[test]
fn active_recipe_terminal_respond_after_observation_does_not_publish() {
    let state = test_state_with_registry();
    let task = test_task("task-active-recipe-respond");
    let policy = load_agent_loop_guard_policy(&state);
    let mut loop_state = active_apply_recipe_loop_state();
    loop_state.round_no = 1;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "http_basic",
        "status=200\nops-repair-bad\n",
    ));
    let actions = vec![AgentAction::Respond {
        content: "observed validation failure".to_string(),
    }];

    let outcome = super::super::handle_respond_action(
        &state,
        &task,
        &actions,
        &mut loop_state,
        &policy,
        0,
        2,
        1,
        "respond:observed_validation_failure",
        "observed validation failure",
        None,
    );

    assert!(outcome.should_stop);
    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("recoverable_failure_continue_round")
    );
    assert!(!outcome.ended_with_user_visible_output);
    assert!(loop_state.delivery_messages.is_empty());
    assert!(loop_state.has_recoverable_failure_context);
}
