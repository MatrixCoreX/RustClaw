use anyhow::Context;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use tracing::{error, warn};

use crate::memory::ux::{
    MemoryBulkClearRequest, MemoryClearPreview, MemoryCorrectionRequest, MemoryExport,
    MemoryFeedbackRequest, MemoryImportConfirmRequest, MemoryImportPreview,
    MemoryImportPreviewRequest, MemoryImportResult, MemoryListFilter, MemoryMarkdownExport,
    MemoryMutationRequest, MemoryMutationResult, MemoryPageResult, MemoryUndoRequest,
    RemoteMemoryDisclosure,
};
use crate::{
    api_err, api_ok, insert_audit_log, memory, now_ts_u64, require_auth_identity_for_api,
    ApiResponse, AppState,
};

#[derive(Debug, serde::Serialize)]
struct MemoryVectorStatus {
    schema_version: u32,
    provider_location: String,
    state: String,
    active_generation: u64,
    queued_jobs: i64,
    running_jobs: i64,
    failed_jobs: i64,
    indexed_rows: i64,
    remote_consent: String,
}

#[derive(Debug, serde::Deserialize)]
struct MemoryVectorMutationRequest {
    expected_policy_digest: String,
}

#[derive(Debug, serde::Serialize)]
struct MemoryVectorMutationResult {
    schema_version: u32,
    status: String,
    queued_rows: usize,
    generation: u64,
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/memory", get(get_memory_overview))
        .route("/memory/recent", get(list_memory_recent_handler))
        .route("/memory/preferences", get(list_memory_preferences_handler))
        .route("/memory/facts", get(list_memory_facts_handler))
        .route("/memory/:memory_id", delete(delete_memory_handler))
        .route("/memory/:memory_id/expire", post(expire_memory_handler))
        .route("/memory/clear", post(clear_memory_handler))
        .route(
            "/memory/settings",
            get(get_memory_settings_handler).post(update_memory_settings_handler),
        )
        .route(
            "/memory/projects/current",
            get(get_current_memory_project_handler),
        )
        .route(
            "/memory/projects/current/link",
            post(link_current_memory_project_handler),
        )
        .route(
            "/memory/projects/current/unlink",
            post(unlink_current_memory_project_handler),
        )
        .route("/memory/items", get(list_items))
        .route("/memory/:memory_id/correct", post(correct_item))
        .route("/memory/:memory_id/feedback", post(record_feedback))
        .route("/memory/:memory_id/delete", post(delete_item))
        .route("/memory/undo", post(undo_mutation))
        .route("/memory/export", get(export_items))
        .route("/memory/export/markdown", get(export_markdown))
        .route("/memory/import/preview", post(import_preview))
        .route("/memory/import/confirm", post(import_confirm))
        .route("/memory/remote-disclosure", get(remote_disclosure))
        .route("/memory/clear/preview", get(clear_preview))
        .route("/memory/clear/scoped", post(clear_scoped))
        .route("/memory/vector/status", get(vector_status))
        .route("/memory/vector/reindex", post(vector_reindex))
        .route("/memory/vector/pause", post(vector_pause))
        .route("/memory/vector/resume", post(vector_resume))
        .route("/memory/vector/cancel", post(vector_cancel))
}

async fn vector_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<MemoryVectorStatus>>) {
    let identity =
        match crate::require_auth_identity_for_api::<MemoryVectorStatus>(&state, &headers) {
            Ok(identity) => identity,
            Err(response) => return response,
        };
    match load_vector_status(&state, &identity.principal_id) {
        Ok(status) => crate::api_ok(status),
        Err(error) => {
            tracing::warn!(error = %error, "memory vector status failed");
            crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_vector_status_failed",
            )
        }
    }
}

