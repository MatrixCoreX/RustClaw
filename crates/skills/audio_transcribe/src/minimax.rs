//! Native speech-to-text transport; compatible proxies remain an explicit option.
use super::{provider_http_failure, trim_trailing_slash, SkillFailure, VendorConfig};
use reqwest::blocking::{multipart, Client};
use serde_json::Value;
use std::path::Path;

pub(super) fn transcribe(
    client: &Client,
    config: &VendorConfig,
    model: &str,
    audio_path: &Path,
    language: Option<&str>,
    token: Option<&str>,
) -> Result<String, SkillFailure> {
    let token = token
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            SkillFailure::new(
                "provider_not_configured",
                "minimax_stt_api_key_missing",
                false,
            )
        })?;
    let form = multipart::Form::new()
        .text("model", model.to_owned())
        .text("response_format", "json")
        .text("stream", "false")
        .file("file", audio_path)
        .map_err(|_| SkillFailure::new("invalid_input", "minimax_stt_audio_unreadable", false))?;
    let mut request = client
        .post(format!(
            "{}/speech_to_text",
            trim_trailing_slash(&config.base_url)
        ))
        .bearer_auth(token)
        .multipart(form);
    // The API accepts primary BCP-47 language tags; omission enables mixed-language ASR.
    if let Some(language) = language_hint(language) {
        request = request.header("language", language);
    }
    let response = request.send().map_err(|_| {
        SkillFailure::new(
            "provider_request_failed",
            "minimax_stt_transport_failed",
            true,
        )
    })?;
    let status = response.status().as_u16();
    if status >= 300 {
        return Err(provider_http_failure(
            status,
            format!("minimax_stt_http_{status}"),
        ));
    }
    let body: Value = response.json().map_err(|_| {
        SkillFailure::new(
            "provider_request_failed",
            "minimax_stt_response_invalid",
            false,
        )
    })?;
    extract_transcript(&body)
}

fn language_hint(language: Option<&str>) -> Option<String> {
    language
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("auto"))
        .and_then(|value| value.split('-').next())
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
}

fn extract_transcript(body: &Value) -> Result<String, SkillFailure> {
    // Error envelopes and malformed/empty responses must never become transcript text.
    let text = body.get("text").and_then(Value::as_str).map(str::trim);
    if body.get("error").is_some_and(|error| !error.is_null()) {
        return Err(SkillFailure::new(
            "provider_request_failed",
            "minimax_stt_provider_error",
            false,
        ));
    }
    text.filter(|text| !text.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            SkillFailure::new(
                "provider_request_failed",
                "minimax_stt_transcript_missing",
                false,
            )
        })
}

#[cfg(test)]
#[path = "minimax_tests.rs"]
mod tests;
