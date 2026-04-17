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

#![allow(dead_code)]

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
    /// 路由结论是直接 LLM 直答（chat / classifier_direct）。
    Chatting,
    /// 路由结论是恢复执行已暂停 task。
    ResumeExecuting,
    /// 路由结论是讨论已暂停 task（非执行）。
    ResumeDiscussing,
    /// 路由结论是 schedule deterministic 短路。
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

#[cfg(test)]
mod tests {
    use super::*;

    fn all_states() -> [AskState; 11] {
        [
            AskState::Received,
            AskState::Routing,
            AskState::Clarifying,
            AskState::Chatting,
            AskState::ResumeExecuting,
            AskState::ResumeDiscussing,
            AskState::ScheduleDirect,
            AskState::Executing,
            AskState::Finalizing,
            AskState::Completed,
            AskState::Failed,
        ]
    }

    #[test]
    fn as_str_is_stable_and_unique() {
        let labels: Vec<&'static str> = all_states().iter().map(|s| s.as_str()).collect();
        let unique: std::collections::HashSet<&'static str> = labels.iter().copied().collect();
        assert_eq!(labels.len(), unique.len(), "as_str labels must be unique");
        assert!(
            labels.iter().all(|s| !s.is_empty()),
            "as_str must not be empty"
        );
    }

    #[test]
    fn terminal_states_cannot_transition() {
        for next in all_states() {
            assert!(
                !AskState::Completed.can_transition_to(next),
                "Completed must not transition to {:?}",
                next
            );
            assert!(
                !AskState::Failed.can_transition_to(next),
                "Failed must not transition to {:?}",
                next
            );
        }
    }

    #[test]
    fn any_non_terminal_can_fail() {
        for s in all_states() {
            if !s.is_terminal() {
                assert!(
                    s.can_transition_to(AskState::Failed),
                    "{:?} should be allowed to fail",
                    s
                );
            }
        }
    }

    #[test]
    fn happy_path_act_is_legal() {
        let path = [
            AskState::Received,
            AskState::Routing,
            AskState::Executing,
            AskState::Executing,
            AskState::Finalizing,
            AskState::Completed,
        ];
        for w in path.windows(2) {
            assert!(
                w[0].can_transition_to(w[1]),
                "{:?} → {:?} must be legal",
                w[0],
                w[1]
            );
        }
    }

    #[test]
    fn happy_path_chat_is_legal() {
        for next in [AskState::Finalizing, AskState::Completed] {
            assert!(AskState::Chatting.can_transition_to(next));
        }
    }

    #[test]
    fn happy_path_clarify_is_legal() {
        assert!(AskState::Received.can_transition_to(AskState::Routing));
        assert!(AskState::Routing.can_transition_to(AskState::Clarifying));
        assert!(AskState::Clarifying.can_transition_to(AskState::Completed));
    }

    #[test]
    fn resume_execution_path_is_legal() {
        assert!(AskState::Routing.can_transition_to(AskState::ResumeExecuting));
        assert!(AskState::ResumeExecuting.can_transition_to(AskState::Executing));
        assert!(AskState::ResumeExecuting.can_transition_to(AskState::Finalizing));
        assert!(AskState::ResumeExecuting.can_transition_to(AskState::Completed));
    }

    #[test]
    fn schedule_direct_path_is_legal() {
        assert!(AskState::Routing.can_transition_to(AskState::ScheduleDirect));
        assert!(AskState::ScheduleDirect.can_transition_to(AskState::Completed));
        assert!(!AskState::ScheduleDirect.can_transition_to(AskState::Finalizing));
    }

    #[test]
    fn illegal_transitions_are_rejected() {
        // 不能跳过 Routing 直接 Executing
        assert!(!AskState::Received.can_transition_to(AskState::Executing));
        // 不能从 Routing 直接到 Completed（必须经过分支状态）
        assert!(!AskState::Routing.can_transition_to(AskState::Completed));
        // Chatting 不能进 Executing
        assert!(!AskState::Chatting.can_transition_to(AskState::Executing));
        // Clarifying 不能进 Finalizing
        assert!(!AskState::Clarifying.can_transition_to(AskState::Finalizing));
        // Executing 不能直接到 Completed（必须经过 Finalizing）
        assert!(!AskState::Executing.can_transition_to(AskState::Completed));
        // 不能自循环（除 Executing 外）
        assert!(!AskState::Routing.can_transition_to(AskState::Routing));
        assert!(!AskState::Finalizing.can_transition_to(AskState::Finalizing));
    }

    #[test]
    fn ask_transition_records_metadata() {
        let t = AskTransition::new(
            Some(AskState::Routing),
            AskState::Executing,
            "act_branch",
            1_700_000_000_000,
            None,
        );
        assert_eq!(t.from, Some(AskState::Routing));
        assert_eq!(t.to, AskState::Executing);
        assert_eq!(t.reason, "act_branch");
        assert_eq!(t.at_ms, 1_700_000_000_000);
        assert_eq!(t.round_no, None);
    }

    #[test]
    fn executing_self_loop_is_legal() {
        assert!(AskState::Executing.can_transition_to(AskState::Executing));
    }
}
