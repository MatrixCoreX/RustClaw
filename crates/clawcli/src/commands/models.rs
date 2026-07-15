use anyhow::Result;

use crate::output;

use super::common::get_v1_json;

const MODEL_READINESS_SUMMARY_TOKEN: &str = "model_readiness_summary";

pub(crate) fn run_models_catalog(
    base_url: &str,
    key: &str,
    provider: Option<&str>,
    json_output: bool,
) -> Result<()> {
    let body = get_v1_json(base_url, key, "/models/catalog", "models_catalog")?;
    let filtered = filter_catalog_response(body, provider);
    if json_output {
        output::print_json_pretty(&filtered);
    } else {
        for line in model_catalog_text_lines(&filtered) {
            println!("{line}");
        }
    }
    Ok(())
}

pub(crate) fn run_models_readiness(base_url: &str, key: &str, json_output: bool) -> Result<()> {
    let body = get_v1_json(base_url, key, "/models/catalog", "models_catalog")?;
    let readiness = model_readiness_json(&body);
    if json_output {
        output::print_json_pretty(&readiness);
    } else {
        for line in model_readiness_text_lines(&body) {
            println!("{line}");
        }
    }
    Ok(())
}

pub(super) fn filter_catalog_response(
    mut body: serde_json::Value,
    provider: Option<&str>,
) -> serde_json::Value {
    let Some(provider) = provider.map(str::trim).filter(|value| !value.is_empty()) else {
        return body;
    };
    let Some(entries) = body
        .pointer_mut("/data/entries")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return body;
    };
    entries.retain(|entry| {
        entry
            .get("provider")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| value == provider)
    });
    body
}

pub(super) fn model_catalog_text_lines(body: &serde_json::Value) -> Vec<String> {
    let entries = body
        .pointer("/data/entries")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut lines = vec![model_catalog_summary_line(body, entries.len())];
    lines.extend(entries
        .iter()
        .map(|entry| {
            let provider = token(entry, "provider");
            let model = token(entry, "model");
            let active = bool_token(entry, "active_text_provider");
            let context = token(entry, "context_window_tokens");
            let api_style = token(entry, "api_style");
            let base_url_kind = token(entry, "base_url_kind");
            let credential_state = token(entry, "credential_state");
            let input_modalities = array_token(entry, "input_modalities");
            let output_modalities = array_token(entry, "output_modalities");
            let capabilities = [
                ("text", bool_token(entry, "supports_text")),
                ("image_input", bool_token(entry, "supports_image_input")),
                ("video_input", bool_token(entry, "supports_video_input")),
                ("audio_input", bool_token(entry, "supports_audio_input")),
                (
                    "image_understanding",
                    bool_token(entry, "supports_image_understanding"),
                ),
                (
                    "audio_transcription",
                    bool_token(entry, "supports_audio_transcription"),
                ),
                (
                    "image_generation",
                    bool_token(entry, "supports_image_generation"),
                ),
                (
                    "audio_generation",
                    bool_token(entry, "supports_audio_generation"),
                ),
                (
                    "video_generation",
                    bool_token(entry, "supports_video_generation"),
                ),
                (
                    "music_generation",
                    bool_token(entry, "supports_music_generation"),
                ),
                ("async_required", bool_token(entry, "async_required")),
                ("dry_run", bool_token(entry, "dry_run_supported")),
            ]
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(" ");
            format!(
            "model_catalog_entry provider={provider} model={model} active={active} api_style={api_style} base_url_kind={base_url_kind} credential_state={credential_state} context_window_tokens={context} input_modalities={input_modalities} output_modalities={output_modalities} {capabilities}"
            )
        })
    );
    lines
}