async fn vector_reindex(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MemoryVectorMutationRequest>,
) -> (StatusCode, Json<ApiResponse<MemoryVectorMutationResult>>) {
    let identity = match crate::require_auth_identity_for_api::<MemoryVectorMutationResult>(
        &state, &headers,
    ) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(_) => {
            return crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_database_unavailable",
            )
        }
    };
    let settings = match crate::memory::settings::resolve_principal_memory_settings(
        &db,
        &identity.principal_id,
        state.policy.memory.long_term_enabled,
    ) {
        Ok(settings) => settings,
        Err(_) => {
            return crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_settings_unavailable",
            )
        }
    };
    if settings.policy_digest != request.expected_policy_digest {
        return crate::api_err(StatusCode::CONFLICT, "memory_settings_revision_conflict");
    }
    let remote_allowed =
        settings.external_context_policy == crate::memory::settings::ExternalContextPolicy::Allow;
    match crate::memory::embedding_jobs::enqueue_reindex(
        &db,
        &state.policy.memory,
        &identity.principal_id,
        &settings.policy_digest,
        remote_allowed,
    ) {
        Ok((_snapshot_id, generation, queued_rows)) => crate::api_ok(MemoryVectorMutationResult {
            schema_version: 1,
            status: "queued".to_string(),
            queued_rows,
            generation,
        }),
        Err(error) if error.to_string() == "memory_embedding_remote_consent_required" => {
            crate::api_err(
                StatusCode::FORBIDDEN,
                "memory_embedding_remote_consent_required",
            )
        }
        Err(error) if error.to_string() == "memory_embedding_reindex_already_running" => {
            crate::api_err(
                StatusCode::CONFLICT,
                "memory_embedding_reindex_already_running",
            )
        }
        Err(error) => {
            tracing::warn!(error = %error, "memory vector reindex failed");
            crate::api_err(StatusCode::BAD_REQUEST, "memory_vector_reindex_failed")
        }
    }
}

async fn vector_pause(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MemoryVectorMutationRequest>,
) -> (StatusCode, Json<ApiResponse<MemoryVectorMutationResult>>) {
    vector_control(state, headers, request, "paused")
}

async fn vector_resume(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MemoryVectorMutationRequest>,
) -> (StatusCode, Json<ApiResponse<MemoryVectorMutationResult>>) {
    vector_control(state, headers, request, "active")
}

async fn vector_cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MemoryVectorMutationRequest>,
) -> (StatusCode, Json<ApiResponse<MemoryVectorMutationResult>>) {
    vector_control(state, headers, request, "cancelled")
}

fn vector_control(
    state: AppState,
    headers: HeaderMap,
    request: MemoryVectorMutationRequest,
    action: &str,
) -> (StatusCode, Json<ApiResponse<MemoryVectorMutationResult>>) {
    let identity = match crate::require_auth_identity_for_api::<MemoryVectorMutationResult>(
        &state, &headers,
    ) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(_) => {
            return crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_database_unavailable",
            )
        }
    };
    let settings = match crate::memory::settings::resolve_principal_memory_settings(
        &db,
        &identity.principal_id,
        state.policy.memory.long_term_enabled,
    ) {
        Ok(settings) => settings,
        Err(_) => {
            return crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_settings_unavailable",
            )
        }
    };
    if settings.policy_digest != request.expected_policy_digest {
        return crate::api_err(StatusCode::CONFLICT, "memory_settings_revision_conflict");
    }
    let profile =
        match crate::memory::vector_store::register_configured_profile(&db, &state.policy.memory) {
            Ok(profile) => profile,
            Err(_) => {
                return crate::api_err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "memory_vector_profile_unavailable",
                )
            }
        };
    let result = match action {
        "paused" => crate::memory::embedding_jobs::set_profile_paused(
            &db,
            &identity.principal_id,
            &profile.profile_id,
            true,
        )
        .map(|_| 0),
        "active" => crate::memory::embedding_jobs::set_profile_paused(
            &db,
            &identity.principal_id,
            &profile.profile_id,
            false,
        )
        .map(|_| 0),
        "cancelled" => crate::memory::embedding_jobs::cancel_profile_jobs(
            &db,
            &identity.principal_id,
            &profile.profile_id,
        ),
        _ => unreachable!(),
    };
    match result {
        Ok(changed) => crate::api_ok(MemoryVectorMutationResult {
            schema_version: 1,
            status: action.to_string(),
            queued_rows: changed,
            generation: crate::memory::vector_store::active_generation_for_principal(
                &db,
                &identity.principal_id,
                &profile.profile_id,
                profile.generation,
            )
            .unwrap_or(profile.generation),
        }),
        Err(error) => {
            tracing::warn!(error = %error, action, "memory vector control failed");
            crate::api_err(StatusCode::BAD_REQUEST, "memory_vector_control_failed")
        }
    }
}

fn load_vector_status(state: &AppState, principal_id: &str) -> anyhow::Result<MemoryVectorStatus> {
    crate::sqlite_busy_retry::with_sqlite_busy_retry(
        crate::sqlite_busy_retry::SqliteBusyRetryPolicy::default(),
        || load_vector_status_once(state, principal_id),
    )
}

