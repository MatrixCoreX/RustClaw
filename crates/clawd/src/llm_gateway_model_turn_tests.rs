use super::*;

#[test]
fn model_delta_archive_projection_is_bounded_and_keeps_terminal_events() {
    let mut projection = ModelTurnEventArchiveProjection::default();
    assert!(projection.should_publish(&ModelTurnEvent::Started { attempt: 1 }));

    let mut published = 0;
    for index in 0..1_024 {
        let event = ModelTurnEvent::TextDelta {
            text: format!("fragment-{index}"),
        };
        published += usize::from(projection.should_publish(&event));
    }

    assert_eq!(published, 17);
    assert!(projection.should_publish(&ModelTurnEvent::Finished {
        reason: claw_core::model_turn::ModelFinishReason::Stop,
    }));
}

#[test]
fn model_delta_archive_projection_resets_for_provider_retry() {
    let mut projection = ModelTurnEventArchiveProjection::default();
    assert!(projection.should_publish(&ModelTurnEvent::Started { attempt: 1 }));
    assert!(projection.should_publish(&ModelTurnEvent::TextDelta {
        text: "first".to_string(),
    }));
    assert!(!projection.should_publish(&ModelTurnEvent::TextDelta {
        text: "second".to_string(),
    }));
    assert!(projection.should_publish(&ModelTurnEvent::Interrupted {
        code: "provider_retry".to_string(),
        retryable: true,
    }));
    assert!(projection.should_publish(&ModelTurnEvent::Started { attempt: 2 }));
    assert!(projection.should_publish(&ModelTurnEvent::ToolCallDelta {
        index: 0,
        id: None,
        name: Some("respond".to_string()),
        arguments_delta: "{}".to_string(),
    }));
}

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
