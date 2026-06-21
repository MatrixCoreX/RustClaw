#[cfg(test)]
use serde_json::json;
use serde_json::Value;

pub(crate) const ASYNC_POLL_ADAPTER_RESULT_KEY: &str = "async_poll_adapter_result";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AsyncJobProtocolPhase {
    Start,
    Checkpoint,
    Poll,
    Observation,
    VerifyFinalize,
}

impl AsyncJobProtocolPhase {
    fn all() -> &'static [Self] {
        &[
            Self::Start,
            Self::Checkpoint,
            Self::Poll,
            Self::Observation,
            Self::VerifyFinalize,
        ]
    }

    fn as_token(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Checkpoint => "checkpoint",
            Self::Poll => "poll",
            Self::Observation => "observation",
            Self::VerifyFinalize => "verify_finalize",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AsyncJobAdapterStatus {
    Accepted,
    Running,
    Succeeded,
    Failed,
    Expired,
}

impl AsyncJobAdapterStatus {
    fn all() -> &'static [Self] {
        &[
            Self::Accepted,
            Self::Running,
            Self::Succeeded,
            Self::Failed,
            Self::Expired,
        ]
    }

    fn as_token(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Expired => "expired",
        }
    }
}

pub(crate) fn async_job_protocol_hint_line() -> String {
    let phases = AsyncJobProtocolPhase::all()
        .iter()
        .map(|phase| phase.as_token())
        .collect::<Vec<_>>()
        .join("|");
    let statuses = AsyncJobAdapterStatus::all()
        .iter()
        .map(|status| status.as_token())
        .collect::<Vec<_>>()
        .join("|");
    format!(
        "async_job_protocol=version:1;phases:{phases};resume_entrypoint:poll_async_job;checkpoint_states:waiting|background;adapter_statuses:{statuses};required_job_fields:job_id|status|poll_after_seconds|expires_at|cancel_ref|message_key;adapter_result_key:{ASYNC_POLL_ADAPTER_RESULT_KEY};adapter_result_fields:job_id|status|poll_after_seconds|expires_at|final_result_json|failure_result_json|error_code|message_key;user_text_fields_forbidden:text|error_text"
    )
}

pub(crate) fn async_poll_adapter_result_matches_job(value: &Value, job_id: &str) -> bool {
    value.is_object()
        && value.get("text").is_none()
        && value.get("error_text").is_none()
        && value.get("job_id").and_then(Value::as_str).map(str::trim) == Some(job_id)
        && async_poll_adapter_status(value).is_some()
}

pub(crate) fn async_poll_adapter_status(value: &Value) -> Option<&str> {
    let status = value.get("status").and_then(Value::as_str).map(str::trim)?;
    AsyncJobAdapterStatus::all()
        .iter()
        .any(|candidate| candidate.as_token() == status)
        .then_some(status)
}

#[cfg(test)]
pub(crate) fn async_job_contract_json() -> Value {
    json!({
        "schema_version": 1,
        "phases": AsyncJobProtocolPhase::all()
            .iter()
            .map(|phase| phase.as_token())
            .collect::<Vec<_>>(),
        "resume_entrypoint": "poll_async_job",
        "checkpoint_states": ["waiting", "background"],
        "adapter_statuses": AsyncJobAdapterStatus::all()
            .iter()
            .map(|status| status.as_token())
            .collect::<Vec<_>>(),
        "required_job_fields": [
            "job_id",
            "status",
            "poll_after_seconds",
            "expires_at",
            "cancel_ref",
            "message_key"
        ],
        "adapter_result_key": ASYNC_POLL_ADAPTER_RESULT_KEY,
        "adapter_result_fields": [
            "job_id",
            "status",
            "poll_after_seconds",
            "expires_at",
            "final_result_json",
            "failure_result_json",
            "error_code",
            "message_key"
        ],
        "forbidden_user_text_fields": ["text", "error_text"]
    })
}

#[cfg(test)]
#[path = "async_job_contract_tests.rs"]
mod tests;