fn load_vector_status_once(
    state: &AppState,
    principal_id: &str,
) -> anyhow::Result<MemoryVectorStatus> {
    let db = state
        .core
        .db
        .get()
        .context("memory_vector_status_db_pool")?;
    let configured = crate::memory::vector_store::configured_profile(&state.policy.memory)
        .context("memory_vector_status_configured_profile")?;
    let profile = crate::memory::vector_store::load_profile(&db, &configured.profile_id)
        .context("memory_vector_status_load_profile")?
        .ok_or_else(|| anyhow::anyhow!("memory_embedding_profile_not_initialized"))?;
    let generation = crate::memory::vector_store::active_generation_for_principal(
        &db,
        principal_id,
        &profile.profile_id,
        profile.generation,
    )
    .context("memory_vector_status_generation")?;
    let count = |status: &str| -> anyhow::Result<i64> {
        Ok(db
            .query_row(
                "SELECT COUNT(*) FROM memory_embedding_jobs
             WHERE principal_id = ?1 AND profile_id = ?2 AND status = ?3",
                rusqlite::params![principal_id, profile.profile_id, status],
                |row| row.get(0),
            )
            .with_context(|| format!("memory_vector_status_job_count:{status}"))?)
    };
    let indexed_rows = db
        .query_row(
            "SELECT COUNT(*) FROM memory_vector_rows
         WHERE principal_id = ?1 AND profile_id = ?2 AND generation = ?3
           AND status = 'active'",
            rusqlite::params![principal_id, profile.profile_id, generation as i64],
            |row| row.get(0),
        )
        .context("memory_vector_status_indexed_rows")?;
    let settings = crate::memory::settings::resolve_principal_memory_settings(
        &db,
        principal_id,
        state.policy.memory.long_term_enabled,
    )
    .context("memory_vector_status_settings")?;
    Ok(MemoryVectorStatus {
        schema_version: 1,
        provider_location: if profile.provider_kind == "remote_http" {
            "remote".to_string()
        } else {
            "local".to_string()
        },
        state: if crate::memory::embedding_jobs::profile_paused(
            &db,
            principal_id,
            &profile.profile_id,
        )
        .context("memory_vector_status_profile_paused")?
        {
            "paused".to_string()
        } else if count("running")? > 0 || count("queued")? > 0 || count("retry_wait")? > 0 {
            "building".to_string()
        } else {
            "ready".to_string()
        },
        active_generation: generation,
        queued_jobs: count("queued")? + count("retry_wait")?,
        running_jobs: count("running")?,
        failed_jobs: count("failed")?,
        indexed_rows,
        remote_consent: settings.external_context_policy.as_str().to_string(),
    })
}

#[cfg(test)]
#[path = "memory_routes_tests.rs"]
mod tests;

async fn undo_mutation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MemoryUndoRequest>,
) -> (StatusCode, Json<ApiResponse<MemoryMutationResult>>) {
    let identity =
        match crate::require_auth_identity_for_api::<MemoryMutationResult>(&state, &headers) {
            Ok(identity) => identity,
            Err(response) => return response,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(_) => {
            return crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_database_unavailable",
            )
        }
    };
    match crate::memory::ux::undo_memory_mutation(
        &db,
        &identity.principal_id,
        &request,
        crate::now_ts_u64() as i64,
    ) {
        Ok(result) => crate::api_ok(result),
        Err(error) => mutation_error(error),
    }
}

async fn remote_disclosure(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<RemoteMemoryDisclosure>>) {
    let identity =
        match crate::require_auth_identity_for_api::<RemoteMemoryDisclosure>(&state, &headers) {
            Ok(identity) => identity,
            Err(response) => return response,
        };
    match crate::memory::ux::remote_memory_disclosure(&state, &identity.principal_id) {
        Ok(result) => crate::api_ok(result),
        Err(error) => {
            tracing::warn!(error = %error, "memory remote disclosure failed");
            crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_remote_disclosure_failed",
            )
        }
    }
}

async fn list_items(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filter): Query<MemoryListFilter>,
) -> (StatusCode, Json<ApiResponse<MemoryPageResult>>) {
    let identity = match crate::require_auth_identity_for_api::<MemoryPageResult>(&state, &headers)
    {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(error) => {
            tracing::error!(error = %error, "memory page database unavailable");
            return crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_database_unavailable",
            );
        }
    };
    match crate::memory::ux::list_memory_page(
        &db,
        &identity.principal_id,
        &filter,
        crate::now_ts_u64() as i64,
    ) {
        Ok(result) => crate::api_ok(result),
        Err(error) => {
            tracing::warn!(error = %error, "memory page query failed");
            crate::api_err(StatusCode::BAD_REQUEST, "memory_page_query_failed")
        }
    }
}

