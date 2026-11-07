use claw_core::types::AuthIdentity;

use crate::memory::import_upload::{self as upload, ParsePolicy};
use crate::memory::ux::MemoryImportPreview;
use crate::AppState;

use super::super::writing;

/// Deliberately not writing::run: the parser owns short admitted staging batches
/// and one publication transaction. An outer transaction would retain the SQLite
/// writer lock for the entire read/parse and turn its batches into savepoints.
pub(super) async fn preview(
    state: AppState,
    actor: AuthIdentity,
    id: String,
    policy: ParsePolicy,
) -> anyhow::Result<MemoryImportPreview> {
    tokio::task::spawn_blocking(move || {
        let db = state.core.db.get().map_err(|_| {
            writing::reject(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "memory_database_unavailable",
            )
        })?;
        upload::parse_and_preview(&db, &actor, &id, policy, crate::now_ts_u64() as i64)
    })
    .await
    .map_err(|error| {
        tracing::warn!(%error,"memory_import_parser_worker_join_failed");
        writing::reject(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "memory_import_parser_worker_unavailable",
        )
    })?
}
