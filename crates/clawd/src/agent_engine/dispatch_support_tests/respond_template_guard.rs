use crate::agent_engine::LoopState;
use crate::executor::StepExecutionStatus;
use crate::AgentAction;

#[test]
fn unresolved_runtime_template_respond_triggers_replan_not_visible_output() {
    assert_unresolved_runtime_template_respond_replans("{{last_output}}");
}

#[test]
fn redacted_runtime_template_respond_triggers_replan_not_visible_output() {
    let redacted = crate::visible_text::sanitize_user_visible_text("{{last_output}}");
    assert_direct_template_guard_replans("{{last_output}}", &redacted);
}

fn test_task() -> crate::ClaimedTask {
    crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "task-unresolved-runtime-template-respond".to_string(),
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

fn assert_unresolved_runtime_template_respond_replans(content: &str) {
    let state = super::test_state_with_registry();
    let task = test_task();
    let policy = crate::agent_engine::support::load_agent_loop_guard_policy(&state);
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;
    let actions = vec![AgentAction::Respond {
        content: content.to_string(),
    }];

    let outcome = super::super::handle_respond_action(
        &state,
        &task,
        &actions,
        &mut loop_state,
        &policy,
        0,
        1,
        1,
        "respond:unresolved_runtime_template",
        content,
        None,
    );

    assert_replan_outcome(&outcome, &loop_state);
}

fn assert_direct_template_guard_replans(content: &str, resolved_text: &str) {
    let state = super::test_state_with_registry();
    let task = test_task();
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;
    let outcome =
        super::super::respond_template_guard::unresolved_runtime_template_respond_outcome(
            &state,
            &task,
            &mut loop_state,
            1,
            1,
            content,
            resolved_text,
        )
        .expect("template guard outcome");

    assert_replan_outcome(&outcome, &loop_state);
}

fn assert_replan_outcome(outcome: &super::super::RespondActionOutcome, loop_state: &LoopState) {
    assert!(outcome.should_stop);
    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("recoverable_failure_continue_round")
    );
    assert!(!outcome.ended_with_user_visible_output);
    assert!(loop_state.delivery_messages.is_empty());
    assert!(loop_state.has_recoverable_failure_context);
    assert!(loop_state.executed_step_results.iter().any(|step| {
        step.skill == "respond"
            && step.status == StepExecutionStatus::Error
            && step
                .error
                .as_deref()
                .is_some_and(|err| err.contains("runtime_template"))
    }));
    assert!(loop_state.task_observations.iter().any(|observation| {
        observation.get("kind").and_then(|value| value.as_str()) == Some("planner_quality_signal")
            && observation.get("signal").and_then(|value| value.as_str())
                == Some("unresolved_runtime_template_respond")
            && observation
                .get("recoverable")
                .and_then(|value| value.as_bool())
                == Some(true)
    }));
}
