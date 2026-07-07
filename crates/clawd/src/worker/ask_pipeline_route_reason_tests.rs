use super::{append_route_reason, route_reason_has_marker};

fn route_with_reason(route_reason: &str) -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        route_reason: route_reason.to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
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
fn append_route_reason_does_not_treat_substring_as_existing_marker() {
    let mut route = route_with_reason(
        "agent_loop_default_entry_extra; prefix:bare_topic_contextual_clarify_sanitized_extra",
    );

    append_route_reason(&mut route, "agent_loop_default_entry");
    append_route_reason(&mut route, "bare_topic_contextual_clarify_sanitized");

    assert!(route_reason_has_marker(&route, "agent_loop_default_entry"));
    assert!(route_reason_has_marker(
        &route,
        "bare_topic_contextual_clarify_sanitized"
    ));
    assert!(route
        .route_reason
        .contains("agent_loop_default_entry_extra"));
    assert!(route
        .route_reason
        .contains("bare_topic_contextual_clarify_sanitized_extra"));
}

#[test]
fn append_route_reason_deduplicates_existing_exact_marker() {
    let mut route = route_with_reason("agent_loop_default_entry");

    append_route_reason(&mut route, "agent_loop_default_entry");

    assert_eq!(route.route_reason, "agent_loop_default_entry");
}
