//! Runtime ask-mode model.
//!
//! First-layer semantic routing is only `FirstLayerDecision`:
//! `clarify`, `direct_answer`, or `planner_execute`.
//! `AskMode` is the runtime projection of that decision plus execution finalization
//! style or resume behavior; it must not infer semantics from derived route labels.

use super::types::FirstLayerDecision;
#[cfg(test)]
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
    /// Direct answer selected by the first-layer gate.
    NormalizerThenChat,
    /// Clarification selected by the first-layer gate.
    NormalizerThenClarify,
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
    pub(crate) fn direct_answer() -> Self {
        Self::from_first_layer_decision(FirstLayerDecision::DirectAnswer)
    }

    pub(crate) fn clarify() -> Self {
        Self::from_first_layer_decision(FirstLayerDecision::Clarify)
    }

    #[cfg(test)]
    pub(crate) fn planner_execute_plain() -> Self {
        Self::from_first_layer_decision_with_finalize(
            FirstLayerDecision::PlannerExecute,
            ActFinalizeStyle::Plain,
        )
    }

    pub(crate) fn planner_execute_chat_wrapped() -> Self {
        Self::from_first_layer_decision_with_finalize(
            FirstLayerDecision::PlannerExecute,
            ActFinalizeStyle::ChatWrapped,
        )
    }

    pub(crate) fn from_first_layer_decision(decision: FirstLayerDecision) -> Self {
        Self::from_first_layer_decision_with_finalize(decision, ActFinalizeStyle::Plain)
    }

    pub(crate) fn from_first_layer_decision_with_finalize(
        decision: FirstLayerDecision,
        finalize_style: ActFinalizeStyle,
    ) -> Self {
        match decision {
            FirstLayerDecision::Clarify => AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenClarify,
            },
            FirstLayerDecision::DirectAnswer => AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenChat,
            },
            FirstLayerDecision::PlannerExecute => AskMode::Act {
                finalize: finalize_style,
            },
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

    /// Derived route label for logs/journals. Do not use this for semantics.
    pub(crate) fn route_label(&self) -> &'static str {
        match self {
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenChat,
            } => "Chat",
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenClarify,
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

    pub(crate) fn first_layer_decision(&self) -> FirstLayerDecision {
        match self {
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenClarify,
            } => FirstLayerDecision::Clarify,
            AskMode::ClarifyOrChat { .. } => FirstLayerDecision::DirectAnswer,
            AskMode::Act { .. } => FirstLayerDecision::PlannerExecute,
        }
    }

    #[cfg(test)]
    pub(crate) fn gate_kind(&self) -> RouteGateKind {
        self.first_layer_decision().gate_kind()
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
                entry: ChatEntryStrategy::NormalizerThenClarify,
            }
        )
    }

    /// Direct-answer entry selected by the first-layer gate.
    #[cfg(test)]
    pub(crate) fn is_normalizer_chat(&self) -> bool {
        matches!(
            self,
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenChat,
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
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenChat,
            } => "clarify_or_chat:normalizer_chat",
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenClarify,
            } => "clarify_or_chat:normalizer_clarify",
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
