use claw_core::types::{AuthIdentity, ExchangeCredentialStatus};
use rusqlite::{params, Connection, OptionalExtension};
use tracing::{info, warn};

use crate::db_init::DbPool;
use crate::{mask_secret, normalize_external_id_opt, now_ts, AppState};

#[path = "auth_seed.rs"]
mod auth_seed;
#[path = "auth_webd.rs"]
mod auth_webd;

pub(crate) use auth_seed::seed_channel_bindings;
pub(crate) use auth_webd::{upsert_webd_login_account, verify_webd_password_login};

fn generate_user_key() -> String {
    format!("rk-{}", uuid::Uuid::new_v4().simple())
}

const PENDING_CHANNEL_BIND_STATUS_PENDING: &str = "pending";
const PENDING_CHANNEL_BIND_STATUS_DETECTED: &str = "detected";
const PENDING_CHANNEL_BIND_STATUS_BOUND: &str = "bound";
const PENDING_CHANNEL_BIND_STATUS_FAILED: &str = "failed";
const PENDING_CHANNEL_BIND_STATUS_EXPIRED: &str = "expired";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingChannelBindSession {
    pub(crate) id: i64,
    pub(crate) channel: String,
    pub(crate) user_key: String,
    pub(crate) bind_token: String,
    pub(crate) status: String,
    pub(crate) external_user_id: Option<String>,
    pub(crate) external_chat_id: Option<String>,
    pub(crate) error_text: Option<String>,
    pub(crate) install_device_code: Option<String>,
    pub(crate) install_verification_url: Option<String>,
    pub(crate) install_poll_interval_seconds: Option<i64>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) expires_at: String,
}

fn map_pending_channel_bind_session(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<PendingChannelBindSession> {
    Ok(PendingChannelBindSession {
        id: row.get(0)?,
        channel: row.get(1)?,
        user_key: row.get(2)?,
        bind_token: row.get(3)?,
        status: row.get(4)?,
        external_user_id: row.get(5)?,
        external_chat_id: row.get(6)?,
        error_text: row.get(7)?,
        install_device_code: row.get(8)?,
        install_verification_url: row.get(9)?,
        install_poll_interval_seconds: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
        expires_at: row.get(13)?,
    })
}

pub(crate) fn ensure_bootstrap_admin_key(db: &Connection) -> anyhow::Result<Option<String>> {
    let existing_count: i64 =
        db.query_row("SELECT COUNT(*) FROM auth_keys", [], |row| row.get(0))?;
    if existing_count > 0 {
        return Ok(None);
    }
    let user_key = generate_user_key();
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES (?1, 'admin', 1, ?2, NULL)",
        params![user_key, now_ts()],
    )?;
    Ok(Some(user_key))
}

pub(crate) fn ensure_key_auth_schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(crate::KEY_AUTH_UPGRADE_SQL)?;
    db.execute_batch(crate::WEBD_LOGIN_SQL)?;
    db.execute_batch(include_str!(
        "../../../../migrations/006_pending_channel_bind_sessions.sql"
    ))?;
    crate::ensure_column_exists(
        db,
        "pending_channel_bind_sessions",
        "install_device_code",
        "ALTER TABLE pending_channel_bind_sessions ADD COLUMN install_device_code TEXT",
    )?;
    crate::ensure_column_exists(
        db,
        "pending_channel_bind_sessions",
        "install_verification_url",
        "ALTER TABLE pending_channel_bind_sessions ADD COLUMN install_verification_url TEXT",
    )?;
    crate::ensure_column_exists(
        db,
        "pending_channel_bind_sessions",
        "install_poll_interval_seconds",
        "ALTER TABLE pending_channel_bind_sessions ADD COLUMN install_poll_interval_seconds INTEGER",
    )?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS exchange_api_credentials (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            user_key    TEXT NOT NULL,
            exchange    TEXT NOT NULL,
            api_key     TEXT NOT NULL,
            api_secret  TEXT NOT NULL,
            passphrase  TEXT,
            enabled     INTEGER NOT NULL DEFAULT 1,
            updated_at  TEXT NOT NULL,
            UNIQUE(user_key, exchange)
        );
        CREATE INDEX IF NOT EXISTS idx_exchange_api_credentials_user_exchange
        ON exchange_api_credentials(user_key, exchange);",
    )?;
    crate::ensure_column_exists(
        db,
        "tasks",
        "user_key",
        "ALTER TABLE tasks ADD COLUMN user_key TEXT",
    )?;
    crate::ensure_column_exists(
        db,
        "scheduled_jobs",
        "user_key",
        "ALTER TABLE scheduled_jobs ADD COLUMN user_key TEXT",
    )?;
    crate::ensure_column_exists(
        db,
        "memories",
        "user_key",
        "ALTER TABLE memories ADD COLUMN user_key TEXT",
    )?;
    crate::ensure_column_exists(
        db,
        "long_term_memories",
        "user_key",
        "ALTER TABLE long_term_memories ADD COLUMN user_key TEXT",
    )?;
    // Phase 2.2 Stage 2: audit_logs 已经搬到独立 audit pool（INIT_AUDIT_SQL 自带 user_key 列）。
    // 这里若主库还残留旧表（迁移之前的部署），仍保持 user_key 列对齐，让一次性
    // migrate_audit_logs_from_main_db 能正确读出 user_key 字段。
    let main_has_audit_logs: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='audit_logs'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if main_has_audit_logs > 0 {
        crate::ensure_column_exists(
            db,
            "audit_logs",
            "user_key",
            "ALTER TABLE audit_logs ADD COLUMN user_key TEXT",
        )?;
    }
    crate::ensure_column_exists(
        db,
        "user_preferences",
        "user_key",
        "ALTER TABLE user_preferences ADD COLUMN user_key TEXT",
    )?;
    rebuild_auth_keys_for_flexible_roles(db)?;
    rebuild_user_preferences_for_key_scope(db)?;
    rebuild_long_term_memories_for_key_scope(db)?;
    rebuild_channel_tables_for_ui(db)?;
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_tasks_user_key_created_at ON tasks(user_key, created_at);
         CREATE INDEX IF NOT EXISTS idx_memories_user_key_chat_created_at ON memories(user_key, chat_id, created_at_ts);
         CREATE INDEX IF NOT EXISTS idx_scheduled_jobs_user_key_chat ON scheduled_jobs(user_key, chat_id);
         CREATE INDEX IF NOT EXISTS idx_user_preferences_user_key_chat ON user_preferences(user_key, chat_id, updated_at_ts);
         CREATE INDEX IF NOT EXISTS idx_long_term_memories_user_key_chat_updated_ts ON long_term_memories(user_key, chat_id, updated_at_ts);",
    )?;
    Ok(())
}

