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
mod tests {
    use super::*;

    #[test]
    fn direct_answer_maps_to_normalizer_chat() {
        let m = AskMode::direct_answer();
        assert_eq!(
            m,
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenChat
            }
        );
        assert!(m.is_chat_gate());
        assert!(m.is_normalizer_chat());
        assert_eq!(m.route_label(), "Chat");
    }

    #[test]
    fn clarify_maps_to_normalizer_clarify() {
        let m = AskMode::clarify();
        assert_eq!(
            m,
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenClarify
            }
        );
        assert!(m.is_clarify_gate());
        assert!(m.is_clarify_only());
        assert!(!m.is_execute_gate());
        assert_eq!(m.route_label(), "AskClarify");
    }

    #[test]
    fn resume_discussion_override_keeps_chat_label() {
        let m = AskMode::direct_answer().with_resume_overrides(true, false);
        assert!(m.is_resume_discussion());
        assert_eq!(m.route_label(), "Chat");
    }

    #[test]
    fn resume_execution_override_wins_over_discussion() {
        let m = AskMode::direct_answer().with_resume_overrides(true, true);
        assert!(m.resume_execution());
        assert!(m.is_execute_gate());
        assert_eq!(m.route_label(), "Act");
    }

    #[test]
    fn planner_execute_plain_maps_to_plain() {
        let m = AskMode::planner_execute_plain();
        assert_eq!(
            m,
            AskMode::Act {
                finalize: ActFinalizeStyle::Plain
            }
        );
        assert!(m.is_execute_gate());
        assert!(!m.finalize_chat_wrapped());
        assert_eq!(m.route_label(), "Act");
    }

    #[test]
    fn planner_execute_chat_wrapped_maps_to_chat_wrapped() {
        let m = AskMode::planner_execute_chat_wrapped();
        assert_eq!(
            m,
            AskMode::Act {
                finalize: ActFinalizeStyle::ChatWrapped
            }
        );
        assert!(m.is_execute_gate());
        assert!(m.finalize_chat_wrapped());
        assert_eq!(m.route_label(), "ChatAct");
    }

    #[test]
    fn named_constructors_are_explicit() {
        assert_eq!(
            AskMode::direct_answer(),
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenChat
            }
        );
        assert_eq!(
            AskMode::clarify(),
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenClarify
            }
        );
        assert_eq!(
            AskMode::planner_execute_plain(),
            AskMode::Act {
                finalize: ActFinalizeStyle::Plain
            }
        );
        assert_eq!(
            AskMode::planner_execute_chat_wrapped(),
            AskMode::Act {
                finalize: ActFinalizeStyle::ChatWrapped
            }
        );
    }

    #[test]
    fn from_first_layer_decision_uses_explicit_finalize_style_only_for_execution() {
        assert_eq!(
            AskMode::from_first_layer_decision_with_finalize(
                FirstLayerDecision::Clarify,
                ActFinalizeStyle::ChatWrapped,
            ),
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenClarify
            }
        );
        assert_eq!(
            AskMode::from_first_layer_decision_with_finalize(
                FirstLayerDecision::DirectAnswer,
                ActFinalizeStyle::Plain,
            ),
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenChat
            }
        );
        assert_eq!(
            AskMode::from_first_layer_decision_with_finalize(
                FirstLayerDecision::PlannerExecute,
                ActFinalizeStyle::ChatWrapped,
            ),
            AskMode::Act {
                finalize: ActFinalizeStyle::ChatWrapped
            }
        );
    }

    #[test]
    fn resume_overrides_layer_on_top_of_normalized_mode() {
        let base = AskMode::from_first_layer_decision(FirstLayerDecision::DirectAnswer);
        assert_eq!(
            base.clone().with_resume_overrides(false, false),
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenChat
            }
        );
        assert_eq!(
            base.clone().with_resume_overrides(true, false),
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::ResumeFollowupDiscussion
            }
        );
        assert_eq!(
            base.with_resume_overrides(true, true),
            AskMode::Act {
                finalize: ActFinalizeStyle::ResumeContinue
            }
        );
    }

    #[test]
    fn derived_route_labels_match_legacy_log_names() {
        assert_eq!(AskMode::direct_answer().route_label(), "Chat");
        assert_eq!(AskMode::clarify().route_label(), "AskClarify");
        assert_eq!(AskMode::planner_execute_plain().route_label(), "Act");
        assert_eq!(
            AskMode::planner_execute_chat_wrapped().route_label(),
            "ChatAct"
        );
    }

    #[test]
    fn as_str_uses_stable_ids() {
        assert_eq!(
            AskMode::direct_answer().as_str(),
            "clarify_or_chat:normalizer_chat"
        );
        assert_eq!(
            AskMode::clarify().as_str(),
            "clarify_or_chat:normalizer_clarify"
        );
        assert_eq!(AskMode::planner_execute_plain().as_str(), "act:plain");
        assert_eq!(
            AskMode::planner_execute_chat_wrapped().as_str(),
            "act:chat_wrapped"
        );
        let rd = AskMode::direct_answer().with_resume_overrides(true, false);
        assert_eq!(rd.as_str(), "clarify_or_chat:resume_followup_discussion");
        let re = AskMode::direct_answer().with_resume_overrides(false, true);
        assert_eq!(re.as_str(), "act:resume_continue");
    }

    #[test]
    fn is_plain_act_only_for_plain_finalize() {
        assert!(AskMode::planner_execute_plain().is_plain_act());
        assert!(!AskMode::planner_execute_chat_wrapped().is_plain_act());
        assert!(!AskMode::direct_answer().is_plain_act());
        assert!(!AskMode::clarify().is_plain_act());
        let resume = AskMode::planner_execute_plain().with_resume_overrides(false, true);
        assert!(!resume.is_plain_act(), "ResumeContinue must not be plain");
        assert!(resume.is_execute_gate());
    }

    #[test]
    fn helpers_are_disjoint_for_each_variant() {
        let cases = [
            AskMode::direct_answer(),
            AskMode::clarify(),
            AskMode::planner_execute_plain(),
            AskMode::planner_execute_chat_wrapped(),
            AskMode::direct_answer().with_resume_overrides(true, false),
            AskMode::direct_answer().with_resume_overrides(false, true),
        ];
        for m in &cases {
            let mut hits = 0;
            if m.is_clarify_only() {
                hits += 1;
            }
            if m.is_resume_discussion() {
                hits += 1;
            }
            if m.finalize_chat_wrapped() {
                hits += 1;
            }
            if m.resume_execution() {
                hits += 1;
            }
            assert!(hits <= 1, "predicate overlap on {m:?} (hits={hits})");
        }
    }

    #[test]
    fn gate_kind_maps_to_three_gates() {
        assert_eq!(AskMode::direct_answer().gate_kind(), RouteGateKind::Chat);
        assert_eq!(AskMode::clarify().gate_kind(), RouteGateKind::Clarify);
        assert_eq!(
            AskMode::planner_execute_plain().gate_kind(),
            RouteGateKind::Execute
        );
        assert_eq!(
            AskMode::planner_execute_chat_wrapped().gate_kind(),
            RouteGateKind::Execute
        );
    }

    #[test]
    fn first_layer_decision_maps_to_three_decisions() {
        assert_eq!(
            AskMode::direct_answer().first_layer_decision(),
            FirstLayerDecision::DirectAnswer
        );
        assert_eq!(
            AskMode::clarify().first_layer_decision(),
            FirstLayerDecision::Clarify
        );
        assert_eq!(
            AskMode::planner_execute_plain().first_layer_decision(),
            FirstLayerDecision::PlannerExecute
        );
        assert_eq!(
            AskMode::planner_execute_chat_wrapped().first_layer_decision(),
            FirstLayerDecision::PlannerExecute
        );
    }

    #[test]
    fn resume_shortcuts_keep_expected_gate_kinds() {
        assert_eq!(
            AskMode::direct_answer()
                .with_resume_overrides(true, false)
                .gate_kind(),
            RouteGateKind::Chat
        );
        assert_eq!(
            AskMode::direct_answer()
                .with_resume_overrides(false, true)
                .gate_kind(),
            RouteGateKind::Execute
        );
    }
}
