use super::{
    build_incremental_plan_prompt, classify_planning_prompt_class,
    contract_scoped_lightweight_planner_skill_scope, contract_scoped_planner_skill_scope,
    PlanningPromptClass,
};
use crate::agent_engine::{attempt_ledger::build_attempt_ledger_compact, LoopState};
use crate::executor::{StepExecutionResult, StepExecutionStatus};
use serde_json::json;
use std::collections::BTreeSet;

#[test]
fn incremental_prompt_carries_structured_failed_attempt_for_planner_repair() {
    let mut loop_state = LoopState::new(3);
    let err = crate::skills::structured_skill_error_from_parts(
        "fs_basic",
        "missing_required_field",
        "missing_required_field",
        None,
        Some(json!({
            "error_code": "missing_required_field",
            "missing_evidence_fields": ["path"],
            "message_key": "clawd.skill.missing_required_field"
        })),
    );
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(err),
        started_at: 100,
        finished_at: 110,
    });

    let attempt_ledger = build_attempt_ledger_compact(&loop_state);
    let prompt = build_incremental_plan_prompt(
        "ledger=__ATTEMPT_LEDGER__\nlast=__LAST_ROUND_OUTPUT__\nround=__ROUND__",
        "read project file",
        "read project file",
        "turn_analysis",
        "tool_spec",
        "skill_playbooks",
        "",
        "auto",
        "zh-CN",
        "rustclaw",
        2,
        "history",
        &attempt_ledger,
        "last round failed",
        "linux",
        "bash",
        "/workspace",
    );

    assert!(prompt.contains("\"tool_or_skill\": \"fs_basic\""));
    assert!(prompt.contains("\"status\": \"error\""));
    assert!(prompt.contains("\"error_code\": \"missing_required_field\""));
    assert!(prompt.contains("\"missing_evidence\": ["));
    assert!(prompt.contains("\"path\""));
    assert!(prompt.contains("\"recovery_action\": \"collect_missing_evidence\""));
    assert!(prompt.contains("\"repair_class\": \"loop_bounded_recovery\""));
    assert!(prompt.contains("\"next_recovery_kind\": \"wait_background\""));
    assert!(prompt.contains("\"forbidden_repeat_signature\""));
    assert!(prompt.contains("round=2"));
}

fn base_route_result() -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    }
}

#[test]
fn lightweight_scope_uses_local_data_skills_for_bounded_multi_locator_boundary() {
    let mut route = base_route_result();
    route.route_reason = "current_workspace_generic_contract_deferred_to_agent_loop; auto_locator_suppressed_multiple_explicit_paths".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;

    assert_eq!(
        contract_scoped_lightweight_planner_skill_scope(Some(&route)),
        Some(BTreeSet::from([
            "archive_basic".to_string(),
            "config_basic".to_string(),
            "db_basic".to_string(),
            "fs_basic".to_string()
        ]))
    );
}

#[test]
fn executable_agent_loop_boundary_leaves_lightweight_skill_scope_open() {
    let mut route = base_route_result();
    route.route_reason = "executionless_finalize_trace_plain; executable_contract_preserved_for_agent_loop; auto_locator_suppressed_multiple_explicit_paths".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;

    assert_eq!(
        contract_scoped_lightweight_planner_skill_scope(Some(&route)),
        None,
        "executable agent-loop boundaries leave ordinary capability choice inside the planner loop"
    );
}

#[test]
fn executable_agent_loop_boundary_keeps_open_and_lightweight_planning_scope_open() {
    let mut route = base_route_result();
    route.route_reason = "current_workspace_generic_contract_deferred_to_agent_loop; executable_contract_preserved_for_agent_loop".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();

    assert_eq!(
        contract_scoped_planner_skill_scope(Some(&route)),
        None,
        "open planning remains unconstrained unless a machine capability ref exists"
    );
    assert_eq!(
        contract_scoped_lightweight_planner_skill_scope(Some(&route)),
        None
    );
}