pub(super) fn model_readiness_json(body: &serde_json::Value) -> serde_json::Value {
    let data = body.pointer("/data").unwrap_or(&serde_json::Value::Null);
    let selected_provider = token(data, "selected_provider");
    let selected_model = token(data, "selected_model");
    let entries = data
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let matched_entry_count = entries
        .iter()
        .filter(|entry| {
            token(entry, "provider") == selected_provider && token(entry, "model") == selected_model
        })
        .count();
    let selected_entry = entries.iter().find(|entry| {
        token(entry, "provider") == selected_provider && token(entry, "model") == selected_model
    });
    let selected_entry_status = if selected_entry.is_some() {
        "found"
    } else {
        "missing"
    };
    let null_entry = serde_json::Value::Null;
    let entry = selected_entry.unwrap_or(&null_entry);
    let credential_state = token(entry, "credential_state");
    let text_generation = bool_value(entry, "supports_text");
    let ready = selected_entry.is_some()
        && text_generation
        && !matches!(credential_state.as_str(), "missing" | "null" | "");
    serde_json::json!({
        "schema_version": data.get("schema_version").cloned().unwrap_or(serde_json::Value::Null),
        "selected_provider": selected_provider,
        "selected_model": selected_model,
        "selected_entry_status": selected_entry_status,
        "entry_count": entries.len(),
        "matched_entry_count": matched_entry_count,
        "credential_state": credential_state,
        "ready": ready,
        "text_generation": text_generation,
        "image_input": bool_value(entry, "supports_image_input"),
        "image_understanding": bool_value(entry, "supports_image_understanding"),
        "image_generation": bool_value(entry, "supports_image_generation"),
        "audio_input": bool_value(entry, "supports_audio_input"),
        "audio_transcription": bool_value(entry, "supports_audio_transcription"),
        "audio_generation": bool_value(entry, "supports_audio_generation"),
        "video_input": bool_value(entry, "supports_video_input"),
        "video_generation": bool_value(entry, "supports_video_generation"),
        "music_generation": bool_value(entry, "supports_music_generation"),
        "async_required": bool_value(entry, "async_required"),
        "dry_run": bool_value(entry, "dry_run_supported"),
    })
}

pub(super) fn model_readiness_text_lines(body: &serde_json::Value) -> Vec<String> {
    let readiness = model_readiness_json(body);
    let keys = [
        "schema_version",
        "selected_provider",
        "selected_model",
        "selected_entry_status",
        "entry_count",
        "matched_entry_count",
        "credential_state",
        "ready",
        "text_generation",
        "image_input",
        "image_understanding",
        "image_generation",
        "audio_input",
        "audio_transcription",
        "audio_generation",
        "video_input",
        "video_generation",
        "music_generation",
        "async_required",
        "dry_run",
    ];
    let fields = keys
        .iter()
        .map(|key| format!("{key}={}", json_token(&readiness, key)))
        .collect::<Vec<_>>()
        .join(" ");
    let mut line = MODEL_READINESS_SUMMARY_TOKEN.to_string();
    line.push(' ');
    line.push_str(&fields);
    vec![line]
}

fn model_catalog_summary_line(body: &serde_json::Value, entry_count: usize) -> String {
    let data = body.pointer("/data").unwrap_or(&serde_json::Value::Null);
    let schema_version = token(data, "schema_version");
    let selected_provider = token(data, "selected_provider");
    let selected_model = token(data, "selected_model");
    format!(
        "model_catalog_summary schema_version={schema_version} selected_provider={selected_provider} selected_model={selected_model} entry_count={entry_count}"
    )
}

fn token(entry: &serde_json::Value, key: &str) -> String {
    entry
        .get(key)
        .map(|value| match value {
            serde_json::Value::String(value) => value.clone(),
            serde_json::Value::Number(value) => value.to_string(),
            serde_json::Value::Bool(value) => value.to_string(),
            _ => "null".to_string(),
        })
        .unwrap_or_else(|| "null".to_string())
}

fn json_token(entry: &serde_json::Value, key: &str) -> String {
    entry
        .get(key)
        .map(|value| match value {
            serde_json::Value::String(value) => value.clone(),
            serde_json::Value::Number(value) => value.to_string(),
            serde_json::Value::Bool(value) => {
                if *value {
                    "1".to_string()
                } else {
                    "0".to_string()
                }
            }
            _ => "null".to_string(),
        })
        .unwrap_or_else(|| "null".to_string())
}

fn array_token(entry: &serde_json::Value, key: &str) -> String {
    entry
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join(",")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "missing".to_string())
}

fn bool_token(entry: &serde_json::Value, key: &str) -> &'static str {
    if bool_value(entry, key) {
        "1"
    } else {
        "0"
    }
}

fn bool_value(entry: &serde_json::Value, key: &str) -> bool {
    entry
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}
