use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use claw_core::types::ApiResponse;
use serde_json::Value;

use crate::AppState;

fn require_hook_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, Json<ApiResponse<Value>>)> {
    let identity = crate::http::ui_routes::require_ui_identity(state, headers)?;
    if identity.role.eq_ignore_ascii_case("admin") {
        Ok(())
    } else {
        Err(super::api_err(StatusCode::FORBIDDEN, "hook_admin_required"))
    }
}

pub(super) async fn get_hook_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_hook_admin(&state, &headers) {
        return response;
    }
    super::api_ok(crate::agent_hooks::hook_admin_status_for_state(&state))
}
