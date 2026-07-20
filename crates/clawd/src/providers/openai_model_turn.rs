use std::collections::BTreeMap;
use std::sync::Arc;

use claw_core::model_turn::{
    ModelContentPart, ModelFinishReason, ModelMessage, ModelRole, ModelToolCall, ModelTurnEvent,
    ModelTurnRequest, ModelTurnResponse,
};
use futures_util::StreamExt;
use serde_json::{json, Map, Value};

use super::client::{
    is_quota_exhausted_response, ChatRequestHints, ModelTurnEventSink, ModelTurnProviderResponse,
    ProviderError,
};
use super::openai_usage_snapshot;
use crate::LlmProviderRuntime;

pub(super) async fn call_openai_model_turn(
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
        .map_err(|err| {
            ProviderError::non_retryable(format!("semaphore closed: {err}"), Value::Null)
        })?;
    let req_body = build_openai_request(&provider, request, hints)?;
    let url = format!(
        "{}/chat/completions",
        provider.config.base_url.trim_end_matches('/')
    );
    let api_key = provider.api_key();
    let response = provider
        .client
        .post(url)
        .bearer_auth(&*api_key)
        .json(&req_body)
        .send()
        .await
        .map_err(|err| {
            if err.is_timeout() {
                ProviderError::timeout(format!("timeout: {err}"), req_body.clone())
            } else {
                ProviderError::retryable(format!("request failed: {err}"), req_body.clone())
            }
        })?;
    let status = response.status();
    if status.is_success() && request.stream {
        return read_openai_stream(response, req_body, event_sink).await;
    }
    let body_text = response.text().await.map_err(|err| {
        ProviderError::retryable(format!("read response failed: {err}"), req_body.clone())
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

    let value: Value = serde_json::from_str(&body_text).map_err(|err| {
        ProviderError::non_retryable_with_response(
            format!("parse response failed: {err}"),
            req_body.clone(),
            body_text.clone(),
            None,
        )
    })?;
    let safe_body_text = provider_safe_raw_response(&value);
    let turn = parse_openai_model_turn(&value).map_err(|code| {
        ProviderError::non_retryable_with_response(
            code,
            req_body.clone(),
            safe_body_text.clone(),
            openai_usage_snapshot(&value),
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
        raw_response: safe_body_text,
        attempts: 1,
        retryable_error_count: 0,
        last_retry_error_kind: None,
    })
}

fn build_openai_request(
    provider: &LlmProviderRuntime,
    request: &ModelTurnRequest,
    hints: &ChatRequestHints,
) -> Result<Value, ProviderError> {
    let messages = request
        .messages
        .iter()
        .flat_map(openai_messages)
        .collect::<Vec<_>>();
    if messages.is_empty() {
        return Err(ProviderError::non_retryable(
            "model_turn_messages_empty".to_string(),
            Value::Null,
        ));
    }
    let params = &provider.config.params;
    let mut body = Map::from_iter([
        ("model".to_string(), json!(provider.config.model)),
        ("messages".to_string(), Value::Array(messages)),
        ("stream".to_string(), Value::Bool(request.stream)),
    ]);
    if let Some(value) = hints.temperature.or(params.default_temperature) {
        body.insert("temperature".to_string(), json!(value));
    }
    if let Some(value) = hints.max_tokens.or(params.default_max_tokens) {
        body.insert("max_tokens".to_string(), json!(value));
    }
    if let Some(value) = params.top_p {
        body.insert("top_p".to_string(), json!(value));
    }
    if !request.tools.is_empty() {
        let tools = request
            .tools
            .iter()
            .map(|tool| {
                let mut function = Map::from_iter([
                    ("name".to_string(), json!(tool.name)),
                    ("description".to_string(), json!(tool.description)),
                    ("parameters".to_string(), tool.input_schema.clone()),
                ]);
                if tool.strict {
                    function.insert("strict".to_string(), Value::Bool(true));
                }
                json!({"type": "function", "function": function})
            })
            .collect::<Vec<_>>();
        body.insert("tools".to_string(), Value::Array(tools));
        body.insert("tool_choice".to_string(), Value::String("auto".to_string()));
        body.insert("parallel_tool_calls".to_string(), Value::Bool(true));
    }
    if let Some(schema) = request.response_schema.as_ref() {
        body.insert(
            "response_format".to_string(),
            json!({
                "type": "json_schema",
                "json_schema": {
                    "name": "structured_response",
                    "strict": true,
                    "schema": schema
                }
            }),
        );
    }
    Ok(Value::Object(body))
}

#[derive(Debug)]
enum SseFrame {
    Data(Value),
    Done,
}

#[derive(Default)]
struct SseDecoder {
    buffer: Vec<u8>,
}

impl SseDecoder {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseFrame>, String> {
        self.buffer.extend_from_slice(bytes);
        let mut frames = Vec::new();
        while let Some((end, separator_len)) = sse_record_end(&self.buffer) {
            let record = self.buffer.drain(..end).collect::<Vec<_>>();
            self.buffer.drain(..separator_len);
            if let Some(frame) = decode_sse_record(&record)? {
                frames.push(frame);
            }
        }
        Ok(frames)
    }
}

#[derive(Default)]
struct StreamToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[derive(Default)]
struct OpenAiStreamAccumulator {
    text: String,
    tool_calls: BTreeMap<usize, StreamToolCall>,
    usage: Option<super::client::LlmUsageSnapshot>,
    finish_reason: ModelFinishReason,
    events: Vec<ModelTurnEvent>,
    raw_response: String,
    done: bool,
}

impl OpenAiStreamAccumulator {
    fn apply(&mut self, frame: SseFrame, sink: Option<&ModelTurnEventSink>) -> Result<(), String> {
        match frame {
            SseFrame::Done => {
                self.done = true;
                Ok(())
            }
            SseFrame::Data(value) => {
                append_bounded_raw_response(&mut self.raw_response, &value);
                if let Some(usage) = openai_usage_snapshot(&value) {
                    self.usage = Some(usage.clone());
                    self.emit(ModelTurnEvent::Usage { usage }, sink);
                }
                let Some(choice) = value
                    .get("choices")
                    .and_then(Value::as_array)
                    .and_then(|choices| choices.first())
                else {
                    return Ok(());
                };
                let delta = choice.get("delta").unwrap_or(&Value::Null);
                if let Some(text) = delta.get("content").and_then(Value::as_str) {
                    self.text.push_str(text);
                    self.emit(
                        ModelTurnEvent::TextDelta {
                            text: text.to_string(),
                        },
                        sink,
                    );
                }
                if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                    for (fallback_index, call) in calls.iter().enumerate() {
                        self.apply_tool_delta(call, fallback_index, sink)?;
                    }
                }
                if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                    self.finish_reason = openai_finish_reason(reason);
                }
                Ok(())
            }
        }
    }

    fn apply_tool_delta(
        &mut self,
        call: &Value,
        fallback_index: usize,
        sink: Option<&ModelTurnEventSink>,
    ) -> Result<(), String> {
        let index = call
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(fallback_index);
        let id = call
            .get("id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let name = call
            .pointer("/function/name")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let arguments_delta = call
            .pointer("/function/arguments")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let pending = self.tool_calls.entry(index).or_default();
        if id.is_some() {
            pending.id = id.clone();
        }
        if name.is_some() {
            pending.name = name.clone();
        }
        pending.arguments.push_str(&arguments_delta);
        self.emit(
            ModelTurnEvent::ToolCallDelta {
                index,
                id,
                name,
                arguments_delta,
            },
            sink,
        );
        Ok(())
    }

    fn finish(
        mut self,
        sink: Option<&ModelTurnEventSink>,
    ) -> Result<ModelTurnProviderResponse, String> {
        if !self.done {
            return Err("model_turn_stream_disconnected".to_string());
        }
        let mut completed_calls = Vec::new();
        let pending_calls = std::mem::take(&mut self.tool_calls);
        for (index, pending) in pending_calls {
            let name = pending
                .name
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| format!("model_turn_tool_name_missing index={index}"))?;
            let arguments = serde_json::from_str(&pending.arguments)
                .map_err(|_| format!("model_turn_tool_arguments_invalid index={index}"))?;
            let call = ModelToolCall {
                id: pending.id.unwrap_or_else(|| format!("tool_call_{index}")),
                name,
                arguments,
            };
            self.emit(ModelTurnEvent::ToolCall { call: call.clone() }, sink);
            completed_calls.push(call);
        }
        if self.text.is_empty() && completed_calls.is_empty() {
            return Err("model_turn_empty_response".to_string());
        }
        if self.finish_reason == ModelFinishReason::Unknown && !completed_calls.is_empty() {
            self.finish_reason = ModelFinishReason::ToolCalls;
        }
        self.emit(
            ModelTurnEvent::Finished {
                reason: self.finish_reason,
            },
            sink,
        );
        let turn = ModelTurnResponse {
            text: self.text,
            tool_calls: completed_calls,
            usage: self.usage,
            finish_reason: self.finish_reason,
            reasoning_metadata: BTreeMap::new(),
            events: self.events,
        };
        Ok(ModelTurnProviderResponse {
            turn,
            request_payload: Value::Null,
            raw_response: self.raw_response,
            attempts: 1,
            retryable_error_count: 0,
            last_retry_error_kind: None,
        })
    }

    fn emit(&mut self, event: ModelTurnEvent, sink: Option<&ModelTurnEventSink>) {
        if let Some(sink) = sink {
            sink(event.clone());
        }
        self.events.push(event);
    }
}

async fn read_openai_stream(
    response: reqwest::Response,
    request_payload: Value,
    event_sink: Option<ModelTurnEventSink>,
) -> Result<ModelTurnProviderResponse, ProviderError> {
    let mut decoder = SseDecoder::default();
    let mut accumulator = OpenAiStreamAccumulator {
        finish_reason: ModelFinishReason::Unknown,
        ..OpenAiStreamAccumulator::default()
    };
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            ProviderError::retryable_with_response(
                format!("model_turn_stream_read_failed:{error}"),
                request_payload.clone(),
                accumulator.raw_response.clone(),
                accumulator.usage.clone(),
            )
        })?;
        let frames = decoder.push(&chunk).map_err(|code| {
            ProviderError::non_retryable_with_response(
                code,
                request_payload.clone(),
                accumulator.raw_response.clone(),
                accumulator.usage.clone(),
            )
        })?;
        for frame in frames {
            accumulator
                .apply(frame, event_sink.as_ref())
                .map_err(|code| {
                    ProviderError::non_retryable_with_response(
                        code,
                        request_payload.clone(),
                        accumulator.raw_response.clone(),
                        accumulator.usage.clone(),
                    )
                })?;
        }
    }
    if !accumulator.done {
        return Err(ProviderError::retryable_with_response(
            "model_turn_stream_disconnected".to_string(),
            request_payload,
            accumulator.raw_response,
            accumulator.usage,
        ));
    }
    let finish_raw_response = accumulator.raw_response.clone();
    let finish_usage = accumulator.usage.clone();
    let mut output = accumulator.finish(event_sink.as_ref()).map_err(|code| {
        ProviderError::non_retryable_with_response(
            code,
            request_payload.clone(),
            finish_raw_response,
            finish_usage,
        )
    })?;
    output.request_payload = request_payload;
    Ok(output)
}

