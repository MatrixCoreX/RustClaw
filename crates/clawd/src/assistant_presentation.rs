use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{AppState, ClaimedTask};

const PRESENTATION_SCHEMA_VERSION: u64 = 1;
const TERMINAL_DELTA_MAX_BYTES: usize = 12 * 1024;

pub(crate) fn publish_terminal_answer(state: &AppState, task: &ClaimedTask, answer_text: &str) {
    let content = crate::visible_text::sanitize_user_visible_text(answer_text);
    if complete_or_replace_provisional_answer(state, task, &content) {
        return;
    }
    for (event_kind, payload) in terminal_answer_events(task, answer_text, TERMINAL_DELTA_MAX_BYTES)
    {
        if let Err(error) =
            crate::task_event_transport::publish_event(state, &task.task_id, event_kind, payload)
        {
            tracing::warn!(
                event = "assistant_presentation_publish_failed",
                task_id = %task.task_id,
                event_kind,
                error = %crate::truncate_for_log(&error.to_string())
            );
            break;
        }
    }
}

pub(crate) fn publish_provisional_answer(
    state: &AppState,
    task: &ClaimedTask,
    stream_id: &str,
    attempt_id: &str,
    answer_text: &str,
    provider_started_at_ms: u64,
    provider_first_byte_at_ms: u64,
) {
    let content = crate::visible_text::sanitize_user_visible_text(answer_text);
    if content.trim().is_empty() {
        return;
    }
    let published_at_ms = epoch_ms();
    let common = presentation_common(task, stream_id, attempt_id, "provisional_low_latency");
    let replay = replay_presentation_state(state, &task.task_id);
    if let Some(previous) = replay.pending_aborted {
        let replacement = with_fields(
            &previous.common,
            json!({
                "sequence": previous.next_sequence,
                "content_offset_bytes": previous.content.len(),
                "old_stream_id": previous.stream_id,
                "new_stream_id": stream_id,
                "stream_replacement_count": replay.replacement_count.saturating_add(1),
            }),
        );
        publish_claimed_payload(state, task, "assistant_output_replaced", replacement);
    }
    publish_claimed_payload(
        state,
        task,
        "assistant_output_started",
        with_fields(
            &common,
            json!({
                "sequence": 0,
                "content_offset_bytes": 0,
                "provider_first_byte_elapsed_ms":
                    provider_first_byte_at_ms.saturating_sub(provider_started_at_ms),
                "first_public_byte_elapsed_ms":
                    published_at_ms.saturating_sub(provider_started_at_ms),
                "provider_started_at_ms": provider_started_at_ms,
            }),
        ),
    );
    let mut sequence = 1u64;
    let mut offset = 0usize;
    for delta in utf8_chunks(&content, TERMINAL_DELTA_MAX_BYTES) {
        publish_claimed_payload(
            state,
            task,
            "assistant_output_delta",
            with_fields(
                &common,
                json!({
                    "sequence": sequence,
                    "content_offset_bytes": offset,
                    "content": delta,
                }),
            ),
        );
        sequence += 1;
        offset += delta.len();
    }
}

pub(crate) fn abort_active_answer(
    state: &AppState,
    task: &ClaimedTask,
    error_code: &str,
    message_key: &str,
    retryable: bool,
) {
    let replay = replay_presentation_state(state, &task.task_id);
    let Some(active) = replay.active else {
        return;
    };
    let mut payload = aborted_payload(&active, error_code, message_key, retryable);
    insert_field(
        &mut payload,
        "stream_abort_count",
        json!(replay.abort_count.saturating_add(1)),
    );
    publish_claimed_payload(state, task, "assistant_output_aborted", payload);
}

pub(crate) fn abort_for_verifier_retry(state: &AppState, task: &ClaimedTask) {
    abort_active_answer(
        state,
        task,
        "answer_verifier_retry",
        "assistant.output.verifier_retry",
        true,
    );
}

