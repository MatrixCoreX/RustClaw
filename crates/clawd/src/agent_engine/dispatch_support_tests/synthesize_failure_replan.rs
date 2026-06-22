use super::*;

#[test]
fn synthesize_failure_observed_facts_include_recent_execution_outputs() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "alpha.md\nbeta.md\n"));

    let facts = synthesize_failure_observed_facts(&loop_state, "last_output");
    let joined = facts.join("\n");

    assert!(joined.contains("synthesize_refs: last_output"));
    assert!(joined.contains("observed_steps_count: 1"));
    assert!(joined.contains("skill=list_dir"));
    assert!(joined.contains("alpha.md"));
}

#[test]
fn synthesize_failure_after_observation_allows_one_replan() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "read_file",
        "large file excerpt...\n",
    ));

    assert!(synthesize_failure_should_replan(&loop_state));

    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some("no publishable answer".to_string()),
        started_at: 0,
        finished_at: 0,
    });

    assert!(!synthesize_failure_should_replan(&loop_state));
}
