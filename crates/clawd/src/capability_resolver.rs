use serde_json::Value;

use crate::{AgentAction, AppState};
use claw_core::skill_registry::{PlannerCapabilityKind, SkillRiskLevel};

pub(crate) fn resolve_agent_actions_for_state(
    state: &AppState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| resolve_agent_action_for_state(state, action))
        .collect()
}

pub(crate) fn resolve_agent_action_for_state(state: &AppState, action: AgentAction) -> AgentAction {
    match action {
        AgentAction::CallCapability { capability, args } => {
            match resolve_capability_action_for_state(state, &capability, args.clone()) {
                Some(resolved) => resolved,
                None => AgentAction::CallCapability { capability, args },
            }
        }
        other => other,
    }
}

pub(crate) fn resolve_capability_action_for_state(
    state: &AppState,
    capability: &str,
    args: Value,
) -> Option<AgentAction> {
    let normalized = normalize_capability_name(capability);
    resolve_registry_capability_action(state, &normalized, args.clone())
        .or_else(|| resolve_static_capability_action_for_state(state, &normalized, args))
}

#[derive(Debug)]
struct ResolverCandidate {
    skill: String,
    action: Option<String>,
    planner_kind: PlannerCapabilityKind,
    preferred: bool,
    risk_level: SkillRiskLevel,
}

fn resolve_static_capability_action_for_state(
    state: &AppState,
    normalized: &str,
    args: Value,
) -> Option<AgentAction> {
    let Some((skill, action)) = (match normalized {
        "filesystem.list_entries" | "filesystem.list_dir" => Some(("fs_basic", Some("list_dir"))),
        "filesystem.count_entries" => Some(("fs_basic", Some("count_entries"))),
        "filesystem.read_text_range" | "filesystem.read_text" | "filesystem.read_file" => {
            Some(("fs_basic", Some("read_text_range")))
        }
        "filesystem.stat_paths" | "filesystem.stat_path" => Some(("fs_basic", Some("stat_paths"))),
        "filesystem.find_entries" | "filesystem.find_files" | "filesystem.find_paths" => {
            Some(("fs_basic", Some("find_entries")))
        }
        "filesystem.grep_text" | "filesystem.search_text" => Some(("fs_basic", Some("grep_text"))),
        "filesystem.compare_paths" => Some(("fs_basic", Some("compare_paths"))),
        "filesystem.write_file" | "filesystem.write_text" => Some(("fs_basic", Some("write_text"))),
        "filesystem.make_dir" | "filesystem.create_dir" => Some(("fs_basic", Some("make_dir"))),
        "filesystem.remove_path" | "filesystem.delete_path" => {
            Some(("fs_basic", Some("remove_path")))
        }
        "config.read_field" => Some(("config_basic", Some("read_field"))),
        "config.read_fields" => Some(("config_basic", Some("read_fields"))),
        "config.list_keys" => Some(("config_basic", Some("list_keys"))),
        "config.validate" => Some(("config_basic", Some("validate"))),
        "config.guard_rustclaw_config" => Some(("config_basic", Some("guard_rustclaw_config"))),
        "config.plan_change" | "config.plan_config_change" => {
            Some(("config_edit", Some("plan_config_change")))
        }
        "config.apply_change"
        | "config.apply_config_change"
        | "config.write_field"
        | "config.set_field" => Some(("config_edit", Some("apply_config_change"))),
        "config.validate_after_change" => Some(("config_edit", Some("validate_config"))),
        "config.guard_after_change" | "config.guard_config" => {
            Some(("config_edit", Some("guard_config")))
        }
        "config.read_back" => Some(("config_edit", Some("read_back"))),
        "config.restart_if_requested" => Some(("config_edit", Some("restart_if_requested"))),
        "transform" | "transform.transform_data" | "data.transform" | "data.transform_records" => {
            Some(("transform", Some("transform_data")))
        }
        "system.run_command" | "system.run_cmd" => Some(("run_cmd", None)),
        _ => None,
    }) else {
        return None;
    };
    if !skill_is_globally_resolvable(state, skill) {
        return None;
    }
    let args = match action {
        Some(action) => with_action(args, action),
        None => args,
    };
    Some(action_for_skill(
        state
            .get_skills_registry()
            .as_ref()
            .and_then(|registry| registry.planner_kind(skill))
            .unwrap_or(PlannerCapabilityKind::Skill),
        skill.to_string(),
        args,
    ))
}

