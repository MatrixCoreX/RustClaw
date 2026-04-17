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

// 注：之前这里曾保留过一个 `run_tool` legacy stub（直接调用
// `crate::skills::execute_builtin_skill` 走"无 task 上下文"路径），但实际生产
// 链路统一走 `run_skill -> run_skill_with_runner -> execute_builtin_skill_for_task`，
// 该 stub 零调用方且会绕过 LLM 预算 / model_io 日志 / provider fallback，
// 留下是潜在事故源，已在审计 P0 时删除。如未来真的需要，请优先提供 task 上下文。
