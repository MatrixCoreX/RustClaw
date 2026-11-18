-- Migration id: 033_memory_owned_retirement_access_v1
-- Physical access paths only. Legacy duplicates and all retirement predicates
-- remain valid; these indexes do not grant access or change canonical data.
CREATE INDEX IF NOT EXISTS main.idx_memory_retrieval_owned_source
ON memory_retrieval_index(principal_id, scope_kind, scope_ref, source_kind, source_ref);

CREATE INDEX IF NOT EXISTS main.idx_memory_embedding_jobs_retirement
ON memory_embedding_jobs(retrieval_id, cancel_requested, status);