fn rebuild_auth_keys_for_flexible_roles(db: &Connection) -> anyhow::Result<()> {
    let table_sql: String = db.query_row(
        "SELECT COALESCE(sql, '') FROM sqlite_master WHERE type = 'table' AND name = 'auth_keys'",
        [],
        |row| row.get(0),
    )?;
    let needs_rebuild = table_sql.contains("CHECK (role IN ('admin', 'user'))")
        || table_sql.contains("CHECK(role IN ('admin', 'user'))");
    if needs_rebuild {
        db.execute_batch(
            "BEGIN IMMEDIATE;
             ALTER TABLE auth_keys RENAME TO auth_keys_old;
             CREATE TABLE auth_keys (
                 user_key     TEXT PRIMARY KEY,
                 role         TEXT NOT NULL,
                 enabled      INTEGER NOT NULL DEFAULT 1,
                 created_at   TEXT NOT NULL,
                 last_used_at TEXT
             );
             INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
             SELECT user_key, role, enabled, created_at, last_used_at
             FROM auth_keys_old;
             DROP TABLE auth_keys_old;
             COMMIT;",
        )?;
    }
    let admin_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM auth_keys WHERE role = 'admin'",
        [],
        |row| row.get(0),
    )?;
    if admin_count <= 1 {
        db.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_auth_keys_single_admin
             ON auth_keys(role) WHERE role = 'admin'",
            [],
        )?;
    } else {
        info!("auth_keys schema: skip single-admin unique index because multiple admin rows already exist");
    }
    Ok(())
}

fn rebuild_user_preferences_for_key_scope(db: &Connection) -> anyhow::Result<()> {
    let table_sql: String = db.query_row(
        "SELECT COALESCE(sql, '') FROM sqlite_master WHERE type = 'table' AND name = 'user_preferences'",
        [],
        |row| row.get(0),
    )?;
    if table_sql.contains("UNIQUE(user_id, chat_id, user_key, pref_key)") {
        return Ok(());
    }
    if !table_sql.contains("UNIQUE(user_id, chat_id, pref_key)") {
        return Ok(());
    }
    db.execute_batch(
        "BEGIN IMMEDIATE;
         ALTER TABLE user_preferences RENAME TO user_preferences_old;
         CREATE TABLE user_preferences (
             id           INTEGER PRIMARY KEY AUTOINCREMENT,
             user_id      INTEGER NOT NULL,
             chat_id      INTEGER NOT NULL,
             pref_key     TEXT NOT NULL,
             pref_value   TEXT NOT NULL,
             confidence   REAL NOT NULL DEFAULT 0.8,
             source       TEXT NOT NULL DEFAULT 'memory_extract',
             updated_at   TEXT NOT NULL,
             updated_at_ts INTEGER NOT NULL DEFAULT 0,
             user_key     TEXT,
             UNIQUE(user_id, chat_id, user_key, pref_key)
         );
         INSERT OR REPLACE INTO user_preferences (
             id, user_id, chat_id, pref_key, pref_value, confidence, source, updated_at, updated_at_ts, user_key
         )
         SELECT
             id, user_id, chat_id, pref_key, pref_value, confidence, source, updated_at, updated_at_ts, user_key
         FROM user_preferences_old
         ORDER BY COALESCE(updated_at_ts, CAST(updated_at AS INTEGER)) ASC, id ASC;
         DROP TABLE user_preferences_old;
         CREATE INDEX IF NOT EXISTS idx_user_preferences_user_chat_updated
         ON user_preferences(user_id, chat_id, updated_at);
         CREATE INDEX IF NOT EXISTS idx_user_preferences_user_chat_updated_ts
         ON user_preferences(user_id, chat_id, updated_at_ts);
         CREATE INDEX IF NOT EXISTS idx_user_preferences_user_key_chat
         ON user_preferences(user_key, chat_id, updated_at_ts);
         COMMIT;",
    )?;
    Ok(())
}

fn rebuild_long_term_memories_for_key_scope(db: &Connection) -> anyhow::Result<()> {
    let table_sql: String = db.query_row(
        "SELECT COALESCE(sql, '') FROM sqlite_master WHERE type = 'table' AND name = 'long_term_memories'",
        [],
        |row| row.get(0),
    )?;
    if table_sql.contains("UNIQUE(user_id, chat_id, user_key)") {
        return Ok(());
    }
    if !table_sql.contains("UNIQUE(user_id, chat_id)") {
        return Ok(());
    }
    db.execute_batch(
        "BEGIN IMMEDIATE;
         ALTER TABLE long_term_memories RENAME TO long_term_memories_old;
         CREATE TABLE long_term_memories (
             id                INTEGER PRIMARY KEY AUTOINCREMENT,
             user_id           INTEGER NOT NULL,
             chat_id           INTEGER NOT NULL,
             summary           TEXT NOT NULL,
             source_memory_id  INTEGER NOT NULL DEFAULT 0,
             created_at        TEXT NOT NULL,
             updated_at        TEXT NOT NULL,
             created_at_ts     INTEGER NOT NULL DEFAULT 0,
             updated_at_ts     INTEGER NOT NULL DEFAULT 0,
             user_key          TEXT,
             UNIQUE(user_id, chat_id, user_key)
         );
         INSERT OR REPLACE INTO long_term_memories (
             id, user_id, chat_id, summary, source_memory_id, created_at, updated_at, created_at_ts, updated_at_ts, user_key
         )
         SELECT
             id, user_id, chat_id, summary, source_memory_id, created_at, updated_at, created_at_ts, updated_at_ts, user_key
         FROM long_term_memories_old
         ORDER BY COALESCE(updated_at_ts, CAST(updated_at AS INTEGER)) ASC, id ASC;
         DROP TABLE long_term_memories_old;
         CREATE INDEX IF NOT EXISTS idx_long_term_memories_updated_at
         ON long_term_memories(updated_at);
         CREATE INDEX IF NOT EXISTS idx_long_term_memories_updated_at_ts
         ON long_term_memories(updated_at_ts);
         CREATE INDEX IF NOT EXISTS idx_long_term_memories_user_key_chat_updated_ts
         ON long_term_memories(user_key, chat_id, updated_at_ts);
         COMMIT;",
    )?;
    Ok(())
}

