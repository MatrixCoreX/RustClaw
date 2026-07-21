use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{output, task};

#[path = "replay_fingerprint.rs"]
mod replay_fingerprint;

use replay_fingerprint::{
    replay_action_sequence, replay_permission_summary, replay_route_fingerprint,
    replay_tool_result_summary, replay_verifier_summary,
};

const REPLAY_SCHEMA_VERSION: u64 = 1;
const REPLAY_BUNDLE_KIND: &str = "rustclaw_task_replay";

pub(crate) fn run_export(
    base_url: &str,
    key: &str,
    task_id: &str,
    output_path: &Path,
    json_output: bool,
) -> Result<()> {
    let task = task::get_task_status(base_url, key, task_id)?;
    let archived_events = crate::events::read_task_event_snapshot(base_url, key, task_id, 0)
        .context("task_replay_archive_read_failed")?;
    let bundle = replay_bundle_json_with_archived_events(&task, &archived_events);
    if let Some(parent) = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("create replay dir {}", parent.display()))?;
    }
    fs::write(output_path, serde_json::to_vec_pretty(&bundle)?)
        .with_context(|| format!("write replay bundle {}", output_path.display()))?;
    let summary = json!({
        "bundle_kind": REPLAY_BUNDLE_KIND,
        "schema_version": REPLAY_SCHEMA_VERSION,
        "task_id": task.task_id,
        "output_path": output_path.display().to_string(),
        "replay_mode": "recorded_only",
        "redaction_policy": "machine_key_redaction_v1",
    });
    if json_output {
        output::print_json_pretty(&summary);
    } else {
        println!("task_id: {}", task.task_id);
        println!("replay_bundle: {}", output_path.display());
        println!("replay_mode: recorded_only");
    }
    Ok(())
}

