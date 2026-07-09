use super::{
    defer_locator_binding_to_agent_loop,
    defer_locator_binding_to_agent_loop_preserving_content_evidence,
};

fn executable_route_with_semantic(kind: crate::OutputSemanticKind) -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "planner boundary test".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            locator_kind: crate::OutputLocatorKind::Path,
            locator_hint: "/tmp/background-only/docs".to_string(),
            requires_content_evidence: true,
            response_shape: crate::OutputResponseShape::Strict,
            semantic_kind: kind,
            ..Default::default()
        },
    }
}

#[test]
fn defer_locator_binding_clears_locator_scoped_directory_contract() {
    let mut route = executable_route_with_semantic(crate::OutputSemanticKind::DirectoryEntryGroups);

    defer_locator_binding_to_agent_loop(&mut route);

    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(!route.output_contract.requires_content_evidence);
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Strict
    );
}

#[test]
fn defer_locator_binding_clears_locatorless_runtime_contract_marker() {
    let mut route = executable_route_with_semantic(crate::OutputSemanticKind::ServiceStatus);

    defer_locator_binding_to_agent_loop(&mut route);

    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(!route.output_contract.requires_content_evidence);
}

#[test]
fn defer_locator_binding_can_preserve_observation_requirement_for_agent_loop() {
    let mut route =
        executable_route_with_semantic(crate::OutputSemanticKind::ContentExcerptSummary);

    defer_locator_binding_to_agent_loop_preserving_content_evidence(&mut route);

    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(route.output_contract.requires_content_evidence);
}
