use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use tracing::info;

pub(crate) fn ensure_key_auth_schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(crate::KEY_AUTH_UPGRADE_SQL)?;
    db.execute_batch(crate::WEBD_LOGIN_SQL)?;
    crate::repo::approval_scope::ensure_approval_scope_grant_schema(db)?;
    db.execute_batch(include_str!(
        "../../../../../migrations/006_pending_channel_bind_sessions.sql"
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
    crate::ensure_column_exists(
        db,
        "tasks",
        "user_key",
        "ALTER TABLE tasks ADD COLUMN user_key TEXT",
    )?;
    crate::ensure_column_exists(
        db,
        "tasks",
        "idempotency_key",
        "ALTER TABLE tasks ADD COLUMN idempotency_key TEXT",
    )?;
    db.execute_batch(include_str!(
        "../../../../../migrations/007_pending_channel_requests.sql"
    ))?;
    crate::ensure_column_exists(
        db,
        "scheduled_jobs",
        "user_key",
        "ALTER TABLE scheduled_jobs ADD COLUMN user_key TEXT",
    )?;
    crate::ensure_column_exists(
        db,
        "scheduled_jobs",
        "isolation_profile",
        "ALTER TABLE scheduled_jobs ADD COLUMN isolation_profile TEXT NOT NULL DEFAULT 'local_current_workspace'",
    )?;
    crate::ensure_column_exists(
        db,
        "scheduled_jobs",
        "permission_policy_json",
        "ALTER TABLE scheduled_jobs ADD COLUMN permission_policy_json TEXT NOT NULL DEFAULT '{}'",
    )?;
    crate::ensure_column_exists(
        db,
        "scheduled_jobs",
        "thread_resume_enabled",
        "ALTER TABLE scheduled_jobs ADD COLUMN thread_resume_enabled INTEGER NOT NULL DEFAULT 1",
    )?;
    crate::ensure_column_exists(
        db,
        "scheduled_jobs",
        "last_thread_task_id",
        "ALTER TABLE scheduled_jobs ADD COLUMN last_thread_task_id TEXT",
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
    // Keep old main-db audit tables readable for one-time migration into audit_db.
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
    rebuild_task_message_id_as_text(db)?;
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_tasks_user_key_created_at ON tasks(user_key, created_at);
         CREATE INDEX IF NOT EXISTS idx_memories_user_key_chat_created_at ON memories(user_key, chat_id, created_at_ts);
         CREATE INDEX IF NOT EXISTS idx_scheduled_jobs_user_key_chat ON scheduled_jobs(user_key, chat_id);
         CREATE INDEX IF NOT EXISTS idx_user_preferences_user_key_chat ON user_preferences(user_key, chat_id, updated_at_ts);
         CREATE INDEX IF NOT EXISTS idx_long_term_memories_user_key_chat_updated_ts ON long_term_memories(user_key, chat_id, updated_at_ts);",
    )?;
    super::auth_principal::ensure_principal_identity_schema(db)?;
    Ok(())
}

pub(super) fn rebuild_task_message_id_as_text(db: &Connection) -> anyhow::Result<()> {
    // This migration is also exercised directly by upgrade verification tests and may
    // encounter a database created before task submission idempotency was introduced.
    // Materialize the nullable column before rebuilding so the copy remains valid for
    // both old and current task schemas.
    crate::ensure_column_exists(
        db,
        "tasks",
        "idempotency_key",
        "ALTER TABLE tasks ADD COLUMN idempotency_key TEXT",
    )?;
    let column_type = db
        .query_row(
            "SELECT type FROM pragma_table_info('tasks') WHERE name = 'message_id'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(column_type) = column_type else {
        crate::ensure_column_exists(
            db,
            "tasks",
            "message_id",
            "ALTER TABLE tasks ADD COLUMN message_id TEXT",
        )?;
        return Ok(());
    };
    if column_type.trim().eq_ignore_ascii_case("TEXT") {
        return Ok(());
    }

    info!("task schema: migrating message_id affinity to TEXT");
    let foreign_keys_enabled =
        db.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))?;
    if foreign_keys_enabled != 0 {
        db.execute_batch("PRAGMA foreign_keys = OFF;")?;
    }

    let migration_result = (|| -> anyhow::Result<()> {
        let tx = db.unchecked_transaction()?;
        let schema_objects = task_schema_objects(&tx)?;
        let before = task_rows_fingerprint(&tx, "tasks")?;
        tx.execute_batch(
            "CREATE TABLE tasks_message_id_text_migration (
                 task_id       TEXT PRIMARY KEY,
                 user_id       INTEGER NOT NULL,
                 chat_id       INTEGER NOT NULL,
                 channel       TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark', 'wechat')),
                 external_user_id TEXT,
                 external_chat_id TEXT,
                 message_id    TEXT,
                 idempotency_key TEXT,
                 user_key      TEXT,
                 kind          TEXT NOT NULL CHECK (kind IN ('ask', 'run_skill', 'admin')),
                 payload_json  TEXT NOT NULL,
                 status        TEXT NOT NULL CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'canceled', 'timeout')),
                 result_json   TEXT,
                 error_text    TEXT,
                 created_at    TEXT NOT NULL,
                 updated_at    TEXT NOT NULL,
                 lease_owner   TEXT,
                 lease_expires_at INTEGER NOT NULL DEFAULT 0,
                 claim_attempt INTEGER NOT NULL DEFAULT 0,
                 claimed_at    INTEGER NOT NULL DEFAULT 0
             );
             INSERT INTO tasks_message_id_text_migration (
                 task_id, user_id, chat_id, channel, external_user_id, external_chat_id,
                 message_id, idempotency_key, user_key, kind, payload_json, status, result_json, error_text,
                 created_at, updated_at, lease_owner, lease_expires_at, claim_attempt, claimed_at
             )
             SELECT
                 task_id, user_id, chat_id, channel, external_user_id, external_chat_id,
                 CAST(message_id AS TEXT), idempotency_key, user_key, kind, payload_json, status, result_json,
                 error_text, created_at, updated_at, lease_owner, lease_expires_at,
                 claim_attempt, claimed_at
             FROM tasks;",
        )?;
        let after = task_rows_fingerprint(&tx, "tasks_message_id_text_migration")?;
        anyhow::ensure!(
            before == after,
            "task message_id migration verification failed"
        );
        tx.execute_batch(
            "DROP TABLE tasks;
             ALTER TABLE tasks_message_id_text_migration RENAME TO tasks;",
        )?;
        for sql in schema_objects {
            tx.execute_batch(&sql)?;
        }
        tx.commit()?;
        Ok(())
    })();

    let restore_result = if foreign_keys_enabled != 0 {
        db.execute_batch("PRAGMA foreign_keys = ON;")
    } else {
        Ok(())
    };
    migration_result?;
    restore_result?;
    let violation = db
        .query_row("PRAGMA foreign_key_check", [], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .optional()?;
    anyhow::ensure!(
        violation.is_none(),
        "task message_id migration foreign key check failed"
    );
    Ok(())
}

