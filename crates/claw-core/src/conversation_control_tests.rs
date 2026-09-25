use super::*;

fn request() -> CancelCurrentConversationTaskRequest {
    CancelCurrentConversationTaskRequest {
        schema_version: CONVERSATION_CONTROL_SCHEMA_VERSION,
        client_request_id: "telegram:account:chat:message".to_string(),
        scope: ConversationInputScopeRef {
            conversation_id: "chat-1".to_string(),
            agent_id: "main".to_string(),
            channel: "telegram".to_string(),
            channel_account_id: "account-1".to_string(),
        },
        expected_task_id: None,
    }
}

#[test]
fn cancel_request_accepts_only_machine_scope_and_idempotency_fields() {
    assert!(request().validate());
    let mut invalid = request();
    invalid.client_request_id = "not valid whitespace".to_string();
    assert!(!invalid.validate());
    let mut invalid = request();
    invalid.scope.conversation_id.clear();
    assert!(!invalid.validate());
}

#[test]
fn cancel_receipt_is_a_machine_contract_without_user_visible_text() {
    let receipt = CancelCurrentConversationTaskReceipt {
        schema_version: CONVERSATION_CONTROL_SCHEMA_VERSION,
        status: CancelCurrentConversationTaskStatus::NoActiveTask,
        task_id: None,
        canceled: 0,
    };
    let value = serde_json::to_value(receipt).expect("serialize receipt");
    assert_eq!(value["status"], "no_active_task");
    assert!(value.get("text").is_none());
    assert!(value.get("message").is_none());
}

#[test]
fn cancel_tail_accepts_only_an_optional_uuid() {
    let task_id = Uuid::new_v4();
    assert_eq!(parse_cancel_expected_task_id(""), Some(None));
    assert_eq!(
        parse_cancel_expected_task_id(&task_id.to_string()),
        Some(Some(task_id))
    );
    assert_eq!(parse_cancel_expected_task_id("now"), None);
    assert_eq!(parse_cancel_expected_task_id("all"), None);
}
