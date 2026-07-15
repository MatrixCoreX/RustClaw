use std::fs;
use std::path::Path;

use serde_json::{json, Value};

pub(super) struct OwnedFieldLookup {
    pub(super) value: Value,
    pub(super) resolved_field_path: String,
    pub(super) match_strategy: &'static str,
}

pub(super) fn lookup_model_catalog_field_alias(
    workspace_root: &Path,
    real_path: &Path,
    root_value: &Value,
    field_path: &str,
) -> Option<OwnedFieldLookup> {
    if !real_path.ends_with("configs/config.toml") {
        return None;
    }
    let segments = split_machine_path(field_path);
    match segments.as_slice() {
        ["providers", provider] => provider_object(workspace_root, root_value, provider),
        ["providers", provider, "text"] => provider_text_object(root_value, provider),
        ["providers", provider, "text", field] => provider_field(root_value, provider, field),
        ["providers", provider, "models"] => {
            provider_models_object(workspace_root, root_value, provider)
        }
        ["providers", provider, field_or_model] => {
            provider_field(root_value, provider, field_or_model).or_else(|| {
                provider_model_object(workspace_root, root_value, provider, field_or_model)
            })
        }
        ["providers", provider, "models", model_token] => {
            provider_model_object(workspace_root, root_value, provider, model_token)
        }
        ["providers", provider, model_token, "capabilities"] => Some(provider_model_capabilities(
            workspace_root,
            root_value,
            provider,
            model_token,
        )),
        ["providers", provider, model_token, field] => {
            provider_model_field(workspace_root, root_value, provider, model_token, field)
        }
        ["providers", provider, "models", model_token, "capabilities"] => Some(
            provider_model_capabilities(workspace_root, root_value, provider, model_token),
        ),
        ["providers", provider, "models", model_token, field] => {
            provider_model_field(workspace_root, root_value, provider, model_token, field)
        }
        ["models", model, field] => model_field(workspace_root, root_value, model, field),
        ["models", model, "capabilities", field] => {
            model_capability_field(workspace_root, root_value, model, field)
        }
        _ => None,
    }
}

fn provider_object(
    workspace_root: &Path,
    root_value: &Value,
    provider: &str,
) -> Option<OwnedFieldLookup> {
    let provider_key = normalize_provider(provider);
    let table = provider_table(root_value, &provider_key)?;
    let text = provider_text_value(root_value, &provider_key)?;
    let selected = table.get("model").cloned().unwrap_or(Value::Null);
    Some(owned(
        json!({
            "provider": provider_key,
            "selected_model": selected,
            "model": table.get("model").cloned().unwrap_or(Value::Null),
            "base_url": table.get("base_url").cloned().unwrap_or(Value::Null),
            "context_window_tokens": table.get("context_window_tokens").cloned().unwrap_or(Value::Null),
            "text": text,
            "model_ids": provider_model_candidates(table),
            "models": provider_model_summaries(workspace_root, root_value, &provider_key),
            "understanding_inputs": table
                .get("model")
                .and_then(Value::as_str)
                .map(|model| understanding_inputs(workspace_root, &provider_key, model))
                .unwrap_or_default(),
            "generation_boundary": generation_boundary(workspace_root, &provider_key),
        }),
        format!("llm.{provider_key}"),
        "model_catalog_provider_alias",
    ))
}

fn provider_text_object(root_value: &Value, provider: &str) -> Option<OwnedFieldLookup> {
    let provider_key = normalize_provider(provider);
    Some(owned(
        provider_text_value(root_value, &provider_key)?,
        format!("llm.{provider_key}"),
        "model_catalog_provider_alias",
    ))
}

