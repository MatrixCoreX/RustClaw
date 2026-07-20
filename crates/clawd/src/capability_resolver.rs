use serde_json::{json, Value};

use crate::{AgentAction, AppState};
use claw_core::skill_registry::{
    PlannerCapabilityKind, PlannerCapabilityMapping, SkillRiskLevel, SkillsRegistry,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapabilityResolutionRecord {
    pub(crate) owner_layer: &'static str,
    pub(crate) reason_code: &'static str,
    pub(crate) outcome: &'static str,
    pub(crate) source: &'static str,
    pub(crate) capability_ref: String,
    pub(crate) canonical_capability_ref: Option<String>,
    pub(crate) resolved_ref: Option<String>,
    pub(crate) planner_kind: Option<&'static str>,
}

impl CapabilityResolutionRecord {
    fn resolved(
        reason_code: &'static str,
        source: &'static str,
        capability_ref: impl Into<String>,
        resolved: &AgentAction,
        planner_kind: PlannerCapabilityKind,
    ) -> Self {
        Self {
            owner_layer: "capability_resolver",
            reason_code,
            outcome: "resolved",
            source,
            capability_ref: capability_ref.into(),
            canonical_capability_ref: None,
            resolved_ref: resolved_action_ref(resolved),
            planner_kind: Some(planner_kind.as_token()),
        }
    }

    fn unresolved(capability_ref: impl Into<String>) -> Self {
        Self {
            owner_layer: "capability_resolver",
            reason_code: "capability_unavailable",
            outcome: "unresolved",
            source: "none",
            capability_ref: capability_ref.into(),
            canonical_capability_ref: None,
            resolved_ref: None,
            planner_kind: None,
        }
    }

    fn blocked(
        reason_code: &'static str,
        capability_ref: impl Into<String>,
        canonical_capability_ref: impl Into<String>,
        skill: &str,
        planner_kind: PlannerCapabilityKind,
    ) -> Self {
        Self {
            owner_layer: "capability_resolver",
            reason_code,
            outcome: "blocked",
            source: "registry",
            capability_ref: capability_ref.into(),
            canonical_capability_ref: Some(canonical_capability_ref.into()),
            resolved_ref: Some(resolved_ref_for_skill(planner_kind, skill)),
            planner_kind: Some(planner_kind.as_token()),
        }
    }

    pub(crate) fn dispatch_observation(
        &self,
        round_no: usize,
        global_step: usize,
        step_in_round: usize,
    ) -> Value {
        json!({
            "observation_kind": "capability_resolution",
            "owner_layer": self.owner_layer,
            "reason_code": self.reason_code,
            "outcome": self.outcome,
            "source": self.source,
            "requested_capability": self.capability_ref,
            "resolved_capability": self.canonical_capability_ref,
            "resolved_tool_or_skill": self.resolved_ref,
            "planner_kind": self.planner_kind,
            "round_no": round_no,
            "global_step": global_step,
            "step_in_round": step_in_round,
        })
    }
}

#[derive(Debug, Clone)]
struct ResolvedCapabilityAction {
    action: AgentAction,
    record: CapabilityResolutionRecord,
}

#[derive(Debug, Clone)]
enum RegistryCapabilityResolution {
    Resolved(ResolvedCapabilityAction),
    Blocked(CapabilityResolutionRecord),
    None,
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
    resolve_capability_action_with_record_for_state(state, capability, args).0
}

pub(crate) fn resolve_capability_action_with_record_for_state(
    state: &AppState,
    capability: &str,
    args: Value,
) -> (Option<AgentAction>, CapabilityResolutionRecord) {
    let (normalized, args) = normalize_capability_invocation(capability, args);
    match resolve_registry_capability_action(state, &normalized, args.clone()) {
        RegistryCapabilityResolution::Resolved(resolved) => {
            return (Some(resolved.action), resolved.record);
        }
        RegistryCapabilityResolution::Blocked(record) => {
            return (None, record);
        }
        RegistryCapabilityResolution::None => {}
    }
    if let Some(tool) = state.mcp_tool(&normalized) {
        let action = AgentAction::CallTool {
            tool: tool.capability,
            args,
        };
        let mut record = CapabilityResolutionRecord::resolved(
            "capability_resolver_mcp_mapping_resolved",
            "mcp",
            normalized.clone(),
            &action,
            PlannerCapabilityKind::Tool,
        );
        record.canonical_capability_ref = Some(normalized);
        return (Some(action), record);
    }
    (None, CapabilityResolutionRecord::unresolved(normalized))
}

#[derive(Debug)]
struct ResolverCandidate {
    skill: String,
    capability: String,
    action: Option<String>,
    planner_kind: PlannerCapabilityKind,
    preferred: bool,
    risk_level: SkillRiskLevel,
}

fn resolve_registry_capability_action(
    state: &AppState,
    normalized_capability: &str,
    args: Value,
) -> RegistryCapabilityResolution {
    let Some(registry) = state.get_skills_registry() else {
        return RegistryCapabilityResolution::None;
    };
    let mut candidates = Vec::new();
    let mut blocked = Vec::new();
    for skill in registry.enabled_names() {
        let Some(mapping) =
            registry_mapping_for_capability(&registry, &skill, normalized_capability, &args)
        else {
            continue;
        };
        let manifest = registry.manifest(&skill);
        let planner_kind = manifest
            .as_ref()
            .map(|manifest| manifest.planner_kind)
            .unwrap_or(PlannerCapabilityKind::Skill);
        if let Some(reason_code) = skill_resolution_block_reason(state, &registry, &skill) {
            blocked.push(CapabilityResolutionRecord::blocked(
                reason_code,
                normalized_capability.to_string(),
                mapping.name.clone(),
                &skill,
                planner_kind,
            ));
            continue;
        }
        candidates.push(ResolverCandidate {
            skill,
            capability: mapping.name.clone(),
            action: mapping.action.clone(),
            planner_kind,
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
    if let Some(candidate) = candidates.into_iter().next() {
        let planner_kind = candidate.planner_kind;
        let canonical_capability_ref = candidate.capability.clone();
        let action = resolve_candidate_action(candidate, args);
        let mut record = CapabilityResolutionRecord::resolved(
            "capability_resolver_registry_mapping_resolved",
            "registry",
            normalized_capability.to_string(),
            &action,
            planner_kind,
        );
        record.canonical_capability_ref = Some(canonical_capability_ref);
        return RegistryCapabilityResolution::Resolved(ResolvedCapabilityAction { record, action });
    }
    if let Some(record) = blocked.into_iter().next() {
        return RegistryCapabilityResolution::Blocked(record);
    }
    RegistryCapabilityResolution::None
}

fn skill_resolution_block_reason(
    state: &AppState,
    registry: &SkillsRegistry,
    skill: &str,
) -> Option<&'static str> {
    let enabled_skills = state.get_skills_list();
    if !enabled_skills.is_empty() && !enabled_skills.contains(skill) {
        return Some("capability_disabled");
    }
    if !registry.is_planner_visible(skill) {
        return Some("capability_unavailable");
    }
    if let Some(manifest) = registry.manifest(skill) {
        if !crate::skill_availability::evaluate_manifest_availability(&manifest).is_available() {
            return Some("capability_unavailable");
        }
    }
    None
}

fn registry_mapping_for_capability<'a>(
    registry: &'a SkillsRegistry,
    skill: &str,
    normalized_capability: &str,
    args: &Value,
) -> Option<&'a PlannerCapabilityMapping> {
    let mappings = registry.planner_capabilities(skill);
    if let Some(mapping) = mappings
        .iter()
        .find(|mapping| mapping.name == normalized_capability)
    {
        return Some(mapping);
    }

    if let Some((capability_skill, capability_action)) = normalized_capability.split_once('.') {
        if registry_skill_name_matches(registry, skill, capability_skill) {
            if let Some(mapping) = registry_mapping_for_action(mappings, capability_action) {
                return Some(mapping);
            }
        }
    }

    let canonical = registry.resolve_canonical(normalized_capability)?;
    if canonical != skill {
        return None;
    }
    let requested_action = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|action| !action.is_empty())
        .map(normalize_capability_name)?;
    registry_mapping_for_action(mappings, &requested_action)
}

