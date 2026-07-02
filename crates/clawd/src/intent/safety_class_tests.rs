use super::classify_route_risk_ceiling;
use crate::{
    execution_recipe::{
        ExecutionRecipeKind, ExecutionRecipeProfile, ExecutionRecipeSpec,
        ExecutionRecipeTargetScope,
    },
    AskMode, IntentOutputContract, RiskCeiling, RouteResult, ScheduleKind, SelfExtensionContract,
    SelfExtensionMode, SelfExtensionTrigger,
};

fn base_route(ask_mode: AskMode) -> RouteResult {
    RouteResult {
        ask_mode,
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract::default(),
    }
}

#[test]
fn chat_route_is_low_risk() {
    let route = base_route(crate::AskMode::direct_answer());
    let out = classify_route_risk_ceiling(&route, None);
    assert_eq!(out.risk_ceiling, RiskCeiling::Low);
}

#[test]
fn current_repo_code_change_is_medium_risk() {
    let route = base_route(crate::AskMode::direct_answer());
    let recipe = ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        profile: ExecutionRecipeProfile::CodeChange,
        target_scope: ExecutionRecipeTargetScope::CurrentRepo,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
    };
    let out = classify_route_risk_ceiling(&route, Some(&recipe));
    assert_eq!(out.risk_ceiling, RiskCeiling::Medium);
}

#[test]
fn system_ops_is_high_risk() {
    let route = base_route(crate::AskMode::planner_execute_plain());
    let recipe = ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        profile: ExecutionRecipeProfile::OpsService,
        target_scope: ExecutionRecipeTargetScope::System,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
    };
    let out = classify_route_risk_ceiling(&route, Some(&recipe));
    assert_eq!(out.risk_ceiling, RiskCeiling::High);
}

#[test]
fn generated_file_delivery_route_is_high_risk() {
    let mut route = base_route(crate::AskMode::planner_execute_plain());
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFileDelivery;

    let out = classify_route_risk_ceiling(&route, None);

    assert_eq!(out.risk_ceiling, RiskCeiling::High);
    assert_eq!(out.reason, "generated_file_delivery_route");
}

#[test]
fn generated_file_path_report_route_is_high_risk() {
    let mut route = base_route(crate::AskMode::direct_answer());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.delivery_required = false;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "pwd_line_abs.txt".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFilePathReport;

    let out = classify_route_risk_ceiling(&route, None);

    assert_eq!(out.risk_ceiling, RiskCeiling::High);
    assert_eq!(out.reason, "generated_file_delivery_route");
}

#[test]
fn config_mutation_route_is_high_risk_even_with_locator_evidence() {
    let mut route = base_route(crate::AskMode::planner_execute_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ConfigMutation;

    let out = classify_route_risk_ceiling(&route, None);

    assert_eq!(out.risk_ceiling, RiskCeiling::High);
    assert_eq!(out.reason, "config_mutation_route");
}

#[test]
fn self_extension_is_high_risk() {
    let mut route = base_route(crate::AskMode::planner_execute_plain());
    route.output_contract.self_extension = SelfExtensionContract {
        mode: SelfExtensionMode::PermanentExtension,
        trigger: SelfExtensionTrigger::ExplicitUserRequest,
        execute_now: false,
        scalar_count_filter: Default::default(),
        list_selector: Default::default(),
        structured_field_selector: None,
    };
    let out = classify_route_risk_ceiling(&route, None);
    assert_eq!(out.risk_ceiling, RiskCeiling::High);
}

#[test]
fn read_only_locator_route_is_low_risk() {
    let mut route = base_route(crate::AskMode::planner_execute_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    let out = classify_route_risk_ceiling(&route, None);
    assert_eq!(out.risk_ceiling, RiskCeiling::Low);
}

#[test]
fn resume_execution_shortcut_is_still_execute_risk() {
    let mut route = base_route(crate::AskMode::direct_answer());
    route.ask_mode = AskMode::direct_answer().with_resume_overrides(false, true);
    let out = classify_route_risk_ceiling(&route, None);
    assert_eq!(out.risk_ceiling, RiskCeiling::Medium);
    assert_eq!(out.reason, "action_route_without_recipe");
}
