use super::renderer_registry::{FinalizerRendererShapeClass, FINALIZER_RENDERER_REGISTRY};
use std::collections::BTreeSet;

#[test]
fn finalizer_renderer_registry_keys_are_unique_machine_tokens() {
    let mut seen = BTreeSet::new();
    for renderer in FINALIZER_RENDERER_REGISTRY {
        assert!(
            seen.insert(renderer.key),
            "duplicate finalizer renderer key {}",
            renderer.key
        );
        assert!(
            renderer
                .key
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'),
            "{} must be a machine token",
            renderer.key
        );
        assert!(!renderer.owner_module.trim().is_empty());
        assert!(!renderer.entrypoint.trim().is_empty());
        assert!(!renderer.summary_contract.trim().is_empty());
    }
}

#[test]
fn finalizer_renderer_registry_covers_initial_shape_classes() {
    let classes = FINALIZER_RENDERER_REGISTRY
        .iter()
        .map(|renderer| renderer.shape_class.as_str())
        .collect::<BTreeSet<_>>();

    for required in [
        FinalizerRendererShapeClass::CapabilityResult,
        FinalizerRendererShapeClass::FinalAnswerShape,
        FinalizerRendererShapeClass::ArtifactDelivery,
        FinalizerRendererShapeClass::TaskLifecycle,
        FinalizerRendererShapeClass::DeterministicFallback,
    ] {
        assert!(
            classes.contains(required.as_str()),
            "missing finalizer renderer shape class {}",
            required.as_str()
        );
    }
}

#[test]
fn task_lifecycle_renderer_order_matches_dispatch_order() {
    let keys = super::renderer_registry::renderers_for_shape_class(
        FinalizerRendererShapeClass::TaskLifecycle,
    )
    .map(|renderer| renderer.key)
    .collect::<Vec<_>>();

    assert_eq!(
        keys,
        vec![
            "agent_loop_clarify_machine_line",
            "route_clarify_machine_envelope",
            "control_machine_envelope"
        ]
    );
}

#[test]
fn renderer_trace_records_machine_fields_and_journal_observation() {
    let renderer = FINALIZER_RENDERER_REGISTRY
        .iter()
        .find(|renderer| renderer.key == "control_machine_envelope")
        .expect("control renderer descriptor");
    let mut loop_state = crate::agent_engine::LoopState::new(2);

    super::renderer_registry::record_renderer_trace(
        &mut loop_state,
        renderer,
        true,
        vec!["executed_step_results[0]".to_string()],
        None,
    );

    let trace = loop_state
        .output_vars
        .get("finalizer.renderer_trace.control_machine_envelope")
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .expect("renderer trace output var");
    assert_eq!(trace["kind"], "finalizer_renderer_trace");
    assert_eq!(trace["renderer_key"], "control_machine_envelope");
    assert_eq!(trace["shape"], "task_lifecycle");
    assert_eq!(trace["summary_contract"], "control_intent");
    assert_eq!(trace["disposition"], "rendered");
    assert!(trace["failure_reason"].is_null());
    assert_eq!(
        trace["evidence_refs"]
            .as_array()
            .and_then(|refs| refs.first())
            .and_then(serde_json::Value::as_str),
        Some("executed_step_results[0]")
    );
    assert!(loop_state.task_observations.iter().any(|observation| {
        observation.get("kind").and_then(serde_json::Value::as_str)
            == Some("finalizer_renderer_trace")
    }));
}
