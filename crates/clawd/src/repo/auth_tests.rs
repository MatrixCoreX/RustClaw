use super::schema::{rebuild_auth_keys_for_flexible_roles, rebuild_channel_tables_for_ui};
use super::*;
use super::{
    get_auth_key_value_by_id_from_db, normalize_auth_key_role, rebind_user_key_references,
    upsert_webd_login_account, verify_webd_password_login,
};
use rusqlite::{params, Connection};

fn auth_lifecycle_test_state() -> AppState {
    let state = AppState::test_default_with_fixture_provider();
    let db = state.core.db.get().expect("main db");
    db.execute_batch(crate::INIT_SQL).expect("base schema");
    crate::ensure_schedule_schema(&db).expect("schedule schema");
    crate::ensure_memory_schema(&db).expect("memory schema");
    crate::ensure_channel_schema(&db).expect("channel schema");
    crate::ensure_task_lease_schema(&db).expect("task lease schema");
    ensure_key_auth_schema(&db).expect("auth schema");
    crate::repo::child_task_graph::ensure_child_task_graph_schema(&db).expect("child task schema");
    crate::memory::indexing::ensure_retrieval_schema(&db).expect("retrieval schema");
    drop(db);
    state
}

fn seed_kb_user_data(state: &AppState, user_key: &str) {
    let db = state.core.skill_storage.kb_pool().get().expect("KB db");
    let payload = serde_json::json!({
        "namespace": "docs",
        "owner_user_key": user_key,
        "updated_at_epoch": 1,
        "next_chunk_seq": 1,
        "docs": {},
        "chunks": []
    })
    .to_string();
    db.execute(
        "INSERT INTO kb_namespaces
            (owner_user_key, namespace, payload_json, updated_at_epoch)
         VALUES (?1, 'docs', ?2, 1)",
        params![user_key, payload],
    )
    .expect("seed KB namespace");
    db.execute(
        "INSERT INTO memory_retrieval_index (
            source_kind, source_ref, user_id, chat_id, user_key, memory_kind,
            search_text, metadata_json, created_at_ts, updated_at_ts
         ) VALUES (
            'kb_doc', ?1, 0, 0, ?2, 'knowledge_doc', 'manual', ?3, 1, 1
         )",
        params![
            format!("kb:{user_key}:docs:chunk-1"),
            user_key,
            serde_json::json!({
                "owner_user_key": user_key,
                "namespace": "docs"
            })
            .to_string()
        ],
    )
    .expect("seed KB retrieval row");
}

fn kb_user_row_counts(state: &AppState, user_key: &str) -> (i64, i64) {
    let db = state.core.skill_storage.kb_pool().get().expect("KB db");
    let namespaces = db
        .query_row(
            "SELECT COUNT(*) FROM kb_namespaces WHERE owner_user_key = ?1",
            params![user_key],
            |row| row.get(0),
        )
        .expect("count KB namespaces");
    let retrieval = db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE user_key = ?1",
            params![user_key],
            |row| row.get(0),
        )
        .expect("count KB retrieval rows");
    (namespaces, retrieval)
}

