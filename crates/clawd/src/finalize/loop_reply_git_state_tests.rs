use super::*;

#[test]
fn git_status_summary_defers_to_synthesis_instead_of_raw_passthrough() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), "git_basic".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "git_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("exit=0\n## main...origin/main\n M Cargo.toml\n?? new_file.txt\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.resolved_intent = "检查当前仓库是否有未提交改动，用一句话告诉我".to_string();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(
        direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context)).is_none(),
        "git status summary should be synthesized from observed evidence"
    );

    assert!(
        direct_non_builtin_skill_raw_answer(&state, &loop_state, Some(&agent_run_context))
            .is_none(),
        "one-sentence summary should not raw-passthrough git status output"
    );
}

#[test]
fn git_repository_state_one_sentence_defers_direct_structured_answer() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), "git_basic".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "git_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("exit=0\n## main...origin/main\n M Cargo.toml\n?? new_file.txt\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("该仓库有 8 个文件存在未提交改动。".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent = "检查当前仓库是否有未提交改动，用一句话告诉我".to_string();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GitRepositoryState;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(
        direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context)).is_none(),
        "git repository state one-sentence delivery should be synthesized"
    );
}

#[test]
fn scalar_git_log_does_not_use_non_builtin_raw_passthrough() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), "git_basic".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "git_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("exit=0\n09342a6a fix: expose nl execution and locator flows\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let mut route = scalar_route_result();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(
        direct_non_builtin_skill_raw_answer(&state, &loop_state, Some(&agent_run_context))
            .is_none(),
        "scalar git requests should use structured extraction or synthesis, not raw passthrough"
    );
}
