use serde_json::json;

use super::*;

fn state_with_workspace_registry() -> crate::AppState {
    state_with_workspace_registry_excluding(&[])
}

fn state_with_workspace_registry_excluding(disabled: &[&str]) -> crate::AppState {
    let state = crate::AppState::test_default_with_fixture_provider();
    let registry_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/skills_registry.toml");
    let registry = claw_core::skill_registry::SkillsRegistry::load_from_path(&registry_path)
        .expect("load workspace skills registry");
    let enabled = registry
        .enabled_names()
        .into_iter()
        .filter(|skill| !disabled.iter().any(|disabled| skill.as_str() == *disabled))
        .collect::<std::collections::HashSet<_>>();
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("skill snapshot lock") = std::sync::Arc::new(crate::SkillViewsSnapshot {
        registry: Some(std::sync::Arc::new(registry)),
        skills_list: std::sync::Arc::new(enabled),
    });
    state
}

fn state_without_registry_with_skills(skills: &[&str]) -> crate::AppState {
    let state = crate::AppState::test_default_with_fixture_provider();
    let enabled = skills
        .iter()
        .map(|skill| (*skill).to_string())
        .collect::<std::collections::HashSet<_>>();
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("skill snapshot lock") = std::sync::Arc::new(crate::SkillViewsSnapshot {
        registry: None,
        skills_list: std::sync::Arc::new(enabled),
    });
    state
}

#[test]
fn resolves_filesystem_list_entries_to_fs_basic() {
    let action = resolve_static_capability_action(
        &normalize_capability_name("filesystem.list_entries"),
        json!({
            "path": ".",
            "names_only": true
        }),
    )
    .expect("capability should resolve");
    match action {
        AgentAction::CallTool { tool, args } => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("list_dir"));
            assert_eq!(args.get("path").and_then(Value::as_str), Some("."));
        }
        other => panic!("unexpected action: {other:?}"),
    }
}

#[test]
fn resolves_bare_fs_basic_capability_to_fs_basic_tool() {
    let action = resolve_static_capability_action(
        &normalize_capability_name("fs_basic"),
        json!({"action": "list_dir", "path": "."}),
    )
    .expect("bare fs_basic capability should resolve");
    match action {
        AgentAction::CallTool { tool, args } => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("list_dir"));
            assert_eq!(args.get("path").and_then(Value::as_str), Some("."));
        }
        other => panic!("unexpected action: {other:?}"),
    }
}

#[test]
fn resolves_system_run_command_to_run_cmd_skill() {
    let action = resolve_static_capability_action(
        &normalize_capability_name("system.run_command"),
        json!({"cmd": "pwd"}),
    )
    .expect("capability should resolve");
    match action {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(args.get("command").and_then(Value::as_str), Some("pwd"));
        }
        other => panic!("unexpected action: {other:?}"),
    }
}

#[test]
fn resolves_transform_capability_to_transform_skill() {
    let action = resolve_static_capability_action(
        &normalize_capability_name("transform"),
        json!({"data":[{"score": 1}], "ops":[{"op":"sort","by":"score"}]}),
    )
    .expect("capability should resolve");
    match action {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "transform");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("transform_data")
            );
        }
        other => panic!("unexpected action: {other:?}"),
    }
}

fn resolve_static_capability_action(normalized: &str, args: Value) -> Option<AgentAction> {
    match normalized {
        "fs_basic" => Some(AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args,
        }),
        "filesystem.list_entries" | "filesystem.list_dir" => Some(AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: with_action(args, "list_dir"),
        }),
        "transform" | "transform.transform_data" | "data.transform" => {
            Some(AgentAction::CallSkill {
                skill: "transform".to_string(),
                args: with_action(args, "transform_data"),
            })
        }
        "system.run_command" | "system.run_cmd" => Some(AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: normalize_run_command_args(args),
        }),
        _ => None,
    }
}

#[test]
fn resolver_candidate_rank_prefers_dedicated_low_risk_tool_before_run_cmd() {
    let mut candidates = vec![
        ResolverCandidate {
            skill: "run_cmd".to_string(),
            action: None,
            planner_kind: PlannerCapabilityKind::Tool,
            preferred: true,
            risk_level: SkillRiskLevel::High,
        },
        ResolverCandidate {
            skill: "fs_basic".to_string(),
            action: Some("list_dir".to_string()),
            planner_kind: PlannerCapabilityKind::Tool,
            preferred: true,
            risk_level: SkillRiskLevel::Low,
        },
    ];
    candidates.sort_by_key(resolver_candidate_rank);
    assert_eq!(candidates[0].skill, "fs_basic");
}

