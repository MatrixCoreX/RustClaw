use anyhow::Context;

use crate::AppState;

/// Copy task artifacts and return the result JSON that must be stored with
/// `status=succeeded`. Channel delivery is triggered by that status write, so
/// materialization has to finish first; otherwise a large file copy can lose
/// the race and the channel will accept a text-only receipt.
pub(crate) fn prepare_succeeded_result_json(
    state: &AppState,
    task_id: &str,
    result_json: &str,
) -> anyhow::Result<String> {
    let root = state.stored_task_files_root(task_id)
        .context("artifact_workspace_owner_unavailable")?;
    crate::task_artifacts::materialize_task_result_artifacts(&root, task_id, result_json)
        .context("artifact_publication_failed")
}
