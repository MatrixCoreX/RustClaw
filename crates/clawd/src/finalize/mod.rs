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
//! - ⏸ Stage 2.4: observed_output 物理拆分（DEFERRED，详见 docs/p33_finalize_merge_proposal.md §4）
//! - ✅ Stage 3.1: journal builder 合并到 finalize/journal.rs
//! - ✅ Stage 3.2: AskState 强契约（AppState::current_ask_state + debug_assert）
//!
//! 详见 docs/p33_finalize_merge_proposal.md。

mod clarify;
mod helpers;
mod journal;
mod loop_reply;
mod search_path_projection;
mod task;

// === JOURNAL BUILDER（Stage 3.1）===
// finalize 子层共享的 journal 构建入口（行为零变化，仅抽离物理位置）。
pub(crate) use journal::{
    build_from_loop_state, build_terminal_from_loop_state, ensure_task_metrics,
};

// === HELPER 层（已物理位于 finalize/helpers.rs，Stage 2.1）===
// 纯函数工具（planner artifact / delivery token 分类 / FinalizerDisposition 等）
pub(crate) use clarify::{render_clarify_question, ClarifyQuestionPolicy, ClarifyRenderRequest};
pub(crate) use helpers::*;

// === TASK 层（已物理位于 finalize/task.rs，Stage 2.2）===
// 任务级编排：DB write / memory / 通知 / journal merge
pub(crate) use task::{
    answer_verifier_retry_answer_has_required_machine_evidence, finalize_ask_direct_success,
    finalize_ask_result, retry_loop_answer_after_verifier, run_direct_classifier_reply,
};

// === LOOP REPLY 层（已物理位于 finalize/loop_reply.rs，Stage 2.3）===
// 从 LoopState 选择 delivery + 构建 journal
pub(crate) use loop_reply::{
    deterministic_matrix_observed_shape_answer, direct_config_edit_observed_answer,
    finalize_loop_reply, raw_command_machine_field_delivery_satisfies_request,
    raw_command_machine_field_projection_from_journal,
    selected_tail_read_range_line_from_step_output,
};