fn provider_text_value(root_value: &Value, provider_key: &str) -> Option<Value> {
    let table = provider_table(root_value, provider_key)?;
    Some(json!({
        "selected_model": table.get("model").cloned().unwrap_or(Value::Null),
        "model": table.get("model").cloned().unwrap_or(Value::Null),
        "base_url": table.get("base_url").cloned().unwrap_or(Value::Null),
        "context_window_tokens": table.get("context_window_tokens").cloned().unwrap_or(Value::Null),
        "timeout_seconds": table.get("timeout_seconds").cloned().unwrap_or(Value::Null),
        "models": table.get("models").cloned().unwrap_or_else(|| json!([])),
    }))
}

fn provider_models_object(
    workspace_root: &Path,
    root_value: &Value,
    provider: &str,
) -> Option<OwnedFieldLookup> {
    let provider_key = normalize_provider(provider);
    provider_table(root_value, &provider_key)?;
    Some(owned(
        json!(provider_model_summaries(
            workspace_root,
            root_value,
            &provider_key
        )),
        format!("llm.{provider_key}.models"),
        "model_catalog_provider_alias",
    ))
}

fn provider_model_object(
    workspace_root: &Path,
    root_value: &Value,
    provider: &str,
    model_token: &str,
) -> Option<OwnedFieldLookup> {
    let provider_key = normalize_provider(provider);
    let model = resolve_provider_model(root_value, &provider_key, model_token)?;
    Some(owned(
        provider_model_summary(workspace_root, root_value, &provider_key, &model),
        format!("models.{model}"),
        "model_catalog_model_alias",
    ))
}

fn provider_field(root_value: &Value, provider: &str, field: &str) -> Option<OwnedFieldLookup> {
    let provider_key = normalize_provider(provider);
    let table = provider_table(root_value, &provider_key)?;
    let resolved_leaf = match field {
        "selected_model" | "model" | "text_model" => "model",
        "base_url" => "base_url",
        "context_window_tokens" => "context_window_tokens",
        "timeout_seconds" => "timeout_seconds",
        "models" => "models",
        _ => return None,
    };
    let value = table.get(resolved_leaf)?.clone();
    Some(owned(
        value,
        format!("llm.{provider_key}.{resolved_leaf}"),
        "model_catalog_provider_alias",
    ))
}

fn provider_model_field(
    workspace_root: &Path,
    root_value: &Value,
    provider: &str,
    model_token: &str,
    field: &str,
) -> Option<OwnedFieldLookup> {
    let provider_key = normalize_provider(provider);
    let model = resolve_provider_model(root_value, &provider_key, model_token)?;
    match field {
        "context_window_tokens" => provider_field(root_value, &provider_key, field),
        "selected" => Some(owned(
            json!(selected_model(root_value).as_deref() == Some(model.as_str())),
            format!("models.{model}.selected"),
            "model_catalog_model_alias",
        )),
        "understanding_inputs" => Some(owned(
            json!(understanding_inputs(workspace_root, &provider_key, &model)),
            format!("models.{model}.understanding_inputs"),
            "model_catalog_model_alias",
        )),
        "generation_boundary" => Some(owned(
            generation_boundary(workspace_root, &provider_key),
            format!("models.{model}.generation_boundary"),
            "model_catalog_model_alias",
        )),
        "skills" => Some(owned(
            media_skill_names(workspace_root, &provider_key),
            format!("models.{model}.skills"),
            "model_catalog_model_alias",
        )),
        _ => None,
    }
}

fn provider_model_capabilities(
    workspace_root: &Path,
    root_value: &Value,
    provider: &str,
    model_token: &str,
) -> OwnedFieldLookup {
    let provider_key = normalize_provider(provider);
    let model = resolve_provider_model(root_value, &provider_key, model_token)
        .unwrap_or_else(|| model_token.to_string());
    owned(
        json!({
            "understanding": understanding_inputs(workspace_root, &provider_key, &model),
            "generation": generation_boundary(workspace_root, &provider_key),
        }),
        format!("models.{model}.capabilities"),
        "model_catalog_model_alias",
    )
}

