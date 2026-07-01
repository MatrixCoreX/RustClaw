//! Runtime ask-mode model.
//!
//! `AskMode` is the runtime ask-flow state used for boundary trace and
//! dispatch compatibility. Ordinary user semantics are owned by the agent loop;
//! legacy `FirstLayerDecision` values may still be emitted as log/journal hints.

use super::types::FirstLayerDecision;
use super::types::RouteGateKind;

/// Runtime ask mode after first-layer convergence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AskMode {
    /// 对用户输出文本的入口（不调技能）。
    ClarifyOrChat { entry: ChatEntryStrategy },
    /// 调技能 / agent loop 的入口。
    Act { finalize: ActFinalizeStyle },
}

/// Entry strategy for user-facing text paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChatEntryStrategy {
    /// Chat/direct-answer compatibility trace.
    #[cfg(test)]
    DirectAnswerTrace,
    /// Clarification compatibility trace.
    ClarifyTrace,
    /// Resume context and continue discussion.
    ResumeFollowupDiscussion,
}

/// Finalization style for planner execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActFinalizeStyle {
    /// Return the loop result directly.
    Plain,
    /// Wrap the loop result with the chat finalizer.
    ChatWrapped,
    /// Reuse the previous plan and continue execution.
    ResumeContinue,
}

impl AskMode {
    #[cfg(test)]
    pub(crate) fn direct_answer() -> Self {
        AskMode::ClarifyOrChat {
            entry: ChatEntryStrategy::DirectAnswerTrace,
        }
    }

    pub(crate) fn clarify() -> Self {
        AskMode::ClarifyOrChat {
            entry: ChatEntryStrategy::ClarifyTrace,
        }
    }

    pub(crate) fn planner_execute_plain() -> Self {
        AskMode::Act {
            finalize: ActFinalizeStyle::Plain,
        }
    }

    #[cfg(test)]
    pub(crate) fn planner_execute_with_chat_finalizer() -> Self {
        AskMode::Act {
            finalize: ActFinalizeStyle::ChatWrapped,
        }
    }

    /// Apply resume flags on top of an already-normalized mode.
    pub(crate) fn with_resume_overrides(
        self,
        direct_resume_discussion: bool,
        direct_resume_execution: bool,
    ) -> Self {
        if direct_resume_execution {
            return AskMode::Act {
                finalize: ActFinalizeStyle::ResumeContinue,
            };
        }
        if direct_resume_discussion {
            return AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::ResumeFollowupDiscussion,
            };
        }
        self
    }

    /// Legacy route label for logs/journals. Do not use this for semantics.
    pub(crate) fn legacy_route_label_for_trace(&self) -> &'static str {
        match self {
            #[cfg(test)]
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::DirectAnswerTrace,
            } => "Chat",
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::ClarifyTrace,
            } => "AskClarify",
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::ResumeFollowupDiscussion,
            } => "Chat",
            AskMode::Act {
                finalize: ActFinalizeStyle::Plain,
            } => "Act",
            AskMode::Act {
                finalize: ActFinalizeStyle::ChatWrapped,
            } => "ChatAct",
            AskMode::Act {
                finalize: ActFinalizeStyle::ResumeContinue,
            } => "Act",
        }
    }

    pub(crate) fn route_trace_decision_for_legacy_journal(&self) -> FirstLayerDecision {
        match self {
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::ClarifyTrace,
            } => FirstLayerDecision::Clarify,
            #[cfg(test)]
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::DirectAnswerTrace,
            } => FirstLayerDecision::DirectAnswer,
            AskMode::ClarifyOrChat { .. } => FirstLayerDecision::DirectAnswer,
            AskMode::Act { .. } => FirstLayerDecision::PlannerExecute,
        }
    }

    pub(crate) fn gate_kind(&self) -> RouteGateKind {
        match self {
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::ClarifyTrace,
            } => RouteGateKind::Clarify,
            #[cfg(test)]
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::DirectAnswerTrace,
            } => RouteGateKind::Chat,
            AskMode::ClarifyOrChat { .. } => RouteGateKind::Chat,
            AskMode::Act { .. } => RouteGateKind::Execute,
        }
    }

    #[cfg(test)]
    pub(crate) fn is_execute_gate(&self) -> bool {
        matches!(self.gate_kind(), RouteGateKind::Execute)
    }

    #[cfg(test)]
    pub(crate) fn is_chat_gate(&self) -> bool {
        matches!(self.gate_kind(), RouteGateKind::Chat)
    }

    #[cfg(test)]
    pub(crate) fn is_clarify_gate(&self) -> bool {
        matches!(self.gate_kind(), RouteGateKind::Clarify)
    }

    /// Direct planner result, without chat wrapping or resume continuation.
    pub(crate) fn is_plain_act(&self) -> bool {
        matches!(
            self,
            AskMode::Act {
                finalize: ActFinalizeStyle::Plain,
            }
        )
    }

    pub(crate) fn is_clarify_only(&self) -> bool {
        matches!(
            self,
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::ClarifyTrace,
            }
        )
    }

    /// Direct-answer compatibility trace.
    #[cfg(test)]
    pub(crate) fn is_direct_answer_trace(&self) -> bool {
        matches!(
            self,
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::DirectAnswerTrace,
            }
        )
    }

    pub(crate) fn is_resume_discussion(&self) -> bool {
        matches!(
            self,
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::ResumeFollowupDiscussion,
            }
        )
    }

    pub(crate) fn finalize_chat_wrapped(&self) -> bool {
        matches!(
            self,
            AskMode::Act {
                finalize: ActFinalizeStyle::ChatWrapped,
            }
        )
    }

    pub(crate) fn resume_execution(&self) -> bool {
        matches!(
            self,
            AskMode::Act {
                finalize: ActFinalizeStyle::ResumeContinue,
            }
        )
    }

    pub(crate) fn act_finalize_style(&self) -> Option<ActFinalizeStyle> {
        match self {
            AskMode::Act { finalize } => Some(*finalize),
            AskMode::ClarifyOrChat { .. } => None,
        }
    }

    /// Stable string id for logging / journal payloads.
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            #[cfg(test)]
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::DirectAnswerTrace,
            } => "clarify_or_chat:direct_answer_trace",
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::ClarifyTrace,
            } => "clarify_or_chat:clarify_trace",
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::ResumeFollowupDiscussion,
            } => "clarify_or_chat:resume_followup_discussion",
            AskMode::Act {
                finalize: ActFinalizeStyle::Plain,
            } => "act:plain",
            AskMode::Act {
                finalize: ActFinalizeStyle::ChatWrapped,
            } => "act:chat_wrapped",
            AskMode::Act {
                finalize: ActFinalizeStyle::ResumeContinue,
            } => "act:resume_continue",
        }
    }
}

#[cfg(test)]
#[path = "ask_mode_tests.rs"]
mod tests;
