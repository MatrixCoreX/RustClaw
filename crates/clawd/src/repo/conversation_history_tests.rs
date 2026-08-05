use claw_core::types::AuthIdentity;
use serde_json::json;
use uuid::Uuid;

use super::*;

fn state_with_tasks() -> AppState {
    let state = AppState::test_default_with_fixture_provider();
    let db = state.core.db.get().unwrap();
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            user_key TEXT,
            principal_id TEXT,
            channel TEXT NOT NULL,
            external_chat_id TEXT,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            status TEXT NOT NULL,
            result_json TEXT,
            error_text TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE conversation_metadata (
            owner_user_key TEXT NOT NULL,
            owner_user_id INTEGER NOT NULL,
            owner_principal_id TEXT,
            conversation_id TEXT NOT NULL,
            title TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY(owner_user_key, conversation_id)
        );
        CREATE UNIQUE INDEX idx_test_conversation_metadata_principal
        ON conversation_metadata(owner_principal_id, conversation_id)
        WHERE owner_principal_id IS NOT NULL;
        CREATE TABLE conversation_archives (
            owner_user_key TEXT NOT NULL,
            owner_user_id INTEGER NOT NULL,
            owner_principal_id TEXT,
            conversation_id TEXT NOT NULL,
            archived_at TEXT NOT NULL,
            PRIMARY KEY(owner_user_key, conversation_id)
        );
        CREATE UNIQUE INDEX idx_test_conversation_archives_principal
        ON conversation_archives(owner_principal_id, conversation_id)
        WHERE owner_principal_id IS NOT NULL;",
    )
    .unwrap();
    drop(db);
    state
}

#[test]
fn archived_conversations_are_hidden_for_the_archiving_identity() {
    let state = state_with_tasks();
    insert_turn(
        &state,
        TurnFixture {
            task_id: Uuid::new_v4(),
            user_id: 42,
            user_key: "owner-key",
            conversation_id: "chat-thread-archived",
            text: "Inspect the workspace",
            answer: "Inspection complete",
            updated_at: 501,
        },
    );

    let archived = archive_conversation(&state, &identity("user"), "chat-thread-archived")
        .expect("archive conversation");
    assert_eq!(archived.status, "ok");
    assert_eq!(archived.conversation_id, "chat-thread-archived");
    assert!(
        list_conversation_history(&state, &identity("user"), None, None)
            .unwrap()
            .turns
            .is_empty()
    );

    let repeated = archive_conversation(&state, &identity("user"), "chat-thread-archived")
        .expect("repeat archive");
    assert_eq!(repeated.conversation_id, "chat-thread-archived");
}

#[test]
fn archive_rejects_unknown_or_inaccessible_conversations() {
    let state = state_with_tasks();
    insert_turn(
        &state,
        TurnFixture {
            task_id: Uuid::new_v4(),
            user_id: 99,
            user_key: "other-key",
            conversation_id: "chat-thread-private",
            text: "private task",
            answer: "private answer",
            updated_at: 601,
        },
    );

    assert_eq!(
        archive_conversation(&state, &identity("user"), "chat-thread-private")
            .unwrap_err()
            .to_string(),
        "conversation_not_found"
    );
    assert_eq!(
        archive_conversation(&state, &identity("user"), "../bad")
            .unwrap_err()
            .to_string(),
        "conversation_id_invalid"
    );
}

#[test]
fn custom_title_is_persisted_and_projected_for_its_owner() {
    let state = state_with_tasks();
    insert_turn(
        &state,
        TurnFixture {
            task_id: Uuid::new_v4(),
            user_id: 42,
            user_key: "owner-key",
            conversation_id: "chat-thread-renamed",
            text: "Inspect the workspace",
            answer: "Inspection complete",
            updated_at: 301,
        },
    );

    let updated = update_conversation_title(
        &state,
        &identity("user"),
        "chat-thread-renamed",
        "  Release readiness  ",
    )
    .unwrap();
    assert_eq!(updated.status, "ok");
    assert_eq!(updated.title, "Release readiness");

    let history = list_conversation_history(&state, &identity("user"), None, None).unwrap();
    assert_eq!(
        history.turns[0].conversation_title.as_deref(),
        Some("Release readiness")
    );
}

