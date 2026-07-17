use serde_json::{json, Value};

const TASK_METRICS_PATHS: &[&str] = &[
    "/result_json/task_journal/summary/task_metrics",
    "/task_journal/summary/task_metrics",
];

pub(super) fn llm_cost_governance_json(data: &Value) -> Value {
    let Some(metrics) = TASK_METRICS_PATHS
        .iter()
        .find_map(|path| data.pointer(path))
    else {
        return Value::Null;
    };
    let cost = metrics
        .get("llm_cost")
        .filter(|value| value.is_object())
        .map(project_cost_summary)
        .unwrap_or(Value::Null);
    let budget = metrics
        .get("llm_cost_budget")
        .filter(|value| value.is_object())
        .map(project_cost_budget)
        .unwrap_or(Value::Null);
    if cost.is_null() && budget.is_null() {
        Value::Null
    } else {
        json!({
            "schema_version": 1,
            "source": "task_journal_llm_cost",
            "cost": cost,
            "budget": budget,
        })
    }
}

fn project_cost_summary(value: &Value) -> Value {
    json!({
        "status": machine_string(value, "status"),
        "logical_call_count": machine_u64(value, "logical_call_count"),
        "covered_logical_call_count": machine_u64(value, "covered_logical_call_count"),
        "provider_record_count": machine_u64(value, "provider_record_count"),
        "usage_record_count": machine_u64(value, "usage_record_count"),
        "priced_record_count": machine_u64(value, "priced_record_count"),
        "unknown_record_count": machine_u64(value, "unknown_record_count"),
        "input_tokens": machine_u64(value, "input_tokens"),
        "output_tokens": machine_u64(value, "output_tokens"),
        "cached_input_tokens": machine_u64(value, "cached_input_tokens"),
        "reasoning_tokens": machine_u64(value, "reasoning_tokens"),
        "estimated_cost_usd_nanos": machine_u64(value, "estimated_cost_usd_nanos"),
        "unknown_reasons": machine_token_array(value.get("unknown_reasons")),
    })
}

fn project_cost_budget(value: &Value) -> Value {
    json!({
        "status": machine_string(value, "status"),
        "enforcement": machine_string(value, "enforcement"),
        "provider": machine_string(value, "provider"),
        "task_known_cost_usd_nanos": machine_u64(value, "task_known_cost_usd_nanos"),
        "user_24h_known_cost_usd_nanos": machine_u64(value, "user_24h_known_cost_usd_nanos"),
        "provider_24h_known_cost_usd_nanos": machine_u64(value, "provider_24h_known_cost_usd_nanos"),
        "task_unknown_record_count": machine_u64(value, "task_unknown_record_count"),
        "user_24h_unknown_record_count": machine_u64(value, "user_24h_unknown_record_count"),
        "provider_24h_unknown_record_count": machine_u64(value, "provider_24h_unknown_record_count"),
        "soft_task_limit_usd_nanos": machine_u64(value, "soft_task_limit_usd_nanos"),
        "soft_user_24h_limit_usd_nanos": machine_u64(value, "soft_user_24h_limit_usd_nanos"),
        "soft_provider_24h_limit_usd_nanos": machine_u64(value, "soft_provider_24h_limit_usd_nanos"),
        "hard_task_limit_usd_nanos": machine_u64(value, "hard_task_limit_usd_nanos"),
        "hard_exceeded": value.get("hard_exceeded").and_then(Value::as_bool),
        "signals": machine_token_array(value.get("signals")),
    })
}

pub(super) fn llm_cost_text_lines(governance: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    push_string_line(&mut lines, governance, "/cost/status", "llm_cost_status");
    push_u64_line(
        &mut lines,
        governance,
        "/cost/estimated_cost_usd_nanos",
        "llm_estimated_cost_usd_nanos",
    );
    push_u64_line(
        &mut lines,
        governance,
        "/cost/unknown_record_count",
        "llm_cost_unknown_record_count",
    );
    push_string_line(
        &mut lines,
        governance,
        "/budget/status",
        "llm_cost_budget_status",
    );
    push_string_line(
        &mut lines,
        governance,
        "/budget/enforcement",
        "llm_cost_budget_enforcement",
    );
    for (pointer, key) in [
        (
            "/budget/task_known_cost_usd_nanos",
            "llm_task_known_cost_usd_nanos",
        ),
        (
            "/budget/soft_task_limit_usd_nanos",
            "llm_soft_task_limit_usd_nanos",
        ),
        (
            "/budget/hard_task_limit_usd_nanos",
            "llm_hard_task_limit_usd_nanos",
        ),
    ] {
        push_u64_line(&mut lines, governance, pointer, key);
    }
    for signal in governance
        .pointer("/budget/signals")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|value| is_machine_token(value))
        .take(16)
    {
        push_machine_line(&mut lines, "llm_cost_budget_signal", signal);
    }
    lines
}

fn machine_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| is_machine_token(text))
        .map(ToString::to_string)
}

fn machine_u64(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn machine_token_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|token| is_machine_token(token))
        .take(32)
        .map(ToString::to_string)
        .collect()
}

fn push_string_line(lines: &mut Vec<String>, value: &Value, pointer: &str, key: &str) {
    if let Some(text) = value.pointer(pointer).and_then(Value::as_str) {
        lines.push(format!("{key}: {text}"));
    }
}

fn push_u64_line(lines: &mut Vec<String>, value: &Value, pointer: &str, key: &str) {
    if let Some(number) = value.pointer(pointer).and_then(Value::as_u64) {
        lines.push(format!("{key}: {number}"));
    }
}

fn push_machine_line(lines: &mut Vec<String>, key: &str, value: &str) {
    let mut line = String::with_capacity(key.len() + value.len() + 2);
    line.push_str(key);
    line.push(':');
    line.push(' ');
    line.push_str(value);
    lines.push(line);
}

fn is_machine_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 128
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':' | '/'))
}
