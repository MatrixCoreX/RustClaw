use std::sync::Arc;

use claw_core::config::AppConfig;
use serde_json::{json, Value};

use crate::{AppState, ClaimedTask, LlmProviderRuntime};

const REQUEST_FIELD: &str = "model_selection";
const STAMP_FIELD: &str = "_rustclaw_model_selection";
const MAX_SELECTION_TOKEN_BYTES: usize = 128;

#[derive(Clone)]
pub(crate) struct TaskModelProviderSelection {
    provider: String,
    model: String,
    providers: Vec<Arc<LlmProviderRuntime>>,
}

pub(crate) fn validate_and_stamp_task_model_selection(
    state: &AppState,
    payload: &mut Value,
) -> Result<(), &'static str> {
    let Some(object) = payload.as_object_mut() else {
        return Ok(());
    };
    object.remove(STAMP_FIELD);
    let Some(requested) = object.remove(REQUEST_FIELD) else {
        return Ok(());
    };
    let requested = requested
        .as_object()
        .ok_or("task_model_selection_object_required")?;
    if requested
        .keys()
        .any(|key| !matches!(key.as_str(), "provider" | "model"))
    {
        return Err("task_model_selection_additional_field_denied");
    }
    let provider = bounded_token(requested.get("provider"), "task_model_provider_invalid")?;
    let model = bounded_token(requested.get("model"), "task_model_id_invalid")?;
    validate_catalog_selection(state, provider, model)?;
    let config = AppConfig::load(&state.reload_ctx.config_path_for_reload)
        .map_err(|_| "task_model_runtime_config_unavailable")?;
    let providers =
        crate::llm_gateway::build_providers_for_selection(&config, Some(provider), Some(model));
    if providers.is_empty()
        || providers
            .iter()
            .any(|runtime| runtime.config.model != model)
    {
        return Err("task_model_runtime_unavailable");
    }
    object.insert(
        STAMP_FIELD.to_string(),
        json!({
            "schema_version": 1,
            "provider": provider,
            "model": model,
            "authority": "server_validated_model_catalog",
        }),
    );
    Ok(())
}

pub(crate) fn providers_for_task_model_selection(
    state: &AppState,
    task: &ClaimedTask,
) -> Option<Vec<Arc<LlmProviderRuntime>>> {
    let payload = serde_json::from_str::<Value>(&task.payload_json).ok()?;
    let stamp = payload.get(STAMP_FIELD)?;
    if !valid_stamp_shape(stamp) {
        return Some(Vec::new());
    }
    let provider = stamp.get("provider").and_then(Value::as_str)?;
    let model = stamp.get("model").and_then(Value::as_str)?;
    if let Some(cached) = cached_task_selection(state, &task.task_id, provider, model) {
        return Some(cached);
    }
    let config = match AppConfig::load(&state.reload_ctx.config_path_for_reload) {
        Ok(config) => config,
        Err(_) => return Some(Vec::new()),
    };
    let providers =
        crate::llm_gateway::build_providers_for_selection(&config, Some(provider), Some(model));
    let providers = providers
        .into_iter()
        .filter(|runtime| runtime.config.model == model)
        .collect::<Vec<_>>();
    if providers.is_empty() {
        return Some(providers);
    }
    Some(cache_task_selection(
        state,
        &task.task_id,
        provider,
        model,
        providers,
    ))
}

fn cached_task_selection(
    state: &AppState,
    task_id: &str,
    provider: &str,
    model: &str,
) -> Option<Vec<Arc<LlmProviderRuntime>>> {
    state
        .metrics
        .selected_llm_providers_per_task
        .lock()
        .ok()?
        .get(task_id)
        .filter(|selection| selection.provider == provider && selection.model == model)
        .map(|selection| selection.providers.clone())
}

fn cache_task_selection(
    state: &AppState,
    task_id: &str,
    provider: &str,
    model: &str,
    providers: Vec<Arc<LlmProviderRuntime>>,
) -> Vec<Arc<LlmProviderRuntime>> {
    let Ok(mut cache) = state.metrics.selected_llm_providers_per_task.lock() else {
        return providers;
    };
    let selection =
        cache
            .entry(task_id.to_string())
            .or_insert_with(|| TaskModelProviderSelection {
                provider: provider.to_string(),
                model: model.to_string(),
                providers,
            });
    if selection.provider == provider && selection.model == model {
        selection.providers.clone()
    } else {
        Vec::new()
    }
}

fn validate_catalog_selection(
    state: &AppState,
    provider: &str,
    model: &str,
) -> Result<(), &'static str> {
    let catalog = claw_core::model_catalog::build_model_catalog_from_workspace(
        &state.skill_rt.workspace_root,
    )
    .map_err(|_| "task_model_catalog_unavailable")?;
    let entry = catalog
        .entries
        .iter()
        .find(|entry| {
            entry.provider == provider
                && (entry.model == model || entry.models.iter().any(|item| item == model))
        })
        .ok_or("task_model_not_allowed")?;
    if !entry.supports_text {
        return Err("task_model_text_unsupported");
    }
    if entry.credential_state == "missing" || entry.credential_state.trim().is_empty() {
        return Err("task_model_credential_missing");
    }
    Ok(())
}

fn bounded_token<'a>(
    value: Option<&'a Value>,
    error_code: &'static str,
) -> Result<&'a str, &'static str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= MAX_SELECTION_TOKEN_BYTES)
        .ok_or(error_code)
}

fn valid_stamp_shape(stamp: &Value) -> bool {
    let Some(object) = stamp.as_object() else {
        return false;
    };
    object.keys().all(|key| {
        matches!(
            key.as_str(),
            "schema_version" | "provider" | "model" | "authority"
        )
    }) && stamp.get("schema_version").and_then(Value::as_u64) == Some(1)
        && stamp.get("authority").and_then(Value::as_str) == Some("server_validated_model_catalog")
        && bounded_token(stamp.get("provider"), "invalid").is_ok()
        && bounded_token(stamp.get("model"), "invalid").is_ok()
}

#[cfg(test)]
#[path = "task_model_selection_tests.rs"]
mod tests;
