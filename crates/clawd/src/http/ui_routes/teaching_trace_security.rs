#[derive(Debug, Default, Deserialize)]
struct TeachingTraceQuery {
    #[serde(default)]
    teaching: Option<bool>,
}

fn teaching_trace_error(
    status: StatusCode,
    error_code: &'static str,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    (
        status,
        Json(ApiResponse {
            ok: false,
            data: Some(json!({
                "error_code": error_code,
                "message_key": format!("clawd.ui.teaching_trace.{error_code}"),
            })),
            error: Some(error_code.to_string()),
        }),
    )
}

fn teaching_trace_access_scope(
    identity: &AuthIdentity,
    task_user_key: Option<&str>,
    opted_in: bool,
) -> Result<&'static str, &'static str> {
    if !opted_in {
        return Err("teaching_trace_opt_in_required");
    }
    if identity.role.eq_ignore_ascii_case("admin") {
        return Ok("admin");
    }
    let owner = task_user_key.map(str::trim).filter(|value| !value.is_empty());
    if owner == Some(identity.user_key.trim()) {
        return Ok("task_owner");
    }
    Err("teaching_trace_access_denied")
}

fn teaching_trace_sensitive_key(key: &str) -> bool {
    let normalized = key.trim().to_ascii_lowercase().replace(['-', '.'], "_");
    matches!(
        normalized.as_str(),
        "api_key"
            | "auth"
            | "authorization"
            | "cookie"
            | "credential"
            | "credentials"
            | "key"
            | "password"
            | "passphrase"
            | "passwd"
            | "private_key"
            | "refresh_token"
            | "secret"
            | "signature"
            | "ticket"
            | "token"
    ) || normalized.ends_with("_api_key")
        || normalized.ends_with("_password")
        || normalized.ends_with("_secret")
        || normalized.ends_with("_token")
}

fn redact_teaching_value(value: &mut Value, parent_key: Option<&str>, depth: usize) -> usize {
    if depth > 16 {
        *value = json!({
            "redacted": true,
            "reason_code": "teaching_trace_depth_limit",
        });
        return 1;
    }
    if parent_key.is_some_and(teaching_trace_sensitive_key) {
        *value = json!("[REDACTED]");
        return 1;
    }
    match value {
        Value::Object(map) => map
            .iter_mut()
            .map(|(key, child)| redact_teaching_value(child, Some(key), depth + 1))
            .sum(),
        Value::Array(items) => items
            .iter_mut()
            .map(|item| redact_teaching_value(item, parent_key, depth + 1))
            .sum(),
        Value::String(text) => {
            let redacted = crate::visible_text::redact_sensitive_text(text);
            if redacted == *text {
                0
            } else {
                *text = redacted;
                1
            }
        }
        _ => 0,
    }
}

fn redact_task_debug_entries(entries: &mut [TaskDebugEntry]) -> usize {
    let mut total_redacted_fields = 0usize;
    for entry in entries {
        let mut entry_redacted_fields = 0usize;
        for text in [
            &mut entry.prompt,
            &mut entry.response,
            &mut entry.raw_response,
            &mut entry.clean_response,
            &mut entry.error,
        ]
        .into_iter()
        .flatten()
        {
            let redacted = crate::visible_text::redact_sensitive_text(text);
            if redacted != *text {
                *text = redacted;
                entry_redacted_fields += 1;
            }
        }
        if let Some(payload) = entry.request_payload.as_mut() {
            entry_redacted_fields += redact_teaching_value(payload, None, 0);
        }
        if entry_redacted_fields > 0 {
            entry.sanitized = Some(true);
        }
        total_redacted_fields += entry_redacted_fields;
    }
    total_redacted_fields
}

fn teaching_trace_layers() -> Value {
    json!({
        "provider_data": {
            "classification": "redacted_provider_io",
            "fields": [
                "calls.entry.prompt",
                "calls.entry.request_payload",
                "calls.entry.raw_response",
                "calls.entry.response",
                "calls.entry.usage",
            ],
        },
        "rustclaw_decisions": {
            "classification": "parsed_machine_decisions",
            "fields": [
                "calls.flow",
                "flow_summary",
                "memory_trace",
                "model_catalog_trace",
                "resume_trace",
            ],
        },
    })
}
