//! Phase 3.1：ask 任务生命周期状态机。
//!
//! 把 ask 任务从"入队"到"出最终答覆"的过程显式建模为一个有限状态机；
//! 每次 transition 通过 [`crate::log_ask_transition`] 写入 tracing + task_journal，
//! 合法 transition 由 [`AskState::can_transition_to`] 守护、`debug_assert!` 在
//! 主路径调用点强保证。
//!
//! # 设计参考
//! [`docs/p31_ask_state_machine_proposal.md`](../../../../docs/p31_ask_state_machine_proposal.md)
//!
//! # Stage A 范围
//! 本文件只引入类型 + 合法 transition 表 + 单测；**不**修改任何现有调用面，
//! Stage B 起接 logger，Stage C 起在主路径插桩。

/// ask 任务的生命周期状态。
///
/// 设计上**只覆盖 ask 主路径**（worker / ask_pipeline / agent_engine / finalize），
/// 不细分 agent_engine 内部 plan / execute / verify 子状态——那些留给 §3.3 或后续
/// sub-PR。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AskState {
    /// task 已被 worker 拿到（claim），prepare 之前。
    Received,
    /// intent_normalizer + post_route_policy 进行中。
    Routing,
    /// 路由判定需要追问（`AskMode::is_clarify_only()`）。
    Clarifying,
    /// 路由结论是直接 LLM 直答（chat）。
    Chatting,
    /// 路由结论是恢复执行已暂停 task。
    ResumeExecuting,
    /// 路由结论是讨论已暂停 task（非执行）。
    ResumeDiscussing,
    /// 路由结论是 schedule 本地短路。
    ScheduleDirect,
    /// agent loop 执行中（含 plan / execute / verify 子循环，本轮先不细分）。
    Executing,
    /// finalize 阶段（loop_finalize / observed_output）。
    Finalizing,
    /// 答覆已生成、即将返回 worker（终态）。
    Completed,
    /// 任何阶段失败（终态）。
    Failed,
}

impl AskState {
    /// 状态的字符串标签，用于日志 `state_from=... state_to=...` 字段。
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Received => "received",
            Self::Routing => "routing",
            Self::Clarifying => "clarifying",
            Self::Chatting => "chatting",
            Self::ResumeExecuting => "resume_executing",
            Self::ResumeDiscussing => "resume_discussing",
            Self::ScheduleDirect => "schedule_direct",
            Self::Executing => "executing",
            Self::Finalizing => "finalizing",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    /// 终态：进入后不应再 transition。
    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }

    /// 该 transition 是否合法。
    ///
    /// 合法表与 [`docs/p31_ask_state_machine_proposal.md`] §3 保持一致。
    /// `Failed` 来自任何非终态都合法（错误可在任意阶段冒出）。
    /// `Executing → Executing` 合法（agent loop 多轮）。
    pub(crate) fn can_transition_to(self, next: AskState) -> bool {
        if self.is_terminal() {
            // 终态不允许再 transition
            return false;
        }
        if next == AskState::Failed {
            // 任何非终态 → Failed 都合法
            return true;
        }
        match (self, next) {
            (AskState::Received, AskState::Routing) => true,

            (AskState::Routing, AskState::Clarifying) => true,
            (AskState::Routing, AskState::Chatting) => true,
            (AskState::Routing, AskState::ResumeExecuting) => true,
            (AskState::Routing, AskState::ResumeDiscussing) => true,
            (AskState::Routing, AskState::ScheduleDirect) => true,
            (AskState::Routing, AskState::Executing) => true,

            (AskState::Clarifying, AskState::Completed) => true,

            (AskState::Chatting, AskState::Finalizing) => true,
            (AskState::Chatting, AskState::Completed) => true,

            (AskState::ResumeExecuting, AskState::Executing) => true,
            (AskState::ResumeExecuting, AskState::Finalizing) => true,
            (AskState::ResumeExecuting, AskState::Completed) => true,

            (AskState::ResumeDiscussing, AskState::Completed) => true,

            (AskState::ScheduleDirect, AskState::Completed) => true,

            // agent loop 自循环（next round）
            (AskState::Executing, AskState::Executing) => true,
            (AskState::Executing, AskState::Finalizing) => true,

            (AskState::Finalizing, AskState::Completed) => true,

            _ => false,
        }
    }
}

