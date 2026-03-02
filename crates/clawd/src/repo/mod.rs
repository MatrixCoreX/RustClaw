use crate::{AppState, ClaimedTask};

pub(crate) fn claim_next_task(state: &AppState) -> anyhow::Result<Option<ClaimedTask>> {
    super::claim_next_task(state)
}

pub(crate) fn update_task_success(
    state: &AppState,
    task_id: &str,
    result_json: &str,
) -> anyhow::Result<()> {
    super::update_task_success(state, task_id, result_json)
}

pub(crate) fn update_task_failure(
    state: &AppState,
    task_id: &str,
    error_text: &str,
) -> anyhow::Result<()> {
    super::update_task_failure(state, task_id, error_text)
}

pub(crate) fn insert_audit_log(
    state: &AppState,
    user_id: Option<i64>,
    action: &str,
    detail_json: Option<&str>,
    error_text: Option<&str>,
) -> anyhow::Result<()> {
    super::insert_audit_log(state, user_id, action, detail_json, error_text)
}
