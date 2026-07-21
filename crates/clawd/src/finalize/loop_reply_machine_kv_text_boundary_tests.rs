use super::*;

#[test]
fn requested_machine_kv_summary_does_not_preserve_web_listing_from_visible_text_json() {
    let task = claimed_task("task-machine-kv-web-text-boundary");
    let mut loop_state = crate::agent_engine::LoopState::new();
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "web_search_extract",
        r#"{"extra":{"action":"search_extract","command_output":"fallback_value"},"text":"{\"candidates\":[{\"title\":\"Hidden Result Title\",\"source\":\"example.com\"}]}"}"#,
    ));
    let mut delivery_messages = vec!["Hidden Result Title".to_string()];
    loop_state.last_user_visible_respond = Some("Hidden Result Title".to_string());
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Strict;
    route.delivery_required = false;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "return command_output",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_ne!(delivery_messages, vec!["Hidden Result Title".to_string()]);
    assert!(
        delivery_messages
            .first()
            .is_some_and(|message| message.contains("fallback_value")),
        "{delivery_messages:?}"
    );
}