fn rebuild_channel_tables_for_ui(db: &Connection) -> anyhow::Result<()> {
    let tasks_sql: String = db.query_row(
        "SELECT COALESCE(sql, '') FROM sqlite_master WHERE type = 'table' AND name = 'tasks'",
        [],
        |row| row.get(0),
    )?;
    if tasks_sql.contains("'wechat'") {
        return Ok(());
    }
    info!(
        "channel schema: rebuilding tasks/scheduled_jobs/memories to allow channel=lark/feishu/wechat"
    );
    db.execute_batch(
        "BEGIN IMMEDIATE;
         ALTER TABLE tasks RENAME TO tasks_old;
         CREATE TABLE tasks (
             task_id       TEXT PRIMARY KEY,
             user_id       INTEGER NOT NULL,
             chat_id       INTEGER NOT NULL,
             channel       TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark', 'wechat')),
             external_user_id TEXT,
             external_chat_id TEXT,
             message_id    INTEGER,
             user_key      TEXT,
             kind          TEXT NOT NULL CHECK (kind IN ('ask', 'run_skill', 'admin')),
             payload_json  TEXT NOT NULL,
             status        TEXT NOT NULL CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'canceled', 'timeout')),
             result_json   TEXT,
             error_text    TEXT,
             created_at    TEXT NOT NULL,
             updated_at    TEXT NOT NULL
         );
         INSERT INTO tasks (
             task_id, user_id, chat_id, channel, external_user_id, external_chat_id, message_id, user_key,
             kind, payload_json, status, result_json, error_text, created_at, updated_at
         )
         SELECT
             task_id, user_id, chat_id, channel, external_user_id, external_chat_id, message_id, user_key,
             kind, payload_json, status, result_json, error_text, created_at, updated_at
         FROM tasks_old;
         DROP TABLE tasks_old;
         CREATE INDEX IF NOT EXISTS idx_tasks_status_created_at ON tasks(status, created_at);
         CREATE INDEX IF NOT EXISTS idx_tasks_user_id_created_at ON tasks(user_id, created_at);
         CREATE INDEX IF NOT EXISTS idx_tasks_user_key_created_at ON tasks(user_key, created_at);
         ALTER TABLE scheduled_jobs RENAME TO scheduled_jobs_old;
         CREATE TABLE scheduled_jobs (
             id                INTEGER PRIMARY KEY AUTOINCREMENT,
             job_id            TEXT NOT NULL UNIQUE,
             user_id           INTEGER NOT NULL,
             chat_id           INTEGER NOT NULL,
             channel           TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark', 'wechat')),
             external_user_id  TEXT,
             external_chat_id  TEXT,
             user_key          TEXT,
             schedule_type     TEXT NOT NULL CHECK (schedule_type IN ('once', 'daily', 'weekly', 'interval', 'cron')),
             run_at            INTEGER,
             time_of_day       TEXT,
             weekday           INTEGER,
             every_minutes     INTEGER,
             cron_expr         TEXT,
             timezone          TEXT NOT NULL,
             task_kind         TEXT NOT NULL CHECK (task_kind IN ('ask', 'run_skill')),
             task_payload_json TEXT NOT NULL,
             enabled           INTEGER NOT NULL DEFAULT 1,
             notify_on_success INTEGER NOT NULL DEFAULT 1,
             notify_on_failure INTEGER NOT NULL DEFAULT 1,
             last_run_at       TEXT,
             next_run_at       INTEGER,
             created_at        TEXT NOT NULL,
             updated_at        TEXT NOT NULL
         );
         INSERT INTO scheduled_jobs (
             id, job_id, user_id, chat_id, channel, external_user_id, external_chat_id, user_key,
             schedule_type, run_at, time_of_day, weekday, every_minutes, cron_expr, timezone,
             task_kind, task_payload_json, enabled, notify_on_success, notify_on_failure,
             last_run_at, next_run_at, created_at, updated_at
         )
         SELECT
             id, job_id, user_id, chat_id, channel, external_user_id, external_chat_id, user_key,
             schedule_type, run_at, time_of_day, weekday, every_minutes, cron_expr, timezone,
             task_kind, task_payload_json, enabled, notify_on_success, notify_on_failure,
             last_run_at, next_run_at, created_at, updated_at
         FROM scheduled_jobs_old;
         DROP TABLE scheduled_jobs_old;
         CREATE INDEX IF NOT EXISTS idx_scheduled_jobs_due ON scheduled_jobs(enabled, next_run_at);
         CREATE INDEX IF NOT EXISTS idx_scheduled_jobs_user_chat ON scheduled_jobs(user_id, chat_id);
         CREATE INDEX IF NOT EXISTS idx_scheduled_jobs_user_key_chat ON scheduled_jobs(user_key, chat_id);
         ALTER TABLE memories RENAME TO memories_old;
         CREATE TABLE memories (
             id               INTEGER PRIMARY KEY AUTOINCREMENT,
             user_id          INTEGER NOT NULL,
             chat_id          INTEGER NOT NULL,
             user_key         TEXT,
             channel          TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark', 'wechat')),
             external_chat_id TEXT,
             role             TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
             content          TEXT NOT NULL,
             created_at       TEXT NOT NULL,
             created_at_ts    INTEGER NOT NULL DEFAULT 0,
             memory_type      TEXT NOT NULL DEFAULT 'generic',
             salience         REAL NOT NULL DEFAULT 0.5,
             is_instructional INTEGER NOT NULL DEFAULT 0,
             safety_flag      TEXT NOT NULL DEFAULT 'normal'
         );
         INSERT INTO memories (
             id, user_id, chat_id, user_key, channel, external_chat_id, role, content,
             created_at, created_at_ts, memory_type, salience, is_instructional, safety_flag
         )
         SELECT
             id, user_id, chat_id, user_key, channel, external_chat_id, role, content,
             created_at, created_at_ts, memory_type, salience, is_instructional, safety_flag
         FROM memories_old;
         DROP TABLE memories_old;
         CREATE INDEX IF NOT EXISTS idx_memories_user_chat_created_at ON memories(user_id, chat_id, created_at);
         CREATE INDEX IF NOT EXISTS idx_memories_user_chat_role_id ON memories(user_id, chat_id, role, id DESC);
         CREATE INDEX IF NOT EXISTS idx_memories_user_chat_created_at_ts ON memories(user_id, chat_id, created_at_ts);
         CREATE INDEX IF NOT EXISTS idx_memories_user_key_chat_created_at ON memories(user_key, chat_id, created_at_ts);
         COMMIT;",
    )?;
    Ok(())
}

