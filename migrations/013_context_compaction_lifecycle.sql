-- Migration id: 013_context_compaction_lifecycle_v1
-- Durable single-writer state for conversation context compaction. Compaction
-- records are projections only; canonical task events remain the source of truth.

CREATE TABLE IF NOT EXISTS context_compaction_states (
    principal_id TEXT NOT NULL,
    conversation_ref TEXT NOT NULL,
    lineage_id TEXT NOT NULL,
    generation INTEGER NOT NULL DEFAULT 0,
    lease_owner TEXT,
    lease_expires_at_ts INTEGER,
    last_snapshot_digest TEXT,
    last_snapshot_task_row_id INTEGER NOT NULL DEFAULT 0,
    last_record_id TEXT,
    revision INTEGER NOT NULL DEFAULT 1,
    updated_at_ts INTEGER NOT NULL,
    PRIMARY KEY(principal_id, conversation_ref)
);

CREATE INDEX IF NOT EXISTS idx_context_compaction_states_lease
ON context_compaction_states(lease_expires_at_ts, lease_owner);

CREATE TABLE IF NOT EXISTS context_compaction_records (
    record_id TEXT PRIMARY KEY,
    principal_id TEXT NOT NULL,
    conversation_ref TEXT NOT NULL,
    lineage_id TEXT NOT NULL,
    generation INTEGER NOT NULL,
    source_task_id TEXT NOT NULL,
    snapshot_digest TEXT NOT NULL,
    snapshot_task_row_id INTEGER NOT NULL,
    snapshot_event_ranges_json TEXT NOT NULL,
    uncovered_tail_task_count INTEGER NOT NULL DEFAULT 0,
    record_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('valid', 'invalidated')),
    invalidation_reason_code TEXT,
    created_at_ts INTEGER NOT NULL,
    invalidated_at_ts INTEGER,
    UNIQUE(principal_id, conversation_ref, generation)
);

CREATE INDEX IF NOT EXISTS idx_context_compaction_records_conversation
ON context_compaction_records(principal_id, conversation_ref, generation DESC);
CREATE INDEX IF NOT EXISTS idx_context_compaction_records_snapshot_head
ON context_compaction_records(principal_id, conversation_ref, snapshot_task_row_id);
