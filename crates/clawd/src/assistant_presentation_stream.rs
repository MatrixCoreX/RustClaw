use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use claw_core::model_turn::{ModelToolCall, ModelTurnEvent};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{AppState, ClaimedTask};

const MAX_RESPOND_ARGUMENT_BYTES: usize = 512 * 1024;

#[derive(Default)]
struct IncrementalRespondParser {
    raw: String,
    failed: bool,
}

impl IncrementalRespondParser {
    fn push(&mut self, delta: &str) {
        if self.failed {
            return;
        }
        if self.raw.len().saturating_add(delta.len()) > MAX_RESPOND_ARGUMENT_BYTES {
            self.failed = true;
            self.raw.clear();
            return;
        }
        self.raw.push_str(delta);
    }

    fn terminal_answer(&self) -> Option<String> {
        if self.failed {
            return None;
        }
        let shape = match top_level_string_field(&self.raw, "shape") {
            FieldScan::Found(value) => value,
            FieldScan::Missing | FieldScan::Incomplete | FieldScan::Invalid => return None,
        };
        if shape != "free_text" {
            return None;
        }
        match top_level_string_field(&self.raw, "content") {
            FieldScan::Found(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
            FieldScan::Found(_)
            | FieldScan::Missing
            | FieldScan::Incomplete
            | FieldScan::Invalid => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FieldScan {
    Found(String),
    Missing,
    Incomplete,
    Invalid,
}

fn top_level_string_field(raw: &str, target: &str) -> FieldScan {
    let bytes = raw.as_bytes();
    let mut cursor = skip_ws(bytes, 0);
    if cursor >= bytes.len() {
        return FieldScan::Incomplete;
    }
    if bytes[cursor] != b'{' {
        return FieldScan::Invalid;
    }
    cursor += 1;

    loop {
        cursor = skip_ws(bytes, cursor);
        if cursor >= bytes.len() {
            return FieldScan::Incomplete;
        }
        if bytes[cursor] == b'}' {
            return FieldScan::Missing;
        }
        let (key, after_key) = match parse_json_string(raw, cursor) {
            ParsedString::Complete { value, end } => (value, end),
            ParsedString::Incomplete => return FieldScan::Incomplete,
            ParsedString::Invalid => return FieldScan::Invalid,
        };
        cursor = skip_ws(bytes, after_key);
        if cursor >= bytes.len() {
            return FieldScan::Incomplete;
        }
        if bytes[cursor] != b':' {
            return FieldScan::Invalid;
        }
        cursor = skip_ws(bytes, cursor + 1);
        if cursor >= bytes.len() {
            return FieldScan::Incomplete;
        }

        if key == target {
            return match parse_json_string(raw, cursor) {
                ParsedString::Complete { value, .. } => FieldScan::Found(value),
                ParsedString::Incomplete => FieldScan::Incomplete,
                ParsedString::Invalid => FieldScan::Invalid,
            };
        }

        cursor = match skip_json_value(raw, cursor) {
            ValueScan::Complete(end) => end,
            ValueScan::Incomplete => return FieldScan::Incomplete,
            ValueScan::Invalid => return FieldScan::Invalid,
        };
        cursor = skip_ws(bytes, cursor);
        if cursor >= bytes.len() {
            return FieldScan::Incomplete;
        }
        match bytes[cursor] {
            b',' => cursor += 1,
            b'}' => return FieldScan::Missing,
            _ => return FieldScan::Invalid,
        }
    }
}

enum ParsedString {
    Complete { value: String, end: usize },
    Incomplete,
    Invalid,
}

fn parse_json_string(raw: &str, start: usize) -> ParsedString {
    let bytes = raw.as_bytes();
    if bytes.get(start) != Some(&b'"') {
        return ParsedString::Invalid;
    }
    let mut cursor = start + 1;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\\' => {
                cursor += 1;
                if cursor >= bytes.len() {
                    return ParsedString::Incomplete;
                }
                if bytes[cursor] == b'u' {
                    if cursor.saturating_add(4) >= bytes.len() {
                        return ParsedString::Incomplete;
                    }
                    cursor += 4;
                }
            }
            b'"' => {
                let end = cursor + 1;
                return serde_json::from_str::<String>(&raw[start..end])
                    .map(|value| ParsedString::Complete { value, end })
                    .unwrap_or(ParsedString::Invalid);
            }
            byte if byte < 0x20 => return ParsedString::Invalid,
            _ => {}
        }
        cursor += 1;
    }
    ParsedString::Incomplete
}

enum ValueScan {
    Complete(usize),
    Incomplete,
    Invalid,
}

fn skip_json_value(raw: &str, start: usize) -> ValueScan {
    let bytes = raw.as_bytes();
    let Some(first) = bytes.get(start).copied() else {
        return ValueScan::Incomplete;
    };
    if first == b'"' {
        return match parse_json_string(raw, start) {
            ParsedString::Complete { end, .. } => ValueScan::Complete(end),
            ParsedString::Incomplete => ValueScan::Incomplete,
            ParsedString::Invalid => ValueScan::Invalid,
        };
    }
    if matches!(first, b'{' | b'[') {
        return skip_compound_json_value(raw, start);
    }
    let mut end = start;
    while end < bytes.len() && !matches!(bytes[end], b',' | b'}' | b']') {
        end += 1;
    }
    if end == bytes.len() {
        return ValueScan::Incomplete;
    }
    let token = raw[start..end].trim();
    if token.is_empty() || serde_json::from_str::<Value>(token).is_err() {
        ValueScan::Invalid
    } else {
        ValueScan::Complete(end)
    }
}

fn skip_compound_json_value(raw: &str, start: usize) -> ValueScan {
    let bytes = raw.as_bytes();
    let mut stack = vec![bytes[start]];
    let mut cursor = start + 1;
    let mut in_string = false;
    let mut escaped = false;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            cursor += 1;
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => stack.push(byte),
            b'}' => {
                if stack.pop() != Some(b'{') {
                    return ValueScan::Invalid;
                }
            }
            b']' => {
                if stack.pop() != Some(b'[') {
                    return ValueScan::Invalid;
                }
            }
            _ => {}
        }
        cursor += 1;
        if stack.is_empty() {
            return if serde_json::from_str::<Value>(&raw[start..cursor]).is_ok() {
                ValueScan::Complete(cursor)
            } else {
                ValueScan::Invalid
            };
        }
    }
    ValueScan::Incomplete
}

