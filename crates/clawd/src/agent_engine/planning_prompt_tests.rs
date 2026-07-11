use super::{build_incremental_plan_prompt, contract_scoped_lightweight_planner_skill_scope};
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
fn lightweight_scope_uses_fs_basic_for_bounded_multi_locator_boundary() {
    let mut route = base_route_result();
    route.route_reason = "current_workspace_generic_contract_deferred_to_agent_loop; auto_locator_suppressed_multiple_explicit_paths".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;

    assert_eq!(
        contract_scoped_lightweight_planner_skill_scope(Some(&route)),
        Some(BTreeSet::from(["fs_basic".to_string()]))
    );
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