async fn correct_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(memory_id): AxumPath<String>,
    Json(request): Json<MemoryCorrectionRequest>,
) -> (StatusCode, Json<ApiResponse<MemoryMutationResult>>) {
    let identity =
        match crate::require_auth_identity_for_api::<MemoryMutationResult>(&state, &headers) {
            Ok(identity) => identity,
            Err(response) => return response,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(_) => {
            return crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_database_unavailable",
            )
        }
    };
    match crate::memory::ux::correct_memory(
        &db,
        &identity.principal_id,
        &identity.principal_id,
        &memory_id,
        &request,
        crate::now_ts_u64() as i64,
    ) {
        Ok(result) => crate::api_ok(result),
        Err(error) => mutation_error(error),
    }
}

async fn record_feedback(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(memory_id): AxumPath<String>,
    Json(request): Json<MemoryFeedbackRequest>,
) -> (StatusCode, Json<ApiResponse<MemoryMutationResult>>) {
    let identity =
        match crate::require_auth_identity_for_api::<MemoryMutationResult>(&state, &headers) {
            Ok(identity) => identity,
            Err(response) => return response,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(_) => {
            return crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_database_unavailable",
            )
        }
    };
    match crate::memory::ux::record_feedback(
        &db,
        &identity.principal_id,
        &memory_id,
        &request,
        crate::now_ts_u64() as i64,
    ) {
        Ok(result) => crate::api_ok(result),
        Err(error) => mutation_error(error),
    }
}

async fn delete_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(memory_id): AxumPath<String>,
    Json(request): Json<MemoryMutationRequest>,
) -> (StatusCode, Json<ApiResponse<MemoryMutationResult>>) {
    let identity =
        match crate::require_auth_identity_for_api::<MemoryMutationResult>(&state, &headers) {
            Ok(identity) => identity,
            Err(response) => return response,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(_) => {
            return crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_database_unavailable",
            )
        }
    };
    match crate::memory::ux::delete_memory_with_revision(
        &db,
        &identity.principal_id,
        &memory_id,
        &request,
        crate::now_ts_u64() as i64,
    ) {
        Ok(result) => crate::api_ok(result),
        Err(error) => mutation_error(error),
    }
}

async fn export_items(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<MemoryExport>>) {
    let identity = match crate::require_auth_identity_for_api::<MemoryExport>(&state, &headers) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(_) => {
            return crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_database_unavailable",
            )
        }
    };
    match crate::memory::ux::export_memory(&db, &identity.principal_id, crate::now_ts_u64() as i64)
    {
        Ok(result) => crate::api_ok(result),
        Err(error) => {
            tracing::warn!(error = %error, "memory export failed");
            crate::api_err(StatusCode::INTERNAL_SERVER_ERROR, "memory_export_failed")
        }
    }
}

async fn export_markdown(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<MemoryMarkdownExport>>) {
    let identity =
        match crate::require_auth_identity_for_api::<MemoryMarkdownExport>(&state, &headers) {
            Ok(identity) => identity,
            Err(response) => return response,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(_) => {
            return crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_database_unavailable",
            )
        }
    };
    match crate::memory::ux::export_memory_markdown(
        &db,
        &identity.principal_id,
        crate::now_ts_u64() as i64,
    ) {
        Ok(result) => crate::api_ok(result),
        Err(error) => {
            tracing::warn!(error = %error, "memory markdown export failed");
            crate::api_err(StatusCode::INTERNAL_SERVER_ERROR, "memory_export_failed")
        }
    }
}

async fn import_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MemoryImportPreviewRequest>,
) -> (StatusCode, Json<ApiResponse<MemoryImportPreview>>) {
    let identity =
        match crate::require_auth_identity_for_api::<MemoryImportPreview>(&state, &headers) {
            Ok(identity) => identity,
            Err(response) => return response,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(_) => {
            return crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_database_unavailable",
            )
        }
    };
    match crate::memory::ux::preview_memory_import(
        &db,
        &identity.principal_id,
        &request,
        crate::now_ts_u64() as i64,
    ) {
        Ok(result) => crate::api_ok(result),
        Err(error) => mutation_error_typed(error),
    }
}

