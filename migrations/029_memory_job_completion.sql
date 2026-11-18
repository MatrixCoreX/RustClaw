-- Migration id: 029_memory_job_completion_v1
-- Historical terminal jobs have no pending charge: upgrading never rebills
-- previously completed work. New charges accompany a durable result marker.
ALTER TABLE memory_jobs ADD COLUMN background_cost_pending_microunits INTEGER NOT NULL DEFAULT 0
CHECK(typeof(background_cost_pending_microunits) = 'integer' AND background_cost_pending_microunits >= 0);
ALTER TABLE memory_jobs ADD COLUMN background_cost_settled_at_ts INTEGER;
ALTER TABLE memory_jobs ADD COLUMN background_cost_retry_at_ts INTEGER NOT NULL DEFAULT 0;

CREATE INDEX idx_memory_jobs_pending_cost
ON memory_jobs(background_cost_retry_at_ts, updated_at_ts, job_id)
WHERE background_cost_pending_microunits > 0;
CREATE INDEX idx_memory_jobs_pending_cost_principal
ON memory_jobs(principal_id) WHERE background_cost_pending_microunits > 0;

-- Response content is private memory-domain data, never checkpoint prose.
-- Only the sensitivity-filtered response is stored; the checkpoint has hashes.
CREATE TABLE memory_generation_outputs (
    job_id TEXT PRIMARY KEY REFERENCES memory_jobs(job_id) ON DELETE CASCADE,
    principal_id TEXT NOT NULL,
    response_text TEXT NOT NULL,
    response_digest TEXT NOT NULL,
    prompt_digest TEXT NOT NULL,
    created_at_ts INTEGER NOT NULL
);
