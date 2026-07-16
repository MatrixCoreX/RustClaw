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
fn git_repository_state_free_summary_defers_direct_structured_answer() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), "git_basic".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "git_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "extra": {
                    "action": "status",
                    "branch": "main",
                    "changed_count": 0,
                    "clean": true,
                    "output": "exit=0\n## main...origin/main\n",
                    "worktree_state": "clean"
                },
                "text": "exit=0\n## main...origin/main\n"
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("git.branch=main git.worktree=clean git.changed.count=0".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GitRepositoryState;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(
        direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context)).is_none(),
        "free git repository-state delivery should be synthesized instead of exposing machine fields"
    );

    assert!(
        direct_non_builtin_skill_raw_answer(&state, &loop_state, Some(&agent_run_context))
            .is_none(),
        "free git repository-state delivery should not raw-passthrough git output"
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

#[test]
fn git_repository_state_strict_requested_machine_fields_drop_changed_list() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "git_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "extra": {
                    "action": "status",
                    "branch": "main",
                    "changed_count": 2,
                    "changed_files": ["Cargo.toml", "README.md"],
                    "output": "exit=0\n## main...origin/main\n M Cargo.toml\n?? README.md\n",
                    "worktree_state": "dirty"
                },
                "text": "exit=0\n## main...origin/main\n M Cargo.toml\n?? README.md\n"
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.delivery_messages.push(
        "git.branch=main\ngit.worktree=dirty\ngit.changed.count=2\ngit.changed[0]=M Cargo.toml"
            .to_string(),
    );

    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::GitRepositoryState;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.resolved_intent =
        "Return a machine summary with branch and worktree_state fields only.".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        original_user_request: Some(
            "Answer only the branch and worktree_state machine fields.".to_string(),
        ),
        user_request: Some("branch worktree_state".to_string()),
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(
        replace_git_repository_state_delivery_with_requested_machine_fields(
            &claimed_task("task-git-machine-fields"),
            &mut loop_state,
            Some(&agent_run_context),
            &mut finalizer_summary,
        )
    );
    assert_eq!(
        loop_state.delivery_messages,
        vec!["branch=main worktree_state=dirty".to_string()]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("branch=main worktree_state=dirty")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn git_status_contract_strict_requested_machine_fields() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "git_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "extra": {
                    "action": "status",
                    "branch": "main",
                    "changed_count": 2,
                    "output": "exit=0\n## main...origin/main\n M Cargo.toml\n?? README.md\n",
                    "worktree_state": "dirty"
                },
                "text": "exit=0\n## main...origin/main\n M Cargo.toml\n?? README.md\n"
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.delivery_messages.push(
        "git.branch=main\ngit.worktree=dirty\ngit.changed.count=2\ngit.changed[0]=M Cargo.toml"
            .to_string(),
    );

    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::GitRepositoryState;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.route_reason = "capability_ref=git.status".to_string();
    route.resolved_intent = "field=branch field=worktree_state".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        user_request: Some("branch worktree_state".to_string()),
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(
        replace_git_repository_state_delivery_with_requested_machine_fields(
            &claimed_task("task-git-machine-fields-capability-ref"),
            &mut loop_state,
            Some(&agent_run_context),
            &mut finalizer_summary,
        )
    );
    assert_eq!(
        loop_state.delivery_messages,
        vec!["branch=main worktree_state=dirty".to_string()]
    );
    assert!(finalizer_summary.is_some());
}
