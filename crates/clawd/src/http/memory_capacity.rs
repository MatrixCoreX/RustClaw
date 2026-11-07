use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Deserialize;

use crate::memory::availability::{self, MemoryAvailability};
use crate::memory::settings::{self, MemorySettingScope};
use crate::{api_err, api_ok, ApiResponse, AppState};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapacityQuery {
    pub(super) scope: Option<MemorySettingScope>,
    pub(super) conversation_id: Option<String>,
}

type Reply = (StatusCode, Json<ApiResponse<MemoryAvailability>>);

pub(super) async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<CapacityQuery>,
) -> Reply {
    match tokio::task::spawn_blocking(move || read(&state, &headers, query)).await {
        Ok(reply) => reply,
        Err(error) => {
            tracing::error!(%error,"memory capacity observation worker failed");
            api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_capacity_unavailable",
            )
        }
    }
}

fn read(state: &AppState, headers: &HeaderMap, query: CapacityQuery) -> Reply {
    let identity = match crate::require_auth_identity_for_api::<MemoryAvailability>(state, headers)
    {
        Ok(identity) => identity,
        Err(reply) => return reply,
    };
    let scope = query.scope.unwrap_or_default();
    if scope == MemorySettingScope::Admin {
        return api_err(StatusCode::BAD_REQUEST, "memory_capacity_scope_invalid");
    }
    let conversation = query
        .conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if scope == MemorySettingScope::Conversation && conversation.is_none() {
        return api_err(
            StatusCode::BAD_REQUEST,
            "memory_settings_conversation_required",
        );
    }
    if scope == MemorySettingScope::Principal && query.conversation_id.is_some() {
        return api_err(StatusCode::BAD_REQUEST, "memory_capacity_scope_invalid");
    }
    let observation = || -> anyhow::Result<_> {
        let db = state.core.db.get()?;
        let transaction = db.unchecked_transaction()?;
        let settings = if scope == MemorySettingScope::Conversation {
            settings::resolve_memory_settings(
                &transaction,
                &identity.principal_id,
                conversation,
                state.policy.memory.long_term_enabled,
            )?
        } else {
            settings::resolve_principal_memory_settings(
                &transaction,
                &identity.principal_id,
                state.policy.memory.long_term_enabled,
            )?
        };
        let (report, path) = availability::observe_sqlite(
            &transaction,
            settings,
            identity.role == "admin",
            state.policy.memory.storage_soft_limit_bytes,
            crate::now_ts_u64() as i64,
        )?;
        transaction.commit()?;
        drop(db);
        Ok((report, path))
    };
    match observation() {
        Ok((mut report, path)) => {
            if identity.role == "admin" {
                report.measurements.database_path_filesystem_available_bytes =
                    availability::filesystem_available(path.as_deref());
            }
            api_ok(report)
        }
        Err(error) => {
            tracing::warn!(%error,"memory capacity observation unavailable");
            api_err(
                StatusCode::SERVICE_UNAVAILABLE,
                "memory_capacity_unavailable",
            )
        }
    }
}

#[cfg(test)]
#[path = "memory_capacity_tests.rs"]
mod tests;
