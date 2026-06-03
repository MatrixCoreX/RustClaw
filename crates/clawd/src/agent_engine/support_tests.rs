use super::{
    append_delivery_message, collect_execution_recipe_progress_hints,
    execution_recipe_phase_progress_key, AgentLoopGuardPolicy, LoopBudgetProfile,
    LoopRecipeOverrides,
};
use crate::agent_engine::LoopState;
use crate::execution_recipe::{
    ExecutionRecipeKind, ExecutionRecipePhase, ExecutionRecipeProfile, ExecutionRecipeRuntimeState,
    ExecutionRecipeSpec, ExecutionRecipeTargetScope,
};
use crate::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, ResumeBehavior, RiskCeiling, RouteResult, ScheduleKind,
};

fn base_policy() -> AgentLoopGuardPolicy {
    AgentLoopGuardPolicy {
        max_steps: 32,
        max_rounds: 2,
        max_tool_calls: 12,
        recoverable_failure_extra_rounds: 1,
        repeat_action_limit: 4,
        no_progress_limit: 1,
        multi_round_enabled: true,
        answer_verifier_retry_limit: 2,
        fast_read: LoopRecipeOverrides {
            max_steps: Some(16),
            max_rounds: Some(2),
            max_tool_calls: Some(6),
            repeat_action_limit: Some(3),
            no_progress_limit: Some(1),
            max_repairs: None,
            run_cmd_timeout_seconds: None,
            run_cmd_validation_timeout_seconds: None,
        },
        grounded_summary: LoopRecipeOverrides {
            max_steps: Some(40),
            max_rounds: Some(4),
            max_tool_calls: Some(16),
            repeat_action_limit: Some(5),
            no_progress_limit: Some(2),
            max_repairs: None,
            run_cmd_timeout_seconds: None,
            run_cmd_validation_timeout_seconds: None,
        },
        multi_step_workspace: LoopRecipeOverrides {
            max_steps: Some(56),
            max_rounds: Some(6),
            max_tool_calls: Some(24),
            repeat_action_limit: Some(6),
            no_progress_limit: Some(2),
            max_repairs: None,
            run_cmd_timeout_seconds: None,
            run_cmd_validation_timeout_seconds: None,
        },
        ops_closed_loop: LoopRecipeOverrides {
            max_steps: Some(48),
            max_rounds: Some(4),
            max_tool_calls: Some(24),
            repeat_action_limit: Some(6),
            no_progress_limit: Some(2),
            max_repairs: Some(3),
            run_cmd_timeout_seconds: Some(180),
            run_cmd_validation_timeout_seconds: Some(90),
        },
    }
}

fn route_with_contract(
    semantic_kind: OutputSemanticKind,
    locator_kind: OutputLocatorKind,
) -> RouteResult {
    RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "test".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Low,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    }
}

#[test]
fn ops_closed_loop_policy_uses_override_budget() {
    let policy = base_policy();
    let recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        ..Default::default()
    });
    let adjusted = policy.adjusted_for_context(recipe, None);
    assert_eq!(adjusted.max_steps, 48);
    assert_eq!(adjusted.max_rounds, 4);
    assert_eq!(adjusted.max_tool_calls, 24);
    assert_eq!(adjusted.repeat_action_limit, 6);
    assert_eq!(adjusted.no_progress_limit, 2);
    assert_eq!(
        adjusted.run_cmd_timeout_override(recipe, crate::execution_recipe::ActionEffect::mutate()),
        Some(180)
    );
    assert_eq!(
        adjusted
            .run_cmd_timeout_override(recipe, crate::execution_recipe::ActionEffect::validate()),
        Some(90)
    );
}

#[test]
fn route_contract_selects_grounded_summary_budget() {
    let policy = base_policy();
    let recipe = ExecutionRecipeRuntimeState::default();
    let route = route_with_contract(
        OutputSemanticKind::CommandOutputSummary,
        OutputLocatorKind::None,
    );

    assert_eq!(
        AgentLoopGuardPolicy::budget_profile_for_context(recipe, Some(&route)),
        LoopBudgetProfile::GroundedSummary
    );
    let adjusted = policy.adjusted_for_context(recipe, Some(&route));
    assert_eq!(adjusted.max_rounds, 4);
    assert_eq!(adjusted.max_tool_calls, 16);
    assert_eq!(adjusted.no_progress_limit, 2);
}

