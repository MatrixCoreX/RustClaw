use serde_json::Value;

use crate::{AppState, ClaimedTask};

pub(crate) async fn run_skill(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    args: Value,
) -> Result<String, String> {
    super::run_skill_with_runner(state, task, skill_name, args).await
}

/// LEGACY COMPATIBILITY ONLY: do not use in main path. Main chain is run_skill -> run_skill_with_runner (dispatcher by registry.kind) -> builtin/runner/external.
#[doc(hidden)]
#[allow(dead_code)]
pub(crate) async fn run_tool(state: &AppState, tool: &str, args: &Value) -> Result<String, String> {
    super::execute_builtin_skill(state, tool, args).await
}
