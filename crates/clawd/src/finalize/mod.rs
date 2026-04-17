//! Phase 3.3 — finalize 单一逻辑入口空间。
//!
//! 本模块是 `crate::finalize::*` facade（**Stage 1：仅做 re-export**）。
//! 调用方应统一使用 `crate::finalize::*` 访问 finalize 相关 API；
//! 旧路径（`crate::worker::finalize_*` / `crate::agent_engine::loop_finalize::*` /
//! `crate::agent_engine::observed_output::synthesize_*` / `crate::finalizer::*`）
//! 仍然有效，但 Stage 2 物理搬移完成后会逐步移除。
//!
//! 详见 docs/p33_finalize_merge_proposal.md。

// === TASK 层 ===
// 任务级编排：DB write / memory / 通知 / journal merge
pub(crate) use crate::worker::{
    finalize_ask_direct_success, finalize_ask_result, run_classifier_direct_reply,
    try_finalize_schedule_direct_success,
};

// === LOOP REPLY 层 ===
// 从 LoopState 选择 delivery + 构建 journal
pub(crate) use crate::agent_engine::finalize_loop_reply;

// === OBSERVED FALLBACK 层 ===
// observed-tier LLM 兜底（finalize 唯一允许的 semantic_judge LLM 入口之一）
pub(crate) use crate::agent_engine::synthesize_answer_from_observed_output;

// === HELPER 层 ===
// 纯函数工具（planner artifact / delivery token 分类 / FinalizerDisposition 等）
pub(crate) use crate::finalizer::*;