#[test]
fn rebuild_channel_tables_upgrades_channel_constraints_for_wechat() {
    let db = Connection::open_in_memory().expect("open sqlite");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            channel TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark')),
            external_user_id TEXT,
            external_chat_id TEXT,
            message_id INTEGER,
            user_key TEXT,
            kind TEXT NOT NULL CHECK (kind IN ('ask', 'run_skill', 'admin')),
            payload_json TEXT NOT NULL,
            status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'canceled', 'timeout')),
            result_json TEXT,
            error_text TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE scheduled_jobs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id TEXT NOT NULL UNIQUE,
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            channel TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark')),
            external_user_id TEXT,
            external_chat_id TEXT,
            user_key TEXT,
            schedule_type TEXT NOT NULL CHECK (schedule_type IN ('once', 'daily', 'weekly', 'interval', 'cron')),
            run_at INTEGER,
            time_of_day TEXT,
            weekday INTEGER,
            every_minutes INTEGER,
            cron_expr TEXT,
            timezone TEXT NOT NULL,
            task_kind TEXT NOT NULL CHECK (task_kind IN ('ask', 'run_skill')),
            task_payload_json TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            notify_on_success INTEGER NOT NULL DEFAULT 1,
            notify_on_failure INTEGER NOT NULL DEFAULT 1,
            last_run_at TEXT,
            next_run_at INTEGER,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE memories (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            user_key TEXT,
            channel TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark')),
            external_chat_id TEXT,
            role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
            content TEXT NOT NULL,
            created_at TEXT NOT NULL,
            created_at_ts INTEGER NOT NULL DEFAULT 0,
            memory_type TEXT NOT NULL DEFAULT 'generic',
            salience REAL NOT NULL DEFAULT 0.5,
            is_instructional INTEGER NOT NULL DEFAULT 0,
            safety_flag TEXT NOT NULL DEFAULT 'normal'
        );",
    )
    .expect("create legacy tables");

    rebuild_channel_tables_for_ui(&db).expect("rebuild channel tables");

    let sql: String = db
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'tasks'",
            [],
            |row| row.get(0),
        )
        .expect("read tasks schema");
    assert!(
        sql.contains("'wechat'"),
        "tasks schema should allow wechat: {sql}"
    );
    let scheduled_sql: String = db
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'scheduled_jobs'",
            [],
            |row| row.get(0),
        )
        .expect("read scheduled_jobs schema");
    assert!(
        scheduled_sql.contains("isolation_profile"),
        "scheduled_jobs schema should preserve automation isolation profile: {scheduled_sql}"
    );
    assert!(
        scheduled_sql.contains("permission_policy_json"),
        "scheduled_jobs schema should preserve automation permission policy: {scheduled_sql}"
    );
    assert!(
        scheduled_sql.contains("thread_resume_enabled"),
        "scheduled_jobs schema should preserve automation thread resume flag: {scheduled_sql}"
    );
    assert!(
        scheduled_sql.contains("last_thread_task_id"),
        "scheduled_jobs schema should preserve automation thread task id: {scheduled_sql}"
    );

    db.execute(
        "INSERT INTO tasks (task_id, user_id, chat_id, channel, kind, payload_json, status, created_at, updated_at)
         VALUES ('t1', 1, 1, 'wechat', 'ask', '{}', 'queued', '1', '1')",
        [],
    )
    .expect("insert wechat task");
}

#[test]
fn get_auth_key_value_by_id_returns_full_key() {
    let db = Connection::open_in_memory().expect("open sqlite");
    db.execute_batch(crate::KEY_AUTH_UPGRADE_SQL)
        .expect("create auth schema");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES (?1, 'admin', 1, '123', NULL)",
        params!["rk-full-key-value"],
    )
    .expect("insert auth key");

    let resolved = get_auth_key_value_by_id_from_db(&db, 1).expect("query key");
    assert_eq!(resolved.as_deref(), Some("rk-full-key-value"));
}

#[test]
fn ensure_bootstrap_admin_key_creates_default_webd_login_for_empty_db() {
    let db = Connection::open_in_memory().expect("open sqlite");
    db.execute_batch(crate::KEY_AUTH_UPGRADE_SQL)
        .expect("create auth schema");
    db.execute_batch(crate::WEBD_LOGIN_SQL)
        .expect("create webd login schema");

    let created_key = ensure_bootstrap_admin_key(&db)
        .expect("bootstrap admin key")
        .expect("created key");

    let login_key =
        verify_webd_password_login(&db, "rustclaw", "123456").expect("verify default webd login");
    assert_eq!(login_key.as_deref(), Some(created_key.as_str()));
}

