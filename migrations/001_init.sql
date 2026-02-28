PRAGMA journal_mode = WAL;

CREATE TABLE IF NOT EXISTS users (
    user_id      INTEGER PRIMARY KEY,
    role         TEXT NOT NULL CHECK (role IN ('admin', 'user')),
    is_allowed   INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT NOT NULL,
    last_seen    TEXT
);

CREATE TABLE IF NOT EXISTS tasks (
    task_id       TEXT PRIMARY KEY,
    user_id       INTEGER NOT NULL,
    chat_id       INTEGER NOT NULL,
    message_id    INTEGER,
    kind          TEXT NOT NULL CHECK (kind IN ('ask', 'run_skill', 'admin')),
    payload_json  TEXT NOT NULL,
    status        TEXT NOT NULL CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'canceled', 'timeout')),
    result_json   TEXT,
    error_text    TEXT,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS audit_logs (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    ts           TEXT NOT NULL,
    user_id      INTEGER,
    action       TEXT NOT NULL,
    detail_json  TEXT,
    error_text   TEXT
);

CREATE TABLE IF NOT EXISTS memories (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id      INTEGER NOT NULL,
    chat_id      INTEGER NOT NULL,
    role         TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    content      TEXT NOT NULL,
    created_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS long_term_memories (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id           INTEGER NOT NULL,
    chat_id           INTEGER NOT NULL,
    summary           TEXT NOT NULL,
    source_memory_id  INTEGER NOT NULL DEFAULT 0,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    UNIQUE(user_id, chat_id)
);

CREATE TABLE IF NOT EXISTS scheduled_jobs (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id            TEXT NOT NULL UNIQUE,
    user_id           INTEGER NOT NULL,
    chat_id           INTEGER NOT NULL,
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

CREATE INDEX IF NOT EXISTS idx_tasks_status_created_at ON tasks(status, created_at);
CREATE INDEX IF NOT EXISTS idx_tasks_user_id_created_at ON tasks(user_id, created_at);
CREATE INDEX IF NOT EXISTS idx_audit_logs_ts ON audit_logs(ts);
CREATE INDEX IF NOT EXISTS idx_memories_user_chat_created_at ON memories(user_id, chat_id, created_at);
CREATE INDEX IF NOT EXISTS idx_long_term_memories_updated_at ON long_term_memories(updated_at);
CREATE INDEX IF NOT EXISTS idx_scheduled_jobs_due ON scheduled_jobs(enabled, next_run_at);
CREATE INDEX IF NOT EXISTS idx_scheduled_jobs_user_chat ON scheduled_jobs(user_id, chat_id);
