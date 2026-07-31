CREATE TABLE IF NOT EXISTS pending_channel_requests (
    pending_request_id TEXT PRIMARY KEY,
    channel TEXT NOT NULL,
    adapter TEXT NOT NULL,
    external_user_id TEXT,
    external_chat_id TEXT,
    message_id TEXT,
    content_digest TEXT NOT NULL,
    attachment_refs_json TEXT NOT NULL,
    context_token TEXT,
    request_json TEXT NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL CHECK (status IN ('pending', 'submitted', 'expired', 'invalid')),
    task_id TEXT,
    error_code TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_pending_channel_requests_binding
ON pending_channel_requests(channel, external_user_id, external_chat_id, status, created_at);

CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_idempotency_key
ON tasks(idempotency_key) WHERE idempotency_key IS NOT NULL;
