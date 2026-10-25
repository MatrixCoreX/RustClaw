use super::*;

fn step(id: &str, output: &str) -> crate::executor::StepExecutionResult {
    crate::executor::StepExecutionResult {
        step_id: id.to_string(),
        skill: "fixture".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(output.to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    }
}

#[test]
fn merge_preserves_each_isolated_read_delta_in_order() {
    let baseline = LoopState::new();
    let mut target = baseline.clone();
    let mut first = baseline.clone();
    first.tool_calls_total = 1;
    first.total_steps_executed = 1;
    first.executed_step_results.push(step("step_1", "first"));
    first
        .successful_action_fingerprints
        .insert("first".to_string(), 1);
    let mut second = baseline.clone();
    second.tool_calls_total = 1;
    second.total_steps_executed = 1;
    second.executed_step_results.push(step("step_2", "second"));
    second
        .successful_action_fingerprints
        .insert("second".to_string(), 1);

    merge_child_read_state(&mut target, &baseline, &first);
    merge_child_read_state(&mut target, &baseline, &second);

    assert_eq!(target.tool_calls_total, 2);
    assert_eq!(target.total_steps_executed, 2);
    assert_eq!(target.executed_step_results.len(), 2);
    assert_eq!(
        target.executed_step_results[0].output.as_deref(),
        Some("first")
    );
    assert_eq!(
        target.executed_step_results[1].output.as_deref(),
        Some("second")
    );
    assert_eq!(target.successful_action_fingerprints.len(), 2);
}

#[test]
fn read_batch_width_respects_runtime_skill_capacity() {
    assert_eq!(bounded_parallel_read_width(4, 1), 1);
    assert_eq!(bounded_parallel_read_width(4, 2), 2);
    assert_eq!(bounded_parallel_read_width(2, 8), 2);
    assert_eq!(bounded_parallel_read_width(3, 0), 1);
}

#[test]
fn parallel_dispatch_future_keeps_nested_runner_state_off_the_thread_stack() {
    let state = AppState::test_default_with_fixture_provider();
    let task = ClaimedTask {
        claim_attempt: 1,
        task_id: "batch-stack-budget".into(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "ui".into(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".into(),
        payload_json: "{}".into(),
    };
    let policy = super::super::support::load_agent_loop_guard_policy(&state);
    let mut loop_state = LoopState::new();
    let future = dispatch_independent_read_batch(
        &state,
        &task,
        "inspect",
        "inspect",
        &[],
        &[],
        &mut loop_state,
        &policy,
        &[],
        0,
        None,
    );
    let bytes = std::mem::size_of_val(&future);
    assert!(
        bytes < 128 * 1024,
        "parallel dispatcher future grew to {bytes} bytes"
    );
}
