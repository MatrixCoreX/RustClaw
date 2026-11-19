use crate::{api_err, api_ok, require_auth_identity_for_api, ApiResponse, AppState};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde_json::Value;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/memory/jobs", get(list))
        .route("/memory/jobs/retry-unavailable", post(retry))
        .route("/memory/jobs/review", get(review))
        .route("/memory/jobs/:job_id/disposition", post(disposition))
}

#[derive(serde::Deserialize)]
struct ReviewRequest {
    action: String,
    review_token: String,
}

async fn review(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_auth_identity_for_api::<Value>(&state, &headers) {
        Ok(v) => v,
        Err(response) => return response,
    };
    match tokio::task::spawn_blocking(move || {
        let db = state.core.db.get()?;
        crate::memory::jobs::review::list(&db, &identity.principal_id)
    })
    .await
    {
        Ok(Ok(value)) => api_ok(value),
        _ => api_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "memory_job_status_failed",
        ),
    }
}

async fn disposition(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<ReviewRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_auth_identity_for_api::<Value>(&state, &headers) {
        Ok(v) => v,
        Err(response) => return response,
    };
    if id.len() > 128
        || request.review_token.len() != 64
        || !matches!(request.action.as_str(), "archive" | "unarchive" | "retry")
    {
        return api_err(StatusCode::BAD_REQUEST, "memory_job_review_invalid_request");
    }
    if !crate::runtime::storage_pressure::admit_write(&state)
        .await
        .is_ok_and(|v| v.write_admission)
    {
        return api_err(StatusCode::SERVICE_UNAVAILABLE, "storage_pressure");
    }
    match tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
        let result = {
            let mut db = state.core.db.get()?;
            crate::memory::jobs::review::disposition(
                &mut db,
                &identity.principal_id,
                &id,
                &request.review_token,
                &request.action,
            )?
        };
        if request.action == "retry" {
            crate::memory::jobs::recovery::retry_selected(&state, &identity.principal_id, Some(&id))
        } else {
            Ok(result)
        }
    })
    .await
    {
        Ok(Ok(value)) => api_ok(value),
        _ => api_err(StatusCode::CONFLICT, "memory_job_review_conflict"),
    }
}

async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_auth_identity_for_api::<Value>(&state, &headers) {
        Ok(v) => v,
        Err(response) => return response,
    };
    match crate::memory::jobs::recovery::list(&state, &identity.principal_id) {
        Ok(jobs) => api_ok(serde_json::json!({"schema_version":1,"jobs":jobs})),
        Err(_) => api_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "memory_job_status_failed",
        ),
    }
}

async fn retry(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_auth_identity_for_api::<Value>(&state, &headers) {
        Ok(v) => v,
        Err(response) => return response,
    };
    if !crate::runtime::storage_pressure::admit_write(&state)
        .await
        .is_ok_and(|v| v.write_admission)
    {
        return api_err(StatusCode::SERVICE_UNAVAILABLE, "storage_pressure");
    }
    let worker_state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::memory::jobs::recovery::retry_unavailable(&worker_state, &identity.principal_id)
    })
    .await;
    match result {
        Ok(Ok(result)) => api_ok(result),
        _ => api_err(StatusCode::INTERNAL_SERVER_ERROR, "memory_job_retry_failed"),
    }
}

#[cfg(test)]
#[path = "memory_job_review_routes_tests.rs"]
mod tests;
