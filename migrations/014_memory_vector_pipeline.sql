-- Migration id: 014_memory_vector_pipeline_v1
-- Versioned async embedding outbox and exact-vector backend. Canonical memory
-- rows remain authoritative; vectors and snapshots are disposable projections.

CREATE TABLE IF NOT EXISTS memory_embedding_profiles (
    profile_id TEXT PRIMARY KEY,
    provider_kind TEXT NOT NULL CHECK(provider_kind IN ('local', 'remote_http', 'mock')),
    endpoint_ref TEXT,
    credential_ref TEXT,
    model_name TEXT NOT NULL,
    dimensions INTEGER NOT NULL,
    normalization TEXT NOT NULL CHECK(normalization IN ('unit_length', 'none')),
    projection_version TEXT NOT NULL,
    profile_version TEXT NOT NULL,
    remote_consent_required INTEGER NOT NULL DEFAULT 1,
    state TEXT NOT NULL CHECK(state IN ('active', 'building', 'paused', 'retired')),
    generation INTEGER NOT NULL DEFAULT 1,
    building_generation INTEGER,
    config_digest TEXT NOT NULL,
    created_at_ts INTEGER NOT NULL,
    updated_at_ts INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS memory_embedding_jobs (
    job_id TEXT PRIMARY KEY,
    retrieval_id INTEGER NOT NULL,
    principal_id TEXT NOT NULL,
    scope_kind TEXT NOT NULL CHECK(scope_kind IN ('conversation', 'principal', 'project')),
    scope_ref TEXT NOT NULL,
    profile_id TEXT NOT NULL,
    profile_generation INTEGER NOT NULL,
    request_item_id TEXT NOT NULL,
    projection_version TEXT NOT NULL,
    projection_digest TEXT NOT NULL,
    consent_policy_digest TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('queued', 'running', 'retry_wait', 'completed', 'cancelled', 'failed')),
    lease_owner TEXT,
    lease_expires_at_ts INTEGER,
    attempt INTEGER NOT NULL DEFAULT 0,
    not_before_ts INTEGER NOT NULL,
    checkpoint_json TEXT NOT NULL DEFAULT '{}',
    error_code TEXT,
    retryable INTEGER NOT NULL DEFAULT 0,
    cancel_requested INTEGER NOT NULL DEFAULT 0,
    created_at_ts INTEGER NOT NULL,
    updated_at_ts INTEGER NOT NULL,
    finished_at_ts INTEGER,
    UNIQUE(retrieval_id, profile_id, profile_generation, projection_digest)
);

CREATE INDEX IF NOT EXISTS idx_memory_embedding_jobs_claim
ON memory_embedding_jobs(status, not_before_ts, lease_expires_at_ts, created_at_ts);
CREATE INDEX IF NOT EXISTS idx_memory_embedding_jobs_partition
ON memory_embedding_jobs(principal_id, profile_id, consent_policy_digest, status);

CREATE TABLE IF NOT EXISTS memory_vector_rows (
    retrieval_id INTEGER NOT NULL,
    principal_id TEXT NOT NULL,
    scope_kind TEXT NOT NULL,
    scope_ref TEXT NOT NULL,
    profile_id TEXT NOT NULL,
    generation INTEGER NOT NULL,
    projection_version TEXT NOT NULL,
    projection_digest TEXT NOT NULL,
    vector_format TEXT NOT NULL CHECK(vector_format IN ('f32le_v1')),
    dimensions INTEGER NOT NULL,
    normalization TEXT NOT NULL,
    vector_blob BLOB NOT NULL,
    vector_checksum TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('active', 'tombstone')),
    created_at_ts INTEGER NOT NULL,
    updated_at_ts INTEGER NOT NULL,
    PRIMARY KEY(retrieval_id, profile_id, generation)
);

CREATE INDEX IF NOT EXISTS idx_memory_vector_scope_profile
ON memory_vector_rows(principal_id, scope_kind, scope_ref, profile_id, status);

CREATE TABLE IF NOT EXISTS memory_vector_snapshots (
    snapshot_id TEXT PRIMARY KEY,
    principal_id TEXT NOT NULL,
    profile_id TEXT NOT NULL,
    generation INTEGER NOT NULL,
    row_count INTEGER NOT NULL,
    source_digest TEXT NOT NULL,
    snapshot_checksum TEXT NOT NULL,
    quality_fixture_digest TEXT,
    state TEXT NOT NULL CHECK(state IN ('building', 'verified', 'active', 'retired', 'corrupt')),
    checkpoint_retrieval_id INTEGER NOT NULL DEFAULT 0,
    created_at_ts INTEGER NOT NULL,
    updated_at_ts INTEGER NOT NULL,
    UNIQUE(principal_id, profile_id, generation)
);