fn sse_record_end(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = buffer.windows(2).position(|window| window == b"\n\n");
    let crlf = buffer.windows(4).position(|window| window == b"\r\n\r\n");
    match (lf, crlf) {
        (Some(left), Some(right)) if left <= right => Some((left, 2)),
        (Some(_), Some(right)) => Some((right, 4)),
        (Some(left), None) => Some((left, 2)),
        (None, Some(right)) => Some((right, 4)),
        (None, None) => None,
    }
}

fn decode_sse_record(record: &[u8]) -> Result<Option<SseFrame>, String> {
    let text =
        std::str::from_utf8(record).map_err(|_| "model_turn_stream_utf8_invalid".to_string())?;
    let payload = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim_start)
        .collect::<Vec<_>>()
        .join("\n");
    if payload.is_empty() {
        return Ok(None);
    }
    if payload == "[DONE]" {
        return Ok(Some(SseFrame::Done));
    }
    serde_json::from_str(&payload)
        .map(SseFrame::Data)
        .map(Some)
        .map_err(|_| "model_turn_stream_json_invalid".to_string())
}

fn append_bounded_raw_response(raw: &mut String, value: &Value) {
    const RAW_RESPONSE_LIMIT: usize = 1024 * 1024;
    if raw.len() >= RAW_RESPONSE_LIMIT {
        return;
    }
    let encoded = provider_safe_raw_response(value);
    let remaining = RAW_RESPONSE_LIMIT.saturating_sub(raw.len());
    let mut end = encoded.len().min(remaining);
    while end > 0 && !encoded.is_char_boundary(end) {
        end -= 1;
    }
    raw.push_str(&encoded[..end]);
    raw.push('\n');
}