fn model_field(
    workspace_root: &Path,
    root_value: &Value,
    model: &str,
    field: &str,
) -> Option<OwnedFieldLookup> {
    let (provider_key, resolved_model) = provider_for_model(root_value, model)?;
    match field {
        "context_window_tokens" => provider_field(root_value, &provider_key, field),
        "selected" => Some(owned(
            json!(selected_model(root_value).as_deref() == Some(resolved_model.as_str())),
            format!("models.{resolved_model}.selected"),
            "model_catalog_model_alias",
        )),
        "understanding_inputs" => Some(owned(
            json!(understanding_inputs(
                workspace_root,
                &provider_key,
                &resolved_model
            )),
            format!("models.{resolved_model}.understanding_inputs"),
            "model_catalog_model_alias",
        )),
        "generation_boundary" => Some(owned(
            generation_boundary(workspace_root, &provider_key),
            format!("models.{resolved_model}.generation_boundary"),
            "model_catalog_model_alias",
        )),
        _ => None,
    }
}

fn model_capability_field(
    workspace_root: &Path,
    root_value: &Value,
    model: &str,
    field: &str,
) -> Option<OwnedFieldLookup> {
    let (provider_key, resolved_model) = provider_for_model(root_value, model)?;
    match field {
        "understanding" => Some(owned(
            json!(understanding_inputs(
                workspace_root,
                &provider_key,
                &resolved_model
            )),
            format!("models.{resolved_model}.capabilities.understanding"),
            "model_catalog_model_alias",
        )),
        "generation" => Some(owned(
            generation_boundary(workspace_root, &provider_key),
            format!("models.{resolved_model}.capabilities.generation"),
            "model_catalog_model_alias",
        )),
        _ => None,
    }
}

fn provider_table<'a>(
    root_value: &'a Value,
    provider_key: &str,
) -> Option<&'a serde_json::Map<String, Value>> {
    root_value.get("llm")?.get(provider_key)?.as_object()
}

fn selected_model(root_value: &Value) -> Option<String> {
    root_value
        .get("llm")
        .and_then(|llm| llm.get("selected_model"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn resolve_provider_model(
    root_value: &Value,
    provider_key: &str,
    model_token: &str,
) -> Option<String> {
    let table = provider_table(root_value, provider_key)?;
    let candidates = provider_model_candidates(table);
    resolve_model_candidate(&candidates, model_token)
}

fn provider_for_model(root_value: &Value, model_token: &str) -> Option<(String, String)> {
    let llm = root_value.get("llm")?.as_object()?;
    for (provider_key, table) in llm {
        let Some(table) = table.as_object() else {
            continue;
        };
        let candidates = provider_model_candidates(table);
        if let Some(model) = resolve_model_candidate(&candidates, model_token) {
            return Some((provider_key.to_string(), model));
        }
    }
    None
}

fn provider_model_candidates(table: &serde_json::Map<String, Value>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(model) = table
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        out.push(model.to_string());
    }
    if let Some(models) = table.get("models").and_then(Value::as_array) {
        for item in models {
            if let Some(model) = item
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if !out.iter().any(|existing| existing == model) {
                    out.push(model.to_string());
                }
            }
        }
    }
    out
}

fn provider_model_summaries(
    workspace_root: &Path,
    root_value: &Value,
    provider_key: &str,
) -> Vec<Value> {
    let Some(table) = provider_table(root_value, provider_key) else {
        return Vec::new();
    };
    provider_model_candidates(table)
        .into_iter()
        .map(|model| provider_model_summary(workspace_root, root_value, provider_key, &model))
        .collect()
}

fn provider_model_summary(
    workspace_root: &Path,
    root_value: &Value,
    provider_key: &str,
    model: &str,
) -> Value {
    let inputs = understanding_inputs(workspace_root, provider_key, model);
    let boundary = generation_boundary(workspace_root, provider_key);
    json!({
        "model": model,
        "provider": provider_key,
        "selected": selected_model(root_value).as_deref() == Some(model),
        "context_window_tokens": provider_field(root_value, provider_key, "context_window_tokens")
            .map(|found| found.value)
            .unwrap_or(Value::Null),
        "understanding_inputs": inputs.clone(),
        "generation_boundary": boundary.clone(),
        "capabilities": {
            "understanding": inputs,
            "generation": boundary,
        },
    })
}

fn resolve_model_candidate(candidates: &[String], model_token: &str) -> Option<String> {
    let token = model_token.trim();
    if token.is_empty() {
        return None;
    }
    candidates
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(token))
        .cloned()
        .or_else(|| {
            candidates
                .iter()
                .find(|candidate| {
                    candidate
                        .rsplit(['-', '_', '.'])
                        .next()
                        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(token))
                })
                .cloned()
        })
}

