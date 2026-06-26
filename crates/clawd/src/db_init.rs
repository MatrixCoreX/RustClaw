use std::path::Path;
use std::time::Duration;

use claw_core::config::AppConfig;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection};
use tracing::debug;

pub(crate) type DbPool = Pool<SqliteConnectionManager>;

/// Phase 2.2 Stage 2: audit_logs 走独立 SQLite 文件 + 独立连接池，
/// 与任务/调度/记忆主库隔离，避免 audit append 抢主库的 WAL writer 锁。
pub(crate) const INIT_AUDIT_SQL: &str = "\
CREATE TABLE IF NOT EXISTS audit_logs (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    ts           TEXT NOT NULL,
    user_id      INTEGER,
    action       TEXT NOT NULL,
    detail_json  TEXT,
    error_text   TEXT,
    user_key     TEXT
);
CREATE INDEX IF NOT EXISTS idx_audit_logs_ts ON audit_logs(ts);
CREATE INDEX IF NOT EXISTS idx_audit_logs_user_key_ts ON audit_logs(user_key, ts);
";

/// 给单元测试 fixture 用的轻量 pool：内存数据库，单连接。
/// 替代了原来 fixture 里的 `Arc::new(Mutex::new(Connection::open_in_memory()))`。
#[cfg(test)]
pub(crate) fn test_pool() -> DbPool {
    let manager = SqliteConnectionManager::memory();
    Pool::builder()
        .max_size(1)
        .build(manager)
        .expect("build test db pool")
}

/// 给单元测试 fixture 用的 audit pool：内存数据库，预先建好 schema。
#[cfg(test)]
pub(crate) fn test_audit_pool() -> DbPool {
    let manager = SqliteConnectionManager::memory();
    let pool = Pool::builder()
        .max_size(1)
        .build(manager)
        .expect("build test audit pool");
    let conn = pool.get().expect("get audit conn");
    conn.execute_batch(INIT_AUDIT_SQL)
        .expect("init audit schema");
    pool
}

