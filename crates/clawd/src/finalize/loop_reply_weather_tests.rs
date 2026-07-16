use super::*;

#[tokio::test]
async fn finalize_loop_reply_projects_weather_structured_fields() {
    let state = test_state();
    let task = claimed_task("task-weather-query-fields");
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::WeatherQuery;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.resolved_intent = "capability_ref=weather.current location=Beijing".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let weather_output = serde_json::json!({
        "extra": {
            "action": "query",
            "locale": "zh-CN",
            "location": "北京",
            "resolved_location": "Beijing, Beijing Municipality, China",
            "temperature": 25.2,
            "weather_code": "多云",
            "weather_code_raw": 3
        },
        "text": "Beijing, Beijing Municipality, China 白天：多云，气温 25.2°C。"
    })
    .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "weather", &weather_output));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        "location: Beijing, Beijing Municipality, China\ntemperature: 25.2\nweather_code: 多云",
    ));
    loop_state.last_publishable_synthesis_output = Some(
        "location: Beijing, Beijing Municipality, China\ntemperature: 25.2\nweather_code: 多云"
            .to_string(),
    );
    loop_state
        .delivery_messages
        .push("temperature=25.2".to_string());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "weather field projection",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("weather query should finalize");

    assert!(!reply.should_fail_task, "reply: {}", reply.text);
    assert_eq!(
        reply.text.trim(),
        "location=北京\ntemperature=25.2\nweather_code=多云"
    );
    assert_eq!(reply.messages, vec![reply.text.clone()]);
}
