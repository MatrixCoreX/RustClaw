use super::*;

fn valid_submission() -> ConversationInputSubmission {
    ConversationInputSubmission {
        schema_version: CONVERSATION_INPUT_SCHEMA_VERSION,
        client_message_id: "client-message-1".to_string(),
        scope: ConversationInputScopeRef {
            conversation_id: "conversation-1".to_string(),
            agent_id: "main".to_string(),
            channel: "ui".to_string(),
            channel_account_id: "browser-session".to_string(),
        },
        content: vec![ConversationInputContent::Text {
            text: "Change the output format to JSON.".to_string(),
        }],
        delivery_mode: ConversationInputDeliveryMode::Auto,
        expected_task_id: None,
        expected_instruction_revision: Some(3),
        source: ConversationInputSource::default(),
    }
}

#[test]
fn valid_submission_round_trips() {
    let submission = valid_submission();
    submission.validate().expect("valid submission");
    let encoded = serde_json::to_string(&submission).expect("serialize submission");
    let decoded: ConversationInputSubmission =
        serde_json::from_str(&encoded).expect("deserialize submission");
    assert_eq!(decoded, submission);
}

#[test]
fn natural_language_is_opaque_but_machine_identifiers_are_strict() {
    let mut submission = valid_submission();
    submission.content = vec![ConversationInputContent::Text {
        text: "不要停止。続けてください。Continue, but keep the first result.".to_string(),
    }];
    submission.validate().expect("multilingual text");

    submission.client_message_id = "contains whitespace".to_string();
    assert_eq!(
        submission.validate(),
        Err(ConversationInputErrorCode::InvalidRequest)
    );
}

#[test]
fn empty_or_oversized_content_is_rejected() {
    let mut submission = valid_submission();
    submission.content.clear();
    assert!(submission.validate().is_err());

    submission.content = vec![ConversationInputContent::Text {
        text: "x".repeat(CONVERSATION_INPUT_MAX_CONTENT_BYTES + 1),
    }];
    assert!(submission.validate().is_err());
}

#[test]
fn attachment_contract_uses_ids_instead_of_paths() {
    let mut submission = valid_submission();
    submission.content = vec![ConversationInputContent::Attachment {
        attachment_id: "artifact:01HXYZ".to_string(),
        media_type: Some("application/pdf".to_string()),
        display_name: Some("report.pdf".to_string()),
    }];
    submission.validate().expect("valid attachment ref");

    submission.content = vec![ConversationInputContent::Attachment {
        attachment_id: "/tmp/private-file".to_string(),
        media_type: None,
        display_name: None,
    }];
    assert!(submission.validate().is_err());
}

#[test]
fn published_schema_is_valid_json_and_matches_contract_version() {
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/schemas/conversation-input-v1.schema.json"
    ))
    .expect("parse published schema");
    assert_eq!(schema["properties"]["schema_version"]["const"], 1);
    assert_eq!(schema["properties"]["content"]["minItems"], 1);
    assert!(schema["required"]
        .as_array()
        .expect("required array")
        .iter()
        .any(|field| field == "client_message_id"));
}

#[test]
fn channel_text_request_keeps_machine_scope_separate_from_opaque_text() {
    let task = crate::types::SubmitTaskRequest {
        user_id: Some(7),
        chat_id: Some(9),
        user_key: Some("rk-test".to_string()),
        channel: Some(ChannelKind::Wechat),
        external_user_id: Some("user-7".to_string()),
        external_chat_id: Some("chat-9".to_string()),
        ingress: None,
        idempotency_key: None,
        kind: crate::types::TaskKind::Ask,
        payload: serde_json::json!({ "text": "不要停止 quoted stop; continue." }),
    };
    let request = ConversationInputClientTaskRequest::channel_text(
        task,
        "wechat:account:message-1",
        "chat-9",
        "main",
        ChannelKind::Wechat,
        "account-1",
        "不要停止 quoted stop; continue.",
        ConversationInputSource::default(),
    );
    request.input.validate().expect("valid input contract");
    assert_eq!(request.input.scope.channel, "wechat");
    assert_eq!(request.input.scope.channel_account_id, "account-1");
    assert_eq!(
        request.input.content,
        vec![ConversationInputContent::Text {
            text: "不要停止 quoted stop; continue.".to_string()
        }]
    );
}
