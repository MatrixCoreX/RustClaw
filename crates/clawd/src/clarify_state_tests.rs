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
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::Path,
        ..crate::IntentOutputContract::default()
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
fn derive_user_input_clarify_state_for_freeform_waiting_request() {
    let route = crate::IntentOutputContract::default();
    let clarify_state = derive_clarify_state_for_ask_outcome(
        "task-freeform",
        "Help me draft a proposal",
        &route,
        "QUESTION",
        &[],
        true,
        &[],
        None,
    )
    .expect("clarify state should be derived");
    assert_eq!(clarify_state.missing_slot, ClarifyMissingSlot::UserInput);
    assert!(!clarify_state.delivery_required);
    assert_eq!(clarify_state.output_shape, None);
    assert_eq!(clarify_state.semantic_kind, None);
}

#[test]
fn derive_locator_clarify_state_preserves_non_free_output_shape() {
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::Path,
        ..crate::IntentOutputContract::default()
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
fn derive_locator_clarify_state_uses_source_request_without_route_trace_marker() {
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::Path,
        ..crate::IntentOutputContract::default()
    };
    let clarify_state = derive_clarify_state_for_ask_outcome(
        "task-structured",
        "读一下那个文件里的名字字段，只输出值",
        &route,
        "LOCATOR_CLARIFY_PROMPT",
        &[],
        true,
        &[],
        None,
    )
    .expect("clarify state should be derived");

    assert!(clarify_state
        .source_request
        .starts_with("读一下那个文件里的名字字段，只输出值"));
    assert!(!clarify_state.source_request.contains("[RESOLVED_INTENT]"));
}

#[test]
fn derive_locator_clarify_state_preserves_structured_field_selector_token() {
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::Path,
        selection: crate::OutputSelectionContract {
            structured_field_selector: Some("name".to_string()),
            ..crate::OutputSelectionContract::default()
        },
        ..crate::IntentOutputContract::default()
    };
    let clarify_state = derive_clarify_state_for_ask_outcome(
        "task-structured-selector",
        "Read that file field value only",
        &route,
        "LOCATOR_CLARIFY_PROMPT",
        &[],
        true,
        &[],
        None,
    )
    .expect("clarify state should be derived");

    assert!(clarify_state
        .source_request
        .split_whitespace()
        .any(|part| part == "structured_field_selector=name"));
    assert_eq!(
        super::structured_field_selector_token_from_text(&clarify_state.source_request).as_deref(),
        Some("name")
    );
}

#[test]
fn derive_locator_clarify_state_marks_non_content_probe_as_existence_contract() {
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Strict,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::None,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: String::new(),
        selection: crate::OutputSelectionContract::default(),
    };

    let clarify_state = derive_clarify_state_for_ask_outcome(
        "task-exists",
        "Check whether the referenced script exists.",
        &route,
        "LOCATOR_CLARIFY_PROMPT",
        &[],
        true,
        &[],
        None,
    )
    .expect("clarify state should be derived");

    assert_eq!(
        clarify_state.semantic_kind.as_deref(),
        Some(crate::OutputSemanticKind::ExistenceWithPath.as_str())
    );
    assert_eq!(
        clarify_state.output_shape.as_deref(),
        Some(crate::OutputResponseShape::Strict.as_str())
    );
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
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::FileToken,
        delivery_required: true,
        locator_kind: crate::OutputLocatorKind::Path,
        ..crate::IntentOutputContract::default()
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