async fn import_confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MemoryImportConfirmRequest>,
) -> (StatusCode, Json<ApiResponse<MemoryImportResult>>) {
    let identity =
        match crate::require_auth_identity_for_api::<MemoryImportResult>(&state, &headers) {
            Ok(identity) => identity,
            Err(response) => return response,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(_) => {
            return crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_database_unavailable",
            )
        }
    };
    match crate::memory::ux::confirm_memory_import(
        &db,
        &identity.principal_id,
        &request,
        crate::now_ts_u64() as i64,
    ) {
        Ok(result) => crate::api_ok(result),
        Err(error) => mutation_error_typed(error),
    }
}

#[derive(Debug, serde::Deserialize)]
struct ClearPreviewQuery {
    mode: Option<String>,
}

async fn clear_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ClearPreviewQuery>,
) -> (StatusCode, Json<ApiResponse<MemoryClearPreview>>) {
    let identity =
        match crate::require_auth_identity_for_api::<MemoryClearPreview>(&state, &headers) {
            Ok(identity) => identity,
            Err(response) => return response,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(_) => {
            return crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_database_unavailable",
            )
        }
    };
    let mode = query.mode.as_deref().unwrap_or("transcript");
    match crate::memory::ux::clear_preview(&db, &identity.principal_id, mode) {
        Ok(result) => crate::api_ok(result),
        Err(error) => crate::api_err(
            StatusCode::BAD_REQUEST,
            &memory_api_error_code(&error, "memory_clear_preview_failed"),
        ),
    }
}

async fn clear_scoped(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MemoryBulkClearRequest>,
) -> (StatusCode, Json<ApiResponse<MemoryClearPreview>>) {
    let identity =
        match crate::require_auth_identity_for_api::<MemoryClearPreview>(&state, &headers) {
            Ok(identity) => identity,
            Err(response) => return response,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(_) => {
            return crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_database_unavailable",
            )
        }
    };
    match crate::memory::ux::clear_with_mode(
        &db,
        &identity.principal_id,
        &request,
        crate::now_ts_u64() as i64,
    ) {
        Ok(result) => crate::api_ok(result),
        Err(error) if error.to_string() == "memory_clear_preview_conflict" => {
            crate::api_err(StatusCode::CONFLICT, "memory_clear_preview_conflict")
        }
        Err(error) => crate::api_err(
            StatusCode::BAD_REQUEST,
            &memory_api_error_code(&error, "memory_clear_failed"),
        ),
    }
}

fn mutation_error(error: anyhow::Error) -> (StatusCode, Json<ApiResponse<MemoryMutationResult>>) {
    let code = memory_api_error_code(&error, "memory_mutation_failed");
    let status = match code.as_str() {
        "memory_revision_conflict" => StatusCode::CONFLICT,
        "memory_not_found" => StatusCode::NOT_FOUND,
        "memory_database_unavailable" => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::BAD_REQUEST,
    };
    crate::api_err(status, &code)
}

fn mutation_error_typed<T: serde::Serialize>(
    error: anyhow::Error,
) -> (StatusCode, Json<ApiResponse<T>>) {
    let code = memory_api_error_code(&error, "memory_mutation_failed");
    let status = if code.contains("conflict") || code.contains("already_confirmed") {
        StatusCode::CONFLICT
    } else {
        StatusCode::BAD_REQUEST
    };
    crate::api_err(status, &code)
}

fn memory_api_error_code(error: &anyhow::Error, fallback: &'static str) -> String {
    let code = error.to_string();
    let valid_machine_code = code.starts_with("memory_")
        && code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
    if valid_machine_code {
        code
    } else {
        fallback.to_string()
    }
}

async fn get_memory_overview(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<memory::api::MemoryOverview>>) {
    let identity =
        match require_auth_identity_for_api::<memory::api::MemoryOverview>(&state, &headers) {
            Ok(identity) => identity,
            Err(resp) => return resp,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            error!("get memory overview db failed: {}", err);
            return api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_database_unavailable",
            );
        }
    };
    let settings = match memory::settings::resolve_principal_memory_settings(
        &db,
        &identity.principal_id,
        state.policy.memory.long_term_enabled,
    ) {
        Ok(settings) => settings,
        Err(err) => {
            error!("resolve memory overview settings failed: {}", err);
            return api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_settings_resolve_failed",
            );
        }
    };
    let memory_scope = memory::scope::resolve_principal_scope(&identity);
    match memory::api::memory_overview(
        &db,
        identity.chat_id,
        &memory_scope.scope_ref,
        settings.use_memory && settings.generate_memory,
        state.policy.memory.hybrid_recall_enabled,
    ) {
        Ok(overview) => api_ok(overview),
        Err(err) => {
            error!("get memory overview failed: {}", err);
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "memory_lookup_failed")
        }
    }
}

