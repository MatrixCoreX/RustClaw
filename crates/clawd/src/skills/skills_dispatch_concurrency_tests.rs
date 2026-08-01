use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Semaphore;

use super::{acquire_skill_dispatch_permits_with_serialization, skill_dispatch_serialization_key};

#[test]
fn registry_effect_selects_mutation_serialization_without_serializing_reads() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let registry_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/skills_registry.toml");
    let registry = claw_core::skill_registry::SkillsRegistry::load_from_path(&registry_path)
        .expect("load registry");
    let enabled = registry
        .enabled_names()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    *state.core.skill_views_snapshot.write().expect("snapshot") =
        Arc::new(crate::SkillViewsSnapshot {
            binding: Default::default(),
            registry: Some(Arc::new(registry)),
            skills_list: Arc::new(enabled),
        });
    assert_eq!(
        skill_dispatch_serialization_key(
            &state,
            "http_basic",
            &serde_json::json!({"action": "download"}),
        )
        .as_deref(),
        Some("__serial_skill__http_basic")
    );
    assert!(skill_dispatch_serialization_key(
        &state,
        "transform",
        &serde_json::json!({"action": "transform_data"}),
    )
    .is_none());
}

#[tokio::test]
async fn serialized_mutation_waits_before_global_but_independent_read_continues() {
    let gates = Arc::new(crate::runtime::state::SkillConcurrencyGates::default());
    let global = Arc::new(Semaphore::new(2));
    let first = acquire_skill_dispatch_permits_with_serialization(
        &gates,
        &global,
        "mutation-first",
        "sample",
        None,
        Some("__serial_skill__sample"),
    )
    .await
    .expect("first mutation");
    let waiting_gates = gates.clone();
    let waiting_global = global.clone();
    let mut waiting = tokio::spawn(async move {
        acquire_skill_dispatch_permits_with_serialization(
            &waiting_gates,
            &waiting_global,
            "mutation-second",
            "sample",
            None,
            Some("__serial_skill__sample"),
        )
        .await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut waiting)
            .await
            .is_err()
    );
    assert_eq!(global.available_permits(), 1);
    let read = acquire_skill_dispatch_permits_with_serialization(
        &gates, &global, "read", "sample", None, None,
    )
    .await
    .expect("independent read");
    drop(read);
    drop(first);
    drop(waiting.await.expect("join").expect("second mutation"));
}