pub(crate) struct AuthKeyListRow {
    pub(crate) key_id: i64,
    pub(crate) user_key: String,
    pub(crate) user_key_masked: String,
    pub(crate) role: String,
    pub(crate) enabled: i64,
    pub(crate) created_at: String,
    pub(crate) last_used_at: Option<String>,
    pub(crate) webd_username: Option<String>,
}

pub(crate) fn list_auth_keys(state: &AppState) -> anyhow::Result<Vec<AuthKeyListRow>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let mut stmt = db.prepare(
        "SELECT rowid,
                user_key,
                role,
                enabled,
                created_at,
                last_used_at,
                (
                    SELECT username
                    FROM webd_login_accounts
                    WHERE user_key = auth_keys.user_key AND enabled = 1
                    ORDER BY updated_at DESC, username ASC
                    LIMIT 1
                ) AS webd_username
         FROM auth_keys
         ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (key_id, user_key, role, enabled, created_at, last_used_at, webd_username) = row?;
        out.push(AuthKeyListRow {
            key_id,
            user_key_masked: mask_secret(&user_key),
            user_key,
            role,
            enabled,
            created_at,
            last_used_at,
            webd_username,
        });
    }
    Ok(out)
}

fn normalize_auth_key_role(raw: &str) -> anyhow::Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("role is required");
    }
    if trimmed.eq_ignore_ascii_case("admin") {
        return Ok("admin".to_string());
    }
    if trimmed.eq_ignore_ascii_case("user") {
        return Ok("user".to_string());
    }
    if trimmed.eq_ignore_ascii_case("guest") {
        return Ok("guest".to_string());
    }
    Ok(trimmed.to_string())
}

fn has_other_admin_key(db: &Connection, exclude_key_id: Option<i64>) -> anyhow::Result<bool> {
    let count: i64 = if let Some(key_id) = exclude_key_id {
        db.query_row(
            "SELECT COUNT(*) FROM auth_keys WHERE role = 'admin' AND rowid != ?1",
            params![key_id],
            |row| row.get(0),
        )?
    } else {
        db.query_row(
            "SELECT COUNT(*) FROM auth_keys WHERE role = 'admin'",
            [],
            |row| row.get(0),
        )?
    };
    Ok(count > 0)
}

fn rebind_user_key_references(
    tx: &rusqlite::Transaction<'_>,
    old_user_key: &str,
    new_user_key: &str,
) -> anyhow::Result<()> {
    let updates = [
        "UPDATE channel_bindings SET user_key = ?2 WHERE user_key = ?1",
        "UPDATE exchange_api_credentials SET user_key = ?2 WHERE user_key = ?1",
        "UPDATE tasks SET user_key = ?2 WHERE user_key = ?1",
        "UPDATE scheduled_jobs SET user_key = ?2 WHERE user_key = ?1",
        "UPDATE memories SET user_key = ?2 WHERE user_key = ?1",
        "UPDATE long_term_memories SET user_key = ?2 WHERE user_key = ?1",
        // Phase 2.2 Stage 2: audit_logs 已经搬到独立 audit pool，
        // 由 rebind_audit_logs_user_key 在主事务提交后 best-effort 更新。
        "UPDATE user_preferences SET user_key = ?2 WHERE user_key = ?1",
        "UPDATE webd_login_accounts SET user_key = ?2 WHERE user_key = ?1",
        "UPDATE pending_channel_bind_sessions SET user_key = ?2 WHERE user_key = ?1",
    ];
    for sql in updates {
        tx.execute(sql, params![old_user_key, new_user_key])?;
    }
    Ok(())
}

/// Phase 2.2 Stage 2: audit_logs 在独立 audit pool 上，需要单独 best-effort 更新。
/// 失败只 warn，不阻塞 user_key 旋转主流程（审计延迟一致性是可接受的）。
fn rebind_audit_logs_user_key(
    audit_db: &DbPool,
    old_user_key: &str,
    new_user_key: &str,
) -> anyhow::Result<u64> {
    let conn = audit_db
        .get()
        .map_err(|e| anyhow::anyhow!("audit db pool: {e}"))?;
    let updated = conn.execute(
        "UPDATE audit_logs SET user_key = ?2 WHERE user_key = ?1",
        params![old_user_key, new_user_key],
    )?;
    Ok(updated as u64)
}

fn rotate_auth_key_row(
    tx: &rusqlite::Transaction<'_>,
    key_rowid: i64,
    old_user_key: &str,
    new_user_key: &str,
) -> anyhow::Result<()> {
    rebind_user_key_references(tx, old_user_key, new_user_key)?;
    tx.execute(
        "UPDATE auth_keys
         SET user_key = ?2,
             enabled = 1,
             created_at = ?3,
             last_used_at = NULL
         WHERE rowid = ?1",
        params![key_rowid, new_user_key, now_ts()],
    )?;
    Ok(())
}

