-- Migration id: 012_memory_ux_audit_v1
-- Append-only user correction, feedback, export/import, deletion, and undo ledger.

CREATE TABLE IF NOT EXISTS memory_revisions (
    revision_id TEXT PRIMARY KEY,
    memory_id TEXT NOT NULL,
    principal_id TEXT NOT NULL,
    scope_kind TEXT NOT NULL,
    scope_ref TEXT NOT NULL,
    object_kind TEXT NOT NULL,
    row_revision INTEGER NOT NULL,
    operation TEXT NOT NULL CHECK(operation IN ('correct', 'delete', 'restore', 'import')),
    previous_snapshot_json TEXT NOT NULL,
    replacement_memory_id TEXT,
    undo_expires_at_ts INTEGER,
    actor_principal_id TEXT NOT NULL,
    created_at_ts INTEGER NOT NULL,
    UNIQUE(memory_id, row_revision, operation)
);

CREATE INDEX IF NOT EXISTS idx_memory_revisions_principal_created
ON memory_revisions(principal_id, created_at_ts DESC);

CREATE TABLE IF NOT EXISTS memory_retrieval_feedback (
    feedback_id TEXT PRIMARY KEY,
    principal_id TEXT NOT NULL,
    memory_id TEXT NOT NULL,
    feedback_kind TEXT NOT NULL
        CHECK(feedback_kind IN ('incorrect', 'irrelevant', 'do_not_use')),
    retrieval_event_ref TEXT,
    expected_revision INTEGER NOT NULL,
    created_at_ts INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_memory_feedback_principal_memory
ON memory_retrieval_feedback(principal_id, memory_id, created_at_ts DESC);

CREATE TABLE IF NOT EXISTS memory_privacy_purge_queue (
    purge_id TEXT PRIMARY KEY,
    principal_id TEXT NOT NULL,
    memory_id TEXT NOT NULL,
    object_kind TEXT NOT NULL,
    purge_after_ts INTEGER NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('grace', 'purged', 'cancelled')),
    created_at_ts INTEGER NOT NULL,
    completed_at_ts INTEGER,
    UNIQUE(principal_id, memory_id, status)
);

CREATE TABLE IF NOT EXISTS memory_import_sessions (
    import_id TEXT PRIMARY KEY,
    principal_id TEXT NOT NULL,
    scope_kind TEXT NOT NULL,
    scope_ref TEXT NOT NULL,
    payload_digest TEXT NOT NULL,
    preview_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('preview', 'confirmed', 'cancelled', 'failed')),
    created_at_ts INTEGER NOT NULL,
    confirmed_at_ts INTEGER,
    UNIQUE(principal_id, payload_digest)
);
