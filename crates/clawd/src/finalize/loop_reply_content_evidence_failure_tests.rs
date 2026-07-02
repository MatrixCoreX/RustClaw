use super::*;

fn service_capability_route() -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_with_chat_finalizer(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "capability_ref=service_control.status".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
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
fn service_capability_failure_answer_returns_structured_envelope() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let route = service_capability_route();
    let context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let error = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "service_control",
            "error_kind": "not_found",
            "service_name": "telegramd",
            "manager_type": "systemd"
        })
    );

    let answer = service_status_failure_answer(&state, "", &error, Some(&context)).expect("answer");
    let envelope: serde_json::Value = serde_json::from_str(&answer).expect("json envelope");

    assert_eq!(
        envelope
            .get("message_key")
            .and_then(serde_json::Value::as_str),
        Some("service.status.failure")
    );
    assert_eq!(
        envelope
            .get("status_code")
            .and_then(serde_json::Value::as_str),
        Some("service_unit_not_found")
    );
    assert_eq!(
        envelope.get("target").and_then(serde_json::Value::as_str),
        Some("telegramd")
    );
    assert!(!answer.contains("__RC_SKILL_ERROR__"));
}
