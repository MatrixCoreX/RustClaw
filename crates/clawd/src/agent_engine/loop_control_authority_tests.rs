use super::{boundary_context_snapshot_json, AgentLoopGuardPolicy};
use crate::agent_engine::support::{
    AnswerVerifierRequiredEvidenceScope, LoopRecipeOverrides, RegistryIdempotencyGuardScope,
    SemanticRouteAuthority,
};
use crate::agent_engine::AgentRunContext;
use crate::{
    ClaimedTask, IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind,
    OutputResponseShape, OutputSemanticKind, ResumeBehavior, RiskCeiling, RouteResult,
    ScheduleKind,
};

fn test_policy() -> AgentLoopGuardPolicy {
    AgentLoopGuardPolicy {
        max_steps: 32,
        max_rounds: 4,
        max_tool_calls: 12,
        recoverable_failure_extra_rounds: 1,
        repeat_action_limit: 4,
        no_progress_limit: 1,
        multi_round_enabled: true,
        answer_verifier_retry_limit: 2,
        answer_verifier_enforce_required_scope: AnswerVerifierRequiredEvidenceScope::Off,
        semantic_route_authority: SemanticRouteAuthority::Legacy,
        agent_loop_canary_bucket: "none".to_string(),
        registry_idempotency_guard_scope: RegistryIdempotencyGuardScope::Off,
        structured_evidence_required_for_selected_contracts: false,
        fast_read: LoopRecipeOverrides::default(),
        grounded_summary: LoopRecipeOverrides::default(),
        multi_step_workspace: LoopRecipeOverrides::default(),
        ops_closed_loop: LoopRecipeOverrides::default(),
    }
}

fn test_task() -> ClaimedTask {
    ClaimedTask {
        task_id: "task-loop-authority".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "telegram".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

fn structured_field_route() -> RouteResult {
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
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ScalarPathOnly,
            locator_hint: "README.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    }
}

#[test]
fn agent_loop_default_selects_any_eligible_low_risk_class_without_canary_token() {
    let mut policy = test_policy();
    policy.semantic_route_authority = SemanticRouteAuthority::AgentLoopDefault;
    policy.agent_loop_canary_bucket = "none".to_string();
    let route = structured_field_route();

    assert_eq!(
        crate::agent_engine::agent_loop_authority_selected_migration_class_for_policy(
            &policy, &route
        ),
        Some("structured_field_read")
    );
    let boundary = boundary_context_snapshot_json(
        &test_task(),
        &policy,
        Some(&AgentRunContext::default()),
        Some(&route),
        super::LoopBudgetProfile::FastRead,
    );
    assert_eq!(
        boundary
            .pointer("/semantic_routing/chosen_authority")
            .and_then(serde_json::Value::as_str),
        Some("agent_loop_default")
    );
    assert_eq!(
        boundary
            .pointer("/budget/selected_migration_class")
            .and_then(serde_json::Value::as_str),
        Some("structured_field_read")
    );
    assert_eq!(
        boundary
            .pointer("/budget/agent_loop_eligibility_bucket")
            .and_then(serde_json::Value::as_str),
        Some("low_risk_structured_read")
    );
    assert_eq!(
        boundary
            .pointer("/budget/agent_loop_eligibility_blocked_reason")
            .and_then(serde_json::Value::as_str),
        Some("none")
    );
}

#[test]
fn agent_loop_default_selects_generic_low_risk_bucket_without_canary_token() {
    let mut policy = test_policy();
    policy.semantic_route_authority = SemanticRouteAuthority::AgentLoopDefault;
    policy.agent_loop_canary_bucket = "none".to_string();
    let mut route = structured_field_route();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;

    assert_eq!(
        crate::agent_engine::agent_loop_authority_selected_migration_class_for_policy(
            &policy, &route
        ),
        Some("low_risk_status_observation")
    );
    let boundary = boundary_context_snapshot_json(
        &test_task(),
        &policy,
        Some(&AgentRunContext::default()),
        Some(&route),
        super::LoopBudgetProfile::FastRead,
    );
    assert_eq!(
        boundary
            .pointer("/budget/selected_migration_class")
            .and_then(serde_json::Value::as_str),
        Some("low_risk_status_observation")
    );
    assert_eq!(
        boundary
            .pointer("/budget/agent_loop_eligibility_bucket")
            .and_then(serde_json::Value::as_str),
        Some("low_risk_status_observation")
    );

    let mut delivery = structured_field_route();
    delivery.wants_file_delivery = true;
    delivery.output_contract.response_shape = OutputResponseShape::FileToken;
    delivery.output_contract.delivery_required = true;
    delivery.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    delivery.output_contract.semantic_kind = OutputSemanticKind::None;
    delivery.output_contract.locator_kind = OutputLocatorKind::Path;
    delivery.output_contract.locator_hint = "README.md".to_string();

    assert_eq!(
        crate::agent_engine::agent_loop_authority_selected_migration_class_for_policy(
            &policy, &delivery
        ),
        Some("low_risk_single_file_delivery")
    );
    let boundary = boundary_context_snapshot_json(
        &test_task(),
        &policy,
        Some(&AgentRunContext::default()),
        Some(&delivery),
        super::LoopBudgetProfile::FastRead,
    );
    assert_eq!(
        boundary
            .pointer("/budget/agent_loop_eligibility_bucket")
            .and_then(serde_json::Value::as_str),
        Some("low_risk_single_file_delivery")
    );
    assert_eq!(
        boundary
            .pointer("/semantic_routing/chosen_authority")
            .and_then(serde_json::Value::as_str),
        Some("agent_loop_default")
    );
}

#[test]
fn agent_loop_canary_still_requires_selected_migration_class() {
    let mut policy = test_policy();
    policy.semantic_route_authority = SemanticRouteAuthority::AgentLoopCanary;
    policy.agent_loop_canary_bucket = "exact_path_list".to_string();
    let route = structured_field_route();

    assert_eq!(
        crate::agent_engine::agent_loop_authority_selected_migration_class_for_policy(
            &policy, &route
        ),
        None
    );
}
