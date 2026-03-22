use claw_core::config::{AppConfig, ChannelBindingConfig};
use claw_core::types::{AuthIdentity, ExchangeCredentialStatus};
use rusqlite::{params, Connection, OptionalExtension};
use tracing::info;

use crate::{mask_secret, normalize_external_id_opt, now_ts, AppState};

fn generate_user_key() -> String {
    format!("rk-{}", uuid::Uuid::new_v4().simple())
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
    crate::ensure_column_exists(
        db,
        "audit_logs",
        "user_key",
        "ALTER TABLE audit_logs ADD COLUMN user_key TEXT",
    )?;
    crate::ensure_column_exists(
        db,
        "user_preferences",
        "user_key",
        "ALTER TABLE user_preferences ADD COLUMN user_key TEXT",
    )?;
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

pub(crate) fn list_auth_keys(
    state: &AppState,
) -> anyhow::Result<Vec<(i64, String, String, i64, String, Option<String>)>> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    let mut stmt = db.prepare(
        "SELECT rowid, user_key, role, enabled, created_at, last_used_at FROM auth_keys ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (key_id, user_key, role, enabled, created_at, last_used_at) = row?;
        out.push((
            key_id,
            mask_secret(&user_key),
            role,
            enabled,
            created_at,
            last_used_at,
        ));
    }
    Ok(out)
}

pub(crate) fn create_auth_key(state: &AppState, role: &str) -> anyhow::Result<String> {
    let role = match role {
        "admin" => "admin",
        _ => "user",
    };
    let user_key = generate_user_key();
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES (?1, ?2, 1, ?3, NULL)",
        params![user_key, role, now_ts()],
    )?;
    Ok(user_key)
}

#[cfg(test)]
mod tests {
    use super::rebuild_channel_tables_for_ui;
    use rusqlite::Connection;

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
        assert!(sql.contains("'wechat'"), "tasks schema should allow wechat: {sql}");

        db.execute(
            "INSERT INTO tasks (task_id, user_id, chat_id, channel, kind, payload_json, status, created_at, updated_at)
             VALUES ('t1', 1, 1, 'wechat', 'ask', '{}', 'queued', '1', '1')",
            [],
        )
        .expect("insert wechat task");
    }
}

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
    let normalized_role = role.map(|v| {
        if v.eq_ignore_ascii_case("admin") {
            "admin"
        } else {
            "user"
        }
    });
    let enabled_i64 = enabled.map(|v| if v { 1_i64 } else { 0_i64 });

    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    let target = db.query_row(
        "SELECT user_key FROM auth_keys WHERE rowid = ?1",
        params![key_id],
        |row| row.get::<_, String>(0),
    );
    let target_user_key = match target {
        Ok(v) => v,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    let actor_user_key = normalize_user_key(actor_user_key);
    if !actor_user_key.is_empty() && target_user_key == actor_user_key {
        if enabled == Some(false) {
            return Err(anyhow::anyhow!("cannot disable current key"));
        }
        if normalized_role == Some("user") {
            return Err(anyhow::anyhow!("cannot demote current admin key"));
        }
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
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
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
        let admin_count: i64 = db.query_row(
            "SELECT COUNT(*) FROM auth_keys WHERE role = 'admin' AND enabled = 1",
            [],
            |row| row.get(0),
        )?;
        if admin_count <= 1 {
            return Err(anyhow::anyhow!("cannot delete the last enabled admin key"));
        }
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
    let changed = tx.execute("DELETE FROM auth_keys WHERE rowid = ?1", params![key_id])?;
    tx.commit()?;
    Ok(changed > 0)
}

fn seed_channel_binding_rows(
    db: &Connection,
    channel: &str,
    bindings: &[ChannelBindingConfig],
) -> anyhow::Result<()> {
    let now = now_ts();
    for binding in bindings {
        let user_key = normalize_user_key(&binding.user_key);
        if user_key.is_empty() {
            continue;
        }
        let external_user_id = normalize_external_id_opt(Some(&binding.external_user_id));
        let external_chat_id = normalize_external_id_opt(Some(&binding.external_chat_id))
            .or_else(|| external_user_id.clone());
        if external_user_id.is_none() && external_chat_id.is_none() {
            continue;
        }
        db.execute(
            "INSERT INTO channel_bindings (channel, external_user_id, external_chat_id, user_key, bound_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(channel, external_user_id, external_chat_id)
             DO UPDATE SET user_key=excluded.user_key, updated_at=excluded.updated_at",
            params![channel, external_user_id, external_chat_id, user_key, now],
        )?;
    }
    Ok(())
}

pub(crate) fn seed_channel_bindings(db: &Connection, config: &AppConfig) -> anyhow::Result<()> {
    seed_channel_binding_rows(db, "telegram", &config.telegram.bindings)?;
    seed_channel_binding_rows(db, "whatsapp", &config.whatsapp.bindings)?;
    seed_channel_binding_rows(db, "whatsapp", &config.whatsapp_cloud.bindings)?;
    seed_channel_binding_rows(db, "whatsapp", &config.whatsapp_web.bindings)?;
    Ok(())
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
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
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
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
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
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
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
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
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
    let now = now_ts();
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    db.execute(
        "INSERT INTO channel_bindings (channel, external_user_id, external_chat_id, user_key, bound_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT(channel, external_user_id, external_chat_id)
         DO UPDATE SET user_key=excluded.user_key, updated_at=excluded.updated_at",
        params![
            channel,
            external_user_id,
            external_chat_id,
            &identity.user_key,
            now
        ],
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
