use std::collections::HashMap;

use anyhow::{anyhow, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

const PRESENTATION_KINDS: &[&str] = &[
    "assistant_output_started",
    "assistant_output_delta",
    "assistant_output_completed",
    "assistant_output_aborted",
    "assistant_output_replaced",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AssistantPresentationEvent {
    pub(crate) kind: String,
    pub(crate) task_id: String,
    pub(crate) stream_id: String,
    pub(crate) attempt_id: String,
    pub(crate) sequence: u64,
    pub(crate) content_offset_bytes: u64,
    pub(crate) content: Option<String>,
    pub(crate) total_content_bytes: Option<u64>,
    pub(crate) content_sha256: Option<String>,
    pub(crate) error_code: Option<String>,
    pub(crate) retryable: Option<bool>,
    pub(crate) old_stream_id: Option<String>,
    pub(crate) new_stream_id: Option<String>,
}

pub(crate) fn decode(event: &Value) -> Result<Option<AssistantPresentationEvent>> {
    let Some(kind) = event
        .get("event_kind")
        .or_else(|| event.get("event_type"))
        .and_then(Value::as_str)
        .map(str::trim)
    else {
        return Ok(None);
    };
    if !PRESENTATION_KINDS.contains(&kind) {
        return Ok(None);
    }
    let payload = event
        .get("payload")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("assistant_presentation_payload_invalid"))?;
    if payload.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return Err(anyhow!("assistant_presentation_schema_version_invalid"));
    }
    let outer_task_id = required_token(event, "task_id")?;
    let task_id = required_payload_token(payload, "task_id")?;
    if task_id != outer_task_id {
        return Err(anyhow!("assistant_presentation_identity_mismatch"));
    }
    required_payload_token(payload, "conversation_id")?;
    required_payload_token(payload, "turn_id")?;
    let stream_id = required_payload_token(payload, "stream_id")?;
    let attempt_id = required_payload_token(payload, "attempt_id")?;
    let sequence = required_payload_u64(payload, "sequence")?;
    let content_offset_bytes = required_payload_u64(payload, "content_offset_bytes")?;
    required_payload_u64(payload, "created_at")?;

    let mut content = None;
    let mut total_content_bytes = None;
    let mut content_sha256 = None;
    let mut error_code = None;
    let mut retryable = None;
    let mut old_stream_id = None;
    let mut new_stream_id = None;
    match kind {
        "assistant_output_started" => {
            if sequence != 0 || content_offset_bytes != 0 {
                return Err(anyhow!("assistant_presentation_start_invalid"));
            }
        }
        "assistant_output_delta" => {
            content = Some(
                payload
                    .get("content")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("assistant_presentation_content_invalid"))?
                    .to_string(),
            );
        }
        "assistant_output_completed" => {
            let total = required_payload_u64(payload, "total_content_bytes")?;
            if total != content_offset_bytes {
                return Err(anyhow!("assistant_presentation_completion_size_mismatch"));
            }
            let digest = required_payload_token(payload, "content_sha256")?;
            let hex = digest
                .strip_prefix("sha256:")
                .ok_or_else(|| anyhow!("assistant_presentation_digest_invalid"))?;
            if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(anyhow!("assistant_presentation_digest_invalid"));
            }
            total_content_bytes = Some(total);
            content_sha256 = Some(digest.to_ascii_lowercase());
        }
        "assistant_output_aborted" => {
            error_code = Some(required_payload_token(payload, "error_code")?);
            required_payload_token(payload, "message_key")?;
            if !payload.get("retryable").is_some_and(Value::is_boolean) {
                return Err(anyhow!("assistant_presentation_retryable_invalid"));
            }
            retryable = payload.get("retryable").and_then(Value::as_bool);
        }
        "assistant_output_replaced" => {
            old_stream_id = Some(required_payload_token(payload, "old_stream_id")?);
            new_stream_id = Some(required_payload_token(payload, "new_stream_id")?);
        }
        _ => unreachable!(),
    }

    Ok(Some(AssistantPresentationEvent {
        kind: kind.to_string(),
        task_id,
        stream_id,
        attempt_id,
        sequence,
        content_offset_bytes,
        content,
        total_content_bytes,
        content_sha256,
        error_code,
        retryable,
        old_stream_id,
        new_stream_id,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamStatus {
    Streaming,
    Completed,
    Aborted,
    Replaced,
}

#[derive(Debug, Clone)]
struct StreamState {
    status: StreamStatus,
    content: String,
    content_bytes: u64,
    next_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PresentationUpdate {
    Started,
    Delta(String),
    Completed,
    Aborted,
    Replaced,
    Duplicate,
}

#[derive(Default)]
pub(crate) struct AssistantPresentationReducer {
    streams: HashMap<String, StreamState>,
    seen: HashMap<(String, u64), AssistantPresentationEvent>,
    latest_stream_id: Option<String>,
    latest_completed_content: Option<String>,
}

impl AssistantPresentationReducer {
    pub(crate) fn apply(
        &mut self,
        event: AssistantPresentationEvent,
    ) -> Result<PresentationUpdate> {
        let key = (event.stream_id.clone(), event.sequence);
        if let Some(previous) = self.seen.get(&key) {
            if previous == &event {
                return Ok(PresentationUpdate::Duplicate);
            }
            return Err(anyhow!("assistant_presentation_duplicate_conflict"));
        }

        if event.kind == "assistant_output_replaced" {
            let old_stream_id = event
                .old_stream_id
                .as_deref()
                .ok_or_else(|| anyhow!("assistant_presentation_old_stream_id_invalid"))?;
            if let Some(stream) = self.streams.get_mut(old_stream_id) {
                stream.status = StreamStatus::Replaced;
            }
            self.seen.insert(key, event);
            return Ok(PresentationUpdate::Replaced);
        }

        if event.kind == "assistant_output_started" {
            if self.streams.contains_key(&event.stream_id) {
                return Err(anyhow!("assistant_presentation_stream_conflict"));
            }
            self.streams.insert(
                event.stream_id.clone(),
                StreamState {
                    status: StreamStatus::Streaming,
                    content: String::new(),
                    content_bytes: 0,
                    next_sequence: 1,
                },
            );
            self.latest_stream_id = Some(event.stream_id.clone());
            self.seen.insert(key, event);
            return Ok(PresentationUpdate::Started);
        }

        let stream = self
            .streams
            .get_mut(&event.stream_id)
            .ok_or_else(|| anyhow!("assistant_presentation_start_missing"))?;
        if stream.status != StreamStatus::Streaming {
            return Err(anyhow!("assistant_presentation_stream_terminal"));
        }
        if event.sequence != stream.next_sequence {
            return Err(anyhow!("assistant_presentation_sequence_gap"));
        }
        if event.content_offset_bytes != stream.content_bytes {
            return Err(anyhow!("assistant_presentation_offset_mismatch"));
        }

        let update = match event.kind.as_str() {
            "assistant_output_delta" => {
                let content = event.content.clone().unwrap_or_default();
                stream.content.push_str(&content);
                stream.content_bytes = stream
                    .content_bytes
                    .saturating_add(u64::try_from(content.len()).unwrap_or(u64::MAX));
                PresentationUpdate::Delta(content)
            }
            "assistant_output_completed" => {
                if event.total_content_bytes != Some(stream.content_bytes) {
                    return Err(anyhow!("assistant_presentation_completion_size_mismatch"));
                }
                let expected = format!("sha256:{:x}", Sha256::digest(stream.content.as_bytes()));
                if event.content_sha256.as_deref() != Some(expected.as_str()) {
                    return Err(anyhow!("assistant_presentation_digest_mismatch"));
                }
                stream.status = StreamStatus::Completed;
                self.latest_completed_content = Some(stream.content.clone());
                PresentationUpdate::Completed
            }
            "assistant_output_aborted" => {
                stream.status = StreamStatus::Aborted;
                PresentationUpdate::Aborted
            }
            _ => return Err(anyhow!("assistant_presentation_kind_invalid")),
        };
        stream.next_sequence = stream.next_sequence.saturating_add(1);
        self.seen.insert(key, event);
        Ok(update)
    }

    pub(crate) fn completed_matches(&self, final_text: Option<&str>) -> bool {
        self.latest_completed_content.as_deref() == final_text
    }

    pub(crate) fn latest_display_content(&self) -> Option<&str> {
        let stream = self.streams.get(self.latest_stream_id.as_deref()?)?;
        matches!(
            stream.status,
            StreamStatus::Streaming | StreamStatus::Completed
        )
        .then_some(stream.content.as_str())
    }
}

fn required_token(value: &Value, key: &str) -> Result<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 512)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("assistant_presentation_{key}_invalid"))
}

fn required_payload_token(payload: &serde_json::Map<String, Value>, key: &str) -> Result<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 512)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("assistant_presentation_{key}_invalid"))
}

fn required_payload_u64(payload: &serde_json::Map<String, Value>, key: &str) -> Result<u64> {
    payload
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("assistant_presentation_{key}_invalid"))
}

#[cfg(test)]
#[path = "assistant_presentation_tests.rs"]
mod tests;
