use rusqlite::params;
use serde_json::{json, Value};

use crate::{AppState, ClaimedTask};

#[path = "routing_context_projection.rs"]
mod projection;
pub(crate) use projection::bounded_prior_context_value;

#[derive(Debug, Clone)]
struct ExecutionAnchor {
    source_task_id: String,
    ts: String,
    task_status: String,
    attempt_status: String,
    capability: String,
    action: Option<String>,
    data: Option<Value>,
    preview: Value,
    error_code: Option<String>,
    message_key: Option<String>,
    retryable: Option<bool>,
    evidence_count: usize,
    artifact_count: usize,
    request: String,
    result: String,
}

#[derive(Debug)]
struct CapabilityAnchorProjection {
    status: String,
    capability: String,
    action: Option<String>,
    data: Option<Value>,
    preview: Value,
    error_code: Option<String>,
    message_key: Option<String>,
    retryable: Option<bool>,
    evidence_locators: Vec<String>,
    artifact_refs: Vec<String>,
}

type RecentExecutionRow = (String, String, String, String, String, String);

fn query_recent_execution_rows(
    state: &AppState,
    db: &rusqlite::Connection,
    user_id: i64,
    chat_id: i64,
    channel: &str,
    user_key: Option<&str>,
    conversation_id: Option<&str>,
    limit: usize,
) -> anyhow::Result<Vec<RecentExecutionRow>> {
    let mut stmt = db.prepare(
        "SELECT task_id, kind, payload_json,
                COALESCE(NULLIF(result_json, ''),
                    json_object(
                        'status', status,
                        'text', '[prior_task_' || status || ']'
                    )
                ),
                updated_at,
                status
         FROM tasks
         WHERE user_id = ?1
           AND chat_id = ?2
           AND channel = ?6
           AND status IN ('succeeded', 'failed', 'canceled', 'timeout')
           AND (
             (?3 IS NOT NULL AND user_key = ?3)
             OR (?3 IS NULL AND (user_key IS NULL OR TRIM(user_key) = ''))
           )
           AND (
             ?4 IS NULL
             OR (
               json_valid(payload_json)
               AND json_type(payload_json, '$.conversation_id') = 'text'
               AND json_extract(payload_json, '$.conversation_id') = ?4
             )
           )
         ORDER BY CAST(updated_at AS INTEGER) DESC, rowid DESC
         LIMIT ?5",
    )?;
    let rows = stmt.query_map(
        params![
            user_id,
            chat_id,
            user_key,
            conversation_id,
            limit as i64,
            channel
        ],
        |row| {
            let task_id: String = row.get(0)?;
            let kind: String = row.get(1)?;
            let payload_json: String = row.get(2)?;
            let result_json: String = row.get(3)?;
            let updated_at: String = row.get(4)?;
            let status: String = row.get(5)?;
            Ok((task_id, kind, payload_json, result_json, updated_at, status))
        },
    )?;
    let mut out = Vec::new();
    for row in rows {
        let mut row = row?;
        if should_skip_recent_execution_row(state, &row.1, &row.3) {
            // Keep the newer turn as a structural recency barrier so an older
            // capability target cannot become the apparent latest anchor.
            // Only omit the transient assistant reply from prompt-visible data.
            row.3 = r#"{"text":"[transient_assistant_reply_omitted]"}"#.to_string();
        } else if row.5 != "succeeded" {
            row.3 = compact_prior_terminal_result(&row.3, &row.5);
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

fn should_skip_recent_execution_row(_state: &AppState, kind: &str, result_json: &str) -> bool {
    if kind != "ask" {
        return false;
    }
    result_json_primary_text(result_json)
        .map(|text| crate::fallback::is_machine_fallback_payload(&text))
        .unwrap_or(false)
}

fn compact_prior_terminal_result(result_json: &str, task_status: &str) -> String {
    let parsed = serde_json::from_str::<Value>(result_json).ok();
    let projection = parsed.as_ref().and_then(capability_anchor_from_result);
    let mut attempt = serde_json::Map::new();
    if let Some(projection) = projection {
        attempt.insert("schema_version".to_string(), json!(1));
        attempt.insert("status".to_string(), json!(projection.status));
        attempt.insert("capability".to_string(), json!(projection.capability));
        if let Some(action) = projection.action {
            attempt.insert("action".to_string(), json!(action));
        }
        if let Some(data) = projection.data {
            attempt.insert("data".to_string(), data);
        }
        if projection.error_code.is_some()
            || projection.message_key.is_some()
            || projection.retryable.is_some()
        {
            attempt.insert(
                "error".to_string(),
                json!({
                    "code": projection.error_code,
                    "message_key": projection.message_key,
                    "retryable": projection.retryable,
                }),
            );
        }
        if let Some(retryable) = projection.retryable {
            attempt.insert("retry".to_string(), json!({"retryable": retryable}));
        }
    }
    let text = truncate_snippet(
        &redact_prior_private_artifact_paths(&task_result_summary(result_json)),
        800,
    );
    json!({
        "status": task_status,
        "text": if text.is_empty() {
            format!("[prior_task_{task_status}]")
        } else {
            text
        },
        "task_journal": {
            "trace": {
                "prior_context_compacted": true,
                "capability_results": if attempt.is_empty() {
                    Vec::<Value>::new()
                } else {
                    vec![Value::Object(attempt)]
                }
            }
        }
    })
    .to_string()
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
    let refs = rows
        .iter()
        .map(|row| crate::http::task_context::context_ref(&row.0))
        .collect::<Vec<_>>();
    format!(
        "{}\n### AUTHORIZED_CONTEXT_REFERENCES\n{}",
        render_recent_execution_anchor_context(&rows),
        json!({"capability": "task_control.recall_context", "context_refs": refs,
            "field_paths": ["/request", "/capabilities", "/capabilities/0/args", "/capabilities/0/data"],
            "instruction_authority": "none", "current_execution_evidence": false})
    )
}

fn load_recent_execution_rows(
    state: &AppState,
    task: &ClaimedTask,
    limit: usize,
) -> Vec<RecentExecutionRow> {
    let conversation_id = crate::conversation_state::task_conversation_id(task);
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
        &task.channel,
        user_key,
        conversation_id.as_deref(),
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
            &task.channel,
            user_key,
            conversation_id.as_deref(),
            limit,
        )
        .unwrap_or_default();
    }

    Vec::new()
}

fn render_recent_execution_context(rows: &[RecentExecutionRow], limit: usize) -> String {
    let mut sections = Vec::new();
    let anchor_block = render_recent_execution_anchor_context(rows);
    if anchor_block != "<none>" {
        sections.push(anchor_block);
    }

    let mut items = Vec::new();
    for (task_id, kind, payload_json, result_json, updated_at, status) in rows.iter().take(limit) {
        let req = task_payload_summary(kind, payload_json);
        let result = if status == "succeeded" {
            task_result_summary(result_json)
        } else {
            format!("[prior_task_{status}]")
        };
        items.push(format!(
            "- source_task_id={task_id} scope=prior_task_context ts={updated_at} kind={kind} terminal_status={status} request={} result={}",
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

fn render_recent_execution_anchor_context(rows: &[RecentExecutionRow]) -> String {
    let Some((task_id, kind, payload_json, result_json, updated_at, status)) = rows.first() else {
        return "<none>".to_string();
    };
    let Some(anchor) =
        extract_execution_anchor(task_id, kind, payload_json, result_json, updated_at, status)
    else {
        return "<none>".to_string();
    };

    let completed_success = anchor.task_status == "succeeded" && anchor.attempt_status == "ok";
    let mut lines = vec![
        format!("- source_task_id={}", anchor.source_task_id),
        "- scope=prior_task_context".to_string(),
        format!("- prior_task_status={}", anchor.task_status),
        format!("- prior_attempt_status={}", anchor.attempt_status),
        format!("- prior_attempted_ts={}", anchor.ts),
        format!("- prior_capability={}", anchor.capability),
    ];
    if let Some(action) = anchor.action.as_deref() {
        lines.push(format!("- prior_action={action}"));
    }
    if let Some(data) = anchor.data.as_ref() {
        lines.push(format!("- prior_capability_summary={}", data));
    }
    let mut preview = anchor.preview.clone();
    preview["recovery"] = json!({"capability":"task_control.recall_context",
        "context_ref":crate::http::task_context::context_ref(&anchor.source_task_id),
        "field_paths":["/capabilities"]});
    lines.push(format!("- prior_context_projection={preview}"));
    if let Some(error_code) = anchor.error_code.as_deref() {
        lines.push(format!("- prior_error_code={error_code}"));
    }
    if let Some(message_key) = anchor.message_key.as_deref() {
        lines.push(format!("- prior_message_key={message_key}"));
    }
    if let Some(retryable) = anchor.retryable {
        lines.push(format!("- prior_retryable={retryable}"));
    }
    if anchor.evidence_count > 0 {
        lines.push(format!("- prior_evidence_count={}", anchor.evidence_count));
    }
    if anchor.artifact_count > 0 {
        lines.push(format!("- prior_artifact_count={}", anchor.artifact_count));
    }
    lines.push(format!(
        "- prior_request={}",
        truncate_snippet(&anchor.request, 400)
    ));
    lines.push(format!(
        "- prior_result={}",
        truncate_snippet(&anchor.result, 220)
    ));
    lines.push(if completed_success {
        "- anchor_rule=prior_task_background_only".to_string()
    } else {
        "- anchor_rule=prior_attempt_semantic_delta".to_string()
    });
    let heading = if completed_success {
        "PRIOR_TASK_EXECUTION_ANCHOR"
    } else {
        "PRIOR_TASK_ATTEMPT_ANCHOR"
    };
    format!("### {heading}\n{}", lines.join("\n"))
}

fn extract_execution_anchor(
    source_task_id: &str,
    kind: &str,
    payload_json: &str,
    result_json: &str,
    updated_at: &str,
    task_status: &str,
) -> Option<ExecutionAnchor> {
    let request = task_payload_summary(kind, payload_json);
    let result = task_result_summary(result_json);
    let payload = serde_json::from_str::<Value>(payload_json).ok();
    let result_value = serde_json::from_str::<Value>(result_json).ok();
    let projection = result_value
        .as_ref()
        .and_then(capability_anchor_from_result)
        .or_else(|| {
            capability_anchor_from_run_skill_payload(kind, payload.as_ref(), task_status)
        })?;

    Some(ExecutionAnchor {
        source_task_id: source_task_id.to_string(),
        ts: updated_at.to_string(),
        task_status: machine_ref(task_status).unwrap_or("unknown").to_string(),
        attempt_status: projection.status,
        capability: projection.capability,
        action: projection.action,
        data: projection.data,
        preview: projection.preview,
        error_code: projection.error_code,
        message_key: projection.message_key,
        retryable: projection.retryable,
        evidence_count: projection.evidence_locators.len(),
        artifact_count: projection.artifact_refs.len(),
        request,
        result: redact_prior_private_artifact_paths(&result),
    })
}

fn capability_anchor_from_result(result: &Value) -> Option<CapabilityAnchorProjection> {
    const POINTERS: [&str; 3] = [
        "/task_journal/trace/capability_results",
        "/final_result_json/task_journal/trace/capability_results",
        "/result/task_journal/trace/capability_results",
    ];
    let (trace, results) = POINTERS.iter().find_map(|pointer| {
        let trace = result.pointer(pointer.trim_end_matches("/capability_results"))?;
        Some((trace, trace.get("capability_results")?.as_array()?))
    })?;
    let (selected_index, mut projection) = results
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, value)| capability_anchor_from_envelope(value).map(|p| (index, p)))?;
    let prior = || {
        results[..selected_index].iter().rev().filter_map(|value| {
            let capability = machine_ref(value.get("capability")?.as_str()?)?;
            if capability_is_runtime_control(capability) {
                return None;
            }
            Some(json!({
                "capability": capability,
                "status": machine_ref(value.get("status")?.as_str()?)?,
                "attempted_effect": value.get("effect").and_then(Value::as_str).and_then(machine_ref),
            }))
        })
    };
    let prior_total = prior().count();
    let prior_steps = prior().take(4).collect::<Vec<_>>();
    projection.preview["prior_steps"] = json!({"total":prior_total,
        "returned":prior_steps.len(),"truncated":prior_total > prior_steps.len()});
    projection.preview["source_trace_compacted"] = json!(
        trace
            .pointer("/trace_storage/truncated")
            .and_then(Value::as_bool)
            == Some(true)
            || trace
                .get("prior_context_compacted")
                .and_then(Value::as_bool)
                == Some(true)
    );
    if !prior_steps.is_empty() {
        let data = projection.data.get_or_insert_with(|| json!({}));
        data.as_object_mut()?
            .insert("prior_steps".into(), json!(prior_steps));
    }
    Some(projection)
}

fn capability_is_runtime_control(capability: &str) -> bool {
    capability.starts_with("task.plan_")
}

fn capability_anchor_from_envelope(value: &Value) -> Option<CapabilityAnchorProjection> {
    let status = machine_ref(value.get("status")?.as_str()?)?.to_string();
    let capability = machine_ref(value.get("capability")?.as_str()?)?.to_string();
    if capability_is_runtime_control(&capability) {
        return None;
    }
    let action = value
        .get("action")
        .and_then(Value::as_str)
        .and_then(machine_ref)
        .map(ToString::to_string);
    let (projected_data, data_truncated) = value
        .get("data")
        .map(bounded_prior_context_value)
        .unwrap_or((None, false));
    let mut context_data = match projected_data {
        Some(Value::Object(fields)) => fields,
        Some(data) => serde_json::Map::from_iter([("observation".into(), data)]),
        None => serde_json::Map::new(),
    };
    let (args, mut args_truncated) = value
        .pointer("/provenance/continuation_args")
        .map(bounded_prior_context_value)
        .unwrap_or((None, false));
    args_truncated |= value
        .pointer("/provenance/continuation_args_projection/truncated")
        .and_then(Value::as_bool)
        == Some(true);
    if let Some(args) = args {
        context_data.insert("prior_args".into(), args);
    }
    let data = (!context_data.is_empty()).then_some(Value::Object(context_data));
    let error_code = value
        .pointer("/error/code")
        .and_then(Value::as_str)
        .and_then(machine_ref)
        .map(ToString::to_string);
    let message_key = value
        .pointer("/error/message_key")
        .and_then(Value::as_str)
        .and_then(machine_ref)
        .map(ToString::to_string);
    let retryable = value
        .pointer("/retry/retryable")
        .and_then(Value::as_bool)
        .or_else(|| value.pointer("/error/retryable").and_then(Value::as_bool));
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
        status,
        capability,
        action,
        data,
        preview: json!({"schema_version":1,"is_preview":true,
            "data_truncated":data_truncated,"args_truncated":args_truncated}),
        error_code,
        message_key,
        retryable,
        evidence_locators,
        artifact_refs,
    })
}

fn capability_anchor_from_run_skill_payload(
    kind: &str,
    payload: Option<&Value>,
    task_status: &str,
) -> Option<CapabilityAnchorProjection> {
    if kind != "run_skill" {
        return None;
    }
    let payload = payload?;
    let capability = machine_ref(payload.get("skill_name")?.as_str()?)?.to_string();
    let (args, args_truncated) = payload
        .get("args")
        .filter(|args| args.is_object())
        .map(bounded_prior_context_value)
        .unwrap_or((None, false));
    let action = args
        .as_ref()
        .and_then(|args| args.get("action"))
        .and_then(Value::as_str)
        .and_then(machine_ref)
        .map(ToString::to_string);
    Some(CapabilityAnchorProjection {
        status: if task_status == "succeeded" {
            "ok".to_string()
        } else {
            "error".to_string()
        },
        capability,
        action,
        data: args,
        preview: json!({"schema_version":1,"is_preview":true,"args_truncated":args_truncated,
            "source_kind":"invocation_payload","current_execution_evidence":false}),
        error_code: None,
        message_key: None,
        retryable: None,
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

fn strip_prior_task_locators(value: Value) -> Value {
    match value {
        Value::Array(items) => {
            Value::Array(items.into_iter().map(strip_prior_task_locators).collect())
        }
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .filter(|(key, _)| !is_prior_task_locator_key(key))
                .map(|(key, value)| (key, strip_prior_task_locators(value)))
                .collect(),
        ),
        Value::String(value) if is_private_artifact_locator(&value) => {
            Value::String("[prior_task_private_artifact]".to_string())
        }
        other => other,
    }
}

fn is_prior_task_locator_key(key: &str) -> bool {
    let key = key.trim().to_ascii_lowercase();
    matches!(
        key.as_str(),
        "path"
            | "credentials"
            | "authorization"
            | "permissions"
            | "confirmation"
            | "confirmations"
            | "policy_grant"
            | "grant"
            | "cookie"
            | "cookies"
            | "password"
            | "secret"
            | "api_key"
            | "access_token"
            | "private_key"
            | "paths"
            | "uri"
            | "url"
            | "locator"
            | "artifact_ref"
            | "artifact_refs"
            | "processing_outputs"
            | "saved_files"
            | "input_value"
            | "input_value_artifact_ref"
            | "fallback_input_value"
            | "fallback_input_value_artifact_ref"
    ) || key.ends_with("_path")
        || key.ends_with("_token")
        || key.ends_with("_secret")
        || key.ends_with("_password")
        || key.ends_with("_key")
        || key.ends_with("_paths")
        || key.ends_with("_uri")
        || key.ends_with("_url")
}

pub(crate) fn is_private_artifact_locator(value: &str) -> bool {
    let normalized = value.trim().replace('\\', "/");
    normalized.contains("/.agent-runtime/artifacts/")
        || normalized.starts_with(".agent-runtime/artifacts/")
        || normalized.contains("/artifacts/skill-invocations/")
}

pub(crate) fn redact_prior_private_artifact_paths(text: &str) -> String {
    text.split_whitespace()
        .map(|token| {
            if is_private_artifact_locator(token) {
                "[prior_task_private_artifact]"
            } else {
                token
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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
        .or_else(|| v.get("messages")?.as_array()?.first()?.as_str())
        .or_else(|| v.pointer("/task_journal/summary/final_answer")?.as_str())
        .unwrap_or("")
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
