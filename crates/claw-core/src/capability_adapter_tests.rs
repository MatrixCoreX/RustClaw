use super::{skill_background_job_capable, skill_uses_external_api, CapabilityAdapterKind};
use crate::skill_registry::{SkillRegistryEntry, SkillsRegistry};

fn registry_entry_from(toml: &str, name: &str) -> SkillRegistryEntry {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "capability_adapter_{}_{}_{}.toml",
        std::process::id(),
        nanos,
        name
    ));
    std::fs::write(&path, toml).unwrap();
    let registry = SkillsRegistry::load_from_path(&path).unwrap();
    let entry = registry.get(name).unwrap().clone();
    let _ = std::fs::remove_file(path);
    entry
}

#[test]
fn adapter_kind_uses_closed_machine_tokens() {
    assert_eq!(
        CapabilityAdapterKind::ExternalApiAdapter.as_token(),
        "external_api_adapter"
    );
    assert_eq!(
        CapabilityAdapterKind::from_token("mcp_tool"),
        Some(CapabilityAdapterKind::McpTool)
    );
    assert_eq!(CapabilityAdapterKind::from_token("unknown_tool"), None);
}

#[test]
fn registry_entry_maps_to_external_api_adapter() {
    let entry = registry_entry_from(
        r#"
[[skills]]
name = "media_generate"
enabled = true
kind = "runner"
planner_kind = "skill"
capabilities = ["llm", "net", "secrets.image_generation_minimax_api_key"]
"#,
        "media_generate",
    );
    assert!(skill_uses_external_api(&entry));
    assert_eq!(
        CapabilityAdapterKind::for_skill_registry_entry(&entry),
        CapabilityAdapterKind::ExternalApiAdapter
    );
}

#[test]
fn registry_entry_maps_to_local_tool_adapter() {
    let entry = registry_entry_from(
        r#"
[[skills]]
name = "filesystem"
enabled = true
kind = "builtin"
planner_kind = "tool"
capabilities = ["fs.read"]
"#,
        "filesystem",
    );
    assert!(!skill_uses_external_api(&entry));
    assert_eq!(
        CapabilityAdapterKind::for_skill_registry_entry(&entry),
        CapabilityAdapterKind::LocalToolAdapter
    );
}

#[test]
fn registry_entry_maps_to_workflow_before_resource_shape() {
    let entry = registry_entry_from(
        r#"
[[skills]]
name = "ops_workflow"
enabled = true
kind = "runner"
planner_kind = "workflow"
capabilities = ["net"]
"#,
        "ops_workflow",
    );
    assert_eq!(
        CapabilityAdapterKind::for_skill_registry_entry(&entry),
        CapabilityAdapterKind::Workflow
    );
}

#[test]
fn async_poll_fields_mark_background_job_capability() {
    let entry = registry_entry_from(
        r#"
[[skills]]
name = "video_generate"
enabled = true
kind = "runner"
planner_kind = "skill"

[[skills.planner_capabilities]]
name = "video.generate"
action = "generate"
optional = ["prompt", "wait_for_completion", "poll_after_seconds"]
"#,
        "video_generate",
    );
    assert!(skill_background_job_capable(&entry));
}
