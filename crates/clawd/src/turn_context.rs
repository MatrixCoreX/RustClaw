use serde_json::Value;

/// Planner-owned cross-turn classification. The runtime never derives this
/// from natural-language matching before the planner loop.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnType {
    TaskRequest,
    TaskAppend,
    TaskReplace,
    TaskCorrect,
    TaskScopeUpdate,
    RunControl,
    ApprovalDecision,
    StatusQuery,
    FeedbackOrError,
    PreferenceOrMemory,
}

impl TurnType {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::TaskRequest => "task_request",
            Self::TaskAppend => "task_append",
            Self::TaskReplace => "task_replace",
            Self::TaskCorrect => "task_correct",
            Self::TaskScopeUpdate => "task_scope_update",
            Self::RunControl => "run_control",
            Self::ApprovalDecision => "approval_decision",
            Self::StatusQuery => "status_query",
            Self::FeedbackOrError => "feedback_or_error",
            Self::PreferenceOrMemory => "preference_or_memory",
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetTaskPolicy {
    ReuseActive,
    ReplaceActive,
    PauseAndQueue,
    Standalone,
}

impl TargetTaskPolicy {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ReuseActive => "reuse_active",
            Self::ReplaceActive => "replace_active",
            Self::PauseAndQueue => "pause_and_queue",
            Self::Standalone => "standalone",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TurnAnalysis {
    pub(crate) turn_type: Option<TurnType>,
    pub(crate) target_task_policy: Option<TargetTaskPolicy>,
    pub(crate) should_interrupt_active_run: bool,
    pub(crate) state_patch: Option<Value>,
    pub(crate) attachment_processing_required: bool,
}
