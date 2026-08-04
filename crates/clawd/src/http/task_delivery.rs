use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use claw_core::channel_delivery::{
    ChannelTaskDeliveryContent, ChannelTaskDeliveryRequest, ChannelTaskDeliveryResponse,
    ChannelTaskDeliveryStatus, CHANNEL_TASK_DELIVERY_RESPONSE_SCHEMA_VERSION,
};
use claw_core::channel_notice::{ChannelNotice, ChannelNoticeActionKind, ChannelNoticeNextAction};
use claw_core::types::{ApiResponse, TaskStatus};
use serde::Serialize;
use serde_json::Value;
use tracing::{error, warn};
use uuid::Uuid;

use crate::repo::TaskDeliveryRecord;
use crate::AppState;

pub(crate) async fn deliver_task_result(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(task_id): AxumPath<Uuid>,
    Json(request): Json<ChannelTaskDeliveryRequest>,
) -> (StatusCode, Json<ApiResponse<ChannelTaskDeliveryResponse>>) {
    if request.validate().is_err() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "channel_task_delivery_request_invalid",
        );
    }
    let record = match crate::repo::get_task_delivery_record(&state, &task_id.to_string()) {
        Ok(Some(record)) => record,
        Ok(None) => return api_error(StatusCode::NOT_FOUND, "task_not_found"),
        Err(error) => {
            error!(
                event = "channel_task_delivery_load_failed",
                task_id = %task_id,
                diagnostic = %error,
                "channel task delivery load failed"
            );
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "channel_task_delivery_load_failed",
            );
        }
    };
    if !authorized_delivery_request(&state, &headers, &record) {
        return api_error(StatusCode::UNAUTHORIZED, "task_delivery_unauthorized");
    }
    let status = crate::parse_task_status(&record.status);
    if matches!(status, TaskStatus::Queued | TaskStatus::Running) {
        return api_error(StatusCode::CONFLICT, "task_not_terminal");
    }
    if record.task.channel == "ui" {
        return api_error(
            StatusCode::BAD_REQUEST,
            "task_channel_delivery_not_supported",
        );
    }
    let payload = match serde_json::from_str::<Value>(&record.task.payload_json) {
        Ok(payload) => payload,
        Err(error) => {
            warn!(
                event = "channel_task_delivery_payload_invalid",
                task_id = %task_id,
                diagnostic = %error,
                "channel task delivery payload invalid"
            );
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "channel_task_delivery_payload_invalid",
            );
        }
    };
    let (text, notice) = terminal_delivery_content(&state, &record, &payload, status.clone());
    let text = if matches!(status, TaskStatus::Succeeded) {
        project_terminal_delivery_content(&text, request.content)
    } else {
        text
    };
    if text.trim().is_empty() {
        return api_ok(ChannelTaskDeliveryResponse {
            schema_version: CHANNEL_TASK_DELIVERY_RESPONSE_SCHEMA_VERSION,
            status: ChannelTaskDeliveryStatus::NotRequired,
            accepted: true,
            delivered: true,
            receipt: None,
            error_code: None,
            message_key: None,
            retryable: false,
        });
    }
    let envelope = match crate::delivery_service::build_daemon_delivery_envelope(
        &state,
        &record.task,
        &payload,
        &text,
        request.source,
        request.content,
        notice,
    ) {
        Ok(envelope) => envelope,
        Err(error) => {
            warn!(
                event = "channel_task_delivery_envelope_failed",
                task_id = %task_id,
                diagnostic = %error,
                "channel task delivery envelope failed"
            );
            return api_error(
                StatusCode::BAD_REQUEST,
                "channel_task_delivery_envelope_invalid",
            );
        }
    };
    match crate::delivery_service::deliver_task_envelope(&state, &record.task, &payload, &envelope)
        .await
    {
        Ok(result) => api_ok(result.into_task_response()),
        Err(error) => {
            error!(
                event = "channel_task_delivery_dispatch_failed",
                task_id = %task_id,
                diagnostic = %error,
                "channel task delivery dispatch failed"
            );
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "channel_task_delivery_dispatch_failed",
            )
        }
    }
}