fn get_auth_key_value_by_id_from_db(
    db: &Connection,
    key_id: i64,
) -> anyhow::Result<Option<String>> {
    let value = db
        .query_row(
            "SELECT user_key FROM auth_keys WHERE rowid = ?1",
            params![key_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(value)
}

pub(crate) fn get_auth_key_value_by_id(
    state: &AppState,
    key_id: i64,
) -> anyhow::Result<Option<String>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    get_auth_key_value_by_id_from_db(&db, key_id)
}

pub(crate) fn create_auth_key(state: &AppState, role: &str) -> anyhow::Result<String> {
    let role = normalize_auth_key_role(role)?;
    let user_key = generate_user_key();
    let mut db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    if role == "admin" {
        let existing_admins = {
            let mut stmt = db.prepare(
                "SELECT rowid, user_key FROM auth_keys WHERE role = 'admin' ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            out
        };
        if existing_admins.len() > 1 {
            anyhow::bail!("multiple admin keys exist; clean them up before rotating admin");
        }
        if let Some((admin_rowid, old_user_key)) = existing_admins.into_iter().next() {
            let tx = db.transaction()?;
            rotate_auth_key_row(&tx, admin_rowid, &old_user_key, &user_key)?;
            tx.commit()?;
            // Phase 2.2 Stage 2: 主事务提交后再异步刷一次 audit_logs（独立 pool）。
            // 失败只 warn，避免审计跨库写入阻塞 admin key 轮换。
            match rebind_audit_logs_user_key(&state.core.audit_db, &old_user_key, &user_key) {
                Ok(n) => {
                    if n > 0 {
                        info!(
                            target = "audit",
                            old_user_key = %mask_secret(&old_user_key),
                            new_user_key = %mask_secret(&user_key),
                            updated_rows = n,
                            "rebound user_key in audit_logs after admin rotation"
                        );
                    }
                }
                Err(err) => warn!(
                    target = "audit",
                    error = %err,
                    "failed to rebind user_key in audit_logs (best-effort, ignored)"
                ),
            }
            return Ok(user_key);
        }
    }
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES (?1, ?2, 1, ?3, NULL)",
        params![user_key, role, now_ts()],
    )?;
    Ok(user_key)
}

fn upsert_channel_binding_row(
    db: &Connection,
    channel: &str,
    external_user_id: Option<&str>,
    external_chat_id: Option<&str>,
    user_key: &str,
) -> anyhow::Result<()> {
    let external_user_id = normalize_external_id_opt(external_user_id);
    let external_chat_id =
        normalize_external_id_opt(external_chat_id).or_else(|| external_user_id.clone());
    if external_user_id.is_none() && external_chat_id.is_none() {
        anyhow::bail!("external_user_id or external_chat_id is required");
    }
    let now = now_ts();
    db.execute(
        "INSERT INTO channel_bindings (channel, external_user_id, external_chat_id, user_key, bound_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT(channel, external_user_id, external_chat_id)
         DO UPDATE SET user_key=excluded.user_key, updated_at=excluded.updated_at",
        params![channel, external_user_id, external_chat_id, user_key, now],
    )?;
    Ok(())
}

fn finalize_latest_pending_channel_bind_session_for_user(
    db: &mut Connection,
    channel: &str,
    user_key: &str,
    external_user_id: Option<&str>,
    external_chat_id: Option<&str>,
) -> anyhow::Result<()> {
    let channel = channel.trim();
    let user_key = normalize_user_key(user_key);
    let external_user_id = normalize_external_id_opt(external_user_id);
    let external_chat_id =
        normalize_external_id_opt(external_chat_id).or_else(|| external_user_id.clone());
    if channel.is_empty() || user_key.is_empty() {
        return Ok(());
    }
    let Some(external_user_id) = external_user_id.as_deref() else {
        return Ok(());
    };
    let Some(external_chat_id) = external_chat_id.as_deref() else {
        return Ok(());
    };

    let session = db
        .query_row(
            "SELECT id, channel, user_key, bind_token, status, external_user_id, external_chat_id, error_text,
                    install_device_code, install_verification_url, install_poll_interval_seconds,
                    created_at, updated_at, expires_at
             FROM pending_channel_bind_sessions
             WHERE channel = ?1
               AND user_key = ?2
               AND status IN (?3, ?4)
             ORDER BY id DESC
             LIMIT 1",
            params![
                channel,
                user_key,
                PENDING_CHANNEL_BIND_STATUS_PENDING,
                PENDING_CHANNEL_BIND_STATUS_DETECTED,
            ],
            map_pending_channel_bind_session,
        )
        .optional()?;
    let Some(session) = session else {
        return Ok(());
    };

    let session = if session.status == PENDING_CHANNEL_BIND_STATUS_DETECTED
        && session.external_user_id.as_deref() == Some(external_user_id)
        && session.external_chat_id.as_deref() == Some(external_chat_id)
    {
        session
    } else {
        mark_pending_channel_bind_session_detected(
            db,
            session.id,
            external_user_id,
            external_chat_id,
        )?
    };

    let _ = finalize_pending_channel_bind_session(db, session.id)?;
    Ok(())
}

pub(crate) fn create_pending_channel_bind_session(
    db: &mut Connection,
    channel: &str,
    user_key: &str,
    expires_at: &str,
) -> anyhow::Result<PendingChannelBindSession> {
    let channel = channel.trim();
    let user_key = normalize_user_key(user_key);
    let expires_at = expires_at.trim();
    if channel.is_empty() {
        anyhow::bail!("channel is required");
    }
    if user_key.is_empty() {
        anyhow::bail!("user_key is required");
    }
    if expires_at.is_empty() {
        anyhow::bail!("expires_at is required");
    }
    let bind_token = format!("pb-{}", uuid::Uuid::new_v4().simple());
    let now = now_ts();
    db.execute(
        "INSERT INTO pending_channel_bind_sessions (
            channel, user_key, bind_token, status, external_user_id, external_chat_id, error_text,
            install_device_code, install_verification_url, install_poll_interval_seconds,
            created_at, updated_at, expires_at
        ) VALUES (?1, ?2, ?3, ?4, NULL, NULL, NULL, NULL, NULL, NULL, ?5, ?5, ?6)",
        params![
            channel,
            user_key,
            bind_token,
            PENDING_CHANNEL_BIND_STATUS_PENDING,
            now,
            expires_at,
        ],
    )?;
    let session_id = db.last_insert_rowid();
    get_pending_channel_bind_session_by_id(db, session_id)?
        .ok_or_else(|| anyhow::anyhow!("created pending bind session not found"))
}