#[test]
fn capability_resolution_record_covers_resolved_mapping() {
    let state = state_without_registry_with_skills(&["fs_basic"]);
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "filesystem.list_entries",
        json!({"path": "."}),
    );
    let action = action.expect("static filesystem capability should resolve");
    match action {
        AgentAction::CallTool { tool, .. } => assert_eq!(tool, "fs_basic"),
        AgentAction::CallSkill { skill, .. } => assert_eq!(skill, "fs_basic"),
        other => panic!("unexpected resolved action: {other:?}"),
    }
    assert_eq!(record.owner_layer, "capability_resolver");
    assert!(matches!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
            | "capability_resolver_static_compat_resolved"
    ));
    assert_eq!(record.outcome, "resolved");
    assert!(matches!(record.source, "registry" | "static_compat"));
    assert_eq!(record.capability_ref, "filesystem.list_entries");
    assert!(matches!(
        record.resolved_ref.as_deref(),
        Some("tool:fs_basic") | Some("skill:fs_basic")
    ));
    assert!(record.planner_kind.is_some());
}

#[test]
fn workspace_registry_does_not_use_static_compat_for_ambiguous_bare_capability() {
    let state = state_with_workspace_registry();
    let (action, record) =
        resolve_capability_action_with_record_for_state(&state, "config_basic", json!({}));

    assert!(action.is_none());
    assert_eq!(record.reason_code, "capability_resolver_unresolved");
    assert_eq!(record.source, "none");
    assert_eq!(record.capability_ref, "config_basic");
}

#[test]
fn registry_resolves_crypto_positions_capability() {
    let state = state_with_workspace_registry();
    let (action, record) =
        resolve_capability_action_with_record_for_state(&state, "crypto.positions", json!({}));
    let action = action.expect("registry crypto.positions capability should resolve");
    match action {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "crypto");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("positions")
            );
        }
        other => panic!("unexpected resolved action: {other:?}"),
    }
    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    assert_eq!(record.source, "registry");
    assert_eq!(record.capability_ref, "crypto.positions");
}

#[test]
fn registry_resolves_bare_skill_capability_by_machine_action() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "task_control",
        json!({"action": "list", "limit": 5}),
    );
    let action = action.expect("bare task_control with machine action should resolve");
    match action {
        AgentAction::CallTool { tool, args } => {
            assert_eq!(tool, "task_control");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("list"));
            assert_eq!(args.get("limit").and_then(Value::as_i64), Some(5));
        }
        other => panic!("unexpected resolved action: {other:?}"),
    }
    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    assert_eq!(record.source, "registry");
    assert_eq!(record.capability_ref, "task_control");
    assert_eq!(record.resolved_ref.as_deref(), Some("tool:task_control"));
}

#[test]
fn registry_resolves_fully_qualified_skill_action_capability() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "browser_web.open_extract",
        json!({"url": "https://example.com"}),
    );
    let action = action.expect("registry skill.action capability should resolve");
    match action {
        AgentAction::CallTool { tool, args } => {
            assert_eq!(tool, "browser_web");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("open_extract")
            );
            assert_eq!(
                args.get("url").and_then(Value::as_str),
                Some("https://example.com")
            );
        }
        other => panic!("unexpected resolved action: {other:?}"),
    }
    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    assert_eq!(record.source, "registry");
    assert_eq!(record.capability_ref, "browser_web.open_extract");
    assert_eq!(record.resolved_ref.as_deref(), Some("tool:browser_web"));
}

#[test]
fn registry_resolves_doc_parse_bare_capability() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "doc_parse",
        json!({"path": "/tmp/example.md"}),
    );
    let action = action.expect("registry doc_parse capability should resolve");
    match action {
        AgentAction::CallTool { tool, args } => {
            assert_eq!(tool, "doc_parse");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("parse_doc")
            );
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some("/tmp/example.md")
            );
        }
        other => panic!("unexpected resolved action: {other:?}"),
    }
    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    assert_eq!(record.source, "registry");
    assert_eq!(record.capability_ref, "doc_parse");
}

#[test]
fn registry_resolves_terminal_layer_representative_capabilities() {
    let state = state_with_workspace_registry();
    let cases = [
        (
            "filesystem.list_entries",
            json!({"path": "."}),
            "tool:fs_basic",
        ),
        (
            "system.run_command",
            json!({"command": "pwd"}),
            "skill:run_cmd",
        ),
        ("git.status", json!({}), "tool:git_basic"),
        (
            "web.search_results",
            json!({"query": "rustclaw"}),
            "tool:web_search_extract",
        ),
        (
            "config.read_field",
            json!({"path": "configs/config.toml", "field_path": "skills.registry_path"}),
            "tool:config_basic",
        ),
        ("process.ps", json!({}), "tool:process_basic"),
        (
            "service.status",
            json!({"target": "clawd"}),
            "tool:service_control",
        ),
        (
            "task_control.list",
            json!({"limit": 5}),
            "tool:task_control",
        ),
        (
            "image_vision.describe",
            json!({"images": ["fixtures/image.png"]}),
            "skill:image_vision",
        ),
        (
            "audio.transcribe",
            json!({"audio_path": "fixtures/audio.wav"}),
            "skill:audio_transcribe",
        ),
        (
            "video.generate",
            json!({"prompt": "test"}),
            "skill:video_generate",
        ),
        (
            "music.generate",
            json!({"prompt": "test"}),
            "skill:music_generate",
        ),
    ];

    for (capability, args, expected_ref) in cases {
        let (action, record) =
            resolve_capability_action_with_record_for_state(&state, capability, args);
        assert!(action.is_some(), "{capability} should resolve");
        assert_eq!(
            record.reason_code, "capability_resolver_registry_mapping_resolved",
            "{capability} should resolve through registry"
        );
        assert_eq!(record.source, "registry");
        assert_eq!(record.capability_ref, capability);
        assert_eq!(record.resolved_ref.as_deref(), Some(expected_ref));
    }
}