#[test]
fn conversation_history_projects_downloadable_task_artifacts() {
    let state = state_with_tasks();
    let task_id = Uuid::new_v4();
    insert_turn(
        &state,
        TurnFixture {
            task_id,
            user_id: 42,
            user_key: "owner-key",
            conversation_id: "chat-thread-artifact",
            text: "Create a report",
            answer: "Report ready",
            updated_at: 351,
        },
    );
    let artifact = json!({
        "schema_version": 1,
        "id": "artifact-1",
        "filename": "report.pdf",
        "kind": "pdf",
        "mime_type": "application/pdf",
        "size_bytes": 42,
        "sha256": "a".repeat(64),
        "download_url": format!("/v1/tasks/{task_id}/artifacts/artifact-1/content"),
        "preview_url": format!("/v1/tasks/{task_id}/artifacts/artifact-1/content?disposition=inline")
    });
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE tasks SET result_json = ?2 WHERE task_id = ?1",
            rusqlite::params![
                task_id.to_string(),
                json!({"text": "Report ready", "artifacts": [artifact]}).to_string()
            ],
        )
        .unwrap();

    let history = list_conversation_history(&state, &identity("user"), None, None).unwrap();

    assert_eq!(history.turns[0].artifacts.len(), 1);
    assert_eq!(history.turns[0].artifacts[0].filename, "report.pdf");
}

#[test]
fn conversation_titles_reject_inaccessible_conversations() {
    let state = state_with_tasks();
    insert_turn(
        &state,
        TurnFixture {
            task_id: Uuid::new_v4(),
            user_id: 99,
            user_key: "other-key",
            conversation_id: "shared-thread-id",
            text: "private task",
            answer: "private answer",
            updated_at: 401,
        },
    );
    assert_eq!(
        update_conversation_title(
            &state,
            &identity("user"),
            "shared-thread-id",
            "Owner-only title",
        )
        .unwrap_err()
        .to_string(),
        "conversation_not_found"
    );
}

#[test]
fn conversation_title_rejects_invalid_machine_refs_and_bounds() {
    let state = state_with_tasks();
    let owner = identity("user");
    assert_eq!(
        update_conversation_title(&state, &owner, "../escape", "title")
            .unwrap_err()
            .to_string(),
        "conversation_id_invalid"
    );
    assert_eq!(
        update_conversation_title(&state, &owner, "thread-ok", "   ")
            .unwrap_err()
            .to_string(),
        "conversation_title_invalid"
    );
    assert_eq!(
        update_conversation_title(&state, &owner, "thread-ok", &"a".repeat(121))
            .unwrap_err()
            .to_string(),
        "conversation_title_invalid"
    );
}

fn identity(role: &str) -> AuthIdentity {
    AuthIdentity {
        user_key: "owner-key".to_string(),
        principal_id: "principal-test-owner".to_string(),
        role: role.to_string(),
        user_id: 42,
        chat_id: 7,
    }
}

struct TurnFixture<'a> {
    task_id: Uuid,
    user_id: i64,
    user_key: &'a str,
    conversation_id: &'a str,
    text: &'a str,
    answer: &'a str,
    updated_at: i64,
}

fn insert_turn(state: &AppState, fixture: TurnFixture<'_>) {
    let db = state.core.db.get().unwrap();
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, kind, payload_json,
            status, result_json, error_text, created_at, updated_at
         ) VALUES (?1, ?2, 7, ?3, 'ui', 'ask', ?4, 'succeeded', ?5, NULL, ?6, ?6)",
        rusqlite::params![
            fixture.task_id.to_string(),
            fixture.user_id,
            fixture.user_key,
            json!({
                "conversation_id": fixture.conversation_id,
                "agent_id": "main",
                "text": fixture.text,
                "attachments": [
                    {"kind": "image", "path": "data/task-inputs/image.png"},
                    {"kind": "file", "path": "data/task-inputs/spec.md"}
                ]
            })
            .to_string(),
            json!({"text": fixture.answer}).to_string(),
            fixture.updated_at.to_string(),
        ],
    )
    .unwrap();
}

