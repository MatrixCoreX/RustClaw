use super::{
    mark_matching_user_memory_as_safety_signal, normalize_language_tag, normalized_preference_key,
    normalized_preference_source_ref_key, normalized_preference_value,
};

use rusqlite::{params, Connection};

#[test]
fn memory_intent_language_tag_normalization_is_structural() {
    assert_eq!(normalize_language_tag("zh_CN"), Some("zh-CN".to_string()));
    assert_eq!(normalize_language_tag("ko-KR"), Some("ko-KR".to_string()));
    assert_eq!(normalize_language_tag("fr"), Some("fr-FR".to_string()));
    assert_eq!(normalize_language_tag("EN_us"), Some("en-US".to_string()));
    assert_eq!(normalize_language_tag("中文"), None);
    assert_eq!(normalize_language_tag("-en"), None);
}

#[test]
fn memory_intent_preference_key_allowlist_is_schema_token_based() {
    assert_eq!(
        normalized_preference_key("response_language"),
        Some("response_language".to_string())
    );
    assert_eq!(normalized_preference_key("中文回复"), None);
}

#[test]
fn memory_intent_preference_source_ref_key_is_structural() {
    assert_eq!(
        normalized_preference_source_ref_key("preference:response_language"),
        Some("response_language".to_string())
    );
    assert_eq!(
        normalized_preference_source_ref_key("response_format"),
        Some("response_format".to_string())
    );
    assert_eq!(
        normalized_preference_source_ref_key("language preference"),
        None
    );
}

#[test]
fn memory_intent_preference_values_reject_unstructured_text() {
    assert_eq!(
        normalized_preference_value("response_format", "plain_text"),
        Some("plain_text".to_string())
    );
    assert_eq!(
        normalized_preference_value("response_format", "plain words"),
        None
    );
}

#[test]
fn structured_safety_signal_marks_memory_and_removes_retrieval_index() {
    let db = Connection::open_in_memory().expect("db");
    db.execute_batch(
        "CREATE TABLE memories (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             user_id INTEGER NOT NULL,
             chat_id INTEGER NOT NULL,
             user_key TEXT NOT NULL,
             role TEXT NOT NULL,
             content TEXT NOT NULL,
             memory_type TEXT NOT NULL,
             salience REAL NOT NULL,
             is_instructional INTEGER NOT NULL,
             safety_flag TEXT NOT NULL
         );
         CREATE TABLE memory_retrieval_index (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             source_kind TEXT NOT NULL,
             source_memory_id INTEGER
         );",
    )
    .expect("schema");
    db.execute(
        "INSERT INTO memories
         (user_id, chat_id, user_key, role, content, memory_type, salience, is_instructional, safety_flag)
         VALUES (?1, ?2, ?3, 'user', ?4, 'generic', 0.48, 0, 'normal')",
        params![7, 9, "user:test", "untrusted prior instruction"],
    )
    .expect("memory");
    let memory_id = db.last_insert_rowid();
    db.execute(
        "INSERT INTO memory_retrieval_index (source_kind, source_memory_id)
         VALUES ('memory', ?1)",
        params![memory_id],
    )
    .expect("index");

    let updated = mark_matching_user_memory_as_safety_signal(
        &db,
        7,
        9,
        "user:test",
        "untrusted prior instruction",
    )
    .expect("mark safety signal");

    assert_eq!(updated, 1);
    let row = db
        .query_row(
            "SELECT memory_type, safety_flag, is_instructional FROM memories WHERE id = ?1",
            params![memory_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .expect("memory row");
    assert_eq!(
        row,
        ("safety_signal".to_string(), "injection_like".to_string(), 0)
    );
    let indexed: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE source_memory_id = ?1",
            params![memory_id],
            |row| row.get(0),
        )
        .expect("index count");
    assert_eq!(indexed, 0);
}
