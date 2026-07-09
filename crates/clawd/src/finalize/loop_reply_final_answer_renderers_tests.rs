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
