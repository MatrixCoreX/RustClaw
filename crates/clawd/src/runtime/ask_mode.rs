//! Runtime ask-mode model.
//!
//! `AskMode` is the runtime ask-flow state used for boundary trace and
//! dispatch compatibility. Ordinary user semantics are owned by the agent loop;
//! journal hints use route-trace tokens, not legacy first-layer decisions.

use super::types::AskRouteTraceDecision;
use super::types::RouteGateKind;

/// Runtime ask mode after first-layer convergence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AskMode {
    /// User-facing response entry for resume discussion and trace-only tests.
    Respond { entry: RespondEntryStrategy },
    /// 调技能 / agent loop 的入口。
    Act { finalize: ActFinalizeStyle },
}

/// Entry strategy for user-facing response paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RespondEntryStrategy {
    /// Respond compatibility trace.
    #[cfg(test)]
    RespondTrace,
    /// Clarification compatibility trace.
    #[cfg(test)]
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
        AskMode::Respond {
            entry: RespondEntryStrategy::RespondTrace,
        }
    }

    #[cfg(test)]
    pub(crate) fn clarify() -> Self {
        AskMode::Respond {
            entry: RespondEntryStrategy::ClarifyTrace,
        }
    }

    pub(crate) fn act_plain() -> Self {
        AskMode::Act {
            finalize: ActFinalizeStyle::Plain,
        }
    }

    #[cfg(test)]
    pub(crate) fn planner_execute_plain() -> Self {
        Self::act_plain()
    }

    #[cfg(test)]
    pub(crate) fn act_with_chat_finalizer() -> Self {
        AskMode::Act {
            finalize: ActFinalizeStyle::ChatWrapped,
        }
    }

    #[cfg(test)]
    pub(crate) fn planner_execute_with_chat_finalizer() -> Self {
        Self::act_with_chat_finalizer()
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
            return AskMode::Respond {
                entry: RespondEntryStrategy::ResumeFollowupDiscussion,
            };
        }
        self
    }

    /// Human-readable route trace label for logs. Do not use this for semantics.
    pub(crate) fn route_trace_label_for_log(&self) -> &'static str {
        match self {
            #[cfg(test)]
            AskMode::Respond {
                entry: RespondEntryStrategy::RespondTrace,
            } => "respond",
            #[cfg(test)]
            AskMode::Respond {
                entry: RespondEntryStrategy::ClarifyTrace,
            } => "clarify",
            AskMode::Respond {
                entry: RespondEntryStrategy::ResumeFollowupDiscussion,
            } => "respond_resume_discussion",
            AskMode::Act {
                finalize: ActFinalizeStyle::Plain,
            } => "act_plain_finalizer",
            AskMode::Act {
                finalize: ActFinalizeStyle::ChatWrapped,
            } => "act_chat_finalizer",
            AskMode::Act {
                finalize: ActFinalizeStyle::ResumeContinue,
            } => "act_resume_continue",
        }
    }

    pub(crate) fn route_trace_decision_for_journal(&self) -> AskRouteTraceDecision {
        match self {
            #[cfg(test)]
            AskMode::Respond {
                entry: RespondEntryStrategy::ClarifyTrace,
            } => AskRouteTraceDecision::Clarify,
            #[cfg(test)]
            AskMode::Respond {
                entry: RespondEntryStrategy::RespondTrace,
            } => AskRouteTraceDecision::Respond,
            AskMode::Respond { .. } => AskRouteTraceDecision::Respond,
            AskMode::Act { .. } => AskRouteTraceDecision::Act,
        }
    }

    pub(crate) fn gate_kind(&self) -> RouteGateKind {
        match self {
            #[cfg(test)]
            AskMode::Respond {
                entry: RespondEntryStrategy::ClarifyTrace,
            } => RouteGateKind::Clarify,
            #[cfg(test)]
            AskMode::Respond {
                entry: RespondEntryStrategy::RespondTrace,
            } => RouteGateKind::Chat,
            AskMode::Respond { .. } => RouteGateKind::Chat,
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
        #[cfg(test)]
        {
            matches!(
                self,
                AskMode::Respond {
                    entry: RespondEntryStrategy::ClarifyTrace,
                }
            )
        }
        #[cfg(not(test))]
        {
            false
        }
    }

    /// Direct-answer compatibility trace.
    #[cfg(test)]
    pub(crate) fn is_direct_answer_trace(&self) -> bool {
        matches!(
            self,
            AskMode::Respond {
                entry: RespondEntryStrategy::RespondTrace,
            }
        )
    }

    pub(crate) fn is_resume_discussion(&self) -> bool {
        matches!(
            self,
            AskMode::Respond {
                entry: RespondEntryStrategy::ResumeFollowupDiscussion,
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
            AskMode::Respond { .. } => None,
        }
    }

    /// Stable string id for logging / journal payloads.
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            #[cfg(test)]
            AskMode::Respond {
                entry: RespondEntryStrategy::RespondTrace,
            } => "respond:trace",
            #[cfg(test)]
            AskMode::Respond {
                entry: RespondEntryStrategy::ClarifyTrace,
            } => "respond:clarify_trace",
            AskMode::Respond {
                entry: RespondEntryStrategy::ResumeFollowupDiscussion,
            } => "respond:resume_followup_discussion",
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
