use super::*;

#[test]
fn direct_workspace_diff_resolves_and_remains_confirmation_exempt() {
    let state = registry_confirmation::workspace_registry_state();
    let task = test_task();
    let plan = crate::agent_engine::direct_capability_plan(
        &state,
        "workspace.diff",
        json!({"checkpoint_id": "checkpoint_1"}),
    );

    assert_eq!(plan.steps[0].action_type, "call_capability");
    assert_eq!(plan.steps[0].skill, "workspace.diff");
    assert_eq!(plan.steps[1].action_type, "synthesize_answer");

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result.needs_confirmation, "issues: {:?}", result.issues);
    assert_eq!(result.approved_steps[0].action_type, "call_tool");
    assert_eq!(result.approved_steps[0].skill, "fs_basic");
    assert_eq!(result.approved_steps[0].args["action"], "diff");
    assert_eq!(
        result.capability_resolutions[0]
            .record
            .canonical_capability_ref
            .as_deref(),
        Some("workspace.diff")
    );
}

#[test]
fn direct_workspace_rewind_resolves_but_requires_one_shot_confirmation() {
    let state = registry_confirmation::workspace_registry_state();
    let task = test_task();
    let plan = crate::agent_engine::direct_capability_plan(
        &state,
        "workspace.revert_checkpoint",
        json!({"checkpoint_id": "checkpoint_1"}),
    );

    assert_eq!(plan.steps[0].action_type, "call_capability");
    assert_eq!(plan.steps[0].skill, "workspace.revert_checkpoint");

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(result.needs_confirmation, "issues: {:?}", result.issues);
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
    assert_eq!(result.approved_steps[0].action_type, "call_tool");
    assert_eq!(result.approved_steps[0].skill, "fs_basic");
    assert_eq!(result.approved_steps[0].args["action"], "rewind");
    assert_eq!(
        result.capability_resolutions[0]
            .record
            .canonical_capability_ref
            .as_deref(),
        Some("workspace.revert_checkpoint")
    );
    assert_eq!(
        result.permission_decision["decision"],
        crate::policy_decision::PolicyDecision::RequireConfirmation.as_token()
    );
}

#[test]
fn direct_canonical_capability_verifies_registry_mapping() {
    let state = registry_confirmation::workspace_registry_state();
    let task = test_task();
    let plan = crate::agent_engine::direct_capability_plan(
        &state,
        "coding_workflow.preview_repair",
        json!({}),
    );

    assert_eq!(plan.steps[0].action_type, "call_capability");
    assert_eq!(plan.steps[0].skill, "coding_workflow.preview_repair");

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert_eq!(result.approved_steps[0].skill, "task_control");
    assert_eq!(
        result.approved_steps[0].args["action"],
        "preview_coding_repair"
    );
    let resolution = &result.capability_resolutions[0];
    assert_eq!(
        resolution.record.capability_ref,
        "coding_workflow.preview_repair"
    );
    assert_eq!(
        resolution.record.canonical_capability_ref.as_deref(),
        Some("coding_workflow.preview_repair")
    );
    assert_eq!(
        resolution.record.resolved_ref.as_deref(),
        Some("tool:task_control")
    );
}

#[test]
fn inline_subagent_capability_verifies_as_read_only_internal_tool() {
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = test_task();
    let plan = crate::agent_engine::direct_capability_plan(
        &state,
        "agent.subagent",
        json!({
            "role": "review",
            "objective": "inspect_runtime_boundary",
            "context_refs": ["AGENTS.md"],
            "allowed_capabilities": ["filesystem.read_text_range"]
        }),
    );

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result.needs_confirmation, "issues: {:?}", result.issues);
    assert_eq!(result.approved_steps[0].action_type, "call_tool");
    assert_eq!(result.approved_steps[0].skill, "subagent");
    assert_eq!(
        result.capability_resolutions[0]
            .record
            .canonical_capability_ref
            .as_deref(),
        Some("agent.subagent")
    );
}

#[test]
fn inline_subagent_capability_rejects_missing_context_evidence() {
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = test_task();
    let plan = crate::agent_engine::direct_capability_plan(
        &state,
        "agent.subagent",
        json!({
            "role": "review",
            "objective": "inspect_runtime_boundary",
            "context_refs": []
        }),
    );

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(!result.approved);
    assert!(result.issues.iter().any(|issue| {
        issue.kind == VerifyIssueKind::MissingRequiredArg
            && issue
                .missing_fields
                .iter()
                .any(|field| field == "context_refs")
    }));
}

