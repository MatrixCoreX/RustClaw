use super::{
    clarify_question_from_answer, derive_clarify_candidate_targets,
    derive_clarify_state_for_ask_outcome, ClarifyMissingSlot,
};

#[test]
fn locator_question_prefers_matching_answer_text() {
    let question = clarify_question_from_answer("LOCATOR_CLARIFY_PROMPT", &[])
        .expect("question should be extracted");
    assert_eq!(question, "LOCATOR_CLARIFY_PROMPT");
}

#[test]
fn derive_locator_clarify_state_from_semantic_clarify() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "看一下那个日志最后 5 行".to_string(),
        needs_clarify: true,
        clarify_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
        route_reason: "clarify".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            requires_content_evidence: true,
            locator_kind: crate::OutputLocatorKind::Path,
            ..crate::IntentOutputContract::default()
        },
    };
    let clarify_state = derive_clarify_state_for_ask_outcome(
        "task-1",
        "看一下那个日志最后 5 行",
        &route,
        "LOCATOR_CLARIFY_PROMPT",
        &[],
        true,
        &[],
        None,
    )
    .expect("clarify state should be derived");
    assert_eq!(clarify_state.missing_slot, ClarifyMissingSlot::Locator);
    assert!(!clarify_state.delivery_required);
    assert_eq!(clarify_state.output_shape, None);
    assert_eq!(clarify_state.semantic_kind, None);
    assert_eq!(clarify_state.source_request, "看一下那个日志最后 5 行");
}

#[test]
fn derive_locator_clarify_state_preserves_non_free_output_shape() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "看一下那个日志".to_string(),
        needs_clarify: true,
        clarify_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
        route_reason: "clarify".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            locator_kind: crate::OutputLocatorKind::Path,
            ..crate::IntentOutputContract::default()
        },
    };
    let clarify_state = derive_clarify_state_for_ask_outcome(
        "task-2",
        "看一下那个日志",
        &route,
        "LOCATOR_CLARIFY_PROMPT",
        &[],
        true,
        &[],
        None,
    )
    .expect("clarify state should be derived");
    assert_eq!(
        clarify_state.output_shape.as_deref(),
        Some(crate::OutputResponseShape::OneSentence.as_str())
    );
    assert_eq!(clarify_state.semantic_kind, None);
}

#[test]
fn clarify_candidate_targets_prefer_prior_observed_entries() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: Some(crate::observed_facts::ObservedFacts {
            ordered_entries: vec![
                "README.md".to_string(),
                "deploy.md".to_string(),
                "README.md".to_string(),
            ],
            ..crate::observed_facts::ObservedFacts::default()
        }),
    };
    let candidates = derive_clarify_candidate_targets(&[], Some(&snapshot));
    assert_eq!(
        candidates,
        vec!["README.md".to_string(), "deploy.md".to_string()]
    );
}

#[test]
fn clarify_candidate_targets_preserve_observed_entry_order() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: Some(crate::observed_facts::ObservedFacts {
            ordered_entries: vec![
                "deploy.md".to_string(),
                "README.md".to_string(),
                "deploy.md".to_string(),
            ],
            ..crate::observed_facts::ObservedFacts::default()
        }),
    };
    let candidates = derive_clarify_candidate_targets(&[], Some(&snapshot));
    assert_eq!(
        candidates,
        vec!["deploy.md".to_string(), "README.md".to_string()]
    );
}

#[test]
fn clarify_candidate_targets_fall_back_to_structured_fuzzy_locator_candidates() {
    let candidates = derive_clarify_candidate_targets(
        &[
            "/tmp/a/Cargo.toml".to_string(),
            "/tmp/b/Cargo.toml".to_string(),
        ],
        None,
    );
    assert_eq!(
        candidates,
        vec![
            "/tmp/a/Cargo.toml".to_string(),
            "/tmp/b/Cargo.toml".to_string()
        ]
    );
}

#[test]
fn derive_clarify_state_seeds_candidate_targets_from_prior_session() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "把那个文件发给我".to_string(),
        needs_clarify: true,
        clarify_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
        route_reason: "clarify".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::FileToken,
            delivery_required: true,
            locator_kind: crate::OutputLocatorKind::Path,
            ..crate::IntentOutputContract::default()
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: Some(crate::observed_facts::ObservedFacts {
            ordered_entries: vec!["act_plan.log".to_string(), "clawd.log".to_string()],
            ..crate::observed_facts::ObservedFacts::default()
        }),
    };
    let clarify_state = derive_clarify_state_for_ask_outcome(
        "task-3",
        "把那个文件发给我",
        &route,
        "LOCATOR_CLARIFY_PROMPT",
        &[],
        true,
        &[],
        Some(&snapshot),
    )
    .expect("clarify state should be derived");
    assert_eq!(
        clarify_state.candidate_targets,
        vec!["act_plan.log".to_string(), "clawd.log".to_string()]
    );
}
