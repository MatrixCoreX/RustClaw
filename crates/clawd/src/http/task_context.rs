use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use claw_core::types::ApiResponse;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{repo::TaskDeliveryRecord, AppState};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecallRequest {
    pub context_ref: String,
    pub field_paths: Vec<String>,
}

pub(crate) fn context_ref(task_id: &str) -> String {
    format!("task-context:v1:{task_id}")
}

pub(crate) async fn recall(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(current_id): Path<Uuid>,
    Json(request): Json<RecallRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let result = (|| -> Result<Value, &'static str> {
        let key = crate::auth_key_from_headers(&headers).ok_or("auth_key_invalid")?;
        let identity = crate::resolve_auth_identity_by_key(&state, key)
            .map_err(|_| "auth_lookup_failed")?
            .ok_or("auth_key_invalid")?;
        let source_id = request
            .context_ref
            .strip_prefix("task-context:v1:")
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or("context_reference_invalid")?;
        if source_id == current_id {
            return Err("context_reference_invalid");
        }
        let load = |id: Uuid| {
            crate::repo::get_task_delivery_record(&state, &id.to_string())
                .map_err(|_| "context_lookup_failed")?
                .ok_or("context_reference_unavailable")
        };
        let current = load(current_id)?;
        if current.task.user_key.as_deref() != Some(identity.user_key.as_str())
            || current.owner_principal_id.as_deref() != Some(identity.principal_id.as_str())
        {
            return Err("context_reference_unauthorized");
        }
        let source = load(source_id)?;
        if !same_scope(&current, &source) {
            return Err("context_reference_unauthorized");
        }
        if current.status != "running"
            || !matches!(
                source.status.as_str(),
                "succeeded" | "failed" | "canceled" | "timeout"
            )
        {
            return Err("context_reference_not_terminal");
        }
        select_context(&source, &request)
    })();
    match result {
        Ok(value) => crate::api_ok(value),
        Err(code) => crate::api_err(
            match code {
                "auth_key_invalid" => StatusCode::UNAUTHORIZED,
                "context_reference_unauthorized" | "context_field_forbidden" => {
                    StatusCode::FORBIDDEN
                }
                "context_reference_unavailable" => StatusCode::NOT_FOUND,
                "auth_lookup_failed" | "context_lookup_failed" => StatusCode::INTERNAL_SERVER_ERROR,
                "context_value_too_large" => StatusCode::PAYLOAD_TOO_LARGE,
                _ => StatusCode::BAD_REQUEST,
            },
            code,
        ),
    }
}

fn same_scope(current: &TaskDeliveryRecord, source: &TaskDeliveryRecord) -> bool {
    let a = &current.task;
    let b = &source.task;
    let conversation = |payload: &str| {
        serde_json::from_str::<Value>(payload)
            .ok()
            .and_then(|v| v.get("conversation_id").cloned())
    };
    current.owner_principal_id.is_some()
        && current.owner_principal_id == source.owner_principal_id
        && a.user_key.is_some()
        && b.user_key.is_some()
        && a.user_id == b.user_id
        && a.chat_id == b.chat_id
        && a.channel == b.channel
        && a.external_user_id == b.external_user_id
        && a.external_chat_id == b.external_chat_id
        && conversation(&a.payload_json) == conversation(&b.payload_json)
}