fn provider_safe_raw_response(value: &Value) -> String {
    let mut safe = value.clone();
    remove_hidden_reasoning_fields(&mut safe);
    safe.to_string()
}

fn remove_hidden_reasoning_fields(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for key in [
                "reasoning",
                "reasoning_content",
                "reasoning_details",
                "reasoning_text",
                "thinking",
            ] {
                map.remove(key);
            }
            for child in map.values_mut() {
                remove_hidden_reasoning_fields(child);
            }
        }
        Value::Array(values) => {
            for child in values {
                remove_hidden_reasoning_fields(child);
            }
        }
        _ => {}
    }
}

fn openai_messages(message: &ModelMessage) -> Vec<Value> {
    let role = match message.role {
        ModelRole::System => "system",
        ModelRole::User => "user",
        ModelRole::Assistant => "assistant",
        ModelRole::Tool => "tool",
    };
    let mut standard_parts = Vec::new();
    let mut tool_messages = Vec::new();
    for part in &message.content {
        match part {
            ModelContentPart::Text { text } => {
                standard_parts.push(json!({"type": "text", "text": text}));
            }
            ModelContentPart::Image { source, .. } => {
                standard_parts.push(json!({
                    "type": "image_url",
                    "image_url": {"url": source}
                }));
            }
            ModelContentPart::ToolResult {
                tool_call_id,
                content,
                ..
            } => tool_messages.push(json!({
                "role": "tool",
                "tool_call_id": tool_call_id,
                "content": content.to_string()
            })),
        }
    }
    let mut messages = Vec::new();
    if standard_parts.len() == 1
        && standard_parts[0].get("type").and_then(Value::as_str) == Some("text")
    {
        messages.push(json!({
            "role": role,
            "content": standard_parts[0].get("text").cloned().unwrap_or(Value::Null)
        }));
    } else if !standard_parts.is_empty() {
        messages.push(json!({"role": role, "content": standard_parts}));
    }
    messages.extend(tool_messages);
    messages
}