pub(crate) fn run_run(
    bundle_path: &Path,
    json_output: bool,
    coverage_output: bool,
    view: &str,
) -> Result<()> {
    let body = fs::read_to_string(bundle_path)
        .with_context(|| format!("read replay bundle {}", bundle_path.display()))?;
    let bundle: Value = serde_json::from_str(&body).context("parse replay bundle")?;
    validate_replay_bundle(&bundle)?;
    let summary = if coverage_output || view == "summary" {
        replay_run_summary(&bundle)
    } else {
        replay_view_json(&bundle, view)?
    };
    if json_output {
        output::print_json_pretty(&summary);
    } else if coverage_output {
        let coverage = summary.get("coverage").cloned().unwrap_or(Value::Null);
        output::print_json_pretty(&coverage);
    } else if view != "summary" {
        print_replay_view_lines(&summary);
    } else {
        println!(
            "task_id: {}",
            summary
                .get("task_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
        );
        println!("replay_mode: recorded_only");
        println!(
            "status: {}",
            summary
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default()
        );
    }
    Ok(())
}

pub(crate) fn run_diff(left_path: &Path, right_path: &Path, json_output: bool) -> Result<()> {
    let left = read_replay_bundle(left_path)?;
    let right = read_replay_bundle(right_path)?;
    let diff = replay_diff_summary(&left, &right);
    if json_output {
        output::print_json_pretty(&diff);
    } else {
        println!(
            "left_task_id: {}",
            diff.pointer("/left/task_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
        );
        println!(
            "right_task_id: {}",
            diff.pointer("/right/task_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
        );
        println!(
            "changed: {}",
            diff.get("changed")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        );
    }
    Ok(())
}

fn read_replay_bundle(path: &Path) -> Result<Value> {
    let body = fs::read_to_string(path)
        .with_context(|| format!("read replay bundle {}", path.display()))?;
    let bundle: Value = serde_json::from_str(&body).context("parse replay bundle")?;
    validate_replay_bundle(&bundle)?;
    Ok(bundle)
}

#[cfg(test)]
fn replay_bundle_json(task: &task::TaskStatusView) -> Value {
    replay_bundle_json_with_archived_events(task, &[])
}

fn replay_bundle_json_with_archived_events(
    task: &task::TaskStatusView,
    archived_events: &[Value],
) -> Value {
    let events = if archived_events.is_empty() {
        task.events
            .iter()
            .map(|event| {
                json!({
                    "event_type": &event.event_type,
                    "line": &event.line,
                    "fields": redact_value(&json!(&event.fields)),
                })
            })
            .collect::<Vec<_>>()
    } else {
        archived_events
            .iter()
            .filter_map(|raw| {
                let event = crate::events::task_event_line(raw)?;
                Some(json!({
                    "schema_version": raw.get("schema_version").and_then(Value::as_u64),
                    "payload_schema_version": raw
                        .get("payload_schema_version")
                        .and_then(Value::as_u64),
                    "seq": raw.get("seq").and_then(Value::as_u64),
                    "timestamp_ms": raw.get("timestamp_ms").and_then(Value::as_u64),
                    "event_hash": raw.get("event_hash").and_then(Value::as_str),
                    "previous_event_hash": raw
                        .get("previous_event_hash")
                        .and_then(Value::as_str),
                    "event_type": event.event_type,
                    "line": event.line,
                    "fields": redact_value(&json!(event.fields)),
                    "raw": redact_value(raw),
                }))
            })
            .collect::<Vec<_>>()
    };
    json!({
        "schema_version": REPLAY_SCHEMA_VERSION,
        "bundle_kind": REPLAY_BUNDLE_KIND,
        "exported_at_unix": unix_timestamp(),
        "redaction": {
            "policy": "machine_key_redaction_v1",
            "secret_value": "<redacted:secret>",
            "private_payload_value": "<redacted:private_payload>",
        },
        "task_id": task.task_id,
        "status": task.status,
        "lifecycle_state": task.lifecycle_state(),
        "event_source": if archived_events.is_empty() {
            "task_result_projection"
        } else {
            "task_event_archive"
        },
        "task": redact_value(&task.raw_data),
        "events": events,
    })
}

fn replay_diff_summary(left: &Value, right: &Value) -> Value {
    let left_summary = replay_run_summary(left);
    let right_summary = replay_run_summary(right);
    let status_changed = left_summary.get("status") != right_summary.get("status");
    let lifecycle_changed =
        left_summary.get("lifecycle_state") != right_summary.get("lifecycle_state");
    let event_count_changed = left_summary.get("event_count") != right_summary.get("event_count");
    let left_artifact_count = replay_artifact_ref_count(left);
    let right_artifact_count = replay_artifact_ref_count(right);
    let artifact_count_changed = left_artifact_count != right_artifact_count;
    let left_route_fingerprint = replay_route_fingerprint(left);
    let right_route_fingerprint = replay_route_fingerprint(right);
    let route_changed = left_route_fingerprint != right_route_fingerprint;
    let left_action_sequence = replay_action_sequence(left);
    let right_action_sequence = replay_action_sequence(right);
    let action_sequence_changed = left_action_sequence != right_action_sequence;
    let left_tool_result_summary = replay_tool_result_summary(left);
    let right_tool_result_summary = replay_tool_result_summary(right);
    let tool_result_changed = left_tool_result_summary != right_tool_result_summary;
    let left_verifier_summary = replay_verifier_summary(left);
    let right_verifier_summary = replay_verifier_summary(right);
    let verifier_changed = left_verifier_summary != right_verifier_summary;
    let left_permission_summary = replay_permission_summary(left);
    let right_permission_summary = replay_permission_summary(right);
    let permission_changed = left_permission_summary != right_permission_summary;
    let diff_classes = replay_diff_classes(ReplayDiffSignals {
        status_changed,
        lifecycle_changed,
        event_count_changed,
        artifact_count_changed,
        route_changed,
        action_sequence_changed,
        tool_result_changed,
        verifier_changed,
        permission_changed,
    });
    json!({
        "bundle_kind": "rustclaw_task_replay_diff",
        "schema_version": REPLAY_SCHEMA_VERSION,
        "replay_mode": "recorded_only",
        "live_provider": false,
        "changed": status_changed
            || lifecycle_changed
            || event_count_changed
            || artifact_count_changed
            || route_changed
            || action_sequence_changed
            || tool_result_changed
            || verifier_changed
            || permission_changed,
        "diff_classes": diff_classes,
        "left": {
            "task_id": left_summary.get("task_id").cloned().unwrap_or(Value::Null),
            "status": left_summary.get("status").cloned().unwrap_or(Value::Null),
            "lifecycle_state": left_summary.get("lifecycle_state").cloned().unwrap_or(Value::Null),
            "event_count": left_summary.get("event_count").cloned().unwrap_or(Value::Null),
            "artifact_ref_count": left_artifact_count,
            "route_fingerprint": left_route_fingerprint,
            "action_sequence": left_action_sequence,
            "tool_result_summary": left_tool_result_summary,
            "verifier_summary": left_verifier_summary,
            "permission_summary": left_permission_summary,
        },
        "right": {
            "task_id": right_summary.get("task_id").cloned().unwrap_or(Value::Null),
            "status": right_summary.get("status").cloned().unwrap_or(Value::Null),
            "lifecycle_state": right_summary.get("lifecycle_state").cloned().unwrap_or(Value::Null),
            "event_count": right_summary.get("event_count").cloned().unwrap_or(Value::Null),
            "artifact_ref_count": right_artifact_count,
            "route_fingerprint": right_route_fingerprint,
            "action_sequence": right_action_sequence,
            "tool_result_summary": right_tool_result_summary,
            "verifier_summary": right_verifier_summary,
            "permission_summary": right_permission_summary,
        },
        "diff": {
            "status_changed": status_changed,
            "lifecycle_changed": lifecycle_changed,
            "event_count_changed": event_count_changed,
            "artifact_count_changed": artifact_count_changed,
            "route_changed": route_changed,
            "action_sequence_changed": action_sequence_changed,
            "tool_result_changed": tool_result_changed,
            "verifier_changed": verifier_changed,
            "permission_changed": permission_changed,
        }
    })
}

fn replay_run_summary(bundle: &Value) -> Value {
    let task = bundle.get("task").unwrap_or(&Value::Null);
    let events = bundle
        .get("events")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    let route_fingerprint = replay_route_fingerprint(bundle);
    let action_sequence = replay_action_sequence(bundle);
    let tool_result_summary = replay_tool_result_summary(bundle);
    let verifier_summary = replay_verifier_summary(bundle);
    let permission_summary = replay_permission_summary(bundle);
    let execution_replay = recorded_execution_replay(
        &route_fingerprint,
        &action_sequence,
        &tool_result_summary,
        &verifier_summary,
        &permission_summary,
    );
    json!({
        "bundle_kind": REPLAY_BUNDLE_KIND,
        "schema_version": REPLAY_SCHEMA_VERSION,
        "replay_mode": "recorded_only",
        "live_provider": false,
        "task_id": bundle.get("task_id").and_then(Value::as_str),
        "status": bundle.get("status").and_then(Value::as_str)
            .or_else(|| task.get("status").and_then(Value::as_str)),
        "lifecycle_state": bundle.get("lifecycle_state").and_then(Value::as_str)
            .or_else(|| task.pointer("/task_lifecycle/state").and_then(Value::as_str)),
        "event_count": events,
        "event_source": bundle.get("event_source").and_then(Value::as_str),
        "redaction_policy": bundle.pointer("/redaction/policy").and_then(Value::as_str),
        "result_source": "recorded_bundle",
        "coverage": replay_recording_coverage(bundle),
        "execution_replay": execution_replay,
        "route_fingerprint": route_fingerprint,
        "action_sequence": action_sequence,
        "tool_result_summary": tool_result_summary,
        "verifier_summary": verifier_summary,
        "permission_summary": permission_summary,
    })
}

fn replay_view_json(bundle: &Value, view: &str) -> Result<Value> {
    let items = match view {
        "llm" => replay_llm_view_items(bundle),
        "tools" => replay_tool_result_summary(bundle),
        "checkpoints" => replay_checkpoint_view_items(bundle),
        "summary" => return Ok(replay_run_summary(bundle)),
        _ => anyhow::bail!("replay_view_unknown:{view}"),
    };
    Ok(json!({
        "bundle_kind": REPLAY_BUNDLE_KIND,
        "schema_version": REPLAY_SCHEMA_VERSION,
        "replay_mode": "recorded_only",
        "live_provider": false,
        "live_tool_invocations": false,
        "view": view,
        "task_id": bundle.get("task_id").and_then(Value::as_str),
        "item_count": items.len(),
        "items": items,
    }))
}

fn replay_llm_view_items(bundle: &Value) -> Vec<Value> {
    replay_event_view_items(bundle, |event_type, _event| event_type == "provider_call")
}

fn replay_checkpoint_view_items(bundle: &Value) -> Vec<Value> {
    replay_event_view_items(bundle, |event_type, event| {
        matches!(event_type, "checkpoint_created" | "coding_checkpoint")
            || event
                .get("fields")
                .and_then(Value::as_object)
                .is_some_and(|fields| {
                    fields.contains_key("checkpoint_id") || fields.contains_key("checkpoint_ref")
                })
    })
}

fn replay_event_view_items(bundle: &Value, predicate: impl Fn(&str, &Value) -> bool) -> Vec<Value> {
    bundle
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, event)| {
            let event_type = event
                .get("event_type")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            if event_type.is_empty() || !predicate(event_type, event) {
                return None;
            }
            Some(json!({
                "index": index,
                "event_type": event_type,
                "fields": event.get("fields").cloned().unwrap_or(Value::Null),
                "line": event.get("line").and_then(Value::as_str),
            }))
        })
        .collect()
}

fn print_replay_view_lines(summary: &Value) {
    println!(
        "replay_view: {}",
        summary
            .get("view")
            .and_then(Value::as_str)
            .unwrap_or_default()
    );
    println!(
        "task_id: {}",
        summary
            .get("task_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
    );
    println!(
        "item_count: {}",
        summary
            .get("item_count")
            .and_then(Value::as_u64)
            .unwrap_or_default()
    );
    for item in summary
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(32)
    {
        let kind = item
            .get("event_type")
            .or_else(|| item.get("stage"))
            .and_then(Value::as_str)
            .unwrap_or("item");
        println!("replay_item: kind={kind}");
    }
}

fn recorded_execution_replay(
    route_fingerprint: &[Value],
    action_sequence: &[Value],
    tool_result_summary: &[Value],
    verifier_summary: &[Value],
    permission_summary: &[Value],
) -> Value {
    let mut steps = Vec::new();
    push_recorded_replay_steps(&mut steps, "route", route_fingerprint);
    push_recorded_replay_steps(&mut steps, "action", action_sequence);
    push_recorded_replay_steps(&mut steps, "tool_result", tool_result_summary);
    push_recorded_replay_steps(&mut steps, "verifier", verifier_summary);
    push_recorded_replay_steps(&mut steps, "permission", permission_summary);
    json!({
        "strategy": "recorded_outputs_first",
        "deterministic": true,
        "live_provider": false,
        "live_tool_invocations": false,
        "provider_call_count": 0,
        "tool_invocation_count": 0,
        "step_count": steps.len(),
        "steps": steps,
    })
}

struct ReplayDiffSignals {
    status_changed: bool,
    lifecycle_changed: bool,
    event_count_changed: bool,
    artifact_count_changed: bool,
    route_changed: bool,
    action_sequence_changed: bool,
    tool_result_changed: bool,
    verifier_changed: bool,
    permission_changed: bool,
}

fn replay_diff_classes(signals: ReplayDiffSignals) -> Vec<&'static str> {
    let mut classes = Vec::new();
    if signals.status_changed || signals.lifecycle_changed {
        classes.push("final_status_changed");
    }
    if signals.event_count_changed {
        classes.push("event_count_changed");
    }
    if signals.artifact_count_changed {
        classes.push("artifact_count_changed");
    }
    if signals.route_changed {
        classes.push("route_changed");
    }
    if signals.action_sequence_changed {
        classes.push("plan_changed");
    }
    if signals.verifier_changed {
        classes.push("verifier_changed");
    }
    if signals.permission_changed {
        classes.push("permission_changed");
    }
    if signals.tool_result_changed {
        classes.push("tool_result_changed");
    }
    classes
}

fn push_recorded_replay_steps(steps: &mut Vec<Value>, stage: &str, items: &[Value]) {
    for item in items {
        steps.push(json!({
            "step_index": steps.len(),
            "stage": stage,
            "source": "recorded_bundle",
            "summary": item,
        }));
    }
}

fn replay_recording_coverage(bundle: &Value) -> Value {
    let event_types = replay_event_types(bundle);
    json!({
        "has_task_checkpoint": value_contains_key(bundle, "task_checkpoint", 0),
        "has_pending_async_job": value_contains_key(bundle, "pending_async_job", 0),
        "has_repair_signal": value_contains_any_key(bundle, &["repair_signal", "repair_signals"], 0),
        "has_resume_claim": value_contains_any_key(
            bundle,
            &[
                "resume_claim",
                "resume_executor_claim",
                "resume_executor_handoff_claim",
                "resume_executor_dispatch_claim",
                "resume_executor_result_projection_claim",
            ],
            0,
        ),
        "has_subagent_results": value_contains_any_key(
            bundle,
            &["child_results", "subagent_results", "finding_refs"],
            0,
        ),
        "has_policy_decision": value_contains_any_key(
            bundle,
            &["permission_decision", "policy_decision", "command_policy"],
            0,
        ),
        "event_types": event_types,
    })
}

fn replay_event_types(bundle: &Value) -> Vec<String> {
    let mut event_types = BTreeSet::new();
    if let Some(events) = bundle.get("events").and_then(Value::as_array) {
        for event in events {
            if event_types.len() >= 64 {
                break;
            }
            if let Some(event_type) = event
                .get("event_type")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                event_types.insert(event_type.to_string());
            }
        }
    }
    event_types.into_iter().collect()
}

