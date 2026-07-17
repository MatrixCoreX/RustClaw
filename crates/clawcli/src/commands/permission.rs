use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

use crate::{client, output, task};

use super::common::get_v1_json;

pub(crate) fn run_permission_inspect(
    base_url: &str,
    key: &str,
    task_id: &str,
    json_output: bool,
) -> Result<()> {
    let task = task::get_task_status(base_url, key, task_id)?;
    let report = permission_report_json(&task);
    if json_output {
        output::print_json_pretty(&report);
    } else {
        for line in permission_report_text_lines(&report) {
            println!("{line}");
        }
    }
    Ok(())
}

pub(crate) fn run_permission_explain(
    base_url: &str,
    key: &str,
    task_id: &str,
    json_output: bool,
) -> Result<()> {
    run_permission_inspect(base_url, key, task_id, json_output)
}

pub(crate) fn run_permission_capability(
    base_url: &str,
    key: &str,
    skill: Option<&str>,
    capability: Option<&str>,
    json_output: bool,
) -> Result<()> {
    let body = get_v1_json(base_url, key, "/capabilities", "capabilities")?;
    let report = capability_permission_report_json(&body, skill, capability);
    if json_output {
        output::print_json_pretty(&report);
    } else {
        for line in capability_permission_text_lines(&report) {
            println!("{line}");
        }
    }
    Ok(())
}

pub(crate) fn run_permission_grants(base_url: &str, key: &str, json_output: bool) -> Result<()> {
    let body = get_v1_json(
        base_url,
        key,
        "/tasks/approval-grants",
        "approval_scope_grants",
    )?;
    let data = body.get("data").cloned().unwrap_or(Value::Null);
    if json_output {
        output::print_json_pretty(&data);
        return Ok(());
    }
    println!(
        "approval_scope_grant_count={}",
        data.get("count").and_then(Value::as_u64).unwrap_or(0)
    );
    if let Some(grants) = data.get("grants").and_then(Value::as_array) {
        for (index, grant) in grants.iter().enumerate() {
            println!(
                "approval_scope_grant#{} id={} scope_kind={} channel={} chat_id={} expires_at={} revoked_at={} use_count={}",
                index + 1,
                value_token(grant.get("grant_id")),
                value_token(grant.get("scope_kind")),
                value_token(grant.get("channel")),
                value_token(grant.get("chat_id")),
                value_token(grant.get("expires_at")),
                value_token(grant.get("revoked_at")),
                value_token(grant.get("use_count")),
            );
        }
    }
    Ok(())
}

pub(crate) fn run_permission_revoke(
    base_url: &str,
    key: &str,
    grant_id: &str,
    json_output: bool,
) -> Result<()> {
    let grant_id = grant_id.trim();
    if grant_id.is_empty() {
        anyhow::bail!("approval_scope_grant_id_required");
    }
    let url = format!(
        "{}{}",
        client::base_v1(base_url),
        "/tasks/approval-grants/revoke"
    );
    let response = client::make_client()?
        .post(&url)
        .header("x-rustclaw-key", key)
        .json(&json!({"grant_id": grant_id}))
        .send()
        .context("request approval_scope_grant_revoke failed")?;
    let status = response.status();
    let body: Value = response
        .json()
        .context("parse approval_scope_grant_revoke response")?;
    if !status.is_success() {
        anyhow::bail!(
            "approval_scope_grant_revoke returned {}: {:?}",
            status,
            body.get("error")
        );
    }
    let data = body.get("data").cloned().unwrap_or(Value::Null);
    if json_output {
        output::print_json_pretty(&data);
    } else {
        println!(
            "approval_scope_grant_status={} grant_id={}",
            value_token(data.get("status")),
            value_token(data.get("grant_id")),
        );
    }
    Ok(())
}

pub(super) fn permission_report_json(task: &task::TaskStatusView) -> Value {
    let mut signals = PermissionSignals::default();
    collect_permission_signals(&task.raw_data, &mut signals, 0);
    json!({
        "report_kind": "rustclaw_permission_report",
        "task_id": task.task_id,
        "status": task.status,
        "execution_state": task.execution_state(),
        "lifecycle_state": task.lifecycle_state(),
        "permission_entry_count": signals.entries.len(),
        "permission_entries": signals.entries,
    })
}

