use std::sync::OnceLock;

use regex::Regex;

const REDACTED: &str = "[REDACTED]";

pub(crate) fn sanitize_user_visible_text(text: &str) -> String {
    let stripped = strip_ansi_sequences(text);
    let stripped = replace_structured_skill_error_payloads(&stripped);
    let redacted = redact_sensitive_url_params(&stripped);
    let redacted = redact_sensitive_key_value_pairs(&redacted);
    let redacted = redact_sensitive_json_string_fields(&redacted);
    redact_authorization_values(&redacted)
}

fn replace_structured_skill_error_payloads(text: &str) -> String {
    const PREFIX: &str = "__RC_SKILL_ERROR__:";
    let mut remaining = text;
    let mut out = String::new();
    while let Some(pos) = remaining.find(PREFIX) {
        out.push_str(&remaining[..pos]);
        let after_prefix = &remaining[pos + PREFIX.len()..];
        let Some((payload, consumed)) = take_json_object_prefix(after_prefix) else {
            out.push_str("skill execution failed");
            remaining = "";
            continue;
        };
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) {
            let replacement = value
                .get("error_text")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .or_else(|| {
                    value
                        .get("text")
                        .and_then(|text| text.get("failure_reason"))
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                })
                .unwrap_or("skill execution failed");
            out.push_str(replacement);
        } else {
            out.push_str("skill execution failed");
        }
        remaining = &after_prefix[consumed..];
    }
    out.push_str(remaining);
    out
}

fn take_json_object_prefix(text: &str) -> Option<(&str, usize)> {
    let bytes = text.as_bytes();
    if bytes.first().copied()? != b'{' {
        return None;
    }
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, ch) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth = depth.saturating_add(1),
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let end = idx + ch.len_utf8();
                    return Some((&text[..end], end));
                }
            }
            _ => {}
        }
    }
    None
}

fn strip_ansi_sequences(text: &str) -> String {
    static OSC_RE: OnceLock<Regex> = OnceLock::new();
    static CSI_RE: OnceLock<Regex> = OnceLock::new();
    static JSON_ESCAPED_CSI_RE: OnceLock<Regex> = OnceLock::new();

    let text = OSC_RE
        .get_or_init(|| Regex::new(r"\x1B\][^\x07]*(?:\x07|\x1B\\)").expect("valid OSC regex"))
        .replace_all(text, "")
        .into_owned();
    let text = CSI_RE
        .get_or_init(|| Regex::new(r"\x1B\[[0-?]*[ -/]*[@-~]").expect("valid CSI regex"))
        .replace_all(&text, "")
        .into_owned();
    JSON_ESCAPED_CSI_RE
        .get_or_init(|| {
            Regex::new(r"\\u001[bB]\[[0-?]*[ -/]*[@-~]").expect("valid JSON escaped CSI regex")
        })
        .replace_all(&text, "")
        .into_owned()
}

fn sensitive_key_name(key: &str) -> bool {
    let normalized = key
        .trim()
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-')
        .to_ascii_lowercase()
        .replace('-', "_");
    if normalized.is_empty() {
        return false;
    }
    normalized == "key"
        || normalized == "auth"
        || normalized.ends_with("_key")
        || normalized.contains("token")
        || normalized.contains("secret")
        || normalized.contains("password")
        || normalized.contains("passwd")
        || normalized.contains("cookie")
        || normalized.contains("credential")
        || normalized.contains("ticket")
        || normalized.contains("signature")
}

fn redact_sensitive_url_params(text: &str) -> String {
    static QUERY_PARAM_RE: OnceLock<Regex> = OnceLock::new();
    QUERY_PARAM_RE
        .get_or_init(|| {
            Regex::new(
                r#"(?P<prefix>[?&;])(?P<key>[A-Za-z0-9_.-]{1,80})=(?P<value>[^&#\s"'<>()\\]+)"#,
            )
            .expect("valid query param regex")
        })
        .replace_all(text, |caps: &regex::Captures<'_>| {
            let whole = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
            let key = caps.name("key").map(|m| m.as_str()).unwrap_or_default();
            if sensitive_key_name(key) {
                format!(
                    "{}{}={}",
                    caps.name("prefix").map(|m| m.as_str()).unwrap_or_default(),
                    key,
                    REDACTED
                )
            } else {
                whole.to_string()
            }
        })
        .into_owned()
}

fn redact_sensitive_key_value_pairs(text: &str) -> String {
    static KV_RE: OnceLock<Regex> = OnceLock::new();
    KV_RE
        .get_or_init(|| {
            Regex::new(r#"\b(?P<key>[A-Za-z][A-Za-z0-9_.-]{0,80})(?P<sep>\s*=\s*)(?P<value>[^\s"'`<>&;\\]+)"#)
                .expect("valid key-value regex")
        })
        .replace_all(text, |caps: &regex::Captures<'_>| {
            let whole = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
            let key = caps.name("key").map(|m| m.as_str()).unwrap_or_default();
            if sensitive_key_name(key) {
                format!(
                    "{}{}{}",
                    key,
                    caps.name("sep").map(|m| m.as_str()).unwrap_or("="),
                    REDACTED
                )
            } else {
                whole.to_string()
            }
        })
        .into_owned()
}

