#[cfg(test)]
use serde_json::json;
use serde_json::Value;

use crate::task_lifecycle::{AsyncJobRef, AsyncJobStatus};

pub(crate) const ASYNC_POLL_ADAPTER_RESULT_KEY: &str = "async_poll_adapter_result";
pub(crate) const ASYNC_POLL_ADAPTER_KINDS: &[&str] = &[
    "skill_poll",
    "local_process_poll",
    "http_job_poll",
    "mcp_job_poll",
    "media_job_poll",
    "browser_job_poll",
    "remote_job_poll",
];

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
    let adapter_kinds = ASYNC_POLL_ADAPTER_KINDS.join("|");
    format!(
        "async_job_protocol=version:1;phases:{phases};resume_entrypoint:poll_async_job;checkpoint_states:waiting|background;adapter_statuses:{statuses};required_job_fields:job_id|status|poll_after_seconds|expires_at|cancel_ref|message_key;poll_adapter_kinds:{adapter_kinds};adapter_result_key:{ASYNC_POLL_ADAPTER_RESULT_KEY};adapter_result_fields:job_id|status|poll_after_seconds|expires_at|final_result_json|failure_result_json|error_code|message_key;user_text_fields_forbidden:text|error_text"
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

pub(crate) fn parse_pending_async_job_ref_from_extra(
    extra: Option<&Value>,
    error_prefix: &str,
) -> Result<Option<AsyncJobRef>, String> {
    let Some(candidate) = pending_async_job_candidate_from_extra(extra) else {
        return Ok(None);
    };
    if candidate.get("text").is_some() || candidate.get("error_text").is_some() {
        return Err(machine_error(error_prefix, "user_text_fields_forbidden"));
    }
    let Some(status) = parse_pending_async_job_status(candidate) else {
        return Err(machine_error(error_prefix, "unsupported_status"));
    };
    if !matches!(status, AsyncJobStatus::Accepted | AsyncJobStatus::Running) {
        return Err(machine_error(error_prefix, "non_pending_status"));
    }
    let job = AsyncJobRef {
        job_id: required_machine_string(candidate, "job_id").unwrap_or_default(),
        status,
        poll_after_seconds: candidate
            .get("poll_after_seconds")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        expires_at: candidate
            .get("expires_at")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        cancel_ref: required_machine_string(candidate, "cancel_ref").unwrap_or_default(),
        message_key: required_machine_string(candidate, "message_key").unwrap_or_default(),
    };
    let missing = job.missing_required_fields();
    if !missing.is_empty() {
        return Err(format!(
            "{error_prefix}: missing_required_fields={}",
            missing.join("|")
        ));
    }
    Ok(Some(job))
}

pub(crate) fn parse_pending_async_job_poll_adapter_from_extra(
    extra: Option<&Value>,
    error_prefix: &str,
) -> Result<Option<Value>, String> {
    let Some(candidate) = pending_async_job_candidate_from_extra(extra) else {
        return Ok(None);
    };
    let Some(adapter) = candidate
        .get("poll_adapter")
        .or_else(|| extra.and_then(|extra| extra.get("poll_adapter")))
    else {
        return Ok(None);
    };
    if adapter.get("text").is_some() || adapter.get("error_text").is_some() {
        return Err(machine_error(
            error_prefix,
            "poll_adapter_user_text_fields_forbidden",
        ));
    }
    let adapter_kind = adapter
        .get("adapter_kind")
        .or_else(|| adapter.get("kind"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| machine_error(error_prefix, "poll_adapter_kind_missing"))?;
    if !async_poll_adapter_kind_supported(adapter_kind) {
        return Err(machine_error(error_prefix, "unsupported_poll_adapter_kind"));
    }
    if skill_runner_poll_adapter_kind_supported(adapter_kind) {
        adapter
            .get("skill_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| machine_error(error_prefix, "poll_adapter_skill_name_missing"))?;
    }
    if let Some(args) = adapter.get("args") {
        if !args.is_object() || args.get("text").is_some() || args.get("error_text").is_some() {
            return Err(machine_error(error_prefix, "poll_adapter_args_invalid"));
        }
    }
    Ok(Some(adapter.clone()))
}

pub(crate) fn async_poll_adapter_kind_supported(adapter_kind: &str) -> bool {
    ASYNC_POLL_ADAPTER_KINDS
        .iter()
        .any(|candidate| *candidate == adapter_kind)
}

pub(crate) fn skill_runner_poll_adapter_kind_supported(adapter_kind: &str) -> bool {
    matches!(
        adapter_kind,
        "skill_poll"
            | "http_job_poll"
            | "mcp_job_poll"
            | "media_job_poll"
            | "browser_job_poll"
            | "remote_job_poll"
    )
}

fn pending_async_job_candidate_from_extra(extra: Option<&Value>) -> Option<&Value> {
    let extra = extra?;
    extra
        .get("pending_async_job")
        .or_else(|| extra.get("async_job"))
        .or_else(|| {
            let kind = extra
                .get("type")
                .or_else(|| extra.get("kind"))
                .and_then(Value::as_str)
                .map(str::trim);
            (kind == Some("pending_async_job")).then_some(extra)
        })
}

fn parse_pending_async_job_status(value: &Value) -> Option<AsyncJobStatus> {
    match value.get("status").and_then(Value::as_str).map(str::trim)? {
        "accepted" => Some(AsyncJobStatus::Accepted),
        "running" => Some(AsyncJobStatus::Running),
        "succeeded" => Some(AsyncJobStatus::Succeeded),
        "failed" => Some(AsyncJobStatus::Failed),
        "expired" => Some(AsyncJobStatus::Expired),
        _ => None,
    }
}

fn required_machine_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn machine_error(error_prefix: &str, reason: &str) -> String {
    format!("{error_prefix}: {reason}")
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
        "poll_adapter_kinds": ASYNC_POLL_ADAPTER_KINDS,
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