async fn get_memory_settings_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<MemorySettingsQuery>,
) -> (
    StatusCode,
    Json<ApiResponse<memory::settings::MemoryEffectiveSettings>>,
) {
    let identity = match require_auth_identity_for_api::<memory::settings::MemoryEffectiveSettings>(
        &state, &headers,
    ) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            error!("get memory settings db failed: {}", err);
            return api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_settings_db_failed",
            );
        }
    };
    let target_principal_id = query
        .target_principal_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&identity.principal_id);
    if target_principal_id != identity.principal_id && identity.role != "admin" {
        return api_err(StatusCode::FORBIDDEN, "memory_settings_admin_required");
    }
    let resolved = match query.scope.unwrap_or_default() {
        memory::settings::MemorySettingScope::Admin => {
            if identity.role != "admin" {
                return api_err(StatusCode::FORBIDDEN, "memory_settings_admin_required");
            }
            memory::settings::resolve_principal_memory_settings(
                &db,
                target_principal_id,
                state.policy.memory.long_term_enabled,
            )
        }
        memory::settings::MemorySettingScope::Principal => {
            memory::settings::resolve_principal_memory_settings(
                &db,
                target_principal_id,
                state.policy.memory.long_term_enabled,
            )
        }
        memory::settings::MemorySettingScope::Conversation => {
            let Some(conversation_id) = query
                .conversation_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return api_err(
                    StatusCode::BAD_REQUEST,
                    "memory_settings_conversation_required",
                );
            };
            memory::settings::resolve_memory_settings(
                &db,
                target_principal_id,
                Some(conversation_id),
                state.policy.memory.long_term_enabled,
            )
        }
    };
    match resolved {
        Ok(result) => api_ok(result),
        Err(err) => {
            error!("get memory settings failed: {}", err);
            api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_settings_resolve_failed",
            )
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct MemorySettingsQuery {
    scope: Option<memory::settings::MemorySettingScope>,
    conversation_id: Option<String>,
    target_principal_id: Option<String>,
}

async fn list_memory_preferences_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (
    StatusCode,
    Json<ApiResponse<Vec<memory::api::MemoryPreferenceItem>>>,
) {
    let identity = match require_auth_identity_for_api::<Vec<memory::api::MemoryPreferenceItem>>(
        &state, &headers,
    ) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            error!("list memory preferences db failed: {}", err);
            return api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_database_unavailable",
            );
        }
    };
    match memory::api::list_preferences(&db, identity.chat_id, &identity.principal_id) {
        Ok(items) => api_ok(items),
        Err(err) => {
            error!("list memory preferences failed: {}", err);
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "memory_lookup_failed")
        }
    }
}

async fn list_memory_facts_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (
    StatusCode,
    Json<ApiResponse<Vec<memory::api::MemoryFactItem>>>,
) {
    let identity =
        match require_auth_identity_for_api::<Vec<memory::api::MemoryFactItem>>(&state, &headers) {
            Ok(identity) => identity,
            Err(resp) => return resp,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            error!("list memory facts db failed: {}", err);
            return api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_database_unavailable",
            );
        }
    };
    match memory::api::list_facts(&db, &identity.principal_id) {
        Ok(items) => api_ok(items),
        Err(err) => {
            error!("list memory facts failed: {}", err);
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "memory_lookup_failed")
        }
    }
}

async fn list_memory_recent_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (
    StatusCode,
    Json<ApiResponse<Vec<memory::api::MemoryRecentItem>>>,
) {
    let identity =
        match require_auth_identity_for_api::<Vec<memory::api::MemoryRecentItem>>(&state, &headers)
        {
            Ok(identity) => identity,
            Err(resp) => return resp,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            error!("list recent memories db failed: {}", err);
            return api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_database_unavailable",
            );
        }
    };
    match memory::api::list_recent(&db, identity.chat_id, &identity.principal_id, 50) {
        Ok(items) => api_ok(items),
        Err(err) => {
            error!("list recent memories failed: {}", err);
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "memory_lookup_failed")
        }
    }
}

async fn delete_memory_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(memory_id): AxumPath<String>,
) -> (
    StatusCode,
    Json<ApiResponse<memory::api::MemoryDeleteResult>>,
) {
    let identity =
        match require_auth_identity_for_api::<memory::api::MemoryDeleteResult>(&state, &headers) {
            Ok(identity) => identity,
            Err(resp) => return resp,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            error!("delete memory db failed: {}", err);
            return api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_database_unavailable",
            );
        }
    };
    match memory::api::delete_memory_object(
        &db,
        identity.user_id,
        identity.chat_id,
        &identity.user_key,
        &identity.principal_id,
        &memory_id,
        now_ts_u64() as i64,
    ) {
        Ok(Some(result)) => api_ok(result),
        Ok(None) => api_err(StatusCode::NOT_FOUND, "memory_not_found"),
        Err(err) => {
            warn!("delete memory failed: {}", err);
            api_err(StatusCode::BAD_REQUEST, "memory_id_invalid")
        }
    }
}

