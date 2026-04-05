use std::path::Path;
use std::time::Duration;

use claw_core::config::AppConfig;
use rusqlite::{params, Connection};
use tracing::debug;

pub(crate) fn init_db(config: &AppConfig) -> anyhow::Result<Connection> {
    if let Some(parent) = Path::new(&config.database.sqlite_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let db = Connection::open(&config.database.sqlite_path)?;
    db.busy_timeout(Duration::from_millis(config.database.busy_timeout_ms))?;
    db.execute_batch(crate::INIT_SQL)?;
    Ok(db)
}

pub(crate) fn seed_users(db: &Connection, config: &AppConfig) -> anyhow::Result<()> {
    let now = crate::now_ts();

    for user_id in &config.telegram.allowlist {
        db.execute(
            "INSERT INTO users (user_id, role, is_allowed, created_at, last_seen)
             VALUES (?1, 'user', 1, ?2, ?2)
             ON CONFLICT(user_id) DO UPDATE SET is_allowed=1, last_seen=excluded.last_seen",
            params![user_id, now],
        )?;
    }

    Ok(())
}

pub(crate) fn ensure_schedule_schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS scheduled_jobs (
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id            TEXT NOT NULL UNIQUE,
            user_id           INTEGER NOT NULL,
            chat_id           INTEGER NOT NULL,
            channel           TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark', 'wechat')),
            external_user_id  TEXT,
            external_chat_id  TEXT,
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
        CREATE INDEX IF NOT EXISTS idx_scheduled_jobs_due ON scheduled_jobs(enabled, next_run_at);
        CREATE INDEX IF NOT EXISTS idx_scheduled_jobs_user_chat ON scheduled_jobs(user_id, chat_id);",
    )?;
    Ok(())
}

pub(crate) fn ensure_memory_schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(crate::MEMORY_UPGRADE_SQL)?;
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memories_user_chat_role_id
         ON memories(user_id, chat_id, role, id DESC);",
    )?;
    crate::ensure_column_exists(
        db,
        "memories",
        "memory_type",
        "ALTER TABLE memories ADD COLUMN memory_type TEXT NOT NULL DEFAULT 'generic'",
    )?;
    crate::ensure_column_exists(
        db,
        "memories",
        "salience",
        "ALTER TABLE memories ADD COLUMN salience REAL NOT NULL DEFAULT 0.5",
    )?;
    crate::ensure_column_exists(
        db,
        "memories",
        "created_at_ts",
        "ALTER TABLE memories ADD COLUMN created_at_ts INTEGER NOT NULL DEFAULT 0",
    )?;
    crate::ensure_column_exists(
        db,
        "user_preferences",
        "updated_at_ts",
        "ALTER TABLE user_preferences ADD COLUMN updated_at_ts INTEGER NOT NULL DEFAULT 0",
    )?;
    crate::ensure_column_exists(
        db,
        "long_term_memories",
        "created_at_ts",
        "ALTER TABLE long_term_memories ADD COLUMN created_at_ts INTEGER NOT NULL DEFAULT 0",
    )?;
    crate::ensure_column_exists(
        db,
        "long_term_memories",
        "updated_at_ts",
        "ALTER TABLE long_term_memories ADD COLUMN updated_at_ts INTEGER NOT NULL DEFAULT 0",
    )?;
    db.execute(
        "UPDATE memories
         SET created_at_ts = CAST(created_at AS INTEGER)
         WHERE created_at_ts = 0 AND created_at GLOB '[0-9]*'",
        [],
    )?;
    db.execute(
        "UPDATE user_preferences
         SET updated_at_ts = CAST(updated_at AS INTEGER)
         WHERE updated_at_ts = 0 AND updated_at GLOB '[0-9]*'",
        [],
    )?;
    db.execute(
        "UPDATE long_term_memories
         SET created_at_ts = CAST(created_at AS INTEGER)
         WHERE created_at_ts = 0 AND created_at GLOB '[0-9]*'",
        [],
    )?;
    db.execute(
        "UPDATE long_term_memories
         SET updated_at_ts = CAST(updated_at AS INTEGER)
         WHERE updated_at_ts = 0 AND updated_at GLOB '[0-9]*'",
        [],
    )?;
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memories_user_chat_created_at_ts
         ON memories(user_id, chat_id, created_at_ts);
         CREATE INDEX IF NOT EXISTS idx_user_preferences_user_chat_updated_ts
         ON user_preferences(user_id, chat_id, updated_at_ts);
         CREATE INDEX IF NOT EXISTS idx_long_term_memories_updated_at_ts
         ON long_term_memories(updated_at_ts);",
    )?;
    crate::ensure_column_exists(
        db,
        "memories",
        "is_instructional",
        "ALTER TABLE memories ADD COLUMN is_instructional INTEGER NOT NULL DEFAULT 0",
    )?;
    crate::ensure_column_exists(
        db,
        "memories",
        "safety_flag",
        "ALTER TABLE memories ADD COLUMN safety_flag TEXT NOT NULL DEFAULT 'normal'",
    )?;
    Ok(())
}

pub(crate) fn ensure_channel_schema(db: &Connection) -> anyhow::Result<()> {
    if let Err(err) = db.execute_batch(crate::CHANNEL_UPGRADE_SQL) {
        debug!("channel schema batch skipped: {}", err);
    }
    crate::ensure_column_exists(
        db,
        "tasks",
        "channel",
        "ALTER TABLE tasks ADD COLUMN channel TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark', 'wechat'))",
    )?;
    crate::ensure_column_exists(
        db,
        "tasks",
        "external_user_id",
        "ALTER TABLE tasks ADD COLUMN external_user_id TEXT",
    )?;
    crate::ensure_column_exists(
        db,
        "tasks",
        "external_chat_id",
        "ALTER TABLE tasks ADD COLUMN external_chat_id TEXT",
    )?;

    crate::ensure_column_exists(
        db,
        "scheduled_jobs",
        "channel",
        "ALTER TABLE scheduled_jobs ADD COLUMN channel TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark', 'wechat'))",
    )?;
    crate::ensure_column_exists(
        db,
        "scheduled_jobs",
        "external_user_id",
        "ALTER TABLE scheduled_jobs ADD COLUMN external_user_id TEXT",
    )?;
    crate::ensure_column_exists(
        db,
        "scheduled_jobs",
        "external_chat_id",
        "ALTER TABLE scheduled_jobs ADD COLUMN external_chat_id TEXT",
    )?;

    crate::ensure_column_exists(
        db,
        "memories",
        "channel",
        "ALTER TABLE memories ADD COLUMN channel TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark', 'wechat'))",
    )?;
    crate::ensure_column_exists(
        db,
        "memories",
        "external_chat_id",
        "ALTER TABLE memories ADD COLUMN external_chat_id TEXT",
    )?;
    Ok(())
}
