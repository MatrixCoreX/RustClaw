use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::browser_session_service::{BrowserSessionBinding, BrowserSessionError};
use crate::{AppState, ClaimedTask};

const OPEN_KEYS: &[&str] = &[
    "action",
    "url",
    "profile",
    "locale",
    "timezone",
    "viewport",
    "domains_allow",
    "domains_deny",
    "allow_proxy_synthetic_dns",
    "include_screenshot",
    "max_elements",
    "max_text_chars",
];
const PAGE_KEYS: &[&str] = &[
    "action",
    "session_id",
    "page_id",
    "expected_page_generation",
    "expected_snapshot_id",
    "target_ref",
    "url",
    "wait_until",
    "include_screenshot",
    "max_elements",
    "max_text_chars",
    "text_cursor",
    "expected_postcondition",
    "value",
    "key",
    "delta_y",
    "timeout_ms",
    "condition",
    "limit",
];

pub(super) async fn execute(
    state: &AppState,
    task: Option<&ClaimedTask>,
    map: &Map<String, Value>,
) -> Result<String, String> {
    let Some(task) = task else {
        return Err(super::builtin_error(
            "browser_session",
            "task_context_required",
            "browser_session.task_context_required",
            None,
            None,
            Some(json!({"field": "task_id"})),
        ));
    };
    let action = super::required_string(map, "action")?
        .trim()
        .to_ascii_lowercase();
    let allowed = if action == "session_open" {
        OPEN_KEYS
    } else {
        PAGE_KEYS
    };
    super::ensure_only_keys(map, allowed)?;
    validate_action_args(&action, map).map_err(|details| {
        super::builtin_error(
            "browser_session",
            "invalid_args",
            "browser_session.invalid_args",
            None,
            None,
            Some(details),
        )
    })?;

    let binding = session_binding(state, task);
    let service = state.core.browser_sessions.clone();
    let cancellation = state.worker.task_cancellation_token(&task.task_id);
    let mut bridge_input = Value::Object(map.clone());
    if let Some(object) = bridge_input.as_object_mut() {
        object.remove("action");
        object.remove("session_id");
    }
    let result = match action.as_str() {
        "session_open" => service.open(binding, bridge_input, cancellation).await,
        "session_close" => {
            service
                .close(
                    required_nonempty(map, "session_id")?,
                    &binding,
                    cancellation,
                )
                .await
        }
        "navigate" | "snapshot" | "screenshot" | "switch_page" | "click" | "type" | "select"
        | "press_key" | "scroll" | "wait_for" | "back" | "download" | "observe_debug" => {
            service
                .request(
                    required_nonempty(map, "session_id")?,
                    &binding,
                    &action,
                    bridge_input,
                    cancellation,
                )
                .await
        }
        _ => {
            return Err(super::builtin_error(
                "browser_session",
                "unsupported_action",
                "browser_session.unsupported_action",
                None,
                None,
                Some(json!({"action": action})),
            ));
        }
    }
    .map_err(structured_error)?;
    publish_browser_session_event(state, task, &action, &result);
    serde_json::to_string(&result).map_err(|error| {
        super::builtin_error(
            "browser_session",
            "serialization_failed",
            "browser_session.serialization_failed",
            None,
            None,
            Some(json!({"provider_error_kind": format!("{:?}", error.classify()).to_ascii_lowercase()})),
        )
    })
}

fn publish_browser_session_event(
    state: &AppState,
    task: &ClaimedTask,
    action: &str,
    projection: &Value,
) {
    let result = projection.get("result").unwrap_or(&Value::Null);
    let snapshot = result.get("snapshot").unwrap_or(result);
    let payload = json!({
        "schema_version": 1,
        "source": "browser_session",
        "data_only": true,
        "action": action,
        "session_id": projection.get("session_id"),
        "page_id": snapshot.get("page_id").or_else(|| result.get("page_id")),
        "page_generation": snapshot.get("page_generation").or_else(|| result.get("page_generation")),
        "snapshot_id": snapshot.get("snapshot_id").or_else(|| result.get("snapshot_id")),
        "action_receipt_id": result.get("action_receipt_id"),
        "current_url": snapshot.get("current_url").or_else(|| result.get("current_url")),
        "postcondition_status": result.get("postcondition_status"),
        "artifacts": result.get("artifacts").cloned().unwrap_or_else(|| json!([])),
        "untrusted_page_content_included": false,
    });
    if let Err(error) =
        crate::task_event_transport::publish_claimed_event(state, task, "browser_session", payload)
    {
        tracing::warn!(
            task_id = task.task_id,
            action,
            error = %error,
            "browser_session_event_publish_failed"
        );
    }
}

