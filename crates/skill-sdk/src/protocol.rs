use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{SkillSdkError, SkillSdkResult};

pub const MAX_PROTOCOL_LINE_BYTES: usize = 1024 * 1024;
pub const SKILL_PROGRESS_FRAME_SCHEMA_VERSION: u32 = 1;
pub const SKILL_PROGRESS_FRAME_RECORD_TYPE: &str = "skill_progress";
pub const MAX_PROGRESS_FRAME_LINE_BYTES: usize = 16 * 1024;
pub const MAX_PROGRESS_FRAME_PARAMS: usize = 16;
pub const MAX_PROGRESS_FRAMES_PER_INVOCATION: u64 = 256;
pub const MAX_PROGRESS_FRAMES_PER_SECOND: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillProgressKind {
    Progress,
    Heartbeat,
    ArtifactReference,
    LogReference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillProgressReference {
    pub reference_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

/// A machine-only, non-terminal record emitted before the final skill response.
///
/// `detail_key` is resolved by the host presentation layer. Skills cannot send
/// arbitrary user-visible progress prose through this contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillProgressFrame {
    pub schema_version: u32,
    pub record_type: String,
    pub request_id: String,
    pub sequence: u64,
    pub kind: SkillProgressKind,
    pub detail_key: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<SkillProgressReference>,
}

impl SkillProgressFrame {
    pub fn to_line(&self) -> SkillSdkResult<String> {
        validate_progress_frame(self, &self.request_id)?;
        let line = serde_json::to_string(self)?;
        if line.len() > MAX_PROGRESS_FRAME_LINE_BYTES {
            return Err(progress_frame_error(
                "progress_frame_oversized",
                format!("bytes={}", line.len()),
            ));
        }
        Ok(line)
    }
}

pub fn validate_progress_frame_line(
    raw: &[u8],
    expected_request_id: &str,
) -> SkillSdkResult<SkillProgressFrame> {
    if raw.is_empty() {
        return Err(progress_frame_error(
            "progress_frame_missing",
            "progress frame is empty",
        ));
    }
    if raw.len() > MAX_PROGRESS_FRAME_LINE_BYTES {
        return Err(progress_frame_error(
            "progress_frame_oversized",
            format!("bytes={}", raw.len()),
        ));
    }
    let text = std::str::from_utf8(raw)
        .map_err(|error| progress_frame_error("progress_frame_utf8_invalid", error.to_string()))?;
    let trimmed = text.trim_end_matches(['\n', '\r']);
    if trimmed.contains('\n') || trimmed.contains('\r') {
        return Err(progress_frame_error(
            "progress_frame_multiple_records",
            "a progress frame must contain exactly one JSON record",
        ));
    }
    let frame: SkillProgressFrame = serde_json::from_str(trimmed)
        .map_err(|error| progress_frame_error("progress_frame_invalid", error.to_string()))?;
    validate_progress_frame(&frame, expected_request_id)?;
    Ok(frame)
}

fn validate_progress_frame(
    frame: &SkillProgressFrame,
    expected_request_id: &str,
) -> SkillSdkResult<()> {
    if frame.schema_version != SKILL_PROGRESS_FRAME_SCHEMA_VERSION {
        return Err(progress_frame_error(
            "progress_frame_schema_unsupported",
            format!("schema_version={}", frame.schema_version),
        ));
    }
    if frame.record_type != SKILL_PROGRESS_FRAME_RECORD_TYPE {
        return Err(progress_frame_error(
            "progress_frame_record_type_invalid",
            format!("record_type={}", frame.record_type),
        ));
    }
    if frame.request_id != expected_request_id {
        return Err(progress_frame_error(
            "progress_frame_request_id_mismatch",
            format!("expected={expected_request_id} actual={}", frame.request_id),
        ));
    }
    if frame.sequence == 0 {
        return Err(progress_frame_error(
            "progress_frame_sequence_invalid",
            "sequence must be greater than zero",
        ));
    }
    if !valid_machine_key(&frame.detail_key, 128) {
        return Err(progress_frame_error(
            "progress_frame_detail_key_invalid",
            "detail_key must be a stable machine key",
        ));
    }
    if frame.params.len() > MAX_PROGRESS_FRAME_PARAMS
        || frame
            .params
            .iter()
            .any(|(key, value)| !valid_machine_key(key, 64) || !valid_progress_param(value))
    {
        return Err(progress_frame_error(
            "progress_frame_params_invalid",
            "params must contain bounded machine values",
        ));
    }
    match (frame.current, frame.total) {
        (None, None) => {}
        (Some(current), Some(total)) if total > 0 && current <= total => {}
        _ => {
            return Err(progress_frame_error(
                "progress_frame_measure_invalid",
                "current and total must be present together with current <= total",
            ));
        }
    }
    match frame.kind {
        SkillProgressKind::ArtifactReference | SkillProgressKind::LogReference => {
            let Some(reference) = frame.reference.as_ref() else {
                return Err(progress_frame_error(
                    "progress_frame_reference_missing",
                    "reference frame requires reference metadata",
                ));
            };
            if !valid_machine_key(&reference.reference_id, 128)
                || reference
                    .media_type
                    .as_deref()
                    .is_some_and(|value| !valid_media_type(value))
            {
                return Err(progress_frame_error(
                    "progress_frame_reference_invalid",
                    "reference metadata is invalid",
                ));
            }
        }
        SkillProgressKind::Progress | SkillProgressKind::Heartbeat if frame.reference.is_some() => {
            return Err(progress_frame_error(
                "progress_frame_reference_unexpected",
                "progress and heartbeat frames cannot contain a reference",
            ));
        }
        SkillProgressKind::Progress | SkillProgressKind::Heartbeat => {}
    }
    Ok(())
}

fn progress_frame_error(code: &'static str, detail: impl Into<String>) -> SkillSdkError {
    SkillSdkError::new(code, detail).phase("progress_frame")
}

fn valid_machine_key(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_media_type(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-' | b'+')
        })
}

