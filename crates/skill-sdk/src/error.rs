use serde::{Deserialize, Serialize};
use std::fmt;

pub type SkillSdkResult<T> = Result<T, SkillSdkError>;

/// Stable, adapter-neutral error returned by SDK, installer, and runtime
/// boundaries. `detail` is diagnostic text; callers must branch on `code`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillSdkError {
    pub code: String,
    pub message_key: String,
    pub detail: String,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
}

impl SkillSdkError {
    pub fn new(code: impl Into<String>, detail: impl Into<String>) -> Self {
        let code = code.into();
        Self {
            message_key: format!("skill_sdk.{code}"),
            code,
            detail: detail.into(),
            retryable: false,
            phase: None,
        }
    }

    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    pub fn phase(mut self, phase: impl Into<String>) -> Self {
        self.phase = Some(phase.into());
        self
    }
}

impl fmt::Display for SkillSdkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for SkillSdkError {}

impl From<std::io::Error> for SkillSdkError {
    fn from(error: std::io::Error) -> Self {
        Self::new("io_failed", error.to_string())
    }
}

impl From<serde_json::Error> for SkillSdkError {
    fn from(error: serde_json::Error) -> Self {
        Self::new("json_invalid", error.to_string())
    }
}
