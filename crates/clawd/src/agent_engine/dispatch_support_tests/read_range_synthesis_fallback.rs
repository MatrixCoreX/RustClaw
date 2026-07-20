use super::*;
use crate::{AgentAction, PlanKind, PlanResult};

#[test]
fn synthesize_direct_fallback_blocks_multiline_raw_read_range_when_plan_requests_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","mode":"tail","excerpt":"1|WARN cache miss ratio above baseline\n2|ERROR provider timeout\n3|INFO provider retry succeeded","path":"/tmp/app.log"}"#,
    ));
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Strict,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::Path,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::RawCommandOutput,
        locator_hint: "/tmp/app.log".to_string(),
        selection: crate::OutputSelectionContract::default(),
    };
    let ctx = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert!(
        !synthesize_direct_fallback_would_passthrough_multiline_read_range(&loop_state, Some(&ctx))
    );

    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: String::new(),
            execution_recipe_summary: None,
            plan_result: Some(PlanResult {
                goal: String::new(),
                missing_slots: Vec::new(),
                needs_confirmation: false,
                output_contract: None,
                steps: vec![crate::plan_step_from_agent_action(
                    &AgentAction::SynthesizeAnswer {
                        evidence_refs: vec!["last_output".to_string()],
                    },
                    "step_2".to_string(),
                    Vec::new(),
                    String::new(),
                )],
                planner_notes: String::new(),
                plan_kind: PlanKind::Single,
                raw_plan_text: String::new(),
            }),
            verify_result: None,
        });

    assert!(
        synthesize_direct_fallback_would_passthrough_multiline_read_range(&loop_state, Some(&ctx))
    );
}

#[test]
fn synthesize_direct_fallback_allows_explicit_scalar_extraction() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r##"{"action":"read_range","excerpt":"1|# Service Notes\n2|\n3|Operators should check the app log first when requests fail, then verify the config file and database tables.","path":"/tmp/service_notes.md"}"##,
    ));
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Scalar,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::Path,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: "/tmp/service_notes.md".to_string(),
        selection: crate::OutputSelectionContract {
            structured_field_selector: Some("value".to_string()),
            ..Default::default()
        },
    };
    let ctx = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert!(synthesize_route_allows_direct_fallback(Some(&ctx)));
    assert!(
        synthesize_direct_fallback_would_passthrough_multiline_read_range(&loop_state, Some(&ctx))
    );
}

#[test]
fn synthesize_direct_fallback_blocks_nested_extra_multiline_read_range() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r##"{"extra":{"action":"read_range","excerpt":"1|# Service Notes\n2|\n3|Operators should check the app log first when requests fail, then verify the config file and database tables.","path":"/tmp/service_notes.md"},"text":"{\"action\":\"read_range\",\"excerpt\":\"1|# Service Notes\\n2|\\n3|Operators should check the app log first when requests fail, then verify the config file and database tables.\",\"path\":\"/tmp/service_notes.md\"}"}"##,
    ));
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Strict,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::Path,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: "/tmp/service_notes.md".to_string(),
        selection: crate::OutputSelectionContract::default(),
    };
    let ctx = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert!(
        synthesize_direct_fallback_would_passthrough_multiline_read_range(&loop_state, Some(&ctx))
    );
}

#[test]
fn bounded_read_range_direct_answer_allows_unclassified_free_path_route() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r##"{"extra":{"action":"read_range","mode":"head","requested_n":4,"start_line":1,"end_line":4,"excerpt":"1|# Device Local Fixture\n2|\n3|This directory contains stable local files for RustClaw NL regression tests.\n4|","path":"/tmp/README.md"}}"##,
    ));
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Free,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::Path,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: "/tmp/README.md".to_string(),
        selection: crate::OutputSelectionContract::default(),
    };
    let ctx = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        synthesize_bounded_read_range_direct_answer(&loop_state, Some(&ctx)).as_deref(),
        Some(
            "# Device Local Fixture\n\nThis directory contains stable local files for RustClaw NL regression tests."
        )
    );
}

#[test]
fn bounded_read_range_direct_answer_blocks_summary_and_scalar_routes() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r##"{"extra":{"action":"read_range","mode":"head","requested_n":3,"excerpt":"1|# Service Notes\n2|\n3|Operators should check logs first.","path":"/tmp/service_notes.md"}}"##,
    ));
    let base_route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::Path,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: "/tmp/service_notes.md".to_string(),
        selection: crate::OutputSelectionContract::default(),
    };
    let summary_ctx = AgentRunContext {
        output_contract: Some(base_route.clone()),
        ..AgentRunContext::default()
    };
    assert!(synthesize_bounded_read_range_direct_answer(&loop_state, Some(&summary_ctx)).is_none());

    let mut generic_evidence_route = base_route.clone();
    generic_evidence_route.semantic_kind = crate::OutputSemanticKind::None;
    let generic_evidence_ctx = AgentRunContext {
        output_contract: Some(generic_evidence_route),
        ..AgentRunContext::default()
    };
    assert!(
        synthesize_bounded_read_range_direct_answer(&loop_state, Some(&generic_evidence_ctx))
            .is_none()
    );

    let mut scalar_route = base_route;
    scalar_route.semantic_kind = crate::OutputSemanticKind::None;
    scalar_route.response_shape = crate::OutputResponseShape::Scalar;
    let scalar_ctx = AgentRunContext {
        output_contract: Some(scalar_route.clone()),
        ..AgentRunContext::default()
    };
    assert!(synthesize_bounded_read_range_direct_answer(&loop_state, Some(&scalar_ctx)).is_none());
}