fn understanding_inputs(workspace_root: &Path, provider_key: &str, model: &str) -> Vec<String> {
    let image = read_config(workspace_root, "image.toml");
    let audio = read_config(workspace_root, "audio.toml");
    let mut inputs = Vec::new();
    if section_provider_models(&image, "image_vision", provider_key)
        .iter()
        .any(|candidate| candidate == model)
    {
        inputs.push("image".to_string());
    }
    if provider_key == "minimax" && model == "MiniMax-M3" {
        inputs.push("video".to_string());
    }
    if section_provider_models(&audio, "audio_transcribe", provider_key)
        .iter()
        .any(|candidate| candidate == model)
    {
        inputs.push("audio".to_string());
    }
    inputs
}

fn generation_boundary(workspace_root: &Path, provider_key: &str) -> Value {
    let image = read_config(workspace_root, "image.toml");
    let audio = read_config(workspace_root, "audio.toml");
    let video = read_config(workspace_root, "video.toml");
    let music = read_config(workspace_root, "music.toml");
    let mut skills = Vec::new();
    if !section_provider_models(&image, "image_generation", provider_key).is_empty()
        || !section_provider_models(&image, "image_edit", provider_key).is_empty()
    {
        skills.push("image.generate");
    }
    if !section_provider_models(&audio, "audio_synthesize", provider_key).is_empty() {
        skills.push("audio.synthesize");
    }
    if !section_provider_models(&video, "video_generation", provider_key).is_empty() {
        skills.push("video.generate");
    }
    if !section_provider_models(&music, "music_generation", provider_key).is_empty() {
        skills.push("music.generate");
    }
    json!({
        "direct_text_model_generation": false,
        "media_skill_capabilities": skills,
        "execution_contract": "async_start_poll_cancel",
        "dry_run_supported": true,
    })
}

fn media_skill_names(workspace_root: &Path, provider_key: &str) -> Value {
    generation_boundary(workspace_root, provider_key)
        .get("media_skill_capabilities")
        .cloned()
        .unwrap_or_else(|| json!([]))
}

fn section_provider_models(config: &Value, section: &str, provider_key: &str) -> Vec<String> {
    config
        .get(section)
        .and_then(|section| section.get(format!("{provider_key}_models")))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn read_config(workspace_root: &Path, file_name: &str) -> Value {
    let path = workspace_root.join("configs").join(file_name);
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| toml::from_str::<toml::Value>(&raw).ok())
        .and_then(|value| serde_json::to_value(value).ok())
        .unwrap_or_else(|| json!({}))
}

fn normalize_provider(provider: &str) -> String {
    provider
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
        .flat_map(char::to_lowercase)
        .collect()
}

fn split_machine_path(field_path: &str) -> Vec<&str> {
    field_path
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn owned(
    value: Value,
    resolved_field_path: String,
    match_strategy: &'static str,
) -> OwnedFieldLookup {
    OwnedFieldLookup {
        value,
        resolved_field_path,
        match_strategy,
    }
}
