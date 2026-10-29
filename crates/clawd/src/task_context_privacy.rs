use serde_json::Value;

pub(crate) fn redact(value: &Value, depth: usize) -> Result<Value, &'static str> {
    if depth > 12 {
        return Err("context_value_too_deep");
    }
    Ok(match value {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .filter(|(key, _)| !sensitive_key(key))
                .map(|(key, value)| Ok::<_, &'static str>((key.clone(), redact(value, depth + 1)?)))
                .collect::<Result<_, _>>()?,
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| redact(v, depth + 1))
                .collect::<Result<_, _>>()?,
        ),
        Value::String(text) => Value::String(redact_value_text(text)),
        other => other.clone(),
    })
}

fn redact_value_text(text: &str) -> String {
    let safe = crate::visible_text::redact_sensitive_text(text);
    let Ok(mut url) = reqwest::Url::parse(&safe) else {
        return safe;
    };
    if url.username().is_empty() && url.password().is_none() {
        return safe;
    }
    if url.set_username("").is_err() || url.set_password(None).is_err() {
        return "[REDACTED]".into();
    }
    url.into()
}

pub(crate) fn sensitive_key(key: &str) -> bool {
    if key.starts_with("__") {
        return true;
    }
    let key = key.to_ascii_lowercase();
    let canonical = key
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>();
    key.split(['_', '-']).any(|part| {
        matches!(
            part,
            "confirm"
                | "confirmation"
                | "approved"
                | "approval"
                | "permission"
                | "permissions"
                | "policy"
                | "yolo"
                | "secret"
                | "password"
                | "token"
                | "authorization"
                | "cookie"
                | "credential"
                | "credentials"
                | "grant"
                | "private"
                | "headers"
        )
    }) || [
        "apikey",
        "userkey",
        "principalid",
        "privatekey",
        "accesskey",
        "accesstoken",
        "refreshtoken",
        "sessiontoken",
        "clientsecret",
        "credentialtoken",
        "permissionmode",
        "policygrant",
        "approvaltoken",
        "confirmationtoken",
    ]
    .iter()
    .any(|suffix| canonical.ends_with(suffix))
}

#[cfg(test)]
#[path = "task_context_privacy_tests.rs"]
mod tests;
