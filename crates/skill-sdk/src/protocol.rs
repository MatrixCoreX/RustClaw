use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{SkillSdkError, SkillSdkResult};

pub const MAX_PROTOCOL_LINE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolRequest {
    pub request_id: String,
    pub args: Value,
    #[serde(default)]
    pub context: Option<Value>,
    pub user_id: i64,
    pub chat_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolStatus {
    Ok,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolResponse {
    pub request_id: String,
    pub status: ProtocolStatus,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub error_text: Option<String>,
    #[serde(default)]
    pub buttons: Option<Value>,
    /// Canonical machine error identifier. Legacy `error_kind` is accepted on
    /// deserialization only so older installed packages can still be read.
    #[serde(default, alias = "error_kind", skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub validation: Option<Value>,
    #[serde(default)]
    pub extra: Option<Value>,
}

impl ProtocolRequest {
    pub fn smoke(request_id: impl Into<String>) -> Self {
        Self {
            request_id: request_id.into(),
            args: serde_json::json!({"action": "protocol_smoke"}),
            context: Some(serde_json::json!({"protocol_smoke": true})),
            user_id: 0,
            chat_id: 0,
            user_key: None,
        }
    }

    pub fn to_line(&self) -> SkillSdkResult<String> {
        if self.request_id.trim().is_empty() {
            return Err(SkillSdkError::new(
                "protocol_request_id_missing",
                "request_id is required",
            ));
        }
        let line = serde_json::to_string(self)?;
        if line.len() > MAX_PROTOCOL_LINE_BYTES {
            return Err(SkillSdkError::new(
                "protocol_request_oversized",
                format!("bytes={}", line.len()),
            ));
        }
        Ok(line)
    }
}

pub fn validate_response_line(
    raw: &[u8],
    expected_request_id: &str,
) -> SkillSdkResult<ProtocolResponse> {
    if raw.is_empty() {
        return Err(SkillSdkError::new(
            "protocol_response_missing",
            "skill emitted no stdout record",
        )
        .phase("protocol_smoke"));
    }
    if raw.len() > MAX_PROTOCOL_LINE_BYTES {
        return Err(SkillSdkError::new(
            "protocol_response_oversized",
            format!("bytes={}", raw.len()),
        )
        .phase("protocol_smoke"));
    }
    let text = std::str::from_utf8(raw).map_err(|error| {
        SkillSdkError::new("protocol_response_utf8_invalid", error.to_string())
            .phase("protocol_smoke")
    })?;
    let trimmed = text.trim_end_matches(['\n', '\r']);
    if trimmed.contains('\n') || trimmed.contains('\r') {
        return Err(SkillSdkError::new(
            "protocol_multiple_stdout_records",
            "stdout must contain exactly one JSON record",
        )
        .phase("protocol_smoke"));
    }
    let response: ProtocolResponse = serde_json::from_str(trimmed).map_err(|error| {
        SkillSdkError::new("protocol_response_invalid", error.to_string()).phase("protocol_smoke")
    })?;
    if response.request_id != expected_request_id {
        return Err(SkillSdkError::new(
            "protocol_request_id_mismatch",
            format!(
                "expected={expected_request_id} actual={}",
                response.request_id
            ),
        )
        .phase("protocol_smoke"));
    }
    match response.status {
        ProtocolStatus::Ok => {
            if response
                .error_text
                .as_deref()
                .is_some_and(|value| !value.is_empty())
            {
                return Err(SkillSdkError::new(
                    "protocol_success_has_error",
                    "status=ok must not carry error_text",
                )
                .phase("protocol_smoke"));
            }
        }
        ProtocolStatus::Error => {
            if response
                .error_text
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .is_empty()
            {
                return Err(SkillSdkError::new(
                    "protocol_error_text_missing",
                    "status=error requires readable error_text",
                )
                .phase("protocol_smoke"));
            }
            let extra = response.extra.as_ref().and_then(Value::as_object);
            if extra
                .and_then(|value| value.get("error_code"))
                .and_then(Value::as_str)
                .is_none()
                || extra
                    .and_then(|value| value.get("message_key"))
                    .and_then(Value::as_str)
                    .is_none()
            {
                return Err(SkillSdkError::new(
                    "protocol_structured_error_missing",
                    "status=error requires extra.error_code and extra.message_key",
                )
                .phase("protocol_smoke"));
            }
        }
    }
    Ok(response)
}
