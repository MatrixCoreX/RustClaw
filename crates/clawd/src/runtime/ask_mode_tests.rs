use super::*;

#[test]
fn direct_answer_maps_to_respond_trace() {
    let m = AskMode::direct_answer();
    assert_eq!(
        m,
        AskMode::Respond {
            entry: RespondEntryStrategy::RespondTrace
        }
    );
    assert!(m.is_chat_gate());
    assert!(m.is_respond_trace());
    assert_eq!(m.route_trace_label_for_log(), "respond");
}

#[test]
fn clarify_maps_to_clarify_trace() {
    let m = AskMode::clarify();
    assert_eq!(
        m,
        AskMode::Respond {
            entry: RespondEntryStrategy::ClarifyTrace
        }
    );
    assert!(m.is_clarify_gate());
    assert!(m.is_clarify_only());
    assert!(!m.is_execute_gate());
    assert_eq!(m.route_trace_label_for_log(), "clarify");
}

#[test]
fn resume_discussion_override_keeps_chat_label() {
    let m = AskMode::direct_answer().with_resume_overrides(true, false);
    assert!(m.is_resume_discussion());
    assert_eq!(m.route_trace_label_for_log(), "respond_resume_discussion");
}

#[test]
fn resume_execution_override_wins_over_discussion() {
    let m = AskMode::direct_answer().with_resume_overrides(true, true);
    assert!(m.resume_execution());
    assert!(m.is_execute_gate());
    assert_eq!(m.route_trace_label_for_log(), "act_resume_continue");
}

#[test]
fn act_plain_maps_to_plain() {
    let m = AskMode::act_plain();
    assert_eq!(
        m,
        AskMode::Act {
            finalize: ActFinalizeStyle::Plain
        }
    );
    assert!(m.is_execute_gate());
    assert!(!m.finalize_chat_wrapped());
    assert_eq!(m.route_trace_label_for_log(), "act_plain_finalizer");
}

#[test]
fn act_with_chat_finalizer_maps_to_chat_wrapped() {
    let m = AskMode::act_with_chat_finalizer();
    assert_eq!(
        m,
        AskMode::Act {
            finalize: ActFinalizeStyle::ChatWrapped
        }
    );
    assert!(m.is_execute_gate());
    assert!(m.finalize_chat_wrapped());
    assert_eq!(m.route_trace_label_for_log(), "act_chat_finalizer");
}

#[test]
fn named_constructors_are_explicit() {
    assert_eq!(
        AskMode::direct_answer(),
        AskMode::Respond {
            entry: RespondEntryStrategy::RespondTrace
        }
    );
    assert_eq!(
        AskMode::clarify(),
        AskMode::Respond {
            entry: RespondEntryStrategy::ClarifyTrace
        }
    );
    assert_eq!(
        AskMode::act_plain(),
        AskMode::Act {
            finalize: ActFinalizeStyle::Plain
        }
    );
    assert_eq!(
        AskMode::act_with_chat_finalizer(),
        AskMode::Act {
            finalize: ActFinalizeStyle::ChatWrapped
        }
    );
}

#[test]
fn legacy_planner_execute_constructors_delegate_to_act_constructors() {
    assert_eq!(AskMode::act_plain(), AskMode::act_plain());
    assert_eq!(
        AskMode::act_with_chat_finalizer(),
        AskMode::act_with_chat_finalizer()
    );
}

#[test]
fn resume_overrides_layer_on_top_of_normalized_mode() {
    let base = AskMode::direct_answer();
    assert_eq!(
        base.clone().with_resume_overrides(false, false),
        AskMode::Respond {
            entry: RespondEntryStrategy::RespondTrace
        }
    );
    assert_eq!(
        base.clone().with_resume_overrides(true, false),
        AskMode::Respond {
            entry: RespondEntryStrategy::ResumeFollowupDiscussion
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
fn route_trace_labels_match_log_names() {
    assert_eq!(
        AskMode::direct_answer().route_trace_label_for_log(),
        "respond"
    );
    assert_eq!(AskMode::clarify().route_trace_label_for_log(), "clarify");
    assert_eq!(
        AskMode::act_plain().route_trace_label_for_log(),
        "act_plain_finalizer"
    );
    assert_eq!(
        AskMode::act_with_chat_finalizer().route_trace_label_for_log(),
        "act_chat_finalizer"
    );
}

#[test]
fn as_str_uses_stable_ids() {
    assert_eq!(AskMode::direct_answer().as_str(), "respond:trace");
    assert_eq!(AskMode::clarify().as_str(), "respond:clarify_trace");
    assert_eq!(AskMode::act_plain().as_str(), "act:plain");
    assert_eq!(
        AskMode::act_with_chat_finalizer().as_str(),
        "act:chat_wrapped"
    );
    let rd = AskMode::direct_answer().with_resume_overrides(true, false);
    assert_eq!(rd.as_str(), "respond:resume_followup_discussion");
    let re = AskMode::direct_answer().with_resume_overrides(false, true);
    assert_eq!(re.as_str(), "act:resume_continue");
}

#[test]
fn is_plain_act_only_for_plain_finalize() {
    assert!(AskMode::act_plain().is_plain_act());
    assert!(!AskMode::act_with_chat_finalizer().is_plain_act());
    assert!(!AskMode::direct_answer().is_plain_act());
    assert!(!AskMode::clarify().is_plain_act());
    let resume = AskMode::act_plain().with_resume_overrides(false, true);
    assert!(!resume.is_plain_act(), "ResumeContinue must not be plain");
    assert!(resume.is_execute_gate());
}

#[test]
fn helpers_are_disjoint_for_each_variant() {
    let cases = [
        AskMode::direct_answer(),
        AskMode::clarify(),
        AskMode::act_plain(),
        AskMode::act_with_chat_finalizer(),
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
    assert_eq!(AskMode::act_plain().gate_kind(), RouteGateKind::Execute);
    assert_eq!(
        AskMode::act_with_chat_finalizer().gate_kind(),
        RouteGateKind::Execute
    );
}

#[test]
fn route_trace_decision_for_journal_maps_to_three_decisions() {
    assert_eq!(
        AskMode::direct_answer().route_trace_decision_for_journal(),
        AskRouteTraceDecision::Respond
    );
    assert_eq!(
        AskMode::clarify().route_trace_decision_for_journal(),
        AskRouteTraceDecision::Clarify
    );
    assert_eq!(
        AskMode::act_plain().route_trace_decision_for_journal(),
        AskRouteTraceDecision::Act
    );
    assert_eq!(
        AskMode::act_with_chat_finalizer().route_trace_decision_for_journal(),
        AskRouteTraceDecision::Act
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