fn skip_ws(bytes: &[u8], mut cursor: usize) -> usize {
    while bytes
        .get(cursor)
        .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
    {
        cursor += 1;
    }
    cursor
}

#[derive(Default)]
struct PendingToolCall {
    name: Option<String>,
    parser: IncrementalRespondParser,
}

#[derive(Default)]
struct ObserverState {
    provider_attempt: usize,
    provider_started_at_ms: u64,
    provider_first_byte_at_ms: Option<u64>,
    published_stream: bool,
    calls: HashMap<usize, PendingToolCall>,
}

pub(crate) struct NativePresentationObserver {
    state: AppState,
    task: ClaimedTask,
    provider: String,
    logical_call_index: u64,
    inner: Mutex<ObserverState>,
}

impl NativePresentationObserver {
    pub(crate) fn new(
        state: AppState,
        task: ClaimedTask,
        provider: String,
        logical_call_index: u64,
    ) -> Self {
        Self {
            state,
            task,
            provider,
            logical_call_index,
            inner: Mutex::new(ObserverState::default()),
        }
    }

    pub(crate) fn observe(&self, event: &ModelTurnEvent) {
        match event {
            ModelTurnEvent::Started { attempt } => {
                let mut inner = self.inner.lock().unwrap();
                inner.provider_attempt = *attempt;
                inner.provider_started_at_ms = epoch_ms();
                inner.provider_first_byte_at_ms = None;
                inner.published_stream = false;
                inner.calls.clear();
            }
            ModelTurnEvent::TextDelta { .. } => {
                self.note_provider_byte();
            }
            ModelTurnEvent::ToolCallDelta {
                index,
                name,
                arguments_delta,
                ..
            } => {
                self.note_provider_byte();
                let candidate = {
                    let mut inner = self.inner.lock().unwrap();
                    let already_published = inner.published_stream;
                    let call = inner.calls.entry(*index).or_default();
                    if let Some(name) = name
                        .as_deref()
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                    {
                        call.name = Some(name.to_string());
                    }
                    call.parser.push(arguments_delta);
                    if already_published || call.name.as_deref() != Some("respond") {
                        None
                    } else {
                        call.parser
                            .terminal_answer()
                            .map(|content| (*index, content))
                    }
                };
                if let Some((index, content)) = candidate {
                    self.publish_answer(index, &content);
                }
            }
            ModelTurnEvent::ToolCall { call } => {
                self.note_provider_byte();
                if let Some(content) = final_respond_content(call) {
                    self.publish_answer(0, content);
                }
            }
            ModelTurnEvent::Interrupted { code, retryable } => {
                super::assistant_presentation::abort_active_answer(
                    &self.state,
                    &self.task,
                    code,
                    "assistant.output.provider_interrupted",
                    *retryable,
                );
            }
            ModelTurnEvent::Usage { .. } | ModelTurnEvent::Finished { .. } => {}
        }
    }

    fn note_provider_byte(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.provider_first_byte_at_ms.get_or_insert_with(epoch_ms);
    }

    fn publish_answer(&self, tool_index: usize, content: &str) {
        let (provider_attempt, provider_started_at_ms, provider_first_byte_at_ms) = {
            let mut inner = self.inner.lock().unwrap();
            if inner.published_stream {
                return;
            }
            inner.published_stream = true;
            (
                inner.provider_attempt,
                inner.provider_started_at_ms,
                inner.provider_first_byte_at_ms.unwrap_or_else(epoch_ms),
            )
        };
        let sanitized = crate::visible_text::sanitize_user_visible_text(content);
        if sanitized.trim().is_empty() {
            return;
        }
        let stream_id = provisional_stream_id(
            &self.task,
            &self.provider,
            self.logical_call_index,
            provider_attempt,
            tool_index,
        );
        super::assistant_presentation::publish_provisional_answer(
            &self.state,
            &self.task,
            &stream_id,
            &format!(
                "llm:{}:provider:{}",
                self.logical_call_index, provider_attempt
            ),
            &sanitized,
            provider_started_at_ms,
            provider_first_byte_at_ms,
        );
    }
}

fn final_respond_content(call: &ModelToolCall) -> Option<&str> {
    if call.name != "respond" {
        return None;
    }
    let arguments = call.arguments.as_object()?;
    if arguments.get("shape").and_then(Value::as_str) != Some("free_text") {
        return None;
    }
    arguments
        .get("content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|content| !content.is_empty())
}

fn provisional_stream_id(
    task: &ClaimedTask,
    provider: &str,
    logical_call_index: u64,
    provider_attempt: usize,
    tool_index: usize,
) -> String {
    let digest = Sha256::digest(
        format!(
            "{task_id}:{claim_attempt}:{provider}:{logical_call_index}:{provider_attempt}:{tool_index}",
            task_id = task.task_id,
            claim_attempt = task.claim_attempt,
        )
        .as_bytes(),
    );
    format!("assistant:{digest:x}")
}

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "assistant_presentation_stream_tests.rs"]
mod tests;