fn authorized_delivery_request(
    state: &AppState,
    headers: &HeaderMap,
    record: &TaskDeliveryRecord,
) -> bool {
    let expected = record
        .task
        .user_key
        .as_deref()
        .map(crate::normalize_user_key)
        .filter(|value| !value.is_empty());
    let provided = crate::auth_key_from_headers(headers)
        .map(crate::normalize_user_key)
        .filter(|value| !value.is_empty());
    let (Some(expected), Some(provided)) = (expected, provided) else {
        return false;
    };
    if expected != provided {
        return false;
    }
    matches!(
        crate::resolve_auth_identity_by_key(state, &provided),
        Ok(Some(_))
    )
}

fn terminal_delivery_content(
    state: &AppState,
    record: &TaskDeliveryRecord,
    payload: &Value,
    status: TaskStatus,
) -> (String, Option<ChannelNotice>) {
    let locale = payload
        .pointer("/channel_ingress/locale")
        .and_then(Value::as_str)
        .or_else(|| payload.get("locale").and_then(Value::as_str))
        .unwrap_or("und");
    if matches!(status, TaskStatus::Succeeded) {
        let messages = terminal_success_messages(record.result_json.as_ref());
        let messages = claw_core::task_delivery_artifacts::merge_task_artifact_delivery_messages(
            &record.task.task_id,
            record.result_json.as_ref(),
            &state.skill_rt.workspace_root,
            messages,
        );
        let text = messages
            .into_iter()
            .map(|message| message.trim().to_string())
            .filter(|message| !message.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        let text = if text.is_empty() {
            claw_core::channel_i18n::common_text_for_locale(
                locale,
                "channel.task.completed_without_text",
            )
        } else {
            text
        };
        return (text, None);
    }

    let has_resume_context = record
        .result_json
        .as_ref()
        .and_then(|result| result.get("resume_context"))
        .is_some_and(|value| !value.is_null());
    let failure_detail = (!has_resume_context && matches!(&status, TaskStatus::Failed))
        .then(|| crate::visible_text::structured_task_failure_detail(record.result_json.as_ref()))
        .flatten();
    let (notice_code, error_code, message_key, default_retryable) = if has_resume_context {
        (
            "task.resume_interrupted",
            "task.resume_interrupted",
            "channel.task.resume_interrupted",
            true,
        )
    } else {
        match status {
            TaskStatus::Canceled => (
                "task.canceled",
                "task.canceled",
                "channel.task.canceled",
                false,
            ),
            TaskStatus::Timeout => ("task.timeout", "task.timeout", "channel.task.timeout", true),
            _ => ("task.failed", "task.failed", "channel.task.failed", true),
        }
    };
    let retryable = failure_detail
        .as_ref()
        .map(|detail| detail.retryable)
        .unwrap_or(default_retryable);
    let mut notice = ChannelNotice::error(notice_code, error_code, message_key, retryable);
    notice.diagnostic_id = Some(format!("task:{}", record.task.task_id));
    if let Some(detail) = failure_detail.as_ref() {
        notice
            .params
            .insert("reason_code".to_string(), detail.error_code.clone());
    }
    if retryable {
        notice.next_actions.push(ChannelNoticeNextAction {
            kind: ChannelNoticeActionKind::Retry,
            message_key: None,
            params: Default::default(),
        });
    }
    let text = claw_core::channel_i18n::common_text_for_locale(locale, message_key);
    (text, Some(notice))
}

fn project_terminal_delivery_content(text: &str, content: ChannelTaskDeliveryContent) -> String {
    match content {
        ChannelTaskDeliveryContent::Full => text.trim().to_string(),
        ChannelTaskDeliveryContent::TextOnly => {
            claw_core::channel_delivery_tokens::strip_legacy_delivery_lines(text)
                .trim()
                .to_string()
        }
        ChannelTaskDeliveryContent::MediaOnly => {
            claw_core::channel_delivery_tokens::legacy_delivery_lines(text)
        }
    }
}

fn terminal_success_messages(result: Option<&Value>) -> Vec<String> {
    let messages = result
        .and_then(|value| value.get("messages"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if !messages.is_empty() {
        return messages;
    }
    result
        .and_then(|value| value.get("text"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| vec![value.to_string()])
        .unwrap_or_default()
}

fn api_ok<T: Serialize>(data: T) -> (StatusCode, Json<ApiResponse<T>>) {
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(data),
            error: None,
        }),
    )
}

fn api_error<T: Serialize>(status: StatusCode, error: &str) -> (StatusCode, Json<ApiResponse<T>>) {
    (
        status,
        Json(ApiResponse {
            ok: false,
            data: None,
            error: Some(error.to_string()),
        }),
    )
}

#[cfg(test)]
#[path = "task_delivery_tests.rs"]
mod tests;
