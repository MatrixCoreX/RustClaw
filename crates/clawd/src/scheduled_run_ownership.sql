-- Capture the run owner once. An unowned legacy row remains visible to administrators.
CREATE TABLE IF NOT EXISTS scheduled_run_owners (
    run_id TEXT PRIMARY KEY,
    principal_id TEXT
);
CREATE INDEX IF NOT EXISTS scheduled_run_owners_principal ON scheduled_run_owners(principal_id,run_id);
CREATE VIEW IF NOT EXISTS scheduled_run_owner_evidence AS
SELECT r.run_id,
    CASE WHEN t.principal_id IS NOT NULL AND (j.principal_id IS NULL OR j.principal_id=t.principal_id)
            THEN t.principal_id
         WHEN t.principal_id IS NULL AND CAST(j.created_at AS INTEGER)<=CAST(r.started_at AS INTEGER)
            THEN j.principal_id END AS principal_id
FROM scheduled_job_runs r LEFT JOIN tasks t ON t.task_id=r.task_id
LEFT JOIN scheduled_jobs j ON j.job_id=r.job_id;
CREATE TRIGGER IF NOT EXISTS scheduled_run_capture_owner AFTER INSERT ON scheduled_job_runs
BEGIN
    INSERT INTO scheduled_run_owners SELECT run_id,principal_id FROM scheduled_run_owner_evidence
    WHERE run_id=NEW.run_id;
END;
CREATE TRIGGER IF NOT EXISTS scheduled_run_remove_owner AFTER DELETE ON scheduled_job_runs
BEGIN
    DELETE FROM scheduled_run_owners WHERE run_id=OLD.run_id;
END;
INSERT INTO scheduled_run_owners
SELECT e.run_id,e.principal_id FROM scheduled_run_owner_evidence e
WHERE NOT EXISTS (SELECT 1 FROM scheduled_run_owners o WHERE o.run_id=e.run_id);