#[test]
fn terminal_control_uses_capability_required_args_instead_of_run_cmd_fallback() {
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = test_task();
    let plan = plan_result(vec![PlanStep {
        step_id: "terminate".to_string(),
        action_type: "call_skill".to_string(),
        skill: "run_cmd".to_string(),
        args: json!({
            "action": "terminal_terminate",
            "session_id": "session-1"
        }),
        depends_on: Vec::new(),
        why: String::new(),
    }]);

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(!result.issues.iter().any(|issue| {
        issue.kind == VerifyIssueKind::MissingRequiredArg
            && issue.missing_fields.iter().any(|field| field == "command")
    }));
}

#[test]
fn terminal_control_reports_its_own_missing_required_arg() {
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = test_task();
    let plan = plan_result(vec![PlanStep {
        step_id: "terminate".to_string(),
        action_type: "call_skill".to_string(),
        skill: "run_cmd".to_string(),
        args: json!({"action": "terminal_terminate"}),
        depends_on: Vec::new(),
        why: String::new(),
    }]);

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.issues.iter().any(|issue| {
        issue.kind == VerifyIssueKind::MissingRequiredArg
            && issue
                .missing_fields
                .iter()
                .any(|field| field == "session_id")
    }));
    assert!(!result.issues.iter().any(|issue| {
        issue.kind == VerifyIssueKind::MissingRequiredArg
            && issue.missing_fields.iter().any(|field| field == "command")
    }));
}

#[test]
fn batch_subagent_capability_verifies_structured_children() {
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = test_task();
    let plan = crate::agent_engine::direct_capability_plan(
        &state,
        "agent.subagent_batch",
        json!({
            "children": [
                {
                    "role": "review",
                    "objective": "inspect_runtime_boundary",
                    "context_refs": ["AGENTS.md"],
                    "allowed_capabilities": ["filesystem.read_text_range"]
                },
                {
                    "role": "test",
                    "objective": "inspect_test_boundary",
                    "context_refs": ["crates/clawd/src/verifier.rs"],
                    "allowed_capabilities": ["filesystem.read_text_range"]
                }
            ]
        }),
    );

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result.needs_confirmation, "issues: {:?}", result.issues);
    assert_eq!(result.approved_steps[0].action_type, "call_tool");
    assert_eq!(result.approved_steps[0].skill, "subagent");
    assert_eq!(
        result.approved_steps[0].args["action"],
        "bounded_parallel_readonly"
    );
}

#[test]
fn batch_subagent_capability_rejects_child_without_objective() {
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = test_task();
    let plan = crate::agent_engine::direct_capability_plan(
        &state,
        "agent.subagent_batch",
        json!({
            "children": [{"role": "review", "task": "ambiguous_child_payload"}]
        }),
    );

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(!result.approved);
    assert!(result.issues.iter().any(|issue| {
        matches!(issue.kind, VerifyIssueKind::InvalidArgumentValue)
            || issue
                .missing_fields
                .iter()
                .any(|field| field == "objective")
    }));
}

#[test]
fn subagent_contract_rejects_parent_findings_and_runtime_policy() {
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = test_task();
    let plan = crate::agent_engine::direct_capability_plan(
        &state,
        "agent.subagent_batch",
        json!({
            "children": [{
                "role": "review",
                "objective": "inspect_runtime_boundary",
                "context_refs": ["AGENTS.md"],
                "allowed_capabilities": ["filesystem.read_text_range"],
                "findings": [{"code": "parent_injected"}],
                "permission_profile": "local_worktree"
            }]
        }),
    );

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(!result.approved);
    let detail = result
        .issues
        .iter()
        .map(|issue| issue.detail.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(detail.contains("children[0].findings"), "{detail}");
    assert!(
        detail.contains("children[0].permission_profile"),
        "{detail}"
    );
}

#[test]
fn persistent_subagent_contract_rejects_open_nested_shapes() {
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = test_task();
    let plan = crate::agent_engine::direct_capability_plan(
        &state,
        "agent.subagent_persistent",
        json!({
            "children": [{
                "node_id": "reviewer",
                "role": "review",
                "objective": "inspect_runtime_boundary",
                "context_refs": ["AGENTS.md"],
                "allowed_capabilities": ["filesystem.read_text_range"],
                "budget": {"max_rounds": 13, "unbounded": true},
                "result_contract": {
                    "output_format": "machine_json",
                    "free_form": true
                },
                "depends_on": [{"required": true}]
            }]
        }),
    );

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(!result.approved);
    let details = result
        .issues
        .iter()
        .map(|issue| issue.detail.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        details.contains("children[0].budget.max_rounds"),
        "{details}"
    );
    assert!(
        details.contains("children[0].budget.unbounded"),
        "{details}"
    );
    assert!(
        details.contains("children[0].result_contract.free_form"),
        "{details}"
    );
    assert!(result.issues.iter().any(|issue| {
        issue
            .missing_fields
            .iter()
            .any(|field| field == "children[0].depends_on[0].node_id")
    }));
}