fn terminal_answer_events(
    task: &ClaimedTask,
    answer_text: &str,
    max_delta_bytes: usize,
) -> Vec<(&'static str, Value)> {
    let content = crate::visible_text::sanitize_user_visible_text(answer_text);
    let content_sha256 = sha256_label(content.as_bytes());
    let stream_id = presentation_stream_id(task, &content_sha256);
    let attempt_id = format!("claim:{}", task.claim_attempt.max(0));
    let conversation_id = presentation_conversation_id(task);
    let turn_id = presentation_turn_id(task);
    let created_at = crate::now_ts_u64();
    let common = json!({
        "schema_version": PRESENTATION_SCHEMA_VERSION,
        "task_id": task.task_id,
        "conversation_id": conversation_id,
        "turn_id": turn_id,
        "stream_id": stream_id,
        "attempt_id": attempt_id,
        "created_at": created_at,
        "publication_mode": "terminal_only",
        "fallback_reason": "terminal_safe_point",
    });
    let mut events = vec![(
        "assistant_output_started",
        with_fields(
            &common,
            json!({
                "sequence": 0,
                "content_offset_bytes": 0,
            }),
        ),
    )];
    let mut offset = 0usize;
    let mut sequence = 1u64;
    for delta in utf8_chunks(&content, max_delta_bytes.max(1)) {
        events.push((
            "assistant_output_delta",
            with_fields(
                &common,
                json!({
                    "sequence": sequence,
                    "content_offset_bytes": offset,
                    "content": delta,
                }),
            ),
        ));
        offset += delta.len();
        sequence += 1;
    }
    events.push((
        "assistant_output_completed",
        with_fields(
            &common,
            json!({
                "sequence": sequence,
                "content_offset_bytes": offset,
                "total_content_bytes": offset,
                "content_sha256": content_sha256,
            }),
        ),
    ));
    events
}