#[test]
fn workspace_delivery_contract_selects_multi_step_budget() {
    let policy = base_policy();
    let recipe = ExecutionRecipeRuntimeState::default();
    let mut route = route_with_contract(
        OutputSemanticKind::GeneratedFileDelivery,
        OutputLocatorKind::Filename,
    );
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.output_contract.response_shape = OutputResponseShape::FileToken;

    assert_eq!(
        AgentLoopGuardPolicy::budget_profile_for_context(recipe, Some(&route)),
        LoopBudgetProfile::MultiStepWorkspace
    );
    let adjusted = policy.adjusted_for_context(recipe, Some(&route));
    assert_eq!(adjusted.max_rounds, 6);
    assert_eq!(adjusted.max_steps, 56);
    assert_eq!(adjusted.max_tool_calls, 24);
}

#[test]
fn ops_closed_loop_runtime_applies_repair_override() {
    let policy = base_policy();
    let mut recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        ..Default::default()
    });
    policy.apply_recipe_runtime_overrides(&mut recipe);
    assert_eq!(recipe.max_repairs, 3);
}

#[test]
fn append_delivery_message_sanitizes_structured_skill_errors() {
    let mut messages = Vec::new();
    append_delivery_message(
        "task-support-test",
        &mut messages,
        r#"执行失败：__RC_SKILL_ERROR__:{"skill":"archive_basic","error_kind":"unknown","error_text":"archive is required","text":null}。"#
            .to_string(),
    );

    assert_eq!(messages, vec!["执行失败：archive is required。"]);
}

#[test]
fn external_workspace_progress_hints_include_mode_and_ready_once() {
    let mut loop_state = LoopState::new(4);
    loop_state.execution_recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        target_scope: ExecutionRecipeTargetScope::ExternalWorkspace,
        inspect_first: true,
        validation_required: true,
        ..Default::default()
    });

    let first = collect_execution_recipe_progress_hints(&mut loop_state);
    assert_eq!(first.len(), 2);
    assert!(first[0].contains("telegram.progress.ops_recipe_scope_external_mode"));
    assert!(first[1].contains("telegram.progress.ops_recipe_inspect"));

    loop_state.execution_recipe.saw_external_target = true;
    let second = collect_execution_recipe_progress_hints(&mut loop_state);
    assert_eq!(second.len(), 1);
    assert!(second[0].contains("telegram.progress.ops_recipe_scope_external_ready"));

    let third = collect_execution_recipe_progress_hints(&mut loop_state);
    assert!(third.is_empty());
}

#[test]
fn greenfield_progress_hints_include_mode_and_creation_ready_once() {
    let mut loop_state = LoopState::new(4);
    loop_state.execution_recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        target_scope: ExecutionRecipeTargetScope::Greenfield,
        inspect_first: true,
        validation_required: true,
        ..Default::default()
    });

    let first = collect_execution_recipe_progress_hints(&mut loop_state);
    assert_eq!(first.len(), 2);
    assert!(first[0].contains("telegram.progress.ops_recipe_scope_greenfield_mode"));
    assert!(first[1].contains("telegram.progress.ops_recipe_inspect"));

    loop_state.execution_recipe.saw_greenfield_creation = true;
    let second = collect_execution_recipe_progress_hints(&mut loop_state);
    assert_eq!(second.len(), 1);
    assert!(second[0].contains("telegram.progress.ops_recipe_scope_greenfield_ready"));

    let third = collect_execution_recipe_progress_hints(&mut loop_state);
    assert!(third.is_empty());
}

#[test]
fn code_change_phase_progress_uses_profile_specific_keys() {
    assert_eq!(
        execution_recipe_phase_progress_key(
            ExecutionRecipeProfile::CodeChange,
            ExecutionRecipePhase::Inspect
        ),
        "telegram.progress.code_change_inspect"
    );
    assert_eq!(
        execution_recipe_phase_progress_key(
            ExecutionRecipeProfile::CodeChange,
            ExecutionRecipePhase::Apply
        ),
        "telegram.progress.code_change_apply"
    );
    assert_eq!(
        execution_recipe_phase_progress_key(
            ExecutionRecipeProfile::CodeChange,
            ExecutionRecipePhase::Validate
        ),
        "telegram.progress.code_change_validate"
    );
}

#[test]
fn skill_authoring_validate_progress_uses_profile_specific_key() {
    assert_eq!(
        execution_recipe_phase_progress_key(
            ExecutionRecipeProfile::SkillAuthoring,
            ExecutionRecipePhase::Validate
        ),
        "telegram.progress.skill_authoring_validate"
    );
}
