use super::*;
use rusqlite::params;

#[test]
fn delivery_record_preserves_terminal_payload_and_channel_route() {
    let state = AppState::test_default_with_fixture_provider();
    let task_id = uuid::Uuid::new_v4().to_string();
    let payload = serde_json::json!({
        "channel": "telegram",
        "telegram_bot_name": "secondary",
        "channel_ingress": {
            "schema_version": 1,
            "channel": "telegram",
            "adapter": "telegram_bot",
            "external_user_id": "7",
            "external_chat_id": "9",
            "reply_target": {"kind": "chat", "external_id": "9"},
            "locale": "zh-CN"
        }
    });
    let result = serde_json::json!({"text": "done"});
    let db = state.core.db.get().expect("db");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            user_key TEXT,
            channel TEXT NOT NULL,
            external_user_id TEXT,
            external_chat_id TEXT,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            status TEXT NOT NULL,
            result_json TEXT,
            error_text TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            claim_attempt INTEGER NOT NULL DEFAULT 0
        );",
    )
    .expect("create tasks table");
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, external_user_id,
            external_chat_id, kind, payload_json, status, result_json,
            created_at, updated_at
         ) VALUES (?1, 7, 9, 'user-key', 'telegram', '7', '9', 'ask', ?2,
                   'succeeded', ?3, '1', '2')",
        params![task_id, payload.to_string(), result.to_string()],
    )
    .expect("insert task");
    drop(db);

    let record = get_task_delivery_record(&state, &task_id)
        .expect("load record")
        .expect("record");
    assert_eq!(record.status, "succeeded");
    assert_eq!(record.task.external_chat_id.as_deref(), Some("9"));
    assert_eq!(record.result_json.as_ref().unwrap()["text"], "done");
    assert_eq!(
        serde_json::from_str::<Value>(&record.task.payload_json).unwrap()["telegram_bot_name"],
        "secondary"
    );
}
