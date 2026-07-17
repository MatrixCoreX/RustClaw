use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use claw_core::types::ApiResponse;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::AppState;

#[derive(Debug, Default, Deserialize)]
pub(super) struct McpToolsQuery {
    server_id: Option<String>,
}

fn require_mcp_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, Json<ApiResponse<Value>>)> {
    let identity = crate::http::ui_routes::require_ui_identity(state, headers)?;
    if identity.role.eq_ignore_ascii_case("admin") {
        Ok(())
    } else {
        Err(super::api_err(StatusCode::FORBIDDEN, "mcp_admin_required"))
    }
}

pub(super) async fn list_mcp_servers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_mcp_admin(&state, &headers) {
        return response;
    }
    super::api_ok(json!({
        "servers": state.mcp_lifecycle_snapshots(),
    }))
}

pub(super) async fn list_mcp_tools(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<McpToolsQuery>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_mcp_admin(&state, &headers) {
        return response;
    }
    let server_id = query
        .server_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let tools = state
        .mcp_tools()
        .into_iter()
        .filter(|tool| server_id.is_none_or(|server_id| tool.server_id == server_id))
        .collect::<Vec<_>>();
    super::api_ok(json!({ "tools": tools }))
}

pub(super) async fn test_mcp_server(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(server_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_mcp_admin(&state, &headers) {
        return response;
    }
    let server_id = server_id.trim();
    if server_id.is_empty() {
        return super::api_err(StatusCode::BAD_REQUEST, "mcp_server_id_required");
    }
    match state.probe_mcp_server(server_id).await {
        Ok(outcome) => super::api_ok(json!({ "probe": outcome })),
        Err(error_code) => {
            let status = if error_code == "mcp_server_not_configured" {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            };
            super::api_err(status, error_code)
        }
    }
}
