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

pub(crate) async fn run_tool(
    state: &AppState,
    tool: &str,
    args: &Value,
) -> Result<String, String> {
    super::execute_builtin_tool(state, tool, args).await
}