pub(crate) fn attach_pending_channel_bind_session_install_flow(
    db: &mut Connection,
    session_id: i64,
    device_code: &str,
    verification_url: &str,
    poll_interval_seconds: i64,
    expires_at: &str,
) -> anyhow::Result<PendingChannelBindSession> {
    let device_code = device_code.trim();
    let verification_url = verification_url.trim();
    let expires_at = expires_at.trim();
    if device_code.is_empty() {
        anyhow::bail!("device_code is required");
    }
    if verification_url.is_empty() {
        anyhow::bail!("verification_url is required");
    }
    if expires_at.is_empty() {
        anyhow::bail!("expires_at is required");
    }
    let now = now_ts();
    let changed = db.execute(
        "UPDATE pending_channel_bind_sessions
         SET install_device_code = ?2,
             install_verification_url = ?3,
             install_poll_interval_seconds = ?4,
             error_text = NULL,
             updated_at = ?5,
             expires_at = ?6
         WHERE id = ?1
           AND status IN (?7, ?8)",
        params![
            session_id,
            device_code,
            verification_url,
            poll_interval_seconds.max(1),
            now,
            expires_at,
            PENDING_CHANNEL_BIND_STATUS_PENDING,
            PENDING_CHANNEL_BIND_STATUS_DETECTED,
        ],
    )?;
    if changed == 0 {
        anyhow::bail!("pending bind session not found or already terminal");
    }
    get_pending_channel_bind_session_by_id(db, session_id)?
        .ok_or_else(|| anyhow::anyhow!("pending bind session not found after install flow update"))
}

pub(crate) fn get_pending_channel_bind_session_by_id(
    db: &Connection,
    session_id: i64,
) -> anyhow::Result<Option<PendingChannelBindSession>> {
    Ok(db
        .query_row(
            "SELECT id, channel, user_key, bind_token, status, external_user_id, external_chat_id, error_text,
                    install_device_code, install_verification_url, install_poll_interval_seconds,
                    created_at, updated_at, expires_at
             FROM pending_channel_bind_sessions
             WHERE id = ?1",
            params![session_id],
            map_pending_channel_bind_session,
        )
        .optional()?)
}

pub(crate) fn get_pending_channel_bind_session_by_token(
    db: &Connection,
    bind_token: &str,
) -> anyhow::Result<Option<PendingChannelBindSession>> {
    Ok(db
        .query_row(
            "SELECT id, channel, user_key, bind_token, status, external_user_id, external_chat_id, error_text,
                    install_device_code, install_verification_url, install_poll_interval_seconds,
                    created_at, updated_at, expires_at
             FROM pending_channel_bind_sessions
             WHERE bind_token = ?1",
            params![bind_token],
            map_pending_channel_bind_session,
        )
        .optional()?)
}

fn mark_pending_channel_bind_session_status(
    db: &mut Connection,
    session_id: i64,
    status: &str,
    error_text: Option<&str>,
) -> anyhow::Result<PendingChannelBindSession> {
    let now = now_ts();
    let changed = db.execute(
        "UPDATE pending_channel_bind_sessions
         SET status = ?2,
             error_text = ?3,
             updated_at = ?4
         WHERE id = ?1
           AND status IN (?5, ?6)",
        params![
            session_id,
            status,
            error_text,
            now,
            PENDING_CHANNEL_BIND_STATUS_PENDING,
            PENDING_CHANNEL_BIND_STATUS_DETECTED,
        ],
    )?;
    if changed == 0 {
        anyhow::bail!("pending bind session not found or already terminal");
    }
    get_pending_channel_bind_session_by_id(db, session_id)?
        .ok_or_else(|| anyhow::anyhow!("pending bind session not found after update"))
}

pub(crate) fn mark_pending_channel_bind_session_detected(
    db: &mut Connection,
    session_id: i64,
    external_user_id: &str,
    external_chat_id: &str,
) -> anyhow::Result<PendingChannelBindSession> {
    let external_user_id = normalize_external_id_opt(Some(external_user_id))
        .ok_or_else(|| anyhow::anyhow!("external_user_id is required"))?;
    let external_chat_id = normalize_external_id_opt(Some(external_chat_id))
        .or_else(|| Some(external_user_id.clone()))
        .ok_or_else(|| anyhow::anyhow!("external_chat_id is required"))?;
    let now = now_ts();
    let changed = db.execute(
        "UPDATE pending_channel_bind_sessions
         SET status = ?2,
             external_user_id = ?3,
             external_chat_id = ?4,
             error_text = NULL,
             updated_at = ?5
         WHERE id = ?1
           AND status IN (?6, ?7)",
        params![
            session_id,
            PENDING_CHANNEL_BIND_STATUS_DETECTED,
            external_user_id,
            external_chat_id,
            now,
            PENDING_CHANNEL_BIND_STATUS_PENDING,
            PENDING_CHANNEL_BIND_STATUS_DETECTED,
        ],
    )?;
    if changed == 0 {
        anyhow::bail!("pending bind session not found or already terminal");
    }
    get_pending_channel_bind_session_by_id(db, session_id)?
        .ok_or_else(|| anyhow::anyhow!("pending bind session not found after detection"))
}

pub(crate) fn mark_pending_channel_bind_session_failed(
    db: &mut Connection,
    session_id: i64,
    error_text: &str,
) -> anyhow::Result<PendingChannelBindSession> {
    mark_pending_channel_bind_session_status(
        db,
        session_id,
        PENDING_CHANNEL_BIND_STATUS_FAILED,
        Some(error_text),
    )
}

pub(crate) fn mark_pending_channel_bind_session_expired(
    db: &mut Connection,
    session_id: i64,
) -> anyhow::Result<PendingChannelBindSession> {
    mark_pending_channel_bind_session_status(
        db,
        session_id,
        PENDING_CHANNEL_BIND_STATUS_EXPIRED,
        Some("expired"),
    )
}

