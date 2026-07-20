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
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Scalar,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::Path,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: "Cargo.toml".to_string(),
        selection: crate::OutputSelectionContract {
            structured_field_selector: Some("value".to_string()),
            ..Default::default()
        },
    };
    let ctx = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        synthesize_direct_observed_fallback_answer(&state, &loop_state, Some(&ctx)).as_deref(),
        Some("\"\"")
    );
}
