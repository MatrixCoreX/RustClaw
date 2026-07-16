use super::*;

#[test]
fn observed_markdown_heading_scalar_ignores_markdown_json_hidden_in_visible_text() {
    let hidden_payload = serde_json::json!({
        "action": "read_range",
        "excerpt": "1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly.",
        "path": "release_checklist.md"
    })
    .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        &serde_json::json!({
            "status": "ok",
            "text": hidden_payload
        })
        .to_string(),
    ));
    let mut route = scalar_route_result();
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "release_checklist.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut delivery =
        vec!["# Release Checklist\n\n1. Verify configuration loads correctly.".to_string()];
    let mut summary = None;

    assert!(!replace_delivery_with_observed_markdown_heading_scalar(
        "task",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut summary,
    ));
    assert_eq!(
        delivery,
        vec!["# Release Checklist\n\n1. Verify configuration loads correctly.".to_string()]
    );
    assert!(summary.is_none());
}
