use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use claw_core::model_turn::{
    ModelContentPart, ModelFinishReason, ModelMessage, ModelRole, ModelToolCall, ModelToolChoice,
    ModelTurnEvent, ModelTurnRequest, ModelTurnResponse,
};
use serde_json::{json, Map, Value};

use super::client::{
    is_quota_exhausted_response, ChatRequestHints, ModelTurnEventSink, ModelTurnProviderResponse,
    ProviderError,
};
use crate::LlmProviderRuntime;

pub(super) async fn call_gemini_model_turn(
    provider: Arc<LlmProviderRuntime>,
    request: &ModelTurnRequest,
    hints: &ChatRequestHints,
    event_sink: Option<ModelTurnEventSink>,
) -> Result<ModelTurnProviderResponse, ProviderError> {
    let _permit = provider
        .semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|error| {
            ProviderError::non_retryable(format!("semaphore closed: {error}"), Value::Null)
        })?;
    let req_body = build_gemini_request(&provider, request, hints)
        .map_err(|code| ProviderError::non_retryable(code, Value::Null))?;
    let api_key = provider.api_key();
    let url = format!(
        "{}/models/{}:generateContent?key={}",
        provider.config.base_url.trim_end_matches('/'),
        provider.config.model,
        &*api_key
    );
    let response = provider
        .client
        .post(url)
        .json(&req_body)
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                ProviderError::timeout(format!("timeout: {error}"), req_body.clone())
            } else {
                ProviderError::retryable(format!("request failed: {error}"), req_body.clone())
            }
        })?;
    let status = response.status();
    let body_text = response.text().await.map_err(|error| {
        ProviderError::retryable(format!("read response failed: {error}"), req_body.clone())
    })?;
    if status.as_u16() == 429 {
        return Err(if is_quota_exhausted_response(&body_text) {
            ProviderError::quota_exhausted_with_response(
                format!("http {}: {}", status.as_u16(), body_text),
                req_body,
                body_text,
                None,
            )
        } else {
            ProviderError::rate_limited_with_response(
                format!("http {}: {}", status.as_u16(), body_text),
                req_body,
                body_text,
                None,
            )
        });
    }
    if status.is_server_error() {
        return Err(ProviderError::retryable_with_response(
            format!("http {}: {}", status.as_u16(), body_text),
            req_body,
            body_text,
            None,
        ));
    }
    if !status.is_success() {
        return Err(ProviderError::non_retryable_with_response(
            format!("http {}: {}", status.as_u16(), body_text),
            req_body,
            body_text,
            None,
        ));
    }
    let value: Value = serde_json::from_str(&body_text).map_err(|error| {
        ProviderError::non_retryable_with_response(
            format!("parse response failed: {error}"),
            req_body.clone(),
            body_text.clone(),
            None,
        )
    })?;
    if let Some(reason) = value
        .pointer("/promptFeedback/blockReason")
        .and_then(Value::as_str)
    {
        return Err(ProviderError::non_retryable_with_response(
            format!("gemini_prompt_blocked reason={reason}"),
            req_body,
            super::output::provider_safe_raw_response(&body_text),
            super::gemini_usage_snapshot(&value),
        ));
    }
    let safe_response = super::output::provider_safe_raw_response(&body_text);
    let turn = parse_gemini_model_turn(&value).map_err(|code| {
        ProviderError::non_retryable_with_response(
            code,
            req_body.clone(),
            safe_response.clone(),
            super::gemini_usage_snapshot(&value),
        )
    })?;
    if let Some(sink) = event_sink.as_ref() {
        for event in &turn.events {
            sink(event.clone());
        }
    }
    Ok(ModelTurnProviderResponse {
        turn,
        request_payload: req_body,
        raw_response: safe_response,
        attempts: 1,
        retryable_error_count: 0,
        last_retry_error_kind: None,
    })
}

