use super::*;

#[test]
fn final_answer_renderer_dispatch_records_structured_trace_when_skipped() {
    let task = claimed_task("task-final-answer-renderer-trace");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    let mut finalizer_summary = None;
    let mut delivery_messages = Vec::new();

    let rendered = replace_delivery_with_requested_machine_kv_summary(
        &task,
        "machine field summary",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    );

    assert!(!rendered);
    let trace = loop_state
        .output_vars
        .get("finalizer.renderer_trace.machine_kv_summary")
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .expect("renderer trace output var");
    assert_eq!(trace["kind"], "finalizer_renderer_trace");
    assert_eq!(trace["renderer_key"], "machine_kv_summary");
    assert_eq!(trace["shape"], "final_answer_shape");
    assert_eq!(trace["disposition"], "skipped");
    assert_eq!(trace["failure_reason"], "not_applicable");
    assert_eq!(
        trace["evidence_refs"]
            .as_array()
            .and_then(|refs| refs.first())
            .and_then(serde_json::Value::as_str),
        Some("task:task-final-answer-renderer-trace")
    );
}

#[test]
fn machine_kv_renderer_restores_http_status_output_path_over_file_token() {
    let task = claimed_task("task-http-download-status-output-path");
    let output_path = "/home/guagua/rustclaw/document/http/download/example.body";
    let prompt = format!(
        "Make a GET request and save the response body to {output_path}. Reply with the HTTP status and saved output_path exactly."
    );
    let mut route = free_route_result();
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.resolved_intent = format!(
        "download response body output_path={output_path}; required_machine_fields=status,output_path"
    );
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        original_user_request: Some(prompt.clone()),
        user_request: Some(prompt.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let observed_answer = format!("status=200\noutput_path={output_path}");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "http_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(format!(
                r#"{{"extra":{{"status_code":200,"output_path":"{output_path}"}}}}"#
            )),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
    loop_state
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: "step_2".to_string(),
            skill: "synthesize_answer".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(observed_answer.clone()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
    loop_state
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: "step_3".to_string(),
            skill: "respond".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(observed_answer.clone()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
    loop_state.last_publishable_synthesis_output = Some(observed_answer.clone());
    loop_state.last_user_visible_respond = Some(observed_answer);
    let mut finalizer_summary = None;
    let mut delivery_messages = vec![format!("FILE:{output_path}")];

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        &prompt,
        &mut loop_state,
        Some(&ctx),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec![format!("status=200 output_path={output_path}")]
    );
    assert_eq!(
        loop_state.last_user_visible_respond,
        delivery_messages.first().cloned()
    );
}
