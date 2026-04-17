//! Phase 3.3 — finalize 单一逻辑入口空间。
//!
//! 本模块是 `crate::finalize::*` facade。调用方应统一使用 `crate::finalize::*`
//! 访问 finalize 相关 API；旧路径在 Stage 2 各分子任务完成后已逐步移除。
//!
//! Stage 进度：
//! - ✅ Stage 1: facade 建立 + 调用面统一
//! - ✅ Stage 2.1: finalizer.rs → finalize/helpers.rs
//! - ✅ Stage 2.2: worker/ask_finalize.rs → finalize/task.rs
//! - ✅ Stage 2.3: agent_engine/loop_finalize.rs → finalize/loop_reply.rs
//! - ⏳ Stage 2.4: agent_engine/observed_output.rs 中 finalize 部分 → finalize/observed.rs
//! - ⏳ Stage 3: journal builder 合并 + AskState 强契约
//!
//! 详见 docs/p33_finalize_merge_proposal.md。

mod helpers;
mod loop_reply;
mod task;

// === HELPER 层（已物理位于 finalize/helpers.rs，Stage 2.1）===
// 纯函数工具（planner artifact / delivery token 分类 / FinalizerDisposition 等）
pub(crate) use helpers::*;

// === TASK 层（已物理位于 finalize/task.rs，Stage 2.2）===
// 任务级编排：DB write / memory / 通知 / journal merge
pub(crate) use task::{
    finalize_ask_direct_success, finalize_ask_result, run_classifier_direct_reply,
    try_finalize_schedule_direct_success,
};

// === LOOP REPLY 层（已物理位于 finalize/loop_reply.rs，Stage 2.3）===
// 从 LoopState 选择 delivery + 构建 journal
pub(crate) use loop_reply::finalize_loop_reply;

// === OBSERVED FALLBACK 层（物理仍在 agent_engine/observed_output.rs，Stage 2.4 后搬移）===
// observed-tier LLM 兜底（finalize 唯一允许的 semantic_judge LLM 入口之一）
pub(crate) use crate::agent_engine::synthesize_answer_from_observed_output;
