use super::*;

#[test]
fn text_request_has_no_native_tool_requirement() {
    let request = ModelTurnRequest::text("inspect the workspace");

    assert!(!request.requires_native_tools());
    assert_eq!(request.messages.len(), 1);
    assert_eq!(request.messages[0].role, ModelRole::User);
}

#[test]
fn model_turn_protocol_round_trips_tool_calls_and_events() {
    let call = ModelToolCall {
        id: "call-1".to_string(),
        name: "call_capability".to_string(),
        arguments: serde_json::json!({
            "capability": "fs.read",
            "args": {"path": "README.md"}
        }),
    };
    let response = ModelTurnResponse {
        text: String::new(),
        tool_calls: vec![call.clone()],
        usage: Some(ModelTurnUsage {
            total_tokens: Some(42),
            ..ModelTurnUsage::default()
        }),
        finish_reason: ModelFinishReason::ToolCalls,
        reasoning_metadata: BTreeMap::new(),
        events: vec![
            ModelTurnEvent::ToolCall { call },
            ModelTurnEvent::Finished {
                reason: ModelFinishReason::ToolCalls,
            },
        ],
    };

    let encoded = serde_json::to_string(&response).expect("serialize model turn");
    let decoded: ModelTurnResponse =
        serde_json::from_str(&encoded).expect("deserialize model turn");

    assert_eq!(decoded, response);
}

#[test]
fn provider_capabilities_are_conservative_by_default() {
    assert_eq!(
        ProviderModelCapabilities::default(),
        ProviderModelCapabilities {
            native_tools: false,
            parallel_tools: false,
            structured_output: false,
            streaming: false,
            reasoning: false,
            vision: false,
            prompt_cache: false,
        }
    );
}

#[test]
fn provider_descriptor_round_trips_budgeting_capabilities() {
    let descriptor = ProviderModelDescriptor {
        capabilities: ProviderModelCapabilities::default(),
        context_window_tokens: Some(128_000),
        output_reserve_tokens: 8_192,
        request_timeout_seconds: 180,
        estimator_confidence: TokenEstimatorConfidence::Conservative,
    };
    let encoded = serde_json::to_value(&descriptor).unwrap();
    assert_eq!(encoded["context_window_tokens"], 128_000);
    assert_eq!(encoded["output_reserve_tokens"], 8_192);
    assert_eq!(encoded["request_timeout_seconds"], 180);
    assert_eq!(encoded["estimator_confidence"], "conservative");
    assert_eq!(
        serde_json::from_value::<ProviderModelDescriptor>(encoded).unwrap(),
        descriptor
    );
}
