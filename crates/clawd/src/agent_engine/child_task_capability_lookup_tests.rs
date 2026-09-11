use super::super::tests::{install_test_registry, test_state};
use super::*;

const REGISTRY: &str = r#"
[[skills]]
name = "optional_probe"
enabled = false
kind = "builtin"
planner_kind = "tool"
planner_capabilities = [
  { name = "optional_probe.inspect", action = "inspect", effect = "observe", isolation_profile = "read_only", network_access = false, filesystem_write = false },
  { name = "optional_probe.write", action = "write", effect = "mutate", isolation_profile = "local_current_workspace", filesystem_write = true },
]
"#;

#[test]
fn child_capability_lookup_uses_runtime_enable_state_not_release_default() {
    let state = test_state();
    install_test_registry(&state, REGISTRY, &["optional_probe"]);
    assert!(selected_capability_by_name(&state, "optional_probe.inspect").is_some());

    install_test_registry(
        &state,
        &REGISTRY.replace("enabled = false", "enabled = true"),
        &[],
    );
    assert!(selected_capability_by_name(&state, "optional_probe.inspect").is_none());
    assert!(selected_capability_by_name(&state, "unknown.inspect").is_none());
}

#[test]
fn enabled_optional_capability_still_enforces_readonly_policy() {
    let state = test_state();
    install_test_registry(&state, REGISTRY, &["optional_probe"]);
    let observe = selected_capability_by_name(&state, "optional_probe.inspect").unwrap();
    let mutate = selected_capability_by_name(&state, "optional_probe.write").unwrap();
    let mut violations = Vec::new();
    append_read_only_violations(&observe, &mut violations);
    assert!(violations.is_empty());
    append_read_only_violations(&mutate, &mut violations);
    assert!(!violations.is_empty());
    assert!(!violations.contains(&"capability_unresolved"));
}