/// 一次 ask 状态转换的记录。
///
/// 由 logger 在每次 `transition_to(...)` 调用时构造一份，写入 tracing 日志
/// （`[ASK_STATE]` 行）以及 [`crate::task_journal::TaskJournal::transitions`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AskTransition {
    /// 来源状态。第一次 transition 之前 task 视为没有 from（用 `None` 表达），
    /// 但实际主路径插桩约定 `Received` 是显式 entry，因此实际数据里 from 总是 `Some`。
    pub(crate) from: Option<AskState>,
    /// 目标状态。
    pub(crate) to: AskState,
    /// 触发原因（自由文本，通常是分支条件名或错误摘要）。
    pub(crate) reason: String,
    /// 发生时刻（Unix 毫秒）。
    pub(crate) at_ms: u64,
    /// 当前 agent loop 轮次（`Executing → Executing` 必填，其它可选）。
    pub(crate) round_no: Option<usize>,
}

impl AskTransition {
    pub(crate) fn new(
        from: Option<AskState>,
        to: AskState,
        reason: impl Into<String>,
        at_ms: u64,
        round_no: Option<usize>,
    ) -> Self {
        Self {
            from,
            to,
            reason: reason.into(),
            at_ms,
            round_no,
        }
    }
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 记录一次 ask 状态转换：写 tracing 日志（`[ASK_STATE]` 行）并返回构造好的
/// [`AskTransition`]，由 caller 自行决定是否 `push` 到 [`crate::task_journal::TaskJournal::transitions`]。
///
/// `from = None` 表示这是 ask 任务的首次状态进入（约定为 `Received`）。
///
/// # 调用面约束（Stage C）
/// 主路径上每次 transition 调用前应满足
/// `prev.can_transition_to(next)`，由 Stage D 在调用点加 `debug_assert!` 强保证。
///
/// # 日志格式
/// ```text
/// [ASK_STATE] task_id=<id> state_from=<label|none> state_to=<label> reason=<text> round_no=<n|none>
/// ```
pub(crate) fn log_ask_transition(
    state: &crate::AppState,
    task_id: &str,
    from: Option<AskState>,
    to: AskState,
    reason: &str,
    round_no: Option<usize>,
) -> AskTransition {
    // §3.1 Stage D: debug 模式下强守合法 transition 表，release build 不触发。
    // from = None 是 ask 任务首次进入（约定 to = Received），任何 to。
    if let Some(prev) = from {
        debug_assert!(
            prev.can_transition_to(to),
            "illegal ask state transition: {:?} -> {:?} (task_id={}, reason={})",
            prev,
            to,
            task_id,
            reason
        );
    }
    let at_ms = now_unix_ms();
    let transition = AskTransition::new(from, to, reason.to_string(), at_ms, round_no);

    // §3.3 Stage 3.2：每次 transition 同步更新 AppState::ask_states 注册表，
    // 让 finalize 子层 invariant `debug_assert!(state.current_ask_state(...))`
    // 能拿到最新值；终态会自动 remove。
    state.ask_states.set(task_id, to);

    if !state.policy.routing.debug_log_ask_state {
        return transition;
    }
    let from_label = from.map(AskState::as_str).unwrap_or("none");
    let to_label = to.as_str();
    let round_label = round_no
        .map(|n| n.to_string())
        .unwrap_or_else(|| "none".to_string());
    tracing::info!(
        "{} ask_state_transition task_id={} state_from={} state_to={} reason={} round_no={} at_ms={}",
        crate::highlight_tag("ask_state"),
        task_id,
        from_label,
        to_label,
        reason,
        round_label,
        at_ms,
    );
    transition
}

#[cfg(test)]
#[path = "ask_state_tests.rs"]
mod tests;