fn permission_report_text_lines(report: &Value) -> Vec<String> {
    let mut lines = vec![
        format!(
            "task_id: {}",
            report.get("task_id").and_then(Value::as_str).unwrap_or("")
        ),
        format!(
            "status: {}",
            report.get("status").and_then(Value::as_str).unwrap_or("")
        ),
        format!(
            "permission_entry_count: {}",
            report
                .get("permission_entry_count")
                .and_then(Value::as_u64)
                .unwrap_or(0)
        ),
    ];
    if let Some(entries) = report.get("permission_entries").and_then(Value::as_array) {
        for entry in entries.iter().take(64) {
            lines.push(format!(
                "permission: source={} decision={} risk_level={} action_effect={} needs_confirmation={} dry_run_required={} reason_code={} isolation_profile={} sandbox_profile={} sandbox_source={} filesystem_write={}",
                value_token(entry.get("source")),
                value_token(entry.get("decision")),
                value_token(entry.get("risk_level")),
                value_token(entry.get("action_effect")),
                value_token(entry.get("needs_confirmation")),
                value_token(entry.get("dry_run_required")),
                value_token(entry.get("reason_code")),
                value_token(entry.get("isolation_profile")),
                value_token(entry.get("sandbox_profile")),
                value_token(entry.get("sandbox_source")),
                value_token(entry.get("filesystem_write")),
            ));
        }
    }
    lines
}

