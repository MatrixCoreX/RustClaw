use serde_json::json;

use super::*;

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
