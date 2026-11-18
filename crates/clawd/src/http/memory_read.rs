use axum::http::StatusCode;
use claw_core::types::AuthIdentity;
use rusqlite::Connection;

use crate::AppState;

/// Authorize the original credential and read its data in one SQLite snapshot.
/// The operation is read-only: retries cannot initialize or repair storage.
pub(super) async fn run<T, F>(
    state: AppState,
    identity: AuthIdentity,
    operation: F,
) -> anyhow::Result<T>
where
    T: Send + 'static,
    F: Fn(&Connection, &AuthIdentity) -> anyhow::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        crate::sqlite_busy_retry::with_sqlite_busy_retry(
            crate::sqlite_busy_retry::SqliteBusyRetryPolicy::default(),
            || {
                let db = state.core.db.get().map_err(|_| {
                    super::writing::reject(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "memory_database_unavailable",
                    )
                })?;
                let snapshot = db.unchecked_transaction()?;
                crate::repo::auth::require_request_identity(&snapshot, &identity)?;
                operation(&snapshot, &identity)
            },
        )
    })
    .await
    .map_err(|error| {
        tracing::warn!(%error, "memory_read_worker_join_failed");
        super::writing::reject(
            StatusCode::INTERNAL_SERVER_ERROR,
            "memory_read_worker_unavailable",
        )
    })?
}
