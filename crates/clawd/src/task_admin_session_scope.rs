use super::*;

type Response = (StatusCode, Json<ApiResponse<Value>>);

fn anchor(
    state: &AppState,
    headers: &HeaderMap,
    task_id: Option<&str>,
) -> Result<crate::TaskAdminTarget, Response> {
    let (identity, key) = require_task_admin_identity(state, headers)?;
    let task_id = task_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            super::super::api_err(StatusCode::BAD_REQUEST, "task_scope_anchor_required")
        })?;
    let target = crate::get_task_admin_target(state, task_id)
        .map_err(|error| {
            error!("task_scope_anchor_lookup_failed error={error}");
            super::super::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task_scope_lookup_failed",
            )
        })?
        .ok_or_else(|| super::super::api_err(StatusCode::NOT_FOUND, "task_not_found"))?;
    let key_matches = target
        .user_key
        .as_deref()
        .is_some_and(|owner| crate::normalize_user_key(owner) == key);
    if target.user_id != identity.user_id && !key_matches {
        return Err(super::super::api_err(
            StatusCode::FORBIDDEN,
            "task_owner_mismatch",
        ));
    }
    Ok(target)
}

fn descriptor(anchor: &crate::TaskAdminTarget) -> Value {
    json!({
        "kind": "current_session",
        "source": "current_task_record",
        "anchor_task_id": anchor.task_id,
        "user_id": anchor.user_id,
        "chat_id": anchor.chat_id,
        "channel": anchor.channel,
    })
}

pub(super) fn list(
    state: &AppState,
    headers: &HeaderMap,
    task_id: Option<&str>,
    limit: usize,
    offset: usize,
) -> Response {
    let anchor = match anchor(state, headers, task_id) {
        Ok(anchor) => anchor,
        Err(response) => return response,
    };
    match crate::repo::list_active_tasks_for_session_page_internal(state, &anchor, limit, offset) {
        Ok(page) => super::super::api_ok(json!({
            "scope": descriptor(&anchor),
            "count": page.tasks.len(),
            "total": page.total,
            "limit": page.limit,
            "offset": page.offset,
            "has_more": page.has_more,
            "tasks": page.tasks,
        })),
        Err(error) => {
            error!("task_session_list_failed error={error}");
            super::super::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task_session_list_failed",
            )
        }
    }
}

pub(super) fn cancel_all(state: &AppState, headers: &HeaderMap, task_id: Option<&str>) -> Response {
    let anchor = match anchor(state, headers, task_id) {
        Ok(anchor) => anchor,
        Err(response) => return response,
    };
    match crate::repo::cancel_tasks_for_session(state, &anchor) {
        Ok(count) => super::super::api_ok(json!({"canceled": count, "scope": descriptor(&anchor)})),
        Err(error) => {
            error!("task_session_cancel_failed error={error}");
            super::super::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task_session_cancel_failed",
            )
        }
    }
}

pub(super) fn cancel_one(
    state: &AppState,
    headers: &HeaderMap,
    task_id: Option<&str>,
    index: usize,
    expected_task_id: Option<&str>,
) -> Response {
    let anchor = match anchor(state, headers, task_id) {
        Ok(anchor) => anchor,
        Err(response) => return response,
    };
    if !(1..=100_001).contains(&index) {
        return super::super::api_err(StatusCode::BAD_REQUEST, "task_index_invalid");
    }
    let Some(expected_task_id) = expected_task_id.filter(|id| !id.trim().is_empty()) else {
        return super::super::api_err(StatusCode::BAD_REQUEST, "task_expected_id_required");
    };
    let page = match crate::repo::list_active_tasks_for_session_page_internal(
        state,
        &anchor,
        1,
        index - 1,
    ) {
        Ok(page) => page,
        Err(error) => {
            error!("task_session_list_failed error={error}");
            return super::super::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task_session_list_failed",
            );
        }
    };
    let Some(target) = page.tasks.into_iter().next() else {
        return super::super::api_err(StatusCode::NOT_FOUND, "task_not_found");
    };
    if target.task_id != expected_task_id {
        return super::super::api_err(StatusCode::CONFLICT, "task_index_changed");
    }
    match crate::cancel_one_task_for_user_chat(
        state,
        anchor.user_id,
        anchor.chat_id,
        &target.task_id,
    ) {
        Ok(count) if count > 0 => super::super::api_ok(
            json!({"canceled": count, "task": target, "scope": descriptor(&anchor)}),
        ),
        Ok(_) => super::super::api_err(StatusCode::CONFLICT, "task_no_longer_active"),
        Err(error) => {
            error!("task_session_cancel_failed error={error}");
            super::super::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task_session_cancel_failed",
            )
        }
    }
}
