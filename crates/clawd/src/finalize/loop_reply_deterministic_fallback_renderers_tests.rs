use super::*;

#[test]
fn deterministic_fallback_renderer_dispatch_records_structured_trace_when_skipped() {
    let state = test_state();
    let task = claimed_task("task-compatibility-renderer-trace");
    let mut loop_state = crate::agent_engine::LoopState::new();
    let mut finalizer_summary = None;

    let rendered = run_deterministic_fallback_renderer_registry(
        &state,
        &task,
        &mut loop_state,
        None,
        &mut finalizer_summary,
    );

    assert!(!rendered);
    let trace = loop_state
        .output_vars
        .get("finalizer.renderer_trace.scalar_placeholder_terminal_direct_answer")
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .expect("renderer trace output var");
    assert_eq!(trace["kind"], "finalizer_renderer_trace");
    assert_eq!(
        trace["renderer_key"],
        "scalar_placeholder_terminal_direct_answer"
    );
    assert_eq!(trace["shape"], "deterministic_fallback");
    assert_eq!(trace["disposition"], "skipped");
    assert_eq!(trace["failure_reason"], "not_applicable");
    assert_eq!(
        trace["evidence_refs"]
            .as_array()
            .and_then(|refs| refs.first())
            .and_then(serde_json::Value::as_str),
        Some("task:task-compatibility-renderer-trace")
    );
}