pub(crate) fn finalize_pending_channel_bind_session(
    db: &mut Connection,
    session_id: i64,
) -> anyhow::Result<PendingChannelBindSession> {
    let tx = db.transaction()?;
    let session = tx
        .query_row(
            "SELECT id, channel, user_key, bind_token, status, external_user_id, external_chat_id, error_text,
                    install_device_code, install_verification_url, install_poll_interval_seconds,
                    created_at, updated_at, expires_at
             FROM pending_channel_bind_sessions
             WHERE id = ?1",
            params![session_id],
            map_pending_channel_bind_session,
        )
        .optional()?;
    let Some(session) = session else {
        anyhow::bail!("pending bind session not found");
    };
    if matches!(
        session.status.as_str(),
        PENDING_CHANNEL_BIND_STATUS_FAILED | PENDING_CHANNEL_BIND_STATUS_EXPIRED
    ) {
        anyhow::bail!("pending bind session is already terminal");
    }
    if session.external_user_id.is_none() && session.external_chat_id.is_none() {
        anyhow::bail!("pending bind session does not have a detected external identity");
    }
    upsert_channel_binding_row(
        &tx,
        &session.channel,
        session.external_user_id.as_deref(),
        session.external_chat_id.as_deref(),
        &session.user_key,
    )?;
    tx.execute(
        "UPDATE pending_channel_bind_sessions
         SET status = ?2,
             error_text = NULL,
             updated_at = ?3
         WHERE id = ?1",
        params![session_id, PENDING_CHANNEL_BIND_STATUS_BOUND, now_ts()],
    )?;
    tx.commit()?;
    get_pending_channel_bind_session_by_id(db, session_id)?
        .ok_or_else(|| anyhow::anyhow!("pending bind session not found after finalize"))
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
pub(crate) fn update_auth_key_by_id(
    state: &AppState,
    key_id: i64,
    role: Option<&str>,
    enabled: Option<bool>,
    actor_user_key: &str,
) -> anyhow::Result<bool> {
    if role.is_none() && enabled.is_none() {
        return Err(anyhow::anyhow!("nothing to update"));
    }
    let normalized_role = match role {
        Some(value) => Some(normalize_auth_key_role(value)?),
        None => None,
    };
    let enabled_i64 = enabled.map(|v| if v { 1_i64 } else { 0_i64 });

    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let target = db.query_row(
        "SELECT user_key, role, enabled FROM auth_keys WHERE rowid = ?1",
        params![key_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        },
    );
    let (target_user_key, target_role, target_enabled) = match target {
        Ok(v) => v,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    let actor_user_key = normalize_user_key(actor_user_key);
    if !actor_user_key.is_empty() && target_user_key == actor_user_key {
        if enabled == Some(false) {
            return Err(anyhow::anyhow!("cannot disable current key"));
        }
        if target_role.eq_ignore_ascii_case("admin")
            && normalized_role
                .as_deref()
                .is_some_and(|value| !value.eq_ignore_ascii_case("admin"))
        {
            return Err(anyhow::anyhow!("cannot change current admin key role"));
        }
    }
    if target_role.eq_ignore_ascii_case("admin") {
        if normalized_role
            .as_deref()
            .is_some_and(|value| !value.eq_ignore_ascii_case("admin"))
        {
            return Err(anyhow::anyhow!("cannot change the admin key role"));
        }
        if enabled == Some(false) && target_enabled != 0 {
            return Err(anyhow::anyhow!("cannot disable the only admin key"));
        }
    }
    if normalized_role.as_deref() == Some("admin")
        && !target_role.eq_ignore_ascii_case("admin")
        && has_other_admin_key(&db, Some(key_id))?
    {
        return Err(anyhow::anyhow!("admin key already exists"));
    }

    let changed = db.execute(
        "UPDATE auth_keys
         SET role = COALESCE(?2, role),
             enabled = COALESCE(?3, enabled)
         WHERE rowid = ?1",
        params![key_id, normalized_role, enabled_i64],
    )?;
    Ok(changed > 0)
}

pub(crate) fn delete_auth_key_by_id(
    state: &AppState,
    key_id: i64,
    actor_user_key: &str,
) -> anyhow::Result<bool> {
    let mut db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let target = db.query_row(
        "SELECT user_key, role FROM auth_keys WHERE rowid = ?1",
        params![key_id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    );
    let (target_user_key, target_role) = match target {
        Ok(v) => v,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
        Err(err) => return Err(err.into()),
    };

    let actor_user_key = normalize_user_key(actor_user_key);
    if !actor_user_key.is_empty() && target_user_key == actor_user_key {
        return Err(anyhow::anyhow!("cannot delete current key"));
    }

    if target_role.eq_ignore_ascii_case("admin") {
        return Err(anyhow::anyhow!(
            "admin key cannot be deleted; rotate a new admin key instead"
        ));
    }

    let tx = db.transaction()?;
    tx.execute(
        "DELETE FROM channel_bindings WHERE user_key = ?1",
        params![target_user_key],
    )?;
    tx.execute(
        "DELETE FROM exchange_api_credentials WHERE user_key = ?1",
        params![target_user_key],
    )?;
    tx.execute(
        "DELETE FROM webd_login_accounts WHERE user_key = ?1",
        params![target_user_key],
    )?;
    let changed = tx.execute("DELETE FROM auth_keys WHERE rowid = ?1", params![key_id])?;
    tx.commit()?;
    Ok(changed > 0)
}

pub(crate) fn normalize_user_key(raw: &str) -> String {
    raw.trim().to_string()
}

pub(crate) fn exchange_credential_status_for_user_key(
    state: &AppState,
    user_key: &str,
) -> anyhow::Result<Vec<ExchangeCredentialStatus>> {
    let user_key = normalize_user_key(user_key);
    if user_key.is_empty() {
        return Ok(Vec::new());
    }
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let mut out = Vec::new();
    for exchange in ["binance", "okx"] {
        let row = db
            .query_row(
                "SELECT api_key, updated_at, enabled
                 FROM exchange_api_credentials
                 WHERE user_key = ?1 AND exchange = ?2
                 LIMIT 1",
                params![user_key, exchange],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        let (configured, api_key_masked, updated_at) = match row {
            Some((api_key, updated_at, enabled)) if enabled == 1 => {
                (true, Some(api_key), Some(updated_at))
            }
            _ => (false, None, None),
        };
        out.push(ExchangeCredentialStatus {
            exchange: exchange.to_string(),
            configured,
            api_key_masked,
            updated_at,
        });
    }
    Ok(out)
}

pub(crate) fn upsert_exchange_credential_for_user_key(
    state: &AppState,
    user_key: &str,
    exchange_raw: &str,
    api_key: &str,
    api_secret: &str,
    passphrase: Option<&str>,
) -> anyhow::Result<ExchangeCredentialStatus> {
    let user_key = normalize_user_key(user_key);
    if user_key.is_empty() {
        return Err(anyhow::anyhow!("user_key is required"));
    }
    let exchange = crate::normalize_exchange_name(exchange_raw)?;
    let api_key = api_key.trim();
    let api_secret = api_secret.trim();
    if api_key.is_empty() || api_secret.is_empty() {
        return Err(anyhow::anyhow!("api_key and api_secret are required"));
    }
    let passphrase = passphrase.map(str::trim).filter(|v| !v.is_empty());
    let now = now_ts();
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    db.execute(
        "INSERT INTO exchange_api_credentials (user_key, exchange, api_key, api_secret, passphrase, enabled, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6)
         ON CONFLICT(user_key, exchange)
         DO UPDATE SET api_key=excluded.api_key, api_secret=excluded.api_secret, passphrase=excluded.passphrase, enabled=1, updated_at=excluded.updated_at",
        params![user_key, exchange, api_key, api_secret, passphrase, now],
    )?;
    Ok(ExchangeCredentialStatus {
        exchange,
        configured: true,
        api_key_masked: Some(api_key.to_string()),
        updated_at: Some(now),
    })
}

fn build_auth_identity(
    user_key: &str,
    role: &str,
    channel: &str,
    external_user_id: Option<&str>,
    external_chat_id: Option<&str>,
) -> AuthIdentity {
    let user_id = crate::stable_i64_from_key(user_key);
    AuthIdentity {
        user_key: user_key.to_string(),
        role: role.to_string(),
        user_id,
        chat_id: crate::build_conversation_chat_id(
            channel,
            external_user_id,
            external_chat_id,
            user_key,
        ),
    }
}

pub(crate) fn resolve_auth_identity_by_key(
    state: &AppState,
    raw_user_key: &str,
) -> anyhow::Result<Option<AuthIdentity>> {
    let user_key = normalize_user_key(raw_user_key);
    if user_key.is_empty() {
        return Ok(None);
    }
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let row = db
        .query_row(
            "SELECT role FROM auth_keys WHERE user_key = ?1 AND enabled = 1 LIMIT 1",
            params![user_key],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(row.map(|role| build_auth_identity(&user_key, &role, "ui", None, Some("console"))))
}

fn touch_auth_key_usage(db: &Connection, user_key: &str) -> anyhow::Result<()> {
    db.execute(
        "UPDATE auth_keys SET last_used_at = ?2 WHERE user_key = ?1",
        params![user_key, now_ts()],
    )?;
    Ok(())
}

pub(crate) fn resolve_channel_binding_identity(
    state: &AppState,
    channel: &str,
    external_user_id: Option<&str>,
    external_chat_id: Option<&str>,
) -> anyhow::Result<Option<AuthIdentity>> {
    let external_user_id = normalize_external_id_opt(external_user_id);
    let external_chat_id =
        normalize_external_id_opt(external_chat_id).or_else(|| external_user_id.clone());
    if external_user_id.is_none() && external_chat_id.is_none() {
        return Ok(None);
    }
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let row = if external_user_id.is_some() && external_chat_id.is_some() {
        db.query_row(
            "SELECT k.user_key, k.role
             FROM channel_bindings b
             JOIN auth_keys k ON k.user_key = b.user_key
             WHERE b.channel = ?1
               AND k.enabled = 1
               AND b.external_user_id = ?2
               AND b.external_chat_id = ?3
             ORDER BY b.id DESC
             LIMIT 1",
            params![channel, external_user_id, external_chat_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    } else if external_chat_id.is_some() {
        db.query_row(
            "SELECT k.user_key, k.role
             FROM channel_bindings b
             JOIN auth_keys k ON k.user_key = b.user_key
             WHERE b.channel = ?1
               AND k.enabled = 1
               AND b.external_chat_id = ?2
             ORDER BY b.id DESC
             LIMIT 1",
            params![channel, external_chat_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    } else {
        db.query_row(
            "SELECT k.user_key, k.role
             FROM channel_bindings b
             JOIN auth_keys k ON k.user_key = b.user_key
             WHERE b.channel = ?1
               AND k.enabled = 1
               AND b.external_user_id = ?2
             ORDER BY b.id DESC
             LIMIT 1",
            params![channel, external_user_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    };
    if let Some((user_key, role)) = row {
        touch_auth_key_usage(&db, &user_key)?;
        return Ok(Some(build_auth_identity(
            &user_key,
            &role,
            channel,
            external_user_id.as_deref(),
            external_chat_id.as_deref(),
        )));
    }
    Ok(None)
}

pub(crate) fn has_channel_binding_for_user_key(
    state: &AppState,
    channel: &str,
    raw_user_key: &str,
) -> anyhow::Result<bool> {
    let channel = channel.trim();
    let user_key = normalize_user_key(raw_user_key);
    if channel.is_empty() || user_key.is_empty() {
        return Ok(false);
    }
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let count: i64 = db.query_row(
        "SELECT COUNT(*)
         FROM channel_bindings
         WHERE channel = ?1
           AND user_key = ?2",
        params![channel, user_key],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

pub(crate) fn reset_channel_binding_state_for_user_key(
    state: &AppState,
    channel: &str,
    raw_user_key: &str,
) -> anyhow::Result<()> {
    let channel = channel.trim();
    let user_key = normalize_user_key(raw_user_key);
    if channel.is_empty() || user_key.is_empty() {
        return Ok(());
    }
    let mut db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let tx = db.transaction()?;
    tx.execute(
        "DELETE FROM channel_bindings
         WHERE channel = ?1
           AND user_key = ?2",
        params![channel, user_key],
    )?;
    tx.execute(
        "DELETE FROM pending_channel_bind_sessions
         WHERE channel = ?1
           AND user_key = ?2",
        params![channel, user_key],
    )?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn bind_channel_identity(
    state: &AppState,
    channel: &str,
    external_user_id: Option<&str>,
    external_chat_id: Option<&str>,
    raw_user_key: &str,
) -> anyhow::Result<Option<AuthIdentity>> {
    let Some(identity) = resolve_auth_identity_by_key(state, raw_user_key)? else {
        return Ok(None);
    };
    let external_user_id = normalize_external_id_opt(external_user_id);
    let external_chat_id =
        normalize_external_id_opt(external_chat_id).or_else(|| external_user_id.clone());
    if external_user_id.is_none() && external_chat_id.is_none() {
        return Ok(None);
    }
    let mut db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    upsert_channel_binding_row(
        &db,
        channel,
        external_user_id.as_deref(),
        external_chat_id.as_deref(),
        &identity.user_key,
    )?;
    finalize_latest_pending_channel_bind_session_for_user(
        &mut db,
        channel,
        &identity.user_key,
        external_user_id.as_deref(),
        external_chat_id.as_deref(),
    )?;
    touch_auth_key_usage(&db, &identity.user_key)?;
    Ok(Some(build_auth_identity(
        &identity.user_key,
        &identity.role,
        channel,
        external_user_id.as_deref(),
        external_chat_id.as_deref(),
    )))
}
