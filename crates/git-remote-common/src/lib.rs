use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[cfg(feature = "forge")]
pub mod forge;
pub mod git;

#[derive(Debug, Deserialize)]
struct Request {
    request_id: String,
    args: Value,
}

#[derive(Debug, Serialize)]
pub struct Response {
    request_id: String,
    status: &'static str,
    text: String,
    error_text: Option<String>,
    extra: Value,
}

#[derive(Debug)]
pub struct SkillError {
    pub code: &'static str,
    pub detail: String,
    pub retryable: bool,
    pub failure_phase: &'static str,
    pub side_effect_applied: bool,
    pub detail_extra: Option<Value>,
}

impl SkillError {
    pub fn new(code: &'static str) -> Self {
        Self {
            code,
            detail: code.to_string(),
            retryable: false,
            failure_phase: "pre_dispatch",
            side_effect_applied: false,
            detail_extra: None,
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    pub fn phase(mut self, phase: &'static str) -> Self {
        self.failure_phase = phase;
        self
    }

    pub fn applied(mut self, applied: bool) -> Self {
        self.side_effect_applied = applied;
        self
    }

    pub fn with_extra(mut self, extra: Value) -> Self {
        self.detail_extra = Some(extra);
        self
    }
}

pub fn dispatch_line(
    source_skill: &'static str,
    line: &str,
    execute: fn(&serde_json::Map<String, Value>) -> Result<Value, SkillError>,
) -> Response {
    let request: Request = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(_) => {
            return error_response(
                source_skill,
                "unknown".to_string(),
                SkillError::new("invalid_input"),
            );
        }
    };
    let args = match request.args.as_object() {
        Some(args) => args,
        None => {
            return error_response(
                source_skill,
                request.request_id,
                SkillError::new("invalid_args"),
            );
        }
    };
    match execute(args) {
        Ok(mut extra) => {
            let object = extra.as_object_mut().expect("skill success must be object");
            object.entry("schema_version").or_insert_with(|| json!(1));
            object
                .entry("source_skill")
                .or_insert_with(|| json!(source_skill));
            object.entry("status").or_insert_with(|| json!("ok"));
            Response {
                request_id: request.request_id,
                status: "ok",
                text: compact_success_text(&extra),
                error_text: None,
                extra,
            }
        }
        Err(error) => error_response(source_skill, request.request_id, error),
    }
}

fn error_response(source_skill: &'static str, request_id: String, error: SkillError) -> Response {
    let mut extra = json!({
        "schema_version": 1,
        "source_skill": source_skill,
        "status": "error",
        "error_code": error.code,
        "message_key": format!("skill.{source_skill}.{}", error.code),
        "retryable": error.retryable,
        "failure_phase": error.failure_phase,
        "side_effect_applied": error.side_effect_applied,
    });
    if let (Some(root), Some(detail)) = (extra.as_object_mut(), error.detail_extra) {
        root.insert("detail".to_string(), detail);
    }
    Response {
        request_id,
        status: "error",
        text: String::new(),
        error_text: Some(error.detail),
        extra,
    }
}

fn compact_success_text(extra: &Value) -> String {
    let action = extra
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let status = extra.get("status").and_then(Value::as_str).unwrap_or("ok");
    format!("action={action} status={status}")
}

pub fn required_string<'a>(
    args: &'a serde_json::Map<String, Value>,
    key: &str,
    error_code: &'static str,
) -> Result<&'a str, SkillError> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.chars().any(char::is_control))
        .ok_or_else(|| SkillError::new(error_code))
}

pub fn optional_string<'a>(
    args: &'a serde_json::Map<String, Value>,
    key: &str,
    error_code: &'static str,
) -> Result<Option<&'a str>, SkillError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let value = value.trim();
            if value.chars().any(char::is_control) {
                Err(SkillError::new(error_code))
            } else {
                Ok((!value.is_empty()).then_some(value))
            }
        }
        Some(_) => Err(SkillError::new(error_code)),
    }
}

pub fn optional_bool(
    args: &serde_json::Map<String, Value>,
    key: &str,
    default: bool,
) -> Result<bool, SkillError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(SkillError::new("invalid_boolean_arg")),
    }
}
