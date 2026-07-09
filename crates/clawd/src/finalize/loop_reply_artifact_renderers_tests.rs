use super::*;

#[test]
fn artifact_renderer_dispatch_records_structured_trace_when_skipped() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);

    let rendered = normalize_file_token_delivery_from_observed_paths(&state, &mut loop_state, None);

    assert!(!rendered);
    let trace = loop_state
        .output_vars
        .get("finalizer.renderer_trace.file_token_delivery")
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .expect("renderer trace output var");
    assert_eq!(trace["kind"], "finalizer_renderer_trace");
    assert_eq!(trace["renderer_key"], "file_token_delivery");
    assert_eq!(trace["shape"], "artifact_delivery");
    assert_eq!(trace["disposition"], "skipped");
    assert_eq!(trace["failure_reason"], "not_applicable");
    assert_eq!(
        trace["evidence_refs"]
            .as_array()
            .and_then(|refs| refs.first())
            .and_then(serde_json::Value::as_str),
        Some("delivery_messages")
    );
}
