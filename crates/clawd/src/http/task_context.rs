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

use crate::task_context_privacy::{redact, sensitive_key};
use crate::{repo::TaskDeliveryRecord, AppState};

#[path = "task_context_cursor.rs"]
mod cursor;
#[path = "task_context_selection.rs"]
mod selection;
#[cfg(test)]
use selection::select_context;
use selection::select_context_with_capabilities;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecallRequest {
    pub context_ref: String,
    pub field_paths: Vec<String>,
    pub cursor: Option<String>,
    pub page_size: Option<usize>,
    pub source_digest: Option<String>,
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
    let result = tokio::task::spawn_blocking(move || -> Result<Value, &'static str> {
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
        if !same_scope(&current, &source)
            || state.task_agent_id(&current.task) != state.task_agent_id(&source.task)
        {
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
        let binding = source_digest(&json!({
            "current_task_id": current_id,
            "principal_id": identity.principal_id,
            "user_key": identity.user_key,
            "task": {"user_id": current.task.user_id, "chat_id": current.task.chat_id,
                "channel": current.task.channel, "external_user_id": current.task.external_user_id,
                "external_chat_id": current.task.external_chat_id,
                "payload_json": current.task.payload_json},
            "agent_id": state.task_agent_id(&current.task),
        }))?;
        let canonical = if (1..=8).contains(&request.field_paths.len())
            && request
                .field_paths
                .iter()
                .any(|path| path == "/capabilities" || path.starts_with("/capabilities/"))
        {
            let result = source
                .result_json
                .as_ref()
                .ok_or("context_result_unavailable")?;
            let journal = selection::source_journal(result);
            if journal
                .and_then(|j| j.get("trace"))
                .is_some_and(crate::task_journal::retained_capability_results_complete)
            {
                None
            } else if let Some(reference) = journal.and_then(|j| j.get("canonical_capabilities")) {
                Some(crate::task_context_archive::read_selected(
                    &state.skill_rt.workspace_root,
                    &source.task.task_id,
                    reference,
                    &request.field_paths,
                )?)
            } else {
                None
            }
        } else {
            None
        };
        select_context_with_capabilities(
            &source,
            &request,
            &binding,
            chrono::Utc::now().timestamp(),
            canonical.as_ref(),
        )
    })
    .await
    .unwrap_or(Err("context_lookup_failed"));
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
                "context_source_compacted"
                | "context_field_compacted"
                | "context_archive_unavailable"
                | "context_archive_corrupt"
                | "context_archive_budget_exceeded"
                | "context_snapshot_changed" => StatusCode::CONFLICT,
                _ => StatusCode::BAD_REQUEST,
            },
            code,
        ),
    }
}

fn same_scope(current: &TaskDeliveryRecord, source: &TaskDeliveryRecord) -> bool {
    let a = &current.task;
    let b = &source.task;
    let conversation = |payload: &str| -> Option<Option<String>> {
        let value = serde_json::from_str::<Value>(payload).ok()?;
        value.as_object()?;
        match value.get("conversation_id") {
            None | Some(Value::Null) => Some(None),
            Some(Value::String(id)) => Some(Some(id.clone())),
            _ => None,
        }
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
        && conversation(&a.payload_json).is_some()
        && conversation(&a.payload_json) == conversation(&b.payload_json)
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

#[cfg(test)]
#[path = "task_context_tests.rs"]
mod tests;