fn complete_or_replace_provisional_answer(
    state: &AppState,
    task: &ClaimedTask,
    final_content: &str,
) -> bool {
    let replay = replay_presentation_state(state, &task.task_id);
    let Some(active) = replay.active else {
        return false;
    };
    let digest = sha256_label(final_content.as_bytes());
    if active.content == final_content {
        let terminal_result_elapsed_ms = active
            .provider_started_at_ms
            .map(|started| epoch_ms().saturating_sub(started));
        let mut fields = json!({
            "sequence": active.next_sequence,
            "content_offset_bytes": active.content.len(),
            "total_content_bytes": active.content.len(),
            "content_sha256": digest,
        });
        if let (Some(object), Some(elapsed)) = (fields.as_object_mut(), terminal_result_elapsed_ms)
        {
            object.insert("terminal_result_elapsed_ms".to_string(), json!(elapsed));
        }
        publish_payload(
            state,
            &task.task_id,
            "assistant_output_completed",
            with_fields(&active.common, fields),
        );
        return true;
    }

    let mut abort = aborted_payload(
        &active,
        "assistant_output_final_mismatch",
        "assistant.output.final_mismatch",
        false,
    );
    insert_field(
        &mut abort,
        "stream_abort_count",
        json!(replay.abort_count.saturating_add(1)),
    );
    publish_payload(state, &task.task_id, "assistant_output_aborted", abort);
    let terminal_events = terminal_answer_events(task, final_content, TERMINAL_DELTA_MAX_BYTES);
    let new_stream_id = terminal_events
        .first()
        .and_then(|(_, payload)| payload.get("stream_id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    publish_payload(
        state,
        &task.task_id,
        "assistant_output_replaced",
        with_fields(
            &active.common,
            json!({
                "sequence": active.next_sequence.saturating_add(1),
                "content_offset_bytes": active.content.len(),
                "old_stream_id": active.stream_id,
                "new_stream_id": new_stream_id,
                "stream_replacement_count": replay.replacement_count.saturating_add(1),
            }),
        ),
    );
    for (event_kind, payload) in terminal_events {
        publish_payload(state, &task.task_id, event_kind, payload);
    }
    true
}

fn aborted_payload(
    active: &ReplayStream,
    error_code: &str,
    message_key: &str,
    retryable: bool,
) -> Value {
    with_fields(
        &active.common,
        json!({
            "sequence": active.next_sequence,
            "content_offset_bytes": active.content.len(),
            "error_code": error_code,
            "message_key": message_key,
            "retryable": retryable,
        }),
    )
}

fn presentation_common(
    task: &ClaimedTask,
    stream_id: &str,
    attempt_id: &str,
    publication_mode: &str,
) -> Value {
    json!({
        "schema_version": PRESENTATION_SCHEMA_VERSION,
        "task_id": task.task_id,
        "conversation_id": presentation_conversation_id(task),
        "turn_id": presentation_turn_id(task),
        "stream_id": stream_id,
        "attempt_id": attempt_id,
        "created_at": crate::now_ts_u64(),
        "publication_mode": publication_mode,
    })
}

fn publish_claimed_payload(
    state: &AppState,
    task: &ClaimedTask,
    event_kind: &'static str,
    payload: Value,
) {
    if let Err(error) =
        crate::task_event_transport::publish_claimed_event(state, task, event_kind, payload)
    {
        tracing::warn!(
            event = "assistant_presentation_publish_failed",
            task_id = %task.task_id,
            event_kind,
            error = %crate::truncate_for_log(&error.to_string())
        );
    }
}

fn publish_payload(state: &AppState, task_id: &str, event_kind: &'static str, payload: Value) {
    if let Err(error) =
        crate::task_event_transport::publish_event(state, task_id, event_kind, payload)
    {
        tracing::warn!(
            event = "assistant_presentation_publish_failed",
            task_id,
            event_kind,
            error = %crate::truncate_for_log(&error.to_string())
        );
    }
}

#[derive(Clone)]
struct ReplayStream {
    stream_id: String,
    common: Value,
    next_sequence: u64,
    content: String,
    provider_started_at_ms: Option<u64>,
}

#[derive(Default)]
struct PresentationReplay {
    active: Option<ReplayStream>,
    pending_aborted: Option<ReplayStream>,
    abort_count: u64,
    replacement_count: u64,
}

fn replay_presentation_state(state: &AppState, task_id: &str) -> PresentationReplay {
    let Ok(batch) = crate::task_event_transport::replay_events_after(state, task_id, 0) else {
        return PresentationReplay::default();
    };
    let mut replay = PresentationReplay::default();
    for event in batch.events {
        let Some(kind) = event
            .get("event_kind")
            .or_else(|| event.get("event_type"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(payload) = event.get("payload") else {
            continue;
        };
        let Some(stream_id) = payload.get("stream_id").and_then(Value::as_str) else {
            continue;
        };
        match kind {
            "assistant_output_started" => {
                if payload.get("sequence").and_then(Value::as_u64) != Some(0)
                    || payload.get("content_offset_bytes").and_then(Value::as_u64) != Some(0)
                {
                    continue;
                }
                replay.active = Some(ReplayStream {
                    stream_id: stream_id.to_string(),
                    common: presentation_common_from_payload(payload),
                    next_sequence: 1,
                    content: String::new(),
                    provider_started_at_ms: payload
                        .get("provider_started_at_ms")
                        .and_then(Value::as_u64),
                });
            }
            "assistant_output_delta" => {
                let Some(active) = replay
                    .active
                    .as_mut()
                    .filter(|active| active.stream_id == stream_id)
                else {
                    continue;
                };
                if payload.get("sequence").and_then(Value::as_u64) != Some(active.next_sequence)
                    || payload.get("content_offset_bytes").and_then(Value::as_u64)
                        != u64::try_from(active.content.len()).ok()
                {
                    replay.active = None;
                    continue;
                }
                let Some(content) = payload.get("content").and_then(Value::as_str) else {
                    replay.active = None;
                    continue;
                };
                active.content.push_str(content);
                active.next_sequence += 1;
            }
            "assistant_output_aborted" => {
                replay.abort_count = replay.abort_count.saturating_add(1);
                if replay
                    .active
                    .as_ref()
                    .is_some_and(|active| active.stream_id == stream_id)
                {
                    let mut aborted = replay.active.take().expect("checked active stream");
                    aborted.next_sequence = aborted.next_sequence.saturating_add(1);
                    replay.pending_aborted = Some(aborted);
                }
            }
            "assistant_output_completed" => {
                if replay
                    .active
                    .as_ref()
                    .is_some_and(|active| active.stream_id == stream_id)
                {
                    replay.active = None;
                }
            }
            "assistant_output_replaced" => {
                replay.replacement_count = replay.replacement_count.saturating_add(1);
                if replay.pending_aborted.as_ref().is_some_and(|aborted| {
                    payload.get("old_stream_id").and_then(Value::as_str)
                        == Some(aborted.stream_id.as_str())
                }) {
                    replay.pending_aborted = None;
                }
            }
            _ => {}
        }
    }
    replay
}

fn presentation_common_from_payload(payload: &Value) -> Value {
    let mut common = json!({});
    let Some(object) = common.as_object_mut() else {
        return common;
    };
    for key in [
        "schema_version",
        "task_id",
        "conversation_id",
        "turn_id",
        "stream_id",
        "attempt_id",
        "created_at",
        "publication_mode",
    ] {
        if let Some(value) = payload.get(key) {
            object.insert(key.to_string(), value.clone());
        }
    }
    if let Some(value) = payload.get("provider_started_at_ms") {
        object.insert("provider_started_at_ms".to_string(), value.clone());
    }
    common
}

fn with_fields(common: &Value, fields: Value) -> Value {
    let mut value = common.clone();
    let Some(object) = value.as_object_mut() else {
        return fields;
    };
    if let Some(fields) = fields.as_object() {
        object.extend(fields.clone());
    }
    value
}

fn insert_field(value: &mut Value, key: &str, field: Value) {
    if let Some(object) = value.as_object_mut() {
        object.insert(key.to_string(), field);
    }
}

fn utf8_chunks(text: &str, max_bytes: usize) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < text.len() {
        let mut end = start.saturating_add(max_bytes).min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            end = text[start..]
                .char_indices()
                .nth(1)
                .map(|(offset, _)| start + offset)
                .unwrap_or(text.len());
        }
        chunks.push(&text[start..end]);
        start = end;
    }
    chunks
}

fn presentation_stream_id(task: &ClaimedTask, content_sha256: &str) -> String {
    let digest = Sha256::digest(
        format!("{}:{}:{}", task.task_id, task.claim_attempt, content_sha256).as_bytes(),
    );
    format!("assistant:{digest:x}")
}

fn presentation_conversation_id(task: &ClaimedTask) -> String {
    let payload = serde_json::from_str::<Value>(&task.payload_json).unwrap_or(Value::Null);
    payload_ref(
        &payload,
        &["conversation_id", "thread_id", "session_id", "thread_ref"],
    )
    .unwrap_or_else(|| format!("{}:{}", normalize_ref(&task.channel), task.chat_id))
}

fn presentation_turn_id(task: &ClaimedTask) -> String {
    let payload = serde_json::from_str::<Value>(&task.payload_json).unwrap_or(Value::Null);
    payload_ref(&payload, &["turn_id", "message_id", "request_id"])
        .unwrap_or_else(|| task.task_id.clone())
}

fn payload_ref(value: &Value, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty() && value.len() <= 256)
            .map(normalize_ref)
            .filter(|value| !value.is_empty())
    })
}

fn normalize_ref(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn sha256_label(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "assistant_presentation_tests.rs"]
mod tests;
