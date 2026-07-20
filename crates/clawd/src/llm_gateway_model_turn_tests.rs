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
