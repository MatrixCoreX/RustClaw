use axum::extract::{rejection::QueryRejection, Path, Query, Request, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::Router;
use claw_core::types::AuthIdentity;
use serde::Deserialize;
use serde_json::Value;

use super::{reading, writing};
use crate::memory::import_upload::{self as upload, UploadError};
use crate::AppState;

#[path = "memory_import/body.rs"]
mod body;
#[path = "memory_import/errors.rs"]
mod errors;
#[path = "memory_import/execution.rs"]
mod execution;

pub(super) use errors::typed_upload_error;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/memory/import/policy", get(policy))
        .route("/memory/import/uploads", post(create))
        .route(
            "/memory/import/uploads/:upload_id",
            get(status).delete(cancel),
        )
        .route("/memory/import/uploads/:upload_id/chunks", put(append))
        .route("/memory/import/uploads/:upload_id/preview", post(preview))
}
fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<AuthIdentity, Response> {
    crate::require_auth_identity_for_api::<Value>(state, headers)
        .map_err(IntoResponse::into_response)
}

async fn policy(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let actor = match authenticate(&state, &headers) {
        Ok(actor) => actor,
        Err(reply) => return reply,
    };
    let ingress = state.policy.memory_import_ingress;
    let lease = state.policy.memory.background_lease_seconds;
    match reading::run(state, actor, move |db, actor| {
        upload::transport_policy(db, actor, ingress, lease)
    })
    .await
    {
        Ok(policy) => crate::api_ok(policy).into_response(),
        Err(error) => errors::failure(error),
    }
}
async fn create(State(state): State<AppState>, headers: HeaderMap, request: Request) -> Response {
    let actor = match authenticate(&state, &headers) {
        Ok(actor) => actor,
        Err(reply) => return reply,
    };
    let ingress = state.policy.memory_import_ingress;
    let policy = match ingress.parsing(state.policy.memory.background_lease_seconds) {
        Ok(policy) => policy,
        Err(error) => return errors::failure(error),
    };
    let raw = match body::read(request, ingress.json_bytes, ingress.body_read_seconds).await {
        Ok(raw) => raw,
        Err(reply) => return reply,
    };
    let request: upload::CreateUpload = match serde_json::from_slice(&raw) {
        Ok(request) => request,
        Err(_) => return errors::failure(UploadError::InvalidMetadata.into()),
    };
    match writing::run(state, actor, move |db, actor| {
        upload::create(
            db,
            actor,
            &request,
            policy.upload,
            crate::now_ts_u64() as i64,
        )
    })
    .await
    {
        Ok(status) => crate::api_ok(status).into_response(),
        Err(error) => errors::failure(error),
    }
}
async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let actor = match authenticate(&state, &headers) {
        Ok(actor) => actor,
        Err(reply) => return reply,
    };
    match reading::run(state, actor, move |db, actor| {
        upload::status(db, actor, &id)
    })
    .await
    {
        Ok(status) => crate::api_ok(status).into_response(),
        Err(error) => errors::failure(error),
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChunkQuery {
    offset: u64,
}
async fn append(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    query: Result<Query<ChunkQuery>, QueryRejection>,
    request: Request,
) -> Response {
    let actor = match authenticate(&state, &headers) {
        Ok(actor) => actor,
        Err(reply) => return reply,
    };
    let Query(query) = match query {
        Ok(query) => query,
        Err(_) => return errors::failure(UploadError::InvalidMetadata.into()),
    };
    let expected = match headers
        .get("x-content-sha256")
        .and_then(|v| v.to_str().ok())
        .filter(|v| upload::valid_digest(v))
    {
        Some(value) => value.to_owned(),
        None => return errors::failure(UploadError::ChunkDigestMismatch.into()),
    };
    let ingress = state.policy.memory_import_ingress;
    let host_limit = match ingress.chunk_bytes() {
        Ok(limit) => limit,
        Err(error) => return errors::failure(error),
    };
    let observed = id.clone();
    let pinned = match reading::run(state.clone(), actor.clone(), move |db, actor| {
        upload::chunk_budget(db, actor, &observed)
    })
    .await
    {
        Ok(bytes) => bytes,
        Err(error) => return errors::failure(error),
    };
    let limit = (host_limit as u64).min(pinned) as usize;
    let bytes = match body::read(request, limit, ingress.upload_read_seconds).await {
        Ok(bytes) => bytes,
        Err(reply) => return reply,
    };
    match writing::run(state, actor, move |db, actor| {
        upload::append(
            db,
            actor,
            &id,
            query.offset,
            &expected,
            &bytes,
            crate::now_ts_u64() as i64,
        )
    })
    .await
    {
        Ok(status) => crate::api_ok(status).into_response(),
        Err(error) => errors::failure(error),
    }
}
async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let actor = match authenticate(&state, &headers) {
        Ok(actor) => actor,
        Err(reply) => return reply,
    };
    match writing::run(state, actor, move |db, actor| {
        upload::cancel(db, actor, &id, crate::now_ts_u64() as i64)
    })
    .await
    {
        Ok(status) => crate::api_ok(status).into_response(),
        Err(error) => errors::failure(error),
    }
}
async fn preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let actor = match authenticate(&state, &headers) {
        Ok(actor) => actor,
        Err(reply) => return reply,
    };
    let policy = match state
        .policy
        .memory_import_ingress
        .parsing(state.policy.memory.background_lease_seconds)
    {
        Ok(policy) => policy,
        Err(error) => return errors::failure(error),
    };
    match execution::preview(state, actor, id, policy).await {
        Ok(preview) => crate::api_ok(preview).into_response(),
        Err(error) => errors::failure(error),
    }
}
