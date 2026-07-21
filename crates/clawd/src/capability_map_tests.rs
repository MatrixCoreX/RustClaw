use super::*;
use claw_core::skill_registry::{
    CapabilityExecutionMode, CapabilityIsolationProfile, PlannerCapabilityEffect,
    PlannerCapabilityMapping, RegistryDedupScope, SkillRiskLevel, SkillsRegistry,
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
fn registry_group_controls_capability_group_token() {
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
    assert_eq!(registry_group_token(&entry).as_deref(), Some("news/web"));
}

#[test]
fn machine_skill_name_cannot_override_registry_group() {
    let entry = registry_entry_from(
        r#"
[[skills]]
name = "task_control"
enabled = true
planner_kind = "tool"
group = "ops"
"#,
        "task_control",
    );
    assert_eq!(registry_group_token(&entry).as_deref(), Some("ops"));
}

#[test]
fn arbitrary_registry_group_survives_without_compiled_taxonomy() {
    let entry = registry_entry_from(
        r#"
[[skills]]
name = "custom_science_tool"
enabled = true
planner_kind = "tool"
group = "Science/Lab"
"#,
        "custom_science_tool",
    );
    assert_eq!(registry_group_token(&entry).as_deref(), Some("science/lab"));
}

#[test]
fn missing_registry_group_remains_explicitly_ungrouped() {
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
    assert_eq!(registry_group_token(&entry), None);
}

#[test]
fn planner_capability_hint_includes_structured_contract() {
    let hint = planner_capability_hint(&PlannerCapabilityMapping {
        name: "filesystem.list_entries".to_string(),
        action: Some("list_dir".to_string()),
        effect: Some(PlannerCapabilityEffect::Observe),
        required: vec!["path".to_string()],
        optional: vec!["names_only".to_string()],
        risk_level: Some(SkillRiskLevel::Low),
        preferred: true,
        once_per_task: Some(false),
        dedup_scope: Some(RegistryDedupScope::Args),
        dedup_fields: Vec::new(),
        idempotent: Some(true),
        execution_mode: Some(CapabilityExecutionMode::AsyncRequired),
        async_adapter_kind: Some("media_job_poll".to_string()),
        isolation_profile: Some(CapabilityIsolationProfile::ReadOnly),
        network_access: Some(false),
        filesystem_write: Some(false),
        external_publish: Some(false),
        credential_access: Some(false),
        subprocess: None,
        package_install: None,
        privilege_escalation: None,
        final_answer_shape: Some("summary_with_evidence".to_string()),
    });
    assert_eq!(
        hint,
        "filesystem.list_entries(action=list_dir,effect=observe,required=path,optional=names_only,risk=low,preferred=true,once_per_task=false,dedup_scope=args,idempotent=true,execution_mode=async_required,async_adapter_kind=media_job_poll,isolation_profile=read_only,network_access=false,filesystem_write=false,external_publish=false,credential_access=false,final_answer_shape=summary_with_evidence)"
    );
}

#[test]
fn permission_profile_hint_uses_registry_machine_fields() {
    let entry = registry_entry_from(
        r#"
[[skills]]
name = "writer"
enabled = true
risk_level = "high"
requires_confirmation = true
side_effect = true
auto_invocable = false
once_per_task = true
dedup_scope = "action"
idempotent = false
"#,
        "writer",
    );
    assert_eq!(
        skill_permission_profile_hint(&entry).as_deref(),
        Some("risk=high,requires_confirmation=true,side_effect=true,auto_invocable=false,once_per_task=true,dedup_scope=action,idempotent=false")
    );
}

#[test]
fn compact_capability_map_omits_registry_skill_detail_duplication() {
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "compact-capability-map".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };

    let full = build_capability_map_for_task_with_detail(&state, &task, true);
    let compact = build_compact_capability_map_for_task(&state, &task);

    assert!(compact.contains("runtime_callable_capability_catalog_v1"));
    assert!(compact.contains("capability_value_contract=exact_catalog_token"));
    assert!(compact.contains("agent_runtime_protocols="));
    assert!(compact.contains("callable_capabilities="));
    assert!(!compact.contains("visible_skills="));
    assert!(!compact.contains("Registry skill hints:"));
    assert!(compact.len() < full.len());
}

#[test]
fn task_callable_catalog_exposes_capabilities_not_execution_skill_names() {
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "callable-capability-catalog".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };

    let names = planner_callable_capability_names_for_task(&state, &task);
    let compact = build_compact_capability_map_for_task(&state, &task);

    assert!(names.contains(&"process.ps".to_string()));
    assert!(names.contains(&"process.port_list".to_string()));
    assert!(!names.contains(&"process_basic".to_string()));
    assert!(compact.contains("process.ps"));
    assert!(!compact.contains("process_basic"));
}