fn capability_permission_report_json(
    body: &Value,
    skill_filter: Option<&str>,
    capability_filter: Option<&str>,
) -> Value {
    let skill_filter = normalize_filter(skill_filter);
    let capability_filter = normalize_filter(capability_filter);
    let mut items = Vec::new();
    if let Some(skill_items) = body
        .pointer("/data/skill_items")
        .and_then(Value::as_array)
        .or_else(|| body.pointer("/data/items").and_then(Value::as_array))
    {
        for item in skill_items {
            if !matches_skill_filter(item, skill_filter.as_deref()) {
                continue;
            }
            if !matches_capability_filter(item, capability_filter.as_deref()) {
                continue;
            }
            items.push(json!({
                "skill": item.get("name").cloned().unwrap_or(Value::Null),
                "runtime_available": item.get("runtime_available").cloned().unwrap_or(Value::Null),
                "unavailable_reason": item.get("unavailable_reason").cloned().unwrap_or(Value::Null),
                "risk_level": item.get("risk_level").cloned().unwrap_or(Value::Null),
                "planner_capabilities": item.get("planner_capabilities").cloned().unwrap_or(Value::Null),
                "capabilities": item.get("capabilities").cloned().unwrap_or(Value::Null),
                "planner_capability_policies": item.get("planner_capability_policies").cloned().unwrap_or(Value::Null),
                "isolation_profile": item.get("isolation_profile").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    json!({
        "report_kind": "rustclaw_capability_permission_report",
        "skill_filter": skill_filter,
        "capability_filter": capability_filter,
        "item_count": items.len(),
        "items": items,
    })
}

fn capability_permission_text_lines(report: &Value) -> Vec<String> {
    let mut lines = vec![format!(
        "item_count: {}",
        report
            .get("item_count")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    )];
    if let Some(items) = report.get("items").and_then(Value::as_array) {
        for item in items.iter().take(128) {
            lines.push(format!(
                "capability: skill={} available={} risk_level={} unavailable_reason={} planner_capabilities={}",
                value_token(item.get("skill")),
                value_token(item.get("runtime_available")),
                value_token(item.get("risk_level")),
                value_token(item.get("unavailable_reason")),
                value_array_tokens(item.get("planner_capabilities")).join(","),
            ));
        }
    }
    lines
}

#[derive(Default)]
struct PermissionSignals {
    seen: BTreeSet<String>,
    entries: Vec<Value>,
}

fn collect_permission_signals(value: &Value, signals: &mut PermissionSignals, depth: usize) {
    if depth > 12 || signals.entries.len() >= 128 {
        return;
    }
    match value {
        Value::Object(map) => {
            for key in ["permission_decision", "policy_decision", "command_policy"] {
                if let Some(Value::Object(decision)) = map.get(key) {
                    push_permission_entry(key, decision, signals);
                }
            }
            for value in map.values() {
                collect_permission_signals(value, signals, depth + 1);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_permission_signals(item, signals, depth + 1);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn push_permission_entry(
    source: &str,
    decision: &Map<String, Value>,
    signals: &mut PermissionSignals,
) {
    let entry = json!({
        "source": source,
        "decision": machine_string_field(decision, "decision")
            .or_else(|| machine_string_field(decision, "policy_decision"))
            .or_else(|| machine_string_field(decision, "policy_authority")),
        "allowed": decision.get("allowed").and_then(Value::as_bool),
        "needs_confirmation": decision
            .get("needs_confirmation")
            .or_else(|| decision.get("requires_confirmation"))
            .and_then(Value::as_bool),
        "dry_run_required": decision.get("dry_run_required").and_then(Value::as_bool),
        "risk_level": machine_string_field(decision, "risk_level"),
        "action_effect": machine_string_field(decision, "action_effect")
            .or_else(|| machine_string_field(decision, "effect")),
        "reason_code": machine_string_field(decision, "reason_code")
            .or_else(|| machine_string_field(decision, "error_code")),
        "isolation_profile": machine_string_field(decision, "isolation_profile")
            .or_else(|| nested_machine_string_field(decision, &["command_policy", "isolation_profile"]))
            .or_else(|| nested_machine_string_field(decision, &["capability_policy", "isolation_profile"])),
        "sandbox_profile": machine_string_field(decision, "sandbox_profile")
            .or_else(|| nested_machine_string_field(decision, &["sandbox", "profile"])),
        "sandbox_source": nested_machine_string_field(decision, &["sandbox", "source"]),
        "filesystem_write": decision.get("filesystem_write").and_then(Value::as_bool)
            .or_else(|| nested_bool_field(decision, &["sandbox", "filesystem_write"])),
        "command_policy": decision.get("command_policy").cloned().unwrap_or(Value::Null),
        "capability_policy": decision.get("capability_policy").cloned().unwrap_or(Value::Null),
    });
    let identity = serde_json::to_string(&entry).unwrap_or_default();
    if signals.seen.insert(identity) {
        signals.entries.push(entry);
    }
}

fn matches_skill_filter(item: &Value, filter: Option<&str>) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    item.get("name")
        .and_then(Value::as_str)
        .is_some_and(|value| value == filter)
}

fn matches_capability_filter(item: &Value, filter: Option<&str>) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    value_array_tokens(item.get("planner_capabilities"))
        .into_iter()
        .chain(value_array_tokens(item.get("capabilities")))
        .any(|value| value == filter)
}

fn normalize_filter(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| is_machine_token(value))
        .map(ToString::to_string)
}

fn machine_string_field(map: &Map<String, Value>, key: &str) -> Option<String> {
    map.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| is_machine_token(value))
        .map(ToString::to_string)
}

fn nested_machine_string_field(map: &Map<String, Value>, path: &[&str]) -> Option<String> {
    let mut current = Value::Object(map.clone());
    for key in path {
        current = current.get(*key)?.clone();
    }
    current
        .as_str()
        .map(str::trim)
        .filter(|value| is_machine_token(value))
        .map(ToString::to_string)
}

fn nested_bool_field(map: &Map<String, Value>, path: &[&str]) -> Option<bool> {
    let mut current = Value::Object(map.clone());
    for key in path {
        current = current.get(*key)?.clone();
    }
    current.as_bool()
}

fn value_token(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.trim().to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Null | Value::Array(_) | Value::Object(_)) | None => String::new(),
    }
}

fn value_array_tokens(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| is_machine_token(value))
        .map(ToString::to_string)
        .collect()
}

fn is_machine_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.' | '/'))
}
