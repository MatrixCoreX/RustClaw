use super::*;

#[test]
fn requested_machine_kv_summary_does_not_replace_model_text_with_path_fact_template() {
    let task = claimed_task("task-machine-kv-path-fact-marker");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"find_name","count":1,"exact":true,"patterns":["rustclaw.service"],"results":["rustclaw.service"],"root":""}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"rustclaw.service","resolved_path":"/home/guagua/rustclaw/rustclaw.service","size_bytes":769},"path":"/home/guagua/rustclaw/rustclaw.service"}],"include_missing":true}}"#,
    ));
    let mut route = free_route_result();
    route.semantic_kind = OutputSemanticKind::None;
    route.locator_hint = "rustclaw.service".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut delivery_messages = vec!["rustclaw.service".to_string()];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "rustclaw.service",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    let answer = delivery_messages.join("\n");
    assert_eq!(answer, "rustclaw.service");
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(answer.as_str())
    );
}
