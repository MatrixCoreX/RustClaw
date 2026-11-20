//! Exact source checks for the optional unchanged-summary optimization.
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

pub(super) fn verified_turn(
    db: &Connection,
    task: &crate::ClaimedTask,
    job_id: &str,
) -> anyhow::Result<Option<Vec<String>>> {
    let binding: Option<(String, i64, i64, String, String)> = db
        .query_row(
            "SELECT principal_id,source_event_start,source_event_end,source_digest,eligibility_json
         FROM memory_jobs WHERE job_id=?1 AND source_task_id=?2",
            params![job_id, task.task_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let Some((principal, start, end, expected, eligibility)) = binding else {
        return Ok(None);
    };
    let Ok(eligibility) = serde_json::from_str::<Value>(&eligibility) else {
        return Ok(None);
    };
    let Some(policy) = eligibility["policy_digest"].as_str() else {
        return Ok(None);
    };
    let rows = db
        .prepare(
            "SELECT e.source_sequence,e.source_memory_id,e.content_digest,
        e.sensitivity,e.source_category,m.role,m.content FROM memory_source_events e
        JOIN memories m ON m.id=e.source_memory_id AND m.principal_id=e.principal_id
        WHERE e.source_task_id=?1 AND e.principal_id=?2 ORDER BY e.source_sequence",
        )?
        .query_map(params![task.task_id, principal], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if rows.len() != 2 {
        return Ok(None);
    }
    for (index, (sequence, id, sha, sensitivity, category, role, content)) in
        rows.iter().enumerate()
    {
        let (expected_role, expected_category) = if index == 0 {
            ("user", "user_authored")
        } else {
            ("assistant", "assistant_authored")
        };
        if *sequence != index as i64 + 1
            || *id < start
            || *id > end
            || sensitivity != "normal"
            || role != expected_role
            || category != expected_category
            || *sha != super::digest(content)
        {
            return Ok(None);
        }
    }
    let mut source_rows = rows
        .iter()
        .map(|row| (row.1, row.2.clone()))
        .collect::<Vec<_>>();
    source_rows.sort_by_key(|row| row.0);
    if source_rows[0].0 != start || source_rows[1].0 != end {
        return Ok(None);
    }
    let actual = super::digest(
        &json!({"source_task_id":task.task_id,"rows":source_rows,
        "eligibility_digest":policy})
        .to_string(),
    );
    if actual != expected {
        return Ok(None);
    }
    Ok(Some(rows.into_iter().map(|row| row.6).collect()))
}
