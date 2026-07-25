use serde_json::{json, Value};
use std::fmt;

#[derive(Debug)]
pub struct OfficeError {
    pub code: &'static str,
    pub message: String,
    pub details: Value,
    pub retryable: bool,
    pub failure_phase: Option<&'static str>,
    pub side_effect_applied: Option<bool>,
    pub recovery_action: Option<&'static str>,
    pub invalid_argument: Option<&'static str>,
}

impl OfficeError {
    pub fn new(code: &'static str, message: impl Into<String>, details: Value) -> Self {
        Self {
            code,
            message: message.into(),
            details,
            retryable: false,
            failure_phase: None,
            side_effect_applied: None,
            recovery_action: None,
            invalid_argument: None,
        }
    }

    pub fn replan_argument(
        code: &'static str,
        message: impl Into<String>,
        details: Value,
        invalid_argument: &'static str,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            details,
            retryable: true,
            failure_phase: Some("pre_dispatch"),
            side_effect_applied: Some(false),
            recovery_action: Some("replan_arguments"),
            invalid_argument: Some(invalid_argument),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_input", message, json!({}))
    }

    pub fn unsupported(message: impl Into<String>, details: Value) -> Self {
        Self::new("unsupported_operation", message, details)
    }

    pub fn extra(&self) -> Value {
        let mut extra = json!({
            "schema_version": 1,
            "source_skill": "office_workspace",
            "status": "error",
            "error_code": self.code,
            "message_key": format!("skill.office_workspace.{}", self.code),
            "retryable": self.retryable,
            "details": self.details,
        });
        if let Some(object) = extra.as_object_mut() {
            if let Some(value) = self.failure_phase {
                object.insert("failure_phase".to_string(), json!(value));
            }
            if let Some(value) = self.side_effect_applied {
                object.insert("side_effect_applied".to_string(), json!(value));
            }
            if let Some(value) = self.recovery_action {
                object.insert("recovery_action".to_string(), json!(value));
            }
            if let Some(value) = self.invalid_argument {
                object.insert("invalid_argument".to_string(), json!(value));
            }
        }
        extra
    }
}

impl fmt::Display for OfficeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for OfficeError {}

pub type OfficeResult<T> = Result<T, OfficeError>;

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