fn select_context(
    source: &TaskDeliveryRecord,
    request: &RecallRequest,
) -> Result<Value, &'static str> {
    if request.field_paths.is_empty() || request.field_paths.len() > 8 {
        return Err("context_fields_invalid");
    }
    let payload: Value =
        serde_json::from_str(&source.task.payload_json).map_err(|_| "context_payload_invalid")?;
    let result = source
        .result_json
        .as_ref()
        .ok_or("context_result_unavailable")?;
    let capabilities = [
        "/task_journal/trace/capability_results",
        "/final_result_json/task_journal/trace/capability_results",
    ]
    .iter()
    .find_map(|path| result.pointer(path).and_then(Value::as_array));
    let mut fields = serde_json::Map::new();
    for path in &request.field_paths {
        if path == "/capabilities" {
            let index = json!(capabilities.into_iter().flatten().take(64).enumerate().map(|(index, item)| json!({
                "index": index, "capability": item.get("capability"), "status": item.get("status"),
                "args_ref": format!("/capabilities/{index}/args"), "data_ref": format!("/capabilities/{index}/data"),
            })).collect::<Vec<_>>());
            check_json_size(&index)?;
            fields.insert(path.clone(), index);
            continue;
        }
        if !path.starts_with('/') || path.len() > 256 {
            return Err("context_fields_invalid");
        }
        if path
            .split('/')
            .any(|part| sensitive_key(&part.replace("~1", "/").replace("~0", "~")))
        {
            return Err("context_field_forbidden");
        }
        let value =
            historical_field(&payload, capabilities, path).ok_or("context_field_unavailable")?;
        check_json_size(value)?;
        fields.insert(path.clone(), redact(value, 0)?);
    }
    let value = json!({"schema_version": 1, "action": "recall_context", "status": "ok", "message_key": "task_control.recall_context.ok",
        "field_value": {"source_task_id": source.task.task_id, "source_status": source.status}, "source_task_id": source.task.task_id,
        "source_status": source.status, "source_digest": source_digest(result)?,
        "instruction_authority": "none", "current_execution_evidence": false,
        "scope": "prior_task_context", "fields": fields});
    check_json_size(&value)?;
    Ok(value)
}

fn historical_field<'a>(
    payload: &'a Value,
    capabilities: Option<&'a Vec<Value>>,
    path: &str,
) -> Option<&'a Value> {
    if path == "/request" {
        return payload.get("text").or_else(|| payload.get("user_message"));
    }
    let mut parts = path.strip_prefix("/capabilities/")?.splitn(3, '/');
    let index = parts.next()?.parse::<usize>().ok().filter(|i| *i < 64)?;
    let capability = capabilities?.get(index)?;
    let field = match parts.next()? {
        "args" => capability.pointer("/provenance/continuation_args"),
        name @ ("data" | "capability" | "status") => capability.get(name),
        _ => None,
    }?;
    match parts.next() {
        Some(suffix) => field.pointer(&format!("/{suffix}")),
        None => Some(field),
    }
}

struct SizeWriter(usize);
impl std::io::Write for SizeWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        if self.0 > 24 * 1024 {
            return Err(std::io::Error::other("context_value_too_large"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn check_json_size(value: &Value) -> Result<(), &'static str> {
    serde_json::to_writer(SizeWriter(0), value).map_err(|_| "context_value_too_large")
}
struct HashWriter(Sha256);
impl std::io::Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn source_digest(value: &Value) -> Result<String, &'static str> {
    let mut writer = HashWriter(Sha256::new());
    serde_json::to_writer(&mut writer, value).map_err(|_| "context_payload_invalid")?;
    Ok(format!("{:x}", writer.0.finalize()))
}

fn redact(value: &Value, depth: usize) -> Result<Value, &'static str> {
    if depth > 12 {
        return Err("context_value_too_deep");
    }
    Ok(match value {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .filter(|(key, _)| !sensitive_key(key))
                .map(|(key, value)| Ok::<_, &'static str>((key.clone(), redact(value, depth + 1)?)))
                .collect::<Result<_, _>>()?,
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| redact(v, depth + 1))
                .collect::<Result<_, _>>()?,
        ),
        Value::String(text) => Value::String(crate::visible_text::redact_sensitive_text(text)),
        other => other.clone(),
    })
}

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.split(['_', '-']).any(|part| {
        matches!(
            part,
            "confirm"
                | "confirmation"
                | "approved"
                | "approval"
                | "permission"
                | "permissions"
                | "policy"
                | "yolo"
                | "secret"
                | "password"
                | "token"
                | "authorization"
                | "cookie"
                | "credential"
                | "credentials"
                | "grant"
                | "private"
                | "headers"
        )
    }) || matches!(
        key.as_str(),
        "api_key" | "user_key" | "principal_id" | "apikey" | "privatekey" | "accesskey"
    )
}

#[cfg(test)]
#[path = "task_context_tests.rs"]
mod tests;
