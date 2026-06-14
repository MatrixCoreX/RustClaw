use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use claw_core::types::ApiResponse;
use serde::Deserialize;
use serde_json::json;
use tracing::{error, info};

use crate::AppState;

#[derive(Debug, Deserialize)]
pub(super) struct CancelTasksRequest {
    user_id: i64,
    chat_id: i64,
    exclude_task_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ActiveTasksRequest {
    user_id: i64,
    chat_id: i64,
    exclude_task_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CancelOneTaskRequest {
    user_id: i64,
    chat_id: i64,
    index: usize,
    exclude_task_id: Option<String>,
}

fn authorize_task_admin_request(
    state: &AppState,
    headers: &HeaderMap,
    requested_user_id: i64,
) -> Result<i64, (StatusCode, Json<ApiResponse<serde_json::Value>>)> {
    let provided_key = headers
        .get("x-rustclaw-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    if let Some(raw_key) = provided_key {
        match crate::resolve_auth_identity_by_key(state, raw_key) {
            Ok(Some(identity)) => return Ok(identity.user_id),
            Ok(None) => {
                return Err(super::api_err::<serde_json::Value>(
                    StatusCode::UNAUTHORIZED,
                    "Invalid user_key",
                ));
            }
            Err(err) => {
                error!("Resolve task admin actor failed: {}", err);
                return Err(super::api_err::<serde_json::Value>(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Auth lookup failed",
                ));
            }
        }
    }
    if crate::is_user_allowed(state, requested_user_id) {
        Ok(requested_user_id)
    } else {
        Err(super::api_err::<serde_json::Value>(
            StatusCode::FORBIDDEN,
            "Unauthorized user",
        ))
    }
}

pub(super) async fn list_active_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ActiveTasksRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let effective_user_id = match authorize_task_admin_request(&state, &headers, req.user_id) {
        Ok(user_id) => user_id,
        Err(resp) => return resp,
    };
    match crate::list_active_tasks_internal(
        &state,
        effective_user_id,
        req.chat_id,
        req.exclude_task_id.as_deref(),
    ) {
        Ok(tasks) => super::api_ok(json!({
            "count": tasks.len(),
            "tasks": tasks,
        })),
        Err(err) => {
            error!("List active tasks failed: {}", err);
            super::api_err::<serde_json::Value>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "List active tasks failed",
            )
        }
    }
}

pub(super) async fn cancel_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CancelTasksRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let effective_user_id = match authorize_task_admin_request(&state, &headers, req.user_id) {
        Ok(user_id) => user_id,
        Err(resp) => return resp,
    };

    let result = crate::cancel_tasks_for_user_chat(
        &state,
        effective_user_id,
        req.chat_id,
        req.exclude_task_id.as_deref(),
    );

    match result {
        Ok(count) => {
            info!(
                "cancel_tasks: user_id={} chat_id={} canceled={}",
                effective_user_id, req.chat_id, count
            );
            super::api_ok(json!({ "canceled": count }))
        }
        Err(err) => {
            error!("Cancel tasks failed: {}", err);
            super::api_err::<serde_json::Value>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Cancel tasks failed",
            )
        }
    }
}

pub(super) async fn cancel_one_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CancelOneTaskRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let effective_user_id = match authorize_task_admin_request(&state, &headers, req.user_id) {
        Ok(user_id) => user_id,
        Err(resp) => return resp,
    };
    if req.index == 0 {
        return super::api_err::<serde_json::Value>(
            StatusCode::BAD_REQUEST,
            crate::i18n_t_with_default(
                &state,
                "clawd.msg.cancel_one_index_invalid",
                "index must be >= 1",
            ),
        );
    }
    let tasks = match crate::list_active_tasks_internal(
        &state,
        effective_user_id,
        req.chat_id,
        req.exclude_task_id.as_deref(),
    ) {
        Ok(tasks) => tasks,
        Err(err) => {
            error!("Cancel one task list failed: {}", err);
            return super::api_err::<serde_json::Value>(
                StatusCode::INTERNAL_SERVER_ERROR,
                crate::i18n_t_with_default(
                    &state,
                    "clawd.msg.cancel_one_failed",
                    "Cancel one task failed",
                ),
            );
        }
    };
    let Some(target) = tasks.into_iter().find(|t| t.index == req.index) else {
        return super::api_err::<serde_json::Value>(
            StatusCode::NOT_FOUND,
            crate::i18n_t_with_default_vars(
                &state,
                "clawd.msg.active_task_index_not_found",
                "Active task index {index} not found",
                &[("index", &req.index.to_string())],
            ),
        );
    };
    let result = crate::cancel_one_task_for_user_chat(
        &state,
        effective_user_id,
        req.chat_id,
        &target.task_id,
    );
    match result {
        Ok(count) if count > 0 => super::api_ok(json!({
            "canceled": count,
            "task": target,
        })),
        Ok(_) => super::api_err::<serde_json::Value>(
            StatusCode::NOT_FOUND,
            crate::i18n_t_with_default(
                &state,
                "clawd.msg.target_task_no_longer_active",
                "Target task is no longer active",
            ),
        ),
        Err(err) => {
            error!("Cancel one task failed: {}", err);
            super::api_err::<serde_json::Value>(
                StatusCode::INTERNAL_SERVER_ERROR,
                crate::i18n_t_with_default(
                    &state,
                    "clawd.msg.cancel_one_failed",
                    "Cancel one task failed",
                ),
            )
        }
    }
}
