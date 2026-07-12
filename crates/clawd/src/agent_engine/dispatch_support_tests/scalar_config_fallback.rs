use super::*;

#[test]
fn synthesize_direct_fallback_allows_wrapped_empty_config_scalar_for_path_contract() {
    let state = test_state_with_registry();
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "config_basic",
        r#"{"extra":{"action":"extract_field","exists":true,"field_path":"workspace.package.repository","format":"toml","path":"Cargo.toml","value":"","value_text":"","value_type":"string"},"text":"{\"action\":\"extract_field\",\"exists\":true,\"field_path\":\"workspace.package.repository\",\"value\":\"\",\"value_text\":\"\",\"value_type\":\"string\"}"}"#,
    ));
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "read workspace.package.repository from Cargo.toml".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "structured field scalar".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: "Cargo.toml".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let ctx = AgentRunContext {
        route_result: Some(route),
        ..AgentRunContext::default()
    };

    assert_eq!(
        synthesize_direct_observed_fallback_answer(&state, &loop_state, Some(&ctx)).as_deref(),
        Some("\"\"")
    );
}
