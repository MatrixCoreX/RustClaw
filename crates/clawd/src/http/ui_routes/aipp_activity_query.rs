fn aipp_task_activity_page_sql(oldest: bool) -> String {
    let (cursor_operator, order) = if oldest { (">", "ASC") } else { ("<", "DESC") };
    format!(
        r#"
        WITH eligible AS MATERIALIZED (
            SELECT tasks.rowid AS activity_sequence,
                   tasks.task_id
              FROM tasks
            {AIPP_TASK_ACTIVITY_MATCH_SQL}
        ), filtered AS MATERIALIZED (
            SELECT activity_sequence, task_id
              FROM eligible
             WHERE (:cursor IS NULL OR activity_sequence {cursor_operator} :cursor)
             ORDER BY activity_sequence {order}
             LIMIT :limit
        ), skill_events AS (
            SELECT task_event_stream.task_id,
                   task_event_stream.created_at_ms,
                   json_extract(task_event_stream.event_json, '$.payload.requested_action_ref') AS action_ref
              FROM task_event_stream
              JOIN filtered ON filtered.task_id = task_event_stream.task_id
             WHERE json_extract(task_event_stream.event_json, '$.event_kind') = 'tool_finished'
               AND json_extract(task_event_stream.event_json, '$.payload.skill') = :skill_name
               AND COALESCE(
                   json_extract(task_event_stream.event_json, '$.payload.requested_action_ref'),
                   ''
               ) != (json_extract(task_event_stream.event_json, '$.payload.skill') || '.capabilities')
               AND task_event_stream.created_at_ms > :cleared_through_event_ms
            UNION ALL
            SELECT task_event_archive.task_id,
                   task_event_archive.created_at_ms,
                   json_extract(task_event_archive.event_json, '$.payload.requested_action_ref') AS action_ref
              FROM task_event_archive
              JOIN filtered ON filtered.task_id = task_event_archive.task_id
             WHERE json_extract(task_event_archive.event_json, '$.event_kind') = 'tool_finished'
               AND json_extract(task_event_archive.event_json, '$.payload.skill') = :skill_name
               AND COALESCE(
                   json_extract(task_event_archive.event_json, '$.payload.requested_action_ref'),
                   ''
               ) != (json_extract(task_event_archive.event_json, '$.payload.skill') || '.capabilities')
               AND task_event_archive.created_at_ms > :cleared_through_event_ms
        ), activity AS (
            SELECT task_id,
                   MAX(created_at_ms) AS event_at_ms,
                   GROUP_CONCAT(DISTINCT action_ref) AS action_refs
              FROM skill_events
             GROUP BY task_id
        ), page_data AS (
            SELECT filtered.activity_sequence,
                   filtered.task_id,
                   page_tasks.channel,
                   page_tasks.status,
                   page_tasks.payload_json,
                   page_tasks.result_json,
                   page_tasks.error_text,
                   page_tasks.created_at,
                   page_tasks.updated_at,
                   activity.event_at_ms,
                   activity.action_refs
              FROM filtered
              JOIN tasks AS page_tasks ON page_tasks.rowid = filtered.activity_sequence
              JOIN activity ON activity.task_id = filtered.task_id
        )
        SELECT page_data.*, total.total_item_count
          FROM (SELECT COUNT(*) AS total_item_count FROM eligible) AS total
          LEFT JOIN page_data ON 1
         ORDER BY page_data.activity_sequence {order}
        "#
    )
}
