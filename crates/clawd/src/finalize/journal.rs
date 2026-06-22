//! Phase 3.3 Stage 3.1 — finalize 层 journal 构建器统一入口。
//!
//! 此前 journal 构建逻辑分散在两处：
//! - `finalize::task::ensure_journal_task_metrics`：TASK 层 backfill 缺失字段
//! - `finalize::loop_reply::build_loop_journal`：LOOP 层 from-scratch 构建
//!
//! Stage 3.1 把两者集中到本模块，作为 finalize 子层共享的 journal builder：
//! 1. `ensure_task_metrics` —— TASK 层入口前的最后兜底（`used_evidence_ids_count`
//!    与 `delivery_consistent` 两个 v1 字段保证非空）。
//! 2. `build_from_loop_state` —— LOOP 层从 `LoopState` + `AgentRunContext`
//!    一次性构建完整 journal（含 route / rounds / step_results / finalizer_summary
//!    / final_answer / final_status）。
//!
//! **不变量**：本模块对 journal 字段的写入顺序、值、JSON 字段名与原实现 byte-identical。
//! 不允许在此处引入 b1_regression 行为变化（含 `task_journal_summary` 输出）。

use crate::agent_engine::{AgentRunContext, LoopState};
use crate::task_journal::{
    delivery_payload_consistent, TaskJournal, TaskJournalFinalStatus, TaskJournalFinalizerSummary,
};
use crate::ClaimedTask;

/// TASK 层入口前调用：保证 v1 task_metrics 中的两个核心字段非空。
///
/// - `used_evidence_ids_count`：若没有 `finalizer_summary` 也没有显式记录，
///   则补 0（语义：这次 finalize 没有引用任何 evidence id）。
/// - `delivery_consistent`：基于 `answer_text` 与 `answer_messages` 即时计算。
///
/// 既存值不会被覆盖，纯增量。
pub(crate) fn ensure_task_metrics(
    journal: &mut TaskJournal,
    answer_text: &str,
    answer_messages: &[String],
) {
    if journal.finalizer_summary.is_none() && journal.task_metrics.used_evidence_ids_count.is_none()
    {
        journal.record_used_evidence_ids_count(0);
    }
    if journal.task_metrics.delivery_consistent.is_none() {
        journal
            .record_delivery_consistent(delivery_payload_consistent(answer_text, answer_messages));
    }
}

fn rollout_switches_from_loop_state(loop_state: &LoopState) -> Vec<String> {
    loop_state
        .output_vars
        .get("rollout_switches_enabled")
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// LOOP 层一次性构建 journal（替代原 `loop_reply::build_loop_journal`）。
///
/// 字段写入顺序与值保持原实现一致：
/// 1. `record_route_result` / `record_context_bundle_summary`（来自 ctx）
/// 2. `rounds = round_traces.clone()`
/// 3. 每个 step 走 `push_step_result`
/// 4. `record_finalizer_summary` 或 `record_used_evidence_ids_count(0)`
/// 5. `record_delivery_consistent`
/// 6. `record_final_answer` / `record_final_status`
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_from_loop_state(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: Option<TaskJournalFinalizerSummary>,
    delivery_consistent: bool,
    final_text: &str,
    final_status: TaskJournalFinalStatus,
) -> TaskJournal {
    let mut journal = TaskJournal::for_task(&task.task_id, "ask", user_text);
    if let Some(ctx) = agent_run_context {
        if let Some(route_result) = ctx.route_result.as_ref() {
            journal.record_route_result(route_result);
        }
        if let Some(context_summary) = ctx.context_bundle_summary.as_deref() {
            journal.record_context_bundle_summary(context_summary.to_string());
        }
    }
    journal.record_rollout_switches_enabled(rollout_switches_from_loop_state(loop_state));
    for attribution in &loop_state.rollout_attribution {
        journal.record_rollout_attribution(attribution.clone());
    }
    journal.rounds = loop_state.round_traces.clone();
    for step in &loop_state.executed_step_results {
        journal.push_step_result(step);
    }
    for observation in &loop_state.task_observations {
        journal.push_task_observation(observation.clone());
    }
    let mut stop_observation =
        crate::agent_hooks::stop_outcome(final_status.as_str()).to_machine_json("agent_loop");
    if let Some(obj) = stop_observation.as_object_mut() {
        obj.insert(
            "final_status".to_string(),
            serde_json::json!(final_status.as_str()),
        );
        if let Some(stop_signal) = loop_state.last_stop_signal.as_deref() {
            obj.insert(
                "final_stop_signal".to_string(),
                serde_json::json!(stop_signal),
            );
        }
    }
    journal.push_task_observation(stop_observation);
    let mut session_end = crate::agent_hooks::session_end_outcome(final_status.as_str())
        .to_machine_json("agent_loop");
    if let Some(obj) = session_end.as_object_mut() {
        obj.insert(
            "final_status".to_string(),
            serde_json::json!(final_status.as_str()),
        );
    }
    journal.push_task_observation(session_end);
    if let Some(summary) = finalizer_summary {
        journal.record_finalizer_summary(summary);
    } else {
        journal.record_used_evidence_ids_count(0);
    }
    if let Some(stop_signal) = loop_state.last_stop_signal.as_deref() {
        journal.record_final_stop_signal(stop_signal.to_string());
    }
    if let Some(lifecycle) = loop_state.task_lifecycle.clone() {
        journal.record_task_lifecycle(lifecycle);
    }
    if let Some(checkpoint) = loop_state.task_checkpoint.clone() {
        journal.record_task_checkpoint(checkpoint);
    }
    journal.record_delivery_consistent(delivery_consistent);
    journal.record_final_answer(final_text.to_string());
    journal.record_final_status(final_status);
    journal
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;
