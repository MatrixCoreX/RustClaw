use serde_json::Value;

pub(super) fn public_failure_object_allowed(reply: &str) -> bool {
    if reply.len() > 24 * 1024 || super::reply_contains_obvious_internal_trace(reply) {
        return false;
    }
    let Ok(value) = serde_json::from_str::<Value>(reply) else {
        return false;
    };
    fn safe(value: &Value, depth: usize) -> bool {
        if depth > 12 {
            return false;
        }
        match value {
            Value::Object(fields) => fields.iter().all(|(key, value)| {
                !crate::visible_text::sensitive_key_name(key)
                    && !matches!(
                        key.trim()
                            .to_ascii_lowercase()
                            .replace(['-', '.'], "_")
                            .as_str(),
                        "authorization"
                            | "cookie"
                            | "cookies"
                            | "headers"
                            | "api_key"
                            | "user_key"
                            | "private_key"
                            | "password"
                            | "secret"
                            | "credentials"
                            | "raw_response"
                            | "raw_request"
                            | "request_payload"
                            | "response_payload"
                            | "resume_context"
                            | "execution_binding"
                            | "stack_trace"
                            | "policy_grant"
                    )
                    && safe(value, depth + 1)
            }),
            Value::Array(items) => items.iter().all(|value| safe(value, depth + 1)),
            Value::String(text) => {
                !serde_json::from_str::<Value>(text)
                    .is_ok_and(|value| value.is_object() || value.is_array())
                    && crate::visible_text::redact_sensitive_text(text) == *text
                    && !super::reply_contains_obvious_internal_trace(text)
            }
            _ => true,
        }
    }
    let truthful_status = value.get("status").is_none_or(|status| {
        matches!(
            status.as_str(),
            Some("error" | "failed" | "partial" | "blocked" | "unavailable")
        )
    });
    value.is_object() && truthful_status && safe(&value, 0)
}

#[cfg(test)]
#[path = "fallback_failure_presentation_tests.rs"]
mod tests;