fn resolve_registry_capability_action(
    state: &AppState,
    normalized_capability: &str,
    args: Value,
) -> Option<AgentAction> {
    let registry = state.get_skills_registry()?;
    let mut candidates = Vec::new();
    for skill in registry.enabled_names() {
        if !skill_is_globally_resolvable(state, &skill) {
            continue;
        }
        let Some(mapping) = registry
            .planner_capabilities(&skill)
            .iter()
            .find(|mapping| mapping.name == normalized_capability)
        else {
            continue;
        };
        let manifest = registry.manifest(&skill);
        candidates.push(ResolverCandidate {
            skill,
            action: mapping.action.clone(),
            planner_kind: manifest
                .as_ref()
                .map(|manifest| manifest.planner_kind)
                .unwrap_or(PlannerCapabilityKind::Skill),
            preferred: mapping.preferred
                || manifest
                    .as_ref()
                    .is_some_and(|manifest| manifest.preferred_over_run_cmd),
            risk_level: mapping
                .risk_level
                .or_else(|| manifest.as_ref().and_then(|manifest| manifest.risk_level))
                .unwrap_or(SkillRiskLevel::Unknown),
        });
    }
    candidates.sort_by_key(resolver_candidate_rank);
    candidates
        .into_iter()
        .next()
        .map(|candidate| resolve_candidate_action(candidate, args))
}

fn resolve_candidate_action(candidate: ResolverCandidate, args: Value) -> AgentAction {
    let mut resolved_args = args.as_object().cloned().unwrap_or_default();
    if let Some(action) = candidate.action.as_deref() {
        resolved_args
            .entry("action".to_string())
            .or_insert_with(|| Value::String(action.to_string()));
    }
    action_for_skill(
        candidate.planner_kind,
        candidate.skill,
        Value::Object(resolved_args),
    )
}

fn action_for_skill(
    planner_kind: PlannerCapabilityKind,
    skill: String,
    args: Value,
) -> AgentAction {
    if skill == "run_cmd" {
        return AgentAction::CallSkill {
            skill,
            args: normalize_run_command_args(args),
        };
    }
    match planner_kind {
        PlannerCapabilityKind::Tool => AgentAction::CallTool { tool: skill, args },
        PlannerCapabilityKind::Skill | PlannerCapabilityKind::Workflow => {
            AgentAction::CallSkill { skill, args }
        }
    }
}

fn resolver_candidate_rank(candidate: &ResolverCandidate) -> (u8, u8, u8, u8, String) {
    (
        if candidate.skill == "run_cmd" { 1 } else { 0 },
        if candidate.preferred { 0 } else { 1 },
        planner_kind_rank(candidate.planner_kind),
        risk_rank(candidate.risk_level),
        candidate.skill.clone(),
    )
}

fn planner_kind_rank(kind: PlannerCapabilityKind) -> u8 {
    match kind {
        PlannerCapabilityKind::Tool => 0,
        PlannerCapabilityKind::Skill => 1,
        PlannerCapabilityKind::Workflow => 2,
    }
}

fn risk_rank(risk: SkillRiskLevel) -> u8 {
    match risk {
        SkillRiskLevel::Low => 0,
        SkillRiskLevel::Medium => 1,
        SkillRiskLevel::High => 2,
        SkillRiskLevel::Unknown => 3,
    }
}

fn skill_is_globally_resolvable(state: &AppState, skill: &str) -> bool {
    let enabled_skills = state.get_skills_list();
    if !enabled_skills.is_empty() && !enabled_skills.contains(skill) {
        return false;
    }
    if let Some(registry) = state.get_skills_registry() {
        if !registry.is_planner_visible(skill) {
            return false;
        }
        if let Some(manifest) = registry.manifest(skill) {
            if !crate::skill_availability::evaluate_manifest_availability(&manifest).is_available()
            {
                return false;
            }
        }
    }
    true
}

fn normalize_capability_name(capability: &str) -> String {
    capability
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .replace("::", ".")
}

fn with_action(args: Value, action: &str) -> Value {
    let mut obj = args.as_object().cloned().unwrap_or_default();
    obj.entry("action".to_string())
        .or_insert_with(|| Value::String(action.to_string()));
    Value::Object(obj)
}

fn normalize_run_command_args(args: Value) -> Value {
    let mut obj = args.as_object().cloned().unwrap_or_default();
    if !obj.contains_key("command") {
        if let Some(cmd) = obj.remove("cmd") {
            obj.insert("command".to_string(), cmd);
        }
    }
    Value::Object(obj)
}

#[cfg(test)]
mod tests {
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
}
