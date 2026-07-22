use std::collections::HashMap;

use super::build_skill_views;

#[test]
fn uninstalled_optional_skill_is_removed_but_core_skill_stays_available() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let initial = vec!["weather".to_string(), "schedule".to_string()];
    let uninstalled = vec!["weather".to_string(), "schedule".to_string()];

    let views = build_skill_views(workspace, None, &HashMap::new(), &initial, &uninstalled)
        .expect("build skill views");

    assert!(!views.execution_skills.contains("weather"));
    assert!(views.execution_skills.contains("schedule"));
}
