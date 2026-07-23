use super::*;
use claw_core::skill_registry::{
    CapabilityExecutionMode, CapabilityIsolationProfile, PlannerCapabilityEffect,
    PlannerCapabilityMapping, RegistryDedupScope, SkillRiskLevel, SkillsRegistry,
};
use std::path::Path;

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
        description: Some("List direct workspace entries in one bounded observation.".to_string()),
        semantic_tags: vec!["directory_listing".to_string()],
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
        "filesystem.list_entries(action=list_dir,purpose=List direct workspace entries in one bounded observation.,semantic_tags=directory_listing,effect=observe,required=path,optional=names_only,risk=low,preferred=true,once_per_task=false,dedup_scope=args,idempotent=true,execution_mode=async_required,async_adapter_kind=media_job_poll,isolation_profile=read_only,network_access=false,filesystem_write=false,external_publish=false,credential_access=false,final_answer_shape=summary_with_evidence)"
    );
}

#[test]
fn real_config_capability_hints_keep_leaf_semantics_distinct() {
    let entry = registry_entry_from(
        &std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/skills_registry.toml"),
        )
        .expect("read registry"),
        "config_basic",
    );
    let validate = entry
        .planner_capabilities
        .iter()
        .find(|mapping| mapping.name == "config.validate")
        .expect("validate mapping");
    let guard = entry
        .planner_capabilities
        .iter()
        .find(|mapping| mapping.name == "config.guard_rustclaw_config")
        .expect("guard mapping");

    let validate_hint = planner_capability_hint(validate);
    let guard_hint = planner_capability_hint(guard);
    assert!(validate_hint.contains("semantic_tags=syntax_validation|structured_parse"));
    assert!(validate_hint.contains("does not assess safety"));
    assert!(guard_hint
        .contains("semantic_tags=rustclaw_config_safety|config_risk_scan|config_problem_check"));
    assert!(guard_hint.contains("instead of reading raw file text"));
}

#[test]
fn native_capability_groups_expose_distinct_registry_tools() {
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "native-capability-groups".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };

    let groups = planner_native_capability_groups_for_task(&state, &task);
    let doc_parse = groups
        .iter()
        .find(|group| group.capability_names.contains(&"doc_parse".to_string()))
        .expect("doc_parse native group");
    let filesystem = groups
        .iter()
        .find(|group| {
            group
                .capability_names
                .contains(&"filesystem.read_text_range".to_string())
        })
        .expect("filesystem native group");

    assert_eq!(doc_parse.tool_name, "call_doc_parse");
    assert!(doc_parse.description.contains("document_summary"));
    assert_ne!(doc_parse.tool_name, filesystem.tool_name);
    assert_eq!(filesystem.capability_names.len(), 18);
    assert!(filesystem
        .capability_names
        .contains(&"filesystem.read_text_range".to_string()));
    assert!(filesystem
        .capability_names
        .contains(&"filesystem.count_entries".to_string()));
    assert!(!filesystem
        .capability_names
        .contains(&"fs.count_entries".to_string()));
    assert!(!filesystem
        .capability_names
        .contains(&"filesystem.read_file".to_string()));
    assert!(!filesystem
        .capability_names
        .contains(&"fs_basic.read_file".to_string()));
    assert!(groups.iter().all(|group| {
        !group.capability_names.is_empty()
            && group.tool_name.chars().count() <= NATIVE_GROUP_TOOL_NAME_CHAR_BUDGET
            && group.description.chars().count() <= NATIVE_GROUP_DESCRIPTION_CHAR_BUDGET
    }));
}

#[test]
fn disclosed_native_groups_keep_core_eager_and_domain_groups_loadable() {
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "native-capability-disclosure".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };

    let full = planner_native_capability_groups_for_task(&state, &task);
    let initial =
        planner_disclosed_native_capability_groups_for_task(&state, &task, &BTreeSet::new());
    let loadable =
        planner_loadable_capability_group_names_for_task(&state, &task, &BTreeSet::new());
    assert_eq!(full.len(), 34);
    assert_eq!(initial.len(), 7);
    assert_eq!(
        initial
            .iter()
            .map(|group| group.capability_names.len())
            .sum::<usize>(),
        71
    );
    assert_eq!(loadable.len(), 27);
    assert_eq!(full.len(), initial.len() + loadable.len());
    assert!(initial
        .iter()
        .any(|group| group.tool_name == "call_fs_basic"));
    assert!(initial
        .iter()
        .any(|group| group.tool_name == "call_git_basic"));
    let domain_group = loadable
        .first()
        .cloned()
        .expect("fixture must expose an on-demand group");
    assert!(!initial.iter().any(|group| group.skill_name == domain_group));
    assert!(initial.iter().all(|group| {
        group.capability_names.iter().all(|capability| {
            state.get_skills_registry().is_some_and(|registry| {
                registry.canonical_planner_capability_name(capability) == Some(capability.as_str())
            })
        })
    }));

    let loaded = BTreeSet::from([domain_group.clone()]);
    let expanded = planner_disclosed_native_capability_groups_for_task(&state, &task, &loaded);
    assert!(expanded
        .iter()
        .any(|group| group.skill_name == domain_group));
    assert!(
        !planner_loadable_capability_group_names_for_task(&state, &task, &loaded)
            .contains(&domain_group)
    );
}

