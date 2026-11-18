-- Explicit host-owned quota-policy initialization, independent of usage.
-- Keep this receipt when a quota row is absent; ordinary operations cannot
-- treat a lost policy as a never-initialized account.
CREATE TABLE IF NOT EXISTS memory_quota_initializations (
    principal_id TEXT PRIMARY KEY CHECK(length(principal_id) > 0),
    initialization_kind TEXT NOT NULL CHECK(initialization_kind IN ('legacy_startup', 'new_identity')),
    initialized_at_ts INTEGER NOT NULL
);
