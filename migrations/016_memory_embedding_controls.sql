-- Migration id: 016_memory_embedding_controls_v1
-- Principal-scoped pause/resume state for blue/green embedding builds.

CREATE TABLE IF NOT EXISTS memory_embedding_controls (
    principal_id TEXT NOT NULL,
    profile_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('active', 'paused')),
    updated_at_ts INTEGER NOT NULL,
    PRIMARY KEY(principal_id, profile_id)
);