#[test]
fn registry_resolves_legacy_machine_capability_aliases_before_static_fallback() {
    let state = state_with_workspace_registry();
    let cases = [
        ("system.run_cmd", json!({"command": "pwd"}), "skill:run_cmd"),
        (
            "filesystem.stat_path",
            json!({"path": "."}),
            "tool:fs_basic",
        ),
        ("filesystem.list_dir", json!({"path": "."}), "tool:fs_basic"),
        (
            "filesystem.read_file",
            json!({"path": "README.md"}),
            "tool:fs_basic",
        ),
        (
            "fs_basic.read_range",
            json!({"path": "README.md"}),
            "tool:fs_basic",
        ),
        (
            "fs_basic.read_file",
            json!({"path": "README.md"}),
            "tool:fs_basic",
        ),
        (
            "filesystem.find_files",
            json!({"root": ".", "pattern": "*.rs"}),
            "tool:fs_basic",
        ),
        (
            "filesystem.search_text",
            json!({"root": ".", "query": "TaskJournal"}),
            "tool:fs_basic",
        ),
        (
            "filesystem.create_dir",
            json!({"path": "/tmp/rustclaw-test"}),
            "tool:fs_basic",
        ),
        (
            "filesystem.delete_path",
            json!({"path": "/tmp/rustclaw-test"}),
            "tool:fs_basic",
        ),
        (
            "config.plan_config_change",
            json!({"field_path": "llm.default_vendor", "value": "minimax"}),
            "tool:config_edit",
        ),
        (
            "config.guard_config",
            json!({"path": "configs/config.toml"}),
            "tool:config_edit",
        ),
        (
            "system_basic.extract_field",
            json!({"path": "configs/config.toml", "field_path": "server.listen"}),
            "tool:system_basic",
        ),
        (
            "system_basic.read_text_range",
            json!({"path": "README.md"}),
            "tool:system_basic",
        ),
        (
            "transform",
            json!({"records": [{"score": 1}], "ops": [{"op": "sort", "by": "score"}]}),
            "tool:transform",
        ),
        (
            "data.transform_records",
            json!({"records": [{"score": 1}], "ops": [{"op": "sort", "by": "score"}]}),
            "tool:transform",
        ),
    ];

    for (capability, args, expected_ref) in cases {
        let (action, record) =
            resolve_capability_action_with_record_for_state(&state, capability, args);
        assert!(action.is_some(), "{capability} should resolve");
        assert_eq!(
            record.reason_code, "capability_resolver_registry_mapping_resolved",
            "{capability} should resolve through registry before static fallback"
        );
        assert_eq!(record.source, "registry");
        assert_eq!(record.capability_ref, capability);
        assert_eq!(record.resolved_ref.as_deref(), Some(expected_ref));
    }
}

#[test]
fn capability_resolution_record_covers_unresolved_mapping() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let (action, record) =
        resolve_capability_action_with_record_for_state(&state, "unknown.example", json!({}));
    assert!(action.is_none());
    assert_eq!(record.owner_layer, "capability_resolver");
    assert_eq!(record.reason_code, "capability_resolver_unresolved");
    assert_eq!(record.outcome, "unresolved");
    assert_eq!(record.source, "none");
    assert_eq!(record.capability_ref, "unknown.example");
    assert!(record.resolved_ref.is_none());
    assert!(record.planner_kind.is_none());
}

#[test]
fn disabled_registry_capability_returns_machine_disabled_record_without_static_fallback() {
    let state = state_with_workspace_registry_excluding(&["fs_basic"]);
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "filesystem.list_entries",
        json!({"path": "."}),
    );

    assert!(
        action.is_none(),
        "disabled registry capability must not fall back to static compat"
    );
    assert_eq!(record.owner_layer, "capability_resolver");
    assert_eq!(record.reason_code, "capability_disabled");
    assert_eq!(record.outcome, "blocked");
    assert_eq!(record.source, "registry");
    assert_eq!(record.capability_ref, "filesystem.list_entries");
    assert_eq!(record.resolved_ref.as_deref(), Some("tool:fs_basic"));
    assert_eq!(record.planner_kind, Some("tool"));
}