#[test]
fn ensure_bootstrap_admin_key_backfills_default_webd_login_for_existing_admin() {
    let db = Connection::open_in_memory().expect("open sqlite");
    db.execute_batch(crate::KEY_AUTH_UPGRADE_SQL)
        .expect("create auth schema");
    db.execute_batch(crate::WEBD_LOGIN_SQL)
        .expect("create webd login schema");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES (?1, 'admin', 1, '123', NULL)",
        params!["rk-existing-admin"],
    )
    .expect("insert existing admin key");

    let created_key = ensure_bootstrap_admin_key(&db).expect("bootstrap admin key");

    assert_eq!(created_key, None);
    let login_key =
        verify_webd_password_login(&db, "rustclaw", "123456").expect("verify default webd login");
    assert_eq!(login_key.as_deref(), Some("rk-existing-admin"));
}

#[test]
fn normalize_auth_key_role_supports_builtin_and_custom_values() {
    assert_eq!(normalize_auth_key_role("admin").expect("admin"), "admin");
    assert_eq!(normalize_auth_key_role("USER").expect("user"), "user");
    assert_eq!(normalize_auth_key_role(" guest ").expect("guest"), "guest");
    assert_eq!(
        normalize_auth_key_role("finance_viewer").expect("custom"),
        "finance_viewer"
    );
}

#[test]
fn admin_rotation_rebinds_crypto_and_kb_storage() {
    let state = auth_lifecycle_test_state();
    let old_user_key = "rk-admin-old";
    {
        let db = state.core.db.get().expect("main db");
        db.execute(
            "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
             VALUES (?1, 'admin', 1, '123', NULL)",
            params![old_user_key],
        )
        .expect("seed admin key");
    }
    super::super::crypto_storage::upsert_for_user_key(
        &state,
        old_user_key,
        "okx",
        "api-key",
        "api-secret",
        None,
    )
    .expect("seed crypto credential");
    seed_kb_user_data(&state, old_user_key);

    let new_user_key = create_auth_key(&state, "admin").expect("rotate admin key");

    assert_eq!(
        super::super::crypto_storage::credential_context_for_user_key(&state, old_user_key)
            .expect("old crypto context"),
        serde_json::json!({})
    );
    assert_eq!(
        super::super::crypto_storage::credential_context_for_user_key(&state, &new_user_key)
            .expect("new crypto context")["okx"]["api_secret"],
        "api-secret"
    );
    assert_eq!(kb_user_row_counts(&state, old_user_key), (0, 0));
    assert_eq!(kb_user_row_counts(&state, &new_user_key), (1, 1));

    let kb = state.core.skill_storage.kb_pool().get().expect("KB db");
    let (payload, source_ref, metadata): (String, String, String) = kb
        .query_row(
            "SELECT n.payload_json, r.source_ref, r.metadata_json
             FROM kb_namespaces n
             JOIN memory_retrieval_index r ON r.user_key = n.owner_user_key
             WHERE n.owner_user_key = ?1",
            params![new_user_key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read rebound KB identities");
    assert!(payload.contains(&new_user_key));
    assert!(source_ref.contains(&new_user_key));
    assert!(metadata.contains(&new_user_key));
}

#[test]
fn deleting_auth_key_removes_only_its_skill_owned_data() {
    let state = auth_lifecycle_test_state();
    let deleted_user_key = create_auth_key(&state, "user").expect("create deleted user");
    let retained_user_key = create_auth_key(&state, "user").expect("create retained user");
    for user_key in [&deleted_user_key, &retained_user_key] {
        super::super::crypto_storage::upsert_for_user_key(
            &state,
            user_key,
            "okx",
            "api-key",
            "api-secret",
            None,
        )
        .expect("seed crypto credential");
        seed_kb_user_data(&state, user_key);
    }
    let key_id = {
        let db = state.core.db.get().expect("main db");
        db.query_row(
            "SELECT rowid FROM auth_keys WHERE user_key = ?1",
            params![deleted_user_key],
            |row| row.get::<_, i64>(0),
        )
        .expect("deleted key id")
    };

    assert!(delete_auth_key_by_id(&state, key_id, "rk-separate-actor").expect("delete auth key"));

    assert_eq!(
        super::super::crypto_storage::credential_context_for_user_key(&state, &deleted_user_key)
            .expect("deleted crypto context"),
        serde_json::json!({})
    );
    assert_eq!(kb_user_row_counts(&state, &deleted_user_key), (0, 0));
    assert_eq!(kb_user_row_counts(&state, &retained_user_key), (1, 1));
    assert_ne!(
        super::super::crypto_storage::credential_context_for_user_key(&state, &retained_user_key)
            .expect("retained crypto context"),
        serde_json::json!({})
    );
}

#[test]
fn factory_reset_clears_skill_owned_data_and_recreates_one_admin() {
    let state = auth_lifecycle_test_state();
    let user_key = create_auth_key(&state, "user").expect("create user");
    super::super::crypto_storage::upsert_for_user_key(
        &state,
        &user_key,
        "okx",
        "api-key",
        "api-secret",
        None,
    )
    .expect("seed crypto credential");
    seed_kb_user_data(&state, &user_key);

    let result = factory_reset_auth_state(&state).expect("factory reset");

    assert_eq!(result.exchange_credentials_deleted, 1);
    assert_eq!(
        super::super::crypto_storage::credential_context_for_user_key(&state, &user_key)
            .expect("cleared crypto context"),
        serde_json::json!({})
    );
    assert_eq!(kb_user_row_counts(&state, &user_key), (0, 0));
    let db = state.core.db.get().expect("main db");
    let (count, admin_key): (i64, String) = db
        .query_row(
            "SELECT COUNT(*), MAX(user_key) FROM auth_keys WHERE role = 'admin'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("admin after reset");
    assert_eq!(count, 1);
    assert_eq!(admin_key, result.admin_user_key);
}

#[test]
fn rebuild_auth_keys_for_flexible_roles_allows_guest_and_custom_roles() {
    let db = Connection::open_in_memory().expect("open sqlite");
    db.execute_batch(
        "CREATE TABLE auth_keys (
            user_key     TEXT PRIMARY KEY,
            role         TEXT NOT NULL CHECK (role IN ('admin', 'user')),
            enabled      INTEGER NOT NULL DEFAULT 1,
            created_at   TEXT NOT NULL,
            last_used_at TEXT
        );",
    )
    .expect("create legacy auth_keys");

    rebuild_auth_keys_for_flexible_roles(&db).expect("rebuild auth_keys");

    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES (?1, ?2, 1, '123', NULL)",
        params!["rk-guest", "guest"],
    )
    .expect("insert guest role");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES (?1, ?2, 1, '124', NULL)",
        params!["rk-custom", "finance_viewer"],
    )
    .expect("insert custom role");
}