#[test]
fn sqlite_locator_generic_scalar_scope_prefers_db_basic_only() {
    let mut route = base_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string();

    let scope = contract_scoped_planner_skill_scope(Some(&route)).expect("sqlite scope");

    assert_eq!(scope, BTreeSet::from(["db_basic".to_string()]));
    assert_eq!(
        contract_scoped_lightweight_planner_skill_scope(Some(&route)),
        Some(scope)
    );
}

#[test]
fn lightweight_scope_keeps_skill_choice_open_for_executionless_or_inline_payload_boundary() {
    for marker in [
        "executionless_finalize_trace_plain",
        "inline_structured_payload_context_execute",
    ] {
        let mut route = base_route_result();
        route.route_reason = marker.to_string();
        route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;

        assert_eq!(
            contract_scoped_lightweight_planner_skill_scope(Some(&route)),
            None,
            "{marker} should leave capability choice in the planner loop instead of suppressing skill playbooks"
        );
    }
}

#[test]
fn lightweight_scope_omits_skill_playbooks_for_structured_clarify_boundary() {
    for (needs_clarify, route_reason) in [
        (true, ""),
        (false, "standalone_freeform_clarify_loop_context"),
        (false, "alias_state_patch_ack"),
    ] {
        let mut route = base_route_result();
        route.needs_clarify = needs_clarify;
        route.route_reason = route_reason.to_string();
        route.clarify_question = "missing_slot=referent".to_string();

        assert_eq!(
            contract_scoped_lightweight_planner_skill_scope(Some(&route)),
            Some(BTreeSet::new()),
            "needs_clarify={needs_clarify} route_reason={route_reason}"
        );
    }
}

#[test]
fn lightweight_scope_uses_run_cmd_for_raw_command_boundary() {
    let mut route = base_route_result();
    route.route_reason = "explicit_command_requires_fresh_execution".to_string();
    route
        .output_contract
        .apply_output_contract_ref(crate::pipeline_types::OutputContractRef::new(
            crate::OutputSemanticKind::RawCommandOutput,
        ));

    assert_eq!(
        contract_scoped_lightweight_planner_skill_scope(Some(&route)),
        Some(BTreeSet::from(["run_cmd".to_string()]))
    );
}

#[test]
fn local_workspace_execution_uses_lightweight_prompt_without_semantic_skill_scope() {
    let mut route = base_route_result();
    route.route_reason = "executable_contract_preserved_for_agent_loop".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "run/nl_eval_tmp/codex_cli_continuous_20260711_new".to_string();
    route.wants_file_delivery = true;
    let loop_state = LoopState::new(2);

    assert_eq!(
        classify_planning_prompt_class(Some(&route), "create files and run tests", &loop_state),
        PlanningPromptClass::LightweightExecution
    );
    assert_eq!(
        contract_scoped_lightweight_planner_skill_scope(Some(&route)),
        None,
        "local execution stays planner-owned and keeps ordinary capability choice inside the planner loop"
    );
}

#[test]
fn local_workspace_execution_with_delivery_noise_uses_lightweight_prompt_without_skill_scope() {
    let mut route = base_route_result();
    route.risk_ceiling = crate::RiskCeiling::High;
    route.route_reason = "file_token_delivery_contract_repair; executable_contract_preserved_for_agent_loop; contract:generated_file_delivery; normalizer_semantic_contract_demoted_to_route_marker; generated_file_delivery_allows_runtime_target".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "run/nl_eval_tmp/codex_cli_continuous_20260711_new".to_string();
    route.wants_file_delivery = true;
    let loop_state = LoopState::new(2);

    assert_eq!(
        classify_planning_prompt_class(Some(&route), "workspace execution", &loop_state),
        PlanningPromptClass::LightweightExecution
    );
    assert_eq!(
        contract_scoped_lightweight_planner_skill_scope(Some(&route)),
        None,
        "delivery repair noise must not reintroduce pre-planner semantic skill whitelists"
    );
}
