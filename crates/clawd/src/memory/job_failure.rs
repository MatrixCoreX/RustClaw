//! Typed durable-job failures. Never infer control flow from error prose.
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct JobFailure {
    pub error_code: String,
    pub message_key: String,
    pub retryable: bool,
    pub phase: String,
    pub provider_error_kind: Option<String>,
}

impl std::fmt::Display for JobFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.error_code)
    }
}
impl std::error::Error for JobFailure {}

pub(crate) fn permanent(error_code: &'static str) -> anyhow::Error {
    JobFailure::new(error_code, false, "preflight").into()
}

impl JobFailure {
    pub(crate) fn new(error_code: &str, retryable: bool, phase: &str) -> Self {
        Self {
            error_code: error_code.into(),
            message_key: format!("memory.{error_code}"),
            retryable,
            phase: phase.into(),
            provider_error_kind: None,
        }
    }

    pub(crate) fn from_error(error: &anyhow::Error) -> Self {
        if let Some(failure) = error.downcast_ref::<Self>() {
            return failure.clone();
        }
        if let Some(rusqlite::Error::SqliteFailure(inner, _)) =
            error.downcast_ref::<rusqlite::Error>()
        {
            use rusqlite::ErrorCode;
            return match inner.code {
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => {
                    Self::new("memory_job_database_busy", true, "database")
                }
                ErrorCode::DiskFull => Self::new("memory_job_storage_full", false, "database"),
                ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase => {
                    Self::new("memory_job_database_corrupt", false, "database")
                }
                _ => Self::new("memory_job_database_failed", false, "database"),
            };
        }
        Self::new("memory_job_internal_failure", false, "execution")
    }

    pub(crate) fn provider(phase: &str) -> Self {
        let mut result = Self::new("memory_job_provider_failed", false, phase);
        if let Some((class, retryable)) = crate::runtime::provider_diagnostics::current() {
            result.error_code = format!("memory_job_provider_{}", class.as_str());
            result.message_key = class.policy().message_key.into();
            result.retryable = retryable && class.policy().provider_retryable;
            result.provider_error_kind = Some(class.as_str().into());
        }
        result
    }
}

#[cfg(test)]
#[path = "job_failure_tests.rs"]
mod tests;
