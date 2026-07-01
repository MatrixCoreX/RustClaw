use super::*;

fn service_status_route() -> RouteResult {
    let mut route = free_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route
}

#[test]
fn service_status_ignores_service_control_json_hidden_in_visible_text() {
    let hidden_payload = serde_json::json!({
        "service_name": "sshd",
        "post_state": "active",
        "summary": "Status: active"
    })
    .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "service_control",
        &serde_json::json!({
            "status": "ok",
            "text": hidden_payload
        })
        .to_string(),
    ));

    assert!(super::super::service_status_system_basic_info_answer(
        &service_status_route(),
        &loop_state
    )
    .is_none());
}

#[test]
fn service_status_ignores_system_info_json_hidden_in_visible_text() {
    let hidden_payload = serde_json::json!({
        "hostname": "ThinkPad-X1",
        "os": "linux"
    })
    .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        &serde_json::json!({
            "status": "ok",
            "text": hidden_payload
        })
        .to_string(),
    ));

    assert!(super::super::service_status_system_basic_info_answer(
        &service_status_route(),
        &loop_state
    )
    .is_none());
}

#[test]
fn service_status_ignores_health_check_json_hidden_in_visible_text() {
    let hidden_payload = serde_json::json!({
        "clawd_health_port_open": true,
        "clawd_process_count": 1,
        "system_health": {
            "hostname": "rustclaw-host"
        }
    })
    .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "health_check",
        &serde_json::json!({
            "status": "ok",
            "text": hidden_payload
        })
        .to_string(),
    ));

    assert!(super::super::service_status_system_basic_info_answer(
        &service_status_route(),
        &loop_state
    )
    .is_none());
}
