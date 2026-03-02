use rusqlite::params;
use serde_json::Value;

use crate::{AppState, ClaimedTask};

pub(crate) fn build_recent_execution_context(
    state: &AppState,
    task: &ClaimedTask,
    limit: usize,
) -> String {
    let db = match state.db.lock() {
        Ok(v) => v,
        Err(_) => return "<none>".to_string(),
    };
    let mut stmt = match db.prepare(
        "SELECT kind, payload_json, result_json, updated_at
         FROM tasks
         WHERE user_id = ?1 AND chat_id = ?2 AND status = 'succeeded'
         ORDER BY CAST(updated_at AS INTEGER) DESC
         LIMIT ?3",
    ) {
        Ok(v) => v,
        Err(_) => return "<none>".to_string(),
    };
    let rows = match stmt.query_map(params![task.user_id, task.chat_id, limit as i64], |row| {
        let kind: String = row.get(0)?;
        let payload_json: String = row.get(1)?;
        let result_json: String = row.get(2)?;
        let updated_at: String = row.get(3)?;
        Ok((kind, payload_json, result_json, updated_at))
    }) {
        Ok(v) => v,
        Err(_) => return "<none>".to_string(),
    };

    let mut items = Vec::new();
    for row in rows {
        let Ok((kind, payload_json, result_json, updated_at)) = row else {
            continue;
        };
        let req = task_payload_summary(&kind, &payload_json);
        let result = task_result_summary(&result_json);
        items.push(format!(
            "- ts={updated_at} kind={kind} request={} result={}",
            truncate_snippet(&req, 220),
            truncate_snippet(&result, 320)
        ));
    }
    if items.is_empty() {
        "<none>".to_string()
    } else {
        items.join("\n")
    }
}

fn task_payload_summary(kind: &str, payload_json: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(payload_json) else {
        return payload_json.to_string();
    };
    match kind {
        "ask" => v
            .get("text")
            .and_then(|x| x.as_str())
            .unwrap_or(payload_json)
            .to_string(),
        "run_skill" => {
            let skill = v.get("skill_name").and_then(|x| x.as_str()).unwrap_or("unknown");
            format!(
                "run_skill:{skill} args={}",
                v.get("args").cloned().unwrap_or(Value::Null)
            )
        }
        _ => payload_json.to_string(),
    }
}

fn task_result_summary(result_json: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(result_json) else {
        return result_json.to_string();
    };
    v.get("text")
        .and_then(|x| x.as_str())
        .or_else(|| v.as_str())
        .unwrap_or(result_json)
        .to_string()
}

fn truncate_snippet(text: &str, max_chars: usize) -> String {
    let t = text.trim();
    if t.chars().count() <= max_chars {
        return t.to_string();
    }
    let mut out = String::new();
    for (idx, ch) in t.chars().enumerate() {
        if idx >= max_chars {
            break;
        }
        out.push(ch);
    }
    out.push_str("...(truncated)");
    out
}
