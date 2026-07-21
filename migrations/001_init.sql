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
    channel       TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark', 'wechat')),
    external_user_id TEXT,
    external_chat_id TEXT,
    message_id    INTEGER,
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

CREATE TABLE IF NOT EXISTS child_task_graphs (
    parent_task_id TEXT PRIMARY KEY,
    schema_version INTEGER NOT NULL,
    status TEXT NOT NULL,
    max_parallel INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS child_task_graph_nodes (
    parent_task_id TEXT NOT NULL,
    child_task_id TEXT PRIMARY KEY,
    role TEXT NOT NULL,
    required INTEGER NOT NULL,
    readiness TEXT NOT NULL,
    permission_profile TEXT NOT NULL,
    merge_policy TEXT NOT NULL,
    owned_paths_json TEXT NOT NULL,
    budget_json TEXT NOT NULL,
    model_policy_json TEXT NOT NULL,
    tool_policy_json TEXT NOT NULL,
    result_contract_json TEXT NOT NULL,
    steering_version INTEGER NOT NULL DEFAULT 0,
    steering_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_child_task_graph_nodes_parent_readiness
ON child_task_graph_nodes(parent_task_id, readiness, created_at);

CREATE TABLE IF NOT EXISTS child_task_graph_edges (
    parent_task_id TEXT NOT NULL,
    predecessor_task_id TEXT NOT NULL,
    successor_task_id TEXT NOT NULL,
    required INTEGER NOT NULL,
    edge_kind TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY(parent_task_id, predecessor_task_id, successor_task_id)
);

CREATE INDEX IF NOT EXISTS idx_child_task_graph_edges_successor
ON child_task_graph_edges(parent_task_id, successor_task_id);

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
    channel      TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark', 'wechat')),
    external_chat_id TEXT,
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
    created_at_ts     INTEGER NOT NULL DEFAULT 0,
    updated_at_ts     INTEGER NOT NULL DEFAULT 0,
    user_key          TEXT,
    UNIQUE(user_id, chat_id, user_key)
);

CREATE TABLE IF NOT EXISTS scheduled_jobs (
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
    isolation_profile TEXT NOT NULL DEFAULT 'local_current_workspace',
    permission_policy_json TEXT NOT NULL DEFAULT '{}',
    thread_resume_enabled INTEGER NOT NULL DEFAULT 1,
    last_thread_task_id TEXT,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tasks_status_created_at ON tasks(status, created_at);
CREATE INDEX IF NOT EXISTS idx_tasks_user_id_created_at ON tasks(user_id, created_at);
CREATE INDEX IF NOT EXISTS idx_audit_logs_ts ON audit_logs(ts);
CREATE INDEX IF NOT EXISTS idx_memories_user_chat_created_at ON memories(user_id, chat_id, created_at);
CREATE INDEX IF NOT EXISTS idx_long_term_memories_updated_at ON long_term_memories(updated_at);
CREATE INDEX IF NOT EXISTS idx_long_term_memories_updated_at_ts ON long_term_memories(updated_at_ts);
CREATE INDEX IF NOT EXISTS idx_long_term_memories_user_key_chat_updated_ts ON long_term_memories(user_key, chat_id, updated_at_ts);
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
CREATE INDEX IF NOT EXISTS idx_scheduled_job_runs_triage ON scheduled_job_runs(triage_status, updated_at);

CREATE TABLE IF NOT EXISTS task_event_stream (
    task_id       TEXT NOT NULL,
    seq           INTEGER NOT NULL,
    event_hash    TEXT NOT NULL,
    event_json    TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (task_id, seq),
    UNIQUE (task_id, event_hash)
);
CREATE INDEX IF NOT EXISTS idx_task_event_stream_task_seq
    ON task_event_stream(task_id, seq);

CREATE TABLE IF NOT EXISTS task_event_artifacts (
    task_id       TEXT NOT NULL,
    artifact_id   TEXT NOT NULL,
    payload_json  TEXT NOT NULL,
    payload_bytes INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (task_id, artifact_id)
);

CREATE TABLE IF NOT EXISTS task_mutation_ledger (
    task_id            TEXT NOT NULL,
    fingerprint_hash   TEXT NOT NULL,
    action_ref         TEXT NOT NULL,
    status             TEXT NOT NULL CHECK (status IN ('started', 'completed', 'uncertain')),
    execution_token    TEXT NOT NULL,
    lease_owner        TEXT NOT NULL,
    claim_attempt      INTEGER NOT NULL,
    outcome_hash       TEXT,
    outcome_json       TEXT,
    started_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL,
    completed_at       INTEGER,
    PRIMARY KEY (task_id, fingerprint_hash)
);
CREATE INDEX IF NOT EXISTS idx_task_mutation_ledger_status_updated
    ON task_mutation_ledger(status, updated_at);

CREATE TABLE IF NOT EXISTS task_checkpoint_actions (
    task_id              TEXT NOT NULL,
    checkpoint_id        TEXT NOT NULL,
    tool_or_skill        TEXT NOT NULL,
    action_ref           TEXT NOT NULL,
    args_json            TEXT NOT NULL,
    output_contract_json TEXT,
    integrity_hash       TEXT NOT NULL,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    PRIMARY KEY (task_id, checkpoint_id),
    FOREIGN KEY (task_id) REFERENCES tasks(task_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_task_checkpoint_actions_updated
    ON task_checkpoint_actions(updated_at);

CREATE TABLE IF NOT EXISTS llm_cost_ledger (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id                  TEXT NOT NULL,
    user_id                  INTEGER NOT NULL,
    provider                 TEXT NOT NULL,
    model                    TEXT NOT NULL,
    logical_call_index       INTEGER NOT NULL,
    prompt_label             TEXT NOT NULL,
    provider_status          TEXT NOT NULL,
    cost_status              TEXT NOT NULL,
    estimated_cost_usd_nanos INTEGER,
    record_json              TEXT NOT NULL,
    created_at_ts            INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_llm_cost_ledger_task_id
    ON llm_cost_ledger(task_id, id);
CREATE INDEX IF NOT EXISTS idx_llm_cost_ledger_user_created
    ON llm_cost_ledger(user_id, created_at_ts);
CREATE INDEX IF NOT EXISTS idx_llm_cost_ledger_provider_created
    ON llm_cost_ledger(provider, created_at_ts);