pub(super) fn build_gemini_request(
    provider: &LlmProviderRuntime,
    request: &ModelTurnRequest,
    hints: &ChatRequestHints,
) -> Result<Value, String> {
    let tool_names = request
        .messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|part| match part {
            ModelContentPart::ToolCall { call, .. } => Some((call.id.clone(), call.name.clone())),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    let mut system_parts = Vec::new();
    let mut contents = Vec::new();
    for message in &request.messages {
        if message.role == ModelRole::System {
            system_parts.extend(gemini_system_parts(message)?);
            continue;
        }
        let parts = gemini_message_parts(message, &tool_names)?;
        if parts.is_empty() {
            continue;
        }
        let role = if message.role == ModelRole::Assistant {
            "model"
        } else {
            "user"
        };
        contents.push(json!({"role": role, "parts": parts}));
    }
    if contents.is_empty() {
        return Err("model_turn_messages_empty".to_string());
    }
    let mut body = Map::from_iter([("contents".to_string(), Value::Array(contents))]);
    if !system_parts.is_empty() {
        body.insert(
            "systemInstruction".to_string(),
            json!({"parts": system_parts}),
        );
    }
    if !request.tools.is_empty() {
        body.insert(
            "tools".to_string(),
            json!([{
                "functionDeclarations": request.tools.iter().map(|tool| {
                    json!({
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema,
                    })
                }).collect::<Vec<_>>()
            }]),
        );
        body.insert(
            "toolConfig".to_string(),
            json!({
                "functionCallingConfig": {
                    "mode": match request.tool_choice {
                        ModelToolChoice::Auto => "AUTO",
                        ModelToolChoice::Required => "ANY",
                    }
                }
            }),
        );
    }

    let params = &provider.config.params;
    let mut generation = Map::new();
    if let Some(value) = hints.temperature.or(params.default_temperature) {
        generation.insert("temperature".to_string(), json!(value));
    }
    if let Some(value) = hints.max_tokens.or(params.default_max_tokens) {
        generation.insert("maxOutputTokens".to_string(), json!(value));
    }
    if let Some(value) = params.top_p {
        generation.insert("topP".to_string(), json!(value));
    }
    if let Some(schema) = request.response_schema.as_ref() {
        generation.insert(
            "responseMimeType".to_string(),
            Value::String("application/json".to_string()),
        );
        generation.insert("responseSchema".to_string(), schema.clone());
    }
    if !generation.is_empty() {
        body.insert("generationConfig".to_string(), Value::Object(generation));
    }
    Ok(Value::Object(body))
}

fn gemini_system_parts(message: &ModelMessage) -> Result<Vec<Value>, String> {
    message
        .content
        .iter()
        .map(|part| match part {
            ModelContentPart::Text { text } => Ok(json!({"text": text})),
            _ => Err("gemini_system_content_unsupported".to_string()),
        })
        .collect()
}

fn gemini_message_parts(
    message: &ModelMessage,
    tool_names: &HashMap<String, String>,
) -> Result<Vec<Value>, String> {
    message
        .content
        .iter()
        .enumerate()
        .map(|(index, part)| match part {
            ModelContentPart::Text { text } => Ok(json!({"text": text})),
            ModelContentPart::Image { source, media_type } => {
                Ok(gemini_image_part(source, media_type.as_deref()))
            }
            ModelContentPart::ToolCall {
                call,
                provider_metadata,
            } => {
                let mut mapped = Map::from_iter([(
                    "functionCall".to_string(),
                    json!({
                        "id": call.id,
                        "name": call.name,
                        "args": call.arguments,
                    }),
                )]);
                if let Some(signature) = provider_metadata.get("thoughtSignature") {
                    mapped.insert("thoughtSignature".to_string(), signature.clone());
                }
                Ok(Value::Object(mapped))
            }
            ModelContentPart::ToolResult {
                tool_call_id,
                content,
                is_error,
            } => {
                let name = tool_names
                    .get(tool_call_id)
                    .ok_or_else(|| format!("gemini_tool_result_name_missing index={index}"))?;
                let response = gemini_tool_response(content, *is_error);
                Ok(json!({
                    "functionResponse": {
                        "id": tool_call_id,
                        "name": name,
                        "response": response,
                    }
                }))
            }
        })
        .collect()
}

fn gemini_image_part(source: &str, media_type: Option<&str>) -> Value {
    if let Some((mime, data)) = split_data_uri(source) {
        return json!({"inlineData": {"mimeType": mime, "data": data}});
    }
    json!({
        "fileData": {
            "mimeType": media_type.unwrap_or("application/octet-stream"),
            "fileUri": source,
        }
    })
}

fn split_data_uri(source: &str) -> Option<(&str, &str)> {
    let rest = source.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    (!mime.is_empty() && !data.is_empty()).then_some((mime, data))
}

fn gemini_tool_response(content: &Value, is_error: bool) -> Value {
    if is_error {
        return json!({"error": content});
    }
    match content {
        Value::Object(_) => content.clone(),
        _ => json!({"result": content}),
    }
}

pub(super) fn parse_gemini_model_turn(value: &Value) -> Result<ModelTurnResponse, String> {
    let candidate = value
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|candidates| candidates.first())
        .ok_or_else(|| "gemini_model_turn_candidate_missing".to_string())?;
    let parts = candidate
        .pointer("/content/parts")
        .and_then(Value::as_array)
        .ok_or_else(|| "gemini_model_turn_parts_missing".to_string())?;
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut tool_call_metadata = Map::new();
    for (index, part) in parts.iter().enumerate() {
        if let Some(fragment) = part.get("text").and_then(Value::as_str) {
            text.push_str(fragment);
        }
        let Some(function) = part.get("functionCall") else {
            continue;
        };
        let name = required_string(function, "name", "gemini_tool_call_name_missing", index)?;
        let id = function
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("gemini_tool_call_{index}"));
        let arguments = function
            .get("args")
            .filter(|args| args.is_object())
            .cloned()
            .ok_or_else(|| format!("gemini_tool_call_args_missing index={index}"))?;
        if let Some(signature) = part.get("thoughtSignature") {
            tool_call_metadata.insert(id.clone(), json!({"thoughtSignature": signature.clone()}));
        }
        tool_calls.push(ModelToolCall {
            id,
            name,
            arguments,
        });
    }
    if text.is_empty() && tool_calls.is_empty() {
        return Err("gemini_model_turn_empty_response".to_string());
    }
    let usage = super::gemini_usage_snapshot(value);
    let finish_reason = if !tool_calls.is_empty() {
        ModelFinishReason::ToolCalls
    } else {
        gemini_finish_reason(
            candidate
                .get("finishReason")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
    };
    let mut events = Vec::new();
    if !text.is_empty() {
        events.push(ModelTurnEvent::TextDelta { text: text.clone() });
    }
    events.extend(
        tool_calls
            .iter()
            .cloned()
            .map(|call| ModelTurnEvent::ToolCall { call }),
    );
    if let Some(usage) = usage.clone() {
        events.push(ModelTurnEvent::Usage { usage });
    }
    events.push(ModelTurnEvent::Finished {
        reason: finish_reason,
    });
    let mut reasoning_metadata = BTreeMap::new();
    if !tool_call_metadata.is_empty() {
        reasoning_metadata.insert(
            "tool_call_metadata".to_string(),
            Value::Object(tool_call_metadata),
        );
    }
    Ok(ModelTurnResponse {
        text,
        tool_calls,
        usage,
        finish_reason,
        reasoning_metadata,
        events,
    })
}

fn required_string(value: &Value, key: &str, code: &str, index: usize) -> Result<String, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{code} index={index}"))
}

fn gemini_finish_reason(reason: &str) -> ModelFinishReason {
    match reason {
        "STOP" => ModelFinishReason::Stop,
        "MAX_TOKENS" => ModelFinishReason::Length,
        "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" => {
            ModelFinishReason::ContentFilter
        }
        "MALFORMED_FUNCTION_CALL" => ModelFinishReason::Error,
        _ => ModelFinishReason::Unknown,
    }
}

#[cfg(test)]
#[path = "gemini_model_turn_tests.rs"]
mod tests;
