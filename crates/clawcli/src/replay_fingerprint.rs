use serde_json::{Map, Value};

pub(crate) fn replay_route_fingerprint(bundle: &Value) -> Vec<Value> {
    let mut items = Vec::new();
    collect_machine_summaries(
        bundle,
        &[
            "route_gate_kind",
            "semantic_route_authority",
            "ask_mode",
            "agent_mode",
            "route_result",
            "route_decision",
            "boundary_context",
        ],
        &[
            "route_gate_kind",
            "semantic_route_authority",
            "ask_mode",
            "agent_mode",
            "intent_kind",
            "target_kind",
            "operation",
            "decision",
            "status",
            "source",
            "profile",
        ],
        &mut items,
        0,
    );
    cap_summary_items(items)
}

pub(crate) fn replay_action_sequence(bundle: &Value) -> Vec<Value> {
    let mut actions = Vec::new();
    collect_replay_actions(bundle, None, &mut actions, 0);
    cap_summary_items(actions)
}

pub(crate) fn replay_tool_result_summary(bundle: &Value) -> Vec<Value> {
    let mut items = Vec::new();
    collect_machine_summaries(
        bundle,
        &[
            "skill",
            "tool",
            "capability",
            "tool_result",
            "skill_result",
            "capability_result",
            "execution_result",
            "async_job",
            "pending_async_job",
        ],
        &[
            "skill",
            "tool",
            "capability",
            "action",
            "status",
            "status_code",
            "error_code",
            "message_key",
            "exit_code",
            "failure_kind",
            "terminal_reason",
            "adapter_kind",
            "execution_mode",
            "job_id",
            "cancel_ref",
        ],
        &mut items,
        0,
    );
    cap_summary_items(items)
}

pub(crate) fn replay_verifier_summary(bundle: &Value) -> Vec<Value> {
    let mut items = Vec::new();
    collect_machine_summaries(
        bundle,
        &[
            "verifier",
            "answer_verifier",
            "plan_verifier",
            "verifier_result",
            "verifier_verdict",
            "output_contract_verdict",
            "repair_signal",
            "repair_signals",
        ],
        &[
            "verifier",
            "answer_verifier",
            "plan_verifier",
            "verifier_result",
            "verifier_verdict",
            "output_contract_verdict",
            "repair_signal",
            "repair_class",
            "owner_layer",
            "status",
            "status_code",
            "error_code",
            "message_key",
            "reason_code",
            "verdict",
            "decision",
            "source",
        ],
        &mut items,
        0,
    );
    cap_summary_items(items)
}

fn collect_replay_actions(
    value: &Value,
    parent_key: Option<&str>,
    actions: &mut Vec<Value>,
    depth: usize,
) {
    if depth > 10 || actions.len() >= 128 {
        return;
    }
    match value {
        Value::Object(map) => {
            if map.contains_key("action_type")
                || (map.contains_key("type")
                    && (map.contains_key("skill")
                        || map.contains_key("tool")
                        || map.contains_key("capability")
                        || map.contains_key("action")))
            {
                if let Some(summary) = compact_machine_object(map, ACTION_FIELD_KEYS) {
                    actions.push(summary);
                }
            }
            for (key, value) in map {
                collect_replay_actions(value, Some(key), actions, depth + 1);
            }
        }
        Value::Array(items) => {
            let scoped = matches!(
                parent_key,
                Some("actions" | "planned_actions" | "steps" | "step_results")
            );
            for item in items {
                if scoped {
                    if let Value::Object(map) = item {
                        if let Some(summary) = compact_machine_object(map, ACTION_FIELD_KEYS) {
                            actions.push(summary);
                        }
                        for value in map.values() {
                            collect_replay_actions(value, None, actions, depth + 1);
                        }
                        continue;
                    }
                }
                collect_replay_actions(item, parent_key, actions, depth + 1);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn collect_machine_summaries(
    value: &Value,
    trigger_keys: &[&str],
    field_keys: &[&str],
    items: &mut Vec<Value>,
    depth: usize,
) {
    if depth > 10 || items.len() >= 128 {
        return;
    }
    match value {
        Value::Object(map) => {
            if trigger_keys.iter().any(|key| map.contains_key(*key)) {
                if let Some(summary) = compact_machine_object(map, field_keys) {
                    items.push(summary);
                }
            }
            for value in map.values() {
                collect_machine_summaries(value, trigger_keys, field_keys, items, depth + 1);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_machine_summaries(value, trigger_keys, field_keys, items, depth + 1);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn compact_machine_object(map: &Map<String, Value>, field_keys: &[&str]) -> Option<Value> {
    let mut compact = Map::new();
    for key in field_keys {
        if let Some(value) = map.get(*key).and_then(compact_machine_value) {
            compact.insert((*key).to_string(), value);
        }
    }
    if compact.is_empty() {
        None
    } else {
        Some(Value::Object(compact))
    }
}

fn compact_machine_value(value: &Value) -> Option<Value> {
    match value {
        Value::Null => None,
        Value::Bool(_) | Value::Number(_) => Some(value.clone()),
        Value::String(value) => compact_machine_string(value).map(Value::String),
        Value::Array(items) => {
            let values = items
                .iter()
                .filter_map(compact_machine_value)
                .take(16)
                .collect::<Vec<_>>();
            if values.is_empty() {
                None
            } else {
                Some(Value::Array(values))
            }
        }
        Value::Object(map) => compact_machine_object(map, NESTED_FIELD_KEYS),
    }
}

fn compact_machine_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 160 || trimmed.contains('\n') || trimmed.contains('\r')
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn cap_summary_items(mut items: Vec<Value>) -> Vec<Value> {
    items.truncate(64);
    items
}

const ACTION_FIELD_KEYS: &[&str] = &[
    "action_type",
    "type",
    "skill",
    "tool",
    "capability",
    "action",
    "status",
    "status_code",
    "error_code",
    "message_key",
    "exit_code",
    "effect",
    "risk_level",
    "execution_mode",
    "adapter_kind",
    "isolation_profile",
];

const NESTED_FIELD_KEYS: &[&str] = &[
    "type",
    "kind",
    "status",
    "status_code",
    "error_code",
    "message_key",
    "reason_code",
    "decision",
    "verdict",
    "source",
    "route_gate_kind",
    "semantic_route_authority",
    "action_type",
    "skill",
    "tool",
    "capability",
    "action",
    "adapter_kind",
    "execution_mode",
    "job_id",
    "cancel_ref",
];
