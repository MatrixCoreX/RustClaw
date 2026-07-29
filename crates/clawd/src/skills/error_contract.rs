use serde_json::{json, Value};

pub(super) const STRUCTURED_SKILL_ERROR_PREFIX: &str = "__RC_SKILL_ERROR__:";

/// Current runner responses are canonical-only. This ratchet must remain empty:
/// adding a producer would reintroduce a machine contract that the runtime is
/// actively removing.
#[allow(dead_code)] // Read by the inventory ratchet and focused contract tests.
pub(super) const CURRENT_LEGACY_ERROR_FIELD_PRODUCERS: &[&str] = &[];

/// Read-only compatibility for structured failures already persisted before
/// `extra.error_code` became canonical. This list has no authority over current
/// runner responses and may only shrink as historical records age out.
pub(super) const HISTORICAL_ERROR_FIELD_PRODUCERS: &[&str] = &[
    "audio_synthesize",
    "audio_transcribe",
    "browser_web",
    "config_edit",
    "config_guard",
    "crypto",
    "db_basic",
    "docker_basic",
    "extension_manager",
    "fs_search",
    "git_basic",
    "health_check",
    "http_basic",
    "image_edit",
    "image_generate",
    "image_vision",
    "invest_copy",
    "kb",
    "log_analyze",
    "map_merchant",
    "music_generate",
    "photo_organize",
    "rss_fetch",
    "service_control",
    "smoke_ping_demo",
    "stock",
    "system_basic",
    "task_control",
    "video_generate",
    "weather",
    "web_search_extract",
    "x",
];

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StructuredSkillError {
    pub(crate) skill: String,
    pub(crate) error_code: String,
    pub(crate) error_text: String,
    pub(crate) platform: Option<String>,
    pub(crate) manager_type: Option<String>,
    pub(crate) service_name: Option<String>,
    pub(crate) extra: Option<Value>,
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn child_extra_object(value: &Value) -> Option<&Value> {
    value.get("extra").filter(|extra| extra.is_object())
}

fn producer_allows_historical_error_field(skill: &str) -> bool {
    HISTORICAL_ERROR_FIELD_PRODUCERS
        .iter()
        .any(|producer| skill.trim().eq_ignore_ascii_case(producer))
}

fn current_error_code(value: &Value) -> Option<String> {
    let extra = child_extra_object(value);
    extra
        .and_then(|extra| string_field(extra, "error_code"))
        .or_else(|| string_field(value, "error_code"))
}

fn historical_error_code(skill: &str, value: &Value) -> Option<String> {
    current_error_code(value).or_else(|| {
        producer_allows_historical_error_field(skill).then(|| {
            let extra = child_extra_object(value);
            extra
                .and_then(|extra| string_field(extra, "error_kind"))
                .or_else(|| string_field(value, "error_kind"))
                .or_else(|| extra.and_then(|extra| string_field(extra, "code")))
                .or_else(|| string_field(value, "code"))
        })?
    })
}

fn canonical_error_extra(skill: &str, error_code: &str, extra: Option<Value>) -> Value {
    let mut object = extra
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    object.remove("error_kind");
    object.remove("code");
    object
        .entry("schema_version".to_string())
        .or_insert_with(|| json!(1));
    object
        .entry("source_skill".to_string())
        .or_insert_with(|| json!(skill.trim()));
    object
        .entry("status".to_string())
        .or_insert_with(|| json!("error"));
    object.insert("error_code".to_string(), json!(error_code));
    object
        .entry("message_key".to_string())
        .or_insert_with(|| json!(format!("skill.{}.{}", skill.trim(), error_code)));
    object
        .entry("retryable".to_string())
        .or_insert_with(|| json!(false));
    Value::Object(object)
}

pub(super) fn structured_skill_error_string(skill: &str, value: &Value) -> String {
    let extra_object = child_extra_object(value);
    let error_code = current_error_code(value).unwrap_or_else(|| "unknown".to_string());
    let error_text = string_field(value, "error_text")
        .or_else(|| extra_object.and_then(|extra| string_field(extra, "failure_reason")))
        .unwrap_or_else(|| "skill execution failed".to_string());
    let extra = canonical_error_extra(skill, &error_code, value.get("extra").cloned());
    let payload = json!({
        "skill": skill.trim(),
        "error_code": error_code,
        "error_text": error_text,
        "platform": string_field(value, "platform")
            .or_else(|| extra_object.and_then(|extra| string_field(extra, "platform"))),
        "manager_type": extra_object.and_then(|extra| string_field(extra, "manager_type")),
        "service_name": extra_object.and_then(|extra| string_field(extra, "service_name")),
        "extra": extra,
        "text": Value::Null,
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
    format!("{STRUCTURED_SKILL_ERROR_PREFIX}{encoded}")
}

pub(crate) fn structured_skill_error_from_parts(
    skill: &str,
    error_code: &str,
    error_text: &str,
    platform: Option<&str>,
    extra: Option<Value>,
) -> String {
    let extra = canonical_error_extra(skill, error_code, extra);
    let payload = json!({
        "skill": skill.trim(),
        "error_code": error_code,
        "error_text": error_text,
        "platform": platform,
        "extra": extra,
        "text": Value::Null,
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
    format!("{STRUCTURED_SKILL_ERROR_PREFIX}{encoded}")
}

pub(crate) fn parse_structured_skill_error(err: &str) -> Option<StructuredSkillError> {
    let payload = err.trim().strip_prefix(STRUCTURED_SKILL_ERROR_PREFIX)?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    let skill = string_field(&value, "skill").unwrap_or_default();
    let error_code = historical_error_code(&skill, &value).unwrap_or_else(|| "unknown".to_string());
    let error_text =
        string_field(&value, "error_text").unwrap_or_else(|| "skill execution failed".to_string());
    let extra = canonical_error_extra(&skill, &error_code, value.get("extra").cloned());
    Some(StructuredSkillError {
        skill,
        error_code,
        error_text,
        platform: string_field(&value, "platform"),
        manager_type: string_field(&value, "manager_type"),
        service_name: string_field(&value, "service_name"),
        extra: Some(extra),
    })
}

#[cfg(test)]
#[path = "error_contract_tests.rs"]
mod tests;
