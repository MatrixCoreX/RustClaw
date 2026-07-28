use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use claw_core::types::ApiResponse;
use serde::Deserialize;

use crate::{
    repo::conversation_history::{
        ConversationArchiveUpdate, ConversationBodyField, ConversationBodyPage,
        ConversationHistoryPage, ConversationTitleUpdate,
    },
    resolve_auth_identity_by_key, AppState,
};

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ConversationHistoryQuery {
    limit: Option<usize>,
    cursor: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ConversationBodyQuery {
    start_byte: Option<usize>,
    limit_bytes: Option<usize>,
    sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConversationTitleRequest {
    title: String,
}

pub(crate) async fn list_conversation_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ConversationHistoryQuery>,
) -> (StatusCode, Json<ApiResponse<ConversationHistoryPage>>) {
    let Some(raw_key) = headers
        .get("x-rustclaw-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return crate::api_err(StatusCode::UNAUTHORIZED, "auth_key_required");
    };
    let identity = match resolve_auth_identity_by_key(&state, raw_key) {
        Ok(Some(identity)) => identity,
        Ok(None) => return crate::api_err(StatusCode::UNAUTHORIZED, "auth_key_invalid"),
        Err(error) => {
            tracing::error!("conversation_history_auth_lookup_failed error={error}");
            return crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "conversation_history_auth_lookup_failed",
            );
        }
    };
    match crate::repo::conversation_history::list_conversation_history(
        &state,
        &identity,
        query.limit,
        query.cursor.as_deref(),
    ) {
        Ok(page) => crate::api_ok(page),
        Err(error) if error.to_string() == "conversation_history_cursor_invalid" => crate::api_err(
            StatusCode::BAD_REQUEST,
            "conversation_history_cursor_invalid",
        ),
        Err(error) => {
            tracing::error!("conversation_history_query_failed error={error}");
            crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "conversation_history_query_failed",
            )
        }
    }
}

pub(crate) async fn get_conversation_body_range(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path((task_id, field)): axum::extract::Path<(String, String)>,
    Query(query): Query<ConversationBodyQuery>,
) -> (StatusCode, Json<ApiResponse<ConversationBodyPage>>) {
    let Some(raw_key) = headers
        .get("x-rustclaw-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return crate::api_err(StatusCode::UNAUTHORIZED, "auth_key_required");
    };
    let identity = match resolve_auth_identity_by_key(&state, raw_key) {
        Ok(Some(identity)) => identity,
        Ok(None) => return crate::api_err(StatusCode::UNAUTHORIZED, "auth_key_invalid"),
        Err(error) => {
            tracing::error!("conversation_body_auth_lookup_failed error={error}");
            return crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "conversation_body_auth_lookup_failed",
            );
        }
    };
    let field = match ConversationBodyField::parse(&field) {
        Ok(field) => field,
        Err(error) => return crate::api_err(StatusCode::BAD_REQUEST, error.to_string()),
    };
    match crate::repo::conversation_history::read_conversation_body_range(
        &state,
        &identity,
        &task_id,
        field,
        query.start_byte,
        query.limit_bytes,
        query.sha256.as_deref(),
    ) {
        Ok(page) => crate::api_ok(page),
        Err(error)
            if matches!(
                error.to_string().as_str(),
                "conversation_body_task_id_invalid"
                    | "conversation_body_sha256_invalid"
                    | "conversation_body_range_invalid"
            ) =>
        {
            crate::api_err(StatusCode::BAD_REQUEST, error.to_string())
        }
        Err(error) if error.to_string() == "conversation_body_not_found" => {
            crate::api_err(StatusCode::NOT_FOUND, "conversation_body_not_found")
        }
        Err(error) if error.to_string() == "conversation_body_stale_snapshot" => {
            crate::api_err(StatusCode::CONFLICT, "conversation_body_stale_snapshot")
        }
        Err(error) => {
            tracing::error!("conversation_body_query_failed error={error}");
            crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "conversation_body_query_failed",
            )
        }
    }
}

pub(crate) async fn update_conversation_title(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(conversation_id): axum::extract::Path<String>,
    Json(request): Json<ConversationTitleRequest>,
) -> (StatusCode, Json<ApiResponse<ConversationTitleUpdate>>) {
    let Some(raw_key) = headers
        .get("x-rustclaw-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return crate::api_err(StatusCode::UNAUTHORIZED, "auth_key_required");
    };
    let identity = match resolve_auth_identity_by_key(&state, raw_key) {
        Ok(Some(identity)) => identity,
        Ok(None) => return crate::api_err(StatusCode::UNAUTHORIZED, "auth_key_invalid"),
        Err(error) => {
            tracing::error!("conversation_title_auth_lookup_failed error={error}");
            return crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "conversation_title_auth_lookup_failed",
            );
        }
    };
    match crate::repo::conversation_history::update_conversation_title(
        &state,
        &identity,
        &conversation_id,
        &request.title,
    ) {
        Ok(result) => crate::api_ok(result),
        Err(error)
            if matches!(
                error.to_string().as_str(),
                "conversation_id_invalid" | "conversation_title_invalid"
            ) =>
        {
            crate::api_err(StatusCode::BAD_REQUEST, error.to_string())
        }
        Err(error) if error.to_string() == "conversation_not_found" => {
            crate::api_err(StatusCode::NOT_FOUND, "conversation_not_found")
        }
        Err(error) => {
            tracing::error!("conversation_title_update_failed error={error}");
            crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "conversation_title_update_failed",
            )
        }
    }
}

pub(crate) async fn archive_conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(conversation_id): axum::extract::Path<String>,
) -> (StatusCode, Json<ApiResponse<ConversationArchiveUpdate>>) {
    let Some(raw_key) = headers
        .get("x-rustclaw-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return crate::api_err(StatusCode::UNAUTHORIZED, "auth_key_required");
    };
    let identity = match resolve_auth_identity_by_key(&state, raw_key) {
        Ok(Some(identity)) => identity,
        Ok(None) => return crate::api_err(StatusCode::UNAUTHORIZED, "auth_key_invalid"),
        Err(error) => {
            tracing::error!("conversation_archive_auth_lookup_failed error={error}");
            return crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "conversation_archive_auth_lookup_failed",
            );
        }
    };
    match crate::repo::conversation_history::archive_conversation(
        &state,
        &identity,
        &conversation_id,
    ) {
        Ok(result) => crate::api_ok(result),
        Err(error) if error.to_string() == "conversation_id_invalid" => {
            crate::api_err(StatusCode::BAD_REQUEST, "conversation_id_invalid")
        }
        Err(error) if error.to_string() == "conversation_not_found" => {
            crate::api_err(StatusCode::NOT_FOUND, "conversation_not_found")
        }
        Err(error) => {
            tracing::error!("conversation_archive_failed error={error}");
            crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "conversation_archive_failed",
            )
        }
    }
}
