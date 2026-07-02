use super::*;
use crate::{AskMode, IntentOutputContract};

fn route_with_capability_ref(capability_ref: &str) -> RouteResult {
    RouteResult {
        ask_mode: AskMode::planner_execute_plain(),
        resolved_intent: capability_ref.to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: capability_ref.to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::None,
            semantic_kind: OutputSemanticKind::None,
            ..IntentOutputContract::default()
        },
    }
}

#[test]
fn service_capability_refs_keep_service_target_without_semantic_kind() {
    for capability_ref in [
        "capability_ref=service.status",
        "capability_ref=service_control.status",
    ] {
        let route = route_with_capability_ref(capability_ref);

        let contract = EvidencePolicyContract::from_route_result(&route);

        assert_eq!(contract.target_object, TaskTargetObject::Service);
        assert_eq!(contract.operation, TaskOperation::Inspect);
        assert_eq!(contract.delivery_shape, TaskDeliveryShape::Raw);
        assert_eq!(contract.required_evidence_fields, vec!["field_value"]);
    }
}