fn valid_progress_param(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => true,
        Value::String(value) => value.len() <= 256 && !value.chars().any(char::is_control),
        Value::Array(values) => values.len() <= 16 && values.iter().all(valid_progress_param),
        Value::Object(_) => false,
    }
}

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

/// Validates the complete stdout captured by admission/protocol smoke.
///
/// Runtime streaming applies additional wall-clock rate checks. Admission has
/// no trustworthy per-record timestamps, so it verifies declaration, order,
/// count, schema, request binding, and the mandatory terminal response.
pub fn validate_protocol_output(
    raw: &[u8],
    expected_request_id: &str,
    progress_frames: bool,
) -> SkillSdkResult<ProtocolResponse> {
    if !progress_frames {
        return validate_response_line(raw, expected_request_id);
    }
    if raw.is_empty() {
        return Err(SkillSdkError::new(
            "protocol_response_missing",
            "skill emitted no stdout record",
        )
        .phase("protocol_smoke"));
    }
    let text = std::str::from_utf8(raw).map_err(|error| {
        SkillSdkError::new("protocol_response_utf8_invalid", error.to_string())
            .phase("protocol_smoke")
    })?;
    let records = text.lines().collect::<Vec<_>>();
    let Some((final_record, progress_records)) = records.split_last() else {
        return Err(SkillSdkError::new(
            "protocol_response_missing",
            "skill emitted no stdout record",
        )
        .phase("protocol_smoke"));
    };
    if progress_records.len() as u64 > MAX_PROGRESS_FRAMES_PER_INVOCATION {
        return Err(SkillSdkError::new(
            "progress_frame_total_limit",
            format!("frames={}", progress_records.len()),
        )
        .phase("protocol_smoke"));
    }
    let mut last_sequence = 0_u64;
    for record in progress_records {
        let frame = validate_progress_frame_line(record.as_bytes(), expected_request_id)?;
        if frame.sequence <= last_sequence {
            return Err(SkillSdkError::new(
                "progress_frame_sequence_invalid",
                format!("previous={last_sequence} actual={}", frame.sequence),
            )
            .phase("protocol_smoke"));
        }
        last_sequence = frame.sequence;
    }
    if validate_progress_frame_line(final_record.as_bytes(), expected_request_id).is_ok() {
        return Err(SkillSdkError::new(
            "protocol_final_response_missing",
            "progress-capable skill emitted no final response",
        )
        .phase("protocol_smoke"));
    }
    validate_response_line(final_record.as_bytes(), expected_request_id).map_err(|error| {
        if progress_records.is_empty() {
            error
        } else {
            SkillSdkError::new(
                "protocol_final_response_invalid",
                format!("code={} detail={}", error.code, error.detail),
            )
            .phase("protocol_smoke")
        }
    })
}
