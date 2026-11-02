use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AccountControlRequest {
    operation: String,
    platform: Option<String>,
    #[serde(default)]
    confirm: bool,
}

fn selected_capability<'a>(
    controls: &'a skill_sdk::AippAccountControls,
    request: &AccountControlRequest,
) -> Option<&'a str> {
    if controls.schema_version != 1 {
        return None;
    }
    if request.platform.as_ref().is_some_and(|value| {
        value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    }) {
        return None;
    }
    match request.operation.as_str() {
        "status" => Some(&controls.status_capability),
        "logout" if request.confirm && request.platform.is_some() => {
            Some(&controls.logout_capability)
        }
        _ => None,
    }
}

pub(super) async fn control_aipp_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(skill_name): AxumPath<String>,
    Json(request): Json<AccountControlRequest>,
) -> axum::response::Response {
    if let Err(response) = require_ui_admin(&state, &headers) {
        return response.into_response();
    }
    let active = match active_aipp_package(&state, &skill_name) {
        Ok(Some(active)) if aipp_is_installed(&state, &skill_name) => active,
        _ => return aipp_api_error(StatusCode::NOT_FOUND, "aipp_not_available").into_response(),
    };
    let Some(controls) = active.aipp.account_controls else {
        return aipp_api_error(StatusCode::BAD_REQUEST, "aipp_account_controls_unsupported")
            .into_response();
    };
    let capability = match selected_capability(&controls, &request) {
        Some(capability) => capability,
        _ => {
            return aipp_api_error(StatusCode::BAD_REQUEST, "aipp_account_control_invalid")
                .into_response()
        }
    };
    let registry = state.get_skills_registry();
    let expected_effect = if request.operation == "status" {
        claw_core::skill_registry::PlannerCapabilityEffect::Observe
    } else {
        claw_core::skill_registry::PlannerCapabilityEffect::Mutate
    };
    let policy_matches = registry
        .as_ref()
        .and_then(|registry| registry.get(&skill_name))
        .is_some_and(|entry| {
            entry
                .planner_capabilities
                .iter()
                .any(|item| item.name == capability && item.effect == Some(expected_effect))
        });
    if !policy_matches {
        return aipp_api_error(StatusCode::CONFLICT, "aipp_account_policy_changed").into_response();
    }
    let mut args = serde_json::Map::new();
    if let Some(platform) = request.platform {
        args.insert("platform".into(), json!(platform));
    }
    if request.operation == "logout" {
        args.insert("confirm".into(), json!(true));
    }
    let task: claw_core::types::SubmitTaskRequest = match serde_json::from_value(json!({
        "channel":"ui", "kind":"ask", "idempotency_key":format!("aipp-account-{}", uuid::Uuid::new_v4()),
        "payload":{"entrypoint":"run_capability", "capability":capability, "args":args, "_task_source":"aipp_account_control"}
    })) {
        Ok(task) => task,
        Err(_) => {
            return aipp_api_error(StatusCode::BAD_REQUEST, "aipp_account_control_invalid")
                .into_response()
        }
    };
    crate::submit_task(State(state), headers, Json(task))
        .await
        .into_response()
}

pub(super) fn public_account_use(value: Option<&Value>) -> Vec<Value> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            if row.get("schema_version").and_then(Value::as_u64) != Some(1) {
                return None;
            }
            let platform = row.get("platform")?.as_str()?;
            if platform.is_empty()
                || platform.len() > 64
                || !platform
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
            {
                return None;
            }
            let mode = match row.get("mode").and_then(Value::as_str) {
                Some("authenticated_browser") => "authenticated_browser",
                _ => "unknown",
            };
            let account = row.get("account");
            let status = match account
                .and_then(|v| v.get("status"))
                .and_then(Value::as_str)
            {
                Some(value @ ("verified" | "saved_session" | "expired" | "signed_out")) => value,
                _ => "unknown",
            };
            let name = account
                .and_then(|v| v.get("display_name"))
                .and_then(Value::as_str)
                .filter(|name| {
                    status == "verified"
                        && name.chars().count() <= 256
                        && !name.chars().any(char::is_control)
                });
            Some(json!({"schema_version":1,"platform":platform,"mode":mode,
            "account":{"schema_version":1,"platform":platform,"status":status,"display_name":name,
            "checked_at":account.and_then(|v| v.get("checked_at")).and_then(Value::as_u64)}}))
        })
        .collect()
}

pub(super) fn task_account_use(result: &Value) -> Vec<Value> {
    let mut rows = public_account_use(result.pointer("/extra/account_use"));
    if let Some(results) = result
        .pointer("/task_journal/trace/capability_results")
        .and_then(Value::as_array)
    {
        for item in results {
            rows.extend(public_account_use(item.pointer("/data/extra/account_use")));
        }
    }
    rows
}

#[cfg(test)]
#[path = "aipp_accounts_tests.rs"]
mod tests;
