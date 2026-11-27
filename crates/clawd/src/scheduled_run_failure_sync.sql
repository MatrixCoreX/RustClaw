-- Copy only a machine-readable failure reason from the recorded execution.
-- Notification acknowledgements have their own lifecycle and fields.
CREATE VIEW IF NOT EXISTS scheduled_run_execution_failure AS
WITH failed AS (
    SELECT t.task_id, CASE WHEN json_valid(t.result_json) THEN t.result_json END AS evidence
    FROM tasks t JOIN scheduled_job_runs r ON r.task_id=t.task_id
    WHERE t.status IN ('failed','timeout')
), reasons AS (
    SELECT task_id, coalesce(
        nullif(json_extract(evidence,'$.error_code'),''),
        CASE WHEN json_extract(evidence,'$.task_journal.summary.final_stop_signal')
                  NOT IN ('respond','synthesize_answer','completed','success')
             THEN nullif(json_extract(evidence,'$.task_journal.summary.final_stop_signal'),'') END,
        nullif(json_extract(evidence,'$.task_journal.summary.final_failure_attribution'),'')
    ) AS reason FROM failed
)
SELECT task_id, reason FROM reasons
WHERE typeof(reason)='text' AND length(reason) BETWEEN 1 AND 128
  AND reason NOT GLOB '*[^a-zA-Z0-9_.:-]*';

CREATE TRIGGER IF NOT EXISTS scheduled_failure_on_execution
AFTER UPDATE OF status,result_json ON tasks
WHEN NEW.status IN ('failed','timeout') AND EXISTS (SELECT 1 FROM scheduled_job_runs WHERE task_id=NEW.task_id)
BEGIN
    UPDATE scheduled_job_runs SET result_json=json_set(result_json,'$.error_code',f.reason)
    FROM scheduled_run_execution_failure f WHERE f.task_id=NEW.task_id
      AND scheduled_job_runs.task_id=f.task_id AND json_extract(result_json,'$.error_code') IS NOT f.reason;
END;
CREATE TRIGGER IF NOT EXISTS scheduled_failure_on_summary
AFTER UPDATE OF result_json ON scheduled_job_runs
WHEN NEW.task_status IN ('failed','timeout')
BEGIN
    UPDATE scheduled_job_runs SET result_json=json_set(result_json,'$.error_code',f.reason)
    FROM scheduled_run_execution_failure f WHERE f.task_id=NEW.task_id
      AND scheduled_job_runs.task_id=f.task_id AND json_extract(result_json,'$.error_code') IS NOT f.reason;
END;
UPDATE scheduled_job_runs SET result_json=json_set(result_json,'$.error_code',f.reason)
FROM scheduled_run_execution_failure f WHERE scheduled_job_runs.task_id=f.task_id
  AND json_extract(result_json,'$.error_code') IS NOT f.reason;
