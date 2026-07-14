use serde_json::{json, Value};

const WARNING_LLM_CALLS: u64 = 8;
const EXCEEDED_LLM_CALLS: u64 = 16;
const WARNING_PROMPT_BYTES_BEFORE_MAX: u64 = 250_000;
const EXCEEDED_PROMPT_BYTES_BEFORE_MAX: u64 = 700_000;
const WARNING_PROMPT_TRUNCATIONS: u64 = 1;
const EXCEEDED_PROMPT_TRUNCATIONS: u64 = 3;
const WARNING_PROVIDER_RETRIES: u64 = 2;
const EXCEEDED_PROVIDER_RETRIES: u64 = 6;
const EXCEEDED_PROVIDER_FINAL_ERRORS: u64 = 1;
const WARNING_ELAPSED_MS: u64 = 300_000;
const EXCEEDED_ELAPSED_MS: u64 = 900_000;

pub(super) struct LlmBudgetMetrics {
    pub(super) prompt_bucket_count: u64,
    pub(super) llm_call_count: u64,
    pub(super) elapsed_ms: u64,
    pub(super) provider_retry_count: u64,
    pub(super) provider_retryable_error_count: u64,
    pub(super) provider_final_error_count: u64,
    pub(super) prompt_truncation_count: u64,
    pub(super) prompt_bytes_before_max: Option<u64>,
    pub(super) prompt_truncated_bytes_total: u64,
}

pub(super) fn llm_budget_health_json(metrics: &LlmBudgetMetrics) -> Value {
    let mut warnings = Vec::new();
    let mut exceeded = Vec::new();
    classify_metric(
        "llm_call_count",
        metrics.llm_call_count,
        WARNING_LLM_CALLS,
        EXCEEDED_LLM_CALLS,
        &mut warnings,
        &mut exceeded,
    );
    if let Some(bytes) = metrics.prompt_bytes_before_max {
        classify_metric(
            "prompt_bytes_before_max",
            bytes,
            WARNING_PROMPT_BYTES_BEFORE_MAX,
            EXCEEDED_PROMPT_BYTES_BEFORE_MAX,
            &mut warnings,
            &mut exceeded,
        );
    }
    classify_metric(
        "prompt_truncation_count",
        metrics.prompt_truncation_count,
        WARNING_PROMPT_TRUNCATIONS,
        EXCEEDED_PROMPT_TRUNCATIONS,
        &mut warnings,
        &mut exceeded,
    );
    classify_metric(
        "provider_retry_count",
        metrics.provider_retry_count,
        WARNING_PROVIDER_RETRIES,
        EXCEEDED_PROVIDER_RETRIES,
        &mut warnings,
        &mut exceeded,
    );
    if metrics.provider_final_error_count >= EXCEEDED_PROVIDER_FINAL_ERRORS {
        exceeded.push("provider_final_error_count");
    }
    classify_metric(
        "elapsed_ms",
        metrics.elapsed_ms,
        WARNING_ELAPSED_MS,
        EXCEEDED_ELAPSED_MS,
        &mut warnings,
        &mut exceeded,
    );

    let status = if !exceeded.is_empty() {
        "exceeded"
    } else if !warnings.is_empty() {
        "warning"
    } else {
        "ok"
    };

    json!({
        "schema_version": 1,
        "threshold_profile": "clawcli_llm_budget_v1",
        "status": status,
        "warnings": warnings,
        "exceeded": exceeded,
        "metrics": {
            "prompt_bucket_count": metrics.prompt_bucket_count,
            "llm_call_count": metrics.llm_call_count,
            "elapsed_ms": metrics.elapsed_ms,
            "provider_retry_count": metrics.provider_retry_count,
            "provider_retryable_error_count": metrics.provider_retryable_error_count,
            "provider_final_error_count": metrics.provider_final_error_count,
            "prompt_truncation_count": metrics.prompt_truncation_count,
            "prompt_bytes_before_max": metrics.prompt_bytes_before_max,
            "prompt_truncated_bytes_total": metrics.prompt_truncated_bytes_total,
        },
        "thresholds": {
            "warning": {
                "llm_call_count": WARNING_LLM_CALLS,
                "prompt_bytes_before_max": WARNING_PROMPT_BYTES_BEFORE_MAX,
                "prompt_truncation_count": WARNING_PROMPT_TRUNCATIONS,
                "provider_retry_count": WARNING_PROVIDER_RETRIES,
                "elapsed_ms": WARNING_ELAPSED_MS,
            },
            "exceeded": {
                "llm_call_count": EXCEEDED_LLM_CALLS,
                "prompt_bytes_before_max": EXCEEDED_PROMPT_BYTES_BEFORE_MAX,
                "prompt_truncation_count": EXCEEDED_PROMPT_TRUNCATIONS,
                "provider_retry_count": EXCEEDED_PROVIDER_RETRIES,
                "provider_final_error_count": EXCEEDED_PROVIDER_FINAL_ERRORS,
                "elapsed_ms": EXCEEDED_ELAPSED_MS,
            }
        }
    })
}

fn classify_metric(
    token: &'static str,
    value: u64,
    warning_threshold: u64,
    exceeded_threshold: u64,
    warnings: &mut Vec<&'static str>,
    exceeded: &mut Vec<&'static str>,
) {
    if value >= exceeded_threshold {
        exceeded.push(token);
    } else if value >= warning_threshold {
        warnings.push(token);
    }
}

pub(super) fn llm_budget_text_lines(llm: &Value) -> Vec<String> {
    let Some(health) = llm.get("budget_health") else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    if let Some(status) = health.get("status").and_then(Value::as_str) {
        push_machine_line(&mut lines, "llm_budget_status", status);
    }
    if let Some(profile) = health.get("threshold_profile").and_then(Value::as_str) {
        push_machine_line(&mut lines, "llm_budget_threshold_profile", profile);
    }
    for token in machine_token_array(health.get("warnings"))
        .into_iter()
        .take(16)
    {
        push_machine_line(&mut lines, "llm_budget_warning", &token);
    }
    for token in machine_token_array(health.get("exceeded"))
        .into_iter()
        .take(16)
    {
        push_machine_line(&mut lines, "llm_budget_exceeded", &token);
    }
    lines
}

fn push_machine_line(lines: &mut Vec<String>, key: &str, value: &str) {
    let mut line = String::with_capacity(key.len() + value.len() + 2);
    line.push_str(key);
    line.push(':');
    line.push(' ');
    line.push_str(value);
    lines.push(line);
}

fn machine_token_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|token| is_machine_token(token))
        .map(ToString::to_string)
        .collect()
}

fn is_machine_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 128
        && token.chars().all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '-' | '.')
        })
}
