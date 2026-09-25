use claw_core::conversation_input::{
    ConversationInputContent, ConversationInputDeliveryMode, ConversationInputPreparationState,
    ConversationInputRecord, ConversationInputScopeRef, ConversationInputSource,
    ConversationInputSubmission, OwnedConversationInputScope, CONVERSATION_INPUT_SCHEMA_VERSION,
};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

use super::*;

fn pool() -> Pool<SqliteConnectionManager> {
    let pool = Pool::builder()
        .max_size(1)
        .build(SqliteConnectionManager::memory())
        .expect("build pool");
    ensure_conversation_input_schema(&pool.get().expect("connection")).expect("schema");
    pool
}

fn scope() -> OwnedConversationInputScope {
    OwnedConversationInputScope {
        owner_principal_id: "principal-1".to_string(),
        conversation: ConversationInputScopeRef {
            conversation_id: "conversation-1".to_string(),
            agent_id: "main".to_string(),
            channel: "telegram".to_string(),
            channel_account_id: "account-1".to_string(),
        },
    }
}

fn attachment(attachment_id: &str, path: &str) -> ConversationInputAttachmentBinding {
    ConversationInputAttachmentBinding {
        attachment_id: attachment_id.to_string(),
        kind: "file".to_string(),
        workspace_rel_path: path.to_string(),
        mime_type: Some("text/plain".to_string()),
        display_name: Some("fixture.txt".to_string()),
        size_bytes: 7,
        sha256: "a".repeat(64),
    }
}

fn accept_pending(
    pool: &DbPool,
    client_message_id: &str,
    attachment_id: &str,
) -> ConversationInputRecord {
    let owned_scope = scope();
    accept_conversation_input(
        pool,
        &AcceptConversationInput {
            owner_principal_id: owned_scope.owner_principal_id,
            submission: ConversationInputSubmission {
                schema_version: CONVERSATION_INPUT_SCHEMA_VERSION,
                client_message_id: client_message_id.to_string(),
                scope: owned_scope.conversation,
                content: vec![
                    ConversationInputContent::Text {
                        text: "Inspect this file.".to_string(),
                    },
                    ConversationInputContent::Attachment {
                        attachment_id: attachment_id.to_string(),
                        media_type: Some("text/plain".to_string()),
                        display_name: Some("fixture.txt".to_string()),
                    },
                ],
                delivery_mode: ConversationInputDeliveryMode::Auto,
                expected_task_id: None,
                expected_instruction_revision: None,
                source: ConversationInputSource::default(),
            },
            preparation_state: ConversationInputPreparationState::Pending,
        },
    )
    .expect("accept pending input")
    .record
}

#[test]
fn attachment_preparation_is_ready_idempotent_and_resolvable() {
    let pool = pool();
    let pending = accept_pending(&pool, "message-attachment", "channel_attachment:fixture");
    let binding = attachment("channel_attachment:fixture", "data/inbox/fixture.txt");

    let prepared = prepare_conversation_input_attachments(
        &pool,
        &scope(),
        pending.receipt.input_id,
        std::slice::from_ref(&binding),
    )
    .expect("prepare attachments");
    assert_eq!(
        prepared.receipt.preparation_state,
        ConversationInputPreparationState::Ready
    );
    let replay = prepare_conversation_input_attachments(
        &pool,
        &scope(),
        pending.receipt.input_id,
        std::slice::from_ref(&binding),
    )
    .expect("replay preparation");
    assert_eq!(
        replay.receipt.preparation_state,
        ConversationInputPreparationState::Ready
    );
    assert_eq!(
        conversation_input_attachments(&pool, pending.receipt.input_id)
            .expect("resolve attachments"),
        vec![binding]
    );
}

#[test]
fn attachment_preparation_rejects_changed_or_unsafe_mappings() {
    let pool = pool();
    let pending = accept_pending(&pool, "message-attachment", "channel_attachment:fixture");
    let binding = attachment("channel_attachment:fixture", "data/inbox/fixture.txt");
    prepare_conversation_input_attachments(
        &pool,
        &scope(),
        pending.receipt.input_id,
        std::slice::from_ref(&binding),
    )
    .expect("prepare attachments");

    let changed = attachment("channel_attachment:fixture", "data/inbox/changed.txt");
    assert!(matches!(
        prepare_conversation_input_attachments(
            &pool,
            &scope(),
            pending.receipt.input_id,
            &[changed]
        ),
        Err(ConversationInputStoreError::IdempotencyConflict)
    ));

    let next = accept_pending(&pool, "message-unsafe", "channel_attachment:unsafe");
    let unsafe_binding = attachment("channel_attachment:unsafe", "../outside.txt");
    assert!(matches!(
        prepare_conversation_input_attachments(
            &pool,
            &scope(),
            next.receipt.input_id,
            &[unsafe_binding]
        ),
        Err(ConversationInputStoreError::InvalidRequest)
    ));
}
