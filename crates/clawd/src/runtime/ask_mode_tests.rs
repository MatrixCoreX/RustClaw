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