fn task_schema_objects(db: &Connection) -> anyhow::Result<Vec<String>> {
    let mut stmt = db.prepare(
        "SELECT sql
         FROM sqlite_master
         WHERE tbl_name = 'tasks'
           AND type IN ('index', 'trigger')
           AND sql IS NOT NULL
         ORDER BY type, name",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn task_rows_fingerprint(db: &Connection, table: &str) -> anyhow::Result<(u64, String)> {
    let table = match table {
        "tasks" => "tasks",
        "tasks_message_id_text_migration" => "tasks_message_id_text_migration",
        _ => anyhow::bail!("unsupported task fingerprint table"),
    };
    let sql = format!(
        "SELECT task_id, user_id, chat_id, channel, external_user_id, external_chat_id,
                CAST(message_id AS TEXT), user_key, kind, payload_json, status, result_json,
                error_text, created_at, updated_at, lease_owner, lease_expires_at,
                claim_attempt, claimed_at
         FROM {table}
         ORDER BY task_id"
    );
    let mut stmt = db.prepare(&sql)?;
    let mut rows = stmt.query([])?;
    let mut count = 0_u64;
    let mut digest = Sha256::new();
    while let Some(row) = rows.next()? {
        count += 1;
        for index in 0..19 {
            hash_sql_value(&mut digest, row.get::<_, SqlValue>(index)?);
        }
    }
    Ok((count, format!("{:x}", digest.finalize())))
}

fn hash_sql_value(digest: &mut Sha256, value: SqlValue) {
    match value {
        SqlValue::Null => digest.update([0]),
        SqlValue::Integer(value) => {
            digest.update([1]);
            digest.update(value.to_le_bytes());
        }
        SqlValue::Real(value) => {
            digest.update([2]);
            digest.update(value.to_bits().to_le_bytes());
        }
        SqlValue::Text(value) => {
            digest.update([3]);
            digest.update((value.len() as u64).to_le_bytes());
            digest.update(value.as_bytes());
        }
        SqlValue::Blob(value) => {
            digest.update([4]);
            digest.update((value.len() as u64).to_le_bytes());
            digest.update(value);
        }
    }
}

pub(super) fn rebuild_auth_keys_for_flexible_roles(db: &Connection) -> anyhow::Result<()> {
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

pub(super) fn rebuild_channel_tables_for_ui(db: &Connection) -> anyhow::Result<()> {
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
    crate::ensure_column_exists(
        db,
        "tasks",
        "idempotency_key",
        "ALTER TABLE tasks ADD COLUMN idempotency_key TEXT",
    )?;
    crate::ensure_column_exists(
        db,
        "tasks",
        "lease_owner",
        "ALTER TABLE tasks ADD COLUMN lease_owner TEXT",
    )?;
    crate::ensure_column_exists(
        db,
        "tasks",
        "lease_expires_at",
        "ALTER TABLE tasks ADD COLUMN lease_expires_at INTEGER NOT NULL DEFAULT 0",
    )?;
    crate::ensure_column_exists(
        db,
        "tasks",
        "claim_attempt",
        "ALTER TABLE tasks ADD COLUMN claim_attempt INTEGER NOT NULL DEFAULT 0",
    )?;
    crate::ensure_column_exists(
        db,
        "tasks",
        "claimed_at",
        "ALTER TABLE tasks ADD COLUMN claimed_at INTEGER NOT NULL DEFAULT 0",
    )?;
    crate::ensure_column_exists(
        db,
        "scheduled_jobs",
        "isolation_profile",
        "ALTER TABLE scheduled_jobs ADD COLUMN isolation_profile TEXT NOT NULL DEFAULT 'local_current_workspace'",
    )?;
    crate::ensure_column_exists(
        db,
        "scheduled_jobs",
        "permission_policy_json",
        "ALTER TABLE scheduled_jobs ADD COLUMN permission_policy_json TEXT NOT NULL DEFAULT '{}'",
    )?;
    crate::ensure_column_exists(
        db,
        "scheduled_jobs",
        "thread_resume_enabled",
        "ALTER TABLE scheduled_jobs ADD COLUMN thread_resume_enabled INTEGER NOT NULL DEFAULT 1",
    )?;
    crate::ensure_column_exists(
        db,
        "scheduled_jobs",
        "last_thread_task_id",
        "ALTER TABLE scheduled_jobs ADD COLUMN last_thread_task_id TEXT",
    )?;
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
             message_id    TEXT,
             idempotency_key TEXT,
             user_key      TEXT,
             kind          TEXT NOT NULL CHECK (kind IN ('ask', 'run_skill', 'admin')),
             payload_json  TEXT NOT NULL,
             status        TEXT NOT NULL CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'canceled', 'timeout')),
             result_json   TEXT,
             error_text    TEXT,
             created_at    TEXT NOT NULL,
             updated_at    TEXT NOT NULL,
             lease_owner   TEXT,
             lease_expires_at INTEGER NOT NULL DEFAULT 0,
             claim_attempt INTEGER NOT NULL DEFAULT 0,
             claimed_at    INTEGER NOT NULL DEFAULT 0
         );
         INSERT INTO tasks (
             task_id, user_id, chat_id, channel, external_user_id, external_chat_id, message_id,
             idempotency_key, user_key, kind, payload_json, status, result_json, error_text,
             created_at, updated_at, lease_owner, lease_expires_at, claim_attempt, claimed_at
         )
         SELECT
             task_id, user_id, chat_id, channel, external_user_id, external_chat_id, message_id,
             idempotency_key, user_key, kind, payload_json, status, result_json, error_text,
             created_at, updated_at, lease_owner, lease_expires_at, claim_attempt, claimed_at
         FROM tasks_old;
         DROP TABLE tasks_old;
         CREATE INDEX IF NOT EXISTS idx_tasks_status_created_at ON tasks(status, created_at);
         CREATE INDEX IF NOT EXISTS idx_tasks_user_id_created_at ON tasks(user_id, created_at);
         CREATE INDEX IF NOT EXISTS idx_tasks_user_key_created_at ON tasks(user_key, created_at);
         CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_idempotency_key
             ON tasks(idempotency_key) WHERE idempotency_key IS NOT NULL;
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
             isolation_profile TEXT NOT NULL DEFAULT 'local_current_workspace',
             permission_policy_json TEXT NOT NULL DEFAULT '{}',
             thread_resume_enabled INTEGER NOT NULL DEFAULT 1,
             last_thread_task_id TEXT,
             created_at        TEXT NOT NULL,
             updated_at        TEXT NOT NULL
         );
         INSERT INTO scheduled_jobs (
             id, job_id, user_id, chat_id, channel, external_user_id, external_chat_id, user_key,
             schedule_type, run_at, time_of_day, weekday, every_minutes, cron_expr, timezone,
             task_kind, task_payload_json, enabled, notify_on_success, notify_on_failure,
             last_run_at, next_run_at, isolation_profile, permission_policy_json,
             thread_resume_enabled, last_thread_task_id, created_at, updated_at
         )
         SELECT
             id, job_id, user_id, chat_id, channel, external_user_id, external_chat_id, user_key,
             schedule_type, run_at, time_of_day, weekday, every_minutes, cron_expr, timezone,
             task_kind, task_payload_json, enabled, notify_on_success, notify_on_failure,
             last_run_at, next_run_at, isolation_profile, permission_policy_json,
             thread_resume_enabled, last_thread_task_id, created_at, updated_at
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
