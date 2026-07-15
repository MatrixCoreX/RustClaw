use anyhow::Result;

use crate::output;

use super::common::get_v1_json;

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
    entries
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
        .collect()
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
    if entry
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        "1"
    } else {
        "0"
    }
}
