use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;

use crate::{AppState, ClaimedTask};

const EVENT_SCHEMA_VERSION: u64 = 1;
const EVENT_REPLAY_LIMIT: u64 = 1024;
const EVENT_MAX_BYTES: usize = 64 * 1024;
const NOTIFIER_CAPACITY: usize = 256;
const MAX_NOTIFIER_TASKS: usize = 4096;

const INIT_TASK_EVENT_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS task_event_stream (
    task_id       TEXT NOT NULL,
    seq           INTEGER NOT NULL,
    event_hash    TEXT NOT NULL,
    event_json    TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (task_id, seq),
    UNIQUE (task_id, event_hash)
);
CREATE INDEX IF NOT EXISTS idx_task_event_stream_task_seq
    ON task_event_stream(task_id, seq);
CREATE TABLE IF NOT EXISTS task_event_artifacts (
    task_id       TEXT NOT NULL,
    artifact_id   TEXT NOT NULL,
    payload_json  TEXT NOT NULL,
    payload_bytes INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (task_id, artifact_id)
);
"#;

#[derive(Clone)]
pub(crate) struct TaskEventNotifier {
    senders: Arc<Mutex<HashMap<String, broadcast::Sender<u64>>>>,
}

impl Default for TaskEventNotifier {
    fn default() -> Self {
        Self {
            senders: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl TaskEventNotifier {
    pub(crate) fn subscribe(&self, task_id: &str) -> broadcast::Receiver<u64> {
        self.sender(task_id).subscribe()
    }

    fn notify(&self, task_id: &str, seq: u64) {
        let _ = self.sender(task_id).send(seq);
    }

    fn sender(&self, task_id: &str) -> broadcast::Sender<u64> {
        let mut senders = self.senders.lock().unwrap();
        if let Some(sender) = senders.get(task_id) {
            return sender.clone();
        }
        if senders.len() >= MAX_NOTIFIER_TASKS {
            let removable = senders
                .iter()
                .find_map(|(id, sender)| (sender.receiver_count() == 0).then(|| id.clone()));
            if let Some(id) = removable {
                senders.remove(&id);
            }
        }
        let (sender, _) = broadcast::channel(NOTIFIER_CAPACITY);
        senders.insert(task_id.to_string(), sender.clone());
        sender
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ReplayBatch {
    pub(crate) events: Vec<Value>,
    pub(crate) cursor_expired: bool,
    pub(crate) oldest_seq: Option<u64>,
    pub(crate) newest_seq: Option<u64>,
}

pub(crate) fn publish_event(
    state: &AppState,
    task_id: &str,
    event_kind: &str,
    payload: Value,
) -> anyhow::Result<Option<Value>> {
    let event_kind = normalize_machine_token(event_kind).context("invalid task event kind")?;
    let mut payload = payload;
    let redacted_fields = redact_event_value(&mut payload, None, 0);
    let timestamp_ms = now_ms();
    let mut context = event_context(&payload);
    context.fill_missing(task_payload_event_context(state, task_id));
    let (payload, artifact_refs) =
        persist_large_payload_if_needed(state, task_id, &event_kind, payload, timestamp_ms)?;
    let fingerprint_source = json!({
        "event_kind": event_kind,
        "payload": payload,
        "thread_id": context.thread_id,
        "session_id": context.session_id,
        "parent_task_id": context.parent_task_id,
        "child_task_id": context.child_task_id,
    });
    let event_hash = value_hash(&fingerprint_source)?;

    let mut db = state.core.db.get().context("task event db pool")?;
    db.execute_batch(INIT_TASK_EVENT_SQL)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(existing) = tx
        .query_row(
            "SELECT event_json FROM task_event_stream WHERE task_id = ?1 AND event_hash = ?2",
            params![task_id, event_hash],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        tx.commit()?;
        return Ok(serde_json::from_str(&existing).ok());
    }
    let seq = tx.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM task_event_stream WHERE task_id = ?1",
        params![task_id],
        |row| row.get::<_, u64>(0),
    )?;
    let event = json!({
        "schema_version": EVENT_SCHEMA_VERSION,
        "seq": seq,
        "timestamp_ms": timestamp_ms,
        "task_id": task_id,
        "thread_id": context.thread_id,
        "session_id": context.session_id,
        "parent_task_id": context.parent_task_id,
        "child_task_id": context.child_task_id,
        "event_kind": event_kind,
        "event_type": event_kind,
        "payload": payload,
        "redaction": {
            "applied": redacted_fields > 0,
            "field_count": redacted_fields,
        },
        "artifact_refs": artifact_refs,
    });
    let serialized = serde_json::to_string(&event)?;
    tx.execute(
        "INSERT INTO task_event_stream(task_id, seq, event_hash, event_json, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![task_id, seq, event_hash, serialized, timestamp_ms],
    )?;
    tx.execute(
        "DELETE FROM task_event_stream WHERE task_id = ?1 AND seq <= ?2",
        params![task_id, seq.saturating_sub(EVENT_REPLAY_LIMIT)],
    )?;
    tx.commit()?;
    state.metrics.task_event_notifier.notify(task_id, seq);
    Ok(Some(event))
}

pub(crate) fn replay_events_after(
    state: &AppState,
    task_id: &str,
    cursor: u64,
) -> anyhow::Result<ReplayBatch> {
    let db = state.core.db.get().context("task event db pool")?;
    db.execute_batch(INIT_TASK_EVENT_SQL)?;
    let (oldest_seq, newest_seq) = db.query_row(
        "SELECT MIN(seq), MAX(seq) FROM task_event_stream WHERE task_id = ?1",
        params![task_id],
        |row| Ok((row.get::<_, Option<u64>>(0)?, row.get::<_, Option<u64>>(1)?)),
    )?;
    let cursor_expired = oldest_seq.is_some_and(|oldest| cursor > 0 && cursor < oldest - 1);
    let effective_cursor = if cursor_expired {
        oldest_seq.unwrap_or(1).saturating_sub(1)
    } else {
        cursor
    };
    let mut statement = db.prepare(
        "SELECT event_json FROM task_event_stream
         WHERE task_id = ?1 AND seq > ?2 ORDER BY seq ASC LIMIT ?3",
    )?;
    let rows = statement.query_map(
        params![task_id, effective_cursor, EVENT_REPLAY_LIMIT],
        |row| row.get::<_, String>(0),
    )?;
    let mut events = Vec::new();
    for row in rows {
        if let Ok(event) = serde_json::from_str::<Value>(&row?) {
            events.push(event);
        }
    }
    Ok(ReplayBatch {
        events,
        cursor_expired,
        oldest_seq,
        newest_seq,
    })
}

pub(crate) fn read_event_artifact(
    state: &AppState,
    task_id: &str,
    artifact_id: &str,
) -> anyhow::Result<Option<Value>> {
    if !valid_artifact_id(artifact_id) {
        return Ok(None);
    }
    let db = state.core.db.get().context("task event artifact db pool")?;
    db.execute_batch(INIT_TASK_EVENT_SQL)?;
    let payload = db
        .query_row(
            "SELECT payload_json FROM task_event_artifacts
             WHERE task_id = ?1 AND artifact_id = ?2",
            params![task_id, artifact_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(payload.and_then(|value| serde_json::from_str(&value).ok()))
}

pub(crate) fn publish_journal_snapshot(
    state: &AppState,
    journal: &crate::task_journal::TaskJournal,
) -> anyhow::Result<usize> {
    let Some(task_id) = journal.task_id.as_deref() else {
        return Ok(0);
    };
    let mut published = 0;
    for event in journal.event_stream_snapshot() {
        let Some(kind) = event.get("event_type").and_then(Value::as_str) else {
            continue;
        };
        let payload = event.get("payload").cloned().unwrap_or(Value::Null);
        let before = replay_events_after(state, task_id, 0)?
            .newest_seq
            .unwrap_or(0);
        let result = publish_event(state, task_id, kind, payload)?;
        let after = result
            .as_ref()
            .and_then(|value| value.get("seq"))
            .and_then(Value::as_u64)
            .unwrap_or(before);
        published += usize::from(after > before);
    }
    Ok(published)
}

pub(crate) fn publish_loop_state_snapshot(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
) {
    let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", user_text);
    journal.rounds = loop_state.round_traces.clone();
    for step in &loop_state.executed_step_results {
        journal.push_step_result(step);
    }
    journal.task_observations = loop_state.task_observations.clone();
    journal.task_lifecycle = loop_state.task_lifecycle.clone();
    journal.task_checkpoint = loop_state.task_checkpoint.clone();
    if let Some(contract) = loop_state.output_contract.as_ref() {
        journal.record_output_contract(contract);
    }
    if let Err(error) = publish_journal_snapshot(state, &journal) {
        tracing::warn!(
            "task event snapshot publish failed task_id={} error={}",
            task.task_id,
            crate::truncate_for_log(&error.to_string())
        );
    }
}

pub(crate) fn publish_persisted_task_events(state: &AppState, task_id: &str) {
    let Ok(uuid) = uuid::Uuid::parse_str(task_id) else {
        return;
    };
    let Ok(Some((task, _, _))) = crate::repo::get_task_query_record(state, uuid) else {
        return;
    };
    let Some(events) = task
        .result_json
        .as_ref()
        .and_then(|value| value.pointer("/task_journal/trace/event_stream"))
        .and_then(Value::as_array)
    else {
        return;
    };
    for event in events {
        let Some(kind) = event.get("event_type").and_then(Value::as_str) else {
            continue;
        };
        let payload = event.get("payload").cloned().unwrap_or(Value::Null);
        if let Err(error) = publish_event(state, task_id, kind, payload) {
            tracing::warn!(
                "persisted task event publish failed task_id={} error={}",
                task_id,
                crate::truncate_for_log(&error.to_string())
            );
            break;
        }
    }
}

pub(crate) fn publish_task_status_projection(state: &AppState, task_id: &str) {
    let Ok(uuid) = uuid::Uuid::parse_str(task_id) else {
        return;
    };
    let Ok(Some((task, _, _))) = crate::repo::get_task_query_record(state, uuid) else {
        return;
    };
    let status = serde_json::to_value(&task.status).unwrap_or(Value::Null);
    let execution_state = serde_json::to_value(task.execution_state).unwrap_or(Value::Null);
    let payload = json!({
        "status": status,
        "execution_state": execution_state,
        "lifecycle": task.lifecycle,
    });
    if let Err(error) = publish_event(state, task_id, "task_state", payload.clone()) {
        tracing::warn!(
            "task status event publish failed task_id={} error={}",
            task_id,
            crate::truncate_for_log(&error.to_string())
        );
        return;
    }
    if matches!(
        task.status,
        claw_core::types::TaskStatus::Succeeded
            | claw_core::types::TaskStatus::Failed
            | claw_core::types::TaskStatus::Canceled
            | claw_core::types::TaskStatus::Timeout
    ) {
        let _ = publish_event(state, task_id, "task_final", payload);
    }
}

#[derive(Default)]
struct EventContext {
    thread_id: Option<String>,
    session_id: Option<String>,
    parent_task_id: Option<String>,
    child_task_id: Option<String>,
}

impl EventContext {
    fn fill_missing(&mut self, fallback: Self) {
        if self.thread_id.is_none() {
            self.thread_id = fallback.thread_id;
        }
        if self.session_id.is_none() {
            self.session_id = fallback.session_id;
        }
        if self.parent_task_id.is_none() {
            self.parent_task_id = fallback.parent_task_id;
        }
        if self.child_task_id.is_none() {
            self.child_task_id = fallback.child_task_id;
        }
    }
}

fn event_context(payload: &Value) -> EventContext {
    EventContext {
        thread_id: first_string(payload, &["thread_id", "thread_ref"]),
        session_id: first_string(payload, &["session_id", "session_ref"]),
        parent_task_id: first_string(payload, &["parent_task_id", "parent_id"]),
        child_task_id: first_string(payload, &["child_task_id", "child_run_id"]),
    }
}

fn task_payload_event_context(state: &AppState, task_id: &str) -> EventContext {
    let Ok(db) = state.core.db.get() else {
        return EventContext::default();
    };
    let payload = db
        .query_row(
            "SELECT payload_json FROM tasks WHERE task_id = ?1 LIMIT 1",
            params![task_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    payload.as_ref().map(event_context).unwrap_or_default()
}

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .filter(|value| valid_event_context_ref(value))
            .map(str::to_string)
    })
}

fn valid_event_context_ref(value: &str) -> bool {
    value.len() <= 256
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
}

fn persist_large_payload_if_needed(
    state: &AppState,
    task_id: &str,
    event_kind: &str,
    payload: Value,
    timestamp_ms: u64,
) -> anyhow::Result<(Value, Vec<Value>)> {
    let serialized = serde_json::to_vec(&payload)?;
    if serialized.len() <= EVENT_MAX_BYTES {
        let artifact_refs = collect_artifact_refs(&payload);
        return Ok((payload, artifact_refs));
    }
    let artifact_id = format!("task_event_payload_{}", bytes_hash(&serialized));
    let db = state.core.db.get().context("task event artifact db pool")?;
    db.execute_batch(INIT_TASK_EVENT_SQL)?;
    db.execute(
        "INSERT OR IGNORE INTO task_event_artifacts
         (task_id, artifact_id, payload_json, payload_bytes, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            task_id,
            artifact_id,
            String::from_utf8_lossy(&serialized),
            serialized.len() as u64,
            timestamp_ms
        ],
    )?;
    let artifact_ref = json!({
        "artifact_id": artifact_id,
        "kind": "task_event_payload",
        "event_kind": event_kind,
        "payload_bytes": serialized.len(),
    });
    Ok((
        json!({
            "payload_omitted": true,
            "artifact_ref": artifact_ref,
        }),
        vec![artifact_ref],
    ))
}

fn collect_artifact_refs(payload: &Value) -> Vec<Value> {
    let mut refs = Vec::new();
    collect_named_values(payload, "artifact_refs", &mut refs, 0);
    refs.truncate(64);
    refs
}

fn collect_named_values(value: &Value, key: &str, out: &mut Vec<Value>, depth: usize) {
    if depth > 8 || out.len() >= 64 {
        return;
    }
    match value {
        Value::Object(map) => {
            for (name, child) in map {
                if name == key {
                    if let Some(items) = child.as_array() {
                        out.extend(items.iter().take(64 - out.len()).cloned());
                    }
                } else {
                    collect_named_values(child, key, out, depth + 1);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_named_values(item, key, out, depth + 1);
            }
        }
        _ => {}
    }
}

fn redact_event_value(value: &mut Value, parent_key: Option<&str>, depth: usize) -> usize {
    if depth > 16 {
        *value = json!({ "redacted": true, "reason_code": "event_depth_limit" });
        return 1;
    }
    if parent_key.is_some_and(is_sensitive_key) {
        *value = json!({ "redacted": true, "reason_code": "sensitive_field" });
        return 1;
    }
    match value {
        Value::Object(map) => map
            .iter_mut()
            .map(|(key, child)| redact_event_value(child, Some(key), depth + 1))
            .sum(),
        Value::Array(items) => items
            .iter_mut()
            .map(|item| redact_event_value(item, parent_key, depth + 1))
            .sum(),
        Value::String(text) if secret_like_value(text) => {
            *value = json!({ "redacted": true, "reason_code": "secret_like_value" });
            1
        }
        _ => 0,
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key.trim().to_ascii_lowercase().replace('-', "_");
    const EXACT: &[&str] = &[
        "api_key",
        "authorization",
        "cookie",
        "credential",
        "credentials",
        "password",
        "passphrase",
        "private_key",
        "raw_llm_request",
        "raw_llm_response",
        "raw_prompt",
        "raw_request",
        "raw_response",
        "refresh_token",
        "secret",
        "token",
    ];
    EXACT.contains(&normalized.as_str())
        || normalized.ends_with("_api_key")
        || normalized.ends_with("_password")
        || normalized.ends_with("_secret")
        || normalized.ends_with("_token")
}

fn secret_like_value(value: &str) -> bool {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    lower.contains(claw_core::secrets::SECRET_TOKEN_REFERENCE_PREFIX)
        || lower.starts_with("bearer ")
        || lower.starts_with("basic ")
        || ["sk-", "tp-", "ghp_", "github_pat_", "xoxb-", "xoxp-"]
            .iter()
            .any(|prefix| lower.starts_with(prefix) && trimmed.len() >= prefix.len() + 12)
}

fn normalize_machine_token(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    (!value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'))
    .then_some(value)
}

fn valid_artifact_id(value: &str) -> bool {
    let Some(hash) = value.strip_prefix("task_event_payload_") else {
        return false;
    };
    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn value_hash(value: &Value) -> anyhow::Result<String> {
    Ok(bytes_hash(&serde_json::to_vec(value)?))
}

fn bytes_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "task_event_transport_tests.rs"]
mod tests;
