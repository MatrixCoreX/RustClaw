use super::*;

fn db() -> Connection {
    let db = Connection::open_in_memory().expect("open");
    db.execute_batch(
        "CREATE TABLE user_preferences (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL, chat_id INTEGER NOT NULL,
            pref_key TEXT NOT NULL, pref_value TEXT NOT NULL,
            confidence REAL NOT NULL, source TEXT NOT NULL,
            updated_at TEXT NOT NULL, updated_at_ts INTEGER NOT NULL,
            user_key TEXT, UNIQUE(user_id, chat_id, user_key, pref_key));
         CREATE TABLE tasks (
            payload_json TEXT NOT NULL, user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL, user_key TEXT, created_at TEXT NOT NULL);
         CREATE TABLE channel_bindings (
            id INTEGER PRIMARY KEY AUTOINCREMENT, channel TEXT NOT NULL,
            external_user_id TEXT, external_chat_id TEXT, user_key TEXT NOT NULL);",
    )
    .expect("schema");
    db
}

#[test]
fn update_is_typed_idempotent_and_conversation_precedes_user_scope() {
    let db = db();
    update(&db, 1, 0, "rk-test", Some("en_us"), None).expect("user pref");
    update(&db, 1, 2, "rk-test", Some("zh-cn"), Some("voice")).expect("chat pref");
    update(&db, 1, 2, "rk-test", Some("zh-CN"), Some("voice")).expect("idempotent");
    let count: i64 = db
        .query_row(
            concat!("SELECT COUNT(*)", " FROM user_preferences"),
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(count, 3);
    let resolved =
        resolve_locale(&db, 1, 2, Some("rk-test"), Some("fr-FR"), "en-US").expect("resolve");
    assert_eq!(resolved.locale, "zh-CN");
    assert_eq!(resolved.source, "conversation_preference");
}

#[test]
fn locale_fallback_order_is_platform_conversation_runtime_safe_default() {
    let db = db();
    let platform = resolve_locale(&db, 1, 2, None, Some("ja-JP"), "zh-CN").expect("platform");
    assert_eq!(platform.source, "platform");
    db.execute(
        "INSERT INTO tasks(payload_json,user_id,chat_id,user_key,created_at)
         VALUES (?1,1,2,NULL,'1')",
        [r#"{"channel_ingress":{"locale":"fr-FR"}}"#],
    )
    .expect("task");
    let conversation = resolve_locale(&db, 1, 2, None, None, "zh-CN").expect("conversation");
    assert_eq!(conversation.locale, "fr-FR");
    assert_eq!(conversation.source, "conversation");
    assert_eq!(
        resolve_locale(&db, 9, 9, None, None, "zh-CN")
            .expect("runtime")
            .source,
        "runtime_default"
    );
    assert_eq!(
        resolve_locale(&db, 9, 9, None, None, "invalid locale")
            .expect("safe")
            .source,
        "safe_default"
    );
}

#[test]
fn legacy_voice_migration_is_binding_scoped_and_idempotent() {
    let db = db();
    db.execute(
        "INSERT INTO channel_bindings(channel,external_user_id,external_chat_id,user_key)
         VALUES ('telegram','42','42','rk-test')",
        [],
    )
    .expect("binding");
    let legacy = std::collections::HashMap::from([
        ("42".to_string(), "voice".to_string()),
        ("missing".to_string(), "text".to_string()),
    ]);
    let first = migrate_legacy_telegram_voice_preferences(&db, &legacy).expect("migrate");
    assert_eq!(first.migrated, 1);
    assert_eq!(first.binding_missing, 1);
    let second = migrate_legacy_telegram_voice_preferences(&db, &legacy).expect("repeat");
    assert_eq!(second.migrated, 0);
    assert_eq!(second.already_current, 1);
}
