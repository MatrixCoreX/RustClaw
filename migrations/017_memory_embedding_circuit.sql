-- Migration id: 017_memory_embedding_circuit_v1
-- Principal/profile-scoped durable circuit state. It contains no query text,
-- memory content, endpoint, or credential material.

CREATE TABLE IF NOT EXISTS memory_embedding_circuits (
    principal_id TEXT NOT NULL,
    profile_id TEXT NOT NULL,
    failure_count INTEGER NOT NULL DEFAULT 0,
    open_until_ts INTEGER,
    last_error_code TEXT,
    updated_at_ts INTEGER NOT NULL,
    PRIMARY KEY(principal_id, profile_id)
);

CREATE INDEX IF NOT EXISTS idx_memory_embedding_circuits_open
ON memory_embedding_circuits(open_until_ts);
