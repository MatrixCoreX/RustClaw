use crate::{api_err, api_ok, require_auth_identity_for_api, ApiResponse, AppState};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde_json::Value;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/memory/jobs", get(list))
        .route("/memory/jobs/retry-unavailable", post(retry))
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
