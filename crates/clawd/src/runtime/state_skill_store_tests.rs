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

#[test]
fn registry_fixed_on_policy_replaces_the_fallback_floor() {
    let workspace =
        std::env::temp_dir().join(format!("rustclaw-fixed-on-registry-{}", std::process::id()));
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let registry_path = workspace.join("skills_registry.toml");
    std::fs::write(
        &registry_path,
        r#"
[[skills]]
name = "schedule"
enabled = false
fixed_on = true

[[skills]]
name = "run_cmd"
enabled = false
fixed_on = false
"#,
    )
    .expect("write registry");
    let initial = vec!["run_cmd".to_string()];
    let uninstalled = vec!["schedule".to_string(), "run_cmd".to_string()];

    let views = build_skill_views(
        &workspace,
        Some(registry_path.to_str().expect("registry path")),
        &HashMap::new(),
        &initial,
        &uninstalled,
    )
    .expect("build skill views");

    assert!(views.execution_skills.contains("schedule"));
    assert!(!views.execution_skills.contains("run_cmd"));
    std::fs::remove_dir_all(workspace).expect("remove workspace");
}
