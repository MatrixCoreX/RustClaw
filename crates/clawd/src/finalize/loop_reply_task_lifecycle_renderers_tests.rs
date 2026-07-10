use super::*;

#[test]
fn task_lifecycle_renderer_dispatch_records_structured_traces_when_skipped() {
    let task = claimed_task("task-lifecycle-renderer-trace");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    let mut delivery_messages = Vec::new();
    let mut finalizer_summary = None;

    let rendered = run_task_lifecycle_renderer_registry(
        &task,
        &mut loop_state,
        &mut delivery_messages,
        &mut finalizer_summary,
        None,
    );

    assert!(!rendered);
    for renderer_key in [
        "agent_loop_clarify_machine_line",
        "route_clarify_machine_envelope",
        "control_machine_envelope",
    ] {
        let trace = loop_state
            .output_vars
            .get(&format!("finalizer.renderer_trace.{renderer_key}"))
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .expect("renderer trace output var");
        assert_eq!(trace["kind"], "finalizer_renderer_trace");
        assert_eq!(trace["renderer_key"], renderer_key);
        assert_eq!(trace["shape"], "task_lifecycle");
        assert_eq!(trace["disposition"], "skipped");
        assert_eq!(trace["failure_reason"], "not_applicable");
        assert_eq!(
            trace["evidence_refs"]
                .as_array()
                .and_then(|refs| refs.first())
                .and_then(serde_json::Value::as_str),
            Some("task:task-lifecycle-renderer-trace")
        );
    }

    let trace_count = loop_state
        .task_observations
        .iter()
        .filter(|observation| {
            observation.get("kind").and_then(serde_json::Value::as_str)
                == Some("finalizer_renderer_trace")
        })
        .count();
    assert_eq!(trace_count, 3);
}
