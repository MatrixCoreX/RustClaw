use rusqlite::params;
use serde_json::Value;

use crate::{AppState, ClaimedTask};

#[derive(Debug, Clone)]
struct ExecutionAnchor {
    ts: String,
    capability: String,
    action: Option<String>,
    data: Option<Value>,
    evidence_locators: Vec<String>,
    artifact_refs: Vec<String>,
    request: String,
    result: String,
}

#[derive(Debug)]
struct CapabilityAnchorProjection {
    capability: String,
    action: Option<String>,
    data: Option<Value>,
    evidence_locators: Vec<String>,
    artifact_refs: Vec<String>,
}

fn query_recent_execution_rows(
    state: &AppState,
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
        let row = row?;
        if should_skip_recent_execution_row(state, &row.0, &row.2) {
            continue;
        }
        out.push(row);
    }
    Ok(out)
}

fn result_json_primary_text(result_json: &str) -> Option<String> {
    let parsed = serde_json::from_str::<Value>(result_json).ok()?;
    parsed
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| {
            parsed
                .get("messages")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
}

fn should_skip_recent_execution_row(state: &AppState, kind: &str, result_json: &str) -> bool {
    if kind != "ask" {
        return false;
    }
    // §7.2: 集合化 fallback 比对 —— 旧 super-fallback 与新 7 个 source 文案任一命中
    // 都跳过，不让历史 fallback turn 进 recent_execution 上下文拼接。
    result_json_primary_text(result_json)
        .map(|text| crate::fallback::is_known_clarify_fallback_text(state, &text))
        .unwrap_or(false)
}

pub(crate) fn build_recent_execution_context(
    state: &AppState,
    task: &ClaimedTask,
    limit: usize,
) -> String {
    let rows = load_recent_execution_rows(state, task, limit);
    if rows.is_empty() {
        return "<none>".to_string();
    }
    render_recent_execution_context(&rows, limit)
}

pub(crate) fn build_recent_execution_anchor_context(
    state: &AppState,
    task: &ClaimedTask,
) -> String {
    let rows = load_recent_execution_rows(state, task, 4);
    if rows.is_empty() {
        return "<none>".to_string();
    }
    render_recent_execution_anchor_context(&rows)
}

fn load_recent_execution_rows(
    state: &AppState,
    task: &ClaimedTask,
    limit: usize,
) -> Vec<(String, String, String, String)> {
    let user_key = task
        .user_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let legacy_chat_id = user_key
        .map(crate::stable_i64_from_key)
        .filter(|legacy| *legacy != task.chat_id);
    let db = match state.core.db.get() {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let rows = match query_recent_execution_rows(
        state,
        &db,
        task.user_id,
        task.chat_id,
        user_key,
        limit,
    ) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    if !rows.is_empty() {
        return rows;
    }

    if let Some(legacy_chat_id) = legacy_chat_id {
        return query_recent_execution_rows(
            state,
            &db,
            task.user_id,
            legacy_chat_id,
            user_key,
            limit,
        )
        .unwrap_or_default();
    }

    Vec::new()
}

fn render_recent_execution_context(
    rows: &[(String, String, String, String)],
    limit: usize,
) -> String {
    let mut sections = Vec::new();
    let anchor_block = render_recent_execution_anchor_context(rows);
    if anchor_block != "<none>" {
        sections.push(anchor_block);
    }

    let mut items = Vec::new();
    for (kind, payload_json, result_json, updated_at) in rows.iter().take(limit) {
        let req = task_payload_summary(kind, payload_json);
        let result = task_result_summary(result_json);
        items.push(format!(
            "- ts={updated_at} kind={kind} request={} result={}",
            truncate_snippet(&req, 220),
            truncate_snippet(&result, 320)
        ));
    }
    if !items.is_empty() {
        sections.push(format!("### RECENT_EXECUTION_EVENTS\n{}", items.join("\n")));
    }

    if sections.is_empty() {
        "<none>".to_string()
    } else {
        sections.join("\n\n")
    }
}

fn render_recent_execution_anchor_context(rows: &[(String, String, String, String)]) -> String {
    let Some(anchor) = rows
        .iter()
        .find_map(|(kind, payload_json, result_json, updated_at)| {
            extract_execution_anchor(kind, payload_json, result_json, updated_at)
        })
    else {
        return "<none>".to_string();
    };

    let mut lines = vec![
        format!("- latest_succeeded_ts={}", anchor.ts),
        format!("- latest_capability={}", anchor.capability),
    ];
    if let Some(action) = anchor.action.as_deref() {
        lines.push(format!("- latest_action={action}"));
    }
    if let Some(data) = anchor.data.as_ref() {
        lines.push(format!(
            "- latest_capability_data={}",
            truncate_snippet(&data.to_string(), 320)
        ));
    }
    if !anchor.evidence_locators.is_empty() {
        lines.push(format!(
            "- latest_evidence_locators={}",
            Value::Array(
                anchor
                    .evidence_locators
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect()
            )
        ));
    }
    if !anchor.artifact_refs.is_empty() {
        lines.push(format!(
            "- latest_artifact_refs={}",
            Value::Array(
                anchor
                    .artifact_refs
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect()
            )
        ));
    }
    lines.push(format!(
        "- latest_request={}",
        truncate_snippet(&anchor.request, 180)
    ));
    lines.push(format!(
        "- latest_result={}",
        truncate_snippet(&anchor.result, 220)
    ));
    lines.push(
        "- anchor_rule=Use this anchor only as supporting evidence for genuinely short follow-up requests. Reuse it only when the current request or recent context already binds exactly one concrete target of the correct type. Do not let this anchor override a needed clarification, and do not treat an artifact-type noun alone as a concrete target.".to_string(),
    );
    format!("### RECENT_EXECUTION_ANCHOR\n{}", lines.join("\n"))
}

fn extract_execution_anchor(
    kind: &str,
    payload_json: &str,
    result_json: &str,
    updated_at: &str,
) -> Option<ExecutionAnchor> {
    let request = task_payload_summary(kind, payload_json);
    let result = task_result_summary(result_json);
    let payload = serde_json::from_str::<Value>(payload_json).ok();
    let result_value = serde_json::from_str::<Value>(result_json).ok();
    let projection = result_value
        .as_ref()
        .and_then(capability_anchor_from_result)
        .or_else(|| capability_anchor_from_run_skill_payload(kind, payload.as_ref()))?;

    Some(ExecutionAnchor {
        ts: updated_at.to_string(),
        capability: projection.capability,
        action: projection.action,
        data: projection.data,
        evidence_locators: projection.evidence_locators,
        artifact_refs: projection.artifact_refs,
        request,
        result,
    })
}

fn capability_anchor_from_result(result: &Value) -> Option<CapabilityAnchorProjection> {
    const POINTERS: [&str; 3] = [
        "/task_journal/trace/capability_results",
        "/final_result_json/task_journal/trace/capability_results",
        "/result/task_journal/trace/capability_results",
    ];
    POINTERS
        .iter()
        .filter_map(|pointer| result.pointer(pointer).and_then(Value::as_array))
        .flat_map(|items| items.iter().rev())
        .find_map(capability_anchor_from_envelope)
}

fn capability_anchor_from_envelope(value: &Value) -> Option<CapabilityAnchorProjection> {
    if value.get("status").and_then(Value::as_str) != Some("ok") {
        return None;
    }
    let capability = machine_ref(value.get("capability")?.as_str()?)?.to_string();
    let action = value
        .get("action")
        .and_then(Value::as_str)
        .and_then(machine_ref)
        .map(ToString::to_string);
    let data = value
        .get("data")
        .filter(|data| !data.is_null() && !data.as_object().is_some_and(|map| map.is_empty()))
        .map(redacted_context_value);
    let evidence_locators = value
        .get("evidence")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("locator").and_then(Value::as_str))
        .filter_map(redacted_reference)
        .collect();
    let artifact_refs = value
        .get("artifacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            ["path", "uri", "id"]
                .into_iter()
                .find_map(|field| item.get(field).and_then(Value::as_str))
        })
        .filter_map(redacted_reference)
        .collect();
    Some(CapabilityAnchorProjection {
        capability,
        action,
        data,
        evidence_locators,
        artifact_refs,
    })
}

