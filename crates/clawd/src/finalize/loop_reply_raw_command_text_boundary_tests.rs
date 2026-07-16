use super::*;

#[test]
fn raw_command_projection_ignores_read_range_json_hidden_in_visible_text() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    let hidden_payload = serde_json::json!({
        "action": "read_range",
        "mode": "tail",
        "requested_n": 2,
        "path": "/tmp/clawd-dev.log",
        "excerpt": "98|first observed line\n99|second observed line"
    })
    .to_string();
    let read_range_output = serde_json::json!({
        "status": "ok",
        "text": hidden_payload
    })
    .to_string();
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "fs_basic", &read_range_output));
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;

    assert!(direct_raw_command_output_projection(&state, &route, &loop_state).is_none());
}
