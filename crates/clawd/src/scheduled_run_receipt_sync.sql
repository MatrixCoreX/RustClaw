-- Only a registered run's own delivery can settle its notification. Accepted and
-- delivered are distinct. Partial or retryable failures still need reconciliation.
CREATE VIEW IF NOT EXISTS scheduled_run_final_receipt AS
SELECT r.task_id, json_patch(r.result_json, json_object('notification', json_object(
    'accepted', json(CASE WHEN c.status = 'failed' THEN 'false' ELSE 'true' END),
    'delivered', json(CASE WHEN c.status IN ('delivered','read') THEN 'true' ELSE 'false' END),
    'delivery_status', c.status, 'delivery_id', c.delivery_id,
    'diagnostic_id', json_extract(c.receipt_json,'$.diagnostic_id'),
    'error_code', CASE WHEN c.status = 'failed' THEN coalesce(json_extract(c.receipt_json,'$.error_code'),'channel_send_failed') END,
    'message_key', json_extract(c.receipt_json,'$.message_key'),
    'retryable', json(CASE WHEN c.status = 'failed' THEN 'false' END)
))) AS final_result
FROM scheduled_job_runs r JOIN channel_delivery_receipts c
  ON c.idempotency_key = r.task_id || ':schedule-terminal'
 AND c.delivery_id = 'delivery:' || c.idempotency_key
WHERE json_type(r.result_json,'$.notification') = 'object'
  AND c.channel = json_extract(r.result_json,'$.notification.runtime_channel')
  AND (c.status IN ('accepted','delivered','read') OR
       (c.status = 'failed' AND json_extract(c.receipt_json,'$.retryable') = 0));

CREATE TRIGGER IF NOT EXISTS scheduled_notification_on_receipt_insert
AFTER INSERT ON channel_delivery_receipts WHEN NEW.idempotency_key LIKE '%:schedule-terminal'
BEGIN
  UPDATE scheduled_job_runs
  SET result_json = p.final_result FROM scheduled_run_final_receipt p
  WHERE p.task_id = substr(NEW.idempotency_key,1,length(NEW.idempotency_key)-length(':schedule-terminal'))
    AND scheduled_job_runs.task_id = p.task_id AND scheduled_job_runs.result_json != p.final_result;
END;
CREATE TRIGGER IF NOT EXISTS scheduled_notification_on_receipt_update
AFTER UPDATE OF status, receipt_json ON channel_delivery_receipts WHEN NEW.idempotency_key LIKE '%:schedule-terminal'
BEGIN
  UPDATE scheduled_job_runs SET result_json = p.final_result FROM scheduled_run_final_receipt p
  WHERE p.task_id = substr(NEW.idempotency_key,1,length(NEW.idempotency_key)-length(':schedule-terminal'))
    AND scheduled_job_runs.task_id = p.task_id AND scheduled_job_runs.result_json != p.final_result;
END;
CREATE TRIGGER IF NOT EXISTS scheduled_notification_on_result
AFTER UPDATE OF result_json ON scheduled_job_runs WHEN json_type(NEW.result_json,'$.notification') = 'object'
BEGIN
  UPDATE scheduled_job_runs SET result_json = p.final_result FROM scheduled_run_final_receipt p
  WHERE p.task_id = NEW.task_id AND scheduled_job_runs.task_id = p.task_id AND scheduled_job_runs.result_json != p.final_result;
END;
UPDATE scheduled_job_runs SET result_json = p.final_result FROM scheduled_run_final_receipt p
WHERE scheduled_job_runs.task_id = p.task_id AND scheduled_job_runs.result_json != p.final_result;