pub(crate) fn init_db(config: &AppConfig) -> anyhow::Result<DbPool> {
    if let Some(parent) = Path::new(&config.database.sqlite_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let busy_ms = config.database.busy_timeout_ms;
    let manager = SqliteConnectionManager::file(&config.database.sqlite_path).with_init(
        move |conn: &mut Connection| {
            conn.busy_timeout(Duration::from_millis(busy_ms))?;
            conn.pragma_update(None, "journal_mode", "WAL")?;
            conn.pragma_update(None, "synchronous", "NORMAL")?;
            conn.pragma_update(None, "foreign_keys", "ON")?;
            Ok(())
        },
    );

    let max_size = config.database.pool_max_size.max(2);
    let pool = Pool::builder()
        .max_size(max_size)
        .build(manager)
        .map_err(|e| anyhow::anyhow!("init db pool: {e}"))?;

    let conn = pool
        .get()
        .map_err(|e| anyhow::anyhow!("get db conn: {e}"))?;
    conn.execute_batch(crate::INIT_SQL)?;
    Ok(pool)
}

/// Phase 2.2 Stage 2: 初始化 audit 专用 SQLite 库 + 连接池。
/// schema 由 [`INIT_AUDIT_SQL`] 建立；与主库相同的 WAL/busy_timeout 配置。
pub(crate) fn init_audit_db(config: &AppConfig) -> anyhow::Result<DbPool> {
    if let Some(parent) = Path::new(&config.database.audit_sqlite_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let busy_ms = config.database.busy_timeout_ms;
    let manager = SqliteConnectionManager::file(&config.database.audit_sqlite_path).with_init(
        move |conn: &mut Connection| {
            conn.busy_timeout(Duration::from_millis(busy_ms))?;
            conn.pragma_update(None, "journal_mode", "WAL")?;
            conn.pragma_update(None, "synchronous", "NORMAL")?;
            conn.pragma_update(None, "foreign_keys", "ON")?;
            Ok(())
        },
    );

    let max_size = config.database.audit_pool_max_size.max(2);
    let pool = Pool::builder()
        .max_size(max_size)
        .build(manager)
        .map_err(|e| anyhow::anyhow!("init audit db pool: {e}"))?;

    let conn = pool
        .get()
        .map_err(|e| anyhow::anyhow!("get audit db conn: {e}"))?;
    conn.execute_batch(INIT_AUDIT_SQL)?;
    Ok(pool)
}

/// Phase 2.2 Stage 2 一次性数据迁移：如果主库 `audit_logs` 还有行（旧部署），
/// 在启动时把它们复制到 audit pool，然后清空主库 audit_logs。
/// 失败只 log warn 不阻断启动（生产数据安全为先；旧库数据保留）。
pub(crate) fn migrate_audit_logs_from_main_db(
    main_pool: &DbPool,
    audit_pool: &DbPool,
) -> anyhow::Result<()> {
    let main_conn = main_pool
        .get()
        .map_err(|e| anyhow::anyhow!("audit-migrate: get main conn: {e}"))?;
    // 主库可能根本没 audit_logs 表（新部署）。先 PRAGMA 探测。
    let table_exists: i64 = main_conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='audit_logs'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if table_exists == 0 {
        return Ok(());
    }
    let pending: i64 = main_conn
        .query_row("SELECT COUNT(*) FROM audit_logs", [], |r| r.get(0))
        .unwrap_or(0);
    if pending == 0 {
        return Ok(());
    }
    tracing::info!(
        "phase2.2-stage2: migrating {} audit_logs row(s) from main db to audit db",
        pending
    );

    // 主库 audit_logs 可能没有 user_key 列（取决于历史 ALTER 是否跑过）。
    // 用 PRAGMA 探测一下。
    let mut stmt = main_conn.prepare("PRAGMA table_info(audit_logs)")?;
    let cols: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);
    let has_user_key = cols.iter().any(|c| c.eq_ignore_ascii_case("user_key"));

    let select_sql = if has_user_key {
        "SELECT ts, user_id, action, detail_json, error_text, user_key FROM audit_logs"
    } else {
        "SELECT ts, user_id, action, detail_json, error_text FROM audit_logs"
    };
    let mut stmt = main_conn.prepare(select_sql)?;
    let rows: Vec<(
        String,
        Option<i64>,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                if has_user_key {
                    row.get::<_, Option<String>>(5)?
                } else {
                    None
                },
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);

    let mut audit_conn = audit_pool
        .get()
        .map_err(|e| anyhow::anyhow!("audit-migrate: get audit conn: {e}"))?;
    let tx = audit_conn.transaction()?;
    for (ts, uid, action, detail, err, ukey) in &rows {
        tx.execute(
            "INSERT INTO audit_logs (ts, user_id, action, detail_json, error_text, user_key) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![ts, uid, action, detail, err, ukey],
        )?;
    }
    tx.commit()?;

    // Best-effort: 清空主库 audit_logs 释放空间（数据已经在 audit_db 了）。
    if let Err(e) = main_conn.execute("DELETE FROM audit_logs", []) {
        tracing::warn!("phase2.2-stage2: clearing main-db audit_logs after migration failed: {e}");
    }
    tracing::info!("phase2.2-stage2: migrated {} audit_logs row(s)", rows.len());
    Ok(())
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
        CREATE INDEX IF NOT EXISTS idx_scheduled_jobs_user_chat ON scheduled_jobs(user_id, chat_id);

        CREATE TABLE IF NOT EXISTS scheduled_job_runs (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id         TEXT NOT NULL UNIQUE,
            job_id         TEXT NOT NULL,
            task_id        TEXT NOT NULL,
            thread_ref     TEXT NOT NULL,
            task_status    TEXT NOT NULL CHECK (task_status IN ('queued', 'running', 'waiting', 'background', 'needs_user', 'succeeded', 'failed', 'canceled', 'cancelled', 'timeout')),
            triage_status  TEXT CHECK (triage_status IS NULL OR triage_status IN ('no_findings', 'findings', 'needs_user', 'failed', 'cancelled')),
            result_json    TEXT NOT NULL DEFAULT '{}',
            started_at     TEXT NOT NULL,
            finished_at    TEXT,
            created_at     TEXT NOT NULL,
            updated_at     TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_scheduled_job_runs_job_updated ON scheduled_job_runs(job_id, updated_at);
        CREATE INDEX IF NOT EXISTS idx_scheduled_job_runs_task ON scheduled_job_runs(task_id);
        CREATE INDEX IF NOT EXISTS idx_scheduled_job_runs_triage ON scheduled_job_runs(triage_status, updated_at);",
    )?;
    Ok(())
}

pub(crate) fn ensure_memory_schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(crate::MEMORY_UPGRADE_SQL)?;
    crate::memory::facts::ensure_memory_fact_schema(db)?;
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
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS followup_frames (
            user_id        INTEGER NOT NULL,
            chat_id        INTEGER NOT NULL,
            user_key       TEXT NOT NULL,
            frame_json     TEXT NOT NULL,
            source_task_id TEXT NOT NULL,
            updated_at_ts  INTEGER NOT NULL,
            expires_at_ts  INTEGER NOT NULL,
            PRIMARY KEY (user_id, chat_id, user_key)
        );
        CREATE INDEX IF NOT EXISTS idx_followup_frames_user_chat_updated_ts
        ON followup_frames(user_id, chat_id, updated_at_ts);
        CREATE INDEX IF NOT EXISTS idx_followup_frames_expires_at_ts
        ON followup_frames(expires_at_ts);",
    )?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS clarify_states (
            user_id         INTEGER NOT NULL,
            chat_id         INTEGER NOT NULL,
            user_key        TEXT NOT NULL,
            state_json      TEXT NOT NULL,
            source_task_id  TEXT NOT NULL,
            updated_at_ts   INTEGER NOT NULL,
            expires_at_ts   INTEGER NOT NULL,
            PRIMARY KEY (user_id, chat_id, user_key)
        );
        CREATE INDEX IF NOT EXISTS idx_clarify_states_user_chat_updated_ts
        ON clarify_states(user_id, chat_id, updated_at_ts);
        CREATE INDEX IF NOT EXISTS idx_clarify_states_expires_at_ts
        ON clarify_states(expires_at_ts);",
    )?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS observed_facts_states (
            user_id         INTEGER NOT NULL,
            chat_id         INTEGER NOT NULL,
            user_key        TEXT NOT NULL,
            facts_json      TEXT NOT NULL,
            source_task_id  TEXT NOT NULL,
            updated_at_ts   INTEGER NOT NULL,
            expires_at_ts   INTEGER NOT NULL,
            PRIMARY KEY (user_id, chat_id, user_key)
        );
        CREATE INDEX IF NOT EXISTS idx_observed_facts_states_user_chat_updated_ts
        ON observed_facts_states(user_id, chat_id, updated_at_ts);
        CREATE INDEX IF NOT EXISTS idx_observed_facts_states_expires_at_ts
        ON observed_facts_states(expires_at_ts);",
    )?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS conversation_states (
            user_id         INTEGER NOT NULL,
            chat_id         INTEGER NOT NULL,
            user_key        TEXT NOT NULL,
            state_json      TEXT NOT NULL,
            last_task_id    TEXT NOT NULL,
            updated_at_ts   INTEGER NOT NULL,
            PRIMARY KEY (user_id, chat_id, user_key)
        );
        CREATE INDEX IF NOT EXISTS idx_conversation_states_user_chat_updated_ts
        ON conversation_states(user_id, chat_id, updated_at_ts);",
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

pub(crate) fn ensure_task_lease_schema(db: &Connection) -> anyhow::Result<()> {
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
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_tasks_lease_owner_expires_at
         ON tasks(lease_owner, lease_expires_at);",
    )?;
    Ok(())
}
