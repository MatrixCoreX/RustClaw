pub(crate) fn channel_allows_shared_ui_task_access(channel: &str) -> bool {
    matches!(
        channel,
        "telegram" | "whatsapp" | "wechat" | "feishu" | "lark"
    )
}

pub(crate) enum TaskViewerAccessError {
    AuthLookup(anyhow::Error),
    TaskOwnerMismatch,
    InvalidUserKey,
}

pub(crate) fn check_task_view_access(
    state: &AppState,
    task_user_key: Option<&str>,
    channel: &str,
    provided_key: Option<&str>,
) -> Result<(), TaskViewerAccessError> {
    let expected_key = task_user_key.map(str::trim).filter(|v| !v.is_empty());
    let provided_key = provided_key.map(crate::normalize_user_key);
    let provided_key = provided_key.as_deref().filter(|v| !v.is_empty());
    let viewer_identity = match provided_key {
        Some(key) => crate::resolve_auth_identity_by_key(state, key)
            .map_err(TaskViewerAccessError::AuthLookup)?,
        None => None,
    };
    if viewer_identity
        .as_ref()
        .is_some_and(|identity| identity.role.eq_ignore_ascii_case("admin"))
    {
        return Ok(());
    }
    if !channel_allows_shared_ui_task_access(channel) {
        if let Some(expected_key) = expected_key {
            if provided_key != Some(expected_key) {
                return Err(TaskViewerAccessError::TaskOwnerMismatch);
            }
        }
    } else if provided_key.is_some() && viewer_identity.is_none() {
        return Err(TaskViewerAccessError::InvalidUserKey);
    }
    Ok(())
}