#[test]
fn rebind_user_key_references_updates_related_tables() {
    let mut db = Connection::open_in_memory().expect("open sqlite");
    db.execute_batch(
        "CREATE TABLE auth_keys (
            user_key     TEXT PRIMARY KEY,
            role         TEXT NOT NULL,
            enabled      INTEGER NOT NULL DEFAULT 1,
            created_at   TEXT NOT NULL,
            last_used_at TEXT
        );
        CREATE TABLE channel_bindings (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            channel TEXT NOT NULL,
            external_user_id TEXT,
            external_chat_id TEXT,
            user_key TEXT NOT NULL,
            bound_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            user_key TEXT
        );
        CREATE TABLE scheduled_jobs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_key TEXT
        );
        CREATE TABLE memories (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_key TEXT
        );
        CREATE TABLE long_term_memories (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_key TEXT
        );
        CREATE TABLE audit_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_key TEXT
        );
        CREATE TABLE user_preferences (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_key TEXT
        );
        CREATE TABLE webd_login_accounts (
            username TEXT PRIMARY KEY,
            password_hash TEXT NOT NULL,
            user_key TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE pending_channel_bind_sessions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_key TEXT NOT NULL
        );",
    )
    .expect("create minimal tables");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES (?1, 'admin', 1, '123', NULL)",
        params!["rk-old-admin"],
    )
    .expect("insert old admin");
    db.execute(
        "INSERT INTO channel_bindings (channel, external_user_id, external_chat_id, user_key, bound_at, updated_at)
         VALUES ('telegram', 'u1', 'c1', ?1, '1', '1')",
        params!["rk-old-admin"],
    )
    .expect("insert channel binding");
    db.execute(
        "INSERT INTO webd_login_accounts (username, password_hash, user_key, enabled, created_at, updated_at)
         VALUES ('admin', 'hash', ?1, 1, '1', '1')",
        params!["rk-old-admin"],
    )
    .expect("insert webd login");

    let tx = db.transaction().expect("begin tx");
    rebind_user_key_references(&tx, "rk-old-admin", "rk-new-admin").expect("rebind refs");
    tx.commit().expect("commit tx");

    let channel_binding_key: String = db
        .query_row(
            "SELECT user_key FROM channel_bindings WHERE channel = 'telegram' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("read channel binding user_key");
    let webd_key: String = db
        .query_row(
            "SELECT user_key FROM webd_login_accounts WHERE username = 'admin' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("read webd user_key");
    assert_eq!(channel_binding_key, "rk-new-admin");
    assert_eq!(webd_key, "rk-new-admin");
}

#[test]
fn upsert_webd_login_account_replaces_previous_username_for_same_key() {
    let db = Connection::open_in_memory().expect("open sqlite");
    db.execute_batch(crate::KEY_AUTH_UPGRADE_SQL)
        .expect("create auth schema");
    db.execute_batch(
        "CREATE TABLE webd_login_accounts (
            username TEXT PRIMARY KEY,
            password_hash TEXT NOT NULL,
            user_key TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )
    .expect("create webd_login_accounts");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES (?1, 'user', 1, '123', NULL)",
        params!["rk-user-1"],
    )
    .expect("insert auth key");

    upsert_webd_login_account(&db, "alice", "pw-1", "rk-user-1").expect("create first username");
    upsert_webd_login_account(&db, "alice_new", "pw-2", "rk-user-1").expect("replace username");

    let usernames: Vec<String> = db
        .prepare("SELECT username FROM webd_login_accounts ORDER BY username")
        .expect("prepare usernames")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query usernames")
        .map(|row| row.expect("username row"))
        .collect();
    assert_eq!(usernames, vec!["alice_new".to_string()]);
    assert_eq!(
        verify_webd_password_login(&db, "alice_new", "pw-2").expect("verify login"),
        Some("rk-user-1".to_string())
    );
}

