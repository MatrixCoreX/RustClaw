use std::collections::BTreeMap;
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

pub(super) async fn call_anthropic_model_turn(
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
    let req_body = build_anthropic_request(&provider, request, hints)
        .map_err(|code| ProviderError::non_retryable(code, Value::Null))?;
    let api_key = provider.api_key();
    let http_request = provider
        .client
        .post(super::anthropic_claude::anthropic_messages_url(&provider))
        .header("anthropic-version", "2023-06-01");
    let http_request = match super::anthropic_claude::anthropic_auth_mode(&provider) {
        super::anthropic_claude::AnthropicAuthMode::XApiKey => {
            http_request.header("x-api-key", &*api_key)
        }
        super::anthropic_claude::AnthropicAuthMode::AuthorizationBearer => {
            http_request.bearer_auth(&*api_key)
        }
    };
    let response = http_request.json(&req_body).send().await.map_err(|error| {
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
    let safe_response = super::output::provider_safe_raw_response(&body_text);
    let turn = parse_anthropic_model_turn(&value).map_err(|code| {
        ProviderError::non_retryable_with_response(
            code,
            req_body.clone(),
            safe_response.clone(),
            super::anthropic_usage_snapshot(&value),
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

pub(super) fn build_anthropic_request(
    provider: &LlmProviderRuntime,
    request: &ModelTurnRequest,
    hints: &ChatRequestHints,
) -> Result<Value, String> {
    let mut system_blocks = Vec::new();
    let mut messages = Vec::new();
    for message in &request.messages {
        if message.role == ModelRole::System {
            system_blocks.extend(anthropic_system_blocks(message)?);
            continue;
        }
        let content = anthropic_message_blocks(message)?;
        if content.is_empty() {
            continue;
        }
        let role = match message.role {
            ModelRole::Assistant => "assistant",
            ModelRole::User | ModelRole::Tool => "user",
            ModelRole::System => unreachable!(),
        };
        messages.push(json!({"role": role, "content": content}));
    }
    if messages.is_empty() {
        return Err("model_turn_messages_empty".to_string());
    }

    let params = &provider.config.params;
    let mut body = Map::from_iter([
        ("model".to_string(), json!(provider.config.model)),
        (
            "max_tokens".to_string(),
            json!(hints
                .max_tokens
                .or(params.default_max_tokens)
                .unwrap_or(4096)),
        ),
        ("messages".to_string(), Value::Array(messages)),
    ]);
    if !system_blocks.is_empty() {
        body.insert("system".to_string(), Value::Array(system_blocks));
    }
    if let Some(value) = hints.temperature.or(params.default_temperature) {
        body.insert("temperature".to_string(), json!(value));
    }
    if let Some(value) = params.top_p {
        body.insert("top_p".to_string(), json!(value));
    }
    if !request.tools.is_empty() {
        body.insert(
            "tools".to_string(),
            Value::Array(
                request
                    .tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "name": tool.name,
                            "description": tool.description,
                            "input_schema": tool.input_schema,
                            "strict": tool.strict,
                        })
                    })
                    .collect(),
            ),
        );
        body.insert(
            "tool_choice".to_string(),
            match request.tool_choice {
                ModelToolChoice::Auto => json!({"type": "auto"}),
                ModelToolChoice::Required => json!({"type": "any"}),
            },
        );
    }
    Ok(Value::Object(body))
}

fn anthropic_system_blocks(message: &ModelMessage) -> Result<Vec<Value>, String> {
    message
        .content
        .iter()
        .map(|part| match part {
            ModelContentPart::Text { text } => Ok(json!({"type": "text", "text": text})),
            _ => Err("anthropic_system_content_unsupported".to_string()),
        })
        .collect()
}

fn anthropic_message_blocks(message: &ModelMessage) -> Result<Vec<Value>, String> {
    message
        .content
        .iter()
        .map(|part| match part {
            ModelContentPart::Text { text } => Ok(json!({"type": "text", "text": text})),
            ModelContentPart::Image { source, media_type } => {
                Ok(anthropic_image_block(source, media_type.as_deref()))
            }
            ModelContentPart::ToolCall { call, .. } => Ok(json!({
                "type": "tool_use",
                "id": call.id,
                "name": call.name,
                "input": call.arguments,
            })),
            ModelContentPart::ToolResult {
                tool_call_id,
                content,
                is_error,
            } => Ok(json!({
                "type": "tool_result",
                "tool_use_id": tool_call_id,
                "content": tool_result_text(content),
                "is_error": is_error,
            })),
        })
        .collect()
}

fn anthropic_image_block(source: &str, _media_type: Option<&str>) -> Value {
    if let Some((mime, data)) = split_data_uri(source) {
        return json!({
            "type": "image",
            "source": {"type": "base64", "media_type": mime, "data": data},
        });
    }
    json!({
        "type": "image",
        "source": {"type": "url", "url": source},
    })
}

fn split_data_uri(source: &str) -> Option<(&str, &str)> {
    let rest = source.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    (!mime.is_empty() && !data.is_empty()).then_some((mime, data))
}

fn tool_result_text(content: &Value) -> String {
    content
        .as_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| content.to_string())
}

pub(super) fn parse_anthropic_model_turn(value: &Value) -> Result<ModelTurnResponse, String> {
    let content = value
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| "anthropic_model_turn_content_missing".to_string())?;
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for (index, block) in content.iter().enumerate() {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(fragment) = block.get("text").and_then(Value::as_str) {
                    text.push_str(fragment);
                }
            }
            Some("tool_use") => {
                let id = required_string(block, "id", "anthropic_tool_call_id_missing", index)?;
                let name =
                    required_string(block, "name", "anthropic_tool_call_name_missing", index)?;
                let arguments = block
                    .get("input")
                    .filter(|input| input.is_object())
                    .cloned()
                    .ok_or_else(|| format!("anthropic_tool_call_input_missing index={index}"))?;
                tool_calls.push(ModelToolCall {
                    id,
                    name,
                    arguments,
                });
            }
            _ => {}
        }
    }
    if text.is_empty() && tool_calls.is_empty() {
        return Err("anthropic_model_turn_empty_response".to_string());
    }
    let usage = super::anthropic_usage_snapshot(value);
    let finish_reason = if !tool_calls.is_empty() {
        ModelFinishReason::ToolCalls
    } else {
        anthropic_finish_reason(
            value
                .get("stop_reason")
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
    Ok(ModelTurnResponse {
        text,
        tool_calls,
        usage,
        finish_reason,
        reasoning_metadata: BTreeMap::new(),
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

fn anthropic_finish_reason(reason: &str) -> ModelFinishReason {
    match reason {
        "end_turn" | "stop_sequence" => ModelFinishReason::Stop,
        "tool_use" => ModelFinishReason::ToolCalls,
        "max_tokens" => ModelFinishReason::Length,
        "refusal" => ModelFinishReason::ContentFilter,
        _ => ModelFinishReason::Unknown,
    }
}

#[cfg(test)]
#[path = "anthropic_model_turn_tests.rs"]
mod tests;