fn parse_openai_model_turn(value: &Value) -> Result<ModelTurnResponse, String> {
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| "model_turn_missing_choice".to_string())?;
    let message = choice
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(|| "model_turn_missing_message".to_string())?;
    let text = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let tool_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .enumerate()
                .map(|(index, call)| parse_tool_call(call, index))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    if text.is_empty() && tool_calls.is_empty() {
        return Err("model_turn_empty_response".to_string());
    }
    let usage = openai_usage_snapshot(value);
    let finish_reason = openai_finish_reason(
        choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
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

fn parse_tool_call(value: &Value, index: usize) -> Result<ModelToolCall, String> {
    let function = value
        .get("function")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("model_turn_tool_function_missing index={index}"))?;
    let name = function
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("model_turn_tool_name_missing index={index}"))?;
    let arguments = match function.get("arguments") {
        Some(Value::String(arguments)) => serde_json::from_str(arguments)
            .map_err(|_| format!("model_turn_tool_arguments_invalid index={index}"))?,
        Some(Value::Object(arguments)) => Value::Object(arguments.clone()),
        _ => return Err(format!("model_turn_tool_arguments_missing index={index}")),
    };
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("tool_call_{index}"));
    Ok(ModelToolCall {
        id,
        name: name.to_string(),
        arguments,
    })
}

fn openai_finish_reason(reason: &str) -> ModelFinishReason {
    match reason {
        "stop" => ModelFinishReason::Stop,
        "tool_calls" | "function_call" => ModelFinishReason::ToolCalls,
        "length" => ModelFinishReason::Length,
        "content_filter" => ModelFinishReason::ContentFilter,
        _ => ModelFinishReason::Unknown,
    }
}

#[cfg(test)]
#[path = "openai_model_turn_tests.rs"]
mod tests;
