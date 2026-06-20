use super::resume_discussion_uses_direct_chat_renderer;

fn resume_discussion_route() -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::ClarifyOrChat {
            entry: crate::ChatEntryStrategy::ResumeFollowupDiscussion,
        },
        resolved_intent: "resume discussion".to_string(),
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
        output_contract: crate::IntentOutputContract::default(),
    }
}

#[test]
fn resume_discussion_direct_renderer_allows_only_pure_chat_contract() {
    let route = resume_discussion_route();
    assert!(resume_discussion_uses_direct_chat_renderer(&route));
}

#[test]
fn resume_discussion_direct_renderer_rejects_evidence_contract() {
    let mut route = resume_discussion_route();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "README.md".to_string();
    assert!(!resume_discussion_uses_direct_chat_renderer(&route));
}