#[test]
fn scoped_text_catalog_discloses_only_active_groups_and_loader_tokens() {
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "scoped-text-capability-catalog".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let loadable =
        planner_loadable_capability_group_names_for_task(&state, &task, &BTreeSet::new());
    let selected = loadable
        .iter()
        .find(|skill| {
            state.get_skills_registry().is_some_and(|registry| {
                registry
                    .planner_exposed_capabilities(skill)
                    .into_iter()
                    .any(|mapping| mapping.name != **skill)
            })
        })
        .cloned()
        .expect("domain group with a distinct capability token");
    let capability = state
        .get_skills_registry()
        .and_then(|registry| {
            registry
                .planner_exposed_capabilities(&selected)
                .into_iter()
                .find(|mapping| mapping.name != selected)
                .map(|mapping| mapping.name.clone())
        })
        .expect("selected capability");

    let initial = build_scoped_compact_capability_map_for_task(
        &state,
        &task,
        &BTreeSet::new(),
        &BTreeSet::new(),
    );
    assert!(initial.contains("loadable_capability_groups="));
    assert!(initial.contains(&selected));
    assert!(!initial.contains(&capability));

    let loaded = build_scoped_compact_capability_map_for_task(
        &state,
        &task,
        &BTreeSet::from([selected.clone()]),
        &BTreeSet::new(),
    );
    assert!(loaded.contains(&capability));
    assert!(!loaded
        .lines()
        .find(|line| line.starts_with("loadable_capability_groups="))
        .and_then(|line| line.split_once('=').map(|(_, tokens)| tokens))
        .is_some_and(|tokens| tokens.split(',').any(|token| token == selected)));
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
    assert!(names.contains(&"agent.subagent".to_string()));
    assert!(names.contains(&"agent.subagent_batch".to_string()));
    assert!(names.contains(&"agent.subagent_persistent".to_string()));
    assert!(!names.contains(&"process_basic".to_string()));
    assert!(compact.contains("process.ps"));
    assert!(compact.contains("agent.subagent"));
    assert!(compact.contains("agent.subagent_batch"));
    assert!(!compact.contains("process_basic"));
}

#[test]
fn child_task_catalog_exposes_only_contract_allowed_capabilities() {
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "child-capability-catalog".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({
            "task_role": "subagent_child",
            "child_task_contract": {
                "scope": {
                    "allowed_capabilities": [
                        "filesystem.read_text_range",
                        "filesystem.find_entries"
                    ]
                }
            }
        })
        .to_string(),
    };

    let names = planner_callable_capability_names_for_task(&state, &task);
    let native_group_names = planner_native_capability_groups_for_task(&state, &task)
        .into_iter()
        .flat_map(|group| group.capability_names)
        .collect::<BTreeSet<_>>();
    let disclosed_group_names =
        planner_disclosed_native_capability_groups_for_task(&state, &task, &BTreeSet::new())
            .into_iter()
            .flat_map(|group| group.capability_names)
            .collect::<BTreeSet<_>>();
    let loadable_groups =
        planner_loadable_capability_group_names_for_task(&state, &task, &BTreeSet::new());
    let compact = build_capability_map_for_task_with_detail(&state, &task, true);

    assert_eq!(
        names,
        vec![
            "filesystem.find_entries".to_string(),
            "filesystem.read_text_range".to_string()
        ]
    );
    assert_eq!(native_group_names, names.iter().cloned().collect());
    assert_eq!(disclosed_group_names, native_group_names);
    assert!(loadable_groups.is_empty());
    assert!(compact.contains("filesystem.find_entries"));
    assert!(compact.contains("filesystem.read_text_range"));
    assert!(!compact.contains("agent.subagent"));
    assert!(!compact.contains("process.ps"));
}