fn capability_anchor_from_run_skill_payload(
    kind: &str,
    payload: Option<&Value>,
) -> Option<CapabilityAnchorProjection> {
    if kind != "run_skill" {
        return None;
    }
    let payload = payload?;
    let capability = machine_ref(payload.get("skill_name")?.as_str()?)?.to_string();
    let args = payload
        .get("args")
        .filter(|args| args.is_object())
        .map(redacted_context_value);
    let action = args
        .as_ref()
        .and_then(|args| args.get("action"))
        .and_then(Value::as_str)
        .and_then(machine_ref)
        .map(ToString::to_string);
    Some(CapabilityAnchorProjection {
        capability,
        action,
        data: args,
        evidence_locators: Vec::new(),
        artifact_refs: Vec::new(),
    })
}

fn machine_ref(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')))
    .then_some(value)
}

fn redacted_reference(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| crate::visible_text::redact_sensitive_text(value))
}

fn redacted_context_value(value: &Value) -> Value {
    let redacted = crate::visible_text::sanitize_user_visible_text(&value.to_string());
    serde_json::from_str(&redacted).unwrap_or(Value::String(redacted))
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
            let skill = v
                .get("skill_name")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown");
            let args = v
                .get("args")
                .map(redacted_context_value)
                .unwrap_or(Value::Null);
            format!("run_skill:{skill} args={}", args)
        }
        _ => payload_json.to_string(),
    }
}

fn task_result_summary(result_json: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(result_json) else {
        return sanitize_result_summary(result_json);
    };
    let text = v
        .get("text")
        .and_then(|x| x.as_str())
        .or_else(|| v.as_str())
        .unwrap_or(result_json)
        .to_string();
    sanitize_result_summary(&text)
}

fn sanitize_result_summary(text: &str) -> String {
    crate::visible_text::sanitize_user_visible_text(text)
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

#[cfg(test)]
#[path = "routing_context_tests.rs"]
mod tests;
