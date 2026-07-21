use super::*;

#[test]
fn text_delta_event_exposes_size_without_model_content() {
    let payload = model_turn_event_payload(
        "vendor-minimax:MiniMax-M3",
        4,
        &ModelTurnEvent::TextDelta {
            text: "private model content".to_string(),
        },
    );

    assert_eq!(payload["type"], "text_delta");
    assert_eq!(payload["text_delta_bytes"], 21);
    assert!(payload.get("text").is_none());
}

#[test]
fn tool_delta_event_exposes_shape_without_argument_fragment() {
    let payload = model_turn_event_payload(
        "vendor-minimax:MiniMax-M3",
        5,
        &ModelTurnEvent::ToolCallDelta {
            index: 0,
            id: Some("call-1".to_string()),
            name: Some("call_capability".to_string()),
            arguments_delta: "{\"credential\":\"secret\"}".to_string(),
        },
    );

    assert_eq!(payload["tool_name"], "call_capability");
    assert_eq!(payload["arguments_delta_bytes"], 23);
    assert!(payload.get("arguments_delta").is_none());
}

#[test]
fn teaching_log_keeps_native_text_and_tool_calls_together() {
    let turn = ModelTurnResponse {
        text: "I will inspect the workspace.".to_string(),
        tool_calls: vec![claw_core::model_turn::ModelToolCall {
            id: "call-1".to_string(),
            name: "call_capability".to_string(),
            arguments: json!({
                "capability": "filesystem.list_entries",
                "args": {"path": "."}
            }),
        }],
        usage: None,
        finish_reason: claw_core::model_turn::ModelFinishReason::ToolCalls,
        reasoning_metadata: Default::default(),
        events: Vec::new(),
    };

    let logged: serde_json::Value = serde_json::from_str(&model_turn_log_response(&turn)).unwrap();
    assert_eq!(logged["text"], "I will inspect the workspace.");
    assert_eq!(logged["tool_calls"][0]["name"], "call_capability");
    assert_eq!(
        logged["tool_calls"][0]["arguments"]["capability"],
        "filesystem.list_entries"
    );
    assert_eq!(logged["finish_reason"], "tool_calls");
}
