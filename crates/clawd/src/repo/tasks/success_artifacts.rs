use tracing::warn;

use crate::AppState;

/// Copy task artifacts and return the result JSON that must be stored with
/// `status=succeeded`. Channel delivery is triggered by that status write, so
/// materialization has to finish first; otherwise a large file copy can lose
/// the race and the channel will accept a text-only receipt.
pub(crate) fn prepare_succeeded_result_json(
    state: &AppState,
    task_id: &str,
    result_json: &str,
) -> String {
    match crate::task_artifacts::materialize_task_result_artifacts(
        &state.skill_rt.workspace_root,
        task_id,
        result_json,
    ) {
        Ok(delivered) => delivered,
        Err(error) => {
            warn!(
                "task artifact materialization failed task_id={} error={}",
                task_id, error
            );
            result_json.to_string()
        }
    }
}
