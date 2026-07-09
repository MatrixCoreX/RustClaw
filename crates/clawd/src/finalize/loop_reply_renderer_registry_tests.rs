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
        FinalizerRendererShapeClass::CompatibilityFallback,
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
        vec!["route_clarify_machine_envelope", "control_machine_envelope"]
    );
}
