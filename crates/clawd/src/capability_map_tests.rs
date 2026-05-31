use super::*;
use claw_core::skill_registry::{
    PlannerCapabilityEffect, PlannerCapabilityMapping, SkillsRegistry,
};

fn registry_entry_from(toml: &str, name: &str) -> SkillRegistryEntry {
    let path = std::env::temp_dir().join(format!("capability_map_{name}.toml"));
    std::fs::write(&path, toml).unwrap();
    let registry = SkillsRegistry::load_from_path(&path).unwrap();
    let entry = registry.get(name).unwrap().clone();
    let _ = std::fs::remove_file(path);
    entry
}

#[test]
fn registry_group_controls_capability_domain() {
    let entry = registry_entry_from(
        r#"
[[skills]]
name = "custom_web_tool"
enabled = true
planner_kind = "tool"
group = "news/web"
capabilities = ["net"]
"#,
        "custom_web_tool",
    );
    assert_eq!(
        infer_domain_from_registry_entry(&entry),
        Some(CapabilityDomain::NewsContent)
    );
}

#[test]
fn filesystem_capability_infers_domain_without_skill_name() {
    let entry = registry_entry_from(
        r#"
[[skills]]
name = "custom_reader"
enabled = true
planner_kind = "tool"
capabilities = ["fs.read"]
"#,
        "custom_reader",
    );
    assert_eq!(
        infer_domain_from_registry_entry(&entry),
        Some(CapabilityDomain::Filesystem)
    );
}

#[test]
fn planner_capability_hint_includes_structured_contract() {
    let hint = planner_capability_hint(&PlannerCapabilityMapping {
        name: "filesystem.list_entries".to_string(),
        action: Some("list_dir".to_string()),
        effect: Some(PlannerCapabilityEffect::Observe),
        required: vec!["path".to_string()],
        optional: vec!["names_only".to_string()],
        risk_level: None,
        preferred: true,
    });
    assert_eq!(
        hint,
        "filesystem.list_entries(action=list_dir,effect=observe,required=path,preferred=true)"
    );
}
