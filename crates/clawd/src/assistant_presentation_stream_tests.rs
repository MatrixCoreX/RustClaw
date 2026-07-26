use super::{final_respond_content, top_level_string_field, FieldScan, IncrementalRespondParser};
use claw_core::model_turn::{ModelToolCall, ModelTurnEvent};
use serde_json::json;

#[test]
fn parser_buffers_until_free_text_shape_and_content_are_complete() {
    let raw = r#"{"shape":"free_text","content":"line 1\n\u4f60\u597d","items":[]}"#;
    let mut parser = IncrementalRespondParser::default();
    for byte in raw.as_bytes() {
        parser.push(std::str::from_utf8(std::slice::from_ref(byte)).expect("ASCII fixture byte"));
        if parser.raw.ends_with(r#"\u597d""#) {
            assert_eq!(parser.terminal_answer().as_deref(), Some("line 1\n你好"));
        } else if !parser.raw.contains(r#"\u597d""#) {
            assert!(parser.terminal_answer().is_none());
        }
    }
    assert_eq!(parser.terminal_answer().as_deref(), Some("line 1\n你好"));
}

#[test]
fn parser_supports_content_before_shape_and_ignores_nested_field_names() {
    let raw = r#"{"metadata":{"shape":"free_text","content":"private"},"content":"public","shape":"free_text"}"#;
    assert_eq!(
        top_level_string_field(raw, "content"),
        FieldScan::Found("public".to_string())
    );
    assert_eq!(
        top_level_string_field(raw, "shape"),
        FieldScan::Found("free_text".to_string())
    );
    let mut parser = IncrementalRespondParser::default();
    parser.push(raw);
    assert_eq!(parser.terminal_answer().as_deref(), Some("public"));
}

#[test]
fn parser_never_treats_non_free_text_or_malformed_arguments_as_terminal_text() {
    for raw in [
        r#"{"shape":"list","content":"not public yet"}"#,
        r#"{"shape":"free_text","content":"unterminated"#,
        r#"{"shape":"free_text","content":42}"#,
        r#"{"content":"missing shape"}"#,
    ] {
        let mut parser = IncrementalRespondParser::default();
        parser.push(raw);
        assert!(parser.terminal_answer().is_none(), "{raw}");
    }
}

#[test]
fn final_tool_call_accepts_only_exact_structured_respond_free_text() {
    let respond = ModelToolCall {
        id: "call-1".to_string(),
        name: "respond".to_string(),
        arguments: json!({"shape":"free_text","content":"visible"}),
    };
    assert_eq!(final_respond_content(&respond), Some("visible"));

    for call in [
        ModelToolCall {
            name: "call_capability".to_string(),
            arguments: json!({"content":"secret tool argument"}),
            ..respond.clone()
        },
        ModelToolCall {
            arguments: json!({"shape":"list","content":"not terminal"}),
            ..respond.clone()
        },
    ] {
        assert!(final_respond_content(&call).is_none());
    }
}

#[test]
fn raw_unicode_is_tested_at_every_utf8_boundary() {
    let raw = r#"{"shape":"free_text","content":"A你好B"}"#;
    let mut parser = IncrementalRespondParser::default();
    let mut starts = raw
        .char_indices()
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    starts.push(raw.len());
    for pair in starts.windows(2) {
        parser.push(&raw[pair[0]..pair[1]]);
    }
    assert_eq!(parser.terminal_answer().as_deref(), Some("A你好B"));
}

#[test]
fn escaped_quotes_backslashes_and_surrogate_pairs_survive_every_input_boundary() {
    let raw = r#"{"shape":"free_text","content":"quote: \" path: C:\\tmp emoji: \ud83d\ude80"}"#;
    let mut parser = IncrementalRespondParser::default();
    for byte in raw.as_bytes() {
        parser.push(std::str::from_utf8(std::slice::from_ref(byte)).expect("ASCII fixture byte"));
    }
    assert_eq!(
        parser.terminal_answer().as_deref(),
        Some("quote: \" path: C:\\tmp emoji: 🚀")
    );
}

fn claimed_task(state: &crate::AppState, task_id: &str) -> crate::ClaimedTask {
    {
        let db = state.core.db.get().expect("task db");
        db.execute_batch(crate::INIT_SQL)
            .expect("initialize task schema");
        crate::db_init::ensure_task_lease_schema(&db).expect("initialize task lease schema");
        db.execute("ALTER TABLE tasks ADD COLUMN user_key TEXT", [])
            .expect("initialize task identity column");
    }
    state.seed_ask_task_row(
        task_id,
        7,
        11,
        r#"{"conversation_id":"conversation-security","turn_id":"turn-security"}"#,
    );
    crate::repo::claim_next_task(state)
        .expect("claim query")
        .expect("claimed task")
}

#[test]
fn observer_never_publishes_private_text_or_non_respond_tool_arguments() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = claimed_task(&state, "task-presentation-security");
    let observer = super::NativePresentationObserver::new(
        state.clone(),
        task.clone(),
        "fixture:model".to_string(),
        1,
    );
    observer.observe(&ModelTurnEvent::Started { attempt: 1 });
    observer.observe(&ModelTurnEvent::TextDelta {
        text: r#"{"hidden_plan":"do not publish"}"#.to_string(),
    });
    observer.observe(&ModelTurnEvent::ToolCallDelta {
        index: 0,
        id: Some("call-private".to_string()),
        name: Some("call_capability".to_string()),
        arguments_delta: r#"{"capability":"filesystem.read_text_range","args":{"path":"secret"}}"#
            .to_string(),
    });
    let before = crate::task_event_transport::replay_events_after(&state, &task.task_id, 0)
        .expect("replay private events");
    assert!(before.events.iter().all(|event| {
        !event
            .get("event_kind")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|kind| kind.starts_with("assistant_output_"))
    }));

    observer.observe(&ModelTurnEvent::ToolCallDelta {
        index: 1,
        id: Some("call-respond".to_string()),
        name: Some("respond".to_string()),
        arguments_delta: r#"{"shape":"free_text","content":"public ans"#.to_string(),
    });
    assert!(
        crate::task_event_transport::replay_events_after(&state, &task.task_id, 0)
            .expect("replay incomplete respond")
            .events
            .iter()
            .all(|event| event["event_kind"] != "assistant_output_delta")
    );
    observer.observe(&ModelTurnEvent::ToolCallDelta {
        index: 1,
        id: None,
        name: None,
        arguments_delta: r#"wer","items":[]}"#.to_string(),
    });

    let after = crate::task_event_transport::replay_events_after(&state, &task.task_id, 0)
        .expect("replay public events");
    let visible = after
        .events
        .iter()
        .filter(|event| event["event_kind"] == "assistant_output_delta")
        .map(|event| event["payload"]["content"].as_str().unwrap_or_default())
        .collect::<String>();
    assert_eq!(visible, "public answer");
    let serialized = serde_json::to_string(&after.events).expect("serialize event evidence");
    assert!(!serialized.contains("hidden_plan"));
    assert!(!serialized.contains("filesystem.read_text_range"));
    assert!(!serialized.contains(r#""path":"secret""#));
}
