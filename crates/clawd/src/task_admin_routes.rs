use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use claw_core::types::{ApiResponse, AuthIdentity};
use serde::Deserialize;
use serde_json::{json, Value};
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
pub(super) struct AutomationRunsRequest {
    user_id: i64,
    chat_id: i64,
    job_id: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CancelOneTaskRequest {
    user_id: i64,
    chat_id: i64,
    index: usize,
    exclude_task_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CancelTaskByIdRequest {
    task_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ResumeTaskByIdRequest {
    task_id: String,
    checkpoint_id: Option<String>,
    resume_reason: Option<String>,
    user_message: Option<String>,
    new_constraints: Option<Value>,
    approval_request_id: Option<String>,
    approve: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct PauseTaskByIdRequest {
    task_id: String,
    pause_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GoalByTaskIdRequest {
    task_id: String,
    operation: String,
    goal: Option<Value>,
}

fn task_admin_sensitive_field_name(field: &str) -> bool {
    let normalized = field.trim().to_ascii_lowercase().replace(['-', '.'], "_");
    normalized == "key"
        || normalized == "auth"
        || normalized.ends_with("_key")
        || normalized.contains("token")
        || normalized.contains("secret")
        || normalized.contains("password")
        || normalized.contains("passwd")
        || normalized.contains("cookie")
        || normalized.contains("credential")
        || normalized.contains("ticket")
        || normalized.contains("signature")
        || normalized.contains("authorization")
}

fn task_admin_public_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, child) in map {
                let value = if task_admin_sensitive_field_name(key) {
                    json!("[REDACTED]")
                } else {
                    task_admin_public_json(child)
                };
                out.insert(key.clone(), value);
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(task_admin_public_json).collect()),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.clone(),
    }
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

fn require_task_admin_identity(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(AuthIdentity, String), (StatusCode, Json<ApiResponse<serde_json::Value>>)> {
    let Some(raw_key) = headers
        .get("x-rustclaw-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return Err(super::api_err::<serde_json::Value>(
            StatusCode::UNAUTHORIZED,
            "task_admin_key_required",
        ));
    };
    let normalized_key = crate::normalize_user_key(raw_key);
    match crate::resolve_auth_identity_by_key(state, &normalized_key) {
        Ok(Some(identity)) => Ok((identity, normalized_key)),
        Ok(None) => Err(super::api_err::<serde_json::Value>(
            StatusCode::UNAUTHORIZED,
            "invalid_user_key",
        )),
        Err(err) => {
            error!("Resolve task admin identity failed: {}", err);
            Err(super::api_err::<serde_json::Value>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "auth_lookup_failed",
            ))
        }
    }
}

fn task_admin_target_matches_identity(
    target: &crate::TaskAdminTarget,
    identity: &AuthIdentity,
    normalized_key: &str,
) -> bool {
    if target.user_id == identity.user_id {
        return true;
    }
    target
        .user_key
        .as_deref()
        .map(crate::normalize_user_key)
        .is_some_and(|task_key| task_key == normalized_key)
}

fn authorized_task_admin_target_by_id(
    state: &AppState,
    headers: &HeaderMap,
    task_id: &str,
) -> Result<crate::TaskAdminTarget, (StatusCode, Json<ApiResponse<serde_json::Value>>)> {
    let (identity, normalized_key) = require_task_admin_identity(state, headers)?;
    let task_id = task_id.trim();
    if task_id.is_empty() {
        return Err(super::api_err::<serde_json::Value>(
            StatusCode::BAD_REQUEST,
            "task_id_required",
        ));
    }
    let target = match crate::get_task_admin_target(state, task_id) {
        Ok(Some(target)) => target,
        Ok(None) => {
            return Err(super::api_err::<serde_json::Value>(
                StatusCode::NOT_FOUND,
                "task_not_found",
            ));
        }
        Err(err) => {
            error!("task_admin_target_lookup_failed err={}", err);
            return Err(super::api_err::<serde_json::Value>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task_lookup_failed",
            ));
        }
    };
    if !task_admin_target_matches_identity(&target, &identity, &normalized_key) {
        return Err(super::api_err::<serde_json::Value>(
            StatusCode::FORBIDDEN,
            "task_owner_mismatch",
        ));
    }
    Ok(target)
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

pub(super) async fn list_automation_runs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<AutomationRunsRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let effective_user_id = match authorize_task_admin_request(&state, &headers, req.user_id) {
        Ok(user_id) => user_id,
        Err(resp) => return resp,
    };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            error!("automation_runs_db_pool_failed err={}", err);
            return super::api_err::<serde_json::Value>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "automation_runs_db_pool_failed",
            );
        }
    };
    match crate::scheduled_run_contract::list_scheduled_run_history(
        &db,
        effective_user_id,
        req.chat_id,
        req.job_id.as_deref(),
        req.limit.unwrap_or(20),
    ) {
        Ok(runs) => super::api_ok(json!({
            "count": runs.len(),
            "runs": runs,
        })),
        Err(err) => {
            error!("automation_runs_query_failed err={}", err);
            super::api_err::<serde_json::Value>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "automation_runs_query_failed",
            )
        }
    }
}

