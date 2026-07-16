use super::{
    archive_database_aggregate_structured_answer, ok_step,
    synthesize_direct_fallback_would_passthrough_multiline_read_range,
};
use crate::agent_engine::{AgentRunContext, LoopState};

#[test]
fn aggregate_structured_answer_ignores_visible_text_json_payloads() {
    let mut loop_state = LoopState::new(1);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "archive_basic",
        r#"{"status":"ok","text":"{\"action\":\"list\",\"entries\":[{\"name\":\"notes.md\"}]}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "archive_basic",
        r#"{"status":"ok","text":"{\"action\":\"read\",\"member\":\"notes.md\",\"content\":\"hello\"}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "db_basic",
        r#"{"status":"ok","text":"{\"action\":\"list_tables\",\"rows\":[{\"name\":\"events\"}]}"}"#,
    ));

    assert!(archive_database_aggregate_structured_answer(&loop_state).is_none());
}

#[test]
fn multiline_read_range_passthrough_guard_ignores_visible_text_json_payload() {
    let mut loop_state = LoopState::new(1);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"status":"ok","text":"{\"action\":\"read_range\",\"content\":\"first line\\nsecond line\"}"}"#,
    ));
    let ctx = AgentRunContext {
        route_result: Some(crate::RouteResult {
            ask_mode: crate::AskMode::act_plain(),
            resolved_intent: "summarize the selected file".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "content excerpt summary".to_string(),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
                locator_hint: "/tmp/notes.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        }),
        ..AgentRunContext::default()
    };

    assert!(
        !synthesize_direct_fallback_would_passthrough_multiline_read_range(&loop_state, Some(&ctx))
    );
}