#[test]
fn upsert_webd_login_account_rejects_username_used_by_another_key() {
    let db = Connection::open_in_memory().expect("open sqlite");
    db.execute_batch(crate::KEY_AUTH_UPGRADE_SQL)
        .expect("create auth schema");
    db.execute_batch(
        "CREATE TABLE webd_login_accounts (
            username TEXT PRIMARY KEY,
            password_hash TEXT NOT NULL,
            user_key TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )
    .expect("create webd_login_accounts");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES (?1, 'user', 1, '123', NULL)",
        params!["rk-user-1"],
    )
    .expect("insert first auth key");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES (?1, 'user', 1, '124', NULL)",
        params!["rk-user-2"],
    )
    .expect("insert second auth key");

    upsert_webd_login_account(&db, "alice", "pw-1", "rk-user-1").expect("create first username");
    let err = upsert_webd_login_account(&db, "alice", "pw-2", "rk-user-2")
        .expect_err("reject duplicate username");
    assert!(
        err.to_string().contains("username already assigned"),
        "unexpected error: {err}"
    );
}

#[test]
fn pending_feishu_bind_session_lifecycle() {
    let mut db = Connection::open_in_memory().expect("open sqlite");
    db.execute_batch(crate::INIT_SQL)
        .expect("create base schema");
    crate::ensure_memory_schema(&db).expect("create memory schema");
    ensure_key_auth_schema(&db).expect("create auth schema");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES (?1, 'user', 1, '123', NULL)",
        params!["rk-pending-session-user"],
    )
    .expect("insert auth key");

    let expires_at = (crate::now_ts().parse::<i64>().expect("current ts") + 600).to_string();
    let created = create_pending_channel_bind_session(
        &mut db,
        "feishu",
        "rk-pending-session-user",
        &expires_at,
    )
    .expect("create pending bind session");
    assert_eq!(created.channel, "feishu");
    assert_eq!(created.user_key, "rk-pending-session-user");
    assert_eq!(created.status, "pending");
    assert!(!created.bind_token.is_empty());

    let by_id = get_pending_channel_bind_session_by_id(&db, created.id)
        .expect("load by id")
        .expect("session by id");
    assert_eq!(by_id.bind_token, created.bind_token);

    let by_token = get_pending_channel_bind_session_by_token(&db, &created.bind_token)
        .expect("load by token")
        .expect("session by token");
    assert_eq!(by_token.id, created.id);

    let detected = mark_pending_channel_bind_session_detected(
        &mut db,
        created.id,
        "feishu-user-123",
        "feishu-chat-456",
    )
    .expect("mark detected");
    assert_eq!(detected.status, "detected");
    assert_eq!(
        detected.external_user_id.as_deref(),
        Some("feishu-user-123")
    );
    assert_eq!(
        detected.external_chat_id.as_deref(),
        Some("feishu-chat-456")
    );

    let bound = finalize_pending_channel_bind_session(&mut db, created.id)
        .expect("finalize pending bind session");
    assert_eq!(bound.status, "bound");

    let binding: (String, String, String, String) = db
        .query_row(
            "SELECT channel, external_user_id, external_chat_id, user_key
             FROM channel_bindings
             WHERE channel = 'feishu'
             ORDER BY id DESC
             LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read channel binding");
    assert_eq!(
        binding,
        (
            "feishu".to_string(),
            "feishu-user-123".to_string(),
            "feishu-chat-456".to_string(),
            "rk-pending-session-user".to_string(),
        )
    );

    let terminal = get_pending_channel_bind_session_by_id(&db, created.id)
        .expect("reload terminal session")
        .expect("terminal session");
    assert_eq!(terminal.status, "bound");
}