pub(super) async fn cancel_task_by_id(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CancelTaskByIdRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let target = match authorized_task_admin_target_by_id(&state, &headers, &req.task_id) {
        Ok(target) => target,
        Err(resp) => return resp,
    };
    if !matches!(target.status.as_str(), "queued" | "running") {
        return super::api_err::<serde_json::Value>(StatusCode::CONFLICT, "task_not_active");
    }
    match crate::cancel_task_by_id(&state, &target.task_id) {
        Ok(count) if count > 0 => super::api_ok(json!({
            "status": "task_cancelled",
            "canceled": count,
            "task_id": target.task_id,
            "user_id": target.user_id,
            "chat_id": target.chat_id,
            "channel": target.channel,
        })),
        Ok(_) => super::api_err::<serde_json::Value>(StatusCode::CONFLICT, "task_not_active"),
        Err(err) => {
            error!("Cancel task by id failed: {}", err);
            super::api_err::<serde_json::Value>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task_cancel_failed",
            )
        }
    }
}

pub(super) async fn resume_task_by_id(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ResumeTaskByIdRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let target = match authorized_task_admin_target_by_id(&state, &headers, &req.task_id) {
        Ok(target) => target,
        Err(resp) => return resp,
    };
    if target.status == "failed" {
        let request_id = req
            .approval_request_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if req.approve != Some(true) || request_id.is_none() {
            return super::api_err::<serde_json::Value>(
                StatusCode::CONFLICT,
                "approval_grant_explicit_decision_required",
            );
        }
        return match crate::repo::approve_task_approval_request(
            &state,
            &target.task_id,
            request_id.unwrap_or_default(),
        ) {
            Ok(Some(update)) => super::api_ok(json!({
                "status": "approval_grant_approved",
                "task_id": update.task_id,
                "approval_request_id": update.request_id,
                "expires_at": update.expires_at,
                "task_lifecycle": {
                    "state": "queued",
                    "reason_code": "approval_grant_approved",
                },
            })),
            Ok(None) => super::api_err::<serde_json::Value>(
                StatusCode::CONFLICT,
                "approval_grant_not_approvable",
            ),
            Err(err) => {
                error!("task_approval_grant_failed err={}", err);
                super::api_err::<serde_json::Value>(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "approval_grant_update_failed",
                )
            }
        };
    }
    if target.status.as_str() != "running" {
        return super::api_err::<serde_json::Value>(StatusCode::CONFLICT, "task_not_resumable");
    }
    let resume_input = crate::repo::TaskResumeControlInput {
        task_id: target.task_id.clone(),
        checkpoint_id: req.checkpoint_id,
        resume_reason: req.resume_reason,
        user_message: req.user_message,
        new_constraints: req.new_constraints,
    };
    match crate::repo::resume_task_with_input(&state, resume_input) {
        Ok(Some(update)) => super::api_ok(json!({
            "status": "task_resume_requested",
            "task_id": update.task_id,
            "checkpoint_id": update.checkpoint_id,
            "task_lifecycle": update.lifecycle,
        })),
        Ok(None) => super::api_err::<serde_json::Value>(StatusCode::CONFLICT, "task_not_resumable"),
        Err(err) => {
            error!("task_resume_by_id_failed err={}", err);
            super::api_err::<serde_json::Value>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task_resume_failed",
            )
        }
    }
}

pub(super) async fn pause_task_by_id(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PauseTaskByIdRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let target = match authorized_task_admin_target_by_id(&state, &headers, &req.task_id) {
        Ok(target) => target,
        Err(resp) => return resp,
    };
    if target.status.as_str() != "running" {
        return super::api_err::<serde_json::Value>(StatusCode::CONFLICT, "task_not_pauseable");
    }
    match crate::repo::pause_task_by_id(&state, &target.task_id, req.pause_seconds.unwrap_or(3600))
    {
        Ok(Some(update)) => super::api_ok(json!({
            "status": "task_pause_requested",
            "task_id": update.task_id,
            "checkpoint_id": update.checkpoint_id,
            "task_lifecycle": update.lifecycle,
        })),
        Ok(None) => super::api_err::<serde_json::Value>(StatusCode::CONFLICT, "task_not_pauseable"),
        Err(err) => {
            error!("task_pause_by_id_failed err={}", err);
            super::api_err::<serde_json::Value>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task_pause_failed",
            )
        }
    }
}

pub(super) async fn goal_by_task_id(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<GoalByTaskIdRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let target = match authorized_task_admin_target_by_id(&state, &headers, &req.task_id) {
        Ok(target) => target,
        Err(resp) => return resp,
    };
    let Some(operation) = crate::repo::TaskGoalControlOperation::parse(&req.operation) else {
        return super::api_err::<serde_json::Value>(
            StatusCode::BAD_REQUEST,
            "task_goal_operation_invalid",
        );
    };
    match crate::repo::update_task_goal_payload(&state, &target.task_id, operation, req.goal) {
        Ok(Some(update)) => {
            let goal = update
                .goal
                .as_ref()
                .map(task_admin_public_json)
                .unwrap_or(Value::Null);
            super::api_ok(json!({
                "status": "task_goal_control_updated",
                "task_id": update.task_id,
                "operation": update.operation,
                "goal": goal,
                "payload_json": task_admin_public_json(&update.payload_json),
            }))
        }
        Ok(None) => super::api_err::<serde_json::Value>(StatusCode::NOT_FOUND, "task_not_found"),
        Err(err) => {
            error!("task_goal_control_failed err={}", err);
            super::api_err::<serde_json::Value>(StatusCode::BAD_REQUEST, "task_goal_control_failed")
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

#[cfg(test)]
#[path = "task_admin_routes_tests.rs"]
mod tests;
