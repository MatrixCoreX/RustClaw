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
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "tail a log slice and provide a takeaway".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "raw route but plan requested synthesis".to_string(),
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
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::RawCommandOutput,
            locator_hint: "/tmp/app.log".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let ctx = AgentRunContext {
        route_result: Some(route),
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
fn synthesize_direct_fallback_blocks_multiline_read_range_for_scalar_extraction() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r##"{"action":"read_range","excerpt":"1|# Service Notes\n2|\n3|Operators should check the app log first when requests fail, then verify the config file and database tables.","path":"/tmp/service_notes.md"}"##,
    ));
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "extract one scalar from a markdown file".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "scalar locator requires evidence".to_string(),
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
            response_shape: crate::OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: "/tmp/service_notes.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let ctx = AgentRunContext {
        route_result: Some(route),
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
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "summarize a markdown document from observed content".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "content excerpt summary".to_string(),
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
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "/tmp/service_notes.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let ctx = AgentRunContext {
        route_result: Some(route),
        ..AgentRunContext::default()
    };

    assert!(
        synthesize_direct_fallback_would_passthrough_multiline_read_range(&loop_state, Some(&ctx))
    );
}