#[test]
fn owner_history_is_bounded_structured_and_cursor_paged() {
    let state = state_with_tasks();
    let first_id = Uuid::new_v4();
    let second_id = Uuid::new_v4();
    insert_turn(
        &state,
        TurnFixture {
            task_id: first_id,
            user_id: 42,
            user_key: "owner-key",
            conversation_id: "chat-thread-one",
            text: "Inspect the workspace",
            answer: "Inspection complete",
            updated_at: 101,
        },
    );
    insert_turn(
        &state,
        TurnFixture {
            task_id: second_id,
            user_id: 42,
            user_key: "owner-key",
            conversation_id: "chat-thread-one",
            text: "Run the focused test",
            answer: "The focused test passed",
            updated_at: 102,
        },
    );

    let first = list_conversation_history(&state, &identity("user"), Some(1), None).unwrap();
    assert_eq!(first.schema_version, 1);
    assert_eq!(first.status, "ok");
    assert!(first.truncated);
    assert_eq!(first.turns.len(), 1);
    assert_eq!(first.turns[0].task_id, second_id.to_string());
    assert_eq!(first.turns[0].agent_id.as_deref(), Some("main"));
    assert_eq!(first.turns[0].attachment_count, 2);
    assert_eq!(first.turns[0].attachment_kinds, ["file", "image"]);
    assert_eq!(first.content_sha256.len(), 64);

    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE tasks SET updated_at = '999' WHERE task_id = ?1",
            rusqlite::params![first_id.to_string()],
        )
        .unwrap();

    let second = list_conversation_history(
        &state,
        &identity("user"),
        Some(1),
        first.next_cursor.as_deref(),
    )
    .unwrap();
    assert!(!second.truncated);
    assert_eq!(second.turns.len(), 1);
    assert_eq!(second.turns[0].task_id, first_id.to_string());
}

#[test]
fn conversation_history_is_owner_scoped_even_for_admin_keys() {
    let state = state_with_tasks();
    insert_turn(
        &state,
        TurnFixture {
            task_id: Uuid::new_v4(),
            user_id: 99,
            user_key: "other-key",
            conversation_id: "chat-thread-other",
            text: "private task",
            answer: "private answer",
            updated_at: 200,
        },
    );

    let owner = list_conversation_history(&state, &identity("user"), None, None).unwrap();
    assert!(owner.turns.is_empty());
    let admin = list_conversation_history(&state, &identity("admin"), None, None).unwrap();
    assert!(admin.turns.is_empty());
}

#[test]
fn admin_keys_can_read_their_own_conversation_history() {
    let state = state_with_tasks();
    insert_turn(
        &state,
        TurnFixture {
            task_id: Uuid::new_v4(),
            user_id: 42,
            user_key: "owner-key",
            conversation_id: "chat-thread-admin-owned",
            text: "inspect own task",
            answer: "inspection complete",
            updated_at: 250,
        },
    );

    let admin = list_conversation_history(&state, &identity("admin"), None, None).unwrap();
    assert_eq!(admin.turns.len(), 1);
    assert_eq!(admin.turns[0].conversation_id, "chat-thread-admin-owned");
}

#[test]
fn malformed_conversation_ids_and_cursors_are_rejected_without_prose_matching() {
    let state = state_with_tasks();
    insert_turn(
        &state,
        TurnFixture {
            task_id: Uuid::new_v4(),
            user_id: 42,
            user_key: "owner-key",
            conversation_id: "conversation with spaces",
            text: "ignored",
            answer: "ignored",
            updated_at: 300,
        },
    );

    let page = list_conversation_history(&state, &identity("user"), None, None).unwrap();
    assert!(page.turns.is_empty());
    assert_eq!(
        list_conversation_history(&state, &identity("user"), None, Some("bad-cursor"))
            .unwrap_err()
            .to_string(),
        "conversation_history_cursor_invalid"
    );
}