fn session_binding(state: &AppState, task: &ClaimedTask) -> BrowserSessionBinding {
    let mut actor_hasher = Sha256::new();
    for part in [
        task.channel.as_str(),
        task.user_key.as_deref().unwrap_or(""),
        task.external_user_id.as_deref().unwrap_or(""),
        task.external_chat_id.as_deref().unwrap_or(""),
    ] {
        actor_hasher.update(part.as_bytes());
        actor_hasher.update([0]);
    }
    actor_hasher.update(task.user_id.to_le_bytes());
    actor_hasher.update(task.chat_id.to_le_bytes());
    let actor_ref = format!("actor:{}", hex::encode(actor_hasher.finalize()));
    let views = state.get_skill_views_snapshot();
    let registry_digest = views
        .binding
        .registry_generation_digest
        .clone()
        .unwrap_or_else(|| "base-registry".to_string());
    let policy = crate::task_execution_policy::effective_policy_for_task(state, task);
    let policy_digest = hex::encode(Sha256::digest(
        serde_json::to_vec(&policy.to_machine_json()).unwrap_or_default(),
    ));
    BrowserSessionBinding {
        actor_ref,
        task_id: task.task_id.clone(),
        registry_generation: views.binding.registry_generation,
        registry_digest,
        policy_digest,
    }
}

fn validate_action_args(action: &str, map: &Map<String, Value>) -> Result<(), Value> {
    const ACTIONS: &[&str] = &[
        "session_open",
        "navigate",
        "snapshot",
        "screenshot",
        "switch_page",
        "click",
        "type",
        "select",
        "press_key",
        "scroll",
        "wait_for",
        "back",
        "download",
        "observe_debug",
        "session_close",
    ];
    if !ACTIONS.contains(&action) {
        return Err(json!({"field": "action", "value": action}));
    }
    if action != "session_open" {
        required_nonempty(map, "session_id").map_err(|_| json!({"field": "session_id"}))?;
    }
    if matches!(
        action,
        "navigate"
            | "snapshot"
            | "screenshot"
            | "switch_page"
            | "click"
            | "type"
            | "select"
            | "press_key"
            | "scroll"
            | "wait_for"
            | "back"
            | "download"
    ) {
        required_nonempty(map, "page_id").map_err(|_| json!({"field": "page_id"}))?;
        if map
            .get("expected_page_generation")
            .and_then(Value::as_u64)
            .is_none()
        {
            return Err(json!({"field": "expected_page_generation"}));
        }
    }
    if matches!(action, "click" | "type" | "select" | "download") {
        required_nonempty(map, "target_ref").map_err(|_| json!({"field": "target_ref"}))?;
        required_nonempty(map, "expected_snapshot_id")
            .map_err(|_| json!({"field": "expected_snapshot_id"}))?;
    }
    if map.get("target_ref").is_some() && map.get("expected_snapshot_id").is_none() {
        return Err(json!({"field": "expected_snapshot_id"}));
    }
    for field in [
        "expected_page_generation",
        "max_elements",
        "max_text_chars",
        "text_cursor",
        "limit",
    ] {
        if let Some(value) = map.get(field) {
            if value.as_u64().is_none() {
                return Err(json!({"field": field}));
            }
        }
    }
    if action == "navigate" {
        required_nonempty(map, "url").map_err(|_| json!({"field": "url"}))?;
    }
    if matches!(action, "type" | "select") {
        required_nonempty(map, "value").map_err(|_| json!({"field": "value"}))?;
    }
    if action == "press_key" {
        required_nonempty(map, "key").map_err(|_| json!({"field": "key"}))?;
    }
    if action == "wait_for" {
        required_nonempty(map, "condition").map_err(|_| json!({"field": "condition"}))?;
    }
    if let Some(viewport) = map.get("viewport") {
        let valid = viewport.as_object().is_some_and(|viewport| {
            viewport.get("width").and_then(Value::as_u64).is_some()
                && viewport.get("height").and_then(Value::as_u64).is_some()
        });
        if !valid {
            return Err(json!({"field": "viewport"}));
        }
    }
    for field in ["domains_allow", "domains_deny"] {
        if let Some(value) = map.get(field) {
            let valid = value.as_array().is_some_and(|items| {
                items.len() <= 128
                    && items.iter().all(|item| {
                        item.as_str()
                            .is_some_and(|text| !text.trim().is_empty() && text.len() <= 253)
                    })
            });
            if !valid {
                return Err(json!({"field": field}));
            }
        }
    }
    Ok(())
}

fn required_nonempty<'a>(map: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    let value = super::required_string(map, key)?.trim();
    if value.is_empty() {
        return Err(format!("invalid_nonempty_{key}"));
    }
    Ok(value)
}

fn structured_error(error: BrowserSessionError) -> String {
    let message_key = error.message_key.clone();
    super::builtin_error(
        "browser_session",
        &error.code.to_ascii_lowercase(),
        message_key,
        None,
        None,
        Some(json!({
            "schema_version": 1,
            "source_skill": "browser_session",
            "status": "error",
            "error_code": error.code,
            "message_key": error.message_key,
            "retryable": error.retryable,
            "details": error.details,
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutation_actions_require_current_snapshot_ref() {
        let map = serde_json::from_value::<Map<String, Value>>(json!({
            "action": "click", "session_id": "s", "page_id": "p", "target_ref": "e1"
        }))
        .unwrap();
        assert!(validate_action_args("click", &map).is_err());
    }

    #[test]
    fn arbitrary_browser_command_is_rejected() {
        let map = serde_json::from_value::<Map<String, Value>>(json!({
            "action": "eval", "session_id": "s"
        }))
        .unwrap();
        assert!(validate_action_args("eval", &map).is_err());
    }
}
