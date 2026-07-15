use serde_json::Value;

pub(super) fn ask_runtime_failure_machine_payload(err: &str) -> String {
    let err = err.trim();
    let classification = AskRuntimeFailureClassification::from_error(err);
    let mut payload = serde_json::json!({
        "schema_version": 1,
        "message_key": "clawd.msg.ask_runtime_failure",
        "reason_code": classification.reason_code,
        "status_code": classification.status_code,
        "failure_attribution": "provider_gap",
        "retryable": false,
    });
    if !err.is_empty() {
        payload["raw_error_present"] = serde_json::json!(true);
        payload["provider_error_class"] = serde_json::json!(classification.provider_error_class);
        if classification.external_provider_blocked {
            payload["external_provider_blocked"] = serde_json::json!(true);
        }
        if let Some(status) = classification.http_status {
            payload["provider_http_status"] = serde_json::json!(status);
        }
        if let Some(code) = classification.provider_error_code {
            payload["provider_error_code"] = serde_json::json!(code);
        }
        if let Some(error_type) = classification.provider_error_type {
            payload["provider_error_type"] = serde_json::json!(error_type);
        }
    }
    payload.to_string()
}

struct AskRuntimeFailureClassification {
    reason_code: &'static str,
    status_code: &'static str,
    provider_error_class: &'static str,
    external_provider_blocked: bool,
    http_status: Option<u16>,
    provider_error_code: Option<String>,
    provider_error_type: Option<String>,
}

impl AskRuntimeFailureClassification {
    fn from_error(err: &str) -> Self {
        let lower = err.to_ascii_lowercase();
        let http_status = provider_http_status(err);
        let provider_error = provider_error_object(err);
        let provider_error_code = provider_error
            .as_ref()
            .and_then(|value| value.get("code"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let provider_error_type = provider_error
            .as_ref()
            .and_then(|value| value.get("type"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let code = provider_error_code
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let error_type = provider_error_type
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();

        let (reason_code, provider_error_class, external_provider_blocked) = if http_status
            == Some(429)
            && (code == "429"
                || error_type == "limitation"
                || lower.contains("quota_exhausted")
                || lower.contains("quota exhausted"))
        {
            ("provider_quota_exceeded", "quota_exceeded", true)
        } else if http_status == Some(402) || code == "arrearage" || error_type == "arrearage" {
            ("provider_account_blocked", "account_blocked", true)
        } else if code == "model_not_found" {
            ("provider_model_unavailable", "model_unavailable", true)
        } else if matches!(http_status, Some(401 | 403)) || lower.contains("unauthorized") {
            ("provider_auth_failed", "auth_or_permission_failed", true)
        } else if lower.contains("timeout") || lower.contains("timed out") {
            ("provider_timeout", "timeout", true)
        } else if lower.contains("rate_limit") || http_status == Some(429) {
            ("provider_rate_limited", "rate_limited", true)
        } else if !err.is_empty() {
            ("ask_runtime_failure", "provider_error", false)
        } else {
            ("ask_runtime_failure", "provider_error", false)
        };

        Self {
            reason_code,
            status_code: reason_code,
            provider_error_class,
            external_provider_blocked,
            http_status,
            provider_error_code,
            provider_error_type,
        }
    }
}

fn provider_http_status(err: &str) -> Option<u16> {
    let lower = err.to_ascii_lowercase();
    let (_, tail) = lower.split_once("http ")?;
    let token = tail
        .split(|ch: char| ch == ':' || ch.is_ascii_whitespace())
        .find(|part| !part.is_empty())?;
    token.parse::<u16>().ok()
}

fn provider_error_object(err: &str) -> Option<Value> {
    let start = err.find('{')?;
    let value = serde_json::from_str::<Value>(err[start..].trim()).ok()?;
    value.get("error").cloned().filter(Value::is_object)
}
