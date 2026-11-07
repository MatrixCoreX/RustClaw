use axum::{http::StatusCode, Json};
use claw_core::types::AuthIdentity;
use rusqlite::Connection;

use crate::{ApiResponse, AppState};

#[derive(Debug)]
enum ExecutionError {
    DatabaseUnavailable,
    WorkerUnavailable,
}

impl ExecutionError {
    fn code(&self) -> &'static str {
        match self {
            Self::DatabaseUnavailable => "memory_database_unavailable",
            Self::WorkerUnavailable => "memory_mutation_worker_unavailable",
        }
    }
}

impl std::fmt::Display for ExecutionError {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str(self.code())
    }
}

impl std::error::Error for ExecutionError {}

pub(super) async fn run<T, F>(
    state: AppState,
    identity: AuthIdentity,
    operation: F,
) -> anyhow::Result<T>
where
    T: Send + 'static,
    F: FnOnce(&Connection, &AuthIdentity) -> anyhow::Result<T> + Send + 'static,
{
    // Machine-only observability of the actual post-authentication queue point.
    // No key, payload or private content is logged.
    tracing::debug!(memory_write_stage = "queued", principal_id = %identity.principal_id, "memory_mutation_writer");
    tokio::task::spawn_blocking(move || {
        let db = state
            .core
            .db
            .get()
            .map_err(|_| ExecutionError::DatabaseUnavailable)?;
        crate::memory::ux::atomic_mutation(&db, |db| {
            crate::repo::auth::require_request_identity(db, &identity)?;
            let result = operation(db, &identity)?;
            crate::repo::auth::require_request_identity(db, &identity)?;
            Ok(result)
        })
    })
    .await
    .map_err(|error| {
        tracing::warn!(error = %error, "memory_mutation_worker_join_failed");
        anyhow::Error::from(ExecutionError::WorkerUnavailable)
    })?
}

pub(super) fn execution_error<T: serde::Serialize>(
    error: &anyhow::Error,
) -> Option<(StatusCode, Json<ApiResponse<T>>)> {
    if let Some(failure) = error.downcast_ref::<ExecutionError>() {
        return Some(crate::api_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            failure.code(),
        ));
    }
    let failure = error.downcast_ref::<crate::repo::auth::RequestIdentityError>()?;
    let status = match failure {
        crate::repo::auth::RequestIdentityError::Revoked => StatusCode::UNAUTHORIZED,
        crate::repo::auth::RequestIdentityError::Changed => StatusCode::FORBIDDEN,
    };
    Some(crate::api_err(status, failure.code()))
}