#[test]
fn direct_bind_finalizes_latest_pending_feishu_session() {
    let mut db = Connection::open_in_memory().expect("open sqlite");
    db.execute_batch(crate::INIT_SQL)
        .expect("create base schema");
    crate::ensure_memory_schema(&db).expect("create memory schema");
    ensure_key_auth_schema(&db).expect("create auth schema");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES (?1, 'user', 1, '123', NULL)",
        params!["rk-direct-bind-user"],
    )
    .expect("insert auth key");

    let expires_at = (crate::now_ts().parse::<i64>().expect("current ts") + 600).to_string();
    let created =
        create_pending_channel_bind_session(&mut db, "feishu", "rk-direct-bind-user", &expires_at)
            .expect("create pending bind session");
    assert_eq!(created.status, "pending");
    assert!(created.external_user_id.is_none());
    assert!(created.external_chat_id.is_none());

    finalize_latest_pending_channel_bind_session_for_user(
        &mut db,
        "feishu",
        "rk-direct-bind-user",
        Some("ou-direct-bind"),
        Some("oc-direct-bind"),
    )
    .expect("finalize latest pending session");

    let terminal = get_pending_channel_bind_session_by_id(&db, created.id)
        .expect("reload terminal session")
        .expect("terminal session");
    assert_eq!(terminal.status, "bound");
    assert_eq!(terminal.external_user_id.as_deref(), Some("ou-direct-bind"));
    assert_eq!(terminal.external_chat_id.as_deref(), Some("oc-direct-bind"));
}
