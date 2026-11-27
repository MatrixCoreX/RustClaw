WITH matching_tasks AS (
    SELECT task_id FROM task_event_archive
    WHERE json_extract(event_json,'$.event_kind')='tool_finished'
      AND json_extract(event_json,'$.payload.skill')=:skill
      AND COALESCE(json_extract(event_json,'$.payload.requested_action_ref'),'')
            != (json_extract(event_json,'$.payload.skill') || '.capabilities')
      AND json_extract(event_json,'$.payload.requested_action_ref') IN (SELECT value FROM json_each(:refs))
    UNION
    SELECT task_id FROM task_event_stream
    WHERE json_extract(event_json,'$.event_kind')='tool_finished'
      AND json_extract(event_json,'$.payload.skill')=:skill
      AND COALESCE(json_extract(event_json,'$.payload.requested_action_ref'),'')
            != (json_extract(event_json,'$.payload.skill') || '.capabilities')
      AND json_extract(event_json,'$.payload.requested_action_ref') IN (SELECT value FROM json_each(:refs))
    UNION
    SELECT task_id FROM tasks
    WHERE kind='run_skill' AND json_valid(payload_json)
      AND json_extract(payload_json,'$.skill_name')=:skill
      AND (:skill || '.' || json_extract(payload_json,'$.args.action')) IN (SELECT value FROM json_each(:refs))
),
editions AS (
    SELECT t.rowid AS sequence,t.task_id,
           CASE WHEN s.run_id GLOB 'manual_*' THEN 'manual'
                WHEN s.run_id IS NOT NULL OR
                     (json_extract(t.payload_json,'$.invocation_source')='schedule'
                      AND json_extract(t.payload_json,'$.schedule_triggered')=1)
                THEN 'scheduled' ELSE 'manual' END AS trigger_kind,
           s.result_json AS scheduled_result
    FROM matching_tasks m JOIN tasks t ON t.task_id=m.task_id
    LEFT JOIN scheduled_job_runs s ON s.id=(SELECT sr.id FROM scheduled_job_runs sr
           WHERE sr.task_id=t.task_id ORDER BY sr.id DESC LIMIT 1)
    WHERE (:admin OR t.principal_id=:principal) AND (:cursor IS NULL OR t.rowid<:cursor)
      AND COALESCE(t.idempotency_key,'') NOT LIKE 'nni-capability:%'
      AND COALESCE(json_extract(t.payload_json,'$._task_source'),'') NOT IN ('distributed_work','aipp_account_control')
),
page AS MATERIALIZED (
    SELECT * FROM editions WHERE (:trigger IS NULL OR trigger_kind=:trigger)
    ORDER BY sequence DESC LIMIT :limit
)
SELECT page.sequence,t.task_id,t.channel,t.status,t.created_at,page.trigger_kind,
       json_extract(page.scheduled_result,'$.notification.delivery_status'),
       COALESCE(json_extract(page.scheduled_result,'$.error_code'),
            CASE WHEN json_valid(t.error_text) THEN COALESCE(json_extract(t.error_text,'$.error_code'),
                json_extract(t.error_text,'$.message_key')) END),
       CASE WHEN t.status='succeeded' THEN json_extract(t.result_json,'$.text') END,json_extract(t.result_json,'$.task_journal.trace.capability_results'),
       substr(json_extract(t.payload_json,'$.text'),1,120)
FROM page JOIN tasks t ON t.rowid=page.sequence ORDER BY page.sequence DESC
