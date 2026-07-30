use std::collections::HashSet;
use std::sync::Arc;

use serde_json::json;

use super::resolve_skill_timeout;
use crate::{AppState, SkillViewsSnapshot};

#[test]
fn resolution_prefers_capability_then_registry_then_global() {
    let mut state = AppState::test_default_with_fixture_provider();
    state.skill_rt.skill_timeout_seconds = 55;
    let registry = claw_core::skill_registry::SkillsRegistry::load_from_str(
        r#"
[[skills]]
name = "timeout_fixture"
enabled = true
kind = "runner"
timeout_seconds = 30
planner_capabilities = [
  { name = "fixture.run", action = "run", timeout_seconds = 7 }
]
input_schema = { type = "object", properties = { action = { type = "string" } } }
"#,
    )
    .expect("load timeout fixture registry");
    let enabled = registry.enabled_names().into_iter().collect::<HashSet<_>>();
    *state.core.skill_views_snapshot.write().unwrap() = Arc::new(SkillViewsSnapshot {
        binding: Default::default(),
        registry: Some(Arc::new(registry)),
        skills_list: Arc::new(enabled),
    });

    let capability = resolve_skill_timeout(&state, "timeout_fixture", &json!({"action": "run"}));
    assert_eq!(capability.seconds, 7);
    assert_eq!(capability.source, "capability");

    let registry = resolve_skill_timeout(&state, "timeout_fixture", &json!({"action": "unknown"}));
    assert_eq!(registry.seconds, 30);
    assert_eq!(registry.source, "registry");

    let global = resolve_skill_timeout(&state, "not_registered", &json!({}));
    assert_eq!(global.seconds, 55);
    assert_eq!(global.source, "global");
}