fn registry_skill_name_matches(
    registry: &SkillsRegistry,
    skill: &str,
    capability_skill: &str,
) -> bool {
    capability_skill == skill
        || registry
            .resolve_canonical(capability_skill)
            .as_deref()
            .is_some_and(|canonical| canonical == skill)
}

fn registry_mapping_for_action<'a>(
    mappings: &'a [PlannerCapabilityMapping],
    requested_action: &str,
) -> Option<&'a PlannerCapabilityMapping> {
    let requested_action = normalize_capability_name(requested_action);
    if let Some(mapping) = mappings.iter().find(|mapping| {
        mapping
            .action
            .as_deref()
            .map(normalize_capability_name)
            .as_deref()
            == Some(requested_action.as_str())
    }) {
        return Some(mapping);
    }

    let mut aliases = mappings.iter().filter(|mapping| {
        mapping
            .name
            .rsplit_once('.')
            .map(|(_, action)| normalize_capability_name(action))
            .as_deref()
            == Some(requested_action.as_str())
    });
    let first = aliases.next()?;
    let canonical_action = first.action.as_deref().map(normalize_capability_name)?;
    aliases
        .all(|mapping| {
            mapping
                .action
                .as_deref()
                .map(normalize_capability_name)
                .as_deref()
                == Some(canonical_action.as_str())
        })
        .then_some(first)
}

