use rusqlite::params;
use serde_json::Value;

use crate::{AppState, ClaimedTask};

fn query_recent_execution_rows(
    db: &rusqlite::Connection,
    user_id: i64,
    chat_id: i64,
    user_key: Option<&str>,
    limit: usize,
) -> anyhow::Result<Vec<(String, String, String, String)>> {
    let mut stmt = db.prepare(
        "SELECT kind, payload_json, result_json, updated_at
         FROM tasks
         WHERE user_id = ?1
           AND chat_id = ?2
           AND status = 'succeeded'
           AND (
             (?3 IS NOT NULL AND user_key = ?3)
             OR (?3 IS NULL AND (user_key IS NULL OR TRIM(user_key) = ''))
           )
         ORDER BY CAST(updated_at AS INTEGER) DESC
         LIMIT ?4",
    )?;
    let rows = stmt.query_map(params![user_id, chat_id, user_key, limit as i64], |row| {
        let kind: String = row.get(0)?;
        let payload_json: String = row.get(1)?;
        let result_json: String = row.get(2)?;
        let updated_at: String = row.get(3)?;
        Ok((kind, payload_json, result_json, updated_at))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub(crate) fn build_recent_execution_context(
    state: &AppState,
    task: &ClaimedTask,
    limit: usize,
) -> String {
    let user_key = task.user_key.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let legacy_chat_id = user_key
        .map(crate::stable_i64_from_key)
        .filter(|legacy| *legacy != task.chat_id);
    let db = match state.db.lock() {
        Ok(v) => v,
        Err(_) => return "<none>".to_string(),
    };
    let rows = match query_recent_execution_rows(&db, task.user_id, task.chat_id, user_key, limit) {
        Ok(v) => v,
        Err(_) => return "<none>".to_string(),
    };

    let mut items = Vec::new();
    for (kind, payload_json, result_json, updated_at) in rows {
        let req = task_payload_summary(&kind, &payload_json);
        let result = task_result_summary(&result_json);
        items.push(format!(
            "- ts={updated_at} kind={kind} request={} result={}",
            truncate_snippet(&req, 220),
            truncate_snippet(&result, 320)
        ));
    }
    if items.is_empty() {
        if let Some(legacy_chat_id) = legacy_chat_id {
            let rows = match query_recent_execution_rows(&db, task.user_id, legacy_chat_id, user_key, limit) {
                Ok(v) => v,
                Err(_) => return "<none>".to_string(),
            };
            for (kind, payload_json, result_json, updated_at) in rows {
                let req = task_payload_summary(&kind, &payload_json);
                let result = task_result_summary(&result_json);
                items.push(format!(
                    "- ts={updated_at} kind={kind} request={} result={}",
                    truncate_snippet(&req, 220),
                    truncate_snippet(&result, 320)
                ));
            }
        }
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
        return sanitize_result_summary(result_json);
    };
    let text = v.get("text")
        .and_then(|x| x.as_str())
        .or_else(|| v.as_str())
        .unwrap_or(result_json)
        .to_string();
    sanitize_result_summary(&text)
}

fn sanitize_result_summary(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let refusal_markers = [
        "no code generation",
        "all executable code is forbidden",
        "不能提供可执行代码",
        "禁止生成任何可执行代码",
        "当前策略明确禁止",
        "不提供java代码示例",
        "不提供可执行的java",
    ];
    if refusal_markers.iter().any(|m| lower.contains(m)) {
        "<assistant policy-style refusal omitted from routing context>".to_string()
    } else {
        text.to_string()
    }
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
