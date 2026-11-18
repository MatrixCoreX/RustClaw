//! Conservative recovery for historical turns lacking an atomic source binding.
use super::*;

/// A time window alone is insufficient. Require exact user input plus one
/// unambiguous assistant row under the same authenticated principal/chat/key.
pub(super) fn verified_ids(
    db: &Connection,
    task: &crate::ClaimedTask,
    principal: &str,
) -> anyhow::Result<Option<(i64, i64)>> {
    let result: Option<String> = db
        .query_row(
            "SELECT result_json FROM tasks WHERE task_id = ?1 AND principal_id = ?2",
            params![task.task_id, principal],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let Some(result) = result.and_then(|raw| serde_json::from_str::<Value>(&raw).ok()) else {
        return Ok(None);
    };
    let Some(text) = result.get("text").and_then(Value::as_str) else {
        return Ok(None);
    };
    let messages = result
        .get("messages")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let answer = crate::finalize::assistant_memory_source_text(text, &messages);
    if answer.is_empty() {
        return Ok(None);
    }
    let marked_answer = format!("{}{}", super::super::LLM_SHORT_TERM_MEMORY_PREFIX, answer);
    let mut stmt = db.prepare("SELECT m.id, m.role FROM tasks t JOIN memories m
        ON m.principal_id = t.principal_id AND m.chat_id = t.chat_id AND m.user_key IS t.user_key
        WHERE t.task_id = ?1 AND t.principal_id = ?2 AND t.status = 'succeeded'
          AND m.created_at_ts BETWEEN CAST(t.updated_at AS INTEGER) - 5 AND CAST(t.updated_at AS INTEGER) + 5
          AND ((m.role = 'user' AND m.content = json_extract(t.payload_json, '$.text')) OR (m.role = 'assistant' AND m.content IN (?3, ?4)))
        ORDER BY m.id LIMIT 4")?;
    let rows = stmt
        .query_map(
            params![task.task_id, principal, answer, marked_answer],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    if rows.len() != 2 {
        return Ok(None);
    }
    let user = rows
        .iter()
        .find(|(_, role)| role == "user")
        .map(|(id, _)| *id);
    let assistant = rows
        .iter()
        .find(|(_, role)| role == "assistant")
        .map(|(id, _)| *id);
    Ok(user.zip(assistant))
}

pub(super) fn verify_legacy_job(
    db: &Connection,
    task: &crate::ClaimedTask,
    job: &MemoryJobSnapshot,
) -> anyhow::Result<()> {
    let (user, assistant) = verified_ids(db, task, &job.principal_id)?
        .ok_or_else(|| permanent("memory_job_source_binding_unverified"))?;
    anyhow::ensure!(
        job.source_event_start == Some(user.min(assistant))
            && job.source_event_end == Some(user.max(assistant)),
        permanent("memory_job_source_binding_mismatch")
    );
    let mut stmt = db.prepare("SELECT e.source_sequence, e.source_memory_id, e.content_digest, m.content
        FROM memory_source_events e JOIN memories m ON m.id = e.source_memory_id AND m.principal_id = e.principal_id
        WHERE e.source_task_id = ?1 AND e.principal_id = ?2 ORDER BY e.source_sequence")?;
    let rows = stmt
        .query_map(params![task.task_id, job.principal_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    anyhow::ensure!(
        rows.len() == 2
            && rows.iter().all(|(sequence, id, digest, content)| {
                ((*sequence == 1 && *id == user) || (*sequence == 2 && *id == assistant))
                    && *digest == sha256_bytes(content.as_bytes())
            }),
        permanent("memory_job_source_digest_mismatch")
    );
    Ok(())
}

#[cfg(test)]
#[path = "job_sources_tests.rs"]
mod tests;
