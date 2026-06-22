use serde_json::json;

use super::*;

fn state_with_workspace_registry() -> crate::AppState {
    let state = crate::AppState::test_default_with_fixture_provider();
    let registry_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/skills_registry.toml");
    let registry = claw_core::skill_registry::SkillsRegistry::load_from_path(&registry_path)
        .expect("load workspace skills registry");
    let enabled = registry
        .enabled_names()
        .into_iter()
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
    let state = crate::AppState::test_default_with_fixture_provider();
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
            | "capability_resolver_static_mapping_resolved"
    ));
    assert_eq!(record.outcome, "resolved");
    assert!(matches!(record.source, "registry" | "static"));
    assert_eq!(record.capability_ref, "filesystem.list_entries");
    assert!(matches!(
        record.resolved_ref.as_deref(),
        Some("tool:fs_basic") | Some("skill:fs_basic")
    ));
    assert!(record.planner_kind.is_some());
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
