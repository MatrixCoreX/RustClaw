-- Migration id: 011_durable_memory_jobs_v1
-- Host-owned durable memory pipeline. Jobs reference canonical source rows and
-- never copy raw credentials, attachment bodies, tool output, or connector payloads.

CREATE TABLE IF NOT EXISTS memory_source_events (
    event_id TEXT PRIMARY KEY,
    source_task_id TEXT NOT NULL,
    source_sequence INTEGER NOT NULL,
    source_memory_id INTEGER,
    principal_id TEXT NOT NULL,
    conversation_scope_ref TEXT,
    source_category TEXT NOT NULL,
    actor_kind TEXT NOT NULL,
    eligibility TEXT NOT NULL,
    content_digest TEXT NOT NULL,
    sensitivity TEXT NOT NULL,
    evidence_ref TEXT,
    created_at_ts INTEGER NOT NULL,
    UNIQUE(source_task_id, source_sequence)
);

CREATE INDEX IF NOT EXISTS idx_memory_source_events_principal_task
ON memory_source_events(principal_id, source_task_id, source_sequence);

CREATE TABLE IF NOT EXISTS memory_jobs (
    job_id TEXT PRIMARY KEY,
    job_kind TEXT NOT NULL CHECK(job_kind IN ('extract', 'consolidate', 'reindex', 'retention')),
    principal_id TEXT NOT NULL,
    scope_kind TEXT NOT NULL CHECK(scope_kind IN ('conversation', 'principal', 'project')),
    scope_ref TEXT NOT NULL,
    source_task_id TEXT,
    source_event_start INTEGER,
    source_event_end INTEGER,
    source_digest TEXT NOT NULL,
    eligibility_json TEXT NOT NULL,
    settings_revision INTEGER NOT NULL,
    policy_digest TEXT NOT NULL,
    provider_name TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    model_name TEXT NOT NULL,
    model_capability_digest TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('queued', 'running', 'retry_wait', 'completed', 'cancelled', 'failed')),
    lease_owner TEXT,
    lease_expires_at_ts INTEGER,
    attempt INTEGER NOT NULL DEFAULT 0,
    not_before_ts INTEGER NOT NULL,
    checkpoint_json TEXT NOT NULL DEFAULT '{}',
    progress_current INTEGER NOT NULL DEFAULT 0,
    progress_total INTEGER NOT NULL DEFAULT 0,
    cancel_requested INTEGER NOT NULL DEFAULT 0,
    error_code TEXT,
    retryable INTEGER NOT NULL DEFAULT 0,
    created_at_ts INTEGER NOT NULL,
    updated_at_ts INTEGER NOT NULL,
    finished_at_ts INTEGER,
    UNIQUE(job_kind, principal_id, source_task_id, source_event_start, source_event_end, policy_digest)
);

CREATE INDEX IF NOT EXISTS idx_memory_jobs_claim
ON memory_jobs(status, not_before_ts, lease_expires_at_ts, created_at_ts);
CREATE INDEX IF NOT EXISTS idx_memory_jobs_principal_status
ON memory_jobs(principal_id, status, created_at_ts);

CREATE TABLE IF NOT EXISTS memory_raw_candidates (
    candidate_id TEXT PRIMARY KEY,
    generation_job_id TEXT NOT NULL,
    principal_id TEXT NOT NULL,
    scope_kind TEXT NOT NULL,
    scope_ref TEXT NOT NULL,
    candidate_kind TEXT NOT NULL,
    content_json TEXT NOT NULL,
    content_digest TEXT NOT NULL,
    trust_tier TEXT NOT NULL,
    sensitivity TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('pending', 'accepted', 'rejected', 'expired')),
    evidence_refs_json TEXT NOT NULL DEFAULT '[]',
    created_at_ts INTEGER NOT NULL,
    reviewed_at_ts INTEGER,
    UNIQUE(generation_job_id, content_digest)
);

CREATE TABLE IF NOT EXISTS memory_evidence (
    evidence_id TEXT PRIMARY KEY,
    principal_id TEXT NOT NULL,
    source_type TEXT NOT NULL,
    source_ref TEXT NOT NULL,
    source_digest TEXT NOT NULL,
    source_event_start INTEGER,
    source_event_end INTEGER,
    redacted_excerpt TEXT,
    availability TEXT NOT NULL CHECK(availability IN ('available', 'source_unavailable', 'purged')),
    created_at_ts INTEGER NOT NULL,
    UNIQUE(principal_id, source_type, source_ref, source_digest)
);

CREATE TABLE IF NOT EXISTS memory_retention_ledger (
    ledger_id TEXT PRIMARY KEY,
    principal_id TEXT,
    scope_kind TEXT,
    scope_ref TEXT,
    object_kind TEXT NOT NULL,
    object_count INTEGER NOT NULL,
    object_digest TEXT NOT NULL,
    reason_code TEXT NOT NULL,
    actor_principal_id TEXT,
    created_at_ts INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS memory_storage_pressure (
    singleton_id INTEGER PRIMARY KEY CHECK(singleton_id = 1),
    state TEXT NOT NULL CHECK(state IN ('normal', 'derived_cleanup', 'backfill_paused', 'automatic_generation_paused', 'explicit_write_blocked')),
    reason_code TEXT,
    observed_bytes INTEGER NOT NULL DEFAULT 0,
    revision INTEGER NOT NULL DEFAULT 1,
    updated_at_ts INTEGER NOT NULL
);

INSERT INTO memory_storage_pressure(singleton_id, state, observed_bytes, revision, updated_at_ts)
VALUES (1, 'normal', 0, 1, 0)
ON CONFLICT(singleton_id) DO NOTHING;

CREATE TABLE IF NOT EXISTS memory_principal_quotas (
    principal_id TEXT PRIMARY KEY,
    max_rows INTEGER NOT NULL,
    max_bytes INTEGER NOT NULL,
    max_background_cost_microunits INTEGER NOT NULL,
    used_rows INTEGER NOT NULL DEFAULT 0,
    used_bytes INTEGER NOT NULL DEFAULT 0,
    used_background_cost_microunits INTEGER NOT NULL DEFAULT 0,
    revision INTEGER NOT NULL DEFAULT 1,
    updated_at_ts INTEGER NOT NULL
);

-- The versioned Rust executor also adds lineage/time columns to existing
-- memory_facts and user_preferences tables using idempotent ALTER statements.
