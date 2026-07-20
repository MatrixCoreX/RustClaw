use super::{ok_step, synthesize_direct_fallback_would_passthrough_multiline_read_range};
use crate::agent_engine::{AgentRunContext, LoopState};

#[test]
fn multiline_read_range_passthrough_guard_ignores_visible_text_json_payload() {
    let mut loop_state = LoopState::new(1);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"status":"ok","text":"{\"action\":\"read_range\",\"content\":\"first line\\nsecond line\"}"}"#,
    ));
    let ctx = AgentRunContext {
        output_contract: Some(crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            locator_hint: "/tmp/notes.md".to_string(),
            selection: crate::OutputSelectionContract::default(),
        }),
        ..AgentRunContext::default()
    };

    assert!(
        !synthesize_direct_fallback_would_passthrough_multiline_read_range(&loop_state, Some(&ctx))
    );
}