#[test]
fn large_conversation_bodies_expose_verified_ranges_without_data_loss() {
    let state = state_with_tasks();
    let task_id = Uuid::new_v4();
    let question = "完整问题段落。".repeat(4_000);
    let answer = "完整回答段落。".repeat(12_000);
    insert_turn(
        &state,
        TurnFixture {
            task_id,
            user_id: 42,
            user_key: "owner-key",
            conversation_id: "chat-thread-large-body",
            text: &question,
            answer: &answer,
            updated_at: 700,
        },
    );
    let error_text = "结构化失败详情。".repeat(2_000);
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE tasks SET error_text = ?2 WHERE task_id = ?1",
            rusqlite::params![task_id.to_string(), error_text],
        )
        .unwrap();

    let history = list_conversation_history(&state, &identity("user"), Some(1), None).unwrap();
    let turn = &history.turns[0];
    let descriptor = turn.assistant_text_result.as_ref().unwrap();
    assert!(!descriptor.complete);
    assert_eq!(descriptor.original_size_bytes, answer.len());
    assert_eq!(
        descriptor.returned_size_bytes,
        turn.assistant_text.as_ref().unwrap().len()
    );
    assert!(!turn.user_text_result.as_ref().unwrap().complete);
    assert!(!turn.error_text_result.as_ref().unwrap().complete);
    assert_eq!(
        descriptor.continuation.as_ref().unwrap().kind,
        "conversation_body_range"
    );

    let mut reconstructed = turn.assistant_text.clone().unwrap();
    let mut next = descriptor
        .continuation
        .as_ref()
        .map(|item| item.next_start_byte);
    while let Some(start) = next {
        let page = read_conversation_body_range(
            &state,
            &identity("user"),
            &task_id.to_string(),
            ConversationBodyField::Assistant,
            Some(start),
            Some(17_003),
            Some(&descriptor.content_sha256),
        )
        .unwrap();
        assert_eq!(page.start_byte, reconstructed.len());
        reconstructed.push_str(&page.text);
        next = page.next_start_byte;
    }
    assert_eq!(reconstructed, answer);

    for (field, result) in [
        (
            ConversationBodyField::User,
            turn.user_text_result.as_ref().unwrap(),
        ),
        (
            ConversationBodyField::Error,
            turn.error_text_result.as_ref().unwrap(),
        ),
    ] {
        let start = result.continuation.as_ref().unwrap().next_start_byte;
        let page = read_conversation_body_range(
            &state,
            &identity("user"),
            &task_id.to_string(),
            field,
            Some(start),
            Some(4096),
            Some(&result.content_sha256),
        )
        .unwrap();
        assert_eq!(page.start_byte, start);
        assert!(!page.text.is_empty());
    }
}

#[test]
fn conversation_body_ranges_are_owner_scoped_and_snapshot_bound() {
    let state = state_with_tasks();
    let task_id = Uuid::new_v4();
    insert_turn(
        &state,
        TurnFixture {
            task_id,
            user_id: 42,
            user_key: "owner-key",
            conversation_id: "chat-thread-snapshot",
            text: "检查快照",
            answer: "原始回答",
            updated_at: 701,
        },
    );
    let page = read_conversation_body_range(
        &state,
        &identity("user"),
        &task_id.to_string(),
        ConversationBodyField::Assistant,
        None,
        None,
        None,
    )
    .unwrap();
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE tasks SET result_json = ?2 WHERE task_id = ?1",
            rusqlite::params![task_id.to_string(), json!({"text": "更新回答"}).to_string()],
        )
        .unwrap();
    assert_eq!(
        read_conversation_body_range(
            &state,
            &identity("user"),
            &task_id.to_string(),
            ConversationBodyField::Assistant,
            None,
            None,
            Some(&page.content_sha256),
        )
        .unwrap_err()
        .to_string(),
        "conversation_body_stale_snapshot"
    );

    let mut other = identity("user");
    other.user_key = "other-key".to_string();
    other.user_id = 99;
    assert_eq!(
        read_conversation_body_range(
            &state,
            &other,
            &task_id.to_string(),
            ConversationBodyField::Assistant,
            None,
            None,
            None,
        )
        .unwrap_err()
        .to_string(),
        "conversation_body_not_found"
    );
}