fn resolve_candidate_action(candidate: ResolverCandidate, args: Value) -> AgentAction {
    let mut resolved_args = args.as_object().cloned().unwrap_or_default();
    if let Some(action) = candidate.action.as_deref() {
        resolved_args.insert(
            "action".to_string(),
            Value::String(normalize_capability_name(action)),
        );
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

fn resolved_action_ref(action: &AgentAction) -> Option<String> {
    match action {
        AgentAction::CallTool { tool, .. } => Some(format!("tool:{tool}")),
        AgentAction::CallSkill { skill, .. } => Some(format!("skill:{skill}")),
        AgentAction::CallCapability { capability, .. } => Some(format!("capability:{capability}")),
        AgentAction::SynthesizeAnswer { .. } => Some("synthesize_answer".to_string()),
        AgentAction::Respond { .. } => Some("respond".to_string()),
        AgentAction::Think { .. } => Some("think".to_string()),
    }
}

fn resolved_ref_for_skill(planner_kind: PlannerCapabilityKind, skill: &str) -> String {
    match planner_kind {
        PlannerCapabilityKind::Tool => format!("tool:{skill}"),
        PlannerCapabilityKind::Skill | PlannerCapabilityKind::Workflow => format!("skill:{skill}"),
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

fn normalize_capability_name(capability: &str) -> String {
    capability
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .replace("::", ".")
}

fn normalize_capability_invocation(capability: &str, args: Value) -> (String, Value) {
    let normalized = normalize_capability_name(capability);
    if normalized == "system.runtime_status" && args_has_command_field(&args) {
        return (
            "system.run_command".to_string(),
            normalize_run_command_args(args),
        );
    }
    if normalized == "system.runtime_status" && args_targets_task_control_status(&args) {
        return (
            "task_control.list".to_string(),
            normalize_task_control_list_args(args),
        );
    }
    if matches!(
        normalized.as_str(),
        "system.run_command" | "system.run_cmd" | "system.shell_run" | "run_cmd"
    ) {
        return (normalized, normalize_run_command_args(args));
    }
    let args = normalize_filesystem_capability_args(&normalized, args);
    let args = normalize_config_basic_capability_args(&normalized, args);
    (normalized, args)
}

fn args_targets_task_control_status(args: &Value) -> bool {
    args.get("kind")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(normalize_capability_name)
        .is_some_and(|kind| {
            matches!(
                kind.as_str(),
                "task_queue_status" | "task_status" | "runtime_tasks" | "task_lifecycle"
            )
        })
}

fn normalize_task_control_list_args(args: Value) -> Value {
    let mut obj = match args {
        Value::Object(obj) => obj,
        other => return other,
    };
    obj.remove("kind");
    Value::Object(obj)
}

fn normalize_filesystem_capability_args(normalized_capability: &str, args: Value) -> Value {
    let mut obj = match args {
        Value::Object(obj) => obj,
        other => return other,
    };
    match normalized_capability {
        "filesystem.list_file_names" | "fs.list_file_names" | "fs_basic.list_file_names" => {
            obj.insert("names_only".to_string(), Value::Bool(true));
            obj.insert("files_only".to_string(), Value::Bool(true));
            obj.insert("dirs_only".to_string(), Value::Bool(false));
        }
        "filesystem.list_directory_names"
        | "fs.list_directory_names"
        | "fs_basic.list_directory_names" => {
            obj.insert("names_only".to_string(), Value::Bool(true));
            obj.insert("dirs_only".to_string(), Value::Bool(true));
            obj.insert("files_only".to_string(), Value::Bool(false));
        }
        _ => {}
    }
    if is_filesystem_write_capability(normalized_capability, &obj) {
        move_arg_alias_if_missing(&mut obj, "path", &["file", "file_path", "target"]);
        move_arg_alias_if_missing(&mut obj, "content", &["text", "data", "body"]);
        move_arg_alias_if_missing(&mut obj, "mode", &["write_mode", "writeMode"]);
    }
    Value::Object(obj)
}

fn is_filesystem_write_capability(
    normalized_capability: &str,
    obj: &serde_json::Map<String, Value>,
) -> bool {
    let action = obj
        .get("action")
        .and_then(Value::as_str)
        .map(normalize_capability_name)
        .or_else(|| {
            normalized_capability
                .rsplit_once('.')
                .map(|(_, action)| normalize_capability_name(action))
        });
    matches!(
        action.as_deref(),
        Some("write_text" | "write_file" | "append_text" | "append_file")
    )
}

fn normalize_config_basic_capability_args(normalized_capability: &str, args: Value) -> Value {
    let mut obj = match args {
        Value::Object(obj) => obj,
        other => return other,
    };
    if !is_config_basic_capability(normalized_capability) {
        return Value::Object(obj);
    }
    let action = obj
        .get("action")
        .and_then(Value::as_str)
        .map(normalize_capability_name)
        .or_else(|| {
            normalized_capability
                .rsplit_once('.')
                .map(|(_, action)| normalize_capability_name(action))
        });
    match action.as_deref() {
        Some("read_field") => {
            move_arg_alias_if_missing(&mut obj, "path", &["file", "file_path", "config_path"]);
            move_arg_alias_if_missing(&mut obj, "field_path", &["field", "key", "field_name"]);
        }
        Some("read_fields") => {
            move_arg_alias_if_missing(&mut obj, "path", &["file", "file_path", "config_path"]);
            move_arg_alias_if_missing(&mut obj, "field_paths", &["fields", "keys", "field_names"]);
        }
        Some("list_keys" | "validate" | "guard_rustclaw_config") => {
            move_arg_alias_if_missing(&mut obj, "path", &["file", "file_path", "config_path"]);
        }
        _ => {}
    }
    Value::Object(obj)
}

fn is_config_basic_capability(normalized_capability: &str) -> bool {
    normalized_capability == "config_basic"
        || normalized_capability == "config"
        || normalized_capability.starts_with("config_basic.")
        || normalized_capability.starts_with("config.")
}

fn move_arg_alias_if_missing(
    obj: &mut serde_json::Map<String, Value>,
    target: &str,
    aliases: &[&str],
) {
    if obj.get(target).is_some_and(value_is_present) {
        return;
    }
    for alias in aliases {
        let Some(value) = obj.remove(*alias) else {
            continue;
        };
        if value_is_present(&value) {
            obj.insert(target.to_string(), value);
            return;
        }
        obj.insert((*alias).to_string(), value);
    }
}

fn value_is_present(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(value) => !value.trim().is_empty(),
        Value::Array(values) => values.iter().any(value_is_present),
        Value::Object(values) => !values.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
    }
}

fn args_has_command_field(args: &Value) -> bool {
    args.as_object().is_some_and(|obj| {
        obj.get("command")
            .or_else(|| obj.get("cmd"))
            .or_else(|| obj.get("shell_command"))
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
    })
}

fn normalize_run_command_args(args: Value) -> Value {
    let mut obj = args.as_object().cloned().unwrap_or_default();
    if !obj.contains_key("command") {
        if let Some(cmd) = obj.remove("cmd").or_else(|| obj.remove("shell_command")) {
            obj.insert("command".to_string(), cmd);
        }
    }
    if obj
        .get("kind")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(normalize_capability_name)
        .is_some_and(|kind| matches!(kind.as_str(), "run_cmd" | "run_command" | "shell_run"))
    {
        obj.remove("kind");
    }
    Value::Object(obj)
}

#[cfg(test)]
#[path = "capability_resolver_tests.rs"]
mod tests;
