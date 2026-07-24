use serde_json::{json, Value};
use std::fmt;

#[derive(Debug)]
pub struct OfficeError {
    pub code: &'static str,
    pub message: String,
    pub details: Value,
    pub retryable: bool,
}

impl OfficeError {
    pub fn new(code: &'static str, message: impl Into<String>, details: Value) -> Self {
        Self {
            code,
            message: message.into(),
            details,
            retryable: false,
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_input", message, json!({}))
    }

    pub fn unsupported(message: impl Into<String>, details: Value) -> Self {
        Self::new("unsupported_operation", message, details)
    }

    pub fn extra(&self) -> Value {
        json!({
            "schema_version": 1,
            "source_skill": "office_workspace",
            "status": "error",
            "error_code": self.code,
            "message_key": format!("skill.office_workspace.{}", self.code),
            "retryable": self.retryable,
            "details": self.details,
        })
    }
}

impl fmt::Display for OfficeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for OfficeError {}

pub type OfficeResult<T> = Result<T, OfficeError>;