fn redact_sensitive_json_string_fields(text: &str) -> String {
    static JSON_FIELD_RE: OnceLock<Regex> = OnceLock::new();
    JSON_FIELD_RE
        .get_or_init(|| {
            Regex::new(
                r#""(?P<key>[^"\\]*(?:\\.[^"\\]*)*)"\s*:\s*"(?P<value>[^"\\]*(?:\\.[^"\\]*)*)""#,
            )
            .expect("valid JSON string field regex")
        })
        .replace_all(text, |caps: &regex::Captures<'_>| {
            let whole = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
            let key = caps.name("key").map(|m| m.as_str()).unwrap_or_default();
            if sensitive_key_name(key) {
                format!("\"{key}\":\"{REDACTED}\"")
            } else {
                whole.to_string()
            }
        })
        .into_owned()
}

fn redact_authorization_values(text: &str) -> String {
    static AUTH_HEADER_RE: OnceLock<Regex> = OnceLock::new();
    static BEARER_RE: OnceLock<Regex> = OnceLock::new();

    let text = AUTH_HEADER_RE
        .get_or_init(|| {
            Regex::new(r"(?i)\b(?P<prefix>authorization\s*[:=]\s*)(?P<scheme>bearer\s+|basic\s+)?(?P<value>[A-Za-z0-9._~+/=-]{8,})")
                .expect("valid authorization regex")
        })
        .replace_all(text, |caps: &regex::Captures<'_>| {
            format!(
                "{}{}{}",
                caps.name("prefix").map(|m| m.as_str()).unwrap_or_default(),
                caps.name("scheme").map(|m| m.as_str()).unwrap_or_default(),
                REDACTED
            )
        })
        .into_owned();
    BEARER_RE
        .get_or_init(|| {
            Regex::new(r"(?i)\b(?P<prefix>bearer\s+)(?P<value>[A-Za-z0-9._~+/=-]{8,})")
                .expect("valid bearer regex")
        })
        .replace_all(&text, |caps: &regex::Captures<'_>| {
            format!(
                "{}{}",
                caps.name("prefix").map(|m| m.as_str()).unwrap_or_default(),
                REDACTED
            )
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::sanitize_user_visible_text;

    #[test]
    fn sanitizes_ansi_and_sensitive_url_params() {
        let raw = "\u{1b}[32mconnected\u{1b}[0m to wss://host/ws?device_id=123&access_key=abc123&service_id=7&ticket=deadbeef";

        let sanitized = sanitize_user_visible_text(raw);

        assert_eq!(
            sanitized,
            "connected to wss://host/ws?device_id=123&access_key=[REDACTED]&service_id=7&ticket=[REDACTED]"
        );
    }

    #[test]
    fn sanitizes_json_escaped_ansi_and_sensitive_fields() {
        let raw =
            r#"{"excerpt":"1|\u001b[32mok\u001b[0m token=abc123456789","api_secret":"plain"}"#;

        let sanitized = sanitize_user_visible_text(raw);

        assert!(!sanitized.contains("\\u001b"));
        assert!(sanitized.contains("token=[REDACTED]"));
        assert!(sanitized.contains(r#""api_secret":"[REDACTED]""#));
        assert!(!sanitized.contains("abc123456789"));
        assert!(!sanitized.contains("plain"));
    }

    #[test]
    fn sanitizes_structured_skill_error_payloads() {
        let raw = r#"已尝试访问文件，但执行失败：__RC_SKILL_ERROR__:{"skill":"archive_basic","error_kind":"unknown","error_text":"archive is required","text":null}。"#;

        let sanitized = sanitize_user_visible_text(raw);

        assert_eq!(
            sanitized,
            "已尝试访问文件，但执行失败：archive is required。"
        );
        assert!(!sanitized.contains("__RC_SKILL_ERROR__"));
        assert!(!sanitized.contains("\"skill\""));
    }

    #[test]
    fn malformed_structured_skill_error_payload_does_not_leak_marker_tail() {
        let raw = r#"执行失败：__RC_SKILL_ERROR__:{"skill":"archive_basic","error_text":"broken""#;

        let sanitized = sanitize_user_visible_text(raw);

        assert_eq!(sanitized, "执行失败：skill execution failed");
        assert!(!sanitized.contains("__RC_SKILL_ERROR__"));
        assert!(!sanitized.contains("archive_basic"));
    }
}
