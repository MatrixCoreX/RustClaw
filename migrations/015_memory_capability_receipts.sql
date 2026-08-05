-- Migration id: 015_memory_capability_receipts_v1
-- Host-owned effectively-once receipts for the built-in memory capability.
-- Ordinary skills never receive access to this table or the runtime database.

CREATE TABLE IF NOT EXISTS memory_capability_write_receipts (
    principal_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    content_digest TEXT NOT NULL,
    memory_id TEXT NOT NULL,
    memory_kind TEXT NOT NULL CHECK(memory_kind IN ('fact', 'preference', 'session_note')),
    scope_kind TEXT NOT NULL CHECK(scope_kind IN ('conversation', 'principal', 'project')),
    scope_ref TEXT NOT NULL,
    source_task_id TEXT NOT NULL,
    created_at_ts INTEGER NOT NULL,
    PRIMARY KEY(principal_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS idx_memory_capability_receipt_memory
ON memory_capability_write_receipts(principal_id, memory_id);