async fn expire_memory_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(memory_id): AxumPath<String>,
) -> (
    StatusCode,
    Json<ApiResponse<memory::api::MemoryExpireResult>>,
) {
    let identity =
        match require_auth_identity_for_api::<memory::api::MemoryExpireResult>(&state, &headers) {
            Ok(identity) => identity,
            Err(resp) => return resp,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            error!("expire memory db failed: {}", err);
            return api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_database_unavailable",
            );
        }
    };
    match memory::api::expire_memory_object(
        &db,
        identity.user_id,
        identity.chat_id,
        &identity.user_key,
        &identity.principal_id,
        &memory_id,
        now_ts_u64() as i64,
    ) {
        Ok(Some(result)) => api_ok(result),
        Ok(None) => api_err(StatusCode::NOT_FOUND, "memory_not_found"),
        Err(err) => {
            warn!("expire memory failed: {}", err);
            api_err(StatusCode::BAD_REQUEST, "memory_id_invalid")
        }
    }
}

async fn clear_memory_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<memory::api::MemoryClearRequest>,
) -> (
    StatusCode,
    Json<ApiResponse<memory::api::MemoryClearResult>>,
) {
    let identity =
        match require_auth_identity_for_api::<memory::api::MemoryClearResult>(&state, &headers) {
            Ok(identity) => identity,
            Err(resp) => return resp,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            error!("clear memory db failed: {}", err);
            return api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_database_unavailable",
            );
        }
    };
    match memory::api::clear_memory_scope(
        &db,
        identity.chat_id,
        &identity.principal_id,
        req.scope,
        now_ts_u64() as i64,
    ) {
        Ok(result) => api_ok(result),
        Err(err) => {
            error!("clear memory failed: {}", err);
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "memory_clear_failed")
        }
    }
}

#[derive(Debug, Deserialize)]
struct MemoryProjectLinkRequest {
    project_ref: String,
}

async fn get_current_memory_project_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (
    StatusCode,
    Json<ApiResponse<memory::scope::ResolvedMemoryScope>>,
) {
    let identity =
        match require_auth_identity_for_api::<memory::scope::ResolvedMemoryScope>(&state, &headers)
        {
            Ok(identity) => identity,
            Err(resp) => return resp,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            error!("get current memory project db failed: {}", err);
            return api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_project_db_failed",
            );
        }
    };
    match memory::scope::resolve_project_scope(&db, &identity, &state.skill_rt.workspace_root) {
        Ok(project) => api_ok(project),
        Err(err) => {
            error!("resolve current memory project failed: {}", err);
            api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_project_resolve_failed",
            )
        }
    }
}

async fn link_current_memory_project_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MemoryProjectLinkRequest>,
) -> (
    StatusCode,
    Json<ApiResponse<memory::scope::ResolvedMemoryScope>>,
) {
    let identity =
        match require_auth_identity_for_api::<memory::scope::ResolvedMemoryScope>(&state, &headers)
        {
            Ok(identity) => identity,
            Err(resp) => return resp,
        };
    if identity.role != "admin" {
        return api_err(StatusCode::FORBIDDEN, "memory_project_admin_required");
    }
    let project_ref = request.project_ref.trim();
    if project_ref.is_empty() || project_ref.len() > 96 {
        return api_err(StatusCode::BAD_REQUEST, "memory_project_ref_invalid");
    }
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            error!("link current memory project db failed: {}", err);
            return api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_project_db_failed",
            );
        }
    };
    let result = memory::project_identity::link_project_path_alias(
        &db,
        project_ref,
        &state.skill_rt.workspace_root,
    )
    .and_then(|_| {
        memory::scope::resolve_project_scope(&db, &identity, &state.skill_rt.workspace_root)
    });
    match result {
        Ok(project) => {
            let _ = insert_audit_log(
                &state,
                Some(identity.user_id),
                "memory_project_alias_link",
                Some(
                    &json!({
                        "actor_principal_id": identity.principal_id,
                        "target_project_ref": project.scope_ref,
                    })
                    .to_string(),
                ),
                None,
            );
            api_ok(project)
        }
        Err(err) => {
            warn!("link current memory project failed: {}", err);
            api_err(
                StatusCode::BAD_REQUEST,
                memory_api_error_code(&err, "memory_project_link_failed"),
            )
        }
    }
}

