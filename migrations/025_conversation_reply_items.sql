-- Durable, non-terminal replies produced while an agent task remains active.
-- These rows are intentionally separate from terminal task results/outbox.

CREATE TABLE IF NOT EXISTS conversation_reply_items (
    reply_id                TEXT PRIMARY KEY,
    task_id                 TEXT NOT NULL,
    input_id                TEXT,
    owner_principal_id      TEXT NOT NULL,
    instruction_revision    INTEGER NOT NULL,
    execution_epoch         INTEGER NOT NULL,
    relation                TEXT NOT NULL CHECK (
        relation IN ('side_reply', 'clarification', 'control_status')
    ),
    lifecycle_stage         TEXT NOT NULL CHECK (
        lifecycle_stage IN ('accepted', 'stop_requested', 'settled')
    ),
    text                    TEXT NOT NULL,
    content_digest          TEXT NOT NULL,
    created_at_ts           INTEGER NOT NULL,
    UNIQUE(task_id, content_digest)
);

CREATE INDEX IF NOT EXISTS idx_conversation_reply_items_task_created
    ON conversation_reply_items(task_id, created_at_ts, reply_id);

CREATE TABLE IF NOT EXISTS conversation_reply_delivery_outbox (
    reply_id             TEXT PRIMARY KEY,
    state                TEXT NOT NULL CHECK (
        state IN ('pending', 'dispatching', 'completed', 'failed')
    ),
    lease_token          TEXT,
    lease_expires_at_ts  INTEGER NOT NULL DEFAULT 0,
    next_attempt_at_ts   INTEGER NOT NULL DEFAULT 0,
    attempt_count        INTEGER NOT NULL DEFAULT 0,
    last_error_code      TEXT,
    created_at_ts        INTEGER NOT NULL,
    updated_at_ts        INTEGER NOT NULL,
    FOREIGN KEY(reply_id) REFERENCES conversation_reply_items(reply_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_conversation_reply_delivery_due
    ON conversation_reply_delivery_outbox(state, next_attempt_at_ts, lease_expires_at_ts);