fn value_contains_any_key(value: &Value, keys: &[&str], depth: usize) -> bool {
    keys.iter().any(|key| value_contains_key(value, key, depth))
}

fn value_contains_key(value: &Value, target_key: &str, depth: usize) -> bool {
    if depth > 10 {
        return false;
    }
    match value {
        Value::Object(map) => {
            map.contains_key(target_key)
                || map
                    .values()
                    .any(|value| value_contains_key(value, target_key, depth + 1))
        }
        Value::Array(items) => items
            .iter()
            .any(|value| value_contains_key(value, target_key, depth + 1)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}

fn replay_artifact_ref_count(bundle: &Value) -> usize {
    let mut count = 0usize;
    collect_replay_artifact_ref_count(bundle, &mut count, 0);
    count
}

fn collect_replay_artifact_ref_count(value: &Value, count: &mut usize, depth: usize) {
    if depth > 8 || *count >= 128 {
        return;
    }
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(items)) = map.get("artifact_refs") {
                *count += items.len();
            }
            for value in map.values() {
                collect_replay_artifact_ref_count(value, count, depth + 1);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_replay_artifact_ref_count(item, count, depth + 1);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn validate_replay_bundle(bundle: &Value) -> Result<()> {
    let schema_version = bundle
        .get("schema_version")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let bundle_kind = bundle
        .get("bundle_kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if schema_version != REPLAY_SCHEMA_VERSION || bundle_kind != REPLAY_BUNDLE_KIND {
        anyhow::bail!("invalid_replay_bundle");
    }
    Ok(())
}

fn redact_value(value: &Value) -> Value {
    redact_value_with_key(None, value)
}

fn redact_value_with_key(key: Option<&str>, value: &Value) -> Value {
    if let Some(kind) = redaction_kind_for_key(key) {
        return Value::String(kind.to_string());
    }
    match value {
        Value::Object(map) => Value::Object(redact_map(map)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| redact_value_with_key(None, item))
                .collect(),
        ),
        Value::String(value) if value_looks_secret_like(value) => {
            Value::String("<redacted:secret>".to_string())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.clone(),
    }
}

fn redact_map(map: &Map<String, Value>) -> Map<String, Value> {
    map.iter()
        .map(|(key, value)| (key.clone(), redact_value_with_key(Some(key), value)))
        .collect()
}

fn redaction_kind_for_key(key: Option<&str>) -> Option<&'static str> {
    let key = key?.trim().to_ascii_lowercase();
    if key.is_empty() {
        return None;
    }
    if matches!(
        key.as_str(),
        "authorization"
            | "api_key"
            | "apikey"
            | "access_token"
            | "refresh_token"
            | "private_key"
            | "client_secret"
            | "app_secret"
            | "password"
            | "credential"
            | "credentials"
            | "secret"
            | "token"
    ) || key.ends_with("_secret")
        || key.ends_with("_token")
        || key.ends_with("_key")
    {
        return Some("<redacted:secret>");
    }
    if matches!(
        key.as_str(),
        "prompt" | "user_prompt" | "raw_prompt" | "text" | "content" | "messages"
    ) {
        return Some("<redacted:private_payload>");
    }
    None
}

fn value_looks_secret_like(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.len() < 24 || trimmed.contains(char::is_whitespace) {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("sk-")
        || lower.starts_with("tp-")
        || lower.starts_with("bearer-")
        || lower.starts_with("rustclaw_")
        || lower.contains("_secret_")
        || lower.contains("_token_")
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "replay_tests.rs"]
mod tests;