async fn unlink_current_memory_project_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MemoryProjectLinkRequest>,
) -> (
    StatusCode,
    Json<ApiResponse<memory::scope::ResolvedMemoryScope>>,
) {
    let identity =
        match require_auth_identity_for_api::<memory::scope::ResolvedMemoryScope>(&state, &headers)
        {
            Ok(identity) => identity,
            Err(resp) => return resp,
        };
    if identity.role != "admin" {
        return api_err(StatusCode::FORBIDDEN, "memory_project_admin_required");
    }
    let project_ref = request.project_ref.trim();
    if project_ref.is_empty() || project_ref.len() > 96 {
        return api_err(StatusCode::BAD_REQUEST, "memory_project_ref_invalid");
    }
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            error!("unlink current memory project db failed: {}", err);
            return api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_project_db_failed",
            );
        }
    };
    let result = memory::project_identity::unlink_project_path_alias(
        &db,
        project_ref,
        &state.skill_rt.workspace_root,
    )
    .and_then(|removed| {
        anyhow::ensure!(removed, "memory_project_alias_not_found");
        memory::scope::resolve_project_scope(&db, &identity, &state.skill_rt.workspace_root)
    });
    match result {
        Ok(project) => {
            let _ = insert_audit_log(
                &state,
                Some(identity.user_id),
                "memory_project_alias_unlink",
                Some(
                    &json!({
                        "actor_principal_id": identity.principal_id,
                        "detached_project_ref": project_ref,
                        "current_project_ref": project.scope_ref,
                    })
                    .to_string(),
                ),
                None,
            );
            api_ok(project)
        }
        Err(err) => {
            warn!("unlink current memory project failed: {}", err);
            api_err(
                StatusCode::BAD_REQUEST,
                memory_api_error_code(&err, "memory_project_unlink_failed"),
            )
        }
    }
}

async fn update_memory_settings_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<memory::settings::MemorySettingsUpdateRequest>,
) -> (
    StatusCode,
    Json<ApiResponse<memory::settings::MemoryEffectiveSettings>>,
) {
    let identity = match require_auth_identity_for_api::<memory::settings::MemoryEffectiveSettings>(
        &state, &headers,
    ) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            error!("update memory settings db failed: {}", err);
            return api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_settings_db_failed",
            );
        }
    };
    match memory::settings::update_memory_settings(
        &db,
        &identity,
        &req,
        state.policy.memory.long_term_enabled,
    ) {
        Ok(result) => {
            memory::retrieval_async::invalidate_principal_query_cache(&result.target_principal_id);
            if !result.generate_memory {
                let scope_ref = result
                    .conversation_id
                    .as_deref()
                    .and_then(|conversation_id| {
                        memory::scope::conversation_scope_ref(
                            &result.target_principal_id,
                            conversation_id,
                        )
                        .ok()
                    });
                if let Err(error) = memory::jobs::request_cancel_for_scope(
                    &db,
                    &result.target_principal_id,
                    scope_ref.as_deref(),
                ) {
                    warn!(error = %error, "memory settings job cancellation failed");
                }
            }
            if result.external_context_policy != memory::settings::ExternalContextPolicy::Allow {
                if let Err(error) = memory::embedding_jobs::revoke_remote_profiles_for_principal(
                    &db,
                    &result.target_principal_id,
                ) {
                    warn!(error = %error, "memory remote embedding revocation failed");
                }
            }
            let _ = insert_audit_log(
                &state,
                Some(identity.user_id),
                "memory_settings_update",
                Some(
                    &json!({
                        "actor_principal_id": identity.principal_id,
                        "target_principal_id": result.target_principal_id,
                        "scope": result.scope,
                        "conversation_id": result.conversation_id,
                        "revision": result.revision,
                        "policy_digest": result.policy_digest,
                    })
                    .to_string(),
                ),
                None,
            );
            api_ok(result)
        }
        Err(err) => {
            error!("update memory settings failed: {}", err);
            let error_code = memory_api_error_code(&err, "memory_settings_update_failed");
            let status = if error_code.contains("revision_conflict") {
                StatusCode::CONFLICT
            } else if error_code.contains("admin_required") {
                StatusCode::FORBIDDEN
            } else if error_code.contains("required") || error_code.contains("invalid") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            api_err(status, &error_code)
        }
    }
}
