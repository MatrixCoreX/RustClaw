use claw_core::skill_registry::PrimaryFallbackRole;
use regex::Regex;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tracing::{info, warn};

use super::{
    attempt_ledger::build_attempt_ledger_compact, build_loop_history_compact,
    build_single_plan_prompt, build_skill_playbooks_text, build_skill_quick_index_text,
    build_turn_analysis_prompt_block, plan_step_label, AgentLoopGuardPolicy, LoopState,
    SinglePlanEnvelope, AGENT_TOOL_SPEC_PATH, LIGHTWEIGHT_EXECUTION_PROMPT_LOGICAL_PATH,
    LOOP_INCREMENTAL_PLAN_PROMPT_LOGICAL_PATH, PLAN_REPAIR_PROMPT_LOGICAL_PATH,
    SINGLE_PLAN_EXECUTION_PROMPT_LOGICAL_PATH,
};
use crate::{
    llm_gateway, plan_step_from_agent_action, AgentAction, AppState, ClaimedTask, PlanKind,
    PlanResult, RouteResult,
};

fn build_incremental_plan_prompt(
    prompt_template: &str,
    user_request: &str,
    goal: &str,
    turn_analysis: &str,
    tool_spec: &str,
    skill_playbooks: &str,
    recent_assistant_replies: &str,
    request_language_hint: &str,
    config_response_language: &str,
    round: usize,
    history_compact: &str,
    attempt_ledger: &str,
    last_round_output: &str,
    runtime_os: &str,
    runtime_shell: &str,
    workspace_root: &str,
) -> String {
    crate::render_prompt_template(
        prompt_template,
        &[
            ("__USER_REQUEST__", user_request),
            ("__GOAL__", goal),
            ("__TURN_ANALYSIS__", turn_analysis),
            ("__TOOL_SPEC__", tool_spec),
            ("__SKILL_PLAYBOOKS__", skill_playbooks),
            ("__RECENT_ASSISTANT_REPLIES__", recent_assistant_replies),
            ("__REQUEST_LANGUAGE_HINT__", request_language_hint),
            ("__CONFIG_RESPONSE_LANGUAGE__", config_response_language),
            ("__ROUND__", &round.to_string()),
            ("__HISTORY_COMPACT__", history_compact),
            ("__ATTEMPT_LEDGER__", attempt_ledger),
            ("__LAST_ROUND_OUTPUT__", last_round_output),
            ("__RUNTIME_OS__", runtime_os),
            ("__RUNTIME_SHELL__", runtime_shell),
            ("__WORKSPACE_ROOT__", workspace_root),
        ],
    )
}

fn ensure_required_contract_block_present(
    route_result: Option<&RouteResult>,
    prompt_text: &str,
) -> Result<(), String> {
    let Some(route) = route_result else {
        return Ok(());
    };
    let Some(contract_line) = crate::contract_matrix::compact_prompt_line_for_route(route) else {
        return Ok(());
    };
    if prompt_text.contains(&contract_line) {
        Ok(())
    } else {
        Err(format!(
            "prompt_budget_error: compact contract block missing from planner prompt; contract_line_hash={}",
            crate::contract_matrix::fnv1a_hex(&contract_line)
        ))
    }
}

fn runtime_os_label() -> String {
    format!(
        "{} (family={}, arch={})",
        std::env::consts::OS,
        std::env::consts::FAMILY,
        std::env::consts::ARCH
    )
}

fn runtime_shell_label() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("COMSPEC")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
        .unwrap_or_else(|| "(unknown shell)".to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlanningPromptClass {
    OpenPlanning,
    LightweightExecution,
}

impl PlanningPromptClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::OpenPlanning => "open_planning",
            Self::LightweightExecution => "lightweight_execution",
        }
    }
}

fn classify_planning_prompt_class(
    route_result: Option<&RouteResult>,
    user_text: &str,
    loop_state: &LoopState,
) -> PlanningPromptClass {
    if loop_state.round_no <= 1
        && route_result.is_some_and(|route| {
            crate::task_context_builder::uses_light_execution_context_budget(route, user_text)
        })
    {
        PlanningPromptClass::LightweightExecution
    } else {
        PlanningPromptClass::OpenPlanning
    }
}

fn build_lightweight_tool_spec(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> String {
    let mut lines = vec![
        "### LIGHT_EXECUTION_RULES".to_string(),
        "- planning_class=lightweight_execution".to_string(),
        "- Prefer one bounded local observation or direct runtime-owned step over broad multi-step planning.".to_string(),
        "- Do not inspect unrelated files, repository history, or extra skills unless the user explicitly asks for that scope.".to_string(),
        "- Preserve user-supplied concrete shell/system commands as run_cmd; do not replace explicit commands with semantic shortcut skills.".to_string(),
        "- Prefer the most specific enabled skill whose interface covers the request; use generic filesystem/system skills only when no dedicated skill fits.".to_string(),
    ];
    if let Some(route) = route_result {
        lines.push(crate::TaskContract::from_route_result(route).compact_prompt_line());
        lines.push(format!(
            "- ask_mode={} derived_route_label={} response_shape={} semantic_kind={} locator_kind={}",
            route.ask_mode.as_str(),
            route.derived_route_label(),
            route.output_contract.response_shape.as_str(),
            route.output_contract.semantic_kind.as_str(),
            route.output_contract.locator_kind.as_str(),
        ));
        if !route.output_contract.locator_hint.trim().is_empty() {
            lines.push(format!(
                "- locator_hint={}",
                route.output_contract.locator_hint.trim()
            ));
        }
    }
    if let Some(path) = auto_locator_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("- auto_locator_path={path}"));
    }
    lines.join("\n")
}

const LIGHTWEIGHT_SKILL_PLAYBOOK_MAX_CHARS: usize = 700;
const LIGHTWEIGHT_SKILL_SUMMARY_MAX_CHARS: usize = 140;

fn fallback_generated_skill_prompt_path(skill: &str) -> String {
    format!("prompts/layers/generated/skills/{skill}.md")
}

fn load_skill_prompt_body_for_planner(state: &AppState, skill: &str) -> Option<String> {
    let logical_path = state
        .skill_registry_prompt_rel_path(skill)
        .unwrap_or_else(|| fallback_generated_skill_prompt_path(skill));
    let body = crate::load_prompt_template_for_state(state, &logical_path, "").0;
    let body = if body.trim().is_empty()
        && logical_path.starts_with("prompts/layers/generated/skills/")
    {
        fs::read_to_string(state.skill_rt.workspace_root.join(&logical_path)).unwrap_or_default()
    } else {
        body
    };
    let trimmed = body.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn trim_chars_with_ellipsis(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let budget = max_chars.saturating_sub(3);
    format!("{}...", text.chars().take(budget).collect::<String>())
}

fn normalized_skill_doc_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("<!--")
        || trimmed.starts_with("```")
        || trimmed.starts_with('>')
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn lightweight_skill_summary_from_prompt(prompt_body: &str) -> String {
    prompt_body
        .lines()
        .filter_map(normalized_skill_doc_line)
        .find(|line| !line.starts_with('#'))
        .map(|line| trim_chars_with_ellipsis(&line, LIGHTWEIGHT_SKILL_SUMMARY_MAX_CHARS))
        .unwrap_or_else(|| "use when the request matches this skill's interface".to_string())
}

fn skill_doc_heading(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let text = trimmed
        .strip_prefix("## ")
        .or_else(|| trimmed.strip_prefix("### "))?
        .trim()
        .to_ascii_lowercase();
    Some(text)
}

fn lightweight_prompt_heading_is_relevant(heading: &str) -> bool {
    heading.contains("capability summary")
        || heading.contains("config entry points")
        || heading == "actions"
        || heading.contains("parameter contract")
        || heading.contains("error contract")
        || heading.contains("natural-language intent mapping")
}

fn compact_skill_playbook_from_prompt(skill: &str, prompt_body: &str) -> String {
    let mut out = Vec::new();
    let mut in_relevant_section = false;
    for line in prompt_body.lines() {
        if let Some(heading) = skill_doc_heading(line) {
            in_relevant_section = lightweight_prompt_heading_is_relevant(&heading);
            continue;
        }
        if !in_relevant_section {
            continue;
        }
        let Some(line) = normalized_skill_doc_line(line) else {
            continue;
        };
        if line.starts_with("##") {
            continue;
        }
        out.push(line);
        if out.join("\n").chars().count() >= LIGHTWEIGHT_SKILL_PLAYBOOK_MAX_CHARS {
            break;
        }
    }
    let body = if out.is_empty() {
        lightweight_skill_summary_from_prompt(prompt_body)
    } else {
        trim_chars_with_ellipsis(&out.join("\n"), LIGHTWEIGHT_SKILL_PLAYBOOK_MAX_CHARS)
    };
    format!("### {skill}\n{body}")
}

fn registry_planner_metadata_hint(state: &AppState, skill: &str) -> Option<String> {
    let manifest = state.skill_manifest(skill)?;
    let mut parts = Vec::new();
    parts.push(format!(
        "planner_kind: {}",
        manifest.planner_kind.as_token()
    ));
    if !manifest.semantic_tags.is_empty() {
        parts.push(format!(
            "semantic_tags: {}",
            manifest.semantic_tags.join(", ")
        ));
    }
    if manifest.preferred_over_run_cmd {
        parts.push("preferred_over_run_cmd: true".to_string());
    }
    if !manifest.validation_actions.is_empty() {
        parts.push(format!(
            "validation_actions: {}",
            manifest.validation_actions.join(", ")
        ));
    }
    if let Some(retryable) = manifest.retryable {
        parts.push(format!("retryable: {retryable}"));
    }
    if let Some(requires_confirmation) = manifest.requires_confirmation {
        parts.push(format!("requires_confirmation: {requires_confirmation}"));
    }
    if !manifest.confirmation_exempt_when.is_empty() {
        parts.push(format!(
            "confirmation_exempt_when: {}",
            format_confirmation_exempt_when(&manifest.confirmation_exempt_when)
        ));
    }
    parts.extend(crate::skill_availability::availability_metadata_parts(
        &crate::skill_availability::evaluate_manifest_availability(&manifest),
    ));
    if !manifest.capabilities.is_empty() {
        let capabilities = manifest
            .capabilities
            .iter()
            .map(|capability| capability.as_token())
            .collect::<Vec<_>>();
        parts.push(format!("capabilities: {}", capabilities.join(", ")));
    }
    Some(format!("Registry metadata: {}", parts.join("; ")))
}

fn format_confirmation_exempt_when(
    matchers: &[std::collections::BTreeMap<String, serde_json::Value>],
) -> String {
    matchers
        .iter()
        .take(4)
        .map(|matcher| {
            matcher
                .iter()
                .map(|(key, value)| format!("{key}={}", compact_json_value_token(value)))
                .collect::<Vec<_>>()
                .join("+")
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn compact_json_value_token(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(v) => v.to_string(),
        serde_json::Value::Number(v) => v.to_string(),
        serde_json::Value::Array(values) => values
            .iter()
            .map(compact_json_value_token)
            .collect::<Vec<_>>()
            .join("|"),
        _ => value.to_string(),
    }
}

fn build_lightweight_skill_playbooks_text(state: &AppState, task: &ClaimedTask) -> String {
    let visible = state.planner_available_skills_for_task(task);
    if visible.is_empty() {
        return "No skill playbooks configured.".to_string();
    }
    visible
        .iter()
        .map(|skill| {
            let mut section = load_skill_prompt_body_for_planner(state, skill)
                .map(|body| compact_skill_playbook_from_prompt(skill, &body))
                .unwrap_or_else(|| {
                    format!(
                        "### {skill}\nNo generated skill prompt was found; use only if the registry/interface is available at runtime."
                    )
                });
            if let Some(hint) = registry_planner_metadata_hint(state, skill) {
                section.push('\n');
                section.push_str(&hint);
            }
            section
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn build_lightweight_skill_quick_index_text(state: &AppState, task: &ClaimedTask) -> String {
    let visible = state.planner_available_skills_for_task(task);
    if visible.is_empty() {
        return "- (no enabled skills)".to_string();
    }
    visible
        .iter()
        .map(|skill| {
            let summary = load_skill_prompt_body_for_planner(state, skill)
                .map(|body| lightweight_skill_summary_from_prompt(&body))
                .unwrap_or_else(|| "generated prompt unavailable".to_string());
            let metadata = registry_planner_metadata_hint(state, skill)
                .and_then(|hint| hint.strip_prefix("Registry metadata: ").map(str::to_string));
            match metadata {
                Some(metadata) => format!("- {skill}: {summary}; {metadata}"),
                None => format!("- {skill}: {summary}"),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn round1_prompt_spec_for_class(
    planning_class: PlanningPromptClass,
) -> (&'static str, &'static str) {
    match planning_class {
        PlanningPromptClass::OpenPlanning => (
            "single_plan_execution_prompt",
            SINGLE_PLAN_EXECUTION_PROMPT_LOGICAL_PATH,
        ),
        PlanningPromptClass::LightweightExecution => (
            "lightweight_execution_prompt",
            LIGHTWEIGHT_EXECUTION_PROMPT_LOGICAL_PATH,
        ),
    }
}

fn parse_plan_action_step(step: &Value, state: &AppState) -> Option<AgentAction> {
    let raw_step = serde_json::to_string(step).ok()?;
    let normalized = crate::parse_agent_action_json_with_repair(&raw_step, state).ok()?;
    serde_json::from_value::<AgentAction>(normalized).ok()
}

fn parse_xml_tool_parameter_value(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        Value::Null
    } else if let Some(value) =
        crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<Value>(trimmed)
    {
        value
    } else {
        Value::String(trimmed.to_string())
    }
}

fn extract_xml_tool_call_steps(raw: &str) -> Vec<Value> {
    let mut steps = Vec::new();
    let mut search_from = 0usize;
    while let Some(invoke_rel) = raw[search_from..].find("<invoke name=\"") {
        let invoke_start = search_from + invoke_rel;
        let name_start = invoke_start + "<invoke name=\"".len();
        let Some(name_end_rel) = raw[name_start..].find('"') else {
            break;
        };
        let name_end = name_start + name_end_rel;
        let invoke_name = raw[name_start..name_end].trim();
        let Some(tag_end_rel) = raw[name_end..].find('>') else {
            break;
        };
        let body_start = name_end + tag_end_rel + 1;
        let Some(close_rel) = raw[body_start..].find("</invoke>") else {
            break;
        };
        let body_end = body_start + close_rel;
        let body = &raw[body_start..body_end];
        search_from = body_end + "</invoke>".len();

        let mut params = serde_json::Map::new();
        let mut param_search = 0usize;
        while let Some(param_rel) = body[param_search..].find("<parameter name=\"") {
            let param_start = param_search + param_rel;
            let name_start = param_start + "<parameter name=\"".len();
            let Some(name_end_rel) = body[name_start..].find('"') else {
                break;
            };
            let name_end = name_start + name_end_rel;
            let param_name = body[name_start..name_end].trim();
            let Some(tag_end_rel) = body[name_end..].find('>') else {
                break;
            };
            let value_start = name_end + tag_end_rel + 1;
            let Some(close_rel) = body[value_start..].find("</parameter>") else {
                break;
            };
            let value_end = value_start + close_rel;
            params.insert(
                param_name.to_string(),
                parse_xml_tool_parameter_value(&body[value_start..value_end]),
            );
            param_search = value_end + "</parameter>".len();
        }

        let step = match invoke_name {
            "call_skill" => {
                let skill = params.get("skill").and_then(|v| v.as_str()).map(str::trim);
                let args = params
                    .get("args")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                skill.map(|skill| {
                    serde_json::json!({
                        "type": "call_skill",
                        "skill": skill,
                        "args": args,
                    })
                })
            }
            "call_tool" => {
                let tool = params.get("tool").and_then(|v| v.as_str()).map(str::trim);
                let args = params
                    .get("args")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                tool.map(|tool| {
                    serde_json::json!({
                        "type": "call_tool",
                        "tool": tool,
                        "args": args,
                    })
                })
            }
            "call_capability" => {
                let capability = params
                    .get("capability")
                    .and_then(|v| v.as_str())
                    .map(str::trim);
                let args = params
                    .get("args")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                capability.map(|capability| {
                    serde_json::json!({
                        "type": "call_capability",
                        "capability": capability,
                        "args": args,
                    })
                })
            }
            other => {
                let args = params
                    .get("args")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                Some(serde_json::json!({
                    "type": "call_skill",
                    "skill": other,
                    "args": args,
                }))
            }
        };

        if let Some(step) = step {
            steps.push(step);
        }
    }
    steps
}

async fn parse_single_plan_actions(
    raw: &str,
    state: &AppState,
    task: &ClaimedTask,
) -> Option<Vec<AgentAction>> {
    let mut step_values = Vec::new();
    if let Ok(validated) = crate::prompt_utils::validate_against_schema::<Value>(
        raw,
        crate::prompt_utils::PromptSchemaId::PlanResult,
    ) {
        if !validated.raw_parse_ok || validated.schema_normalized {
            info!(
                "plan_result schema_parse_recovery task_id={} raw_parse_ok={} schema_normalized={}",
                task.task_id, validated.raw_parse_ok, validated.schema_normalized
            );
        }
        match validated.value {
            Value::Object(map) => {
                if let Some(steps) = map.get("steps").and_then(|v| v.as_array()) {
                    step_values.extend(steps.iter().cloned());
                } else {
                    step_values.push(Value::Object(map));
                }
            }
            Value::Array(arr) => step_values.extend(arr),
            other => step_values.push(other),
        }
    }
    if step_values.is_empty() {
        if let Some(value) =
            crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<Value>(raw)
        {
            match value {
                Value::Object(map) => {
                    if let Some(steps) = map.get("steps").and_then(|v| v.as_array()) {
                        step_values.extend(steps.iter().cloned());
                    } else {
                        step_values.push(Value::Object(map));
                    }
                }
                Value::Array(arr) => step_values.extend(arr),
                other => step_values.push(other),
            }
        }
    }
    if step_values.is_empty() {
        for candidate in crate::prompt_utils::extract_agent_action_objects(raw) {
            if let Ok(value) = serde_json::from_str::<Value>(&candidate) {
                step_values.push(value);
            }
        }
    }
    if step_values.is_empty() {
        step_values.extend(extract_xml_tool_call_steps(raw));
    }
    if step_values.is_empty() {
        let value = crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<Value>(raw)?;
        let env = serde_json::from_value::<SinglePlanEnvelope>(value).ok()?;
        step_values.extend(env.steps);
    }
    if step_values.is_empty() {
        return None;
    }

    let mut actions = Vec::new();
    for step in step_values {
        let Some(action) = parse_plan_action_step(&step, state) else {
            continue;
        };
        match action {
            AgentAction::Think { .. } => {}
            AgentAction::Respond { content } => actions.push(AgentAction::Respond { content }),
            _ => actions.push(action),
        }
    }
    if actions.is_empty() {
        None
    } else {
        Some(actions)
    }
}

fn build_plan_result(
    goal: &str,
    raw_plan_text: &str,
    plan_kind: PlanKind,
    actions: &[AgentAction],
) -> PlanResult {
    let mut previous_actionable_step_id: Option<String> = None;
    let mut steps = Vec::new();
    for (idx, action) in actions.iter().enumerate() {
        let step_id = format!("step_{}", idx + 1);
        let depends_on = previous_actionable_step_id
            .as_ref()
            .map(|v| vec![v.clone()])
            .unwrap_or_default();
        let why = plan_step_label(action);
        let step = plan_step_from_agent_action(action, step_id.clone(), depends_on, why);
        if !matches!(action, AgentAction::Think { .. }) {
            previous_actionable_step_id = Some(step_id);
        }
        steps.push(step);
    }
    PlanResult {
        goal: goal.to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        steps,
        planner_notes: String::new(),
        plan_kind,
        raw_plan_text: raw_plan_text.to_string(),
    }
}

fn has_executable_observation_or_action(actions: &[AgentAction]) -> bool {
    actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. }
                | AgentAction::CallTool { .. }
                | AgentAction::CallCapability { .. }
        )
    })
}

fn has_tool_or_skill_observation(actions: &[AgentAction]) -> bool {
    actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. }
                | AgentAction::CallTool { .. }
                | AgentAction::CallCapability { .. }
        )
    })
}

fn planned_action_skill_name(action: &AgentAction) -> Option<&str> {
    match action {
        AgentAction::CallSkill { skill, .. } => Some(skill.as_str()),
        AgentAction::CallTool { tool, .. } => Some(tool.as_str()),
        AgentAction::CallCapability { capability, .. } => Some(capability.as_str()),
        AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => None,
    }
}

fn contains_unavailable_skill_action(state: &AppState, actions: &[AgentAction]) -> bool {
    let enabled_skills = state.get_skills_list();
    if enabled_skills.is_empty() {
        return false;
    }
    actions.iter().any(|action| {
        let Some(skill) = planned_action_skill_name(action) else {
            return false;
        };
        let canonical = state.resolve_canonical_skill_name(skill);
        canonical.trim().is_empty() || !enabled_skills.contains(&canonical)
    })
}

fn route_semantic_tag(route_result: &RouteResult) -> Option<&'static str> {
    let tag = route_result.output_contract.semantic_kind.as_str();
    if tag == "none" || tag == "raw_command_output" {
        return None;
    }
    Some(tag)
}

fn registry_preferred_skill_names_for_route(
    state: &AppState,
    route_result: &RouteResult,
) -> Vec<String> {
    let Some(route_tag) = route_semantic_tag(route_result) else {
        return Vec::new();
    };
    let Some(registry) = state.get_skills_registry() else {
        return Vec::new();
    };
    let enabled_skills = state.get_skills_list();
    registry
        .enabled_names()
        .into_iter()
        .filter(|name| enabled_skills.is_empty() || enabled_skills.contains(name))
        .filter(|name| {
            registry.get(name).is_some_and(|entry| {
                entry.preferred_over_run_cmd
                    && entry
                        .semantic_tags
                        .iter()
                        .any(|tag| tag.trim().eq_ignore_ascii_case(route_tag))
            })
        })
        .collect()
}

#[cfg(test)]
fn registry_preferred_skill_matches_route(state: &AppState, route_result: &RouteResult) -> bool {
    !registry_preferred_skill_names_for_route(state, route_result).is_empty()
}

fn actions_use_ad_hoc_command_without_route_preferred_skill(
    state: &AppState,
    route_result: &RouteResult,
    actions: &[AgentAction],
) -> bool {
    let preferred_skills = registry_preferred_skill_names_for_route(state, route_result);
    if preferred_skills.is_empty() {
        return false;
    }
    if actions.iter().any(|action| {
        planned_action_skill_name(action).is_some_and(|skill| {
            let canonical = state.resolve_canonical_skill_name(skill);
            preferred_skills
                .iter()
                .any(|preferred| preferred.eq_ignore_ascii_case(&canonical))
        }) || action_satisfies_structured_key_listing_contract(route_result, action)
    }) {
        return false;
    }
    actions.iter().any(|action| {
        let Some(skill) = planned_action_skill_name(action) else {
            return false;
        };
        let canonical = state.resolve_canonical_skill_name(skill);
        if canonical.eq_ignore_ascii_case("run_cmd")
            && action_has_internal_literal_command_marker(action)
        {
            return false;
        }
        if action_satisfies_structured_key_listing_contract(route_result, action) {
            return false;
        }
        action_uses_generic_fallback_capability_for_preferred_route(state, &canonical)
    })
}

fn action_satisfies_structured_key_listing_contract(
    route_result: &RouteResult,
    action: &AgentAction,
) -> bool {
    if !action_is_structured_key_listing(action) {
        return false;
    }
    match route_result.output_contract.semantic_kind {
        crate::OutputSemanticKind::StructuredKeys => true,
        crate::OutputSemanticKind::FileNames => action_structured_key_listing_path(action)
            .or_else(|| {
                let hint = route_result.output_contract.locator_hint.trim();
                (!hint.is_empty()).then_some(hint)
            })
            .is_some_and(path_has_structured_document_extension),
        _ => false,
    }
}

fn action_is_structured_key_listing(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let Some(action_name) = args.get("action").and_then(Value::as_str) else {
                return false;
            };
            (skill.eq_ignore_ascii_case("config_basic")
                && action_name.eq_ignore_ascii_case("list_keys"))
                || (skill.eq_ignore_ascii_case("system_basic")
                    && action_name.eq_ignore_ascii_case("structured_keys"))
        }
        AgentAction::CallCapability { .. } => false,
        AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => false,
    }
}

fn action_structured_key_listing_path(action: &AgentAction) -> Option<&str> {
    match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. }
            if action_is_structured_key_listing(action) =>
        {
            args.get("path").and_then(Value::as_str).map(str::trim)
        }
        _ => None,
    }
    .filter(|path| !path.is_empty())
}

fn path_has_structured_document_extension(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.trim().to_ascii_lowercase())
        .is_some_and(|extension| matches!(extension.as_str(), "json" | "toml" | "yaml" | "yml"))
}

fn action_uses_generic_fallback_capability_for_preferred_route(
    state: &AppState,
    canonical_skill_name: &str,
) -> bool {
    if !canonical_skill_name.eq_ignore_ascii_case("run_cmd") {
        return false;
    }
    if let Some(registry) = state.get_skills_registry() {
        if registry.get(canonical_skill_name).is_some_and(|entry| {
            matches!(
                entry.primary_fallback_role,
                Some(PrimaryFallbackRole::Fallback)
            )
        }) {
            return true;
        }
    }

    // Compatibility for older registries without `primary_fallback_role`.
    canonical_skill_name.eq_ignore_ascii_case("run_cmd")
}

fn action_has_internal_literal_command_marker(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => args
            .get(super::CLAWD_LITERAL_COMMAND_ARG)
            .and_then(Value::as_bool)
            .unwrap_or(false),
        AgentAction::CallCapability { .. } => false,
        AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => false,
    }
}

fn has_discussion_followup_action(actions: &[AgentAction]) -> bool {
    actions.iter().any(|action| match action {
        AgentAction::Respond { .. } => true,
        AgentAction::SynthesizeAnswer { .. } => true,
        AgentAction::CallSkill { .. }
        | AgentAction::CallTool { .. }
        | AgentAction::CallCapability { .. }
        | AgentAction::Think { .. } => false,
    })
}

fn is_discussion_followup_action(action: &AgentAction) -> bool {
    match action {
        AgentAction::Respond { .. } => true,
        AgentAction::SynthesizeAnswer { .. } => true,
        AgentAction::CallSkill { .. }
        | AgentAction::CallTool { .. }
        | AgentAction::CallCapability { .. }
        | AgentAction::Think { .. } => false,
    }
}

fn synthesize_answer_requires_runtime_execution(evidence_refs: &[String]) -> bool {
    evidence_refs.len() > 1
        || evidence_refs
            .iter()
            .any(|reference| reference.trim() != "last_output")
}

fn should_preserve_terminal_followup_for_observed_finalize(action: &AgentAction) -> bool {
    match action {
        AgentAction::SynthesizeAnswer { evidence_refs } => {
            synthesize_answer_requires_runtime_execution(evidence_refs)
        }
        _ => false,
    }
}

fn has_authoritative_delivery(loop_state: &LoopState) -> bool {
    !loop_state.delivery_messages.is_empty()
        || loop_state
            .last_user_visible_respond
            .as_deref()
            .map(str::trim)
            .is_some_and(|text| !text.is_empty())
        || loop_state
            .last_publishable_synthesis_output
            .as_deref()
            .map(str::trim)
            .is_some_and(|text| !text.is_empty())
}

fn is_plain_respond_only_plan(actions: &[AgentAction]) -> Option<&str> {
    match actions {
        [AgentAction::Respond { content }] => Some(content.as_str()),
        _ => None,
    }
}

fn is_delivery_failure_terminal_reply(actions: &[AgentAction]) -> bool {
    let Some(content) = is_plain_respond_only_plan(actions) else {
        return false;
    };
    let trimmed = content.trim();
    !trimmed.is_empty() && crate::finalize::parse_delivery_token(trimmed).is_none()
}

fn missing_target_path_from_step_error(
    step: &crate::executor::StepExecutionResult,
) -> Option<String> {
    let err = step.error.as_deref()?.trim();
    if !crate::skills::is_missing_target_skill_error(&step.skill, err) {
        return None;
    }
    if let Some(path) = err.strip_prefix("__RC_READ_FILE_NOT_FOUND__:") {
        let path = path.trim();
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }
    if let Some(structured) = crate::skills::parse_structured_skill_error(err) {
        if let Some(extra) = structured.extra.as_ref() {
            for key in ["path", "target_path", "resolved_path"] {
                if let Some(path) = extra.get(key).and_then(Value::as_str).map(str::trim) {
                    if !path.is_empty() {
                        return Some(path.to_string());
                    }
                }
            }
        }
        let text = structured.error_text.trim();
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    None
}

fn terminal_reply_mentions_observed_missing_target(
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    let Some(content) = is_plain_respond_only_plan(actions) else {
        return false;
    };
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return false;
    }
    loop_state
        .executed_step_results
        .iter()
        .filter_map(missing_target_path_from_step_error)
        .any(|path| trimmed.contains(path.trim()))
}

fn route_expects_terminal_user_answer(route_result: &RouteResult) -> bool {
    if route_result.output_contract.delivery_required {
        return false;
    }
    !matches!(
        route_result.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    )
}

fn last_executable_action(actions: &[AgentAction]) -> Option<&AgentAction> {
    actions.iter().rev().find(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
        )
    })
}

fn route_explicitly_requests_raw_command_output(route_result: Option<&RouteResult>) -> bool {
    route_result.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
    })
}

fn action_supports_direct_observed_finalize(
    state: &AppState,
    route_result: Option<&RouteResult>,
    action: &AgentAction,
) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let canonical = state.resolve_canonical_skill_name(skill);
            if action_supports_read_range_direct_observed_finalize(route_result, &canonical, args) {
                return true;
            }
            if action_supports_structured_direct_observed_finalize(route_result, &canonical, args) {
                return true;
            }
            if canonical == "process_basic" {
                let action_name = args
                    .get("action")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .map(str::to_ascii_lowercase)
                    .unwrap_or_default();
                return route_result.is_some_and(|route| {
                    route.output_contract.semantic_kind == crate::OutputSemanticKind::ServiceStatus
                        && matches!(action_name.as_str(), "ps" | "port_list")
                });
            }
            if route_result.is_some_and(|route| {
                route.output_contract.requires_content_evidence
                    && route_expects_terminal_user_answer(route)
            }) {
                return false;
            }
            if canonical == "run_cmd" && route_explicitly_requests_raw_command_output(route_result)
            {
                return true;
            }
            match canonical.as_str() {
                "health_check" | "service_control" => true,
                "system_basic" => args
                    .get("action")
                    .and_then(|value| value.as_str())
                    .map(|action| {
                        matches!(
                            action.trim().to_ascii_lowercase().as_str(),
                            "info" | "diagnose_runtime"
                        )
                    })
                    .unwrap_or(false),
                _ if !state.is_builtin_skill(&canonical) => true,
                _ => false,
            }
        }
        AgentAction::SynthesizeAnswer { .. } => false,
        AgentAction::CallCapability { .. } => false,
        AgentAction::Respond { .. } | AgentAction::Think { .. } => false,
    }
}

fn action_supports_read_range_direct_observed_finalize(
    route_result: Option<&RouteResult>,
    canonical: &str,
    args: &Value,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .map(|action| action.trim().to_ascii_lowercase());
    let action = action.as_deref();
    let is_read_range = (canonical == "fs_basic" && action == Some("read_text_range"))
        || (canonical == "system_basic" && action == Some("read_range"));
    if !is_read_range
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::None
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Scalar
        )
    {
        return false;
    }
    route.ask_mode.is_plain_act()
}

fn action_supports_structured_direct_observed_finalize(
    route_result: Option<&RouteResult>,
    canonical: &str,
    args: &Value,
) -> bool {
    if route_result.is_some_and(|route| {
        !matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
                | crate::OutputSemanticKind::StructuredKeys
                | crate::OutputSemanticKind::DirectoryEntryGroups
                | crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::DirectoryNames
                | crate::OutputSemanticKind::FilePaths
                | crate::OutputSemanticKind::QuantityComparison
                | crate::OutputSemanticKind::ContentPresenceCheck
                | crate::OutputSemanticKind::ConfigValidation
                | crate::OutputSemanticKind::ConfigRiskAssessment
        )
    }) {
        return false;
    }
    let action = args
        .get("action")
        .and_then(|value| value.as_str())
        .map(|action| action.trim().to_ascii_lowercase());
    let action = action.as_deref();
    let response_shape = route_result.map(|route| route.output_contract.response_shape);
    let one_sentence = matches!(
        response_shape,
        Some(crate::OutputResponseShape::OneSentence)
    );
    match canonical {
        "config_basic" => match action {
            Some("read_field" | "read_fields") => true,
            Some("list_keys") => {
                !one_sentence
                    && route_result.is_none_or(|route| {
                        !matches!(
                            route.output_contract.semantic_kind,
                            crate::OutputSemanticKind::FileNames
                        ) || args
                            .get("path")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|path| !path.is_empty())
                            .or_else(|| {
                                let hint = route.output_contract.locator_hint.trim();
                                (!hint.is_empty()).then_some(hint)
                            })
                            .is_some_and(path_has_structured_document_extension)
                    })
            }
            Some("validate") => !one_sentence,
            _ => false,
        },
        "config_edit" => match action {
            Some("guard_config") => route_result.is_some_and(|route| {
                route.output_contract.semantic_kind
                    == crate::OutputSemanticKind::ConfigRiskAssessment
            }),
            _ => false,
        },
        "system_basic" => match action {
            Some("extract_field" | "extract_fields") => true,
            Some("structured_keys") => {
                !one_sentence
                    && route_result.is_none_or(|route| {
                        !matches!(
                            route.output_contract.semantic_kind,
                            crate::OutputSemanticKind::FileNames
                        ) || args
                            .get("path")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|path| !path.is_empty())
                            .or_else(|| {
                                let hint = route.output_contract.locator_hint.trim();
                                (!hint.is_empty()).then_some(hint)
                            })
                            .is_some_and(path_has_structured_document_extension)
                    })
            }
            Some("tree_summary") => route_result.is_some_and(|route| {
                matches!(
                    route.output_contract.semantic_kind,
                    crate::OutputSemanticKind::None
                )
            }),
            Some("dir_compare") => route_result.is_some_and(|route| {
                matches!(
                    route.output_contract.semantic_kind,
                    crate::OutputSemanticKind::None | crate::OutputSemanticKind::QuantityComparison
                )
            }),
            _ => false,
        },
        "fs_basic" => match action {
            Some("grep_text") => route_result.is_some_and(|route| {
                route.output_contract.semantic_kind
                    == crate::OutputSemanticKind::ContentPresenceCheck
            }),
            Some("find_entries") => route_result.is_some_and(|route| {
                matches!(
                    route.output_contract.semantic_kind,
                    crate::OutputSemanticKind::FileNames
                        | crate::OutputSemanticKind::DirectoryNames
                        | crate::OutputSemanticKind::FilePaths
                )
            }),
            Some("list_dir") => route_result.is_some_and(|route| {
                matches!(
                    route.output_contract.semantic_kind,
                    crate::OutputSemanticKind::FileNames
                        | crate::OutputSemanticKind::DirectoryNames
                        | crate::OutputSemanticKind::DirectoryEntryGroups
                )
            }),
            _ => false,
        },
        _ => false,
    }
}

fn observation_only_plan_can_finalize_from_direct_output(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: &[AgentAction],
) -> bool {
    if route_result.is_some_and(|route| {
        route.output_contract.requires_content_evidence
            && route_expects_terminal_user_answer(route)
            && structured_scalar_observation_units(actions) > 1
            && !last_executable_action(actions).is_some_and(action_is_structured_field_bundle_read)
    }) {
        return false;
    }
    last_executable_action(actions)
        .is_some_and(|action| action_supports_direct_observed_finalize(state, route_result, action))
}

fn action_is_structured_field_bundle_read(action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill.as_str(), args)
        }
        _ => return false,
    };
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .map(|action| action.trim().to_ascii_lowercase());
    let action = action.as_deref();
    let fields = args.get("field_paths").or_else(|| args.get("fields"));
    let field_count = string_list_from_value(fields).len();
    ((skill.eq_ignore_ascii_case("config_basic") && action == Some("read_fields"))
        || (skill.eq_ignore_ascii_case("system_basic") && action == Some("extract_fields")))
        && field_count > 0
}

fn route_uses_runtime_owned_observed_finalizer(route_result: &RouteResult) -> bool {
    if route_result.output_contract.delivery_required
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
    {
        return true;
    }
    matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
            | crate::OutputSemanticKind::HiddenEntriesCheck
            | crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::DirectoryNames
            | crate::OutputSemanticKind::FilePaths
            | crate::OutputSemanticKind::ContentPresenceCheck
            | crate::OutputSemanticKind::ServiceStatus
            | crate::OutputSemanticKind::ScalarPathOnly
            | crate::OutputSemanticKind::ExistenceWithPath
            | crate::OutputSemanticKind::GitCommitSubject
            | crate::OutputSemanticKind::GitRepositoryState
            | crate::OutputSemanticKind::StructuredKeys
            | crate::OutputSemanticKind::ArchiveList
            | crate::OutputSemanticKind::ArchiveRead
            | crate::OutputSemanticKind::ArchivePack
            | crate::OutputSemanticKind::ArchiveUnpack
            | crate::OutputSemanticKind::DockerPs
            | crate::OutputSemanticKind::DockerImages
            | crate::OutputSemanticKind::DockerLogs
            | crate::OutputSemanticKind::DockerContainerLifecycle
    )
}

fn observation_action_evidence_refs(actions: &[AgentAction]) -> Vec<String> {
    let refs = actions
        .iter()
        .enumerate()
        .filter_map(|(idx, action)| {
            matches!(
                action,
                AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
            )
            .then(|| format!("step_{}", idx + 1))
        })
        .collect::<Vec<_>>();
    match refs.as_slice() {
        [] => Vec::new(),
        [_] => vec!["last_output".to_string()],
        _ => refs,
    }
}

fn append_synthesize_for_observation_only_terminal_answer(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route_result) = route_result else {
        return actions;
    };
    if route_uses_runtime_owned_observed_finalizer(route_result)
        || !route_expects_terminal_user_answer(route_result)
        || has_authoritative_delivery(loop_state)
        || has_discussion_followup_action(&actions)
        || workspace_synthesis_needs_more_text_evidence(Some(route_result), loop_state, &actions)
        || observation_only_plan_can_finalize_from_direct_output(
            state,
            Some(route_result),
            &actions,
        )
    {
        return actions;
    }
    let evidence_refs = observation_action_evidence_refs(&actions);
    if evidence_refs.is_empty() {
        return actions;
    }
    let refs_log = evidence_refs.join(",");
    let mut rewritten = actions;
    rewritten.push(AgentAction::SynthesizeAnswer {
        evidence_refs: evidence_refs.clone(),
    });
    rewritten.push(AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    });
    info!("plan_append_synthesize_for_observation_only_terminal_answer refs={refs_log}");
    rewritten
}

fn append_respond_for_terminal_synthesize_answer(actions: Vec<AgentAction>) -> Vec<AgentAction> {
    if !matches!(actions.last(), Some(AgentAction::SynthesizeAnswer { .. })) {
        return actions;
    }
    let mut rewritten = actions;
    rewritten.push(AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    });
    info!("plan_append_respond_for_terminal_synthesize_answer");
    rewritten
}

fn replace_workspace_synthesis_respond_only_plan(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if loop_state.has_tool_or_skill_output
        || !route_needs_workspace_respond_only_default_evidence(route)
        || !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free
                | crate::OutputResponseShape::OneSentence
                | crate::OutputResponseShape::Strict
        )
        || is_plain_respond_only_plan(&actions).is_none()
    {
        return actions;
    }

    info!("plan_replace_workspace_synthesis_respond_only_with_default_evidence");
    workspace_summary_default_evidence_actions()
}

fn workspace_summary_default_evidence_actions() -> Vec<AgentAction> {
    vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "workspace_glance",
                "path": ".",
                "max_entries": 30,
            }),
        },
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: serde_json::json!({
                "action": "read_fields",
                "path": "Cargo.toml",
                "format": "toml",
                "field_paths": [
                    "workspace.package.version",
                    "package.version",
                    "workspace.package.name",
                    "package.name",
                    "workspace.package.description",
                    "package.description",
                ],
            }),
        },
        AgentAction::CallSkill {
            skill: "git_basic".to_string(),
            args: serde_json::json!({
                "action": "log",
                "n": 8,
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": "README.md",
                "mode": "head",
                "n": 40,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec![
                "step_1".to_string(),
                "step_2".to_string(),
                "step_3".to_string(),
                "step_4".to_string(),
            ],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ]
}

fn should_prefer_observed_finalize(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
) -> bool {
    let Some(route_result) = route_result else {
        return false;
    };
    !route_result.needs_clarify
        && route_result.output_contract.requires_content_evidence
        && route_expects_terminal_user_answer(route_result)
        && !has_authoritative_delivery(loop_state)
}

fn strip_terminal_discussion_for_observed_finalize(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if should_prefer_observed_finalize(route_result, loop_state)
        && has_executable_observation_or_action(&actions)
        && has_discussion_followup_action(&actions)
    {
        return actions;
    }
    if !should_prefer_observed_finalize(route_result, loop_state)
        || loop_state.has_tool_or_skill_output
        || !has_executable_observation_or_action(&actions)
        || !has_discussion_followup_action(&actions)
    {
        return actions;
    }
    let mut stripped = actions.clone();
    while stripped.last().is_some_and(is_discussion_followup_action) {
        if stripped
            .last()
            .is_some_and(should_preserve_terminal_followup_for_observed_finalize)
        {
            break;
        }
        stripped.pop();
    }
    let trailing_preserved_synthesize = stripped
        .last()
        .is_some_and(should_preserve_terminal_followup_for_observed_finalize);
    let prefix_without_terminal = if trailing_preserved_synthesize {
        &stripped[..stripped.len().saturating_sub(1)]
    } else {
        &stripped[..]
    };
    if has_executable_observation_or_action(&stripped)
        && (!has_discussion_followup_action(&stripped)
            || (trailing_preserved_synthesize
                && !has_discussion_followup_action(prefix_without_terminal)))
    {
        stripped
    } else {
        actions
    }
}

fn strip_terminal_discussion_for_scalar_path_observation(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify
        || loop_state.has_tool_or_skill_output
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::ScalarPathOnly
        || !has_tool_or_skill_observation(&actions)
        || !has_discussion_followup_action(&actions)
    {
        return actions;
    }

    let mut stripped = actions.clone();
    while stripped.last().is_some_and(is_discussion_followup_action) {
        stripped.pop();
    }
    if has_tool_or_skill_observation(&stripped) && !has_discussion_followup_action(&stripped) {
        stripped
    } else {
        actions
    }
}

fn strip_terminal_discussion_for_direct_skill_passthrough(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route_result) = route_result else {
        return actions;
    };
    if route_result.needs_clarify
        || route_result.output_contract.delivery_required
        || !has_executable_observation_or_action(&actions)
        || !has_discussion_followup_action(&actions)
    {
        return actions;
    }
    if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::FilePaths {
        return actions;
    }
    if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
        && route_result.output_contract.requires_content_evidence
        && actions
            .iter()
            .any(|action| matches!(action, AgentAction::SynthesizeAnswer { .. }))
    {
        return actions;
    }
    if has_mixed_last_output_terminal_respond(&actions) {
        return actions;
    }
    let mut stripped = actions.clone();
    while stripped.last().is_some_and(is_discussion_followup_action) {
        stripped.pop();
    }
    if !has_executable_observation_or_action(&stripped) || has_discussion_followup_action(&stripped)
    {
        return actions;
    }
    if observation_only_plan_can_finalize_from_direct_output(state, Some(route_result), &stripped) {
        stripped
    } else {
        actions
    }
}

fn delivery_success_terminal_reply(state: &AppState, actions: &[AgentAction]) -> bool {
    let Some(content) = is_plain_respond_only_plan(actions) else {
        return false;
    };
    let Some((_kind, raw_path)) = crate::finalize::parse_delivery_file_token(content) else {
        return false;
    };
    let path = raw_path.trim();
    if path.is_empty() || path.contains('\n') {
        return false;
    }
    let candidate = Path::new(path);
    let resolved = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(candidate)
    };
    resolved.is_file()
}

fn observation_only_plan_missing_user_answer(
    state: &AppState,
    route_result: &RouteResult,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    if should_prefer_observed_finalize(Some(route_result), loop_state)
        || (route_uses_runtime_owned_observed_finalizer(route_result)
            && has_executable_observation_or_action(actions))
        || observation_only_plan_can_finalize_from_direct_output(state, Some(route_result), actions)
    {
        return false;
    }
    has_executable_observation_or_action(actions)
        && !has_discussion_followup_action(actions)
        && route_expects_terminal_user_answer(route_result)
        && !has_authoritative_delivery(loop_state)
}

fn run_cmd_likely_mutates(command: &str) -> bool {
    let lower = format!(" {} ", command.to_ascii_lowercase());
    // Plain stdout commands are observations; redirection/tee below covers output writes.
    command.contains('>')
        || lower.contains(" tee ")
        || lower.contains(" sed -i")
        || lower.contains(" perl -0pi")
        || lower.contains(" perl -pi")
        || lower.contains(" cat <<")
        || lower.contains(" cp ")
        || lower.contains(" mv ")
        || lower.contains(" systemctl start")
        || lower.contains(" systemctl stop")
        || lower.contains(" systemctl restart")
        || lower.contains(" systemctl reload")
        || lower.contains(" systemctl enable")
        || lower.contains(" systemctl disable")
}

fn action_is_likely_mutating(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            match skill.trim().to_ascii_lowercase().as_str() {
                "write_file" | "remove_file" | "make_dir" => true,
                "fs_basic" => args
                    .get("action")
                    .and_then(|value| value.as_str())
                    .map(|action| {
                        matches!(
                            action.trim().to_ascii_lowercase().as_str(),
                            "write_text" | "append_text" | "make_dir" | "remove_path"
                        )
                    })
                    .unwrap_or(false),
                "service_control" => args
                    .get("action")
                    .and_then(|value| value.as_str())
                    .map(|action| {
                        matches!(
                            action.trim().to_ascii_lowercase().as_str(),
                            "start" | "stop" | "restart" | "reload" | "enable" | "disable"
                        )
                    })
                    .unwrap_or(false),
                "run_cmd" => args
                    .get("command")
                    .and_then(|value| value.as_str())
                    .map(run_cmd_likely_mutates)
                    .unwrap_or(false),
                _ => false,
            }
        }
        AgentAction::SynthesizeAnswer { .. } => false,
        AgentAction::CallCapability { .. } => false,
        AgentAction::Respond { .. } | AgentAction::Think { .. } => false,
    }
}

fn is_non_mutating_run_cmd_action(state: &AppState, action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill, args)
        }
        AgentAction::CallCapability { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => {
            return false;
        }
    };
    if state.resolve_canonical_skill_name(skill) != "run_cmd" {
        return false;
    }
    let Some(command) = run_cmd_command_from_args(args) else {
        return false;
    };
    if command.is_empty() {
        return false;
    }
    !crate::execution_recipe::classify_skill_action_effect(state, "run_cmd", args).mutates
}

fn mark_run_cmd_action_continue_on_error(action: &mut AgentAction) -> bool {
    let args = match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => args,
        AgentAction::CallCapability { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => {
            return false;
        }
    };
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    if obj.get(super::CLAWD_CONTINUE_ON_ERROR_ARG) == Some(&Value::Bool(true)) {
        return false;
    }
    obj.insert(
        super::CLAWD_CONTINUE_ON_ERROR_ARG.to_string(),
        Value::Bool(true),
    );
    true
}

fn mark_non_mutating_run_cmd_sequences_continue_on_error(
    state: &AppState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let mut actions = actions;
    let mut changed = false;
    let mut idx = 0usize;
    while idx < actions.len() {
        if !is_non_mutating_run_cmd_action(state, &actions[idx]) {
            idx += 1;
            continue;
        }
        let start = idx;
        while idx < actions.len() && is_non_mutating_run_cmd_action(state, &actions[idx]) {
            idx += 1;
        }
        if idx.saturating_sub(start) < 2 {
            continue;
        }
        for action in &mut actions[start..idx] {
            changed |= mark_run_cmd_action_continue_on_error(action);
        }
    }
    if changed {
        info!("plan_mark_run_cmd_sequence_continue_on_error");
    }
    actions
}

fn action_satisfies_recipe_profile_validation(
    state: &AppState,
    loop_state: &LoopState,
    action: &AgentAction,
) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            crate::execution_recipe::validation_satisfies_recipe_profile(
                loop_state.execution_recipe,
                state,
                skill,
                args,
            )
        }
        AgentAction::SynthesizeAnswer { .. } => false,
        AgentAction::CallCapability { .. } => false,
        AgentAction::Respond { .. } | AgentAction::Think { .. } => false,
    }
}

fn actions_missing_recipe_profile_validation(
    state: &AppState,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    if !loop_state.execution_recipe.validation_required
        || !matches!(
            loop_state.execution_recipe.profile,
            crate::execution_recipe::ExecutionRecipeProfile::ConfigChange
                | crate::execution_recipe::ExecutionRecipeProfile::CodeChange
                | crate::execution_recipe::ExecutionRecipeProfile::SkillAuthoring
        )
    {
        return false;
    }
    let mut saw_mutation = loop_state.execution_recipe.saw_mutation;
    let mut saw_profile_validation = loop_state.execution_recipe.saw_validation
        && !matches!(
            loop_state.execution_recipe.profile,
            crate::execution_recipe::ExecutionRecipeProfile::ConfigChange
                | crate::execution_recipe::ExecutionRecipeProfile::CodeChange
                | crate::execution_recipe::ExecutionRecipeProfile::SkillAuthoring
        );
    for action in actions {
        if action_is_likely_mutating(action) {
            saw_mutation = true;
            saw_profile_validation = false;
            continue;
        }
        if saw_mutation && action_satisfies_recipe_profile_validation(state, loop_state, action) {
            saw_profile_validation = true;
        }
    }
    saw_mutation && !saw_profile_validation
}

fn actions_violate_recipe_target_scope(
    state: &AppState,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    match loop_state.execution_recipe.target_scope {
        crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo => {
            actions.iter().any(|action| match action {
                AgentAction::CallSkill { skill, args }
                | AgentAction::CallTool { tool: skill, args } => {
                    crate::execution_recipe::action_conflicts_with_recipe_target_scope(
                        loop_state.execution_recipe,
                        state,
                        skill,
                        args,
                    )
                }
                AgentAction::SynthesizeAnswer { .. } => false,
                AgentAction::CallCapability { .. } => false,
                AgentAction::Respond { .. } | AgentAction::Think { .. } => false,
            })
        }
        crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace => {
            let mut saw_external_target = loop_state.execution_recipe.saw_external_target;
            let mut saw_scope_conflict = false;
            for action in actions {
                match action {
                    AgentAction::CallSkill { skill, args }
                    | AgentAction::CallTool { tool: skill, args } => {
                        if crate::execution_recipe::action_targets_external_workspace(
                            state, skill, args,
                        ) {
                            saw_external_target = true;
                        }
                        if crate::execution_recipe::action_conflicts_with_recipe_target_scope(
                            loop_state.execution_recipe,
                            state,
                            skill,
                            args,
                        ) {
                            saw_scope_conflict = true;
                        }
                    }
                    AgentAction::SynthesizeAnswer { .. } => {}
                    AgentAction::CallCapability { .. } => {}
                    AgentAction::Respond { .. } | AgentAction::Think { .. } => {}
                }
            }
            saw_scope_conflict || !saw_external_target
        }
        crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield => {
            !loop_state.execution_recipe.saw_greenfield_creation
                && !actions.iter().any(|action| match action {
                    AgentAction::CallSkill { skill, args }
                    | AgentAction::CallTool { tool: skill, args } => {
                        crate::execution_recipe::action_satisfies_greenfield_creation(
                            state, skill, args,
                        )
                    }
                    AgentAction::SynthesizeAnswer { .. } => false,
                    AgentAction::CallCapability { .. } => false,
                    AgentAction::Respond { .. } | AgentAction::Think { .. } => false,
                })
        }
        crate::execution_recipe::ExecutionRecipeTargetScope::Unknown
        | crate::execution_recipe::ExecutionRecipeTargetScope::System => false,
    }
}

fn should_force_actionable_plan_repair(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    let Some(route_result) = route_result else {
        return false;
    };
    if route_result.needs_clarify {
        return false;
    }
    if route_result.output_contract.delivery_required
        && !loop_state.has_tool_or_skill_output
        && is_delivery_failure_terminal_reply(actions)
    {
        return false;
    }
    if route_result.output_contract.delivery_required
        && !loop_state.has_tool_or_skill_output
        && delivery_success_terminal_reply(state, actions)
    {
        return false;
    }
    if loop_state.execution_recipe.is_active()
        && matches!(
            loop_state.execution_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Apply
        )
        && !actions.iter().any(action_is_likely_mutating)
    {
        return true;
    }
    if actions_missing_recipe_profile_validation(state, loop_state, actions) {
        return true;
    }
    if actions_violate_recipe_target_scope(state, loop_state, actions) {
        return true;
    }
    if contains_unavailable_skill_action(state, actions) {
        return true;
    }
    if !loop_state.execution_recipe.is_active()
        && terminal_reply_mentions_observed_missing_target(loop_state, actions)
    {
        return false;
    }
    if structured_scalar_compare_missing_required_extracts_for_round(
        route_result,
        loop_state,
        actions,
    ) {
        return true;
    }
    if actions_use_ad_hoc_command_without_route_preferred_skill(state, route_result, actions) {
        return true;
    }
    if no_content_evidence_execute_route_read_only_file_plan_requires_repair(
        state,
        Some(route_result),
        loop_state,
        actions,
    ) {
        return true;
    }
    if plain_act_filesystem_text_read_only_plan_requires_repair(
        state,
        Some(route_result),
        loop_state,
        actions,
    ) {
        return true;
    }
    if content_evidence_plan_only_has_locator_observation(route_result, loop_state, actions) {
        return true;
    }
    if scalar_count_plan_uses_listing_instead_of_structured_count(
        state,
        route_result,
        loop_state,
        actions,
    ) {
        return true;
    }
    let lightweight_route_has_executable_plan =
        route_qualifies_for_lightweight_repair_skip(Some(route_result))
            && !loop_state.has_tool_or_skill_output
            && has_executable_observation_or_action(actions);
    if lightweight_route_has_executable_plan
        && !observation_only_plan_missing_user_answer(state, route_result, loop_state, actions)
    {
        return false;
    }
    if observation_only_plan_missing_user_answer(state, route_result, loop_state, actions) {
        return true;
    }
    if has_executable_observation_or_action(actions) {
        return false;
    }
    if has_discussion_followup_action(actions) && loop_state.has_tool_or_skill_output {
        return false;
    }
    let requires_action_before_reply =
        !loop_state.has_tool_or_skill_output && route_result.is_execute_gate();
    route_result.output_contract.requires_content_evidence || requires_action_before_reply
}

fn scalar_count_plan_uses_listing_instead_of_structured_count(
    state: &AppState,
    route_result: &RouteResult,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    if loop_state.has_tool_or_skill_output
        || route_result.needs_clarify
        || route_result.output_contract.delivery_required
        || route_result.output_contract.semantic_kind != crate::OutputSemanticKind::ScalarCount
    {
        return false;
    }
    let saw_listing = actions
        .iter()
        .any(|action| action_is_directory_listing_plan_action(state, action));
    let saw_structured_count = actions
        .iter()
        .any(|action| action_is_structured_count_plan_action(state, action));
    saw_listing && !saw_structured_count
}

fn action_is_directory_listing_plan_action(state: &AppState, action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (state.resolve_canonical_skill_name(skill), args)
        }
        _ => return false,
    };
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    skill == "list_dir"
        || (skill == "fs_basic" && action_name.eq_ignore_ascii_case("list_dir"))
        || (skill == "system_basic" && action_name.eq_ignore_ascii_case("inventory_dir"))
}

fn action_is_structured_count_plan_action(state: &AppState, action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (state.resolve_canonical_skill_name(skill), args)
        }
        AgentAction::CallCapability { capability, .. } => {
            return capability.eq_ignore_ascii_case("filesystem.count_entries");
        }
        _ => return false,
    };
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    (skill == "fs_basic" && action_name.eq_ignore_ascii_case("count_entries"))
        || (skill == "system_basic" && action_name.eq_ignore_ascii_case("count_inventory"))
}

fn route_qualifies_for_lightweight_repair_skip(route_result: Option<&RouteResult>) -> bool {
    route_result.is_some_and(|route| {
        crate::task_context_builder::uses_light_execution_context_budget(
            route,
            &route.resolved_intent,
        )
    })
}

async fn repair_plan_actions(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    turn_analysis: &str,
    user_text: &str,
    repair_reason: &str,
    tool_spec: &str,
    skill_playbooks: &str,
    attempt_ledger: &str,
    raw_plan: &str,
    round_no: usize,
) -> Result<String, String> {
    let runtime_os = runtime_os_label();
    let runtime_shell = runtime_shell_label();
    let workspace_root = state.skill_rt.workspace_root.display().to_string();
    let resolved_prompt = crate::bootstrap::load_required_prompt_template_for_state_with_meta(
        state,
        PLAN_REPAIR_PROMPT_LOGICAL_PATH,
    )
    .map_err(|e| e.to_string())?;
    let prompt_template = resolved_prompt.template;
    let prompt_source = resolved_prompt.source;
    let prompt_version = resolved_prompt.version;
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_text);
    let user_request_for_prompt =
        crate::language_policy::task_user_request_for_prompt(task, user_text);
    let prompt = crate::render_prompt_template(
        &prompt_template,
        &[
            ("__GOAL__", goal),
            ("__TURN_ANALYSIS__", turn_analysis),
            ("__USER_REQUEST__", &user_request_for_prompt),
            ("__REPAIR_REASON__", repair_reason),
            ("__TOOL_SPEC__", tool_spec),
            ("__SKILL_PLAYBOOKS__", skill_playbooks),
            ("__ATTEMPT_LEDGER__", attempt_ledger),
            ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.policy.command_intent.default_locale,
            ),
            ("__RUNTIME_OS__", &runtime_os),
            ("__RUNTIME_SHELL__", &runtime_shell),
            ("__WORKSPACE_ROOT__", &workspace_root),
            ("__RAW_PLAN__", raw_plan),
        ],
    );
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "plan_repair_prompt",
        &prompt_source,
        prompt_version.as_deref(),
        Some(round_no),
    );
    let repaired =
        llm_gateway::run_with_fallback_with_prompt_source(state, task, &prompt, &prompt_source)
            .await?;
    info!(
        "plan_llm_repair_response task_id={} round={} raw={}",
        task.task_id,
        round_no,
        crate::truncate_for_log(&repaired)
    );
    Ok(repaired)
}

fn plan_repair_reason(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    initial_actions: Option<&[AgentAction]>,
) -> &'static str {
    let Some(actions) = initial_actions else {
        return "plan_parse_failed";
    };
    if actions_violate_recipe_target_scope(state, loop_state, actions) {
        return match loop_state.execution_recipe.target_scope {
            crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo => {
                "current_repo_scope_rejects_external_target"
            }
            crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace => {
                "external_workspace_requires_explicit_target"
            }
            crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield => {
                "greenfield_requires_artifact_creation"
            }
            crate::execution_recipe::ExecutionRecipeTargetScope::Unknown
            | crate::execution_recipe::ExecutionRecipeTargetScope::System => {
                "ops_closed_loop_requires_scope_alignment"
            }
        };
    }
    if loop_state.execution_recipe.is_active()
        && matches!(
            loop_state.execution_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Apply
        )
        && !actions.iter().any(action_is_likely_mutating)
    {
        return "ops_closed_loop_apply_requires_mutation";
    }
    if actions_missing_recipe_profile_validation(state, loop_state, actions) {
        return match loop_state.execution_recipe.profile {
            crate::execution_recipe::ExecutionRecipeProfile::ConfigChange => {
                "config_change_requires_post_change_validation"
            }
            crate::execution_recipe::ExecutionRecipeProfile::CodeChange => {
                "code_change_requires_verification"
            }
            crate::execution_recipe::ExecutionRecipeProfile::SkillAuthoring => {
                "skill_authoring_requires_integration_validation"
            }
            _ => "ops_closed_loop_requires_validation",
        };
    }
    if contains_unavailable_skill_action(state, actions) {
        return "unavailable_skill_requires_replan";
    }
    let Some(route_result) = route_result else {
        return "non_actionable_plan_for_current_route";
    };
    if structured_scalar_compare_missing_required_extracts_for_round(
        route_result,
        loop_state,
        actions,
    ) {
        return "structured_scalar_compare_requires_extract_fields";
    }
    if actions_use_ad_hoc_command_without_route_preferred_skill(state, route_result, actions) {
        return "preferred_skill_required_for_semantic_route";
    }
    if no_content_evidence_execute_route_read_only_file_plan_requires_repair(
        state,
        Some(route_result),
        loop_state,
        actions,
    ) {
        return "execute_route_requires_non_readonly_file_plan";
    }
    if plain_act_filesystem_text_read_only_plan_requires_repair(
        state,
        Some(route_result),
        loop_state,
        actions,
    ) {
        return "plain_act_file_action_requires_non_readonly_plan";
    }
    if content_evidence_plan_only_has_locator_observation(route_result, loop_state, actions) {
        return "content_evidence_requires_content_observation";
    }
    if scalar_count_plan_uses_listing_instead_of_structured_count(
        state,
        route_result,
        loop_state,
        actions,
    ) {
        return "scalar_count_requires_structured_count_action";
    }
    if observation_only_plan_missing_user_answer(state, route_result, loop_state, actions) {
        return "plan_missing_terminal_user_answer";
    }
    "non_actionable_plan_for_current_route"
}

fn can_fallback_to_initial_plan_after_repair_failure(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    let Some(route_result) = route_result else {
        return false;
    };
    let has_terminal_answer = has_discussion_followup_action(actions);
    let fallback_shape_is_safe = if has_terminal_answer {
        !observation_only_plan_missing_user_answer(state, route_result, loop_state, actions)
            && !content_evidence_plan_only_has_locator_observation(
                route_result,
                loop_state,
                actions,
            )
    } else {
        true
    };
    !route_result.needs_clarify
        && !loop_state.has_tool_or_skill_output
        && !contains_unavailable_skill_action(state, actions)
        && !structured_scalar_compare_missing_required_extracts_for_round(
            route_result,
            loop_state,
            actions,
        )
        && !no_content_evidence_execute_route_read_only_file_plan_requires_repair(
            state,
            Some(route_result),
            loop_state,
            actions,
        )
        && !plain_act_filesystem_text_read_only_plan_requires_repair(
            state,
            Some(route_result),
            loop_state,
            actions,
        )
        && !scalar_count_plan_uses_listing_instead_of_structured_count(
            state,
            route_result,
            loop_state,
            actions,
        )
        && (!actions_use_ad_hoc_command_without_route_preferred_skill(state, route_result, actions)
            || safe_observation_run_cmd_plan_can_fallback(state, Some(route_result), actions))
        && has_executable_observation_or_action(actions)
        && fallback_shape_is_safe
}

fn safe_observation_run_cmd_plan_can_fallback(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: &[AgentAction],
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if route.needs_clarify
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
    {
        return false;
    }

    let mut saw_run_cmd = false;
    for action in actions {
        match action {
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. } => {
                if !is_non_mutating_run_cmd_action(state, action) {
                    return false;
                }
                saw_run_cmd = true;
            }
            AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. } => {}
            AgentAction::CallCapability { .. } | AgentAction::Think { .. } => return false,
        }
    }
    saw_run_cmd
}

fn action_is_filesystem_text_read_observation(state: &AppState, action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill, args)
        }
        AgentAction::SynthesizeAnswer { .. }
        | AgentAction::CallCapability { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => {
            return false;
        }
    };
    let canonical = state.resolve_canonical_skill_name(skill);
    match canonical.as_str() {
        "read_file" => true,
        "fs_basic" => args
            .get("action")
            .and_then(Value::as_str)
            .map(|action| action.trim().eq_ignore_ascii_case("read_text_range"))
            .unwrap_or(false),
        "system_basic" => args
            .get("action")
            .and_then(Value::as_str)
            .map(|action| action.trim().eq_ignore_ascii_case("read_range"))
            .unwrap_or(false),
        "doc_parse" => args
            .get("path")
            .and_then(Value::as_str)
            .map(|path| !path.trim().is_empty())
            .unwrap_or(false),
        _ => false,
    }
}

fn no_content_evidence_execute_route_read_only_file_plan_requires_repair(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if route.needs_clarify
        || loop_state.has_tool_or_skill_output
        || !route.is_execute_gate()
        || route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !(route_locator_hint_is_path_like(route) || actions.iter().any(action_has_path_like_arg))
        || actions.iter().any(action_is_likely_mutating)
    {
        return false;
    }
    let executable_actions = actions.iter().filter(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
        )
    });
    let mut saw_read = false;
    for action in executable_actions {
        if !action_is_filesystem_text_read_observation(state, action) {
            return false;
        }
        saw_read = true;
    }
    saw_read
}

fn plain_act_filesystem_text_read_only_plan_requires_repair(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if route.needs_clarify
        || loop_state.has_tool_or_skill_output
        || !route.is_execute_gate()
        || !route.ask_mode.is_plain_act()
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::None
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        || !(route_locator_hint_is_path_like(route) || actions.iter().any(action_has_path_like_arg))
        || actions.iter().any(action_is_likely_mutating)
    {
        return false;
    }
    if crate::task_context_builder::uses_light_execution_context_budget(
        route,
        &route.resolved_intent,
    ) && observation_only_plan_can_finalize_from_direct_output(state, Some(route), actions)
    {
        return false;
    }
    let executable_actions = actions.iter().filter(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
        )
    });
    let mut saw_read = false;
    for action in executable_actions {
        if !action_is_filesystem_text_read_observation(state, action) {
            return false;
        }
        saw_read = true;
    }
    saw_read
}

fn action_has_path_like_arg(action: &AgentAction) -> bool {
    let args = match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => args,
        AgentAction::SynthesizeAnswer { .. }
        | AgentAction::CallCapability { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => {
            return false;
        }
    };
    action_path_arg(args).is_some_and(|path| {
        let value = path.trim();
        !value.is_empty()
            && (Path::new(value).is_absolute()
                || value.contains('/')
                || value.contains('\\')
                || value.starts_with('.')
                || Path::new(value).extension().is_some())
    })
}

fn scalar_path_auto_locator_observation_plan(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if route.needs_clarify
        || route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::ScalarPathOnly
    {
        return None;
    }
    let hint = route.output_contract.locator_hint.trim().to_string();
    let path = (!hint.is_empty() && Path::new(&hint).exists())
        .then_some(hint)
        .or_else(|| {
            auto_locator_path
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_string)
        })?;
    let path = path.as_str();
    let resolved_path = Path::new(path);
    if !resolved_path.exists() {
        return None;
    }
    let current_workspace_directory = route.output_contract.locator_kind
        == crate::OutputLocatorKind::CurrentWorkspace
        && route.output_contract.locator_hint.trim().is_empty();
    if resolved_path.is_dir() && !current_workspace_directory {
        return None;
    }
    Some(vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "paths": [path],
            "include_missing": true,
        }),
    }])
}

fn route_requires_scalar_content_observation(route: &RouteResult) -> bool {
    route.output_contract.requires_content_evidence
        || route
            .route_reason
            .contains("execution_required_read_file_extract_scalar")
        || route
            .route_reason
            .contains("request_requires_fresh_file_observation_to_extract_title")
}

fn scalar_content_auto_locator_observation_plan(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if route.needs_clarify
        || !route_requires_scalar_content_observation(route)
        || route.output_contract.delivery_required
        || !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        )
        || matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::RawCommandOutput
                | crate::OutputSemanticKind::ScalarPathOnly
                | crate::OutputSemanticKind::ExistenceWithPath
                | crate::OutputSemanticKind::ExistenceWithPathSummary
                | crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::ContentExcerptWithSummary
                | crate::OutputSemanticKind::ScalarCount
                | crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::DirectoryNames
                | crate::OutputSemanticKind::FilePaths
                | crate::OutputSemanticKind::DirectoryEntryGroups
        )
    {
        return None;
    }
    let path = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or_else(|| {
            let hint = route.output_contract.locator_hint.trim();
            (!hint.is_empty()).then_some(hint)
        })?;
    if !Path::new(path).is_file() {
        return None;
    }
    if is_supported_archive_path(path) {
        return None;
    }
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::ConfigValidation {
        return Some(vec![config_basic_validate_action(path.to_string())]);
    }
    Some(vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": path,
                "mode": "head",
                "n": 120
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ])
}

fn scalar_content_auto_locator_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let actions = scalar_content_auto_locator_observation_plan(route_result, auto_locator_path)?;
    let actions = normalize_planned_actions_with_original_and_context(
        state,
        route_result,
        loop_state,
        user_text,
        original_user_text,
        Some(goal),
        auto_locator_path,
        actions,
    );
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn active_task_append_current_locator_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    let route = route_result?;
    let analysis = turn_analysis?;
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || route.needs_clarify
        || !route.is_execute_gate()
        || !route.ask_mode.is_plain_act()
        || route.output_contract.delivery_required
        || analysis.turn_type != Some(crate::intent_router::TurnType::TaskAppend)
        || analysis.target_task_policy != Some(crate::intent_router::TargetTaskPolicy::ReuseActive)
    {
        return None;
    }
    let state_patch = analysis.state_patch.as_ref()?;
    let target = state_patch
        .get("deictic_reference")
        .and_then(Value::as_object)
        .and_then(|value| value.get("target"))
        .and_then(Value::as_str)
        .map(str::trim);
    if target != Some("current_turn_locator") {
        return None;
    }
    let literals = state_patch
        .get("required_content_literals")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if literals.is_empty() {
        return None;
    }
    let path = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    let path_obj = Path::new(path);
    if path_obj.is_dir() {
        return None;
    }
    let mut content = literals.join("\n");
    content.push('\n');
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "append_text",
                "path": path,
                "content": content,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn file_facts_auto_locator_observation_plan(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if route.needs_clarify
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None | crate::OutputSemanticKind::QuantityComparison
        )
        || (!route_expects_terminal_user_answer(route)
            && !(route.output_contract.semantic_kind
                == crate::OutputSemanticKind::QuantityComparison
                && route.output_contract.response_shape == crate::OutputResponseShape::Scalar))
        || route.output_contract.response_shape == crate::OutputResponseShape::FileToken
        || (matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
        ) && route.output_contract.semantic_kind
            != crate::OutputSemanticKind::QuantityComparison)
    {
        return None;
    }
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison
        && crate::task_contract::target_locators_for_route(route).len() > 1
    {
        return None;
    }
    let path = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    let path_ref = Path::new(path);
    let quantity_metadata_target = route.output_contract.semantic_kind
        == crate::OutputSemanticKind::QuantityComparison
        && path_ref.exists();
    if !(path_ref.is_file() || quantity_metadata_target) || is_supported_archive_path(path) {
        return None;
    }
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison
        && matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::Strict
        )
        && path_ref.is_dir()
    {
        let max_entries = contract_hint_selector_limit(&route.resolved_intent)
            .or_else(|| first_ascii_integer_limit(&route.resolved_intent))
            .or_else(|| first_ascii_integer_limit(&route.route_reason));
        if route.output_contract.response_shape == crate::OutputResponseShape::Strict
            && max_entries.is_none()
            && contract_hint_selector_extension(&route.resolved_intent).is_none()
            && contract_hint_selector_sort_by(&route.resolved_intent).is_none()
        {
            // A strict single-target quantity contract without selector metadata is a
            // path metadata request, not a ranked directory inventory request.
        } else {
            let max_entries = max_entries.unwrap_or(50);
            return Some(vec![
                AgentAction::CallTool {
                    tool: "fs_basic".to_string(),
                    args: serde_json::json!({
                        "action": "list_dir",
                        "path": path,
                        "files_only": true,
                        "names_only": false,
                        "sort_by": "size_desc",
                        "max_entries": max_entries,
                    }),
                },
                AgentAction::SynthesizeAnswer {
                    evidence_refs: vec!["last_output".to_string()],
                },
                AgentAction::Respond {
                    content: "{{last_output}}".to_string(),
                },
            ]);
        }
    }
    let targets = vec![path.to_string()];
    let mut actions = vec![fs_basic_stat_paths_action_for_explicit_targets(&targets)];
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison
        && path_ref.is_dir()
    {
        actions.push(AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "count_entries",
                "path": path,
                "recursive": true,
                "count_files": true,
                "count_dirs": true,
            }),
        });
    }
    let evidence_refs = (1..=actions.len())
        .map(|idx| format!("step_{idx}"))
        .collect::<Vec<_>>();
    actions.push(AgentAction::SynthesizeAnswer { evidence_refs });
    actions.push(AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    });
    Some(actions)
}

fn resolve_existing_metadata_locator_path(workspace_root: &Path, raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let raw_path = Path::new(raw);
    let candidate = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        workspace_root.join(raw_path)
    };
    if !candidate.exists() {
        return None;
    }
    Some(
        candidate
            .canonicalize()
            .unwrap_or(candidate)
            .display()
            .to_string(),
    )
}

fn file_facts_auto_locator_target_path(
    workspace_root: &Path,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    if let Some(path) = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        return Some(path.to_string());
    }
    let route = route_result?;
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::QuantityComparison {
        return None;
    }
    let targets = route_locator_targets(route);
    if targets.len() > 1 {
        return None;
    }
    if let Some(target) = targets.first() {
        if let Some(path) = resolve_existing_metadata_locator_path(workspace_root, target) {
            return Some(path);
        }
    }
    let hint = route.output_contract.locator_hint.trim();
    if !hint.is_empty() {
        if let Some(path) = resolve_existing_metadata_locator_path(workspace_root, hint) {
            return Some(path);
        }
    }
    if route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace {
        return Some(
            workspace_root
                .canonicalize()
                .unwrap_or_else(|_| workspace_root.to_path_buf())
                .display()
                .to_string(),
        );
    }
    None
}

fn file_facts_auto_locator_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let target = file_facts_auto_locator_target_path(
        &state.skill_rt.workspace_root,
        route_result,
        auto_locator_path,
    )?;
    let actions = file_facts_auto_locator_observation_plan(route_result, Some(&target))?;
    let actions = normalize_planned_actions_with_original_and_context(
        state,
        route_result,
        loop_state,
        user_text,
        original_user_text,
        Some(goal),
        Some(&target),
        actions,
    );
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn generic_directory_auto_locator_observation_plan(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !route_expects_terminal_user_answer(route)
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
        || !matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
                | crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::DirectoryNames
                | crate::OutputSemanticKind::DirectoryEntryGroups
                | crate::OutputSemanticKind::FilePaths
                | crate::OutputSemanticKind::DirectoryPurposeSummary
                | crate::OutputSemanticKind::WorkspaceProjectSummary
        )
    {
        return None;
    }

    let path = route_directory_locator_path(route, auto_locator_path)?;
    if !Path::new(&path).is_dir() {
        return None;
    }

    let mut args = serde_json::json!({
        "action": "list_dir",
        "path": path,
        "names_only": false,
        "max_entries": 1000,
        "sort_by": if route.output_contract.semantic_kind == crate::OutputSemanticKind::DirectoryEntryGroups {
            "mtime_desc"
        } else {
            "size_desc"
        },
    });
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::FileNames
        || route.output_contract.semantic_kind == crate::OutputSemanticKind::FilePaths
    {
        args["files_only"] = Value::Bool(true);
    } else if route.output_contract.semantic_kind == crate::OutputSemanticKind::DirectoryNames {
        args["dirs_only"] = Value::Bool(true);
    }

    Some(vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args,
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ])
}

fn directory_entry_groups_auto_locator_observation_plan(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if route.needs_clarify
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::DirectoryEntryGroups
    {
        return None;
    }
    let path = route_directory_locator_path(route, auto_locator_path)?;
    if !Path::new(&path).is_dir() {
        return None;
    }
    Some(vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "list_dir",
            "path": path,
            "names_only": false,
            "max_entries": 1000,
            "sort_by": "mtime_desc",
        }),
    }])
}

fn directory_entry_groups_auto_locator_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let actions =
        directory_entry_groups_auto_locator_observation_plan(route_result, auto_locator_path)?;
    let actions = normalize_planned_actions_with_original_and_context(
        state,
        route_result,
        loop_state,
        user_text,
        original_user_text,
        Some(goal),
        auto_locator_path,
        actions,
    );
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn directory_tree_auto_locator_observation_plan(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if route.needs_clarify
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !route_expects_terminal_user_answer(route)
        || !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free
                | crate::OutputResponseShape::OneSentence
                | crate::OutputResponseShape::Strict
        )
        || !matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
                | crate::OutputSemanticKind::DirectoryPurposeSummary
                | crate::OutputSemanticKind::WorkspaceProjectSummary
        )
    {
        return None;
    }
    if crate::task_contract::target_locators_for_route(route).len() > 1 {
        return None;
    }
    if directory_purpose_extension_locator(route).is_some() {
        return None;
    }
    let path = route_directory_locator_path(route, auto_locator_path)?;
    if !Path::new(&path).is_dir() {
        return None;
    }
    let tree_summary = AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "tree_summary",
            "path": path,
            "max_depth": 2,
            "max_children_per_dir": 12,
            "include_hidden": false,
        }),
    };
    if matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::DirectoryPurposeSummary
            | crate::OutputSemanticKind::WorkspaceProjectSummary
    ) {
        Some(vec![
            tree_summary,
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ])
    } else {
        Some(vec![tree_summary])
    }
}

fn directory_tree_auto_locator_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    if crate::intent::surface_signals::inline_json_transform_request(user_text)
        || original_user_text
            .is_some_and(crate::intent::surface_signals::inline_json_transform_request)
    {
        return None;
    }
    let actions = directory_tree_auto_locator_observation_plan(route_result, auto_locator_path)?;
    let actions = normalize_planned_actions_with_original_and_context(
        state,
        route_result,
        loop_state,
        user_text,
        original_user_text,
        Some(goal),
        auto_locator_path,
        actions,
    );
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn directory_purpose_extension_locator(route: &RouteResult) -> Option<String> {
    if route.needs_clarify
        || !route.is_execute_gate()
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::DirectoryPurposeSummary
        || !matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::CurrentWorkspace | crate::OutputLocatorKind::Path
        )
    {
        return None;
    }
    extension_from_globish_pattern(route.output_contract.locator_hint.trim())
}

fn directory_purpose_extension_inventory_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let route = route_result?;
    let ext = directory_purpose_extension_locator(route)?;
    let root = route_directory_locator_path(route, auto_locator_path)?;
    if !Path::new(&root).is_dir() {
        return None;
    }
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "find_entries",
            "root": root,
            "ext": ext,
            "target_kind": "file",
            "max_results": 100,
        }),
    }];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn step_output_action(value: &Value) -> Option<String> {
    value
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|action| !action.is_empty())
        .map(|action| action.to_ascii_lowercase())
}

fn executed_step_is_successful_text_read(step: &crate::executor::StepExecutionResult) -> bool {
    if !step.is_ok() {
        return false;
    }
    if step.skill.eq_ignore_ascii_case("read_file") || step.skill.eq_ignore_ascii_case("doc_parse")
    {
        return step
            .output
            .as_deref()
            .map(str::trim)
            .is_some_and(|output| !output.is_empty());
    }
    if !(step.skill.eq_ignore_ascii_case("fs_basic")
        || step.skill.eq_ignore_ascii_case("system_basic"))
    {
        return false;
    }
    step.output
        .as_deref()
        .and_then(|output| serde_json::from_str::<Value>(output).ok())
        .and_then(|value| step_output_action(&value))
        .is_some_and(|action| action == "read_text_range" || action == "read_range")
}

fn executed_find_entries_candidate_paths(
    step: &crate::executor::StepExecutionResult,
) -> Vec<String> {
    if !step.is_ok()
        || !(step.skill.eq_ignore_ascii_case("fs_basic")
            || step.skill.eq_ignore_ascii_case("fs_search"))
    {
        return Vec::new();
    }
    let Some(value) = step
        .output
        .as_deref()
        .and_then(|output| serde_json::from_str::<Value>(output).ok())
    else {
        return Vec::new();
    };
    let Some(action) = step_output_action(&value) else {
        return Vec::new();
    };
    if !matches!(action.as_str(), "find_entries" | "find_ext" | "find_name") {
        return Vec::new();
    }
    value
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn safe_representative_find_result_paths(root: &str, candidates: Vec<String>) -> Vec<String> {
    let root_path = Path::new(root);
    let canonical_root = root_path
        .canonicalize()
        .unwrap_or_else(|_| root_path.to_path_buf());
    let mut selected = Vec::new();
    for candidate in candidates {
        if selected.len() >= 3 {
            break;
        }
        if candidate.contains('\0') {
            continue;
        }
        let raw = Path::new(&candidate);
        if raw.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        }) {
            continue;
        }
        let full_path = if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            root_path.join(raw)
        };
        let canonical_candidate = full_path
            .canonicalize()
            .unwrap_or_else(|_| full_path.clone());
        if !canonical_candidate.starts_with(&canonical_root) || !canonical_candidate.is_file() {
            continue;
        }
        let read_path = canonical_candidate.display().to_string();
        if !selected.iter().any(|existing| existing == &read_path) {
            selected.push(read_path);
        }
    }
    selected
}

fn directory_purpose_representative_reads_after_find_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    let route = route_result?;
    if !loop_state.has_tool_or_skill_output
        || directory_purpose_extension_locator(route).is_none()
        || loop_state
            .executed_step_results
            .iter()
            .any(executed_step_is_successful_text_read)
    {
        return None;
    }
    let root = route_directory_locator_path(route, auto_locator_path)?;
    let candidates = loop_state
        .executed_step_results
        .iter()
        .rev()
        .flat_map(executed_find_entries_candidate_paths)
        .collect::<Vec<_>>();
    let selected = safe_representative_find_result_paths(&root, candidates);
    if selected.is_empty() {
        return None;
    }
    let mut actions = selected
        .into_iter()
        .map(|path| AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": path,
                "mode": "head",
                "n": 60,
            }),
        })
        .collect::<Vec<_>>();
    let evidence_refs = (1..=actions.len())
        .map(|idx| format!("step_{idx}"))
        .collect::<Vec<_>>();
    actions.push(AgentAction::SynthesizeAnswer { evidence_refs });
    actions.push(AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    });
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn directory_compare_locator_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let route = route_result?;
    if route.needs_clarify
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
    {
        return None;
    }
    let targets = crate::task_contract::target_locators_for_route(route);
    if targets.len() != 2 {
        return None;
    }
    let left =
        resolve_directory_locator_for_dir_compare(&state.skill_rt.workspace_root, &targets[0])?;
    let right =
        resolve_directory_locator_for_dir_compare(&state.skill_rt.workspace_root, &targets[1])?;
    if left.eq_ignore_ascii_case(&right) {
        return None;
    }
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "dir_compare",
            "left_path": left,
            "right_path": right,
            "recursive": true,
            "include_hidden": false,
            "max_diffs": 20,
        }),
    }];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn quantity_compare_pair_locator_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let route = route_result?;
    if route.needs_clarify
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::QuantityComparison
    {
        return None;
    }
    let targets = crate::task_contract::target_locators_for_route(route);
    if targets.len() != 2 {
        return None;
    }
    let left = resolve_existing_metadata_locator_path(&state.skill_rt.workspace_root, &targets[0])?;
    let right =
        resolve_existing_metadata_locator_path(&state.skill_rt.workspace_root, &targets[1])?;
    if left.eq_ignore_ascii_case(&right) {
        return None;
    }
    let action = AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "compare_paths",
            "left_path": left,
            "right_path": right,
        }),
    };
    let (skill, args) = match &action {
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        _ => return None,
    };
    if !crate::contract_matrix::action_policy_for_output_contract(
        Some(&route.output_contract),
        skill,
        args,
    )
    .is_some_and(|policy| policy.is_allowed())
    {
        return None;
    }
    let actions = vec![action];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn resolve_directory_locator_for_dir_compare(workspace_root: &Path, raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let path = Path::new(raw);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };
    if !path.is_dir() {
        return None;
    }
    Some(path.canonicalize().unwrap_or(path).display().to_string())
}

fn scalar_path_auto_locator_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let actions = scalar_path_auto_locator_observation_plan(route_result, auto_locator_path)?;
    let actions = canonicalize_legacy_file_config_capabilities(actions);
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn scalar_path_directory_locator_search_observation_plan(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    current_user_text: &str,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if route.needs_clarify
        || route.output_contract.delivery_required
        || route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::ScalarPathOnly
    {
        return None;
    }
    let root = route_directory_locator_path(route, auto_locator_path)?;
    if !Path::new(&root).is_dir() {
        return None;
    }
    let target =
        single_name_target_for_directory_locator(route, current_user_text).or_else(|| {
            single_existing_name_target_for_directory_locator(&root, route, current_user_text)
        })?;
    Some(vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "find_entries",
            "root": root,
            "pattern": target,
            "target_kind": "any",
            "max_results": 50,
        }),
    }])
}

fn scalar_path_directory_locator_search_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    current_user_text: &str,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let actions = scalar_path_directory_locator_search_observation_plan(
        route_result,
        auto_locator_path,
        current_user_text,
    )?;
    let actions = canonicalize_legacy_file_config_capabilities(actions);
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn explicit_command_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    original_user_text: &str,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || route.needs_clarify
        || !route_allows_explicit_command_preservation(route_result)
        || !run_cmd_available_for_plan(state)
    {
        return None;
    }
    let request_text = original_user_text.trim();
    let command = explicit_command_single_step_segment(&state.policy.command_intent, request_text)?;
    let mut args = serde_json::json!({
        "command": command,
        "request_text": request_text,
        "cwd": state.skill_rt.workspace_root.display().to_string(),
    });
    args[super::CLAWD_LITERAL_COMMAND_ARG] = Value::Bool(true);
    if literal_command_failure_can_replan(route_result) {
        args[super::CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG] = Value::Bool(true);
    }
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: args.clone(),
    }];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({
        "steps": [{
            "type": "call_skill",
            "skill": "run_cmd",
            "args": args,
        }]
    }))
    .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn contract_hint_preferred_action_ref(
    original_user_text: &str,
) -> Option<crate::contract_matrix::ActionRef> {
    crate::intent_router::contract_test_hint_value(original_user_text, "preferred_action_ref")
        .and_then(|value| crate::contract_matrix::ActionRef::parse(&value))
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn route_locator_targets(route: &RouteResult) -> Vec<String> {
    crate::task_contract::target_locators_for_route(route)
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .collect()
}

fn first_route_locator_target(
    route: &RouteResult,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    route_locator_targets(route).into_iter().next().or_else(|| {
        auto_locator_path
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(str::to_string)
    })
}

fn two_route_locator_targets(route: &RouteResult) -> Option<(String, String)> {
    let targets = route_locator_targets(route);
    (targets.len() >= 2).then(|| (targets[0].clone(), targets[1].clone()))
}

fn recent_child_paths_for_directory(path: &str, limit: usize) -> Option<Vec<String>> {
    let mut entries = fs::read_dir(path)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then(|| {
                let modified = metadata.modified().ok();
                (entry.path(), modified)
            })
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return None;
    }
    entries.sort_by(|(left_path, left_modified), (right_path, right_modified)| {
        right_modified
            .cmp(left_modified)
            .then_with(|| left_path.cmp(right_path))
    });
    let out = entries
        .into_iter()
        .take(limit)
        .map(|(path, _)| path.display().to_string())
        .collect::<Vec<_>>();
    (!out.is_empty()).then_some(out)
}

fn preferred_read_text_range_path_for_contract_hint(
    path: &str,
    workspace_root: &Path,
) -> Option<String> {
    let raw_target = Path::new(path);
    let target_storage;
    let target = if raw_target.is_absolute() || raw_target.exists() {
        raw_target
    } else {
        target_storage = workspace_root.join(raw_target);
        target_storage.as_path()
    };
    if target.is_file() {
        return Some(target.display().to_string());
    }
    if !target.is_dir() {
        return Some(path.to_string());
    }

    for name in [
        "README.md",
        "README.zh-CN.md",
        "README_cn.md",
        "package.json",
        "Cargo.toml",
    ] {
        let candidate = target.join(name);
        if candidate.is_file() {
            return Some(candidate.display().to_string());
        }
    }

    let mut candidates = fs::read_dir(target)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            matches!(
                ext.as_str(),
                "md" | "txt" | "toml" | "json" | "yaml" | "yml"
            )
            .then_some(path)
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates
        .into_iter()
        .next()
        .map(|path| path.display().to_string())
}

fn scalar_path_find_entries_args(path: &str) -> Value {
    let path_obj = Path::new(path);
    let root = path_obj
        .parent()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(".");
    let pattern = path_obj
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(path);
    serde_json::json!({
        "action": "find_entries",
        "root": root,
        "pattern": pattern,
        "target_kind": "any",
        "max_results": 50,
    })
}

fn route_prefers_text_excerpt_action_for_contract_hint(route: &RouteResult) -> bool {
    !route.needs_clarify
        && !route.output_contract.delivery_required
        && route.output_contract.requires_content_evidence
        && route.output_contract.locator_kind == crate::OutputLocatorKind::Path
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None | crate::OutputSemanticKind::WorkspaceProjectSummary
        )
}

fn contract_hint_selector_value(original_user_text: &str, key: &str) -> Option<String> {
    crate::intent_router::contract_test_hint_value(original_user_text, key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn contract_hint_selector_bool(original_user_text: &str, key: &str) -> Option<bool> {
    contract_hint_selector_value(original_user_text, key).and_then(|value| {
        match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        }
    })
}

fn contract_hint_selector_query(original_user_text: &str) -> Option<String> {
    contract_hint_selector_value(original_user_text, "selector_query")
        .map(|value| value.replace(['\r', '\n'], " "))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value.len() <= 160)
}

fn contract_hint_selector_case_insensitive(original_user_text: &str) -> Option<bool> {
    ["selector_case_insensitive", "selector_ignore_case"]
        .iter()
        .find_map(|key| contract_hint_selector_bool(original_user_text, key))
}

fn contract_hint_selector_extension(original_user_text: &str) -> Option<String> {
    ["selector_extension", "file_extension"]
        .iter()
        .find_map(|key| contract_hint_selector_value(original_user_text, key))
        .map(|value| value.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|value| {
            (1..=16).contains(&value.len()) && value.chars().all(|ch| ch.is_ascii_alphanumeric())
        })
}

fn contract_hint_selector_limit(original_user_text: &str) -> Option<u64> {
    contract_hint_selector_value(original_user_text, "selector_limit")
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value.clamp(1, 1000))
}

fn first_ascii_integer_limit(text: &str) -> Option<u64> {
    let mut token = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            token.push(ch);
            continue;
        }
        if let Some(limit) = parse_limit_token(&token) {
            return Some(limit);
        }
        token.clear();
    }
    parse_limit_token(&token)
}

fn parse_limit_token(token: &str) -> Option<u64> {
    if token.is_empty() {
        return None;
    }
    token
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .map(|value| value.clamp(1, 1000))
}

fn contract_hint_selector_sort_by(original_user_text: &str) -> Option<String> {
    contract_hint_selector_value(original_user_text, "selector_sort_by")
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| {
            matches!(
                value.as_str(),
                "name" | "mtime_desc" | "mtime_asc" | "size_desc" | "size_asc"
            )
        })
}

fn contract_hint_selector_target_kind(original_user_text: &str) -> Option<String> {
    contract_hint_selector_value(original_user_text, "selector_target_kind")
        .map(|value| value.to_ascii_lowercase())
        .and_then(|value| match value.as_str() {
            "file" | "files" => Some("file".to_string()),
            "dir" | "dirs" | "directory" | "directories" => Some("dir".to_string()),
            "any" => Some("any".to_string()),
            _ => None,
        })
}

fn preferred_run_cmd_for_contract_hint(
    state: &AppState,
    route: &RouteResult,
    auto_locator_path: Option<&str>,
) -> Option<AgentAction> {
    let cwd = state.skill_rt.workspace_root.display().to_string();
    let command = match route.output_contract.semantic_kind {
        crate::OutputSemanticKind::PackageManagerDetection => {
            r#"for m in apt-get apt dnf yum brew pacman zypper apk; do if command -v "$m" >/dev/null 2>&1; then printf 'manager=%s\nbasis=command_path:%s\n' "$m" "$m"; exit 0; fi; done; printf 'manager=unknown\nbasis=path_scan_none\n'"#.to_string()
        }
        crate::OutputSemanticKind::QuantityComparison => {
            let (left, right) = two_route_locator_targets(route)?;
            format!(
                "stat -c 'size_bytes=%s path=%n' {} {} 2>/dev/null || wc -c {} {}",
                shell_single_quote(&left),
                shell_single_quote(&right),
                shell_single_quote(&left),
                shell_single_quote(&right)
            )
        }
        crate::OutputSemanticKind::ScalarCount => {
            let path = first_route_locator_target(route, auto_locator_path)?;
            format!(
                "find {} -mindepth 1 -maxdepth 1 2>/dev/null | wc -l | tr -d ' '",
                shell_single_quote(&path)
            )
        }
        crate::OutputSemanticKind::RecentScalarEqualityCheck => {
            "git branch --show-current | awk '{print \"field_value=\" $0}'".to_string()
        }
        crate::OutputSemanticKind::ServiceStatus => {
            let filter = process_status_filter_token(&route.resolved_intent)
                .unwrap_or_else(|| "clawd".to_string());
            format!(
                "ps -eo pid,comm,args | grep -F {} | grep -v grep || true",
                shell_single_quote(&filter)
            )
        }
        crate::OutputSemanticKind::SqliteTableListing
        | crate::OutputSemanticKind::SqliteTableNamesOnly
        | crate::OutputSemanticKind::SqliteDatabaseKindJudgment => {
            let db_path = first_route_locator_target(route, auto_locator_path)?;
            format!(
                "sqlite3 {} '.tables' | tr -s ' ' '\\n' | sed '/^$/d'",
                shell_single_quote(&db_path)
            )
        }
        crate::OutputSemanticKind::SqliteSchemaVersion => {
            let db_path = first_route_locator_target(route, auto_locator_path)?;
            format!(
                "sqlite3 {} 'PRAGMA schema_version;' | awk '{{print \"schema_version=\" $0}}'",
                shell_single_quote(&db_path)
            )
        }
        crate::OutputSemanticKind::DockerPs => "docker ps 2>&1 || true".to_string(),
        crate::OutputSemanticKind::DockerImages => "docker images 2>&1 || true".to_string(),
        crate::OutputSemanticKind::DockerLogs => "docker ps 2>&1 || true".to_string(),
        crate::OutputSemanticKind::DockerContainerLifecycle => "docker version 2>&1 || true".to_string(),
        _ => return None,
    };
    let mut args = serde_json::json!({
        "command": command,
        "cwd": cwd,
    });
    args[super::CLAWD_LITERAL_COMMAND_ARG] = Value::Bool(true);
    Some(AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args,
    })
}

fn preferred_fs_basic_for_contract_hint(
    state: &AppState,
    route: &RouteResult,
    action_name: &str,
    auto_locator_path: Option<&str>,
    original_user_text: &str,
) -> Option<AgentAction> {
    let action_name = if action_name != "read_text_range"
        && (route.output_contract.semantic_kind
            == crate::OutputSemanticKind::WorkspaceProjectSummary
            || route_prefers_text_excerpt_action_for_contract_hint(route))
    {
        "read_text_range"
    } else {
        action_name
    };
    let mut args = match action_name {
        "stat_paths" => {
            if route.output_contract.semantic_kind
                == crate::OutputSemanticKind::RecentArtifactsJudgment
            {
                let root = first_route_locator_target(route, auto_locator_path)?;
                let paths =
                    recent_child_paths_for_directory(&root, 2).unwrap_or_else(|| vec![root]);
                serde_json::json!({"action": "stat_paths", "paths": paths})
            } else if let Some((left, right)) = two_route_locator_targets(route) {
                serde_json::json!({"action": "stat_paths", "paths": [left, right]})
            } else {
                let path = first_route_locator_target(route, auto_locator_path)?;
                serde_json::json!({"action": "stat_paths", "paths": [path]})
            }
        }
        "find_entries" => {
            let path = first_route_locator_target(route, auto_locator_path)?;
            if route.output_contract.semantic_kind == crate::OutputSemanticKind::ScalarPathOnly {
                scalar_path_find_entries_args(&path)
            } else {
                let default_target_kind = if route.output_contract.semantic_kind
                    == crate::OutputSemanticKind::DirectoryEntryGroups
                {
                    "any"
                } else {
                    "file"
                };
                let target_kind = contract_hint_selector_target_kind(original_user_text)
                    .unwrap_or_else(|| default_target_kind.to_string());
                serde_json::json!({
                    "action": "find_entries",
                    "root": path,
                    "target_kind": target_kind,
                    "max_results": 50,
                    "include_hidden": route.output_contract.semantic_kind == crate::OutputSemanticKind::HiddenEntriesCheck,
                })
            }
        }
        "count_entries" => {
            let path = first_route_locator_target(route, auto_locator_path)?;
            serde_json::json!({"action": "count_entries", "path": path})
        }
        "compare_paths" => {
            let (left, right) = two_route_locator_targets(route)?;
            serde_json::json!({"action": "compare_paths", "left_path": left, "right_path": right})
        }
        "list_dir" => {
            let path = first_route_locator_target(route, auto_locator_path)?;
            let mut args = serde_json::json!({
                "action": "list_dir",
                "path": path,
                "names_only": false,
                "max_entries": 1000,
                "sort_by": if route.output_contract.semantic_kind == crate::OutputSemanticKind::RecentArtifactsJudgment {
                    "mtime_desc"
                } else {
                    "name"
                },
            });
            if let Some(kind) = contract_hint_selector_target_kind(original_user_text) {
                if kind == "file" {
                    args["files_only"] = Value::Bool(true);
                } else if kind == "dir" {
                    args["dirs_only"] = Value::Bool(true);
                }
            }
            if route.output_contract.semantic_kind == crate::OutputSemanticKind::HiddenEntriesCheck
            {
                args["include_hidden"] = Value::Bool(true);
            }
            args
        }
        "read_text_range" => {
            let target = first_route_locator_target(route, auto_locator_path)?;
            let path = preferred_read_text_range_path_for_contract_hint(
                &target,
                &state.skill_rt.workspace_root,
            )
            .unwrap_or(target);
            serde_json::json!({
                "action": "read_text_range",
                "path": path,
                "mode": "head",
                "n": 80,
            })
        }
        "grep_text" => {
            let path = first_route_locator_target(route, auto_locator_path)?;
            let query = contract_hint_selector_query(original_user_text)?;
            let mut args = serde_json::json!({
                "action": "grep_text",
                "root": path,
                "query": query,
                "max_results": 50,
            });
            if contract_hint_selector_case_insensitive(original_user_text).unwrap_or(
                route.output_contract.semantic_kind
                    == crate::OutputSemanticKind::ContentPresenceCheck,
            ) {
                args["case_insensitive"] = Value::Bool(true);
            }
            args
        }
        _ => return None,
    };
    if let Some(obj) = args.as_object_mut() {
        if let Some(limit) = contract_hint_selector_limit(original_user_text) {
            let key = if action_name == "list_dir" {
                "max_entries"
            } else {
                "max_results"
            };
            obj.insert(key.to_string(), Value::Number(limit.into()));
        }
        if let Some(sort_by) = contract_hint_selector_sort_by(original_user_text) {
            obj.insert("sort_by".to_string(), Value::String(sort_by));
        }
        if let Some(extension) = contract_hint_selector_extension(original_user_text) {
            let key = if action_name == "list_dir" {
                "ext_filter"
            } else {
                "extension"
            };
            obj.insert(key.to_string(), Value::String(extension));
        }
    }
    Some(AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args,
    })
}

fn config_path_for_contract_hint(route: &RouteResult, auto_locator_path: Option<&str>) -> String {
    first_route_locator_target(route, auto_locator_path)
        .unwrap_or_else(|| "configs/config.toml".to_string())
}

fn preferred_config_basic_for_contract_hint(
    route: &RouteResult,
    action_name: Option<&str>,
    auto_locator_path: Option<&str>,
) -> Option<AgentAction> {
    let action = action_name.unwrap_or(match route.output_contract.semantic_kind {
        crate::OutputSemanticKind::ConfigRiskAssessment => "guard_rustclaw_config",
        crate::OutputSemanticKind::ConfigValidation => "validate",
        crate::OutputSemanticKind::StructuredKeys => "list_keys",
        _ => "validate",
    });
    let path = config_path_for_contract_hint(route, auto_locator_path);
    let args = match action {
        "guard_rustclaw_config" => serde_json::json!({
            "action": "guard_rustclaw_config",
            "path": path,
        }),
        "validate" => serde_json::json!({
            "action": "validate",
            "path": path,
        }),
        "list_keys" => serde_json::json!({
            "action": "list_keys",
            "path": path,
            "max_keys": 200,
        }),
        "read_fields" | "read_field" => serde_json::json!({
            "action": action,
            "path": path,
        }),
        _ => return None,
    };
    Some(AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args,
    })
}

fn preferred_config_edit_for_contract_hint(
    route: &RouteResult,
    action_name: Option<&str>,
    auto_locator_path: Option<&str>,
) -> Option<AgentAction> {
    let action = action_name.unwrap_or(match route.output_contract.semantic_kind {
        crate::OutputSemanticKind::ConfigRiskAssessment => "guard_config",
        crate::OutputSemanticKind::ConfigValidation => "validate_config",
        _ => "guard_config",
    });
    let path = config_path_for_contract_hint(route, auto_locator_path);
    let args = match action {
        "guard_config" => serde_json::json!({
            "action": "guard_config",
            "path": path,
        }),
        "validate_config" => serde_json::json!({
            "action": "validate_config",
            "path": path,
        }),
        _ => return None,
    };
    Some(AgentAction::CallTool {
        tool: "config_edit".to_string(),
        args,
    })
}

fn preferred_archive_basic_for_contract_hint(
    state: &AppState,
    route: &RouteResult,
    action_name: Option<&str>,
    auto_locator_path: Option<&str>,
    original_user_text: &str,
) -> Option<AgentAction> {
    if !archive_basic_enabled_for_planning(state) {
        return None;
    }
    let action = action_name.unwrap_or(match route.output_contract.semantic_kind {
        crate::OutputSemanticKind::ArchiveRead => "read",
        crate::OutputSemanticKind::ArchivePack => "pack",
        crate::OutputSemanticKind::ArchiveUnpack => "unpack",
        _ => "list",
    });
    let args = match action {
        "list" => {
            let archive = archive_list_auto_locator_target_path(Some(route), auto_locator_path)
                .or_else(|| {
                    let hint = route.output_contract.locator_hint.trim();
                    is_supported_archive_path(hint).then(|| hint.to_string())
                })?;
            serde_json::json!({
                "action": "list",
                "archive": archive,
            })
        }
        "read" => {
            let (archive, member) =
                archive_read_locator_parts(Some(route), auto_locator_path, original_user_text)?;
            serde_json::json!({
                "action": "read",
                "archive": archive,
                "member": member,
            })
        }
        "pack" => {
            let (source, archive) = archive_pack_pair_for_route(route)?;
            serde_json::json!({
                "action": "pack",
                "source": source,
                "archive": archive,
            })
        }
        "unpack" => {
            let (archive, dest) = archive_unpack_pair_for_route(route)?;
            serde_json::json!({
                "action": "unpack",
                "archive": archive,
                "dest": dest,
            })
        }
        _ => return None,
    };
    Some(AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args,
    })
}

fn preferred_structured_action_for_contract_hint(
    state: &AppState,
    route: &RouteResult,
    preferred: &crate::contract_matrix::ActionRef,
    auto_locator_path: Option<&str>,
    original_user_text: &str,
) -> Option<AgentAction> {
    match preferred.skill.as_str() {
        "run_cmd" if run_cmd_available_for_plan(state) => {
            preferred_run_cmd_for_contract_hint(state, route, auto_locator_path)
        }
        "package_manager" if package_manager_available_for_plan(state) => {
            Some(AgentAction::CallSkill {
                skill: "package_manager".to_string(),
                args: serde_json::json!({"action": preferred.action.as_deref().unwrap_or("detect")}),
            })
        }
        "fs_basic" => preferred_fs_basic_for_contract_hint(
            state,
            route,
            preferred.action.as_deref().unwrap_or("stat_paths"),
            auto_locator_path,
            original_user_text,
        ),
        "doc_parse" if doc_parse_is_enabled(state) => {
            let path = first_route_locator_target(route, auto_locator_path)?;
            if !doc_parse_supported_path(&path) {
                return None;
            }
            Some(AgentAction::CallSkill {
                skill: "doc_parse".to_string(),
                args: serde_json::json!({
                    "action": preferred.action.as_deref().unwrap_or("parse_doc"),
                    "path": path,
                    "max_chars": 12000,
                    "include_metadata": true,
                }),
            })
        }
        "config_basic" => preferred_config_basic_for_contract_hint(
            route,
            preferred.action.as_deref(),
            auto_locator_path,
        ),
        "config_edit" => preferred_config_edit_for_contract_hint(
            route,
            preferred.action.as_deref(),
            auto_locator_path,
        ),
        "config_guard" => {
            preferred_config_edit_for_contract_hint(route, Some("guard_config"), auto_locator_path)
        }
        "archive_basic" => preferred_archive_basic_for_contract_hint(
            state,
            route,
            preferred.action.as_deref(),
            auto_locator_path,
            original_user_text,
        ),
        "health_check" if health_check_available_for_plan(state) => Some(AgentAction::CallSkill {
            skill: "health_check".to_string(),
            args: serde_json::json!({}),
        }),
        "process_basic" if process_basic_available_for_plan(state) => {
            Some(AgentAction::CallSkill {
                skill: "process_basic".to_string(),
                args: serde_json::json!({
                    "action": preferred.action.as_deref().unwrap_or("ps"),
                    "limit": 200,
                    "filter": process_status_filter_token(&route.resolved_intent)
                        .unwrap_or_else(|| "clawd".to_string()),
                }),
            })
        }
        "service_control" => Some(AgentAction::CallSkill {
            skill: "service_control".to_string(),
            args: serde_json::json!({
                "action": preferred.action.as_deref().unwrap_or("status"),
                "target": process_status_filter_token(&route.resolved_intent)
                    .unwrap_or_else(|| "clawd".to_string()),
                "manager_type": "rustclaw",
            }),
        }),
        "git_basic" if git_basic_available_for_plan(state) => {
            if route.output_contract.semantic_kind
                == crate::OutputSemanticKind::WorkspaceProjectSummary
            {
                preferred_fs_basic_for_contract_hint(
                    state,
                    route,
                    "read_text_range",
                    auto_locator_path,
                    original_user_text,
                )
            } else {
                Some(AgentAction::CallSkill {
                    skill: "git_basic".to_string(),
                    args: serde_json::json!({
                        "action": match route.output_contract.semantic_kind {
                            crate::OutputSemanticKind::GitCommitSubject => "log",
                            crate::OutputSemanticKind::GitRepositoryState => "status",
                            crate::OutputSemanticKind::RecentScalarEqualityCheck => "current_branch",
                            _ => preferred.action.as_deref().unwrap_or("status"),
                        },
                    }),
                })
            }
        }
        "db_basic" => {
            if !matches!(
                route.output_contract.semantic_kind,
                crate::OutputSemanticKind::SqliteSchemaVersion
                    | crate::OutputSemanticKind::SqliteTableListing
                    | crate::OutputSemanticKind::SqliteTableNamesOnly
                    | crate::OutputSemanticKind::SqliteDatabaseKindJudgment
            ) {
                return None;
            }
            let db_path = first_route_locator_target(route, auto_locator_path)?;
            Some(AgentAction::CallSkill {
                skill: "db_basic".to_string(),
                args: serde_json::json!({
                    "action": match route.output_contract.semantic_kind {
                        crate::OutputSemanticKind::SqliteSchemaVersion => "schema_version",
                        crate::OutputSemanticKind::SqliteTableListing
                        | crate::OutputSemanticKind::SqliteTableNamesOnly
                        | crate::OutputSemanticKind::SqliteDatabaseKindJudgment => "list_tables",
                        _ => preferred.action.as_deref().unwrap_or("list_tables"),
                    },
                    "db_path": db_path,
                }),
            })
        }
        "docker_basic" if docker_basic_available_for_plan(state) => Some(AgentAction::CallSkill {
            skill: "docker_basic".to_string(),
            args: serde_json::json!({
                "action": preferred.action.as_deref().unwrap_or(match route.output_contract.semantic_kind {
                    crate::OutputSemanticKind::DockerImages => "images",
                    crate::OutputSemanticKind::DockerLogs => "ps",
                    crate::OutputSemanticKind::DockerContainerLifecycle => "version",
                    _ => "ps",
                }),
            }),
        }),
        _ => None,
    }
}

fn route_has_contract_hint_context(route: &RouteResult, original_user_text: &str) -> bool {
    crate::intent_router::contract_test_hint_semantic_kind(original_user_text).is_some()
        || crate::intent_router::contract_test_hint_value(
            original_user_text,
            "preferred_action_ref",
        )
        .is_some()
        || route.route_reason.contains("contract_hint_fast_path")
}

fn contract_hint_existence_summary_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route: &RouteResult,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ExistenceWithPathSummary {
        return None;
    }
    let target = first_route_locator_target(route, auto_locator_path)?;
    let read_path =
        preferred_read_text_range_path_for_contract_hint(&target, &state.skill_rt.workspace_root)
            .unwrap_or_else(|| target.clone());
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "stat_paths",
                "paths": [target],
                "include_missing": true,
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": read_path,
                "mode": "head",
                "n": 80,
            }),
        },
    ];
    if !actions.iter().all(|action| {
        let (skill, args) = match action {
            AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
            AgentAction::CallTool { tool, args } => (tool.as_str(), args),
            _ => return false,
        };
        crate::contract_matrix::action_policy_for_output_contract(
            Some(&route.output_contract),
            skill,
            args,
        )
        .is_some_and(|policy| policy.is_allowed())
    }) {
        return None;
    }
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn contract_hint_preferred_action_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    original_user_text: &str,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || route.needs_clarify
        || !route.is_execute_gate()
    {
        return None;
    }
    if !route_has_contract_hint_context(route, original_user_text) {
        return None;
    }
    if let Some(plan_result) = contract_hint_existence_summary_deterministic_plan_result(
        state,
        goal,
        route,
        auto_locator_path,
    ) {
        return Some(plan_result);
    }
    let preferred_actions = if let Some(preferred) =
        contract_hint_preferred_action_ref(original_user_text)
    {
        vec![preferred]
    } else {
        crate::contract_matrix::preferred_action_refs_for_output_contract(&route.output_contract)
    };
    for preferred in preferred_actions {
        let Some(action) = preferred_structured_action_for_contract_hint(
            state,
            route,
            &preferred,
            auto_locator_path,
            original_user_text,
        ) else {
            continue;
        };
        let (skill, args) = match &action {
            AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
            AgentAction::CallTool { tool, args } => (tool.as_str(), args),
            _ => continue,
        };
        if !crate::contract_matrix::action_policy_for_output_contract(
            Some(&route.output_contract),
            skill,
            args,
        )
        .is_some_and(|policy| policy.is_allowed())
        {
            continue;
        }
        let actions = vec![action];
        let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
            .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
        return Some(build_plan_result(
            goal,
            &raw_plan_text,
            PlanKind::Single,
            &actions,
        ));
    }
    None
}

fn package_manager_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("package_manager")
}

fn git_basic_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("git_basic")
}

fn normalizer_answer_candidate_from_resolved_prompt(resolved_prompt: &str) -> Option<String> {
    let (_intent, candidate) = resolved_prompt.rsplit_once("\nanswer_candidate:")?;
    let candidate = candidate.trim();
    (!candidate.is_empty()).then(|| candidate.to_string())
}

fn package_manager_detect_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || route.needs_clarify
        || !route.is_execute_gate()
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::PackageManagerDetection
        || !package_manager_available_for_plan(state)
    {
        return None;
    }
    let mut args = serde_json::json!({"action": "detect"});
    if let Some(path) = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        args["path"] = Value::String(path.to_string());
    }
    let actions = vec![
        AgentAction::CallSkill {
            skill: "package_manager".to_string(),
            args,
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn package_manager_dry_run_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    original_user_text: &str,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || route.needs_clarify
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || !matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
        || route.output_contract.delivery_required
        || !package_manager_available_for_plan(state)
    {
        return None;
    }
    let packages = normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent)
        .and_then(|candidate| {
            crate::package_commands::package_install_packages_from_commandish_text(&candidate)
        })
        .or_else(|| {
            crate::package_commands::package_install_packages_from_preview_text(original_user_text)
        })?;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "package_manager".to_string(),
            args: serde_json::json!({
                "action": "smart_install",
                "packages": packages,
                "dry_run": true,
                "use_sudo": true
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn existence_with_path_locator_observation_plan(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    current_user_text: &str,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if route.needs_clarify
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::ExistenceWithPath
    {
        return None;
    }

    let hint = route.output_contract.locator_hint.trim();
    let current_surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_text);
    let current_has_structural_locator =
        current_surface.has_explicit_path_or_url() || current_surface.has_filename_candidates();
    let explicit_targets = explicit_document_file_targets(current_user_text)
        .into_iter()
        .take(8)
        .collect::<Vec<_>>();
    if explicit_targets.len() == 1
        && route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
    {
        return Some(vec![fs_basic_stat_paths_action_for_explicit_targets(
            &explicit_targets,
        )]);
    }
    if explicit_targets.len() >= 2 {
        return Some(vec![fs_basic_stat_paths_action_for_explicit_targets(
            &explicit_targets,
        )]);
    }

    match route.output_contract.locator_kind {
        crate::OutputLocatorKind::Filename | crate::OutputLocatorKind::CurrentWorkspace
            if !hint.is_empty() =>
        {
            Some(vec![AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "find_entries",
                    "root": ".",
                    "pattern": hint,
                    "target_kind": "any",
                    "max_results": 50,
                }),
            }])
        }
        crate::OutputLocatorKind::Path => {
            let path = auto_locator_path
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .or_else(|| (!hint.is_empty()).then_some(hint))?;
            if is_supported_archive_path(path)
                && archive_entry_target_for_route_or_text(route, current_user_text, path).is_some()
            {
                return None;
            }
            if Path::new(path).is_dir() {
                if let Some(file_name) =
                    single_filename_target_for_directory_locator(route, current_user_text)
                {
                    return Some(vec![AgentAction::CallTool {
                        tool: "fs_basic".to_string(),
                        args: serde_json::json!({
                            "action": "find_entries",
                            "root": path,
                            "pattern": file_name,
                            "target_kind": "file",
                            "max_results": 50,
                        }),
                    }]);
                }
                if !current_has_structural_locator {
                    return None;
                }
            }
            Some(vec![AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "stat_paths",
                    "paths": [path],
                    "include_missing": true,
                }),
            }])
        }
        _ => None,
    }
}

fn single_filename_target_for_directory_locator(
    route: &RouteResult,
    current_user_text: &str,
) -> Option<String> {
    let filenames = crate::delivery_utils::extract_filename_candidates(current_user_text);
    if filenames.len() == 1 {
        return Some(filenames[0].clone());
    }
    if !filenames.is_empty() {
        return None;
    }
    let current_surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_text);
    if !current_surface.has_explicit_path_or_url() && !current_surface.has_filename_candidates() {
        return None;
    }
    let filenames = crate::delivery_utils::extract_filename_candidates(&route.resolved_intent);
    (filenames.len() == 1).then(|| filenames[0].clone())
}

fn search_name_target_token_is_safe(candidate: &str) -> bool {
    let trimmed = candidate.trim().trim_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\''
                | '`'
                | ','
                | '，'
                | '。'
                | ':'
                | '：'
                | ';'
                | '；'
                | '('
                | ')'
                | '（'
                | '）'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '《'
                | '》'
        )
    });
    if trimmed.is_empty()
        || trimmed.len() > 128
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || crate::worker::has_explicit_path_or_url_locator_hint(trimmed)
        || crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(trimmed)
    {
        return false;
    }
    let mut has_ascii_alnum = false;
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() {
            has_ascii_alnum = true;
            continue;
        }
        if !matches!(ch, '_' | '-' | '.') {
            return false;
        }
    }
    has_ascii_alnum
}

fn push_unique_search_name_candidate(values: &mut Vec<String>, candidate: &str) {
    let trimmed = candidate.trim().trim_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\''
                | '`'
                | ','
                | '，'
                | '。'
                | ':'
                | '：'
                | ';'
                | '；'
                | '('
                | ')'
                | '（'
                | '）'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '《'
                | '》'
        )
    });
    if search_name_target_token_is_safe(trimmed)
        && !values
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(trimmed))
    {
        values.push(trimmed.to_string());
    }
}

fn single_quoted_search_name_target(text: &str) -> Option<String> {
    static QUOTED_RE: OnceLock<Regex> = OnceLock::new();
    let re = QUOTED_RE.get_or_init(|| {
        Regex::new(r#""([^"\n]+)"|'([^'\n]+)'|`([^`\n]+)`"#).expect("quoted search name regex")
    });
    let mut candidates = Vec::new();
    for caps in re.captures_iter(text) {
        let candidate = caps
            .get(1)
            .or_else(|| caps.get(2))
            .or_else(|| caps.get(3))
            .map(|m| m.as_str())
            .unwrap_or_default();
        push_unique_search_name_candidate(&mut candidates, candidate);
    }
    (candidates.len() == 1).then(|| candidates.remove(0))
}

fn search_name_targets_outside_locators(text: &str) -> Vec<String> {
    let mut remaining = text.to_string();
    for locator in
        crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(text)
    {
        remaining = remaining.replace(&locator.locator_hint, " ");
    }
    for filename in crate::delivery_utils::extract_filename_candidates(text) {
        remaining = remaining.replace(&filename, " ");
    }
    let mut candidates = Vec::new();
    for token in
        remaining.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')))
    {
        push_unique_search_name_candidate(&mut candidates, token);
    }
    candidates
}

fn single_identifier_search_name_target_outside_locators(text: &str) -> Option<String> {
    let mut candidates = search_name_targets_outside_locators(text);
    (candidates.len() == 1).then(|| candidates.remove(0))
}

fn single_existing_name_target_for_directory_locator(
    root: &str,
    route: &RouteResult,
    current_user_text: &str,
) -> Option<String> {
    let mut matching_tokens = Vec::new();
    for text in [current_user_text, route.resolved_intent.as_str()] {
        for token in search_name_targets_outside_locators(text) {
            if directory_has_unique_entry_for_search_name(root, &token)
                && !matching_tokens
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(&token))
            {
                matching_tokens.push(token);
            }
        }
    }
    (matching_tokens.len() == 1).then(|| matching_tokens.remove(0))
}

fn directory_has_unique_entry_for_search_name(root: &str, token: &str) -> bool {
    let root = Path::new(root);
    if !root.is_dir() {
        return false;
    }
    let token = token.to_ascii_lowercase();
    if token.len() < 2 {
        return false;
    }
    let mut stack = vec![root.to_path_buf()];
    let mut visits = 0usize;
    let mut matches = 0usize;
    while let Some(dir) = stack.pop() {
        visits = visits.saturating_add(1);
        if visits > 10_000 || matches > 1 {
            return false;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let name = name.to_ascii_lowercase();
            let stem = path
                .file_stem()
                .and_then(|value| value.to_str())
                .map(|value| value.to_ascii_lowercase());
            if name == token || stem.as_deref() == Some(token.as_str()) {
                matches = matches.saturating_add(1);
                if matches > 1 {
                    return false;
                }
            }
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    matches == 1
}

fn single_name_target_for_directory_locator(
    route: &RouteResult,
    current_user_text: &str,
) -> Option<String> {
    single_filename_target_for_directory_locator(route, current_user_text)
        .or_else(|| single_quoted_search_name_target(current_user_text))
        .or_else(|| single_quoted_search_name_target(&route.resolved_intent))
        .or_else(|| single_identifier_search_name_target_outside_locators(current_user_text))
}

fn archive_entry_target_for_route_or_text(
    route: &RouteResult,
    current_user_text: &str,
    archive_path: &str,
) -> Option<String> {
    let archive_path = archive_path.trim();
    if archive_path.is_empty() || !is_supported_archive_path(archive_path) {
        return None;
    }

    let mut path_candidates = Vec::new();
    let mut filename_candidates = Vec::new();
    for text in [current_user_text, route.resolved_intent.as_str()] {
        for locator in
            crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(text)
        {
            push_archive_entry_target_candidate(
                &mut path_candidates,
                &mut filename_candidates,
                &locator.locator_hint,
                archive_path,
            );
        }
        for filename in crate::delivery_utils::extract_filename_candidates(text) {
            push_archive_entry_target_candidate(
                &mut path_candidates,
                &mut filename_candidates,
                &filename,
                archive_path,
            );
        }
    }

    path_candidates
        .into_iter()
        .next()
        .or_else(|| filename_candidates.into_iter().next())
}

fn push_archive_entry_target_candidate(
    path_candidates: &mut Vec<String>,
    filename_candidates: &mut Vec<String>,
    candidate: &str,
    archive_path: &str,
) {
    let Some(candidate) = normalize_archive_entry_target_candidate(candidate, archive_path) else {
        return;
    };
    let target = if candidate.contains('/') || candidate.contains('\\') {
        path_candidates
    } else {
        filename_candidates
    };
    if !target
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&candidate))
    {
        target.push(candidate);
    }
}

fn normalize_archive_entry_target_candidate(candidate: &str, archive_path: &str) -> Option<String> {
    let trimmed = candidate.trim().trim_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\''
                | '`'
                | ','
                | '，'
                | '。'
                | ':'
                | '：'
                | ';'
                | '；'
                | '('
                | ')'
                | '（'
                | '）'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '《'
                | '》'
        )
    });
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains("://")
        || Path::new(trimmed).is_absolute()
        || is_supported_archive_path(trimmed)
        || archive_locator_candidate_matches_archive(trimmed, archive_path)
        || crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(trimmed)
    {
        return None;
    }
    if !trimmed.contains('/')
        && !trimmed.contains('\\')
        && !archive_entry_target_candidate_has_extension(trimmed)
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn archive_locator_candidate_matches_archive(candidate: &str, archive_path: &str) -> bool {
    let candidate_norm = candidate.replace('\\', "/");
    let archive_norm = archive_path.trim().replace('\\', "/");
    if candidate_norm.eq_ignore_ascii_case(&archive_norm) {
        return true;
    }
    let archive_name = archive_norm
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(archive_norm.as_str());
    candidate_norm.eq_ignore_ascii_case(archive_name)
}

fn archive_entry_target_candidate_has_extension(candidate: &str) -> bool {
    let basename = candidate
        .rsplit(|ch| ch == '/' || ch == '\\')
        .next()
        .unwrap_or(candidate);
    let Some((stem, ext)) = basename.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && (1..=16).contains(&ext.len())
        && ext.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn existence_with_path_locator_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    current_user_text: &str,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let actions = existence_with_path_locator_observation_plan(
        route_result,
        auto_locator_path,
        current_user_text,
    )?;
    let actions = canonicalize_legacy_file_config_capabilities(actions);
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn file_paths_locator_observation_plan(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if route.needs_clarify
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::FilePaths
    {
        return None;
    }

    let hint = route.output_contract.locator_hint.trim();
    if !hint.is_empty() {
        if Path::new(hint).is_dir() {
            return None;
        }
        if let Some((root, pattern)) = split_path_like_file_locator_hint(hint) {
            return Some(vec![AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "find_entries",
                    "root": root,
                    "pattern": pattern,
                    "target_kind": "file",
                    "max_results": 50,
                }),
            }]);
        }
        return Some(vec![AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "find_entries",
                "root": ".",
                "pattern": hint,
                "target_kind": "file",
                "max_results": 50,
            }),
        }]);
    }

    let path = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .filter(|path| Path::new(path).is_file())?;
    let name = Path::new(path).file_name()?.to_string_lossy().to_string();
    let root = Path::new(path)
        .parent()
        .and_then(|parent| parent.to_str())
        .filter(|parent| !parent.is_empty())
        .unwrap_or(".");
    Some(vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "find_entries",
            "root": root,
            "pattern": name,
            "target_kind": "file",
            "max_results": 50,
        }),
    }])
}

fn split_path_like_file_locator_hint(hint: &str) -> Option<(String, String)> {
    let trimmed = hint.trim();
    if trimmed.is_empty() || (!trimmed.contains('/') && !trimmed.contains('\\')) {
        return None;
    }
    let normalized = trimmed.replace('\\', "/");
    let (root, file_name) = normalized.rsplit_once('/')?;
    let file_name = file_name.trim();
    if file_name.is_empty() {
        return None;
    }
    let root = if root.trim().is_empty() {
        if normalized.starts_with('/') {
            "/"
        } else {
            "."
        }
    } else {
        root.trim()
    };
    Some((root.to_string(), file_name.to_string()))
}

fn file_paths_locator_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let actions = file_paths_locator_observation_plan(route_result, auto_locator_path)?;
    let actions = canonicalize_legacy_file_config_capabilities(actions);
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn doc_parse_supported_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.trim().to_ascii_lowercase().as_str(),
                "md" | "txt" | "html" | "htm" | "pdf" | "docx"
            )
        })
        .unwrap_or(false)
}

fn doc_parse_is_enabled(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("doc_parse")
}

fn log_analyze_is_enabled(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("log_analyze")
}

fn log_analyze_supported_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.trim().eq_ignore_ascii_case("log"))
        .unwrap_or(false)
}

fn route_allows_single_file_content_understanding(route: &RouteResult) -> bool {
    route
        .output_contract
        .semantic_kind
        .is_content_excerpt_summary()
        && !route.output_contract.delivery_required
        && matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
}

fn single_file_content_understanding_target_path(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify || !route_allows_single_file_content_understanding(route) {
        return None;
    }
    auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or_else(|| {
            let hint = route.output_contract.locator_hint.trim();
            (!hint.is_empty() && Path::new(hint).is_file()).then_some(hint)
        })
        .filter(|path| Path::new(path).is_file())
        .map(ToString::to_string)
}

fn content_excerpt_summary_auto_locator_observation_plan(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<Vec<AgentAction>> {
    let path = single_file_content_understanding_target_path(route_result, auto_locator_path)?;
    if repo_text_artifact_prefers_bounded_fs_read(&path) {
        return Some(vec![AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": path,
                "mode": "head",
                "n": 120
            }),
        }]);
    }
    if !doc_parse_supported_path(&path) {
        return None;
    }
    Some(vec![AgentAction::CallSkill {
        skill: "doc_parse".to_string(),
        args: serde_json::json!({
            "action": "parse_doc",
            "path": path,
            "max_chars": 12000,
            "include_metadata": true
        }),
    }])
}

fn repo_text_artifact_prefers_bounded_fs_read(path: &str) -> bool {
    let path = Path::new(path);
    let Some(ext) = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.trim().to_ascii_lowercase())
    else {
        return false;
    };
    if !matches!(
        ext.as_str(),
        "md" | "txt" | "toml" | "json" | "yaml" | "yml" | "rs"
    ) {
        return false;
    }
    path.components().any(|component| {
        matches!(
            component,
            std::path::Component::Normal(value)
                if matches!(
                    value.to_str(),
                    Some("prompts" | "crates" | "configs" | "docker" | "scripts" | "UI" | "docs")
                )
        )
    })
}

fn content_excerpt_summary_auto_locator_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if !matches!(
        route_result.map(|route| route.output_contract.semantic_kind),
        Some(crate::OutputSemanticKind::ContentExcerptSummary)
    ) {
        return None;
    }
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let actions =
        content_excerpt_summary_auto_locator_observation_plan(route_result, auto_locator_path)?;
    let actions = canonicalize_legacy_file_config_capabilities(actions);
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn archive_basic_enabled_for_planning(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("archive_basic")
}

fn archive_list_auto_locator_target_path(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
    {
        return None;
    }
    auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .filter(|path| is_supported_archive_path(path))
        .or_else(|| {
            let hint = route.output_contract.locator_hint.trim();
            (!hint.is_empty()).then_some(hint)
        })
        .filter(|path| is_supported_archive_path(path))
        .filter(|path| route_allows_archive_list_auto_locator(route, path))
        .map(ToString::to_string)
}

fn archive_read_locator_parts(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    current_user_text: &str,
) -> Option<(String, String)> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
    {
        return None;
    }

    let hint = route.output_contract.locator_hint.trim();
    let hint_parts =
        if route.output_contract.semantic_kind == crate::OutputSemanticKind::ArchiveRead {
            hint.split('|')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
    let auto_archive = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .filter(|path| is_supported_archive_path(path))
        .map(str::to_string);
    let hint_archive = hint_parts
        .first()
        .copied()
        .or_else(|| (!hint.is_empty()).then_some(hint))
        .map(str::to_string);
    let text_archive = archive_path_target_for_route_or_text(route, current_user_text);
    let archive = auto_archive
        .or_else(|| choose_archive_path_candidate(hint_archive, text_archive))
        .filter(|path| is_supported_archive_path(path))?;

    let member = if route.output_contract.semantic_kind == crate::OutputSemanticKind::ArchiveRead {
        let mut parts = hint_parts;
        if parts.len() >= 2 {
            if parts
                .first()
                .is_some_and(|part| is_supported_archive_path(part))
            {
                parts.remove(0);
            }
            Some(parts.join("/"))
        } else {
            archive_entry_target_for_route_or_text(route, current_user_text, &archive)
        }
    } else if matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
            | crate::OutputSemanticKind::ContentExcerptSummary
            | crate::OutputSemanticKind::ContentExcerptWithSummary
    ) {
        archive_entry_target_for_route_or_text(route, current_user_text, &archive)
    } else {
        return None;
    }?;

    if !archive_member_path_is_safe(&member) {
        return None;
    }
    Some((archive, member))
}

fn choose_archive_path_candidate(
    hint_archive: Option<String>,
    text_archive: Option<String>,
) -> Option<String> {
    match (hint_archive, text_archive) {
        (Some(hint), Some(text)) if archive_path_candidate_is_more_specific_match(&hint, &text) => {
            Some(text)
        }
        (Some(hint), _) => Some(hint),
        (None, Some(text)) => Some(text),
        (None, None) => None,
    }
}

fn archive_path_candidate_is_more_specific_match(hint: &str, text: &str) -> bool {
    let hint = hint.trim();
    let text = text.trim();
    if hint.is_empty() || text.is_empty() || hint.eq_ignore_ascii_case(text) {
        return false;
    }
    let hint_name = Path::new(hint)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(hint);
    let text_name = Path::new(text)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(text);
    hint_name.eq_ignore_ascii_case(text_name)
        && !hint.contains('/')
        && !hint.contains('\\')
        && (text.contains('/') || text.contains('\\'))
}

fn archive_path_target_for_route_or_text(
    route: &RouteResult,
    current_user_text: &str,
) -> Option<String> {
    for text in [current_user_text, route.resolved_intent.as_str()] {
        for locator in
            crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(text)
        {
            let candidate = locator.locator_hint.trim();
            if is_supported_archive_path(candidate) {
                return Some(candidate.to_string());
            }
        }
    }
    for text in [current_user_text, route.resolved_intent.as_str()] {
        for filename in crate::delivery_utils::extract_filename_candidates(text) {
            let candidate = filename.trim();
            if is_supported_archive_path(candidate) {
                return Some(candidate.to_string());
            }
        }
    }
    None
}

fn archive_member_path_is_safe(member: &str) -> bool {
    let member = member.trim();
    if member.is_empty() {
        return false;
    }
    let path = Path::new(member);
    !path.is_absolute()
        && path.components().all(|component| {
            matches!(
                component,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
}

fn has_archive_read_observation(loop_state: &LoopState, archive: &str, member: &str) -> bool {
    loop_state.executed_step_results.iter().rev().any(|step| {
        if !step.is_ok() || step.skill != "archive_basic" {
            return false;
        }
        let Some(output) = step.output.as_deref() else {
            return false;
        };
        let Ok(value) = serde_json::from_str::<Value>(output) else {
            return false;
        };
        value.get("action").and_then(Value::as_str) == Some("read")
            && value
                .get("archive")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|observed| observed == archive)
            && value
                .get("member")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|observed| observed == member)
    })
}

fn archive_read_deterministic_plan_result(
    goal: &str,
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    current_user_text: &str,
) -> Option<PlanResult> {
    if !archive_basic_enabled_for_planning(state) {
        return None;
    }
    let (archive, member) =
        archive_read_locator_parts(route_result, auto_locator_path, current_user_text)?;
    if has_archive_read_observation(loop_state, &archive, &member) {
        return None;
    }
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: serde_json::json!({
            "action": "read",
            "archive": archive,
            "member": member,
        }),
    }];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        if loop_state.round_no <= 1 {
            PlanKind::Single
        } else {
            PlanKind::Incremental
        },
        &actions,
    ))
}

fn archive_unpack_deterministic_plan_result(
    goal: &str,
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
) -> Option<PlanResult> {
    if loop_state.has_tool_or_skill_output || !archive_basic_enabled_for_planning(state) {
        return None;
    }
    let route = route_result?;
    if route.needs_clarify
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::ArchiveUnpack
    {
        return None;
    }
    let (archive, dest) = archive_unpack_pair_for_route(route)?;
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: serde_json::json!({
            "action": "unpack",
            "archive": archive,
            "dest": dest,
        }),
    }];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        if loop_state.round_no <= 1 {
            PlanKind::Single
        } else {
            PlanKind::Incremental
        },
        &actions,
    ))
}

fn route_allows_archive_list_auto_locator(route: &RouteResult, archive_path: &str) -> bool {
    match route.output_contract.semantic_kind {
        crate::OutputSemanticKind::ArchiveRead => false,
        crate::OutputSemanticKind::ExistenceWithPath => {
            archive_entry_target_for_route_or_text(route, &route.resolved_intent, archive_path)
                .is_some()
        }
        crate::OutputSemanticKind::ArchiveList | crate::OutputSemanticKind::ScalarCount => true,
        _ => route_expects_terminal_user_answer(route),
    }
}

fn archive_list_auto_locator_deterministic_plan_result(
    goal: &str,
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || !archive_basic_enabled_for_planning(state)
    {
        return None;
    }
    let archive = archive_list_auto_locator_target_path(route_result, auto_locator_path)?;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "archive_basic".to_string(),
            args: serde_json::json!({
                "action": "list",
                "archive": archive,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn transform_skill_enabled_for_planning(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("transform")
}

fn inline_json_transform_args_from_text(text: &str) -> Option<Value> {
    let explicit_transform = crate::intent::surface_signals::inline_json_transform_request(text)
        .then(|| {
            json_values_any(text)
                .into_iter()
                .rev()
                .filter_map(explicit_transform_args_from_value)
                .next()
                .map(normalize_transform_args)
        })
        .flatten();
    explicit_transform.or_else(|| derive_single_object_rename_args_from_text(text))
}

fn explicit_transform_args_from_value(value: Value) -> Option<Value> {
    let args = value
        .as_object()
        .filter(|obj| obj.get("skill").and_then(Value::as_str) == Some("transform"))
        .and_then(|obj| obj.get("args").cloned())
        .unwrap_or(value);
    let obj = args.as_object()?;
    let has_structured_input = obj.contains_key("data")
        || obj.contains_key("records")
        || obj.contains_key("csv_text")
        || obj.contains_key("csv");
    let has_structured_ops = obj
        .get("ops")
        .and_then(Value::as_array)
        .is_some_and(|ops| !ops.is_empty());
    (has_structured_input && has_structured_ops).then_some(args)
}

fn json_values_any(text: &str) -> Vec<Value> {
    json_values_any_raw(text)
        .into_iter()
        .map(|(_, value)| value)
        .collect()
}

fn json_values_any_raw(text: &str) -> Vec<(String, Value)> {
    let bytes = text.as_bytes();
    let mut values = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let opener = bytes[i];
        if opener != b'{' && opener != b'[' {
            i += 1;
            continue;
        }
        let start = i;
        let mut stack = vec![opener];
        let mut in_string = false;
        let mut escaped = false;
        let mut j = i + 1;
        let mut consumed_until = None;
        while j < bytes.len() {
            let c = bytes[j];
            if in_string {
                if escaped {
                    escaped = false;
                } else if c == b'\\' {
                    escaped = true;
                } else if c == b'"' {
                    in_string = false;
                }
                j += 1;
                continue;
            }
            match c {
                b'"' => in_string = true,
                b'{' | b'[' => stack.push(c),
                b'}' | b']' => {
                    let Some(last) = stack.pop() else {
                        break;
                    };
                    let matched = matches!((last, c), (b'{', b'}') | (b'[', b']'));
                    if !matched {
                        break;
                    }
                    if stack.is_empty() {
                        let raw = &text[start..=j];
                        if let Ok(value) = serde_json::from_str::<Value>(raw) {
                            values.push((raw.to_string(), value));
                            consumed_until = Some(j + 1);
                        }
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        i = consumed_until.unwrap_or(start + 1);
    }
    values
}

fn route_has_inline_transform_contract(route_result: Option<&RouteResult>) -> bool {
    route_result.is_some_and(|route| {
        route
            .route_reason
            .contains("inline_json_transform_structured_execute")
            || route
                .route_reason
                .contains("parsed_inline_json_transform_contract_repair")
            || route
                .route_reason
                .contains("normalizer_unavailable_inline_json_transform")
            || route
                .route_reason
                .contains("inline_structured_transform_contract_repair")
            || route
                .route_reason
                .contains("direct_answer_gate_inline_transform_execute")
            || route
                .route_reason
                .contains("inline_structured_payload_context_execute")
    })
}

fn transformable_input_value(value: &Value) -> bool {
    match value {
        Value::Array(items) => !items.is_empty() && items.iter().any(Value::is_object),
        Value::Object(obj) => {
            if obj
                .get("data")
                .or_else(|| obj.get("records"))
                .or_else(|| obj.get("input"))
                .is_some_and(|item| {
                    item.as_array().is_some_and(|items| {
                        !items.is_empty() && items.iter().any(Value::is_object)
                    }) || item.is_object()
                })
            {
                return true;
            }
            !obj.is_empty()
                && !obj.contains_key("action")
                && !obj.contains_key("skill")
                && !obj.contains_key("operation")
        }
        _ => false,
    }
}

fn last_transformable_input_value(text: &str) -> Option<Value> {
    last_transformable_input_value_with_raw(text).map(|(_, value)| value)
}

fn last_transformable_input_value_with_raw(text: &str) -> Option<(String, Value)> {
    json_values_any_raw(text)
        .into_iter()
        .rev()
        .find(|(_, value)| transformable_input_value(value))
}

fn remove_last_json_payload_from_text<'a>(text: &'a str, raw: &str) -> Option<String> {
    let start = text.rfind(raw)?;
    let end = start.saturating_add(raw.len());
    let mut remaining = String::with_capacity(text.len().saturating_sub(raw.len()) + 1);
    remaining.push_str(&text[..start]);
    remaining.push(' ');
    remaining.push_str(&text[end..]);
    Some(remaining)
}

fn schema_field_token(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch == '-' || ch.is_ascii_alphanumeric())
}

fn schema_shaped_target_token(candidate: &str, source: &str) -> bool {
    schema_field_token(candidate)
        && candidate != source
        && !candidate.chars().all(|ch| ch.is_ascii_uppercase())
        && (candidate.contains('_')
            || candidate.contains('-')
            || candidate.chars().any(|ch| ch.is_ascii_digit())
            || source.contains('_')
            || source.contains('-')
            || source.chars().any(|ch| ch.is_ascii_digit()))
}

fn schema_tokens_in_text(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch == '_' || ch == '-' || ch.is_ascii_alphanumeric() {
            current.push(ch);
            continue;
        }
        if schema_field_token(&current) {
            tokens.push(std::mem::take(&mut current));
        } else {
            current.clear();
        }
    }
    if schema_field_token(&current) {
        tokens.push(current);
    }
    tokens
}

fn derive_single_object_rename_args_from_text(text: &str) -> Option<Value> {
    let (raw, input_value) = last_transformable_input_value_with_raw(text)?;
    let input_obj = input_value.as_object()?;
    if input_obj.is_empty() {
        return None;
    }
    let instruction = remove_last_json_payload_from_text(text, &raw)?;
    let tokens = schema_tokens_in_text(&instruction);
    let input_keys = input_obj.keys().map(String::as_str).collect::<Vec<_>>();
    let mut source_positions = tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| input_keys.iter().any(|key| key == &token.as_str()))
        .collect::<Vec<_>>();
    source_positions.dedup_by(|(_, left), (_, right)| left == right);
    if source_positions.len() != 1 {
        return None;
    }
    let (source_index, source_token) = source_positions[0];
    let target_candidates = tokens
        .iter()
        .skip(source_index + 1)
        .filter(|token| !input_keys.iter().any(|key| key == &token.as_str()))
        .filter(|token| schema_shaped_target_token(token, source_token))
        .fold(Vec::<&String>::new(), |mut acc, token| {
            if !acc
                .iter()
                .any(|existing| existing.as_str() == token.as_str())
            {
                acc.push(token);
            }
            acc
        });
    if target_candidates.len() != 1 {
        return None;
    }
    Some(normalize_transform_args(serde_json::json!({
        "action": "transform_data",
        "data": input_value,
        "ops": [{
            "op": "rename",
            "from": source_token,
            "to": target_candidates[0]
        }],
        "result_shape": "single_object",
        "output_format": "json"
    })))
}

fn answer_candidate_from_route(route_result: Option<&RouteResult>) -> Option<&str> {
    let resolved = route_result?.resolved_intent.as_str();
    let (_, candidate) = resolved.rsplit_once("\nanswer_candidate:")?;
    Some(candidate.trim()).filter(|candidate| !candidate.is_empty())
}

fn parse_answer_candidate_value(candidate: &str) -> Option<Value> {
    let trimmed = candidate.trim();
    serde_json::from_str::<Value>(trimmed)
        .ok()
        .or_else(|| {
            crate::extract_first_json_value_any(trimmed)
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        })
        .or_else(|| {
            trimmed
                .parse::<i64>()
                .ok()
                .map(|value| Value::Number(value.into()))
        })
        .or_else(|| {
            trimmed
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(Value::Number)
        })
}

fn json_sort_key(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn json_rows_equal(left: &[Value], right: &[Value]) -> bool {
    left.len() == right.len() && left.iter().zip(right).all(|(a, b)| a == b)
}

fn json_multiset_equal(left: &[Value], right: &[Value]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut left_keys = left.iter().map(json_sort_key).collect::<Vec<_>>();
    let mut right_keys = right.iter().map(json_sort_key).collect::<Vec<_>>();
    left_keys.sort();
    right_keys.sort();
    left_keys == right_keys
}

fn object_keys(value: &Value) -> Vec<String> {
    value
        .as_object()
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default()
}

fn common_object_keys(rows: &[Value]) -> Vec<String> {
    let Some(first) = rows.first() else {
        return Vec::new();
    };
    object_keys(first)
        .into_iter()
        .filter(|key| {
            rows.iter()
                .all(|row| row.as_object().is_some_and(|obj| obj.contains_key(key)))
        })
        .collect()
}

fn value_for_key<'a>(row: &'a Value, key: &str) -> &'a Value {
    row.as_object()
        .and_then(|obj| obj.get(key))
        .unwrap_or(&Value::Null)
}

fn transform_cmp_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => a
            .as_f64()
            .partial_cmp(&b.as_f64())
            .unwrap_or(std::cmp::Ordering::Equal),
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        _ => json_sort_key(a).cmp(&json_sort_key(b)),
    }
}

fn derive_sort_op_from_candidate(input: &[Value], output: &[Value]) -> Option<Value> {
    if !json_multiset_equal(input, output) || json_rows_equal(input, output) {
        return None;
    }
    for key in common_object_keys(input) {
        let mut asc = input.to_vec();
        asc.sort_by(|a, b| transform_cmp_values(value_for_key(a, &key), value_for_key(b, &key)));
        if json_rows_equal(&asc, output) {
            return Some(serde_json::json!({"op": "sort", "by": key, "order": "asc"}));
        }
        let mut desc = asc;
        desc.reverse();
        if json_rows_equal(&desc, output) {
            return Some(serde_json::json!({"op": "sort", "by": key, "order": "desc"}));
        }
    }
    None
}

fn numeric_value(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn numbers_equal(left: f64, right: f64) -> bool {
    (left - right).abs() < 1e-9
}

fn derive_group_sum_op_from_candidate(input: &[Value], output: &[Value]) -> Option<Value> {
    if input.is_empty() || output.is_empty() {
        return None;
    }
    let input_keys = common_object_keys(input);
    let output_keys = common_object_keys(output);
    for group_key in input_keys.iter().filter(|key| output_keys.contains(key)) {
        for input_value_key in input_keys.iter() {
            if input_value_key == group_key {
                continue;
            }
            if !input
                .iter()
                .any(|row| numeric_value(value_for_key(row, input_value_key)).is_some())
            {
                continue;
            }
            for output_value_key in output_keys.iter().filter(|key| *key != group_key) {
                let mut sums: HashMap<String, f64> = HashMap::new();
                for row in input {
                    let group_value = value_for_key(row, group_key);
                    let key = json_sort_key(group_value);
                    let value = numeric_value(value_for_key(row, input_value_key))?;
                    *sums.entry(key).or_insert(0.0) += value;
                }
                if sums.len() != output.len() {
                    continue;
                }
                let mut matched = true;
                for row in output {
                    let group = json_sort_key(value_for_key(row, group_key));
                    let Some(expected) = sums.get(&group) else {
                        matched = false;
                        break;
                    };
                    let Some(actual) = numeric_value(value_for_key(row, output_value_key)) else {
                        matched = false;
                        break;
                    };
                    if !numbers_equal(*expected, actual) {
                        matched = false;
                        break;
                    }
                }
                if matched {
                    return Some(serde_json::json!({
                        "op": "group",
                        "by": [group_key],
                        "aggregations": [{
                            "op": "sum",
                            "field": input_value_key,
                            "name": output_value_key
                        }]
                    }));
                }
            }
        }
    }
    None
}

fn derive_project_op_from_candidate(input: &[Value], output: &[Value]) -> Option<Value> {
    if input.len() != output.len() || input.is_empty() {
        return None;
    }
    let output_keys = common_object_keys(output);
    if output_keys.is_empty() {
        return None;
    }
    let input_keys = common_object_keys(input);
    if !output_keys.iter().all(|key| input_keys.contains(key)) {
        return None;
    }
    let projected = input
        .iter()
        .map(|row| {
            let mut obj = serde_json::Map::new();
            for key in &output_keys {
                obj.insert(key.clone(), value_for_key(row, key).clone());
            }
            Value::Object(obj)
        })
        .collect::<Vec<_>>();
    (projected == output).then(|| serde_json::json!({"op": "project", "fields": output_keys}))
}

fn derive_filter_op_from_candidate(input: &[Value], output: &[Value]) -> Option<Value> {
    if output.is_empty() || output.len() >= input.len() {
        return None;
    }
    for key in common_object_keys(input) {
        let mut seen_values = std::collections::HashSet::new();
        let candidate_values = output
            .iter()
            .filter_map(|row| {
                let value = value_for_key(row, &key).clone();
                seen_values.insert(json_sort_key(&value)).then_some(value)
            })
            .collect::<Vec<_>>();
        for value in candidate_values {
            let filtered = input
                .iter()
                .filter(|row| value_for_key(row, &key) == &value)
                .cloned()
                .collect::<Vec<_>>();
            if filtered == output {
                return Some(serde_json::json!({
                    "op": "filter",
                    "field": key,
                    "cmp": "eq",
                    "value": value
                }));
            }
        }
    }
    None
}

fn derive_dedup_op_from_candidate(input: &[Value], output: &[Value]) -> Option<Value> {
    if output.is_empty() || output.len() >= input.len() {
        return None;
    }
    for key in common_object_keys(input) {
        let mut seen = std::collections::HashSet::new();
        let deduped = input
            .iter()
            .filter(|row| seen.insert(json_sort_key(value_for_key(row, &key))))
            .cloned()
            .collect::<Vec<_>>();
        if deduped == output {
            return Some(serde_json::json!({"op": "dedup", "field": key}));
        }
    }
    None
}

fn derive_aggregate_scalar_op_from_candidate(input: &[Value], output: &Value) -> Option<Value> {
    let target = numeric_value(output)?;
    for key in common_object_keys(input) {
        let values = input
            .iter()
            .map(|row| numeric_value(value_for_key(row, &key)))
            .collect::<Option<Vec<_>>>()?;
        let sum = values.iter().sum::<f64>();
        if numbers_equal(sum, target) {
            return Some(serde_json::json!({
                "op": "aggregate",
                "aggregations": [{"op": "sum", "field": key, "name": "value"}]
            }));
        }
    }
    None
}

fn unique_common_numeric_key(rows: &[Value]) -> Option<String> {
    let numeric_keys = common_object_keys(rows)
        .into_iter()
        .filter(|key| {
            rows.iter()
                .all(|row| numeric_value(value_for_key(row, key)).is_some())
        })
        .collect::<Vec<_>>();
    (numeric_keys.len() == 1).then(|| numeric_keys[0].clone())
}

fn contextual_inline_structured_transform_args_from_payload(
    text: &str,
    route_result: Option<&RouteResult>,
) -> Option<Value> {
    if !route_has_inline_transform_contract(route_result) {
        return None;
    }
    let input_value = last_transformable_input_value(text)?;
    let rows = input_value.as_array()?;
    if rows.is_empty() || !rows.iter().all(Value::is_object) {
        return None;
    }
    let sort_key = unique_common_numeric_key(rows)?;
    Some(normalize_transform_args(serde_json::json!({
        "action": "transform_data",
        "data": input_value,
        "ops": [{
            "op": "sort",
            "by": sort_key,
            "order": "desc"
        }],
        "output_format": "md_table"
    })))
}

fn inline_json_scalar_count_args_from_contract(
    text: &str,
    route_result: Option<&RouteResult>,
) -> Option<Value> {
    let route = route_result?;
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ScalarCount {
        return None;
    }
    let input_value = last_transformable_input_value(text)?;
    if !matches!(input_value, Value::Array(_)) {
        return None;
    }
    Some(normalize_transform_args(serde_json::json!({
        "action": "transform_data",
        "data": input_value,
        "ops": [{
            "op": "aggregate",
            "aggregations": [{"op": "count", "name": "count"}]
        }],
        "result_shape": "scalar",
        "output_format": "json"
    })))
}

fn derive_rename_op_from_candidate(input: &Value, output: &Value) -> Option<Value> {
    let input_obj = input.as_object()?;
    let output_obj = output.as_object()?;
    let removed = input_obj
        .iter()
        .filter(|(key, _)| !output_obj.contains_key(*key))
        .collect::<Vec<_>>();
    let added = output_obj
        .iter()
        .filter(|(key, _)| !input_obj.contains_key(*key))
        .collect::<Vec<_>>();
    if removed.len() != 1 || added.len() != 1 {
        return None;
    }
    let (from, removed_value) = removed[0];
    let (to, added_value) = added[0];
    if removed_value != added_value {
        return None;
    }
    Some(serde_json::json!({"op": "rename", "from": from, "to": to}))
}

fn inline_json_transform_args_from_candidate(
    text: &str,
    route_result: Option<&RouteResult>,
) -> Option<Value> {
    if !crate::intent::surface_signals::inline_json_transform_request(text)
        && !route_has_inline_transform_contract(route_result)
    {
        return None;
    }
    let input_value = last_transformable_input_value(text)?;
    let candidate_value =
        answer_candidate_from_route(route_result).and_then(parse_answer_candidate_value)?;
    match (&input_value, &candidate_value) {
        (Value::Array(input), Value::Array(output)) => {
            let op = derive_sort_op_from_candidate(input, output)
                .or_else(|| derive_group_sum_op_from_candidate(input, output))
                .or_else(|| derive_project_op_from_candidate(input, output))
                .or_else(|| derive_filter_op_from_candidate(input, output))
                .or_else(|| derive_dedup_op_from_candidate(input, output))?;
            Some(normalize_transform_args(serde_json::json!({
                "action": "transform_data",
                "data": input_value,
                "ops": [op],
                "output_format": "json"
            })))
        }
        (Value::Array(input), scalar) => {
            let op = derive_aggregate_scalar_op_from_candidate(input, scalar)?;
            Some(normalize_transform_args(serde_json::json!({
                "action": "transform_data",
                "data": input_value,
                "ops": [op],
                "result_shape": "scalar",
                "output_format": "json"
            })))
        }
        (Value::Object(_), Value::Object(_)) => {
            let op = derive_rename_op_from_candidate(&input_value, &candidate_value)?;
            Some(normalize_transform_args(serde_json::json!({
                "action": "transform_data",
                "data": input_value,
                "ops": [op],
                "result_shape": "single_object",
                "output_format": "json"
            })))
        }
        _ => None,
    }
}

fn answer_candidate_is_markdown_table(route_result: Option<&RouteResult>) -> bool {
    answer_candidate_from_route(route_result).is_some_and(|candidate| {
        let lines = candidate
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        lines.len() >= 2
            && lines
                .first()
                .is_some_and(|line| line.starts_with('|') && line.ends_with('|'))
            && lines
                .get(1)
                .is_some_and(|line| line.chars().all(|ch| matches!(ch, '|' | '-' | ':' | ' ')))
    })
}

fn inline_csv_transform_args_from_text(
    text: &str,
    route_result: Option<&RouteResult>,
) -> Option<Value> {
    if !crate::intent::surface_signals::inline_json_transform_request(text)
        || crate::extract_first_json_value_any(text).is_some()
        || !answer_candidate_is_markdown_table(route_result)
    {
        return None;
    }
    let csv_lines = crate::intent::surface_signals::inline_csv_record_block(text)?;
    Some(serde_json::json!({
        "action": "transform_data",
        "csv_text": csv_lines.join("\n"),
        "ops": [],
        "output_format": "md_table"
    }))
}

fn inline_json_transform_deterministic_plan_result(
    goal: &str,
    state: &AppState,
    loop_state: &LoopState,
    original_user_text: &str,
    route_result: Option<&RouteResult>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || !transform_skill_enabled_for_planning(state)
    {
        return None;
    }
    let args = inline_json_transform_args_from_text(original_user_text)
        .or_else(|| inline_json_transform_args_from_text(goal))
        .or_else(|| inline_json_scalar_count_args_from_contract(original_user_text, route_result))
        .or_else(|| inline_json_scalar_count_args_from_contract(goal, route_result))
        .or_else(|| inline_json_transform_args_from_candidate(original_user_text, route_result))
        .or_else(|| inline_json_transform_args_from_candidate(goal, route_result))
        .or_else(|| {
            contextual_inline_structured_transform_args_from_payload(
                original_user_text,
                route_result,
            )
        })
        .or_else(|| contextual_inline_structured_transform_args_from_payload(goal, route_result))
        .or_else(|| inline_csv_transform_args_from_text(original_user_text, route_result))
        .or_else(|| inline_csv_transform_args_from_text(goal, route_result))?;
    let actions = vec![AgentAction::CallSkill {
        skill: "transform".to_string(),
        args,
    }];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn replace_scalar_path_respond_only_with_auto_locator_observation(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output || is_plain_respond_only_plan(&actions).is_none() {
        return actions;
    }
    let auto_locator_path = auto_locator_path.or_else(|| {
        loop_state
            .output_vars
            .get("auto_locator_path")
            .map(String::as_str)
    });
    if let Some(observation) =
        scalar_path_auto_locator_observation_plan(route_result, auto_locator_path)
    {
        info!("plan_replace_scalar_path_respond_only_with_auto_locator_observation");
        observation
    } else {
        actions
    }
}

fn file_delivery_respond_only_observation_plan(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: &[AgentAction],
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if !route.wants_file_delivery
        && !route.output_contract.delivery_required
        && route.output_contract.response_shape != crate::OutputResponseShape::FileToken
    {
        return None;
    }
    let content = is_plain_respond_only_plan(actions)?;
    let Some((_kind, raw_path)) = crate::finalize::parse_delivery_file_token(content) else {
        return None;
    };
    let path = raw_path.trim();
    if path.is_empty() || path.contains('\n') {
        return None;
    }
    let candidate = Path::new(path);
    let resolved = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(candidate)
    };
    if !resolved.is_file() {
        return None;
    }
    let token = format!("FILE:{}", resolved.display());
    Some(vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "stat_paths",
                "paths": [resolved.display().to_string()],
                "include_missing": true,
            }),
        },
        AgentAction::Respond { content: token },
    ])
}

fn replace_file_delivery_respond_only_with_path_observation(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output || is_plain_respond_only_plan(&actions).is_none() {
        return actions;
    }
    if let Some(observation) =
        file_delivery_respond_only_observation_plan(state, route_result, &actions)
    {
        info!("plan_replace_file_delivery_respond_only_with_path_observation");
        observation
    } else {
        actions
    }
}

fn action_make_dir_path(state: &AppState, action: &AgentAction) -> Option<String> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill, args),
        AgentAction::CallTool { tool, args } => (tool, args),
        _ => return None,
    };
    let canonical = state.resolve_canonical_skill_name(skill);
    let obj = args.as_object()?;
    match canonical.as_str() {
        "make_dir" => obj.get("path").and_then(Value::as_str),
        "fs_basic" => {
            let action = obj.get("action").and_then(Value::as_str)?.trim();
            if action == "make_dir" {
                obj.get("path").and_then(Value::as_str)
            } else {
                None
            }
        }
        _ => None,
    }
    .map(str::trim)
    .filter(|path| !path.is_empty())
    .map(ToString::to_string)
}

fn file_delivery_contract_requires_file_token(route: &RouteResult) -> bool {
    route.wants_file_delivery
        || route.output_contract.delivery_required
        || route.output_contract.response_shape == crate::OutputResponseShape::FileToken
}

fn generated_file_write_action_path(state: &AppState, action: &AgentAction) -> Option<String> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill, args),
        AgentAction::CallTool { tool, args } => (tool, args),
        _ => return None,
    };
    let canonical = state.resolve_canonical_skill_name(skill);
    let obj = args.as_object()?;
    match canonical.as_str() {
        "write_file" => obj.get("path").and_then(Value::as_str),
        "fs_basic" => {
            let action = obj.get("action").and_then(Value::as_str)?.trim();
            if matches!(action, "write_text" | "append_text") {
                obj.get("path").and_then(Value::as_str)
            } else {
                None
            }
        }
        _ => None,
    }
    .map(str::trim)
    .filter(|path| !path.is_empty())
    .map(ToString::to_string)
}

fn resolve_delivery_token_path(state: &AppState, path: &str) -> PathBuf {
    let candidate = Path::new(path.trim());
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(candidate)
    }
}

fn delivery_write_parent_matches_make_dir(
    state: &AppState,
    write_path: &str,
    make_dir_path: &str,
) -> bool {
    let write_path = resolve_delivery_token_path(state, write_path);
    let make_dir_path = resolve_delivery_token_path(state, make_dir_path);
    write_path
        .parent()
        .is_some_and(|parent| same_existing_or_display_path(parent, &make_dir_path))
}

fn strip_redundant_make_dir_before_file_delivery_write(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !file_delivery_contract_requires_file_token(route) {
        return actions;
    }
    let write_paths = actions
        .iter()
        .filter_map(|action| generated_file_write_action_path(state, action))
        .collect::<Vec<_>>();
    if write_paths.is_empty() {
        return actions;
    }
    let original_len = actions.len();
    let stripped = actions
        .into_iter()
        .filter(|action| {
            let Some(make_dir_path) = action_make_dir_path(state, action) else {
                return true;
            };
            !write_paths.iter().any(|write_path| {
                delivery_write_parent_matches_make_dir(state, write_path, &make_dir_path)
            })
        })
        .collect::<Vec<_>>();
    if stripped.len() != original_len {
        info!(
            "plan_strip_redundant_make_dir_before_file_delivery_write removed={}",
            original_len.saturating_sub(stripped.len())
        );
    }
    stripped
}

fn append_file_token_after_generated_file_write_delivery(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !file_delivery_contract_requires_file_token(route) {
        return actions;
    }
    if actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::Respond { content }
                if crate::finalize::parse_delivery_file_token(content.trim()).is_some()
        )
    }) {
        return actions;
    }
    let Some(path) = actions
        .iter()
        .rev()
        .find_map(|action| generated_file_write_action_path(state, action))
    else {
        return actions;
    };
    let resolved = resolve_delivery_token_path(state, &path);
    let token = format!("FILE:{}", resolved.display());
    let mut rewritten = actions;
    match rewritten.last_mut() {
        Some(AgentAction::Respond { content }) => {
            *content = token;
        }
        _ => rewritten.push(AgentAction::Respond { content: token }),
    }
    info!("plan_append_file_token_after_generated_file_write_delivery");
    rewritten
}

fn scalar_count_locator_path(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::ScalarCount
        || route_requests_hidden_entries_count(route)
    {
        return None;
    }
    route_directory_locator_path(route, auto_locator_path)
}

fn replace_scalar_count_plan_with_count_inventory(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    user_text: &str,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output {
        return actions;
    }
    if actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { skill, .. } | AgentAction::CallTool { tool: skill, .. }
                if skill.eq_ignore_ascii_case("run_cmd")
        )
    }) {
        return actions;
    }
    let Some(path) = scalar_count_explicit_count_path_from_actions(&actions)
        .or_else(|| scalar_count_locator_path(route_result, auto_locator_path))
    else {
        return actions;
    };
    if !Path::new(&path).is_dir() {
        info!("plan_replace_scalar_count_missing_locator_with_path_facts path={path}");
        let answer = if crate::language_policy::request_language_hint(user_text)
            .to_ascii_lowercase()
            .starts_with("en")
        {
            format!("{path} does not exist, so the matching item count cannot be computed.")
        } else {
            format!("{path} 不存在，无法统计匹配项数量。")
        };
        return vec![
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "stat_paths",
                    "paths": [path],
                    "include_missing": true,
                }),
            },
            AgentAction::Respond { content: answer },
        ];
    }
    info!("plan_replace_scalar_count_plan_with_count_inventory");
    if scalar_count_actions_include_listing(&actions) {
        info!("plan_scalar_count_listing_requires_structured_count_repair");
        return actions;
    }
    let inventory_kind = scalar_count_inventory_kind_from_actions(&actions);
    let mut args = serde_json::json!({
        "action": "count_entries",
        "path": path,
    });
    if let Some(obj) = args.as_object_mut() {
        apply_scalar_count_inventory_filters_from_actions(obj, &actions);
        match inventory_kind {
            ScalarCountInventoryKind::Any => {}
            ScalarCountInventoryKind::Files => {
                obj.insert("kind_filter".to_string(), Value::String("file".to_string()));
                obj.insert("count_files".to_string(), Value::Bool(true));
                obj.insert("count_dirs".to_string(), Value::Bool(false));
                obj.insert("files_only".to_string(), Value::Bool(true));
            }
            ScalarCountInventoryKind::Dirs => {
                obj.insert("kind_filter".to_string(), Value::String("dir".to_string()));
                obj.insert("count_files".to_string(), Value::Bool(false));
                obj.insert("count_dirs".to_string(), Value::Bool(true));
                obj.insert("dirs_only".to_string(), Value::Bool(true));
            }
        }
    }
    vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args,
    }]
}

fn scalar_count_actions_include_listing(actions: &[AgentAction]) -> bool {
    actions.iter().any(|action| {
        let (skill, args) = match action {
            AgentAction::CallSkill { skill, args }
            | AgentAction::CallTool { tool: skill, args } => (skill.as_str(), args),
            _ => return false,
        };
        let action_name = args
            .get("action")
            .and_then(Value::as_str)
            .map(|value| value.trim().to_ascii_lowercase());
        skill.eq_ignore_ascii_case("list_dir")
            || (skill.eq_ignore_ascii_case("fs_basic")
                && matches!(action_name.as_deref(), Some("list_dir")))
            || (skill.eq_ignore_ascii_case("system_basic")
                && matches!(action_name.as_deref(), Some("inventory_dir")))
    })
}

fn scalar_count_explicit_count_path_from_actions(actions: &[AgentAction]) -> Option<String> {
    let mut selected: Option<String> = None;
    for action in actions {
        let (skill, args) = match action {
            AgentAction::CallSkill { skill, args }
            | AgentAction::CallTool { tool: skill, args } => (skill.as_str(), args),
            _ => continue,
        };
        let action_name = args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        let is_count_action = (skill.eq_ignore_ascii_case("fs_basic")
            && action_name.eq_ignore_ascii_case("count_entries"))
            || (skill.eq_ignore_ascii_case("system_basic")
                && action_name.eq_ignore_ascii_case("count_inventory"));
        if !is_count_action {
            continue;
        }
        let Some(path) = args
            .get("path")
            .or_else(|| args.get("root"))
            .or_else(|| args.get("dir"))
            .or_else(|| args.get("directory"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
        else {
            continue;
        };
        match &selected {
            None => selected = Some(path.to_string()),
            Some(existing) if existing == path => {}
            Some(_) => return None,
        }
    }
    selected
}

fn apply_scalar_count_inventory_filters_from_actions(
    out: &mut serde_json::Map<String, Value>,
    actions: &[AgentAction],
) {
    for args in actions.iter().filter_map(action_structured_args) {
        let Some(obj) = args.as_object() else {
            continue;
        };
        for key in ["include_hidden", "recursive"] {
            if out.get(key).is_none() {
                if let Some(value) = obj.get(key).and_then(Value::as_bool) {
                    out.insert(key.to_string(), Value::Bool(value));
                }
            }
        }
        if out.get("ext_filter").is_none() {
            if let Some(value) = structured_ext_filter_arg(obj) {
                out.insert("ext_filter".to_string(), value);
            }
        }
    }
}

fn structured_ext_filter_arg(obj: &serde_json::Map<String, Value>) -> Option<Value> {
    for key in ["ext_filter", "ext", "extension", "extensions"] {
        let Some(value) = obj.get(key) else {
            continue;
        };
        match value {
            Value::String(text) if !text.trim().is_empty() => {
                return Some(Value::String(text.trim().to_string()));
            }
            Value::Array(items)
                if items
                    .iter()
                    .any(|item| item.as_str().is_some_and(|text| !text.trim().is_empty())) =>
            {
                return Some(Value::Array(items.clone()));
            }
            _ => {}
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScalarCountInventoryKind {
    Any,
    Files,
    Dirs,
}

fn scalar_count_inventory_kind_from_actions(actions: &[AgentAction]) -> ScalarCountInventoryKind {
    let mut inferred: Option<ScalarCountInventoryKind> = None;
    for args in actions.iter().filter_map(action_structured_args) {
        let Some(kind) = scalar_count_inventory_kind_from_args(args) else {
            continue;
        };
        match inferred {
            None => inferred = Some(kind),
            Some(existing) if existing == kind => {}
            Some(_) => return ScalarCountInventoryKind::Any,
        }
    }
    inferred.unwrap_or(ScalarCountInventoryKind::Any)
}

fn action_structured_args(action: &AgentAction) -> Option<&Value> {
    match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => Some(args),
        _ => None,
    }
}

fn scalar_count_inventory_kind_from_args(args: &Value) -> Option<ScalarCountInventoryKind> {
    let obj = args.as_object()?;
    let files_only = structured_true_arg(
        args,
        &[
            "files_only",
            "file_only",
            "regular_files_only",
            "regular_file_only",
        ],
    );
    let dirs_only = structured_true_arg(
        args,
        &[
            "dirs_only",
            "dir_only",
            "directories_only",
            "directory_only",
            "folders_only",
            "folder_only",
        ],
    );
    if files_only && !dirs_only {
        return Some(ScalarCountInventoryKind::Files);
    }
    if dirs_only && !files_only {
        return Some(ScalarCountInventoryKind::Dirs);
    }
    if structured_ext_filter_arg(obj).is_some() {
        return Some(ScalarCountInventoryKind::Files);
    }

    let count_files = structured_bool_arg(args, "count_files");
    let count_dirs = structured_bool_arg(args, "count_dirs");
    match (count_files, count_dirs) {
        (Some(true), Some(false)) => return Some(ScalarCountInventoryKind::Files),
        (Some(false), Some(true)) => return Some(ScalarCountInventoryKind::Dirs),
        _ => {}
    }

    for key in [
        "kind_filter",
        "target_kind",
        "kind",
        "entry_kind",
        "entry_type",
        "item_kind",
        "item_type",
    ] {
        if let Some(kind) = obj
            .get(key)
            .and_then(Value::as_str)
            .and_then(scalar_count_inventory_kind_from_structured_value)
        {
            return Some(kind);
        }
    }
    None
}

fn structured_true_arg(args: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .any(|key| matches!(structured_bool_arg(args, key), Some(true)))
}

fn structured_bool_arg(args: &Value, key: &str) -> Option<bool> {
    match args.as_object()?.get(key)? {
        Value::Bool(value) => Some(*value),
        Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" => Some(true),
            "false" | "no" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn scalar_count_inventory_kind_from_structured_value(
    value: &str,
) -> Option<ScalarCountInventoryKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "file" | "files" | "regular_file" | "regular_files" => {
            Some(ScalarCountInventoryKind::Files)
        }
        "dir" | "dirs" | "directory" | "directories" | "folder" | "folders" => {
            Some(ScalarCountInventoryKind::Dirs)
        }
        _ => None,
    }
}

fn hidden_entries_count_locator_path(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || !route_requests_hidden_entries_count(route)
    {
        return None;
    }
    route_directory_locator_path(route, auto_locator_path)
}

fn route_directory_locator_path(
    route: &RouteResult,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let hint = route.output_contract.locator_hint.trim();
    let current_workspace_fallback =
        route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace;
    let hint_looks_like_path = hint.contains(['/', '\\']) || hint.starts_with('.');
    let auto_dir = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .filter(|path| Path::new(path).is_dir());
    if !hint.is_empty() {
        if let Some(path) = auto_dir.filter(|path| locator_path_matches_hint(path, hint)) {
            return Some(path.to_string());
        }
        if Path::new(hint).is_dir()
            || matches!(
                route.output_contract.locator_kind,
                crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
            )
            || hint_looks_like_path
        {
            return Some(hint.to_string());
        }
    }
    auto_dir
        .or_else(|| (current_workspace_fallback || hint.is_empty()).then_some("."))
        .map(ToString::to_string)
}

fn locator_path_matches_hint(path: &str, hint: &str) -> bool {
    let path = path.trim().trim_end_matches(['/', '\\']);
    let hint = hint.trim().trim_end_matches(['/', '\\']);
    if path.is_empty() || hint.is_empty() {
        return false;
    }
    if path.eq_ignore_ascii_case(hint) {
        return true;
    }
    path.ends_with(&format!("/{hint}")) || path.ends_with(&format!("\\{hint}"))
}

fn route_requests_hidden_entries_count(route: &RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::HiddenEntriesCheck
}

fn replace_hidden_entries_count_plan_with_inventory_dir(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output {
        return actions;
    }
    let Some(path) = hidden_entries_count_locator_path(route_result, auto_locator_path) else {
        return actions;
    };
    info!("plan_replace_hidden_entries_count_plan_with_inventory_dir");
    vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "list_dir",
            "path": path,
            "include_hidden": true,
            "names_only": true,
            "max_entries": 1000,
        }),
    }]
}

fn route_requests_service_status(route: &RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::ServiceStatus
        || (route.output_contract.semantic_kind == crate::OutputSemanticKind::ContentExcerptSummary
            && route.output_contract.requires_content_evidence
            && !route.output_contract.delivery_required
            && route.output_contract.locator_kind == crate::OutputLocatorKind::None)
}

fn safe_service_status_target(raw: &str) -> Option<String> {
    let target = raw.trim().trim_matches(|ch| matches!(ch, '"' | '\'' | '`'));
    if target.is_empty()
        || !target
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
    {
        return None;
    }
    Some(target.to_string())
}

fn shell_like_words(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in command.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if matches!(ch, '\'' | '"') {
            quote = Some(ch);
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn command_basename(token: &str) -> &str {
    token.rsplit('/').next().unwrap_or(token)
}

fn token_starts_new_shell_clause(token: &str) -> bool {
    let trimmed = token.trim();
    trimmed.is_empty()
        || matches!(
            trimmed,
            ">" | ">>" | "2>" | "2>>" | "<" | "|" | "||" | "&&" | ";"
        )
        || trimmed.contains('|')
        || trimmed.contains(';')
        || trimmed.contains('<')
        || trimmed.contains('>')
        || trimmed == "if"
        || trimmed == "then"
        || trimmed == "else"
        || trimmed == "fi"
}

fn pgrep_status_target(words: &[String]) -> Option<String> {
    if words
        .first()
        .map(|word| command_basename(word).eq_ignore_ascii_case("pgrep"))
        != Some(true)
    {
        return None;
    }
    words
        .iter()
        .skip(1)
        .take_while(|word| !token_starts_new_shell_clause(word))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .find(|word| !word.starts_with('-'))
        .and_then(|word| safe_service_status_target(word))
}

fn systemctl_status_target(words: &[String]) -> Option<(String, Option<&'static str>)> {
    if words
        .first()
        .map(|word| command_basename(word).eq_ignore_ascii_case("systemctl"))
        != Some(true)
    {
        return None;
    }
    let subcommand_idx = words.iter().position(|word| {
        let word = word.trim();
        word.eq_ignore_ascii_case("is-active") || word.eq_ignore_ascii_case("status")
    })?;
    words
        .iter()
        .skip(subcommand_idx + 1)
        .find(|word| !word.starts_with('-'))
        .and_then(|word| safe_service_status_target(word))
        .map(|target| (target, Some("systemd")))
}

fn service_command_status_target(words: &[String]) -> Option<(String, Option<&'static str>)> {
    if words
        .first()
        .map(|word| command_basename(word).eq_ignore_ascii_case("service"))
        != Some(true)
        || words.len() < 3
        || !words[2].eq_ignore_ascii_case("status")
    {
        return None;
    }
    safe_service_status_target(&words[1]).map(|target| (target, Some("service")))
}

fn service_status_command_target(command: &str) -> Option<(String, Option<&'static str>)> {
    let words = shell_like_words(command);
    pgrep_status_target(&words)
        .map(|target| (target, None))
        .or_else(|| systemctl_status_target(&words))
        .or_else(|| service_command_status_target(&words))
}

fn service_status_target_for_action(
    route: &RouteResult,
    action: &AgentAction,
) -> Option<(String, Option<&'static str>)> {
    let route_target = safe_service_status_target(route.output_contract.locator_hint.trim());
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let skill = skill.trim().to_ascii_lowercase();
            let command = if skill == "run_cmd" {
                args.get("command").and_then(Value::as_str)
            } else if skill == "system_basic"
                && args
                    .get("action")
                    .and_then(Value::as_str)
                    .is_some_and(|action| action.trim().eq_ignore_ascii_case("run_cmd"))
            {
                args.get("command").and_then(Value::as_str)
            } else {
                None
            };
            command
                .and_then(service_status_command_target)
                .or_else(|| route_target.map(|target| (target, None)))
        }
        AgentAction::CallCapability { .. }
        | AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => None,
    }
}

fn rewrite_service_status_plan_to_service_control(
    route_result: Option<&RouteResult>,
    preserve_explicit_command: bool,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if preserve_explicit_command {
        return actions;
    }
    let Some(route) = route_result else {
        return actions;
    };
    if !route_requests_service_status(route) {
        return actions;
    }
    let mut rewritten = actions;
    let mut changed = false;
    for action in rewritten.iter_mut() {
        let should_consider = matches!(
            action,
            AgentAction::CallSkill { skill, .. } | AgentAction::CallTool { tool: skill, .. }
                if {
                    let skill = skill.trim().to_ascii_lowercase();
                    skill == "run_cmd" || skill == "system_basic"
                }
        );
        if !should_consider {
            continue;
        }
        let Some((target, manager_type)) = service_status_target_for_action(route, action) else {
            continue;
        };
        let mut args = serde_json::json!({
            "action": "status",
            "target": target,
        });
        if let Some(manager_type) = manager_type {
            args["manager_type"] = Value::String(manager_type.to_string());
        }
        *action = AgentAction::CallSkill {
            skill: "service_control".to_string(),
            args,
        };
        changed = true;
    }
    if changed {
        while rewritten.last().is_some_and(is_discussion_followup_action) {
            rewritten.pop();
        }
        info!("plan_rewrite_service_status_to_service_control");
    }
    rewritten
}

fn is_service_control_status_action(action: &AgentAction) -> bool {
    matches!(
        action,
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.trim().eq_ignore_ascii_case("service_control")
                && args
                    .get("action")
                    .and_then(Value::as_str)
                    .is_some_and(|action| action.trim().eq_ignore_ascii_case("status"))
    )
}

fn strip_service_status_discussion_actions(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ServiceStatus
        || !actions.iter().any(is_service_control_status_action)
        || !actions.iter().any(is_discussion_followup_action)
    {
        return actions;
    }
    let rewritten = actions
        .into_iter()
        .filter(|action| !is_discussion_followup_action(action))
        .collect::<Vec<_>>();
    if rewritten.is_empty() {
        return rewritten;
    }
    info!("plan_strip_service_status_discussion_actions");
    rewritten
}

fn structured_keys_locator_path(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::StructuredKeys
    {
        return None;
    }
    let hint = route.output_contract.locator_hint.trim();
    auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or_else(|| (!hint.is_empty()).then_some(hint))
        .filter(|path| Path::new(path).is_file())
        .map(ToString::to_string)
}

fn replace_structured_keys_read_plan(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output || actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { skill, args }
                | AgentAction::CallTool { tool: skill, args }
                if (skill.eq_ignore_ascii_case("config_basic")
                    && args.get("action").and_then(Value::as_str) == Some("list_keys")
                    && args
                        .get("path")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|path| !path.is_empty())
                        .is_some())
                    || (skill.eq_ignore_ascii_case("system_basic")
                        && args.get("action").and_then(Value::as_str) == Some("structured_keys"))
        )
    }) {
        return actions;
    }
    if actions
        .iter()
        .any(action_is_structured_field_read_with_explicit_field)
    {
        return actions;
    }
    let Some(path) = structured_keys_locator_path(route_result, auto_locator_path) else {
        return actions;
    };
    if !actions.iter().any(|action| {
        matches!(
            action,
                AgentAction::CallSkill { skill, args }
                    | AgentAction::CallTool { tool: skill, args }
                    if (skill.eq_ignore_ascii_case("fs_basic")
                        && args.get("action").and_then(Value::as_str) == Some("read_text_range"))
                        || (skill.eq_ignore_ascii_case("config_basic")
                            && matches!(
                                args.get("action").and_then(Value::as_str),
                                Some("read_field" | "read_fields" | "validate")
                            ))
                        || (skill.eq_ignore_ascii_case("system_basic")
                            && matches!(
                                args.get("action").and_then(Value::as_str),
                                Some("read_range" | "extract_field" | "extract_fields" | "validate_structured")
                            ))
        )
    }) {
        return actions;
    }
    info!("plan_replace_structured_keys_read_plan_with_structured_keys");
    vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: serde_json::json!({
            "action": "list_keys",
            "path": path,
            "max_keys": 1000,
        }),
    }]
}

fn has_structured_keys_observation(loop_state: &LoopState, path: &str) -> bool {
    let requested_path = path.trim();
    loop_state.executed_step_results.iter().rev().any(|step| {
        if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "config_basic") {
            return false;
        }
        let Some(output) = step.output.as_deref() else {
            return false;
        };
        let Ok(value) = serde_json::from_str::<Value>(output) else {
            return false;
        };
        if value.get("action").and_then(Value::as_str) != Some("structured_keys") {
            return false;
        }
        if !value
            .get("exists")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return false;
        }
        if !value.get("keys").is_some_and(Value::is_array)
            && !value.get("identity_values").is_some_and(Value::is_array)
            && !value.get("indices_preview").is_some_and(Value::is_array)
        {
            return false;
        }
        if requested_path.is_empty() {
            return true;
        }
        value
            .get("resolved_path")
            .or_else(|| value.get("path"))
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|observed| observed == requested_path)
    })
}

fn structured_keys_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    user_text: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    let route = route_result?;
    let path = structured_keys_locator_path(route_result, auto_locator_path)?;
    if has_structured_keys_observation(loop_state, &path) {
        return None;
    }
    let enabled_skills = state.get_skills_list();
    if !enabled_skills.is_empty() && !enabled_skills.contains("config_basic") {
        return None;
    }
    let field_path = structured_current_turn_field_selectors(route, user_text, Some(&path))
        .into_iter()
        .next();
    if let Some(field_path) = field_path.as_deref() {
        if structured_field_path_resolves_scalar_value(&path, field_path) {
            let actions = vec![config_basic_read_field_action(
                path.to_string(),
                field_path.to_string(),
            )];
            let raw_plan_text = serde_json::json!({
                "steps": [{
                    "type": "call_tool",
                    "tool": "config_basic",
                    "args": actions
                        .first()
                        .and_then(|action| match action {
                            AgentAction::CallTool { args, .. } => Some(args.clone()),
                            _ => None,
                        })
                        .unwrap_or_else(|| serde_json::json!({})),
                }]
            })
            .to_string();
            return Some(build_plan_result(
                goal,
                &raw_plan_text,
                if loop_state.round_no <= 1 {
                    PlanKind::Single
                } else {
                    PlanKind::Incremental
                },
                &actions,
            ));
        }
    }
    let mut args = serde_json::json!({
        "action": "list_keys",
        "path": path,
        "max_keys": 1000,
    });
    if let Some(field_path) = field_path {
        args["field_path"] = Value::String(field_path);
    }
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args,
    }];
    let raw_plan_text = serde_json::json!({
        "steps": [{
            "type": "call_tool",
            "tool": "config_basic",
            "args": actions
                .first()
                .and_then(|action| match action {
                    AgentAction::CallTool { args, .. } => Some(args.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| serde_json::json!({})),
        }]
    })
    .to_string();
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        if loop_state.round_no <= 1 {
            PlanKind::Single
        } else {
            PlanKind::Incremental
        },
        &actions,
    ))
}

fn action_is_structured_field_read_with_explicit_field(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let action_name = args
                .get("action")
                .and_then(Value::as_str)
                .map(|value| value.trim().to_ascii_lowercase());
            let is_field_read = if skill.eq_ignore_ascii_case("config_basic") {
                matches!(action_name.as_deref(), Some("read_field" | "read_fields"))
            } else if skill.eq_ignore_ascii_case("system_basic") {
                matches!(
                    action_name.as_deref(),
                    Some("extract_field" | "extract_fields")
                )
            } else {
                false
            };
            if !is_field_read {
                return false;
            }
            if json_trimmed_string_arg(args, &["field_path", "field", "key"]).is_some() {
                return true;
            }
            let field_count = args
                .get("field_paths")
                .or_else(|| args.get("fields"))
                .map(|value| string_list_from_value(Some(value)).len())
                .unwrap_or_default();
            field_count == 1
        }
        AgentAction::CallCapability { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Think { .. } => false,
    }
}

fn action_observes_bounded_file_content(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let skill = skill.trim();
            if skill.eq_ignore_ascii_case("run_cmd") {
                return run_cmd_command_from_args(args)
                    .and_then(readonly_file_read_from_shell_command)
                    .is_some();
            }
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            ((skill.eq_ignore_ascii_case("system_basic")
                && action.eq_ignore_ascii_case("read_range"))
                || (skill.eq_ignore_ascii_case("fs_basic")
                    && action.eq_ignore_ascii_case("read_text_range")))
                || skill.eq_ignore_ascii_case("read_file")
                || (skill.eq_ignore_ascii_case("doc_parse")
                    && action.eq_ignore_ascii_case("parse_doc"))
        }
        AgentAction::CallCapability { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => false,
    }
}

fn planned_bounded_file_read_path(action: &AgentAction) -> Option<&str> {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let skill = skill.trim();
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            let is_bounded_read = (skill.eq_ignore_ascii_case("system_basic")
                && action.eq_ignore_ascii_case("read_range"))
                || (skill.eq_ignore_ascii_case("fs_basic")
                    && action.eq_ignore_ascii_case("read_text_range"))
                || skill.eq_ignore_ascii_case("read_file");
            is_bounded_read
                .then(|| args.get("path").and_then(Value::as_str))
                .flatten()
        }
        AgentAction::CallCapability { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => None,
    }
}

fn planned_structured_config_observation_path(action: &AgentAction) -> Option<&str> {
    match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. }
            if action_is_readonly_config_observation(action) =>
        {
            args.get("path").and_then(Value::as_str).map(str::trim)
        }
        _ => None,
    }
    .filter(|path| !path.is_empty())
}

fn route_allows_single_document_parse_synthesis(route: &RouteResult) -> bool {
    !route.needs_clarify
        && route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
        && matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
}

fn prefer_doc_parse_for_single_document_synthesis(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output
        || !doc_parse_is_enabled(state)
        || !actions
            .iter()
            .any(|action| matches!(action, AgentAction::SynthesizeAnswer { .. }))
    {
        return actions;
    }
    let Some(route) = route_result else {
        return actions;
    };
    if route
        .output_contract
        .semantic_kind
        .is_content_excerpt_summary()
    {
        return actions;
    }
    if !route_allows_single_document_parse_synthesis(route) {
        return actions;
    }
    let mut rewritten = Vec::with_capacity(actions.len());
    let mut changed = false;
    for action in actions {
        if let Some(path) = planned_bounded_file_read_path(&action)
            .filter(|path| Path::new(path).is_file())
            .filter(|path| doc_parse_supported_path(path))
            .filter(|path| !repo_text_artifact_prefers_bounded_fs_read(path))
        {
            rewritten.push(AgentAction::CallSkill {
                skill: "doc_parse".to_string(),
                args: serde_json::json!({
                    "action": "parse_doc",
                    "path": path,
                    "mode": "auto",
                    "max_chars": 12000,
                    "include_metadata": true
                }),
            });
            changed = true;
        } else {
            rewritten.push(action);
        }
    }
    if changed {
        info!("plan_prefer_doc_parse_for_single_document_synthesis");
    }
    rewritten
}

fn prefer_log_analyze_for_single_log_synthesis(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output
        || !log_analyze_is_enabled(state)
        || !actions
            .iter()
            .any(|action| matches!(action, AgentAction::SynthesizeAnswer { .. }))
    {
        return actions;
    }
    let Some(route) = route_result else {
        return actions;
    };
    if route
        .output_contract
        .semantic_kind
        .is_content_excerpt_summary()
    {
        return actions;
    }
    if !route_allows_single_document_parse_synthesis(route) {
        return actions;
    }
    let mut rewritten = Vec::with_capacity(actions.len());
    let mut changed = false;
    for action in actions {
        if let Some(path) = planned_bounded_file_read_path(&action)
            .filter(|path| Path::new(path).is_file())
            .filter(|path| log_analyze_supported_path(path))
        {
            rewritten.push(AgentAction::CallSkill {
                skill: "log_analyze".to_string(),
                args: serde_json::json!({
                    "path": path,
                    "max_matches": 50
                }),
            });
            changed = true;
        } else {
            rewritten.push(action);
        }
    }
    if changed {
        info!("plan_prefer_log_analyze_for_single_log_synthesis");
    }
    rewritten
}

fn existence_path_summary_target_path(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind
            != crate::OutputSemanticKind::ExistenceWithPathSummary
    {
        return None;
    }
    auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or_else(|| {
            let hint = route.output_contract.locator_hint.trim();
            (!hint.is_empty() && Path::new(hint).is_file()).then_some(hint)
        })
        .map(ToString::to_string)
}

fn ensure_existence_path_summary_has_bounded_content(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output {
        return actions;
    }
    let Some(path) = existence_path_summary_target_path(route_result, auto_locator_path) else {
        return actions;
    };
    let mut rewritten = actions;
    if !rewritten.iter().any(action_observes_bounded_file_content)
        && !path_metadata_facts_response_is_sufficient(&rewritten)
    {
        let insert_at = rewritten
            .iter()
            .position(|action| {
                matches!(
                    action,
                    AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. }
                )
            })
            .unwrap_or(rewritten.len());
        rewritten.insert(
            insert_at,
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "read_text_range",
                    "path": path,
                    "mode": "head",
                    "n": 30
                }),
            },
        );
        info!("plan_insert_existence_path_summary_read_range");
    }
    if !rewritten
        .iter()
        .any(|action| matches!(action, AgentAction::SynthesizeAnswer { .. }))
    {
        let evidence_refs = observation_action_evidence_refs(&rewritten);
        if !evidence_refs.is_empty() {
            let insert_at = rewritten
                .iter()
                .rposition(|action| matches!(action, AgentAction::Respond { .. }))
                .unwrap_or(rewritten.len());
            rewritten.insert(
                insert_at,
                AgentAction::SynthesizeAnswer {
                    evidence_refs: evidence_refs.clone(),
                },
            );
            match rewritten.get_mut(insert_at + 1) {
                Some(AgentAction::Respond { content }) => {
                    *content = "{{last_output}}".to_string();
                }
                _ => rewritten.push(AgentAction::Respond {
                    content: "{{last_output}}".to_string(),
                }),
            }
            info!(
                "plan_insert_existence_path_summary_synthesis refs={}",
                evidence_refs.join(",")
            );
        }
    }
    rewritten
}

fn planned_action_is_path_metadata_facts(action: &AgentAction) -> bool {
    let Some(skill) = planned_action_skill_name(action).map(str::trim) else {
        return false;
    };
    let Some(action_name) = action_args(action)
        .and_then(|args| args.get("action"))
        .and_then(Value::as_str)
        .map(str::trim)
    else {
        return false;
    };
    matches!(
        (
            skill.to_ascii_lowercase().as_str(),
            action_name.to_ascii_lowercase().as_str()
        ),
        ("fs_basic", "stat_paths")
            | ("fs_basic", "compare_paths")
            | ("system_basic", "path_batch_facts")
            | ("system_basic", "compare_paths")
    )
}

fn path_metadata_facts_action_requests_metadata_fields(action: &AgentAction) -> bool {
    let (AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. }) = action else {
        return false;
    };
    let Some(fields) = args.get("fields").and_then(Value::as_array) else {
        return false;
    };
    fields.iter().any(|field| {
        field.as_str().map(str::trim).is_some_and(|field| {
            matches!(
                field.to_ascii_lowercase().as_str(),
                "size" | "size_bytes" | "exists" | "kind" | "modified" | "modified_ts"
            )
        })
    })
}

fn path_metadata_facts_response_is_sufficient(actions: &[AgentAction]) -> bool {
    if !actions.iter().any(planned_action_is_path_metadata_facts) {
        return false;
    }
    let Some(AgentAction::Respond { content }) = actions.last() else {
        return false;
    };
    if !content.contains("{{") {
        return false;
    }
    let metadata_fields = [
        "size",
        "size_bytes",
        "exists",
        "kind",
        "modified",
        "modified_ts",
    ];
    let content_lower = content.to_ascii_lowercase();
    let placeholder_refs = extract_output_placeholder_evidence_refs(content);
    let response_mentions_metadata = metadata_fields
        .iter()
        .any(|field| content_lower.contains(field));
    let placeholder_mentions_metadata = placeholder_refs.iter().any(|reference| {
        let reference = reference.trim().to_ascii_lowercase();
        metadata_fields
            .iter()
            .any(|field| reference == *field || reference.ends_with(&format!(".{field}")))
    });
    placeholder_mentions_metadata
        || response_mentions_metadata
        || actions
            .iter()
            .filter(|action| planned_action_is_path_metadata_facts(action))
            .any(path_metadata_facts_action_requests_metadata_fields)
}

fn strip_configured_command_prefix<'a>(request: &'a str, prefix: &str) -> Option<&'a str> {
    let request = request.trim_start();
    let prefix = prefix.trim_start();
    if request.is_empty() || prefix.is_empty() {
        return None;
    }
    if prefix.is_ascii() {
        let request_lower = request.to_ascii_lowercase();
        let prefix_lower = prefix.to_ascii_lowercase();
        request_lower
            .starts_with(&prefix_lower)
            .then(|| &request[prefix.len()..])
    } else {
        request
            .starts_with(prefix)
            .then(|| &request[prefix.len()..])
    }
}

fn trim_leading_command_delimiters(mut text: &str) -> &str {
    loop {
        text = text.trim_start();
        let Some(ch) = text.chars().next() else {
            return text;
        };
        if matches!(
            ch,
            ':' | '：' | '-' | '—' | '–' | '`' | '"' | '\'' | '“' | '”' | ' '
        ) {
            text = &text[ch.len_utf8()..];
            continue;
        }
        return text;
    }
}

fn trim_leading_command_separators_preserve_quotes(mut text: &str) -> &str {
    loop {
        text = text.trim_start();
        let Some(ch) = text.chars().next() else {
            return text;
        };
        if matches!(ch, ':' | '：' | '-' | '—' | '–' | ' ') {
            text = &text[ch.len_utf8()..];
            continue;
        }
        return text;
    }
}

fn looks_like_concrete_command_tail(tail: &str) -> bool {
    let tail = trim_leading_command_delimiters(tail);
    let first_token = tail
        .split_whitespace()
        .next()
        .unwrap_or(tail)
        .trim_matches(|ch: char| {
            ch.is_ascii_punctuation()
                || matches!(ch, '，' | '。' | '；' | '：' | '、' | '！' | '？')
        });
    first_token
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .count()
        >= 2
}

fn explicit_command_segment_before_followup(tail: &str) -> Option<&str> {
    let tail = trim_leading_command_separators_preserve_quotes(tail);
    let boundary = tail.char_indices().find_map(|(idx, ch)| {
        (idx > 0 && matches!(ch, ',' | '，' | ';' | '；' | '。' | '\n')).then_some(idx)
    })?;
    Some(&tail[..boundary])
}

fn explicit_command_followup_tail(tail: &str) -> Option<&str> {
    let tail = trim_leading_command_separators_preserve_quotes(tail);
    let boundary = tail.char_indices().find_map(|(idx, ch)| {
        (idx > 0 && matches!(ch, ',' | '，' | ';' | '；' | '。' | '\n')).then_some(idx)
    })?;
    let delimiter_len = tail[boundary..]
        .chars()
        .next()
        .map(char::len_utf8)
        .unwrap_or(0);
    Some(tail[boundary + delimiter_len..].trim())
}

fn whole_explicit_command_tail(tail: &str) -> Option<&str> {
    let tail = trim_leading_command_separators_preserve_quotes(tail).trim();
    if tail.is_empty() || tail.contains('\n') {
        return None;
    }
    if tail
        .chars()
        .any(|ch| matches!(ch, '|' | ';' | '&' | '>' | '<'))
    {
        return Some(tail);
    }
    let mut tokens = tail.split_whitespace();
    let first = tokens.next()?;
    if tokens.clone().next().is_none() {
        return Some(first);
    }
    tokens
        .all(structural_command_argument_token)
        .then_some(tail)
}

fn markdown_code_span_command_segment(text: &str) -> Option<&str> {
    let text = text.trim();
    let rest = text.strip_prefix('`')?;
    let close = rest.find('`')?;
    let command = rest[..close].trim();
    if command.is_empty() {
        return None;
    }
    let suffix = rest[close + '`'.len_utf8()..].trim();
    if suffix.chars().all(|ch| {
        matches!(
            ch,
            '.' | '。' | '!' | '！' | '?' | '？' | ',' | '，' | ';' | '；'
        )
    }) {
        Some(command)
    } else {
        None
    }
}

fn structural_command_argument_token(token: &str) -> bool {
    let token = token.trim_matches(|ch: char| {
        ch.is_ascii_punctuation() && !matches!(ch, '-' | '_' | '.' | '/' | '\\' | '~' | '=')
    });
    if token.is_empty() {
        return false;
    }
    let quoted = (token.starts_with('"') && token.ends_with('"'))
        || (token.starts_with('\'') && token.ends_with('\''));
    let flag = token.starts_with('-') && token.chars().any(|ch| ch.is_ascii_alphanumeric());
    let path_like = token.starts_with('/')
        || token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with("~/")
        || token.contains('/')
        || token.contains('\\')
        || token.contains('.');
    let assignment = token
        .split_once('=')
        .is_some_and(|(name, value)| !name.is_empty() && !value.is_empty());
    quoted || flag || path_like || assignment
}

fn configured_standalone_command_token_value<'a>(
    runtime: &'a crate::CommandIntentRuntime,
    token: &str,
) -> Option<&'a str> {
    runtime.standalone_commands.iter().find_map(|candidate| {
        if candidate.is_ascii() && token.is_ascii() {
            candidate
                .eq_ignore_ascii_case(token)
                .then_some(candidate.as_str())
        } else {
            (candidate == token).then_some(candidate.as_str())
        }
    })
}

fn configured_standalone_command_token(runtime: &crate::CommandIntentRuntime, token: &str) -> bool {
    configured_standalone_command_token_value(runtime, token).is_some()
}

fn command_candidate_end_boundary(text: &str, end_idx: usize) -> bool {
    if end_idx >= text.len() {
        return true;
    }
    let Some(next) = text[end_idx..].chars().next() else {
        return true;
    };
    !next.is_ascii_alphanumeric() && !matches!(next, '_' | '-' | '/' | '\\' | '~' | '`')
}

fn configured_standalone_command_sequence_from_segment(
    runtime: &crate::CommandIntentRuntime,
    segment: &str,
) -> Option<String> {
    let segment = trim_leading_command_separators_preserve_quotes(segment).trim();
    if segment.is_empty()
        || segment.contains('\n')
        || segment.contains('`')
        || segment
            .chars()
            .any(|ch| matches!(ch, '|' | ';' | '&' | '>' | '<'))
    {
        return None;
    }

    let mut commands = Vec::new();
    for (idx, ch) in segment.char_indices() {
        if !ch.is_ascii_alphabetic() || !command_candidate_start_boundary(segment, idx) {
            continue;
        }
        let mut end = idx;
        for (offset, candidate) in segment[idx..].char_indices() {
            if candidate.is_ascii_alphanumeric() || matches!(candidate, '_' | '-') {
                end = idx + offset + candidate.len_utf8();
                continue;
            }
            break;
        }
        if end <= idx || !command_candidate_end_boundary(segment, end) {
            continue;
        }
        let token = &segment[idx..end];
        if !simple_bare_command_token(token) {
            continue;
        }
        if let Some(canonical) = configured_standalone_command_token_value(runtime, token) {
            commands.push(canonical.to_string());
        }
    }

    (commands.len() >= 2).then(|| commands.join("; "))
}

fn standalone_command_segment_before_freeform_tail<'a>(
    runtime: &crate::CommandIntentRuntime,
    tail: &'a str,
) -> Option<&'a str> {
    let tail = trim_leading_command_separators_preserve_quotes(tail).trim();
    if tail.is_empty() || tail.contains('\n') {
        return None;
    }

    let mut tokens = tail.split_whitespace();
    let first = tokens.next()?;
    let first_start = tail.find(first)?;
    let first_end = first_start + first.len();
    let first_token =
        first.trim_matches(|ch: char| ch.is_ascii_punctuation() && !matches!(ch, '_' | '-' | '.'));
    if !simple_bare_command_token(first_token)
        || !configured_standalone_command_token(runtime, first_token)
    {
        return None;
    }
    let mut end = first_end;
    let mut search_from = first_end;

    for raw_token in tokens {
        let token_start = tail[search_from..].find(raw_token)? + search_from;
        let token_end = token_start + raw_token.len();
        if structural_command_argument_token(raw_token) {
            end = token_end;
            search_from = token_end;
            continue;
        }
        return Some(tail[..end].trim());
    }

    None
}

fn path_command_segment_before_freeform_tail<'a>(tail: &'a str) -> Option<&'a str> {
    let path_env = std::env::var_os("PATH");
    path_command_segment_before_freeform_tail_with_path_env(tail, path_env.as_deref())
}

fn path_command_segment_before_freeform_tail_with_path_env<'a>(
    tail: &'a str,
    path_env: Option<&std::ffi::OsStr>,
) -> Option<&'a str> {
    let tail = trim_leading_command_separators_preserve_quotes(tail).trim();
    if tail.is_empty() || tail.contains('\n') {
        return None;
    }

    let mut tokens = tail.split_whitespace();
    let first = tokens.next()?;
    let first_start = tail.find(first)?;
    let first_end = first_start + first.len();
    let first_token =
        first.trim_matches(|ch: char| ch.is_ascii_punctuation() && !matches!(ch, '_' | '-' | '.'));
    if !simple_bare_command_token(first_token)
        || !command_token_resolves_in_path(first_token, path_env)
    {
        return None;
    }

    let mut end = first_end;
    let mut search_from = first_end;
    let mut saw_structural_arg = false;
    for raw_token in tokens {
        let token_start = tail[search_from..].find(raw_token)? + search_from;
        let token_end = token_start + raw_token.len();
        if structural_command_argument_token(raw_token) {
            saw_structural_arg = true;
            end = token_end;
            search_from = token_end;
            continue;
        }
        return saw_structural_arg.then(|| tail[..end].trim());
    }

    saw_structural_arg.then(|| tail[..end].trim())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExplicitCommandCandidate {
    command: String,
    single_step_safe: bool,
}

fn standalone_structural_command_from_segment(
    runtime: &crate::CommandIntentRuntime,
    segment: &str,
) -> Option<String> {
    let segment = trim_leading_command_separators_preserve_quotes(segment).trim();
    if segment.is_empty() || segment.contains('\n') || segment.contains('`') {
        return None;
    }
    let mut tokens = segment.split_whitespace();
    let first = tokens.next()?;
    let first_token =
        first.trim_matches(|ch: char| ch.is_ascii_punctuation() && !matches!(ch, '_' | '-' | '.'));
    if !simple_bare_command_token(first_token)
        || !configured_standalone_command_token(runtime, first_token)
    {
        return None;
    }
    if !tokens.all(structural_command_argument_token) {
        return None;
    }
    let command = crate::bootstrap::config_loaders::trim_command_text(segment.to_string());
    (!command.is_empty()).then_some(command)
}

fn followup_tail_has_structured_command_payload(
    runtime: &crate::CommandIntentRuntime,
    followup: &str,
) -> bool {
    let followup = followup.trim();
    !followup.is_empty()
        && (configured_explicit_command_candidate(runtime, followup).is_some()
            || shellish_literal_command_segment(followup).is_some()
            || leading_shellish_command_sequence_segment(followup).is_some())
}

fn standalone_command_candidate_from_tail(
    runtime: &crate::CommandIntentRuntime,
    tail: &str,
) -> Option<ExplicitCommandCandidate> {
    let tail = trim_leading_command_separators_preserve_quotes(tail).trim();
    if tail.is_empty() || tail.contains('\n') {
        return None;
    }

    if let Some(segment) = explicit_command_segment_before_followup(tail) {
        let command = configured_standalone_command_sequence_from_segment(runtime, segment)
            .or_else(|| standalone_structural_command_from_segment(runtime, segment))?;
        let followup = explicit_command_followup_tail(tail).unwrap_or("");
        return Some(ExplicitCommandCandidate {
            command,
            single_step_safe: !followup_tail_has_structured_command_payload(runtime, followup),
        });
    }

    if let Some(segment) = standalone_command_segment_before_freeform_tail(runtime, tail) {
        let command = standalone_structural_command_from_segment(runtime, segment)?;
        let followup = tail.get(segment.len()..).unwrap_or_default();
        return Some(ExplicitCommandCandidate {
            command,
            single_step_safe: !followup_tail_has_structured_command_payload(runtime, followup),
        });
    }

    let command = standalone_structural_command_from_segment(runtime, tail)?;
    Some(ExplicitCommandCandidate {
        command,
        single_step_safe: true,
    })
}

fn command_candidate_start_boundary(text: &str, idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let Some(prev) = text[..idx].chars().next_back() else {
        return true;
    };
    !prev.is_ascii_alphanumeric() && !matches!(prev, '_' | '-' | '.' | '/' | '\\' | '~' | '`')
}

fn embedded_standalone_command_candidate(
    runtime: &crate::CommandIntentRuntime,
    request: &str,
) -> Option<ExplicitCommandCandidate> {
    let request = request.trim();
    if request.is_empty() {
        return None;
    }
    request
        .char_indices()
        .filter(|(idx, ch)| {
            ch.is_ascii_alphabetic() && command_candidate_start_boundary(request, *idx)
        })
        .filter_map(|(idx, _)| standalone_command_candidate_from_tail(runtime, &request[idx..]))
        .next()
}

fn configured_explicit_command_candidate_from_text(
    runtime: &crate::CommandIntentRuntime,
    text: &str,
    allow_whole_tail: bool,
) -> Option<ExplicitCommandCandidate> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    runtime
        .execute_prefixes
        .iter()
        .filter_map(|prefix| strip_configured_command_prefix(text, prefix))
        .filter_map(|tail| {
            let segment = explicit_command_segment_before_followup(tail).or_else(|| {
                allow_whole_tail.then(|| {
                    markdown_code_span_command_segment(tail)
                        .or_else(|| whole_explicit_command_tail(tail))
                        .or_else(|| standalone_command_segment_before_freeform_tail(runtime, tail))
                        .or_else(|| path_command_segment_before_freeform_tail(tail))
                })?
            })?;
            let segment = markdown_code_span_command_segment(segment).unwrap_or(segment);
            let command = configured_standalone_command_sequence_from_segment(runtime, segment)
                .unwrap_or_else(|| {
                    crate::bootstrap::config_loaders::trim_command_text(segment.to_string())
                });
            let freeform_followup = tail.get(segment.len()..).unwrap_or_default();
            looks_like_concrete_command_tail(&command).then(|| ExplicitCommandCandidate {
                command,
                single_step_safe: explicit_command_followup_tail(tail).map_or_else(
                    || !followup_tail_has_structured_command_payload(runtime, freeform_followup),
                    |followup| followup.is_empty(),
                ),
            })
        })
        .next()
}

fn configured_explicit_command_candidate(
    runtime: &crate::CommandIntentRuntime,
    request: &str,
) -> Option<ExplicitCommandCandidate> {
    let request = request.trim();
    if request.is_empty() {
        return None;
    }
    configured_explicit_command_candidate_from_text(runtime, request, true).or_else(|| {
        request
            .split(|ch| matches!(ch, ',' | '，' | ';' | '；' | '。' | '\n'))
            .filter_map(|clause| {
                configured_explicit_command_candidate_from_text(runtime, clause, true)
            })
            .next()
    })
}

fn configured_explicit_command_segment(
    runtime: &crate::CommandIntentRuntime,
    request: &str,
) -> Option<String> {
    configured_explicit_command_candidate(runtime, request).map(|candidate| candidate.command)
}

fn contains_angle_placeholder_token(text: &str) -> bool {
    let mut chars = text.char_indices().peekable();
    while let Some((start_idx, ch)) = chars.next() {
        if ch != '<' {
            continue;
        }
        let Some((end_idx, _)) = chars.clone().find(|(_, candidate)| *candidate == '>') else {
            continue;
        };
        let inner = text[start_idx + ch.len_utf8()..end_idx].trim();
        if inner.is_empty() {
            continue;
        }
        let has_identifier_char = inner.chars().any(|candidate| candidate.is_alphanumeric());
        let placeholder_shaped = inner.chars().all(|candidate| {
            candidate.is_alphanumeric() || matches!(candidate, '_' | '-' | '.' | ' ' | '\t')
        });
        if has_identifier_char && placeholder_shaped {
            return true;
        }
    }
    false
}

fn literal_command_segment_has_unresolved_template(segment: &str) -> bool {
    contains_angle_placeholder_token(segment) || literal_segment_looks_like_output_template(segment)
}

fn literal_segment_looks_like_output_template(segment: &str) -> bool {
    let segment = segment.trim();
    if segment.is_empty()
        || segment.contains('\n')
        || segment
            .chars()
            .any(|ch| matches!(ch, '|' | ';' | '&' | '>' | '<'))
    {
        return false;
    }
    let mut words = segment.split_whitespace();
    let Some(first) = words.next() else {
        return false;
    };
    let Some(rest) = words.next() else {
        return false;
    };
    if words.next().is_some() || !first.ends_with(':') {
        return false;
    }
    let label = first.trim_end_matches(':');
    let label_ok = !label.is_empty()
        && label
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'));
    let placeholder_ok = rest
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '<' | '>'));
    label_ok && placeholder_ok
}

fn shellish_literal_command_segment(request: &str) -> Option<String> {
    let mut parts = request.split('`');
    parts.next();
    parts
        .step_by(2)
        .map(|segment| crate::bootstrap::config_loaders::trim_command_text(segment.to_string()))
        .find(|segment| {
            !literal_command_segment_has_unresolved_template(segment)
                && looks_like_concrete_command_tail(segment)
                && segment
                    .chars()
                    .any(|ch| matches!(ch, '|' | ';' | '&' | '>' | '<') || ch.is_whitespace())
        })
}

fn simple_bare_command_token(token: &str) -> bool {
    !token.is_empty()
        && !token.starts_with('-')
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        && token.chars().any(|ch| ch.is_ascii_alphabetic())
        && token
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .count()
            >= 2
}

fn command_token_resolves_in_path(token: &str, path_env: Option<&std::ffi::OsStr>) -> bool {
    let Some(path_env) = path_env else {
        return false;
    };
    std::env::split_paths(path_env).any(|dir| dir.join(token).is_file())
}

fn leading_shellish_command_sequence_segment_with_path_env(
    request: &str,
    path_env: Option<&std::ffi::OsStr>,
) -> Option<String> {
    let request = request.trim_start();
    if request.is_empty() {
        return None;
    }
    let ascii_end = request
        .char_indices()
        .find_map(|(idx, ch)| (!ch.is_ascii()).then_some(idx))
        .unwrap_or(request.len());
    let ascii_prefix = request[..ascii_end].trim();
    if ascii_prefix.is_empty() {
        return None;
    }
    let mut commands = Vec::new();
    for raw_token in ascii_prefix.split_whitespace() {
        let token = raw_token
            .trim_matches(|ch: char| ch.is_ascii_punctuation() && !matches!(ch, '_' | '-' | '.'));
        if !simple_bare_command_token(token) || !command_token_resolves_in_path(token, path_env) {
            break;
        }
        commands.push(token.to_string());
    }
    (commands.len() >= 3).then(|| commands.join("; "))
}

fn leading_shellish_command_sequence_segment(request: &str) -> Option<String> {
    let path_env = std::env::var_os("PATH");
    leading_shellish_command_sequence_segment_with_path_env(request, path_env.as_deref())
}

pub(super) fn explicit_command_segment(
    runtime: &crate::CommandIntentRuntime,
    request: &str,
) -> Option<String> {
    configured_explicit_command_segment(runtime, request)
        .or_else(|| {
            embedded_standalone_command_candidate(runtime, request)
                .map(|candidate| candidate.command)
        })
        .or_else(|| shellish_literal_command_segment(request))
        .or_else(|| leading_shellish_command_sequence_segment(request))
}

fn explicit_command_single_step_segment(
    runtime: &crate::CommandIntentRuntime,
    request: &str,
) -> Option<String> {
    if let Some(candidate) = configured_explicit_command_candidate(runtime, request) {
        return candidate.single_step_safe.then_some(candidate.command);
    }
    if let Some(candidate) = embedded_standalone_command_candidate(runtime, request) {
        return candidate.single_step_safe.then_some(candidate.command);
    }
    shellish_literal_command_segment(request)
        .or_else(|| leading_shellish_command_sequence_segment(request))
}

fn route_allows_explicit_command_preservation(route_result: Option<&RouteResult>) -> bool {
    route_result.is_some_and(|route| {
        route.is_execute_gate()
            && (route.output_contract.requires_content_evidence
                || route.output_contract.semantic_kind
                    == crate::OutputSemanticKind::RawCommandOutput)
    })
}

fn run_cmd_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("run_cmd")
}

fn process_basic_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("process_basic")
}

fn system_basic_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("system_basic")
}

fn health_check_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("health_check")
}

fn action_is_run_cmd(state: &AppState, action: &AgentAction) -> bool {
    planned_action_skill_name(action)
        .map(|skill| state.resolve_canonical_skill_name(skill) == "run_cmd")
        .unwrap_or(false)
}

fn literal_command_failure_can_replan(route_result: Option<&RouteResult>) -> bool {
    route_result.is_some_and(|route| {
        route.is_execute_gate()
            && !matches!(
                route.output_contract.semantic_kind,
                crate::OutputSemanticKind::RawCommandOutput
                    | crate::OutputSemanticKind::ExecutionFailedStep
            )
    })
}

fn structural_contract_deterministic_plan_overrides_literal_command_guard(
    route_result: Option<&RouteResult>,
) -> bool {
    route_result.is_some_and(|route| {
        route.is_execute_gate()
            && route.output_contract.requires_content_evidence
            && !route.output_contract.delivery_required
            && matches!(
                route.output_contract.semantic_kind,
                crate::OutputSemanticKind::StructuredKeys
                    | crate::OutputSemanticKind::DirectoryPurposeSummary
                    | crate::OutputSemanticKind::DirectoryEntryGroups
                    | crate::OutputSemanticKind::FileNames
                    | crate::OutputSemanticKind::DirectoryNames
                    | crate::OutputSemanticKind::FilePaths
                    | crate::OutputSemanticKind::ContentExcerptSummary
                    | crate::OutputSemanticKind::ContentExcerptWithSummary
                    | crate::OutputSemanticKind::ExistenceWithPath
                    | crate::OutputSemanticKind::ExistenceWithPathSummary
            )
    })
}

fn missing_target_failure_can_replan(route_result: Option<&RouteResult>) -> bool {
    route_result.is_some_and(|route| {
        route.is_execute_gate()
            && route.output_contract.requires_content_evidence
            && matches!(
                route.output_contract.semantic_kind,
                crate::OutputSemanticKind::FilePaths
                    | crate::OutputSemanticKind::FileNames
                    | crate::OutputSemanticKind::DirectoryNames
                    | crate::OutputSemanticKind::DirectoryPurposeSummary
                    | crate::OutputSemanticKind::ContentExcerptSummary
                    | crate::OutputSemanticKind::ContentExcerptWithSummary
                    | crate::OutputSemanticKind::ExistenceWithPathSummary
            )
    })
}

fn mark_missing_target_repairable_actions(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !missing_target_failure_can_replan(route_result) {
        return actions;
    }
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, mut args } => {
                let canonical = state.resolve_canonical_skill_name(&skill);
                if matches!(
                    canonical.as_str(),
                    "read_file" | "list_dir" | "system_basic"
                ) {
                    if let Some(obj) = args.as_object_mut() {
                        obj.insert(
                            super::CLAWD_MISSING_TARGET_REPAIRABLE_ARG.to_string(),
                            Value::Bool(true),
                        );
                    }
                }
                AgentAction::CallSkill { skill, args }
            }
            AgentAction::CallTool { tool, mut args } => {
                let canonical = state.resolve_canonical_skill_name(&tool);
                if matches!(
                    canonical.as_str(),
                    "read_file" | "list_dir" | "system_basic"
                ) {
                    if let Some(obj) = args.as_object_mut() {
                        obj.insert(
                            super::CLAWD_MISSING_TARGET_REPAIRABLE_ARG.to_string(),
                            Value::Bool(true),
                        );
                    }
                }
                AgentAction::CallTool { tool, args }
            }
            other => other,
        })
        .collect()
}

fn mark_explicit_literal_run_cmd_actions(
    actions: Vec<AgentAction>,
    failure_repairable: bool,
) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, mut args } => {
                if skill.trim().eq_ignore_ascii_case("run_cmd") {
                    if let Some(obj) = args.as_object_mut() {
                        obj.insert(
                            super::CLAWD_LITERAL_COMMAND_ARG.to_string(),
                            Value::Bool(true),
                        );
                        if failure_repairable {
                            obj.insert(
                                super::CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG.to_string(),
                                Value::Bool(true),
                            );
                        }
                    }
                }
                AgentAction::CallSkill { skill, args }
            }
            AgentAction::CallTool { tool, mut args } => {
                if tool.trim().eq_ignore_ascii_case("run_cmd") {
                    if let Some(obj) = args.as_object_mut() {
                        obj.insert(
                            super::CLAWD_LITERAL_COMMAND_ARG.to_string(),
                            Value::Bool(true),
                        );
                        if failure_repairable {
                            obj.insert(
                                super::CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG.to_string(),
                                Value::Bool(true),
                            );
                        }
                    }
                }
                AgentAction::CallTool { tool, args }
            }
            other => other,
        })
        .collect()
}

fn replace_explicit_command_substitute_plan_with_run_cmd(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output
        || !route_allows_explicit_command_preservation(route_result)
        || !run_cmd_available_for_plan(state)
    {
        return actions;
    }
    let Some(original_user_text) = original_user_text
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return actions;
    };
    if explicit_command_segment(&state.policy.command_intent, original_user_text).is_none() {
        return actions;
    }
    if actions
        .iter()
        .any(|action| action_is_run_cmd(state, action))
    {
        return mark_explicit_literal_run_cmd_actions(
            actions,
            literal_command_failure_can_replan(route_result),
        );
    }
    let Some(first_observation_idx) = actions.iter().position(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
        )
    }) else {
        return actions;
    };
    let mut rewritten = actions;
    let exact_command = explicit_command_segment(&state.policy.command_intent, original_user_text);
    let mut args = serde_json::json!({
        "request_text": original_user_text,
        "cwd": state.skill_rt.workspace_root.display().to_string(),
    });
    if let Some(command) = exact_command {
        args["command"] = serde_json::Value::String(command);
    }
    args[super::CLAWD_LITERAL_COMMAND_ARG] = Value::Bool(true);
    if literal_command_failure_can_replan(route_result) {
        args[super::CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG] = Value::Bool(true);
    }
    rewritten[first_observation_idx] = AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args,
    };
    info!("plan_rewrite_explicit_command_substitute_to_run_cmd");
    rewritten
}

#[cfg(test)]
fn normalize_planned_actions(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    normalize_planned_actions_with_original(
        state,
        route_result,
        loop_state,
        user_text,
        None,
        auto_locator_path,
        actions,
    )
}

#[cfg(test)]
fn normalize_planned_actions_with_original(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    normalize_planned_actions_with_original_and_context(
        state,
        route_result,
        loop_state,
        user_text,
        original_user_text,
        None,
        auto_locator_path,
        actions,
    )
}

fn normalize_planned_actions_with_original_and_context(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    plan_context: Option<&str>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let actions = crate::capability_resolver::resolve_agent_actions_for_state(state, actions);
    let terminal_mixed_last_output_content = terminal_mixed_last_output_respond_content(&actions);
    let actions = replace_scalar_path_respond_only_with_auto_locator_observation(
        route_result,
        loop_state,
        auto_locator_path,
        actions,
    );
    let actions = replace_file_delivery_respond_only_with_path_observation(
        state,
        route_result,
        loop_state,
        actions,
    );
    let actions = replace_explicit_command_substitute_plan_with_run_cmd(
        state,
        route_result,
        loop_state,
        original_user_text,
        actions,
    );
    let explicit_command_request = route_allows_explicit_command_preservation(route_result)
        && original_user_text.or(Some(user_text)).is_some_and(|text| {
            explicit_command_segment(&state.policy.command_intent, text).is_some()
        });
    let defer_legacy_semantic_rewrites = !explicit_command_request
        && route_result.is_some_and(|route| {
            actions_use_ad_hoc_command_without_route_preferred_skill(state, route, &actions)
        });
    if defer_legacy_semantic_rewrites {
        info!("plan_defer_legacy_semantic_rewrite_to_registry_repair");
    }
    let skip_legacy_semantic_rewrites = explicit_command_request || defer_legacy_semantic_rewrites;
    let actions = normalize_legacy_compatibility_actions(
        state,
        route_result,
        loop_state,
        user_text,
        original_user_text,
        plan_context,
        auto_locator_path,
        actions,
        skip_legacy_semantic_rewrites,
    );
    let actions =
        rewrite_process_ps_run_cmd_to_process_basic(state, user_text, original_user_text, actions);
    let actions = rewrite_append_run_cmd_to_fs_basic(state, user_text, original_user_text, actions);
    let actions = rewrite_readonly_file_read_run_cmd_to_fs_basic(
        state,
        user_text,
        original_user_text,
        actions,
    );
    let actions = rewrite_readonly_find_run_cmd_to_fs_basic(
        state,
        route_result,
        user_text,
        original_user_text,
        actions,
    );
    let actions =
        strip_terminal_discussion_for_direct_skill_passthrough(state, route_result, actions);
    let actions = normalize_evidence_contract_actions(
        state,
        route_result,
        loop_state,
        user_text,
        plan_context,
        auto_locator_path,
        actions,
    );
    let actions =
        strip_unrequested_config_edit_actions(route_result, user_text, original_user_text, actions);
    let actions = normalize_terminal_delivery_actions(
        state,
        route_result,
        loop_state,
        user_text,
        terminal_mixed_last_output_content,
        actions,
    );
    let actions = canonicalize_legacy_file_config_capabilities(actions);
    let actions = rewrite_single_target_structured_field_read_to_auto_locator(
        route_result,
        auto_locator_path,
        actions,
    );
    mark_non_mutating_run_cmd_sequences_continue_on_error(state, actions)
}

fn canonicalize_legacy_file_config_capabilities(actions: Vec<AgentAction>) -> Vec<AgentAction> {
    actions
        .into_iter()
        .enumerate()
        .map(|(idx, action)| match action {
            AgentAction::CallTool { tool, args } => {
                let Some(canonical) =
                    crate::virtual_tools::canonicalize_legacy_tool_call(&tool, args.clone())
                else {
                    return AgentAction::CallTool { tool, args };
                };
                info!(
                    "plan_canonicalize_legacy_tool idx={} from={} to={} args={}",
                    idx,
                    tool,
                    canonical.tool,
                    crate::truncate_for_log(&canonical.args.to_string())
                );
                AgentAction::CallTool {
                    tool: canonical.tool,
                    args: canonical.args,
                }
            }
            AgentAction::CallSkill { skill, args } => {
                let Some(canonical) =
                    crate::virtual_tools::canonicalize_legacy_tool_call(&skill, args.clone())
                else {
                    return AgentAction::CallSkill { skill, args };
                };
                info!(
                    "plan_canonicalize_legacy_tool idx={} from={} to={} args={}",
                    idx,
                    skill,
                    canonical.tool,
                    crate::truncate_for_log(&canonical.args.to_string())
                );
                AgentAction::CallTool {
                    tool: canonical.tool,
                    args: canonical.args,
                }
            }
            other => other,
        })
        .collect()
}

fn normalize_legacy_compatibility_actions(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    plan_context: Option<&str>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
    skip_legacy_semantic_rewrites: bool,
) -> Vec<AgentAction> {
    let actions = rewrite_service_status_plan_to_service_control(
        route_result,
        skip_legacy_semantic_rewrites,
        actions,
    );
    let actions = split_sequential_run_cmd_actions(user_text, original_user_text, actions);
    let actions = replace_hidden_entries_count_plan_with_inventory_dir(
        route_result,
        loop_state,
        auto_locator_path,
        actions,
    );
    let actions = replace_scalar_count_plan_with_count_inventory(
        route_result,
        loop_state,
        auto_locator_path,
        user_text,
        actions,
    );
    let actions =
        replace_structured_keys_read_plan(route_result, loop_state, auto_locator_path, actions);
    let actions = ensure_existence_path_summary_has_bounded_content(
        route_result,
        loop_state,
        auto_locator_path,
        actions,
    );
    let actions = ensure_content_excerpt_summary_has_bounded_content(
        route_result,
        loop_state,
        auto_locator_path,
        actions,
    );
    let actions =
        prefer_log_analyze_for_single_log_synthesis(state, route_result, loop_state, actions);
    let actions =
        prefer_doc_parse_for_single_document_synthesis(state, route_result, loop_state, actions);
    let actions =
        strip_terminal_discussion_for_observed_finalize(route_result, loop_state, actions);
    let actions =
        strip_terminal_discussion_for_scalar_path_observation(route_result, loop_state, actions);
    let actions =
        strip_terminal_discussion_for_direct_skill_passthrough(state, route_result, actions);
    let actions = normalize_action_schema_aliases(state, route_result, actions);
    let actions = repair_guard_config_default_path_for_invalid_locator(
        route_result,
        auto_locator_path,
        actions,
    );
    let actions = rewrite_directory_entry_groups_tree_summary_to_list_dir(
        route_result,
        auto_locator_path,
        actions,
    );
    let actions = rewrite_archive_basic_short_archive_to_active_bound_target(plan_context, actions);
    let actions = rewrite_invalid_rustclaw_config_section_field_reads_to_guard(
        route_result,
        auto_locator_path,
        actions,
    );
    let actions =
        rewrite_rustclaw_config_risk_assessment_to_guard(route_result, auto_locator_path, actions);
    let actions = rewrite_rustclaw_main_config_excerpt_read_to_guard(
        route_result,
        auto_locator_path,
        actions,
    );
    let actions =
        rewrite_rustclaw_config_validation_to_guard(route_result, auto_locator_path, actions);
    let actions = rewrite_sqlite_table_listing_plan_to_db_basic(
        route_result,
        auto_locator_path,
        skip_legacy_semantic_rewrites,
        actions,
    );
    let actions = rewrite_sqlite_schema_version_plan_to_db_basic(
        route_result,
        auto_locator_path,
        skip_legacy_semantic_rewrites,
        actions,
    );
    let actions = rewrite_sqlite_table_probe_to_requested_schema_value(
        route_result,
        user_text,
        original_user_text,
        actions,
    );
    let actions = rewrite_sqlite_count_query_to_requested_schema_column(
        route_result,
        user_text,
        original_user_text,
        actions,
    );
    let actions = rewrite_docker_readonly_run_cmd_to_docker_basic(
        state,
        skip_legacy_semantic_rewrites,
        actions,
    );
    let actions = rewrite_archive_unpack_run_cmd_to_archive_basic(
        route_result,
        skip_legacy_semantic_rewrites,
        actions,
    );
    let actions = rewrite_archive_pack_plan_to_archive_basic(
        route_result,
        skip_legacy_semantic_rewrites,
        actions,
    );
    let actions = rewrite_single_target_structured_field_read_to_auto_locator(
        route_result,
        auto_locator_path,
        actions,
    );
    let actions =
        rewrite_single_target_file_read_to_auto_locator(route_result, auto_locator_path, actions);
    actions
}

fn rewrite_directory_entry_groups_tree_summary_to_list_dir(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if route_result.is_none_or(|route| {
        route.output_contract.semantic_kind != crate::OutputSemanticKind::DirectoryEntryGroups
    }) {
        return actions;
    }
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args }
            | AgentAction::CallTool { tool: skill, args }
                if skill.eq_ignore_ascii_case("system_basic")
                    && args
                        .get("action")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .is_some_and(|action| action.eq_ignore_ascii_case("tree_summary")) =>
            {
                let path = args
                    .get("path")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
                    .or_else(|| {
                        auto_locator_path
                            .map(str::trim)
                            .filter(|path| !path.is_empty())
                    })
                    .or_else(|| {
                        route_result
                            .map(|route| route.output_contract.locator_hint.trim())
                            .filter(|path| !path.is_empty())
                    });
                let mut mapped = serde_json::Map::new();
                mapped.insert("action".to_string(), Value::String("list_dir".to_string()));
                if let Some(path) = path {
                    mapped.insert("path".to_string(), Value::String(path.to_string()));
                }
                mapped.insert("names_only".to_string(), Value::Bool(false));
                mapped.insert("max_entries".to_string(), Value::Number(1000.into()));
                mapped.insert("sort_by".to_string(), Value::String("name".to_string()));
                AgentAction::CallTool {
                    tool: "fs_basic".to_string(),
                    args: Value::Object(mapped),
                }
            }
            other => other,
        })
        .collect()
}

fn rewrite_rustclaw_config_validation_to_guard(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if actions.iter().any(is_config_guard_action) {
        return actions;
    }
    actions
        .into_iter()
        .map(|action| {
            let Some((path, format, profile)) =
                config_validation_action_target(&action, route_result, auto_locator_path)
                    .map(|(path, format, profile)| (path, format, Some(profile)))
                    .or_else(|| {
                        plain_rustclaw_main_config_validation_action_target(
                            &action,
                            route_result,
                            auto_locator_path,
                        )
                        .map(|(path, format)| (path, format, None))
                    })
            else {
                return action;
            };
            if profile == Some(ConfigValidationProfile::SyntaxOnly) {
                return action;
            }
            info!(
                "plan_rewrite_rustclaw_config_validation_to_guard path={}",
                crate::truncate_for_log(&path)
            );
            let mut args = serde_json::Map::new();
            args.insert(
                "action".to_string(),
                Value::String("guard_config".to_string()),
            );
            args.insert("path".to_string(), Value::String(path));
            if let Some(format) = format {
                args.insert("format".to_string(), Value::String(format));
            }
            AgentAction::CallTool {
                tool: "config_edit".to_string(),
                args: Value::Object(args),
            }
        })
        .collect()
}

fn repair_guard_config_default_path_for_invalid_locator(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallTool { tool, args } => {
                let args = repair_guard_config_args_for_invalid_locator(
                    &tool,
                    args,
                    route_result,
                    auto_locator_path,
                );
                AgentAction::CallTool { tool, args }
            }
            AgentAction::CallSkill { skill, args } => {
                let args = repair_guard_config_args_for_invalid_locator(
                    &skill,
                    args,
                    route_result,
                    auto_locator_path,
                );
                AgentAction::CallSkill { skill, args }
            }
            other => other,
        })
        .collect()
}

fn repair_guard_config_args_for_invalid_locator(
    skill: &str,
    args: Value,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Value {
    let Some(action_name) = args.get("action").and_then(Value::as_str).map(str::trim) else {
        return args;
    };
    let is_guard_action = (skill.eq_ignore_ascii_case("config_edit")
        && action_name.eq_ignore_ascii_case("guard_config"))
        || (skill.eq_ignore_ascii_case("config_basic")
            && action_name.eq_ignore_ascii_case("guard_rustclaw_config"))
        || skill.eq_ignore_ascii_case("config_guard");
    if !is_guard_action {
        return args;
    }
    let should_repair = args
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .is_none_or(|path| {
            !is_rustclaw_config_guard_path(path) && !path_has_structured_text_extension(path)
        });
    if !should_repair {
        return args;
    }
    let path = route_result
        .map(|route| route.output_contract.locator_hint.trim())
        .filter(|path| is_rustclaw_config_guard_path(path))
        .or_else(|| {
            auto_locator_path
                .map(str::trim)
                .filter(|path| is_rustclaw_config_guard_path(path))
        })
        .unwrap_or("configs/config.toml");
    let mut obj = args.as_object().cloned().unwrap_or_default();
    obj.insert("path".to_string(), Value::String(path.to_string()));
    obj.entry("format".to_string())
        .or_insert_with(|| Value::String("toml".to_string()));
    Value::Object(obj)
}

fn rewrite_rustclaw_config_risk_assessment_to_guard(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if route_result.is_none_or(|route| {
        route.output_contract.semantic_kind != crate::OutputSemanticKind::ConfigRiskAssessment
    }) || actions.iter().any(is_config_guard_action)
    {
        return actions;
    }
    let Some(path) =
        rustclaw_config_risk_assessment_target(route_result, auto_locator_path, &actions)
    else {
        return actions;
    };
    info!(
        "plan_rewrite_rustclaw_config_risk_assessment_to_guard path={}",
        crate::truncate_for_log(&path)
    );
    let mut args = serde_json::Map::new();
    args.insert(
        "action".to_string(),
        Value::String("guard_config".to_string()),
    );
    args.insert("path".to_string(), Value::String(path));
    args.insert("format".to_string(), Value::String("toml".to_string()));
    vec![AgentAction::CallTool {
        tool: "config_edit".to_string(),
        args: Value::Object(args),
    }]
}

fn rustclaw_config_risk_assessment_target(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: &[AgentAction],
) -> Option<String> {
    actions
        .iter()
        .filter_map(planned_config_risk_observation_path)
        .find(|path| is_rustclaw_config_guard_path(path))
        .or_else(|| {
            auto_locator_path
                .map(str::trim)
                .filter(|path| is_rustclaw_config_guard_path(path))
        })
        .or_else(|| {
            route_result
                .map(|route| route.output_contract.locator_hint.trim())
                .filter(|path| is_rustclaw_config_guard_path(path))
        })
        .map(ToString::to_string)
}

fn planned_config_risk_observation_path(action: &AgentAction) -> Option<&str> {
    planned_structured_config_observation_path(action)
        .or_else(|| planned_bounded_file_read_path(action))
}

fn rewrite_rustclaw_main_config_excerpt_read_to_guard(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if actions.iter().any(is_config_guard_action) {
        return actions;
    }
    let Some(path) =
        rustclaw_main_config_excerpt_guard_target(route_result, auto_locator_path, &actions)
    else {
        return actions;
    };
    info!(
        "plan_rewrite_rustclaw_main_config_excerpt_read_to_guard path={}",
        crate::truncate_for_log(&path)
    );
    let mut args = serde_json::Map::new();
    args.insert(
        "action".to_string(),
        Value::String("guard_config".to_string()),
    );
    args.insert("path".to_string(), Value::String(path));
    args.insert("format".to_string(), Value::String("toml".to_string()));
    vec![AgentAction::CallTool {
        tool: "config_edit".to_string(),
        args: Value::Object(args),
    }]
}

fn rustclaw_main_config_excerpt_guard_target(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: &[AgentAction],
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::ContentExcerptSummary
        || !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
        )
    {
        return None;
    }
    actions
        .iter()
        .filter_map(planned_broad_rustclaw_main_config_read_path)
        .find(|path| is_rustclaw_main_config_path(path))
        .or_else(|| {
            let has_broad_main_config_read = actions
                .iter()
                .any(|action| planned_broad_config_read_without_path(action));
            has_broad_main_config_read.then(|| {
                auto_locator_path
                    .map(str::trim)
                    .filter(|path| is_rustclaw_main_config_path(path))
            })?
        })
        .or_else(|| {
            let has_broad_main_config_read = actions
                .iter()
                .any(|action| planned_broad_config_read_without_path(action));
            has_broad_main_config_read.then(|| {
                let hint = route.output_contract.locator_hint.trim();
                is_rustclaw_main_config_path(hint).then_some(hint)
            })?
        })
        .map(ToString::to_string)
}

fn planned_broad_rustclaw_main_config_read_path(action: &AgentAction) -> Option<&str> {
    planned_bounded_file_read_path(action)
        .filter(|path| is_rustclaw_main_config_path(path))
        .filter(|_| planned_broad_config_excerpt_read(action))
}

fn planned_broad_config_read_without_path(action: &AgentAction) -> bool {
    planned_bounded_file_read_path(action).is_none() && planned_broad_config_excerpt_read(action)
}

fn planned_broad_config_excerpt_read(action: &AgentAction) -> bool {
    let args = match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => args,
        AgentAction::CallCapability { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Think { .. } => return false,
    };
    if !action_observes_bounded_file_content(action) {
        return false;
    }
    let mode = args
        .get("mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if mode.eq_ignore_ascii_case("tail") || mode.eq_ignore_ascii_case("last") {
        return false;
    }
    let n = args
        .get("n")
        .or_else(|| args.get("line_count"))
        .or_else(|| args.get("count"))
        .or_else(|| args.get("limit"))
        .and_then(parse_positive_usize);
    if n.is_some_and(|value| value < 80) {
        return false;
    }
    let start_line = args
        .get("start_line")
        .or_else(|| args.get("line_start"))
        .and_then(parse_i64_value);
    if start_line.is_some_and(|line| line > 1) {
        return false;
    }
    let end_line = args
        .get("end_line")
        .or_else(|| args.get("line_end"))
        .and_then(parse_i64_value);
    if let (Some(start), Some(end)) = (start_line, end_line) {
        if end >= start && (end - start + 1) < 80 {
            return false;
        }
    } else if let Some(end) = end_line {
        if end < 80 {
            return false;
        }
    }
    let max_bytes = args.get("max_bytes").and_then(parse_positive_usize);
    if max_bytes.is_some_and(|value| value < 4096) {
        return false;
    }
    true
}

fn rewrite_invalid_rustclaw_config_section_field_reads_to_guard(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if actions.iter().any(is_config_guard_action) {
        return actions;
    }
    let Some((path, format)) = invalid_rustclaw_config_section_field_read_target(
        route_result,
        auto_locator_path,
        &actions,
    ) else {
        return actions;
    };
    info!(
        "plan_rewrite_invalid_rustclaw_config_section_field_read_to_guard path={}",
        crate::truncate_for_log(&path)
    );
    let mut args = serde_json::Map::new();
    args.insert(
        "action".to_string(),
        Value::String("guard_config".to_string()),
    );
    args.insert("path".to_string(), Value::String(path));
    if let Some(format) = format {
        args.insert("format".to_string(), Value::String(format));
    }
    vec![AgentAction::CallTool {
        tool: "config_edit".to_string(),
        args: Value::Object(args),
    }]
}

fn invalid_rustclaw_config_section_field_read_target(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: &[AgentAction],
) -> Option<(String, Option<String>)> {
    actions.iter().find_map(|action| {
        let (skill, args) = match action {
            AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
            AgentAction::CallTool { tool, args } => (tool.as_str(), args),
            _ => return None,
        };
        let action_name = args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        let is_field_read = (skill.eq_ignore_ascii_case("config_basic")
            && matches!(action_name, "read_field" | "read_fields"))
            || (skill.eq_ignore_ascii_case("system_basic")
                && matches!(action_name, "extract_field" | "extract_fields"));
        if !is_field_read || !config_field_args_are_section_headers(args) {
            return None;
        }
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .or_else(|| {
                auto_locator_path
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
            })
            .or_else(|| {
                route_result
                    .map(|route| route.output_contract.locator_hint.trim())
                    .filter(|path| !path.is_empty())
            })?;
        if !is_rustclaw_main_config_path(path) {
            return None;
        }
        let format = args
            .get("format")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .or_else(|| Some("toml".to_string()));
        Some((path.to_string(), format))
    })
}

fn config_field_args_are_section_headers(args: &Value) -> bool {
    let fields = args
        .get("field_paths")
        .or_else(|| args.get("fields"))
        .map(config_field_selector_list)
        .filter(|fields| !fields.is_empty())
        .or_else(|| {
            args.get("field_path")
                .or_else(|| args.get("field"))
                .or_else(|| args.get("key"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|field| !field.is_empty())
                .map(|field| vec![field.to_string()])
        })
        .unwrap_or_default();
    fields.len() >= 2
        && fields
            .iter()
            .all(|field| config_field_is_section_header(field))
}

fn config_field_selector_list(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|field| !field.is_empty())
            .map(ToString::to_string)
            .collect(),
        Value::String(text) => text
            .split(',')
            .map(str::trim)
            .filter(|field| !field.is_empty())
            .map(ToString::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn config_field_is_section_header(field: &str) -> bool {
    let field = field.trim();
    field.len() > 2
        && field.starts_with('[')
        && field.ends_with(']')
        && !field[1..field.len() - 1].trim().is_empty()
}

fn is_config_guard_action(action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        _ => return false,
    };
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    (skill.eq_ignore_ascii_case("config_edit") && action_name.eq_ignore_ascii_case("guard_config"))
        || (skill.eq_ignore_ascii_case("config_basic")
            && action_name.eq_ignore_ascii_case("guard_rustclaw_config"))
        || skill.eq_ignore_ascii_case("config_guard")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigValidationProfile {
    SyntaxOnly,
    RustClawSemanticGuard,
}

fn parse_config_validation_profile(value: &str) -> Option<ConfigValidationProfile> {
    match value.trim().to_ascii_lowercase().as_str() {
        "syntax_only" => Some(ConfigValidationProfile::SyntaxOnly),
        "rustclaw_semantic_guard" => Some(ConfigValidationProfile::RustClawSemanticGuard),
        _ => None,
    }
}

fn config_validation_profile_from_args(args: &Value) -> Option<ConfigValidationProfile> {
    args.get("validation_profile")
        .and_then(Value::as_str)
        .and_then(parse_config_validation_profile)
        .or_else(|| {
            args.get("_clawd_validation")
                .and_then(Value::as_object)
                .and_then(|obj| obj.get("validation_profile"))
                .and_then(Value::as_str)
                .and_then(parse_config_validation_profile)
        })
}

fn config_validation_action_target(
    action: &AgentAction,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<(String, Option<String>, ConfigValidationProfile)> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        _ => return None,
    };
    let profile = config_validation_profile_from_args(args)?;
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let is_validation = (skill.eq_ignore_ascii_case("config_basic")
        && action_name.eq_ignore_ascii_case("validate"))
        || (skill.eq_ignore_ascii_case("system_basic")
            && action_name.eq_ignore_ascii_case("validate_structured"))
        || (skill.eq_ignore_ascii_case("config_edit")
            && action_name.eq_ignore_ascii_case("validate_config"));
    if !is_validation {
        return None;
    }
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or_else(|| {
            auto_locator_path
                .map(str::trim)
                .filter(|path| !path.is_empty())
        })
        .or_else(|| {
            route_result
                .map(|route| route.output_contract.locator_hint.trim())
                .filter(|path| !path.is_empty())
        })?;
    if !is_rustclaw_main_config_path(path) {
        return None;
    }
    let format = args
        .get("format")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| Some("toml".to_string()));
    Some((path.to_string(), format, profile))
}

fn plain_rustclaw_main_config_validation_action_target(
    action: &AgentAction,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<(String, Option<String>)> {
    if route_result.is_none_or(|route| !route.output_contract.requires_content_evidence) {
        return None;
    }
    if route_result.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::ConfigValidation
    }) {
        return None;
    }
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        _ => return None,
    };
    if config_validation_profile_from_args(args).is_some() {
        return None;
    }
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let is_validation = (skill.eq_ignore_ascii_case("config_basic")
        && action_name.eq_ignore_ascii_case("validate"))
        || (skill.eq_ignore_ascii_case("system_basic")
            && action_name.eq_ignore_ascii_case("validate_structured"))
        || (skill.eq_ignore_ascii_case("config_edit")
            && action_name.eq_ignore_ascii_case("validate_config"));
    if !is_validation {
        return None;
    }
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or_else(|| {
            auto_locator_path
                .map(str::trim)
                .filter(|path| !path.is_empty())
        })
        .or_else(|| {
            route_result
                .map(|route| route.output_contract.locator_hint.trim())
                .filter(|path| !path.is_empty())
        })?;
    if !is_rustclaw_main_config_path(path) {
        return None;
    }
    let format = args
        .get("format")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| Some("toml".to_string()));
    Some((path.to_string(), format))
}

fn is_rustclaw_main_config_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/").trim().to_ascii_lowercase();
    normalized == "configs/config.toml"
        || normalized.ends_with("/configs/config.toml")
        || normalized == "config.toml"
}

fn is_rustclaw_config_guard_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/").trim().to_ascii_lowercase();
    if is_rustclaw_main_config_path(&normalized) {
        return true;
    }
    let relative_configs_path = normalized.starts_with("configs/") && normalized.ends_with(".toml");
    let absolute_configs_path = normalized.contains("/configs/") && normalized.ends_with(".toml");
    relative_configs_path || absolute_configs_path
}

fn normalize_action_schema_aliases(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let actions = normalize_doc_parse_schema_aliases(actions);
    let actions = normalize_transform_schema_aliases(actions);
    let actions = normalize_fs_basic_schema_aliases(actions);
    let actions = normalize_system_basic_schema_aliases(actions);
    let actions = normalize_git_basic_schema_aliases(route_result, actions);
    let actions = fill_missing_read_range_path_from_route_locator(route_result, actions);
    let actions = rewrite_filtered_list_dir_to_inventory_dir(state, route_result, actions);
    let actions = inject_structural_extension_filter_for_directory_inventory(route_result, actions);
    let actions = normalize_archive_basic_schema_aliases(route_result, actions);
    let actions = strip_file_lines_count_before_tail_read_range(actions);
    let actions = strip_directory_read_range_after_inventory_dir(actions);
    let actions = broaden_default_read_range_for_structured_text(actions);
    let actions = rewrite_config_validation_read_plan_to_validate(route_result, None, actions);
    let actions =
        rewrite_invalid_rustclaw_config_section_field_reads_to_guard(route_result, None, actions);
    let actions = rewrite_rustclaw_config_risk_assessment_to_guard(route_result, None, actions);
    let actions = rewrite_structured_multi_field_read_plan_to_read_fields(
        route_result,
        route_result
            .map(|route| route.resolved_intent.as_str())
            .unwrap_or_default(),
        None,
        None,
        actions,
    );
    let actions = rewrite_structured_scalar_field_read_plan_to_read_field(
        state,
        route_result,
        route_result
            .map(|route| route.resolved_intent.as_str())
            .unwrap_or_default(),
        None,
        None,
        actions,
    );
    enforce_output_contract_tool_args(route_result, actions)
}

fn broaden_default_read_range_for_structured_text(actions: Vec<AgentAction>) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|mut action| {
            match &mut action {
                AgentAction::CallSkill { skill, args }
                | AgentAction::CallTool { tool: skill, args }
                    if skill.eq_ignore_ascii_case("fs_basic")
                        || skill.eq_ignore_ascii_case("system_basic") =>
                {
                    let Some(obj) = args.as_object_mut() else {
                        return action;
                    };
                    if !is_read_range_action(skill, obj) {
                        return action;
                    }
                    if read_range_has_explicit_bounds(obj) {
                        return action;
                    }
                    let Some(path) = obj.get("path").and_then(Value::as_str).map(str::to_string)
                    else {
                        return action;
                    };
                    if !path_has_structured_text_extension(&path) {
                        return action;
                    }
                    obj.entry("mode".to_string())
                        .or_insert_with(|| Value::String("head".to_string()));
                    obj.entry("n".to_string())
                        .or_insert(Value::Number(500.into()));
                    info!(
                        "plan_broaden_structured_text_read_range path={}",
                        crate::truncate_for_log(&path)
                    );
                }
                _ => {}
            }
            action
        })
        .collect()
}

fn is_read_range_action(skill: &str, obj: &serde_json::Map<String, Value>) -> bool {
    let action = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    (skill.eq_ignore_ascii_case("fs_basic") && action.eq_ignore_ascii_case("read_text_range"))
        || (skill.eq_ignore_ascii_case("system_basic") && action.eq_ignore_ascii_case("read_range"))
}

fn read_range_has_explicit_bounds(obj: &serde_json::Map<String, Value>) -> bool {
    if obj.get("n").is_some()
        || obj.get("start_line").is_some()
        || obj.get("end_line").is_some()
        || obj.get("line_start").is_some()
        || obj.get("line_end").is_some()
    {
        return true;
    }
    obj.get("mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|mode| !mode.is_empty())
        .is_some_and(|mode| {
            !mode.eq_ignore_ascii_case("head")
                && !mode.eq_ignore_ascii_case("full")
                && !mode.eq_ignore_ascii_case("all")
        })
}

fn path_has_structured_text_extension(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|ext| matches!(ext.as_str(), "json" | "toml" | "yaml" | "yml"))
}

fn structured_config_format_for_path(path: &str) -> Option<&'static str> {
    match Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json") => Some("json"),
        Some("toml") => Some("toml"),
        Some("yaml" | "yml") => Some("yaml"),
        _ => None,
    }
}

fn config_basic_validate_action(path: String) -> AgentAction {
    let mut args = serde_json::Map::new();
    args.insert("action".to_string(), Value::String("validate".to_string()));
    args.insert("path".to_string(), Value::String(path.clone()));
    if let Some(format) = structured_config_format_for_path(&path) {
        args.insert("format".to_string(), Value::String(format.to_string()));
    }
    args.insert(
        "validation_profile".to_string(),
        Value::String("syntax_only".to_string()),
    );
    AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: Value::Object(args),
    }
}

fn action_is_structured_config_validation(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let action_name = args
                .get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            skill.eq_ignore_ascii_case("config_basic")
                && action_name.eq_ignore_ascii_case("validate")
        }
        _ => false,
    }
}

fn config_validation_target_path(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: &[AgentAction],
) -> Option<String> {
    actions
        .iter()
        .find_map(planned_bounded_file_read_path)
        .or_else(|| {
            actions
                .iter()
                .find_map(planned_structured_config_observation_path)
        })
        .or_else(|| {
            auto_locator_path
                .map(str::trim)
                .filter(|path| !path.is_empty())
        })
        .or_else(|| {
            route_result
                .map(|route| route.output_contract.locator_hint.trim())
                .filter(|path| !path.is_empty())
        })
        .filter(|path| path_has_structured_document_extension(path))
        .map(ToString::to_string)
}

fn rewrite_config_validation_read_plan_to_validate(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::ConfigValidation
        || actions.iter().any(action_is_structured_config_validation)
    {
        return actions;
    }
    let Some(path) = config_validation_target_path(Some(route), auto_locator_path, &actions) else {
        return actions;
    };

    let mut rewritten = Vec::with_capacity(actions.len().max(1));
    let mut inserted = false;
    let mut changed = false;
    for action in actions {
        if !inserted
            && (action_observes_bounded_file_content(&action)
                || action_is_readonly_config_observation(&action))
        {
            rewritten.push(config_basic_validate_action(path.clone()));
            inserted = true;
            changed = true;
            continue;
        }
        if !inserted
            && matches!(
                action,
                AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. }
            )
        {
            rewritten.push(config_basic_validate_action(path.clone()));
            inserted = true;
            changed = true;
        }
        rewritten.push(action);
    }
    if !inserted {
        rewritten.push(config_basic_validate_action(path.clone()));
        changed = true;
    }
    if changed {
        info!(
            "plan_rewrite_config_validation_read_plan_to_validate path={}",
            crate::truncate_for_log(&path)
        );
    }
    rewritten
}

fn rewrite_unrequested_path_like_config_field_read_to_validate(
    state: &AppState,
    route_result: Option<&RouteResult>,
    user_text: &str,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
    {
        return actions;
    }

    let mut changed = false;
    let mut rewritten = Vec::with_capacity(actions.len());
    for action in actions {
        let Some(path) = unrequested_path_like_config_field_validation_path(
            state,
            route,
            user_text,
            auto_locator_path,
            &action,
        ) else {
            rewritten.push(action);
            continue;
        };
        info!(
            "plan_rewrite_unrequested_path_like_config_field_read_to_validate path={}",
            crate::truncate_for_log(&path)
        );
        rewritten.push(config_basic_validate_action(path));
        changed = true;
    }

    if changed {
        rewritten
    } else {
        rewritten
    }
}

fn unrequested_path_like_config_field_validation_path(
    state: &AppState,
    route: &RouteResult,
    user_text: &str,
    auto_locator_path: Option<&str>,
    action: &AgentAction,
) -> Option<String> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill.as_str(), args)
        }
        _ => return None,
    };
    if !skill.eq_ignore_ascii_case("system_basic") && !skill.eq_ignore_ascii_case("config_basic") {
        return None;
    }

    let request = structured_extract_request(args)?;
    let current = resolve_workspace_path(&state.skill_rt.workspace_root, &request.path);
    let current_text = current.display().to_string();
    if !path_has_structured_text_extension(&request.path)
        && !path_has_structured_text_extension(&current_text)
    {
        return None;
    }
    if structured_file_has_all_fields(&current, &request.fields) {
        return None;
    }
    if request
        .fields
        .iter()
        .any(|field| current_request_mentions_token(user_text, route, field))
    {
        return None;
    }
    if structured_scalar_field_selector(route, user_text, None, Some(&current_text)).is_some() {
        return None;
    }
    if !request
        .fields
        .iter()
        .any(|field| field_token_looks_like_locator(field))
    {
        return None;
    }

    let auto_path = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .filter(|path| path_has_structured_text_extension(path))
        .filter(|path| {
            same_existing_or_display_path(Path::new(path), &current) || !current.exists()
        });
    auto_path
        .map(ToString::to_string)
        .or_else(|| path_has_structured_text_extension(&request.path).then(|| request.path.clone()))
        .or_else(|| path_has_structured_text_extension(&current_text).then_some(current_text))
}

fn field_token_looks_like_locator(value: &str) -> bool {
    let token = value.trim();
    if token.is_empty() {
        return false;
    }
    Path::new(token).components().count() > 1 || filename_candidate_has_document_extension(token)
}

fn current_request_mentions_token(user_text: &str, route: &RouteResult, token: &str) -> bool {
    let token = token.trim();
    if token.is_empty() {
        return false;
    }
    let token_lower = token.to_ascii_lowercase();
    [user_text, route.resolved_intent.as_str()]
        .iter()
        .any(|text| text.contains(token) || text.to_ascii_lowercase().contains(&token_lower))
}

fn normalize_evidence_contract_actions(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    plan_context: Option<&str>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let actions = replace_file_paths_anchor_respond_only_with_find_entries(
        route_result,
        plan_context,
        actions,
    );
    let actions = replace_scalar_path_anchor_respond_only_with_stat_paths(
        route_result,
        plan_context,
        actions,
    );
    let actions = replace_content_evidence_synthesize_only_with_file_reads(
        state,
        route_result,
        loop_state,
        user_text,
        plan_context,
        actions,
    );
    let actions = replace_workspace_synthesis_respond_only_plan(route_result, loop_state, actions);
    let actions = rewrite_extract_field_alias_args(actions);
    let actions = rewrite_config_change_preview_to_config_edit_plan(
        route_result,
        user_text,
        auto_locator_path,
        actions,
    );
    let actions = rewrite_structured_multi_field_read_plan_to_read_fields(
        route_result,
        user_text,
        plan_context,
        auto_locator_path,
        actions,
    );
    let actions = rewrite_structured_scalar_field_read_plan_to_read_field(
        state,
        route_result,
        user_text,
        plan_context,
        auto_locator_path,
        actions,
    );
    let actions =
        rewrite_config_validation_read_plan_to_validate(route_result, auto_locator_path, actions);
    let actions = rewrite_unrequested_path_like_config_field_read_to_validate(
        state,
        route_result,
        user_text,
        auto_locator_path,
        actions,
    );
    let actions = rewrite_extract_field_paths_to_structured_candidates(
        state,
        route_result,
        auto_locator_path,
        actions,
    );
    let actions = prune_unscoped_workspace_summary_evidence_for_scope(state, route_result, actions);
    let actions =
        strip_unrequested_workspace_artifact_mutations(state, route_result, loop_state, actions);
    let actions =
        ensure_workspace_synthesis_has_default_text_evidence(route_result, loop_state, actions);
    let actions =
        append_synthesize_for_unscoped_workspace_text_evidence(route_result, loop_state, actions);
    let actions = rewrite_dir_compare_paths_to_unique_workspace_directories(state, actions);
    let actions = replace_directory_compare_search_plan(state, route_result, actions);
    let actions = rewrite_constructed_missing_stat_path_to_exact_find_entries(
        state,
        route_result,
        user_text,
        actions,
    );
    let actions = ensure_explicit_multi_file_targets_have_path_facts(
        route_result,
        loop_state,
        user_text,
        actions,
    );
    let actions = ensure_existence_multi_file_targets_have_path_facts(
        route_result,
        loop_state,
        user_text,
        actions,
    );
    let actions = append_synthesize_answer_for_structured_scalar_compare(route_result, actions);
    let actions =
        rewrite_unresolved_template_arg_multi_file_read_plan(route_result, user_text, actions);
    let actions = strip_unresolved_template_reads_after_inventory_dir(actions);
    let actions =
        strip_workspace_synthesis_without_text_evidence(route_result, loop_state, actions);
    actions
}

#[derive(Debug, Default)]
struct ActiveAnchorPlanContext {
    bound_target: Option<String>,
    ordered_entries: Vec<String>,
}

fn active_anchor_plan_context(plan_context: Option<&str>) -> ActiveAnchorPlanContext {
    let Some(plan_context) = plan_context else {
        return ActiveAnchorPlanContext::default();
    };
    let mut parsed = ActiveAnchorPlanContext::default();
    let mut in_active_anchor = false;
    for line in plan_context.lines() {
        let line = line.trim();
        if line == "### ACTIVE_EXECUTION_ANCHOR" {
            in_active_anchor = true;
            continue;
        }
        if in_active_anchor && line.starts_with("### ") {
            break;
        }
        if !in_active_anchor {
            continue;
        }
        if let Some(target) = line
            .strip_prefix("followup_bound_target:")
            .or_else(|| line.strip_prefix("observed_bound_target:"))
            .map(str::trim)
            .filter(|target| !target.is_empty())
        {
            parsed.bound_target = Some(target.to_string());
            continue;
        }
        if let Some(entries) = line
            .strip_prefix("followup_ordered_entries:")
            .or_else(|| line.strip_prefix("observed_ordered_entries:"))
        {
            parsed
                .ordered_entries
                .extend(active_anchor_ordered_entry_targets(entries));
        }
    }
    parsed
}

fn active_anchor_ordered_entry_targets(entries: &str) -> Vec<String> {
    entries
        .split(" | ")
        .filter_map(|entry| {
            let (ordinal, target) = entry.trim().split_once(':')?;
            ordinal
                .chars()
                .all(|ch| ch.is_ascii_digit())
                .then_some(target.trim())
        })
        .filter(|target| !target.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn normalize_path_token_for_anchor_match(path: &str) -> String {
    let mut normalized = path
        .trim()
        .trim_matches('`')
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string();
    while normalized.contains("//") {
        normalized = normalized.replace("//", "/");
    }
    normalized
}

fn plain_path_response_items(content: &str) -> Vec<String> {
    content
        .lines()
        .map(|line| line.trim().trim_matches('`'))
        .filter(|line| !line.is_empty())
        .filter(|line| !line.contains("{{") && !line.contains("}}"))
        .map(ToOwned::to_owned)
        .collect()
}

fn active_anchor_contains_all_path_items(
    anchor: &ActiveAnchorPlanContext,
    items: &[String],
) -> bool {
    if items.is_empty() || anchor.ordered_entries.is_empty() {
        return false;
    }
    let entry_set = anchor
        .ordered_entries
        .iter()
        .map(|entry| normalize_path_token_for_anchor_match(entry))
        .collect::<HashSet<_>>();
    items.iter().all(|item| {
        field_token_looks_like_locator(item)
            && entry_set.contains(&normalize_path_token_for_anchor_match(item))
    })
}

fn find_entries_action_for_selected_anchor_path(
    path: &str,
    bound_target: Option<&str>,
) -> Option<AgentAction> {
    let path = path.trim().trim_matches('`');
    if path.is_empty() || path.contains('\n') || path.contains("{{") {
        return None;
    }
    let path_obj = Path::new(path);
    let basename = path_obj
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(path);
    let root = if path_obj.components().count() > 1 || path_obj.is_absolute() {
        path_obj
            .parent()
            .and_then(|parent| parent.to_str())
            .filter(|parent| !parent.is_empty())
            .unwrap_or(".")
            .to_string()
    } else {
        bound_target
            .map(str::trim)
            .filter(|target| !target.is_empty())
            .unwrap_or(".")
            .to_string()
    };
    Some(AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "find_entries",
            "root": root,
            "pattern": basename,
            "target_kind": "file",
            "max_results": 50,
        }),
    })
}

fn replace_file_paths_anchor_respond_only_with_find_entries(
    route_result: Option<&RouteResult>,
    plan_context: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::FilePaths
        || !route.output_contract.requires_content_evidence
    {
        return actions;
    }
    let Some(content) = is_plain_respond_only_plan(&actions) else {
        return actions;
    };
    let items = plain_path_response_items(content);
    let anchor = active_anchor_plan_context(plan_context);
    if !active_anchor_contains_all_path_items(&anchor, &items) {
        return actions;
    }
    let mut rewritten = items
        .iter()
        .filter_map(|item| {
            find_entries_action_for_selected_anchor_path(item, anchor.bound_target.as_deref())
        })
        .collect::<Vec<_>>();
    if rewritten.is_empty() {
        return actions;
    }
    rewritten.push(AgentAction::Respond {
        content: content.trim().to_string(),
    });
    info!(
        "plan_replace_file_paths_anchor_respond_only_with_find_entries entries={}",
        items.len()
    );
    rewritten
}

fn replace_scalar_path_anchor_respond_only_with_stat_paths(
    route_result: Option<&RouteResult>,
    plan_context: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::ScalarPathOnly
        || !route.output_contract.requires_content_evidence
    {
        return actions;
    }
    let Some(content) = is_plain_respond_only_plan(&actions) else {
        return actions;
    };
    let items = plain_path_response_items(content);
    if items.len() != 1 {
        return actions;
    }
    let anchor = active_anchor_plan_context(plan_context);
    if !active_anchor_contains_all_path_items(&anchor, &items) {
        return actions;
    }
    info!("plan_replace_scalar_path_anchor_respond_only_with_stat_paths");
    vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "paths": items,
            "include_missing": true,
        }),
    }]
}

#[derive(Debug, Clone)]
struct ParsedConfigChangePreview {
    path: String,
    field_path: String,
    value: Value,
}

fn rewrite_config_change_preview_to_config_edit_plan(
    route_result: Option<&RouteResult>,
    user_text: &str,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify
        || route.output_contract.delivery_required
        || actions.iter().any(action_targets_config_edit)
        || !actions.iter().any(action_is_readonly_config_observation)
        || actions.iter().any(action_is_obvious_mutation)
    {
        return actions;
    }
    let Some(parsed) = parse_config_change_preview(user_text, route, auto_locator_path) else {
        return actions;
    };
    info!(
        "plan_rewrite_config_change_preview_to_config_edit_plan path={} field={}",
        crate::truncate_for_log(&parsed.path),
        crate::truncate_for_log(&parsed.field_path)
    );
    vec![AgentAction::CallTool {
        tool: "config_edit".to_string(),
        args: serde_json::json!({
            "action": "plan_config_change",
            "path": parsed.path,
            "field_path": parsed.field_path,
            "value": parsed.value,
        }),
    }]
}

fn parse_config_change_preview(
    user_text: &str,
    route: &RouteResult,
    auto_locator_path: Option<&str>,
) -> Option<ParsedConfigChangePreview> {
    let field_path = crate::intent::surface_signals::extract_dotted_field_selector(user_text)?;
    let value = parse_config_change_value_after_field(user_text, &field_path)?;
    let path = config_change_preview_path(user_text, route, auto_locator_path)
        .unwrap_or_else(|| "configs/config.toml".to_string());
    Some(ParsedConfigChangePreview {
        path,
        field_path,
        value,
    })
}

fn config_change_preview_path(
    user_text: &str,
    route: &RouteResult,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    auto_locator_path
        .map(str::trim)
        .filter(|path| path_has_structured_text_extension(path))
        .map(ToString::to_string)
        .or_else(|| route_locator_structured_config_path(route))
        .or_else(|| {
            crate::intent::locator_extractor::extract_explicit_locator_for_fallback(user_text)
                .and_then(|locator| {
                    path_has_structured_text_extension(&locator.locator_hint)
                        .then_some(locator.locator_hint)
                })
        })
}

fn route_locator_structured_config_path(route: &RouteResult) -> Option<String> {
    let hint = route.output_contract.locator_hint.trim();
    if hint.is_empty() || !path_has_structured_text_extension(hint) {
        return None;
    }
    matches!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
            | crate::OutputLocatorKind::Filename
            | crate::OutputLocatorKind::CurrentWorkspace
    )
    .then(|| hint.to_string())
}

fn rewrite_structured_scalar_field_read_plan_to_read_field(
    state: &AppState,
    route_result: Option<&RouteResult>,
    user_text: &str,
    plan_context: Option<&str>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    let identifier_presence_contract = route_reason_has_structural_marker(
        route,
        "structured_identifier_presence_requires_content_evidence",
    );
    if route.needs_clarify
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || route.output_contract.semantic_kind == crate::OutputSemanticKind::StructuredKeys
        || actions.iter().any(action_is_structured_scalar_field_read)
        || (!actions.iter().any(action_observes_structured_source)
            && !(identifier_presence_contract
                && actions
                    .iter()
                    .any(planned_action_is_single_path_metadata_facts)))
        || actions.iter().any(|action| {
            !action_observes_structured_source(action)
                && !(identifier_presence_contract
                    && planned_action_is_single_path_metadata_facts(action))
                && !matches!(
                    action,
                    AgentAction::SynthesizeAnswer { .. }
                        | AgentAction::Respond { .. }
                        | AgentAction::Think { .. }
                )
        })
    {
        return actions;
    }
    let Some(path) = structured_scalar_field_read_target_path(route, auto_locator_path, &actions)
    else {
        return actions;
    };
    let Some(field_path) = structured_current_turn_field_selector(route, user_text, Some(&path))
        .or_else(|| {
            structured_scalar_field_selector_from_structural_candidates(
                state,
                route,
                user_text,
                plan_context,
                &path,
            )
        })
    else {
        return actions;
    };
    if route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        && !actions.iter().any(action_is_readonly_config_observation)
    {
        return actions;
    }

    info!(
        "plan_rewrite_structured_scalar_field_read_to_config_basic path={} field={}",
        crate::truncate_for_log(&path),
        crate::truncate_for_log(&field_path)
    );
    vec![config_basic_read_field_action(path, field_path)]
}

fn rewrite_structured_multi_field_read_plan_to_read_fields(
    route_result: Option<&RouteResult>,
    user_text: &str,
    _plan_context: Option<&str>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || actions.iter().any(action_is_structured_scalar_field_read)
        || !actions.iter().any(action_observes_structured_source)
        || actions.iter().any(|action| {
            !action_observes_structured_source(action)
                && !matches!(
                    action,
                    AgentAction::SynthesizeAnswer { .. }
                        | AgentAction::Respond { .. }
                        | AgentAction::Think { .. }
                )
        })
    {
        return actions;
    }
    let Some(path) = structured_scalar_field_read_target_path(route, auto_locator_path, &actions)
    else {
        return actions;
    };
    let field_paths = structured_current_turn_field_selectors(route, user_text, Some(&path));
    if field_paths.len() < 2 {
        return actions;
    }

    info!(
        "plan_rewrite_structured_multi_field_read_to_config_basic path={} fields={:?}",
        crate::truncate_for_log(&path),
        field_paths
    );
    vec![config_basic_read_fields_action(path, field_paths)]
}

fn action_observes_structured_source(action: &AgentAction) -> bool {
    action_observes_bounded_file_content(action) || action_is_readonly_config_observation(action)
}

fn action_is_structured_scalar_field_read(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let action_name = args
                .get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            (skill.eq_ignore_ascii_case("config_basic")
                && matches!(
                    action_name.to_ascii_lowercase().as_str(),
                    "read_field" | "read_fields"
                ))
                || (skill.eq_ignore_ascii_case("system_basic")
                    && matches!(
                        action_name.to_ascii_lowercase().as_str(),
                        "extract_field" | "extract_fields"
                    ))
        }
        AgentAction::CallCapability { .. }
        | AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => false,
    }
}

fn structured_scalar_field_read_target_path(
    route: &RouteResult,
    auto_locator_path: Option<&str>,
    actions: &[AgentAction],
) -> Option<String> {
    actions
        .iter()
        .find_map(planned_structured_config_observation_path)
        .or_else(|| actions.iter().find_map(planned_bounded_file_read_path))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            actions
                .iter()
                .find_map(planned_single_path_metadata_facts_path)
        })
        .or_else(|| {
            auto_locator_path
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(ToString::to_string)
        })
        .or_else(|| route_locator_structured_config_path(route))
        .filter(|path| path_has_structured_document_extension(path))
}

fn structured_scalar_field_selector(
    route: &RouteResult,
    user_text: &str,
    plan_context: Option<&str>,
    target_path: Option<&str>,
) -> Option<String> {
    structured_field_selectors(route, user_text, plan_context, target_path)
        .into_iter()
        .next()
}

fn structured_current_turn_field_selector(
    route: &RouteResult,
    user_text: &str,
    target_path: Option<&str>,
) -> Option<String> {
    structured_current_turn_field_selectors(route, user_text, target_path)
        .into_iter()
        .next()
}

fn structured_current_turn_field_selectors(
    route: &RouteResult,
    user_text: &str,
    target_path: Option<&str>,
) -> Vec<String> {
    let current_turn_sources = [Some(user_text), Some(route.resolved_intent.as_str())];
    structured_field_selectors_from_sources(&current_turn_sources, target_path)
}

fn structured_field_selectors(
    route: &RouteResult,
    user_text: &str,
    plan_context: Option<&str>,
    target_path: Option<&str>,
) -> Vec<String> {
    let selectors = structured_current_turn_field_selectors(route, user_text, target_path);
    if !selectors.is_empty() {
        return selectors;
    }

    let fallback_sources = [plan_context];
    structured_field_selectors_from_sources(&fallback_sources, target_path)
}

fn structured_field_selectors_from_sources(
    sources: &[Option<&str>],
    target_path: Option<&str>,
) -> Vec<String> {
    let mut selectors = Vec::new();
    for text in sources.iter().flatten() {
        for candidate in extract_dotted_field_selectors_for_structured_target(text) {
            push_unique_selector(&mut selectors, candidate);
        }
    }
    if !selectors.is_empty() {
        return selectors;
    }

    if let Some(path) = target_path {
        for text in sources.iter().flatten() {
            for candidate in extract_schema_identity_field_selectors(path, text) {
                push_unique_selector(&mut selectors, candidate);
            }
        }
        if !selectors.is_empty() {
            return selectors;
        }

        for text in sources.iter().flatten() {
            for candidate in extract_schema_backed_field_selectors(path, text) {
                push_unique_selector(&mut selectors, candidate);
            }
        }
    }

    selectors
}

fn planned_action_is_single_path_metadata_facts(action: &AgentAction) -> bool {
    planned_single_path_metadata_facts_path(action).is_some()
}

fn planned_single_path_metadata_facts_path(action: &AgentAction) -> Option<String> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill.as_str(), args)
        }
        AgentAction::CallCapability { .. }
        | AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => return None,
    };
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let is_stat = (skill.eq_ignore_ascii_case("fs_basic")
        && action_name.eq_ignore_ascii_case("stat_paths"))
        || (skill.eq_ignore_ascii_case("system_basic")
            && action_name.eq_ignore_ascii_case("path_batch_facts"));
    if !is_stat {
        return None;
    }
    let Some(obj) = args.as_object() else {
        return None;
    };
    let paths = string_list_from_value(obj.get("paths"))
        .into_iter()
        .chain(string_list_from_value(obj.get("targets")))
        .chain(string_list_from_value(obj.get("path")))
        .collect::<Vec<_>>();
    (paths.len() == 1).then(|| paths[0].clone())
}

fn route_reason_has_structural_marker(route: &RouteResult, marker: &str) -> bool {
    route.route_reason.split(';').map(str::trim).any(|part| {
        part == marker
            || part
                .rsplit_once(':')
                .is_some_and(|(_, suffix)| suffix.trim() == marker)
    })
}

fn route_allows_structured_field_token_fallback(route: &RouteResult) -> bool {
    if route.needs_clarify
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        )
    {
        return false;
    }
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::None
        && matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
        && !route.output_contract.locator_hint.trim().is_empty()
    {
        return true;
    }
    [
        "single_path_field_extraction_semantic_kind_none_is_valid",
        "contract_valid_minor_repair_fields_only",
        "structured_field_selector_requires_scalar_value",
        "structured_keys_scalar_response_requires_field_value",
        "structured_identifier_presence_requires_content_evidence",
    ]
    .iter()
    .any(|marker| route_reason_has_structural_marker(route, marker))
}

fn structured_scalar_field_selector_from_structural_candidates(
    state: &AppState,
    route: &RouteResult,
    user_text: &str,
    plan_context: Option<&str>,
    target_path: &str,
) -> Option<String> {
    if !route_allows_structured_field_token_fallback(route)
        || !path_has_structured_document_extension(target_path)
    {
        return None;
    }
    let current_turn_sources = [Some(user_text), Some(route.resolved_intent.as_str())];
    structured_scalar_field_selector_from_candidate_sources(
        state,
        &current_turn_sources,
        target_path,
    )
    .or_else(|| {
        let fallback_sources = [plan_context];
        structured_scalar_field_selector_from_candidate_sources(
            state,
            &fallback_sources,
            target_path,
        )
    })
}

fn structured_scalar_field_selector_from_candidate_sources(
    state: &AppState,
    sources: &[Option<&str>],
    target_path: &str,
) -> Option<String> {
    let current = resolve_workspace_path(&state.skill_rt.workspace_root, target_path);
    let mut selectors = Vec::new();
    for text in sources.iter().flatten() {
        for token in schema_field_candidate_tokens(text) {
            let fields = vec![token.clone()];
            let token_matches_target = structured_file_has_all_fields(&current, &fields)
                || find_structured_field_candidate(
                    &state.skill_rt.workspace_root,
                    &current,
                    &fields,
                    state.skill_rt.locator_scan_max_files,
                )
                .is_some();
            if token_matches_target {
                push_unique_selector(&mut selectors, token);
            }
        }
    }
    if selectors.len() == 1 {
        selectors.into_iter().next()
    } else {
        selectors.clear();
        for text in sources.iter().flatten() {
            for selector in extract_schema_identity_presence_selectors(&current, text) {
                push_unique_selector(&mut selectors, selector);
            }
        }
        (selectors.len() == 1).then(|| selectors.remove(0))
    }
}

fn push_unique_selector(out: &mut Vec<String>, candidate: String) {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return;
    }
    if out
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(candidate))
    {
        return;
    }
    out.push(candidate.to_string());
}

fn structured_field_selector_candidate_is_valid(candidate: &str) -> bool {
    let token = candidate.trim();
    !token.is_empty()
        && !token.contains('/')
        && !token.contains('\\')
        && !filename_candidate_has_document_extension(token)
        && !path_has_structured_document_extension(token)
        && !crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(token)
}

fn extract_dotted_field_selectors_for_structured_target(text: &str) -> Vec<String> {
    static DOTTED_SELECTOR_RE: OnceLock<Regex> = OnceLock::new();
    let re = DOTTED_SELECTOR_RE.get_or_init(|| {
        Regex::new(r"\b[A-Za-z_$][A-Za-z0-9_$-]*(?:\.[A-Za-z_$][A-Za-z0-9_$-]*)+\b")
            .expect("valid dotted selector regex")
    });
    let mut out = Vec::new();
    for candidate in re.find_iter(text) {
        let token = candidate.as_str().trim();
        if structured_field_selector_candidate_is_valid(token) {
            push_unique_selector(&mut out, token.to_string());
        }
    }
    out
}

fn extract_schema_backed_field_selectors(path: &str, text: &str) -> Vec<String> {
    let Some(value) = parse_structured_file_value(Path::new(path)) else {
        return Vec::new();
    };
    let index = structured_field_leaf_index(&value);
    let mut out = Vec::new();
    for token in schema_field_candidate_tokens(text) {
        let lower = token.to_ascii_lowercase();
        let Some(paths) = index.get(&lower) else {
            continue;
        };
        if paths.iter().any(|path| path.eq_ignore_ascii_case(&token)) {
            push_unique_selector(&mut out, token);
            continue;
        }
        if paths.len() == 1 {
            push_unique_selector(&mut out, paths[0].clone());
        }
    }
    out
}

fn extract_schema_identity_field_selectors(path: &str, text: &str) -> Vec<String> {
    let Some(value) = parse_structured_file_value(Path::new(path)) else {
        return Vec::new();
    };
    let tokens = schema_field_candidate_tokens(text).collect::<Vec<_>>();
    if tokens.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    collect_schema_identity_field_selectors(&value, &tokens, &mut out);
    out
}

fn extract_schema_identity_presence_selectors(path: &Path, text: &str) -> Vec<String> {
    let Some(value) = parse_structured_file_value(path) else {
        return Vec::new();
    };
    let tokens = schema_field_candidate_tokens(text).collect::<Vec<_>>();
    if tokens.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    collect_schema_identity_presence_selectors(&value, &tokens, &mut out);
    out
}

fn structured_field_path_resolves_scalar_value(path: &str, field_path: &str) -> bool {
    let Some(value) = parse_structured_file_value(Path::new(path)) else {
        return false;
    };
    lookup_structured_field_value_with_identity(&value, field_path).is_some_and(|value| {
        matches!(
            value,
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
        )
    })
}

fn lookup_structured_field_value_with_identity<'a>(
    value: &'a Value,
    field_path: &str,
) -> Option<&'a Value> {
    lookup_structured_field_value(value, field_path)
        .or_else(|| lookup_structured_array_identity_field_value(value, field_path))
}

fn lookup_structured_array_identity_field_value<'a>(
    value: &'a Value,
    field_path: &str,
) -> Option<&'a Value> {
    let segments = field_path.split('.').collect::<Vec<_>>();
    if segments.len() < 2 {
        return None;
    }
    let selector_value = segments.first()?.trim();
    if selector_value.is_empty() || selector_value.contains('[') || selector_value.contains(']') {
        return None;
    }
    let nested_field_path = segments[1..].join(".");
    if nested_field_path.trim().is_empty() {
        return None;
    }

    let mut matches = Vec::new();
    collect_structured_array_identity_field_values(
        value,
        selector_value,
        &nested_field_path,
        &mut matches,
    );
    (matches.len() == 1).then(|| matches.remove(0))
}

fn collect_structured_array_identity_field_values<'a>(
    value: &'a Value,
    selector_value: &str,
    nested_field_path: &str,
    out: &mut Vec<&'a Value>,
) {
    match value {
        Value::Object(map) => {
            for child in map.values() {
                collect_structured_array_identity_field_values(
                    child,
                    selector_value,
                    nested_field_path,
                    out,
                );
            }
        }
        Value::Array(items) => {
            for item in items {
                if structured_array_item_matches_identity(item, selector_value) {
                    if let Some(nested_value) =
                        lookup_structured_field_value(item, nested_field_path)
                    {
                        out.push(nested_value);
                    }
                }
                collect_structured_array_identity_field_values(
                    item,
                    selector_value,
                    nested_field_path,
                    out,
                );
            }
        }
        _ => {}
    }
}

fn structured_array_item_matches_identity(item: &Value, selector_value: &str) -> bool {
    item.as_object().is_some_and(|map| {
        ["name", "id", "key"].iter().any(|identity_key| {
            map.get(*identity_key)
                .and_then(Value::as_str)
                .is_some_and(|value| value == selector_value)
        })
    })
}

fn collect_schema_identity_field_selectors(
    value: &Value,
    tokens: &[String],
    out: &mut Vec<String>,
) {
    match value {
        Value::Object(map) => {
            for child in map.values() {
                collect_schema_identity_field_selectors(child, tokens, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                if let Some(obj) = item.as_object() {
                    for identity_key in ["name", "id", "key"] {
                        let Some(identity_value) = obj.get(identity_key).and_then(Value::as_str)
                        else {
                            continue;
                        };
                        if !schema_text_tokens_contain(tokens, identity_value) {
                            continue;
                        }
                        for field_key in obj.keys() {
                            if field_key == identity_key || !schema_field_token_is_valid(field_key)
                            {
                                continue;
                            }
                            if schema_text_tokens_contain(tokens, field_key) {
                                push_unique_selector(out, format!("{identity_value}.{field_key}"));
                            }
                        }
                    }
                }
                collect_schema_identity_field_selectors(item, tokens, out);
            }
        }
        _ => {}
    }
}

fn collect_schema_identity_presence_selectors(
    value: &Value,
    tokens: &[String],
    out: &mut Vec<String>,
) {
    match value {
        Value::Object(map) => {
            for child in map.values() {
                collect_schema_identity_presence_selectors(child, tokens, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                if let Some(obj) = item.as_object() {
                    for identity_key in ["name", "id", "key"] {
                        let Some(identity_value) = obj.get(identity_key).and_then(Value::as_str)
                        else {
                            continue;
                        };
                        if schema_text_tokens_contain(tokens, identity_value)
                            && schema_field_token_is_valid(identity_key)
                        {
                            push_unique_selector(out, format!("{identity_value}.{identity_key}"));
                        }
                    }
                }
                collect_schema_identity_presence_selectors(item, tokens, out);
            }
        }
        _ => {}
    }
}

fn schema_text_tokens_contain(tokens: &[String], needle: &str) -> bool {
    tokens
        .iter()
        .any(|token| token.eq_ignore_ascii_case(needle))
}

fn structured_field_leaf_index(value: &Value) -> HashMap<String, Vec<String>> {
    let mut out = HashMap::new();
    collect_structured_field_leaf_index(value, "", &mut out);
    out
}

fn collect_structured_field_leaf_index(
    value: &Value,
    prefix: &str,
    out: &mut HashMap<String, Vec<String>>,
) {
    let Some(obj) = value.as_object() else {
        return;
    };
    for (key, child) in obj {
        if !schema_field_token_is_valid(key) {
            continue;
        }
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        let lower = key.to_ascii_lowercase();
        out.entry(lower).or_insert_with(Vec::new).push(path.clone());
        collect_structured_field_leaf_index(child, &path, out);
    }
}

fn schema_field_candidate_tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split_whitespace().filter_map(|token| {
        let trimmed = token
            .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && !matches!(ch, '_' | '-' | '$'));
        schema_field_token_is_valid(trimmed).then(|| trimmed.to_string())
    })
}

fn schema_field_token_is_valid(token: &str) -> bool {
    !token.is_empty()
        && !token.contains('/')
        && !token.contains('\\')
        && !path_has_structured_document_extension(token)
        && !crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(token)
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '$'))
}

fn config_basic_read_field_action(path: String, field_path: String) -> AgentAction {
    AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: serde_json::json!({
            "action": "read_field",
            "path": path,
            "field_path": field_path,
        }),
    }
}

fn config_basic_read_fields_action(path: String, field_paths: Vec<String>) -> AgentAction {
    AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: serde_json::json!({
            "action": "read_fields",
            "path": path,
            "field_paths": field_paths,
        }),
    }
}

fn parse_config_change_value_after_field(user_text: &str, field_path: &str) -> Option<Value> {
    let lower = user_text.to_ascii_lowercase();
    let field_lower = field_path.to_ascii_lowercase();
    let field_idx = lower.find(&field_lower)?;
    let suffix = user_text.get(field_idx + field_path.len()..)?;
    config_value_candidate_tokens(suffix).find_map(parse_config_value_token)
}

fn config_value_candidate_tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                ',' | '，'
                    | '。'
                    | ';'
                    | '；'
                    | ':'
                    | '：'
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | '《'
                    | '》'
            )
    })
    .map(|token| {
        token
            .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | '=' | '>' | '-' | '→'))
            .trim()
            .to_string()
    })
    .filter(|token| !token.is_empty())
}

fn parse_config_value_token(token: String) -> Option<Value> {
    if token.eq_ignore_ascii_case("true") {
        return Some(Value::Bool(true));
    }
    if token.eq_ignore_ascii_case("false") {
        return Some(Value::Bool(false));
    }
    if token.eq_ignore_ascii_case("null") {
        return Some(Value::Null);
    }
    if let Ok(value) = token.parse::<i64>() {
        return Some(Value::Number(value.into()));
    }
    if let Ok(value) = token.parse::<f64>() {
        if let Some(number) = serde_json::Number::from_f64(value) {
            return Some(Value::Number(number));
        }
    }
    None
}

fn action_targets_config_edit(action: &AgentAction) -> bool {
    matches!(
        action,
        AgentAction::CallTool { tool, .. } | AgentAction::CallSkill { skill: tool, .. }
            if tool.eq_ignore_ascii_case("config_edit")
    )
}

fn action_is_readonly_config_observation(action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallTool { tool, args } | AgentAction::CallSkill { skill: tool, args } => {
            (tool.as_str(), args)
        }
        _ => return false,
    };
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if skill.eq_ignore_ascii_case("config_basic") {
        return matches!(
            action,
            "read_field"
                | "read_fields"
                | "list_keys"
                | "validate"
                | "extract_field"
                | "extract_fields"
                | "structured_keys"
                | "validate_structured"
        );
    }
    skill.eq_ignore_ascii_case("system_basic")
        && matches!(
            action,
            "extract_field" | "extract_fields" | "structured_keys" | "validate_structured"
        )
}

fn action_is_obvious_mutation(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallTool { tool, args } | AgentAction::CallSkill { skill: tool, args } => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if tool.eq_ignore_ascii_case("config_edit") {
                return matches!(
                    action.as_str(),
                    "apply_config_change" | "apply_change" | "write_field" | "set_field"
                );
            }
            if tool.eq_ignore_ascii_case("fs_basic") || tool.eq_ignore_ascii_case("system_basic") {
                return action.contains("write")
                    || action.contains("append")
                    || action.contains("patch")
                    || action.contains("delete")
                    || action.contains("remove");
            }
            false
        }
        _ => false,
    }
}

fn strip_unrequested_config_edit_actions(
    route_result: Option<&RouteResult>,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !actions.iter().any(action_targets_config_edit)
        || current_turn_requests_config_edit(route_result, user_text, original_user_text, &actions)
    {
        return actions;
    }
    let before = actions.len();
    let stripped = actions
        .into_iter()
        .filter(|action| !action_targets_config_edit(action))
        .collect::<Vec<_>>();
    let dropped = before.saturating_sub(stripped.len());
    if dropped > 0 {
        info!("plan_strip_unrequested_config_edit_actions dropped={dropped}");
    }
    stripped
}

fn current_turn_requests_config_edit(
    route_result: Option<&RouteResult>,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: &[AgentAction],
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if route.needs_clarify || !route.is_execute_gate() {
        return false;
    }
    let request = original_user_text.unwrap_or(user_text).trim();
    if request.is_empty() {
        return false;
    }

    let config_actions = actions
        .iter()
        .filter(|action| action_targets_config_edit(action))
        .collect::<Vec<_>>();
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::ConfigRiskAssessment {
        return !config_actions.is_empty()
            && config_actions
                .iter()
                .all(|action| config_edit_action_is_route_guard(action, route));
    }
    if route.output_contract.requires_content_evidence
        && !config_actions.is_empty()
        && config_actions
            .iter()
            .all(|action| config_edit_action_is_route_guard(action, route))
    {
        return true;
    }
    !config_actions.is_empty()
        && config_actions
            .iter()
            .all(|action| config_edit_action_has_current_structural_anchor(action, request))
}

fn config_edit_action_is_route_guard(action: &AgentAction, route: &RouteResult) -> bool {
    let (tool, args) = match action {
        AgentAction::CallTool { tool, args } | AgentAction::CallSkill { skill: tool, args } => {
            (tool.as_str(), args)
        }
        _ => return false,
    };
    if !tool.eq_ignore_ascii_case("config_edit") {
        return false;
    }
    if args.get("action").and_then(Value::as_str).map(str::trim) != Some("guard_config") {
        return false;
    }
    let Some(path) = json_trimmed_string_arg(args, &["path", "file", "file_path", "config_path"])
    else {
        return false;
    };
    let locator = route.output_contract.locator_hint.trim();
    is_rustclaw_config_guard_path(&path)
        && (locator.is_empty()
            || is_rustclaw_config_guard_path(locator)
            || path
                .replace('\\', "/")
                .eq_ignore_ascii_case(&locator.replace('\\', "/")))
}

fn config_edit_action_has_current_structural_anchor(action: &AgentAction, request: &str) -> bool {
    let (tool, args) = match action {
        AgentAction::CallTool { tool, args } | AgentAction::CallSkill { skill: tool, args } => {
            (tool.as_str(), args)
        }
        _ => return true,
    };
    if !tool.eq_ignore_ascii_case("config_edit") {
        return true;
    }
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if matches!(action_name, "guard_config" | "validate_config" | "validate") {
        return config_edit_path_is_anchored(args, request).unwrap_or(false);
    }

    config_edit_path_is_anchored(args, request).unwrap_or(false)
        && config_edit_field_is_anchored(args, request).unwrap_or(false)
        && config_edit_value_is_anchored(args, request).unwrap_or(true)
}

fn config_edit_path_is_anchored(args: &Value, request: &str) -> Option<bool> {
    let path = json_trimmed_string_arg(args, &["path", "file", "file_path", "config_path"])?;
    let mut tokens = vec![path.clone(), path.replace('\\', "/")];
    if let Some(file_name) = Path::new(&path).file_name().and_then(|name| name.to_str()) {
        tokens.push(file_name.to_string());
    }
    Some(
        tokens
            .iter()
            .any(|token| structural_token_present(request, token)),
    )
}

fn config_edit_field_is_anchored(args: &Value, request: &str) -> Option<bool> {
    let field_path = json_trimmed_string_arg(args, &["field_path", "field", "key"])?;
    let mut tokens = vec![field_path.clone()];
    if let Some(leaf) = field_path
        .rsplit('.')
        .next()
        .filter(|leaf| !leaf.is_empty())
    {
        tokens.push(leaf.to_string());
    }
    Some(
        tokens
            .iter()
            .any(|token| structural_token_present(request, token)),
    )
}

fn config_edit_value_is_anchored(args: &Value, request: &str) -> Option<bool> {
    let token = scalar_value_anchor_token(args.get("value")?)?;
    Some(structural_token_present(request, &token))
}

fn json_trimmed_string_arg(args: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| args.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn scalar_value_anchor_token(value: &Value) -> Option<String> {
    match value {
        Value::Null => Some("null".to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        }
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn structural_token_present(text: &str, token: &str) -> bool {
    let token = token
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`'))
        .replace('\\', "/");
    if token.is_empty() {
        return false;
    }
    text.replace('\\', "/")
        .to_ascii_lowercase()
        .contains(&token.to_ascii_lowercase())
}

fn normalize_terminal_delivery_actions(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    terminal_mixed_last_output_content: Option<String>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let actions =
        rewrite_observed_terminal_synthesis_concrete_respond(route_result, loop_state, actions);
    let actions = strip_pre_observation_synthesize_before_concrete_respond(loop_state, actions);
    let actions =
        rewrite_pre_observation_concrete_respond_to_placeholder(route_result, loop_state, actions);
    let actions =
        rewrite_mixed_placeholder_observed_synthesis_respond(route_result, loop_state, actions);
    let actions =
        rewrite_mixed_placeholder_structured_output_respond(route_result, loop_state, actions);
    let actions = rewrite_terminal_synthesis_placeholder_respond(actions);
    let actions = strip_intermediate_synthesize_before_later_execution(actions);
    let actions = append_respond_for_terminal_synthesize_answer(actions);
    let actions = rewrite_terminal_placeholder_respond_to_synthesize_answer(loop_state, actions);
    let actions =
        strip_terminal_placeholder_respond_for_exact_listing_contract(route_result, actions);
    let actions = inject_synthesize_answer_for_bare_placeholder_respond(actions, user_text);
    let actions = append_synthesize_for_observation_only_terminal_answer(
        state,
        route_result,
        loop_state,
        actions,
    );
    let actions = restore_terminal_mixed_last_output_respond(
        route_result,
        terminal_mixed_last_output_content,
        actions,
    );
    let actions = strip_service_status_discussion_actions(route_result, actions);
    let actions = strip_redundant_make_dir_before_file_delivery_write(state, route_result, actions);
    let actions =
        append_file_token_after_generated_file_write_delivery(state, route_result, actions);
    mark_missing_target_repairable_actions(state, route_result, actions)
}

fn string_list_from_value(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect(),
        Some(Value::String(item)) => {
            let item = item.trim();
            if item.is_empty() {
                Vec::new()
            } else {
                vec![item.to_string()]
            }
        }
        _ => Vec::new(),
    }
}

fn parse_positive_usize(value: &Value) -> Option<usize> {
    match value {
        Value::Number(number) => number.as_u64().map(|n| n as usize).filter(|n| *n > 0),
        Value::String(text) => text.trim().parse::<usize>().ok().filter(|n| *n > 0),
        _ => None,
    }
}

fn parse_i64_value(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn parse_line_range_text(text: &str) -> Option<(usize, usize)> {
    let nums = text
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .collect::<Vec<_>>();
    match nums.as_slice() {
        [end] => Some((1, *end)),
        [start, end, ..] => Some((*start, (*end).max(*start))),
        _ => None,
    }
}

fn parse_line_range_value(value: &Value) -> Option<(usize, usize)> {
    match value {
        Value::String(text) => parse_line_range_text(text),
        Value::Array(items) => {
            if items.is_empty() {
                return None;
            }
            let start = parse_positive_usize(items.first()?)?;
            let end = items.get(1).and_then(parse_positive_usize).unwrap_or(start);
            Some((start, end.max(start)))
        }
        Value::Object(obj) => {
            let start = obj
                .get("start_line")
                .or_else(|| obj.get("start"))
                .or_else(|| obj.get("from"))
                .and_then(parse_positive_usize)
                .unwrap_or(1);
            let end = obj
                .get("end_line")
                .or_else(|| obj.get("end"))
                .or_else(|| obj.get("to"))
                .and_then(parse_positive_usize)?;
            Some((start, end.max(start)))
        }
        Value::Number(_) => parse_positive_usize(value).map(|end| (1, end)),
        _ => None,
    }
}

fn normalize_read_range_negative_bounds(obj: &mut serde_json::Map<String, Value>) -> bool {
    let Some(start) = obj.get("start_line").and_then(parse_i64_value) else {
        return false;
    };
    let Some(end) = obj.get("end_line").and_then(parse_i64_value) else {
        return false;
    };
    if start >= 0 || end >= 0 || start > end {
        return false;
    }
    let n = end.saturating_sub(start).saturating_add(1);
    if n <= 0 {
        return false;
    }
    obj.insert("mode".to_string(), Value::String("tail".to_string()));
    obj.insert(
        "n".to_string(),
        Value::Number(serde_json::Number::from(n as u64)),
    );
    obj.remove("start_line");
    obj.remove("end_line");
    true
}

fn line_count_template_tail_n(start: &str, end: &str) -> Option<usize> {
    let start = start.trim();
    let end = end.trim();
    if !start.contains("line_count") || !end.contains("line_count") {
        return None;
    }
    let marker = start.rsplit_once('-')?.1.trim();
    let offset = marker
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .parse::<usize>()
        .ok()?;
    Some(offset.saturating_add(1)).filter(|n| *n > 0)
}

fn normalize_read_range_line_count_template(obj: &mut serde_json::Map<String, Value>) -> bool {
    let Some(start) = obj.get("start_line").and_then(Value::as_str) else {
        return false;
    };
    let Some(end) = obj.get("end_line").and_then(Value::as_str) else {
        return false;
    };
    let Some(n) = line_count_template_tail_n(start, end) else {
        return false;
    };
    obj.insert("mode".to_string(), Value::String("tail".to_string()));
    obj.insert(
        "n".to_string(),
        Value::Number(serde_json::Number::from(n as u64)),
    );
    obj.remove("start_line");
    obj.remove("end_line");
    true
}

fn normalize_read_range_negative_start_count(obj: &mut serde_json::Map<String, Value>) -> bool {
    let Some(start) = obj.get("start_line").and_then(parse_i64_value) else {
        return false;
    };
    if start >= 0 {
        return false;
    }
    let n = obj
        .get("line_count")
        .or_else(|| obj.get("count"))
        .or_else(|| obj.get("limit"))
        .or_else(|| obj.get("n"))
        .and_then(parse_i64_value)
        .unwrap_or_else(|| start.saturating_abs());
    if n <= 0 {
        return false;
    }
    obj.insert("mode".to_string(), Value::String("tail".to_string()));
    obj.insert(
        "n".to_string(),
        Value::Number(serde_json::Number::from(n as u64)),
    );
    obj.remove("start_line");
    obj.remove("end_line");
    obj.remove("line_count");
    obj.remove("count");
    obj.remove("limit");
    true
}

fn has_non_empty_json_value(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => items.iter().any(has_non_empty_json_value),
        Value::Object(map) => !map.is_empty(),
        _ => true,
    }
}

fn normalize_inventory_dir_sort_by_value(obj: &serde_json::Map<String, Value>) -> Option<String> {
    let sort_by = obj
        .get("sort_by")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_ascii_lowercase();
    let descending = obj
        .get("order")
        .or_else(|| obj.get("sort_order"))
        .or_else(|| obj.get("direction"))
        .and_then(Value::as_str)
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !matches!(value.as_str(), "asc" | "ascending")
        })
        .unwrap_or(true);
    match sort_by.as_str() {
        "mtime_desc" | "mtime_asc" | "size_desc" | "size_asc" | "name" => Some(sort_by),
        "mtime" | "modified" | "modified_ts" | "modified_time" => Some(if descending {
            "mtime_desc".to_string()
        } else {
            "mtime_asc".to_string()
        }),
        "size" | "size_bytes" | "bytes" => Some(if descending {
            "size_desc".to_string()
        } else {
            "size_asc".to_string()
        }),
        _ => None,
    }
}

fn normalize_read_range_line_aliases(obj: &mut serde_json::Map<String, Value>) {
    normalize_arg_alias(obj, "start_line", &["line_start", "from_line"]);
    normalize_arg_alias(obj, "end_line", &["line_end", "to_line"]);
    if obj.get("start_line").is_some_and(has_non_empty_json_value)
        && obj.get("end_line").is_some_and(has_non_empty_json_value)
    {
        obj.entry("mode".to_string())
            .or_insert_with(|| Value::String("range".to_string()));
    }
    let Some(range_value) = obj
        .remove("lines")
        .or_else(|| obj.remove("line_range"))
        .or_else(|| obj.remove("range"))
    else {
        return;
    };
    if let Some(mode) = range_value
        .as_str()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .filter(|mode| matches!(mode.as_str(), "head" | "tail" | "full" | "all"))
    {
        obj.insert("mode".to_string(), Value::String(mode));
        return;
    }
    let Some((start, end)) = parse_line_range_value(&range_value) else {
        return;
    };
    obj.insert("mode".to_string(), Value::String("range".to_string()));
    obj.insert(
        "start_line".to_string(),
        Value::Number(serde_json::Number::from(start as u64)),
    );
    obj.insert(
        "end_line".to_string(),
        Value::Number(serde_json::Number::from(end as u64)),
    );
    obj.entry("n".to_string()).or_insert_with(|| {
        Value::Number(serde_json::Number::from(
            end.saturating_sub(start).saturating_add(1) as u64,
        ))
    });
}

fn normalize_path_alias_to_path(obj: &mut serde_json::Map<String, Value>, aliases: &[&str]) {
    if obj.get("path").is_some_and(has_non_empty_json_value) {
        return;
    }
    for alias in aliases {
        let Some(value) = obj.remove(*alias) else {
            continue;
        };
        if has_non_empty_json_value(&value) {
            obj.insert("path".to_string(), value);
            return;
        }
    }
}

fn normalize_arg_alias(
    obj: &mut serde_json::Map<String, Value>,
    canonical: &str,
    aliases: &[&str],
) {
    if obj.get(canonical).is_some_and(has_non_empty_json_value) {
        return;
    }
    for alias in aliases {
        let Some(value) = obj.remove(*alias) else {
            continue;
        };
        if has_non_empty_json_value(&value) {
            obj.insert(canonical.to_string(), value);
            return;
        }
    }
}

fn normalize_path_batch_facts_args(obj: &mut serde_json::Map<String, Value>) {
    if obj.contains_key("paths") {
        return;
    }
    if let Some(paths) = obj
        .remove("targets")
        .or_else(|| obj.remove("target_paths"))
        .or_else(|| obj.remove("path_list"))
        .or_else(|| obj.remove("path_array"))
    {
        obj.insert("paths".to_string(), paths);
    } else if let Some(path) = obj.remove("path") {
        obj.insert("paths".to_string(), Value::Array(vec![path]));
    }
}

fn normalize_system_basic_args(mut args: Value) -> Value {
    let Some(obj) = args.as_object_mut() else {
        return args;
    };
    let action_name = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    match action_name.as_str() {
        "read" | "read_file" => {
            obj.insert(
                "action".to_string(),
                Value::String("read_range".to_string()),
            );
            normalize_read_range_line_aliases(obj);
            normalize_read_range_negative_start_count(obj);
            normalize_read_range_negative_bounds(obj);
            normalize_read_range_line_count_template(obj);
        }
        "read_range" => {
            normalize_read_range_line_aliases(obj);
            normalize_read_range_negative_start_count(obj);
            normalize_read_range_negative_bounds(obj);
            normalize_read_range_line_count_template(obj);
        }
        "list" | "list_dir" | "ls" => {
            obj.insert(
                "action".to_string(),
                Value::String("inventory_dir".to_string()),
            );
            normalize_path_alias_to_path(obj, &["dir_path", "directory_path", "directory", "dir"]);
            obj.entry("names_only".to_string())
                .or_insert_with(|| Value::Bool(true));
        }
        "count_dir" | "count_directory" | "count_children" | "count_entries" | "count_items"
        | "directory_count" | "dir_count" | "inventory_count" => {
            obj.insert(
                "action".to_string(),
                Value::String("count_inventory".to_string()),
            );
            normalize_path_alias_to_path(obj, &["dir_path", "directory_path", "directory", "dir"]);
        }
        "count_inventory" => {
            normalize_path_alias_to_path(obj, &["dir_path", "directory_path", "directory", "dir"]);
        }
        "check_exists" | "exists" | "path_exists" | "stat_paths" => {
            obj.insert(
                "action".to_string(),
                Value::String("path_batch_facts".to_string()),
            );
            normalize_path_alias_to_path(
                obj,
                &[
                    "target",
                    "target_path",
                    "file",
                    "file_path",
                    "dir_path",
                    "directory_path",
                    "directory",
                    "dir",
                ],
            );
            normalize_path_batch_facts_args(obj);
        }
        "path_batch_facts" => {
            normalize_path_batch_facts_args(obj);
        }
        "find_name" => {
            obj.insert("action".to_string(), Value::String("find_path".to_string()));
            normalize_arg_alias(obj, "name", &["pattern", "query", "target", "keyword"]);
        }
        "find_path" => {
            normalize_arg_alias(obj, "name", &["query", "target", "keyword", "name_pattern"]);
        }
        "inventory_dir" => {
            normalize_path_alias_to_path(obj, &["dir_path", "directory_path", "directory", "dir"]);
            let has_extension_filter = obj.get("ext_filter").is_some_and(has_non_empty_json_value)
                || obj.get("extension").is_some_and(has_non_empty_json_value)
                || obj.get("extensions").is_some_and(has_non_empty_json_value);
            let dirs_only = obj
                .get("dirs_only")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if has_extension_filter && !dirs_only {
                obj.insert("files_only".to_string(), Value::Bool(true));
            }
            if !obj.contains_key("ext_filter") {
                if let Some(value) = obj.remove("extension").or_else(|| obj.remove("extensions")) {
                    obj.insert("ext_filter".to_string(), value);
                }
            }
            if let Some(sort_by) = normalize_inventory_dir_sort_by_value(obj) {
                obj.insert("sort_by".to_string(), Value::String(sort_by));
            }
        }
        "compare_paths" => {
            if obj.contains_key("left_path") && obj.contains_key("right_path") {
                return args;
            }
            if !obj.contains_key("left_path") {
                if let Some(value) = obj
                    .remove("path1")
                    .or_else(|| obj.remove("left"))
                    .or_else(|| obj.remove("source_path"))
                    .or_else(|| obj.remove("first_path"))
                {
                    obj.insert("left_path".to_string(), value);
                }
            }
            if !obj.contains_key("right_path") {
                if let Some(value) = obj
                    .remove("path2")
                    .or_else(|| obj.remove("right"))
                    .or_else(|| obj.remove("target_path"))
                    .or_else(|| obj.remove("second_path"))
                {
                    obj.insert("right_path".to_string(), value);
                }
            }
            if obj.contains_key("left_path") && obj.contains_key("right_path") {
                return args;
            }
            let paths = string_list_from_value(obj.get("paths"))
                .into_iter()
                .chain(string_list_from_value(obj.get("targets")))
                .collect::<Vec<_>>();
            if paths.len() >= 2 {
                obj.entry("left_path".to_string())
                    .or_insert_with(|| Value::String(paths[0].clone()));
                obj.entry("right_path".to_string())
                    .or_insert_with(|| Value::String(paths[1].clone()));
            }
        }
        _ => {}
    }
    args
}

fn normalize_fs_basic_args_for_planner(mut args: Value) -> Value {
    let Some(obj) = args.as_object_mut() else {
        return args;
    };
    let action_name = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    match action_name.as_str() {
        "read_text_range" => {
            normalize_read_range_line_aliases(obj);
            normalize_read_range_negative_start_count(obj);
            normalize_read_range_negative_bounds(obj);
            normalize_read_range_line_count_template(obj);
        }
        "append_text" => {
            normalize_path_alias_to_path(obj, &["file", "file_path", "target"]);
            normalize_arg_alias(obj, "content", &["text", "data", "body", "line"]);
        }
        "write_text" => {
            normalize_path_alias_to_path(obj, &["file", "file_path", "target"]);
            normalize_arg_alias(obj, "content", &["text", "data", "body"]);
        }
        "grep_text" => {
            if obj
                .get("case_sensitive")
                .and_then(Value::as_bool)
                .is_some_and(|case_sensitive| !case_sensitive)
            {
                obj.entry("case_insensitive".to_string())
                    .or_insert(Value::Bool(true));
            }
            normalize_arg_alias(obj, "max_results", &["max_matches", "limit"]);
        }
        _ => {}
    }
    args
}

fn normalize_fs_basic_schema_aliases(actions: Vec<AgentAction>) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args } if skill.eq_ignore_ascii_case("fs_basic") => {
                AgentAction::CallSkill {
                    skill,
                    args: normalize_fs_basic_args_for_planner(args),
                }
            }
            AgentAction::CallTool { tool, args } if tool.eq_ignore_ascii_case("fs_basic") => {
                AgentAction::CallTool {
                    tool,
                    args: normalize_fs_basic_args_for_planner(args),
                }
            }
            other => other,
        })
        .collect()
}

fn normalize_system_basic_schema_aliases(actions: Vec<AgentAction>) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args } if skill == "system_basic" => {
                AgentAction::CallSkill {
                    skill,
                    args: normalize_system_basic_args(args),
                }
            }
            other => other,
        })
        .collect()
}

fn normalize_git_basic_args(mut args: Value, route_result: Option<&RouteResult>) -> Value {
    let Some(obj) = args.as_object_mut() else {
        return args;
    };
    let action_name = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .replace('-', "_");

    let normalized = match action_name.as_str() {
        "branches" | "list_branches" | "all_branches" => {
            if route_result.is_some_and(|route| {
                matches!(
                    route.output_contract.response_shape,
                    crate::OutputResponseShape::Scalar
                )
            }) {
                "current_branch"
            } else {
                "branch"
            }
        }
        "current_branch_name" | "branch_current" | "get_current_branch" => "current_branch",
        "cached_diff" | "staged_diff" => "diff_cached",
        "changed_file" | "changed_file_names" => "changed_files",
        "revparse" | "head" => "rev_parse",
        _ => return args,
    };
    obj.insert("action".to_string(), Value::String(normalized.to_string()));
    info!(
        "plan_normalize_git_basic_action_alias action={} normalized={}",
        action_name, normalized
    );
    args
}

fn normalize_git_basic_schema_aliases(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args } if skill.eq_ignore_ascii_case("git_basic") => {
                AgentAction::CallSkill {
                    skill,
                    args: normalize_git_basic_args(args, route_result),
                }
            }
            other => other,
        })
        .collect()
}

fn git_repository_state_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.has_tool_or_skill_output
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::GitRepositoryState
    {
        return None;
    }
    let action = git_repository_state_action_from_text(user_text).unwrap_or("status");
    Some(build_plan_result(
        goal,
        "deterministic:git_repository_state",
        PlanKind::Single,
        &[AgentAction::CallSkill {
            skill: "git_basic".to_string(),
            args: serde_json::json!({ "action": action }),
        }],
    ))
}

fn git_repository_state_action_from_text(user_text: &str) -> Option<&'static str> {
    if structural_token_present(user_text, "remote") {
        return Some("remote");
    }
    if structural_token_present(user_text, "status") {
        return Some("status");
    }
    if structural_token_present(user_text, "HEAD") {
        return Some("rev_parse");
    }
    if structural_token_present(user_text, "branch") {
        return Some("branch");
    }
    None
}

fn recent_scalar_current_workspace_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || route.needs_clarify
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind
            != crate::OutputSemanticKind::RecentScalarEqualityCheck
        || route.output_contract.locator_kind != crate::OutputLocatorKind::CurrentWorkspace
        || !git_basic_available_for_plan(state)
    {
        return None;
    }
    let probe = AgentAction::CallSkill {
        skill: "git_basic".to_string(),
        args: serde_json::json!({ "action": "current_branch" }),
    };
    let AgentAction::CallSkill { skill, args } = &probe else {
        return None;
    };
    if !crate::contract_matrix::action_policy_for_output_contract(
        Some(&route.output_contract),
        skill,
        args,
    )
    .is_some_and(|policy| policy.is_allowed())
    {
        return None;
    }
    let actions = vec![
        probe,
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn recent_scalar_file_pair_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || route.needs_clarify
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind
            != crate::OutputSemanticKind::RecentScalarEqualityCheck
    {
        return None;
    }

    let mut targets = structured_or_text_multi_file_targets(route, user_text);
    if let Some(path) = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        targets.push(path.to_string());
    }
    targets.dedup_by(|left, right| left.eq_ignore_ascii_case(right));

    let resolved_targets = targets
        .iter()
        .filter_map(|target| {
            resolve_existing_metadata_locator_path(&state.skill_rt.workspace_root, target)
        })
        .fold(Vec::<String>::new(), |mut out, path| {
            if !out
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&path))
            {
                out.push(path);
            }
            out
        });
    if resolved_targets.len() < 2 {
        return None;
    }

    let mut structured_read: Option<AgentAction> = None;
    let mut text_query: Option<String> = None;
    for path in resolved_targets
        .iter()
        .filter(|path| path_has_structured_document_extension(path))
    {
        let selectors = structured_current_turn_field_selectors(route, user_text, Some(path));
        let Some(field_path) = selectors
            .into_iter()
            .find(|field| structured_field_selector_can_yield_scalar(state, path, field))
        else {
            continue;
        };
        text_query = structured_field_leaf_query(&field_path);
        structured_read = Some(config_basic_read_field_action(path.clone(), field_path));
        break;
    }
    let structured_read = structured_read?;
    let query = text_query?;
    let text_path = resolved_targets
        .iter()
        .find(|path| !path_has_structured_document_extension(path) && Path::new(path).is_file())?
        .clone();

    let actions = vec![
        structured_read,
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "grep_text",
                "path": text_path,
                "query": query,
                "case_insensitive": true,
                "max_results": 8,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let actions = normalize_planned_actions_with_original_and_context(
        state,
        route_result,
        loop_state,
        user_text,
        original_user_text,
        Some(goal),
        auto_locator_path,
        actions,
    );
    if actions
        .iter()
        .filter_map(|action| match action {
            AgentAction::CallSkill { skill, args } => Some((skill.as_str(), args)),
            AgentAction::CallTool { tool, args } => Some((tool.as_str(), args)),
            _ => None,
        })
        .any(|(skill, args)| {
            !crate::contract_matrix::action_policy_for_output_contract(
                Some(&route.output_contract),
                skill,
                args,
            )
            .is_some_and(|policy| policy.is_allowed())
        })
    {
        return None;
    }
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn structured_field_selector_can_yield_scalar(
    state: &AppState,
    path: &str,
    field_path: &str,
) -> bool {
    if structured_field_path_resolves_scalar_value(path, field_path) {
        return true;
    }
    let current = resolve_workspace_path(&state.skill_rt.workspace_root, path);
    resolve_cargo_workspace_package_fields(
        &state.skill_rt.workspace_root,
        &current,
        &[field_path.to_string()],
    )
    .is_some_and(|(target, fields)| {
        fields.len() == 1
            && structured_field_path_resolves_scalar_value(
                target.to_string_lossy().as_ref(),
                &fields[0],
            )
    })
}

fn structured_field_leaf_query(field_path: &str) -> Option<String> {
    field_path
        .split('.')
        .next_back()
        .map(str::trim)
        .filter(|leaf| schema_field_token_is_valid(leaf))
        .map(ToString::to_string)
}

fn service_status_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.has_tool_or_skill_output
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || !route_requests_service_status(route)
    {
        return None;
    }
    if route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && health_check_available_for_plan(state)
    {
        let action = AgentAction::CallSkill {
            skill: "health_check".to_string(),
            args: serde_json::json!({}),
        };
        if let AgentAction::CallSkill { skill, args } = &action {
            if crate::contract_matrix::action_policy_for_output_contract(
                Some(&route.output_contract),
                skill,
                args,
            )
            .is_some_and(|policy| policy.is_allowed())
            {
                return Some(build_plan_result(
                    goal,
                    "deterministic:service_status_scalar_health_check",
                    PlanKind::Single,
                    &[action],
                ));
            }
        }
    }
    if health_check_available_for_plan(state)
        && route_reason_has_marker(route, "execution_recipe_health_check_observation")
    {
        return Some(build_plan_result(
            goal,
            "deterministic:service_status_health_check_recipe",
            PlanKind::Single,
            &[AgentAction::CallSkill {
                skill: "health_check".to_string(),
                args: serde_json::json!({}),
            }],
        ));
    }
    if health_check_available_for_plan(state)
        && request_mentions_workspace_product(state, user_text)
    {
        return Some(build_plan_result(
            goal,
            "deterministic:service_status_health_check",
            PlanKind::Single,
            &[AgentAction::CallSkill {
                skill: "health_check".to_string(),
                args: serde_json::json!({}),
            }],
        ));
    }
    if process_basic_available_for_plan(state) {
        if let Some(port) = first_port_filter_token(user_text) {
            return Some(build_plan_result(
                goal,
                "deterministic:service_status_port_list",
                PlanKind::Single,
                &[AgentAction::CallSkill {
                    skill: "process_basic".to_string(),
                    args: serde_json::json!({
                        "action": "port_list",
                        "filter": port,
                    }),
                }],
            ));
        }
        if let Some(filter) = process_status_filter_token(user_text) {
            return Some(build_plan_result(
                goal,
                "deterministic:service_status_process_list",
                PlanKind::Single,
                &[AgentAction::CallSkill {
                    skill: "process_basic".to_string(),
                    args: serde_json::json!({
                        "action": "ps",
                        "limit": 200,
                        "filter": filter,
                    }),
                }],
            ));
        }
    }
    if system_basic_available_for_plan(state) && !process_basic_available_for_plan(state) {
        let action = AgentAction::CallTool {
            tool: "system_basic".to_string(),
            args: serde_json::json!({"action":"info"}),
        };
        if let AgentAction::CallTool { tool: skill, args } = &action {
            if crate::contract_matrix::action_policy_for_output_contract(
                Some(&route.output_contract),
                skill,
                args,
            )
            .is_some_and(|policy| policy.is_allowed())
            {
                return Some(build_plan_result(
                    goal,
                    "deterministic:service_status_system_info",
                    PlanKind::Single,
                    &[action],
                ));
            }
        }
    }
    None
}

fn runtime_status_query_kind(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> Option<&str> {
    turn_analysis
        .filter(|analysis| analysis.turn_type == Some(crate::intent_router::TurnType::StatusQuery))
        .and_then(|analysis| analysis.state_patch.as_ref())
        .and_then(|patch| patch.get("runtime_status_query"))
        .and_then(Value::as_object)
        .and_then(|query| query.get("kind"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
}

fn runtime_status_query_system_basic_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "current_user" => Some("current_user"),
        "host_name" => Some("host_name"),
        "current_working_directory" | "current_process_cwd" | "process_cwd" => {
            Some("current_working_directory")
        }
        _ => None,
    }
}

fn runtime_status_scalar_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.has_tool_or_skill_output
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::RawCommandOutput
        || route.output_contract.delivery_required
        || !system_basic_available_for_plan(state)
    {
        return None;
    }
    let kind = runtime_status_query_kind(turn_analysis)
        .and_then(runtime_status_query_system_basic_kind)?;
    let action = AgentAction::CallTool {
        tool: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "runtime_status",
            "kind": kind,
        }),
    };
    if let AgentAction::CallTool { tool: skill, args } = &action {
        if !crate::contract_matrix::action_policy_for_output_contract(
            Some(&route.output_contract),
            skill,
            args,
        )
        .is_some_and(|policy| policy.is_allowed())
        {
            return None;
        }
    }
    Some(build_plan_result(
        goal,
        "deterministic:runtime_status_scalar_system_basic",
        PlanKind::Single,
        &[action],
    ))
}

fn route_reason_has_marker(route: &RouteResult, marker: &str) -> bool {
    route
        .route_reason
        .split(';')
        .any(|part| part.trim() == marker)
}

fn runtime_status_scalar_info_fallback_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> Option<PlanResult> {
    let _ = (state, goal, route_result, loop_state, turn_analysis);
    None
}

fn request_mentions_workspace_product(state: &AppState, text: &str) -> bool {
    let Some(product) = state
        .skill_rt
        .workspace_root
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')))
        .any(|token| token.eq_ignore_ascii_case(product))
}

fn first_port_filter_token(text: &str) -> Option<String> {
    text.split_whitespace()
        .find_map(port_filter_from_structural_token)
}

fn process_status_filter_token(text: &str) -> Option<String> {
    let candidates = text
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')))
        .filter_map(|token| {
            let token = token.trim_matches(|ch: char| matches!(ch, '.' | '-' | '_'));
            if safe_process_status_filter_token(token) {
                Some(token.to_string())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return None;
    }
    if let Some(active) = candidates
        .iter()
        .find(|token| process_table_contains_filter_token(token))
    {
        return Some(active.clone());
    }
    if let Some(structural) = candidates.iter().find(|token| {
        token.contains(['_', '-', '.'])
            || token.chars().any(|ch| ch.is_ascii_digit())
            || (token.chars().any(|ch| ch.is_ascii_uppercase())
                && !token_is_ascii_uppercase_acronym(token))
    }) {
        return Some(structural.clone());
    }
    (candidates.len() == 1 && !token_is_ascii_uppercase_acronym(&candidates[0]))
        .then(|| candidates[0].clone())
}

fn token_is_ascii_uppercase_acronym(token: &str) -> bool {
    let mut saw_alpha = false;
    for ch in token.chars() {
        if ch.is_ascii_alphabetic() {
            saw_alpha = true;
            if !ch.is_ascii_uppercase() {
                return false;
            }
        } else {
            return false;
        }
    }
    saw_alpha
}

fn port_filter_from_structural_token(token: &str) -> Option<String> {
    let trimmed = token.trim_matches(|ch: char| {
        matches!(
            ch,
            ',' | ';' | ')' | '(' | '[' | ']' | '{' | '}' | '"' | '\''
        )
    });
    let numeric = trimmed
        .parse::<u16>()
        .ok()
        .filter(|port| *port >= 1024)
        .map(|port| port.to_string());
    if numeric.is_some() {
        return numeric;
    }
    let (_, port_part) = trimmed.rsplit_once(':')?;
    port_part
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .map(|port| port.to_string())
}

fn safe_process_status_filter_token(token: &str) -> bool {
    token.len() >= 3
        && !token.chars().all(|ch| ch.is_ascii_digit())
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

#[cfg(target_os = "linux")]
fn process_table_contains_filter_token(token: &str) -> bool {
    let token = token.trim();
    if token.is_empty() {
        return false;
    }
    let token_lower = token.to_ascii_lowercase();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        let dir = entry.path();
        if std::fs::read_to_string(dir.join("comm"))
            .ok()
            .map(|comm| comm.trim().to_ascii_lowercase())
            .is_some_and(|comm| comm == token_lower)
        {
            return true;
        }
        if std::fs::read(dir.join("cmdline"))
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .is_some_and(|cmdline| {
                cmdline.split('\0').any(|arg| {
                    let base = command_basename(arg).to_ascii_lowercase();
                    base == token_lower
                })
            })
        {
            return true;
        }
    }
    false
}

#[cfg(not(target_os = "linux"))]
fn process_table_contains_filter_token(_token: &str) -> bool {
    false
}

fn route_locator_hint_for_read_range_fill(route_result: Option<&RouteResult>) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
    {
        return None;
    }
    if !matches!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
    ) {
        return None;
    }
    route
        .output_contract
        .locator_hint
        .trim()
        .split('|')
        .next()
        .map(str::trim)
        .filter(|hint| !hint.is_empty())
        .map(ToString::to_string)
}

fn is_system_basic_read_range_missing_path(action: &AgentAction) -> bool {
    matches!(
        action,
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("system_basic")
                && args
                    .get("action")
                    .and_then(Value::as_str)
                    .is_some_and(|action| action.eq_ignore_ascii_case("read_range"))
                && args
                    .get("path")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .is_none_or(|path| path.is_empty())
    )
}

fn fill_missing_read_range_path_from_route_locator(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(locator_hint) = route_locator_hint_for_read_range_fill(route_result) else {
        return actions;
    };
    let missing_indices = actions
        .iter()
        .enumerate()
        .filter_map(|(idx, action)| is_system_basic_read_range_missing_path(action).then_some(idx))
        .collect::<Vec<_>>();
    if missing_indices.len() != 1 {
        return actions;
    }

    let mut rewritten = actions;
    let Some(action) = rewritten.get_mut(missing_indices[0]) else {
        return rewritten;
    };
    match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => {
            if let Some(obj) = args.as_object_mut() {
                obj.insert("path".to_string(), Value::String(locator_hint.clone()));
            }
        }
        AgentAction::CallCapability { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => {}
    }
    info!(
        "plan_fill_missing_read_range_path_from_route_locator idx={} path={}",
        missing_indices[0], locator_hint
    );
    rewritten
}

fn bool_arg_any(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter()
        .any(|key| obj.get(*key).and_then(Value::as_bool).unwrap_or(false))
}

fn structured_directory_filter_requested(obj: &serde_json::Map<String, Value>) -> bool {
    bool_arg_any(
        obj,
        &[
            "dirs_only",
            "directories_only",
            "directory_only",
            "folders_only",
        ],
    ) || string_arg_any_matches(
        obj,
        &["kind_filter", "kind", "entry_type", "target_kind"],
        &[
            "dir",
            "dirs",
            "directory",
            "directories",
            "folder",
            "folders",
        ],
    )
}

fn structured_file_filter_requested(obj: &serde_json::Map<String, Value>) -> bool {
    bool_arg_any(obj, &["files_only", "file_only"])
        || string_arg_any_matches(
            obj,
            &["kind_filter", "kind", "entry_type", "target_kind"],
            &["file", "files"],
        )
}

fn string_arg_any_matches(
    obj: &serde_json::Map<String, Value>,
    keys: &[&str],
    values: &[&str],
) -> bool {
    keys.iter().any(|key| {
        obj.get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|raw| values.iter().any(|value| raw.eq_ignore_ascii_case(value)))
    })
}

fn normalize_schema_token(raw: &str) -> String {
    raw.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if matches!(ch, '-' | ' ' | '.') {
                '_'
            } else {
                ch
            }
        })
        .collect()
}

fn list_dir_args_need_inventory_dir(
    state: &AppState,
    route_result: Option<&RouteResult>,
    obj: &serde_json::Map<String, Value>,
) -> bool {
    let Some(manifest) = state.skill_manifest("list_dir") else {
        return route_result.is_some_and(|route| {
            matches!(
                route.output_contract.semantic_kind,
                crate::OutputSemanticKind::DirectoryNames | crate::OutputSemanticKind::FileNames
            )
        }) || obj.contains_key("dirs_only")
            || obj.contains_key("directories_only")
            || obj.contains_key("directory_only")
            || obj.contains_key("folders_only")
            || obj.contains_key("files_only")
            || obj.contains_key("kind_filter")
            || obj.contains_key("kind")
            || obj.contains_key("entry_type")
            || obj.contains_key("include_hidden")
            || obj.contains_key("sort_by")
            || obj.contains_key("ext_filter")
            || obj.contains_key("extension")
            || obj.contains_key("extensions");
    };
    let route_semantic = route_result
        .map(|route| route.output_contract.semantic_kind.as_str())
        .unwrap_or("none");
    if manifest
        .runtime_rewrite_semantic_kinds
        .iter()
        .any(|kind| kind == route_semantic)
    {
        return true;
    }
    obj.keys().any(|key| {
        let normalized = normalize_schema_token(key);
        manifest
            .runtime_rewrite_arg_keys
            .iter()
            .any(|candidate| candidate == &normalized)
    })
}

fn list_dir_runtime_mapping_from_registry(
    state: &AppState,
) -> (String, String, Option<serde_json::Value>) {
    let Some(manifest) = state.skill_manifest("list_dir") else {
        return (
            "system_basic".to_string(),
            "inventory_dir".to_string(),
            Some(serde_json::json!({"names_only": true})),
        );
    };
    let runtime_skill = manifest
        .runtime_skill
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("system_basic")
        .to_string();
    let runtime_action = manifest
        .runtime_action
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("inventory_dir")
        .to_string();
    (runtime_skill, runtime_action, manifest.runtime_default_args)
}

fn merge_default_args(obj: &mut serde_json::Map<String, Value>, defaults: Option<Value>) {
    let Some(defaults) = defaults.and_then(|value| value.as_object().cloned()) else {
        return;
    };
    for (key, value) in defaults {
        obj.entry(key).or_insert(value);
    }
}

fn inventory_dir_args_from_list_dir_args(
    state: &AppState,
    route_result: Option<&RouteResult>,
    args: Value,
) -> Option<(String, Value)> {
    let mut obj = args.as_object()?.clone();
    if !list_dir_args_need_inventory_dir(state, route_result, &obj) {
        return None;
    }
    let (runtime_skill, runtime_action, default_args) =
        list_dir_runtime_mapping_from_registry(state);
    merge_default_args(&mut obj, default_args);
    normalize_path_alias_to_path(
        &mut obj,
        &["dir_path", "directory_path", "directory", "dir"],
    );
    obj.insert("action".to_string(), Value::String(runtime_action));
    let route_semantic = route_result.map(|route| route.output_contract.semantic_kind);
    let directory_filter_requested = structured_directory_filter_requested(&obj);
    let file_filter_requested = structured_file_filter_requested(&obj);
    let mut dirs_only = directory_filter_requested
        || (route_semantic == Some(crate::OutputSemanticKind::DirectoryNames)
            && file_filter_requested);
    let mut files_only =
        route_semantic == Some(crate::OutputSemanticKind::FileNames) || file_filter_requested;
    if dirs_only {
        files_only = false;
    } else if files_only {
        dirs_only = false;
    }
    obj.insert("dirs_only".to_string(), Value::Bool(dirs_only));
    obj.insert("files_only".to_string(), Value::Bool(files_only));
    if dirs_only
        || files_only
        || matches!(
            route_semantic,
            Some(crate::OutputSemanticKind::DirectoryNames | crate::OutputSemanticKind::FileNames)
        )
    {
        obj.insert("names_only".to_string(), Value::Bool(true));
    } else {
        obj.entry("names_only".to_string())
            .or_insert_with(|| Value::Bool(true));
    }
    if matches!(
        route_semantic,
        Some(crate::OutputSemanticKind::DirectoryNames | crate::OutputSemanticKind::FileNames)
    ) {
        obj.insert("include_hidden".to_string(), Value::Bool(false));
    }
    for key in [
        "directories_only",
        "directory_only",
        "folders_only",
        "file_only",
        "kind_filter",
        "kind",
        "entry_type",
        "extension",
        "extensions",
    ] {
        if key == "extension" || key == "extensions" {
            if !obj.contains_key("ext_filter") {
                if let Some(value) = obj.remove(key) {
                    obj.insert("ext_filter".to_string(), value);
                }
                continue;
            }
        }
        obj.remove(key);
    }
    Some((runtime_skill, Value::Object(obj)))
}

fn rewrite_filtered_list_dir_to_inventory_dir(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args } if skill.eq_ignore_ascii_case("list_dir") => {
                if let Some((runtime_skill, args)) =
                    inventory_dir_args_from_list_dir_args(state, route_result, args.clone())
                {
                    info!(
                        "plan_rewrite_list_dir_to_inventory_dir runtime_skill={}",
                        runtime_skill
                    );
                    AgentAction::CallSkill {
                        skill: runtime_skill,
                        args,
                    }
                } else {
                    AgentAction::CallSkill { skill, args }
                }
            }
            AgentAction::CallTool { tool, args } if tool.eq_ignore_ascii_case("list_dir") => {
                if let Some((runtime_skill, args)) =
                    inventory_dir_args_from_list_dir_args(state, route_result, args.clone())
                {
                    info!(
                        "plan_rewrite_list_dir_to_inventory_dir runtime_skill={}",
                        runtime_skill
                    );
                    AgentAction::CallTool {
                        tool: runtime_skill,
                        args,
                    }
                } else {
                    AgentAction::CallTool { tool, args }
                }
            }
            other => other,
        })
        .collect()
}

fn normalize_doc_parse_args(mut args: Value) -> Value {
    let Some(obj) = args.as_object_mut() else {
        return args;
    };
    let action_name = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(action_name.as_str(), "parse_doc" | "parse") {
        obj.insert("action".to_string(), Value::String("parse_doc".to_string()));
    }
    normalize_path_alias_to_path(obj, &["file", "file_path", "document", "document_path"]);
    args
}

fn normalize_doc_parse_schema_aliases(actions: Vec<AgentAction>) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args } if skill == "doc_parse" => {
                AgentAction::CallSkill {
                    skill,
                    args: normalize_doc_parse_args(args),
                }
            }
            other => other,
        })
        .collect()
}

fn normalize_transform_args(mut args: Value) -> Value {
    let Some(obj) = args.as_object_mut() else {
        return args;
    };
    let action_name = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .replace('-', "_");
    if !matches!(action_name.as_str(), "transform_data") {
        obj.insert(
            "action".to_string(),
            Value::String("transform_data".to_string()),
        );
        info!(
            "plan_normalize_transform_action_alias action={} normalized=transform_data",
            action_name
        );
    }
    normalize_arg_alias(obj, "data", &["records", "items", "rows", "array"]);
    if !obj.contains_key("ops") {
        let sort_by = obj
            .remove("sort_by")
            .or_else(|| obj.remove("sort_field"))
            .or_else(|| obj.remove("order_by"))
            .or_else(|| obj.remove("by"));
        if let Some(sort_by) = sort_by.filter(has_non_empty_json_value) {
            let mut op = serde_json::Map::new();
            op.insert("op".to_string(), Value::String("sort".to_string()));
            op.insert("by".to_string(), sort_by);
            if let Some(order) = obj
                .remove("order")
                .or_else(|| obj.remove("sort_order"))
                .filter(has_non_empty_json_value)
            {
                op.insert("order".to_string(), order);
            }
            obj.insert("ops".to_string(), Value::Array(vec![Value::Object(op)]));
            info!("plan_normalize_transform_sort_alias_to_ops");
        }
    }
    normalize_transform_ops(obj);
    args
}

fn normalize_transform_ops(obj: &mut serde_json::Map<String, Value>) {
    let Some(ops) = obj.get_mut("ops").and_then(Value::as_array_mut) else {
        return;
    };
    for op in ops {
        let Some(op_obj) = op.as_object_mut() else {
            continue;
        };
        let op_name = op_obj
            .get("op")
            .and_then(Value::as_str)
            .map(|name| name.trim().to_ascii_lowercase())
            .unwrap_or_default();
        if op_name == "filter" {
            normalize_transform_filter_op(op_obj);
        }
    }
}

fn normalize_transform_filter_op(op: &mut serde_json::Map<String, Value>) {
    let Some(where_obj) = op.get("where").and_then(Value::as_object).cloned() else {
        return;
    };
    if !op.contains_key("field") {
        if let Some(field) = where_obj
            .get("field")
            .or_else(|| where_obj.get("path"))
            .filter(|value| has_non_empty_json_value(value))
            .cloned()
        {
            op.insert("field".to_string(), field);
        }
    }
    if !op.contains_key("path") {
        if let Some(path) = where_obj
            .get("path")
            .filter(|value| has_non_empty_json_value(value))
            .cloned()
        {
            op.insert("path".to_string(), path);
        }
    }
    if !op.contains_key("cmp") {
        if let Some(cmp) = where_obj
            .get("cmp")
            .or_else(|| where_obj.get("operator"))
            .and_then(Value::as_str)
            .and_then(normalize_transform_cmp_alias)
        {
            op.insert("cmp".to_string(), Value::String(cmp.to_string()));
        } else if let Some(cmp) = transform_where_comparator_key(&where_obj) {
            op.insert("cmp".to_string(), Value::String(cmp.to_string()));
        }
    }
    if !op.contains_key("value") {
        if let Some(value) = where_obj
            .get("value")
            .filter(|value| has_non_empty_json_value(value))
            .cloned()
        {
            op.insert("value".to_string(), value);
        } else if let Some(cmp_key) = transform_where_comparator_key(&where_obj) {
            if let Some(value) = where_obj.get(cmp_key).cloned() {
                op.insert("value".to_string(), value);
            }
        }
    }
}

fn transform_where_comparator_key(where_obj: &serde_json::Map<String, Value>) -> Option<&str> {
    where_obj
        .keys()
        .filter_map(|key| normalize_transform_cmp_alias(key).map(|cmp| (key.as_str(), cmp)))
        .find(|(key, _)| !matches!(*key, "field" | "path" | "cmp" | "operator" | "value" | "op"))
        .map(|(key, _)| key)
}

fn normalize_transform_cmp_alias(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "eq" | "equal" | "equals" => Some("eq"),
        "ne" | "neq" | "not_eq" | "not_equals" => Some("ne"),
        "gt" => Some("gt"),
        "gte" | "ge" => Some("gte"),
        "lt" => Some("lt"),
        "lte" | "le" => Some("lte"),
        "contains" => Some("contains"),
        "in" => Some("in"),
        "exists" => Some("exists"),
        _ => None,
    }
}

fn normalize_transform_schema_aliases(actions: Vec<AgentAction>) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args } if skill.eq_ignore_ascii_case("transform") => {
                AgentAction::CallSkill {
                    skill,
                    args: normalize_transform_args(args),
                }
            }
            AgentAction::CallTool { tool, args } if tool.eq_ignore_ascii_case("transform") => {
                AgentAction::CallTool {
                    tool,
                    args: normalize_transform_args(args),
                }
            }
            other => other,
        })
        .collect()
}

fn normalize_archive_basic_args(mut args: Value, route_result: Option<&RouteResult>) -> Value {
    let Some(obj) = args.as_object_mut() else {
        return args;
    };
    let mut action_name = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(action_name.as_str(), "list" | "read" | "pack" | "unpack")
        && unknown_archive_action_can_normalize_to_list(obj, route_result)
    {
        obj.insert("action".to_string(), Value::String("list".to_string()));
        action_name = "list".to_string();
    }
    match action_name.as_str() {
        "pack" => {
            normalize_arg_alias(
                obj,
                "source",
                &["source_path", "src", "input", "input_path"],
            );
            normalize_arg_alias(
                obj,
                "archive",
                &[
                    "output",
                    "archive_path",
                    "target",
                    "destination",
                    "output_path",
                    "target_path",
                    "destination_path",
                ],
            );
            if let Some((source, archive)) = route_result.and_then(archive_pack_pair_for_route) {
                obj.entry("source".to_string())
                    .or_insert_with(|| Value::String(source));
                obj.entry("archive".to_string())
                    .or_insert_with(|| Value::String(archive));
            }
            if !obj.contains_key("format") {
                if let Some(archive) = obj
                    .get("archive")
                    .and_then(Value::as_str)
                    .filter(|archive| is_supported_archive_path(archive))
                {
                    obj.insert(
                        "format".to_string(),
                        Value::String(archive_format_for_path(archive).to_string()),
                    );
                }
            }
        }
        "unpack" => {
            normalize_arg_alias(
                obj,
                "archive",
                &["archive_path", "path", "input", "input_path"],
            );
            normalize_arg_alias(
                obj,
                "dest",
                &["dest_path", "destination", "destination_path", "output_dir"],
            );
        }
        "list" => {
            normalize_arg_alias(
                obj,
                "archive",
                &["archive_path", "path", "input", "input_path"],
            );
        }
        "read" => {
            normalize_arg_alias(
                obj,
                "archive",
                &["archive_path", "path", "input", "input_path"],
            );
            normalize_arg_alias(
                obj,
                "member",
                &[
                    "entry",
                    "entry_path",
                    "member_path",
                    "file",
                    "file_path",
                    "path_inside_archive",
                ],
            );
        }
        _ => {}
    }
    args
}

fn unknown_archive_action_can_normalize_to_list(
    obj: &serde_json::Map<String, Value>,
    route_result: Option<&RouteResult>,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if archive_args_have_pack_or_unpack_shape(obj) {
        return false;
    }
    let archive = archive_arg_candidate_from_obj(obj).or_else(|| {
        let hint = route.output_contract.locator_hint.trim();
        (!hint.is_empty()).then(|| hint.to_string())
    });
    let Some(archive) = archive.filter(|archive| is_supported_archive_path(archive)) else {
        return false;
    };
    if archive_args_have_entry_selector(obj) {
        return true;
    }
    archive_list_auto_locator_target_path(Some(route), Some(&archive)).is_some()
}

fn archive_args_have_entry_selector(obj: &serde_json::Map<String, Value>) -> bool {
    [
        "entry",
        "entry_path",
        "member",
        "member_path",
        "file",
        "file_path",
        "name",
        "path_inside_archive",
    ]
    .iter()
    .any(|key| {
        obj.get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some_and(|value| !is_supported_archive_path(value))
    }) || obj
        .get("entries")
        .and_then(Value::as_array)
        .is_some_and(|entries| {
            entries.iter().any(|value| {
                value
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_some_and(|value| !is_supported_archive_path(value))
            })
        })
        || obj
            .get("members")
            .and_then(Value::as_array)
            .is_some_and(|entries| {
                entries.iter().any(|value| {
                    value
                        .as_str()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .is_some_and(|value| !is_supported_archive_path(value))
                })
            })
}

fn archive_args_have_pack_or_unpack_shape(obj: &serde_json::Map<String, Value>) -> bool {
    [
        "source",
        "source_path",
        "src",
        "dest",
        "dest_path",
        "destination",
        "destination_path",
        "output",
        "output_path",
    ]
    .iter()
    .any(|key| obj.contains_key(*key))
}

fn archive_arg_candidate_from_obj(obj: &serde_json::Map<String, Value>) -> Option<String> {
    ["archive", "archive_path", "path", "input", "input_path"]
        .iter()
        .find_map(|key| {
            obj.get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        })
}

fn normalize_archive_basic_schema_aliases(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args } if skill == "archive_basic" => {
                AgentAction::CallSkill {
                    skill,
                    args: normalize_archive_basic_args(args, route_result),
                }
            }
            other => other,
        })
        .collect()
}

fn rewrite_archive_basic_short_archive_to_active_bound_target(
    plan_context: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let targets = active_bound_archive_targets_from_plan_context(plan_context);
    if targets.is_empty() {
        return actions;
    }
    actions
        .into_iter()
        .enumerate()
        .map(|(idx, action)| {
            let AgentAction::CallSkill { skill, mut args } = action else {
                return action;
            };
            if !skill.eq_ignore_ascii_case("archive_basic") {
                return AgentAction::CallSkill { skill, args };
            }
            let Some(obj) = args.as_object_mut() else {
                return AgentAction::CallSkill { skill, args };
            };
            let action_name = obj
                .get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            if !action_name.eq_ignore_ascii_case("list") {
                return AgentAction::CallSkill { skill, args };
            }
            let Some(current_archive) = obj
                .get("archive")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
            else {
                return AgentAction::CallSkill { skill, args };
            };
            let Some(target) = targets
                .iter()
                .find(|target| short_locator_matches_target_basename(&current_archive, target))
            else {
                return AgentAction::CallSkill { skill, args };
            };
            obj.insert("archive".to_string(), Value::String(target.clone()));
            info!(
                "plan_rewrite_archive_short_archive_to_active_bound_target idx={} from={} to={}",
                idx, current_archive, target
            );
            AgentAction::CallSkill { skill, args }
        })
        .collect()
}

fn active_bound_archive_targets_from_plan_context(plan_context: Option<&str>) -> Vec<String> {
    let Some(plan_context) = plan_context else {
        return Vec::new();
    };
    let mut targets = Vec::new();
    for line in plan_context.lines() {
        let trimmed = line.trim_start();
        let target = ["followup_bound_target:", "observed_bound_target:"]
            .iter()
            .find_map(|prefix| trimmed.strip_prefix(prefix))
            .map(str::trim)
            .filter(|target| is_supported_archive_path(target));
        let Some(target) = target else {
            continue;
        };
        if !targets
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(target))
        {
            targets.push(target.to_string());
        }
    }
    targets
}

fn short_locator_matches_target_basename(locator: &str, target: &str) -> bool {
    let locator = trim_archive_bound_locator_token(locator);
    if locator.is_empty()
        || locator.contains('/')
        || locator.contains('\\')
        || Path::new(&locator).is_absolute()
        || Path::new(&locator).components().count() != 1
        || !is_supported_archive_path(&locator)
    {
        return false;
    }
    Path::new(target)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case(&locator))
        .unwrap_or(false)
}

fn trim_archive_bound_locator_token(token: &str) -> String {
    token
        .trim()
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\''
                    | '`'
                    | ','
                    | '，'
                    | '。'
                    | ':'
                    | '：'
                    | ';'
                    | '；'
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | '《'
                    | '》'
            )
        })
        .to_string()
}

fn strip_directory_read_range_after_inventory_dir(actions: Vec<AgentAction>) -> Vec<AgentAction> {
    let mut inventory_paths: Vec<String> = Vec::new();
    let mut old_to_new: Vec<Option<usize>> = vec![None; actions.len() + 1];
    let mut stripped = Vec::new();
    let mut stripped_indices = Vec::new();

    for (idx, action) in actions.into_iter().enumerate() {
        let old_idx = idx + 1;
        if let Some((action_name, path)) = system_basic_action_path(&action) {
            if action_name == "read_range" && inventory_paths.iter().any(|known| known == &path) {
                stripped_indices.push(old_idx);
                continue;
            }
            if action_name == "inventory_dir" && !inventory_paths.iter().any(|known| known == &path)
            {
                inventory_paths.push(path);
            }
        }
        old_to_new[old_idx] = Some(stripped.len() + 1);
        stripped.push(action);
    }

    if stripped_indices.is_empty() {
        return stripped;
    }

    for action in &mut stripped {
        if let AgentAction::SynthesizeAnswer { evidence_refs } = action {
            *evidence_refs = rewrite_evidence_refs_after_step_strip(evidence_refs, &old_to_new);
        }
    }

    info!(
        "plan_strip_directory_read_range_after_inventory_dir stripped_steps={}",
        stripped_indices
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    stripped
}

fn system_basic_action_path(action: &AgentAction) -> Option<(String, String)> {
    let AgentAction::CallSkill { skill, args } = action else {
        return None;
    };
    if skill != "system_basic" {
        return None;
    }
    let obj = args.as_object()?;
    let action_name = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_ascii_lowercase();
    let raw_path = if action_name == "inventory_dir" {
        obj.get("path").and_then(Value::as_str).unwrap_or(".")
    } else {
        obj.get("path").and_then(Value::as_str)?
    };
    let path = normalize_plan_path(raw_path)?;
    Some((action_name, path))
}

fn normalize_plan_path(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_trailing_slashes = trimmed.trim_end_matches('/');
    if without_trailing_slashes.is_empty() {
        Some("/".to_string())
    } else {
        Some(without_trailing_slashes.to_string())
    }
}

fn system_basic_action_path_and_args(
    action: &AgentAction,
) -> Option<(String, String, &serde_json::Map<String, Value>)> {
    let AgentAction::CallSkill { skill, args } = action else {
        return None;
    };
    if skill != "system_basic" {
        return None;
    }
    let obj = args.as_object()?;
    let (action_name, path) = system_basic_action_path(action)?;
    Some((action_name, path, obj))
}

fn strip_file_lines_count_before_tail_read_range(actions: Vec<AgentAction>) -> Vec<AgentAction> {
    let mut rewritten = Vec::with_capacity(actions.len());
    let mut stripped = 0usize;
    let mut idx = 0usize;
    while idx < actions.len() {
        let should_strip = if let Some((action_name, path, _)) =
            system_basic_action_path_and_args(&actions[idx])
        {
            action_name == "file_lines_count"
                && actions.get(idx + 1).is_some_and(|next| {
                    system_basic_action_path_and_args(next).is_some_and(
                        |(next_action, next_path, next_args)| {
                            next_action == "read_range"
                                && next_path == path
                                && next_args
                                    .get("mode")
                                    .and_then(Value::as_str)
                                    .is_some_and(|mode| mode.eq_ignore_ascii_case("tail"))
                        },
                    )
                })
        } else {
            false
        };
        if should_strip {
            stripped += 1;
        } else {
            rewritten.push(actions[idx].clone());
        }
        idx += 1;
    }
    if stripped > 0 {
        info!(
            "plan_strip_file_lines_count_before_tail_read_range stripped_steps={}",
            stripped
        );
    }
    rewritten
}

fn rewrite_evidence_refs_after_step_strip(
    refs: &[String],
    old_to_new: &[Option<usize>],
) -> Vec<String> {
    let mut rewritten = Vec::new();
    for evidence_ref in refs {
        let Some(replacement) =
            rewrite_single_evidence_ref_after_step_strip(evidence_ref, old_to_new)
        else {
            continue;
        };
        if !rewritten.iter().any(|existing| existing == &replacement) {
            rewritten.push(replacement);
        }
    }
    if rewritten.is_empty() {
        rewritten.push("last_output".to_string());
    }
    rewritten
}

fn rewrite_single_evidence_ref_after_step_strip(
    evidence_ref: &str,
    old_to_new: &[Option<usize>],
) -> Option<String> {
    let trimmed = evidence_ref.trim();
    if let Some(step_idx) = trimmed
        .strip_prefix("step_")
        .and_then(|value| value.parse::<usize>().ok())
    {
        return old_to_new
            .get(step_idx)
            .and_then(|value| *value)
            .map(|new_idx| format!("step_{new_idx}"));
    }
    if let Some(step_idx) = trimmed
        .strip_prefix('s')
        .filter(|value| value.chars().all(|ch| ch.is_ascii_digit()))
        .and_then(|value| value.parse::<usize>().ok())
    {
        return old_to_new
            .get(step_idx)
            .and_then(|value| *value)
            .map(|new_idx| format!("s{new_idx}"));
    }
    Some(evidence_ref.to_string())
}

fn enforce_output_contract_tool_args(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };

    actions
        .into_iter()
        .map(|mut action| {
            match &mut action {
                AgentAction::CallSkill { skill, args }
                | AgentAction::CallTool { tool: skill, args }
                    if skill.eq_ignore_ascii_case("system_basic") =>
                {
                    if rewrite_inventory_ext_filter_action_to_fs_basic(route, skill, args) {
                        return action;
                    }
                    let Some(obj) = args.as_object_mut() else {
                        return action;
                    };
                    let action_name = obj
                        .get("action")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .unwrap_or_default()
                        .to_string();
                    let action_name_lower = action_name.to_ascii_lowercase();
                    if action_name_lower == "inventory_dir" {
                        enforce_file_names_inventory_args(route, obj);
                        enforce_directory_names_inventory_args(route, obj);
                        enforce_general_directory_inventory_args(route, obj);
                        enforce_strict_directory_metadata_inventory_args(route, obj);
                    }
                    if route.output_contract.semantic_kind
                        != crate::OutputSemanticKind::HiddenEntriesCheck
                    {
                        return action;
                    }
                    if matches!(
                        action_name_lower.as_str(),
                        "inventory_dir" | "count_inventory" | "workspace_glance"
                    ) {
                        obj.insert("include_hidden".to_string(), Value::Bool(true));
                        info!(
                            "plan_contract_enforce_hidden_inventory action={}",
                            action_name
                        );
                    }
                }
                AgentAction::CallSkill { skill, args }
                | AgentAction::CallTool { tool: skill, args }
                    if skill.eq_ignore_ascii_case("fs_search") =>
                {
                    enforce_fs_search_path_output_args(route, args);
                }
                AgentAction::CallSkill { skill, args }
                | AgentAction::CallTool { tool: skill, args }
                    if skill.eq_ignore_ascii_case("fs_basic") =>
                {
                    if rewrite_inventory_ext_filter_action_to_fs_basic(route, skill, args) {
                        return action;
                    }
                    let Some(obj) = args.as_object_mut() else {
                        return action;
                    };
                    let action_name = obj
                        .get("action")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    if matches!(action_name.as_str(), "find_entries" | "list_dir") {
                        enforce_file_names_inventory_args(route, obj);
                        enforce_directory_names_inventory_args(route, obj);
                    }
                    if action_name == "list_dir" {
                        enforce_strict_directory_metadata_inventory_args(route, obj);
                    }
                }
                _ => {}
            }
            action
        })
        .collect()
}

fn structural_extension_filter_from_text(text: &str) -> Option<String> {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '*')))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .find_map(extension_from_globish_pattern)
}

fn route_allows_structural_extension_inventory_filter(route: &RouteResult) -> bool {
    !route.output_contract.delivery_required
        && route.output_contract.requires_content_evidence
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
                | crate::OutputSemanticKind::QuantityComparison
                | crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::FilePaths
                | crate::OutputSemanticKind::DirectoryEntryGroups
        )
}

fn inject_structural_extension_filter_for_directory_inventory(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route_allows_structural_extension_inventory_filter(route) {
        return actions;
    }
    let Some(ext) = structural_extension_filter_from_text(&route.resolved_intent) else {
        return actions;
    };
    let mut changed = false;
    let actions = actions
        .into_iter()
        .map(|mut action| {
            match &mut action {
                AgentAction::CallSkill { skill, args }
                | AgentAction::CallTool { tool: skill, args }
                    if skill.eq_ignore_ascii_case("fs_basic")
                        || skill.eq_ignore_ascii_case("system_basic") =>
                {
                    let Some(obj) = args.as_object_mut() else {
                        return action;
                    };
                    let action_name = obj
                        .get("action")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    if matches!(action_name.as_str(), "list_dir" | "inventory_dir") {
                        if !obj.get("ext_filter").is_some_and(has_non_empty_json_value)
                            && !obj.get("extension").is_some_and(has_non_empty_json_value)
                            && !obj.get("extensions").is_some_and(has_non_empty_json_value)
                        {
                            obj.insert(
                                "ext_filter".to_string(),
                                Value::Array(vec![Value::String(ext.clone())]),
                            );
                            obj.insert("files_only".to_string(), Value::Bool(true));
                            obj.insert("dirs_only".to_string(), Value::Bool(false));
                            if route.output_contract.semantic_kind
                                == crate::OutputSemanticKind::QuantityComparison
                            {
                                obj.insert(
                                    "max_entries".to_string(),
                                    Value::Number(serde_json::Number::from(1000)),
                                );
                            }
                            changed = true;
                        }
                    } else if matches!(action_name.as_str(), "find_entries" | "find_ext") {
                        if !obj.get("ext").is_some_and(has_non_empty_json_value)
                            && !obj.get("ext_filter").is_some_and(has_non_empty_json_value)
                        {
                            obj.insert("ext".to_string(), Value::String(ext.clone()));
                            obj.insert(
                                "target_kind".to_string(),
                                Value::String("file".to_string()),
                            );
                            changed = true;
                        }
                    }
                }
                _ => {}
            }
            action
        })
        .collect();
    if changed {
        info!(
            "plan_inject_structural_extension_inventory_filter ext={}",
            crate::truncate_for_log(&ext)
        );
    }
    actions
}

fn route_requests_general_directory_inventory(route: &RouteResult) -> bool {
    if route.output_contract.delivery_required {
        return false;
    }
    if route.output_contract.delivery_intent == crate::OutputDeliveryIntent::DirectoryLookup {
        return true;
    }
    route.output_contract.response_shape == crate::OutputResponseShape::Free
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None | crate::OutputSemanticKind::DirectoryPurposeSummary
        )
}

fn inventory_dir_has_filter_args(obj: &serde_json::Map<String, Value>) -> bool {
    obj.get("ext_filter").is_some_and(has_non_empty_json_value)
        || obj.get("extension").is_some_and(has_non_empty_json_value)
        || obj.get("extensions").is_some_and(has_non_empty_json_value)
}

fn enforce_general_directory_inventory_args(
    route: &RouteResult,
    obj: &mut serde_json::Map<String, Value>,
) {
    if !route_requests_general_directory_inventory(route) || inventory_dir_has_filter_args(obj) {
        return;
    }
    obj.insert("files_only".to_string(), Value::Bool(false));
    obj.insert("dirs_only".to_string(), Value::Bool(false));
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::FileNames {
        obj.insert("names_only".to_string(), Value::Bool(true));
        info!("plan_contract_enforce_directory_entry_names_inventory");
        return;
    }
    obj.insert("names_only".to_string(), Value::Bool(false));
    info!("plan_contract_enforce_general_directory_inventory");
}

fn route_requires_strict_directory_metadata_inventory(route: &RouteResult) -> bool {
    !route.output_contract.delivery_required
        && route.output_contract.requires_content_evidence
        && route.output_contract.response_shape == crate::OutputResponseShape::Strict
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::None
}

fn enforce_strict_directory_metadata_inventory_args(
    route: &RouteResult,
    obj: &mut serde_json::Map<String, Value>,
) {
    if !route_requires_strict_directory_metadata_inventory(route) {
        return;
    }
    obj.insert("names_only".to_string(), Value::Bool(false));
    obj.entry("max_entries".to_string())
        .or_insert_with(|| Value::Number(serde_json::Number::from(1000)));
    info!("plan_contract_enforce_strict_directory_metadata_inventory");
}

fn enforce_directory_names_inventory_args(
    route: &RouteResult,
    obj: &mut serde_json::Map<String, Value>,
) {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::DirectoryNames {
        return;
    }
    let directory_filter_requested = structured_directory_filter_requested(obj);
    let file_filter_requested = structured_file_filter_requested(obj);
    obj.insert("files_only".to_string(), Value::Bool(false));
    obj.insert(
        "dirs_only".to_string(),
        Value::Bool(directory_filter_requested || file_filter_requested),
    );
    obj.insert("names_only".to_string(), Value::Bool(true));
    obj.insert("include_hidden".to_string(), Value::Bool(false));
    info!("plan_contract_enforce_directory_names_inventory");
}

fn enforce_file_names_inventory_args(
    route: &RouteResult,
    obj: &mut serde_json::Map<String, Value>,
) {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::FileNames
        || route.output_contract.delivery_intent == crate::OutputDeliveryIntent::DirectoryLookup
    {
        return;
    }
    obj.insert("files_only".to_string(), Value::Bool(true));
    obj.insert("dirs_only".to_string(), Value::Bool(false));
    obj.insert("names_only".to_string(), Value::Bool(true));
    obj.insert("include_hidden".to_string(), Value::Bool(false));
    info!("plan_contract_enforce_file_names_inventory");
}

fn first_ext_filter_value(obj: &serde_json::Map<String, Value>) -> Option<String> {
    let value = obj
        .get("ext_filter")
        .or_else(|| obj.get("ext"))
        .or_else(|| obj.get("extension"))
        .or_else(|| obj.get("extensions"))?;
    match value {
        Value::String(text) => normalize_extension_filter_text(text),
        Value::Array(items) => items
            .iter()
            .find_map(|item| item.as_str().and_then(normalize_extension_filter_text)),
        _ => None,
    }
}

fn normalize_extension_filter_text(text: &str) -> Option<String> {
    text.trim()
        .trim_start_matches('.')
        .trim()
        .to_ascii_lowercase()
        .split(',')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn extension_from_globish_pattern(text: &str) -> Option<String> {
    let cleaned = text
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_ascii_lowercase();
    let (_prefix, ext) = cleaned.rsplit_once('.')?;
    if ext.is_empty()
        || ext.contains(['*', '?', '/', '\\'])
        || !ext
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return None;
    }
    cleaned
        .contains('*')
        .then(|| ext.to_string())
        .or_else(|| cleaned.strip_prefix('.').map(ToString::to_string))
}

fn should_rewrite_inventory_ext_filter_to_fs_basic(route: &RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::FilePaths
        && matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::CurrentWorkspace | crate::OutputLocatorKind::Path
        )
        && !route.output_contract.delivery_required
}

fn rewrite_inventory_ext_filter_action_to_fs_basic(
    route: &RouteResult,
    skill: &mut String,
    args: &mut Value,
) -> bool {
    if !should_rewrite_inventory_ext_filter_to_fs_basic(route) {
        return false;
    }
    let Some(obj) = args.as_object() else {
        return false;
    };
    let action_name = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !matches!(action_name, "inventory_dir" | "list_dir") {
        return false;
    }
    let Some(ext) = first_ext_filter_value(obj) else {
        return false;
    };
    let root = obj
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(".");
    let max_results = obj
        .get("max_entries")
        .or_else(|| obj.get("limit"))
        .and_then(Value::as_u64)
        .unwrap_or(100);
    *skill = "fs_basic".to_string();
    *args = serde_json::json!({
        "action": "find_entries",
        "root": root,
        "ext": ext,
        "target_kind": "file",
        "max_results": max_results,
        "recursive": true
    });
    info!("plan_contract_rewrite_inventory_ext_filter_to_fs_basic");
    true
}

fn route_prefers_fs_search_name_result(route: &RouteResult) -> bool {
    !route.output_contract.delivery_required
        && matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        )
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::ScalarPathOnly
                | crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::DirectoryNames
                | crate::OutputSemanticKind::FilePaths
        )
}

fn enforce_fs_search_path_output_args(route: &RouteResult, args: &mut Value) -> bool {
    if !route_prefers_fs_search_name_result(route) {
        return false;
    }
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    let mut changed = false;
    let is_grep_text = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|action| action.eq_ignore_ascii_case("grep_text"));
    if obj
        .get("root")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        if let Some(path) = ["path", "dir", "directory", "search_root"]
            .iter()
            .find_map(|key| obj.get(*key).and_then(Value::as_str))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .filter(|value| Path::new(value).is_dir())
            .map(ToString::to_string)
        {
            obj.insert("root".to_string(), Value::String(path));
            changed = true;
        }
    }
    if !is_grep_text && !obj.contains_key("pattern") {
        if let Some(pattern) = ["basename_pattern", "name", "keyword", "query"]
            .iter()
            .find_map(|key| obj.get(*key).and_then(Value::as_str))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
        {
            obj.insert("pattern".to_string(), Value::String(pattern));
            changed = true;
        }
    }
    if !obj.contains_key("target_kind") {
        if obj
            .get("type")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some_and(|value| value.eq_ignore_ascii_case("file"))
        {
            obj.insert("target_kind".to_string(), Value::String("file".to_string()));
            changed = true;
        }
    }
    if !is_grep_text && route.output_contract.semantic_kind == crate::OutputSemanticKind::FilePaths
    {
        if let Some(ext) = first_ext_filter_value(obj).or_else(|| {
            obj.get("pattern")
                .and_then(Value::as_str)
                .and_then(extension_from_globish_pattern)
        }) {
            obj.insert("action".to_string(), Value::String("find_ext".to_string()));
            obj.insert("ext".to_string(), Value::String(ext));
            changed = true;
        }
    }
    let has_action = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if !has_action {
        if let Some(query) = obj
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
        {
            obj.insert("action".to_string(), Value::String("find_name".to_string()));
            obj.entry("pattern".to_string())
                .or_insert_with(|| Value::String(query));
            changed = true;
        }
    }
    changed
}

fn action_is_workspace_summary_evidence(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, .. } if skill == "list_dir" || skill == "read_file" => true,
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic" =>
        {
            args.get("action")
                .and_then(|value| value.as_str())
                .is_some_and(|action| {
                    matches!(
                        action.trim().to_ascii_lowercase().as_str(),
                        "list_dir" | "read_text_range" | "stat_paths"
                    )
                })
        }
        AgentAction::CallSkill { skill, args } if skill == "system_basic" => args
            .get("action")
            .and_then(|value| value.as_str())
            .is_some_and(|action| {
                matches!(
                    action.trim().to_ascii_lowercase().as_str(),
                    "find_name"
                        | "find_path"
                        | "inventory_dir"
                        | "read_range"
                        | "tree_summary"
                        | "workspace_glance"
                )
            }),
        _ => false,
    }
}

fn route_needs_unscoped_workspace_text_evidence(route: &RouteResult) -> bool {
    !route.needs_clarify
        && !route.output_contract.delivery_required
        && route.output_contract.requires_content_evidence
        && route_expects_terminal_user_answer(route)
        && route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::None
}

fn route_needs_workspace_synthesis_evidence(route: &RouteResult) -> bool {
    !route.needs_clarify
        && !route.output_contract.delivery_required
        && route.output_contract.requires_content_evidence
        && route_expects_terminal_user_answer(route)
        && route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None | crate::OutputSemanticKind::WorkspaceProjectSummary
        )
}

fn route_needs_workspace_summary_default_evidence(route: &RouteResult) -> bool {
    !route.needs_clarify
        && !route.output_contract.delivery_required
        && route.output_contract.requires_content_evidence
        && route_expects_terminal_user_answer(route)
        && route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::WorkspaceProjectSummary
}

fn route_needs_workspace_respond_only_default_evidence(route: &RouteResult) -> bool {
    route_needs_workspace_synthesis_evidence(route)
}

fn route_disallows_unrequested_workspace_artifact_mutation(
    route: &RouteResult,
    loop_state: &LoopState,
) -> bool {
    route_needs_unscoped_workspace_text_evidence(route)
        && !route.wants_file_delivery
        && route.output_contract.delivery_intent == crate::OutputDeliveryIntent::None
        && !loop_state.execution_recipe.is_active()
}

fn route_locator_hint_is_path_like(route: &RouteResult) -> bool {
    let hint = route.output_contract.locator_hint.trim();
    if hint.is_empty() {
        return false;
    }
    if matches!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
    ) {
        return true;
    }
    let path = Path::new(hint);
    path.is_absolute()
        || path.components().count() > 1
        || path.extension().is_some()
        || hint.starts_with('.')
        || hint.starts_with('~')
}

fn action_path_arg(args: &Value) -> Option<&str> {
    args.get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn action_targets_route_locator_artifact(
    state: &AppState,
    route: &RouteResult,
    action: &AgentAction,
) -> bool {
    if !route_locator_hint_is_path_like(route) {
        return false;
    }
    let locator = resolve_workspace_path(
        &state.skill_rt.workspace_root,
        route.output_contract.locator_hint.trim(),
    );
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let Some(raw_path) = action_path_arg(args) else {
                return false;
            };
            let action_path = resolve_workspace_path(&state.skill_rt.workspace_root, raw_path);
            match state.resolve_canonical_skill_name(skill).as_str() {
                "write_file" | "remove_file" => {
                    same_existing_or_display_path(&locator, &action_path)
                }
                "make_dir" => {
                    same_existing_or_display_path(&locator, &action_path)
                        || locator.parent().is_some_and(|parent| {
                            same_existing_or_display_path(parent, &action_path)
                        })
                }
                "fs_basic" => match args
                    .get("action")
                    .and_then(Value::as_str)
                    .map(|action| action.trim().to_ascii_lowercase())
                    .as_deref()
                {
                    Some("write_text" | "append_text" | "remove_path") => {
                        same_existing_or_display_path(&locator, &action_path)
                    }
                    Some("make_dir") => {
                        same_existing_or_display_path(&locator, &action_path)
                            || locator.parent().is_some_and(|parent| {
                                same_existing_or_display_path(parent, &action_path)
                            })
                    }
                    _ => false,
                },
                _ => false,
            }
        }
        AgentAction::SynthesizeAnswer { .. } => false,
        AgentAction::CallCapability { .. } => false,
        AgentAction::Respond { .. } | AgentAction::Think { .. } => false,
    }
}

fn strip_unrequested_workspace_artifact_mutations(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route_disallows_unrequested_workspace_artifact_mutation(route, loop_state)
        || !actions.iter().any(action_is_likely_mutating)
    {
        return actions;
    }

    let original_len = actions.len();
    let mut removed_mutations = 0usize;
    let mut removed_discussion = 0usize;
    let stripped = actions
        .into_iter()
        .filter(|action| {
            if action_is_likely_mutating(action) {
                if action_targets_route_locator_artifact(state, route, action) {
                    return true;
                }
                removed_mutations += 1;
                return false;
            }
            if is_discussion_followup_action(action) {
                removed_discussion += 1;
                return false;
            }
            true
        })
        .collect::<Vec<_>>();
    if removed_mutations > 0 {
        info!(
            "plan_strip_unrequested_workspace_artifact_mutations removed_mutations={} removed_discussion={} kept={}",
            removed_mutations,
            removed_discussion,
            original_len.saturating_sub(removed_mutations + removed_discussion)
        );
    }
    stripped
}

fn action_reads_workspace_text_content(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, .. } | AgentAction::CallTool { tool: skill, .. }
            if skill.eq_ignore_ascii_case("read_file")
                || skill.eq_ignore_ascii_case("doc_parse") =>
        {
            true
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("fs_basic") =>
        {
            args.get("action")
                .and_then(|value| value.as_str())
                .is_some_and(|action| {
                    matches!(
                        action.trim().to_ascii_lowercase().as_str(),
                        "read_text_range" | "grep_text"
                    )
                })
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("fs_search") =>
        {
            args.get("action")
                .and_then(|value| value.as_str())
                .is_some_and(|action| action.trim().eq_ignore_ascii_case("grep_text"))
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("system_basic") =>
        {
            args.get("action")
                .and_then(|value| value.as_str())
                .is_some_and(|action| action.trim().eq_ignore_ascii_case("read_range"))
        }
        AgentAction::CallCapability { .. }
        | AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::CallSkill { .. }
        | AgentAction::CallTool { .. } => false,
    }
}

fn action_observes_content_presence_search(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("fs_basic")
                || skill.eq_ignore_ascii_case("fs_search") =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|action| action.eq_ignore_ascii_case("grep_text"))
        }
        _ => false,
    }
}

fn executed_step_reads_workspace_text_content(step: &crate::executor::StepExecutionResult) -> bool {
    if !step.is_ok() {
        return false;
    }
    if step.skill.eq_ignore_ascii_case("read_file") || step.skill.eq_ignore_ascii_case("doc_parse")
    {
        return step
            .output
            .as_deref()
            .map(str::trim)
            .is_some_and(|output| !output.is_empty());
    }
    if !step.skill.eq_ignore_ascii_case("system_basic") {
        if !step.skill.eq_ignore_ascii_case("fs_search") {
            return false;
        }
        return step
            .output
            .as_deref()
            .and_then(|output| serde_json::from_str::<Value>(output).ok())
            .and_then(|value| {
                value
                    .get("action")
                    .and_then(Value::as_str)
                    .map(|action| action.trim().eq_ignore_ascii_case("grep_text"))
            })
            .unwrap_or(false);
    }
    step.output
        .as_deref()
        .and_then(|output| serde_json::from_str::<Value>(output).ok())
        .and_then(|value| {
            value
                .get("action")
                .and_then(Value::as_str)
                .map(|action| action.trim().eq_ignore_ascii_case("read_range"))
        })
        .unwrap_or(false)
}

fn action_observes_locator_only(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("fs_basic") =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|action| {
                    matches!(
                        action.to_ascii_lowercase().as_str(),
                        "find_entries" | "stat_paths"
                    )
                })
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("fs_search") =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|action| {
                    matches!(
                        action.to_ascii_lowercase().as_str(),
                        "find_name" | "find_ext"
                    )
                })
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("system_basic") =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|action| {
                    matches!(
                        action.to_ascii_lowercase().as_str(),
                        "find_path" | "path_batch_facts"
                    )
                })
        }
        _ => false,
    }
}

fn content_evidence_plan_only_has_locator_observation(
    route_result: &RouteResult,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::ContentPresenceCheck
    {
        let executable_actions = actions
            .iter()
            .filter(|action| {
                matches!(
                    action,
                    AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
                )
            })
            .collect::<Vec<_>>();
        if !executable_actions.is_empty()
            && !executable_actions.iter().any(|action| {
                action_observes_content_presence_search(action)
                    || action_reads_workspace_text_content(action)
            })
        {
            return true;
        }
    }
    if route_uses_runtime_owned_observed_finalizer(route_result)
        && has_tool_or_skill_observation(actions)
    {
        return false;
    }
    if path_metadata_facts_plan_satisfies_route(route_result, actions) {
        return false;
    }
    if name_listing_observation_plan_satisfies_route(route_result, actions) {
        return false;
    }
    if structured_listing_terminal_plan_satisfies_observation(actions) {
        return false;
    }
    if loop_state.has_tool_or_skill_output
        || route_result.needs_clarify
        || !route_result.output_contract.requires_content_evidence
        || !route_expects_terminal_user_answer(route_result)
        || has_workspace_text_content_evidence(loop_state, actions)
    {
        return false;
    }
    let executable_actions = actions
        .iter()
        .filter(|action| {
            matches!(
                action,
                AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
            )
        })
        .collect::<Vec<_>>();
    !executable_actions.is_empty()
        && executable_actions
            .iter()
            .all(|action| action_observes_locator_only(action))
}

fn name_listing_observation_plan_satisfies_route(
    route_result: &RouteResult,
    actions: &[AgentAction],
) -> bool {
    if !matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::DirectoryNames
            | crate::OutputSemanticKind::FilePaths
    ) {
        return false;
    }
    actions.iter().any(action_provides_name_listing_evidence)
}

fn structured_listing_terminal_plan_satisfies_observation(actions: &[AgentAction]) -> bool {
    if !has_discussion_followup_action(actions) {
        return false;
    }
    actions.iter().any(action_provides_name_listing_evidence)
        && actions.iter().any(|action| {
            matches!(
                action,
                AgentAction::SynthesizeAnswer { evidence_refs }
                    if !evidence_refs.is_empty()
            )
        })
}

fn action_provides_name_listing_evidence(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, .. } | AgentAction::CallTool { tool: skill, .. }
            if skill.eq_ignore_ascii_case("list_dir") =>
        {
            true
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("fs_basic") =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|action| {
                    matches!(
                        action.to_ascii_lowercase().as_str(),
                        "find_entries" | "list_dir"
                    )
                })
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("fs_search") =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|action| {
                    matches!(
                        action.to_ascii_lowercase().as_str(),
                        "find_name" | "find_ext"
                    )
                })
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("system_basic") =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|action| {
                    matches!(
                        action.to_ascii_lowercase().as_str(),
                        "inventory_dir" | "find_path" | "structured_keys"
                    )
                })
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("config_basic") =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|action| action.eq_ignore_ascii_case("list_keys"))
        }
        _ => false,
    }
}

fn path_metadata_facts_plan_satisfies_route(
    route_result: &RouteResult,
    actions: &[AgentAction],
) -> bool {
    if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::ExistenceWithPath
        || (route_result.output_contract.semantic_kind == crate::OutputSemanticKind::None
            && route_result.output_contract.requires_content_evidence
            && matches!(
                route_result.output_contract.locator_kind,
                crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
            )
            && !route_result.output_contract.delivery_required)
    {
        return path_metadata_facts_response_is_sufficient(actions)
            || path_metadata_facts_synthesizes_terminal_answer(actions);
    }
    route_requests_path_metadata_compare(route_result)
        && (structured_scalar_observation_units(actions) >= 2
            || actions_satisfy_single_path_metadata_facts(route_result, actions))
}

fn path_metadata_facts_synthesizes_terminal_answer(actions: &[AgentAction]) -> bool {
    actions.iter().any(planned_action_is_path_metadata_facts)
        && actions
            .iter()
            .any(|action| matches!(action, AgentAction::SynthesizeAnswer { .. }))
        && matches!(
            actions.last(),
            Some(AgentAction::Respond { content }) if content.contains("{{")
        )
}

fn has_workspace_text_content_evidence(loop_state: &LoopState, actions: &[AgentAction]) -> bool {
    loop_state
        .executed_step_results
        .iter()
        .any(executed_step_reads_workspace_text_content)
        || actions.iter().any(action_reads_workspace_text_content)
}

fn has_run_cmd_observation_action(actions: &[AgentAction]) -> bool {
    actions.iter().any(action_skill_is_run_cmd)
}

fn workspace_synthesis_needs_more_text_evidence(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    route_needs_workspace_synthesis_evidence(route)
        && has_only_workspace_summary_observation_actions(actions)
        && !has_listing_grounded_synthesis_answer_plan(route, actions)
        && !has_workspace_text_content_evidence(loop_state, actions)
        && !has_compact_structured_observation_answer_plan(actions)
        && !has_mixed_last_output_terminal_respond(actions)
        && !has_run_cmd_observation_action(actions)
}

fn has_only_workspace_summary_observation_actions(actions: &[AgentAction]) -> bool {
    let mut saw_observation = false;
    for action in actions {
        match action {
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. } => {
                saw_observation = true;
                if !action_is_workspace_summary_evidence(action) {
                    return false;
                }
            }
            _ => {}
        }
    }
    saw_observation
}

fn has_listing_grounded_synthesis_answer_plan(
    route: &RouteResult,
    actions: &[AgentAction],
) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::None
        && route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && has_discussion_followup_action(actions)
        && actions.iter().any(action_is_directory_listing_observation)
}

fn action_is_directory_listing_observation(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, .. } | AgentAction::CallTool { tool: skill, .. }
            if skill.eq_ignore_ascii_case("list_dir") =>
        {
            true
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("fs_basic") =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|action| action.eq_ignore_ascii_case("list_dir"))
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("system_basic") =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|action| action.eq_ignore_ascii_case("inventory_dir"))
        }
        _ => false,
    }
}

fn strip_workspace_synthesis_without_text_evidence(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !workspace_synthesis_needs_more_text_evidence(route_result, loop_state, &actions)
        || !has_discussion_followup_action(&actions)
    {
        return actions;
    }

    let stripped = actions
        .iter()
        .filter(|action| !is_discussion_followup_action(action))
        .cloned()
        .collect::<Vec<_>>();
    if stripped.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
        )
    }) {
        info!("plan_strip_workspace_synthesis_without_text_evidence");
        stripped
    } else {
        actions
    }
}

fn append_synthesize_for_unscoped_workspace_text_evidence(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route_needs_unscoped_workspace_text_evidence(route)
        || has_discussion_followup_action(&actions)
        || workspace_synthesis_needs_more_text_evidence(route_result, loop_state, &actions)
        || has_compact_structured_observation_action(&actions)
    {
        return actions;
    }
    let evidence_refs = actions
        .iter()
        .enumerate()
        .filter_map(|(idx, action)| {
            action_reads_workspace_text_content(action).then(|| format!("step_{}", idx + 1))
        })
        .collect::<Vec<_>>();
    if evidence_refs.is_empty() {
        return actions;
    }
    let mut rewritten = actions;
    let refs_log = evidence_refs.join(",");
    rewritten.push(AgentAction::SynthesizeAnswer { evidence_refs });
    info!("plan_append_unscoped_workspace_text_evidence_synthesis refs={refs_log}");
    rewritten
}

fn action_reads_git_history(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("git_basic") =>
        {
            args.get("action")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .is_some_and(|action| {
                    matches!(
                        action.to_ascii_lowercase().as_str(),
                        "log" | "show" | "status" | "diff" | "diff_cached" | "changed_files"
                    )
                })
        }
        _ => false,
    }
}

fn ensure_workspace_synthesis_has_default_text_evidence(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if loop_state.has_tool_or_skill_output
        || !route.is_execute_gate()
        || !route_needs_workspace_summary_default_evidence(route)
        || !has_executable_observation_or_action(&actions)
        || !has_discussion_followup_action(&actions)
        || has_mixed_last_output_terminal_respond(&actions)
        || has_run_cmd_observation_action(&actions)
    {
        return actions;
    }
    let has_text_evidence = actions.iter().any(action_reads_workspace_text_content);
    let has_git_history = actions.iter().any(action_reads_git_history);
    if has_text_evidence && has_git_history {
        return actions;
    }
    let insert_idx = actions
        .iter()
        .position(is_discussion_followup_action)
        .unwrap_or(actions.len());
    let mut rewritten = Vec::with_capacity(actions.len() + 2);
    rewritten.extend(actions[..insert_idx].iter().cloned());
    if !has_git_history {
        rewritten.push(AgentAction::CallSkill {
            skill: "git_basic".to_string(),
            args: serde_json::json!({
                "action": "log",
                "n": 8,
            }),
        });
    }
    if !has_text_evidence {
        rewritten.push(AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": "README.md",
                "mode": "head",
                "n": 40,
            }),
        });
    }
    rewritten.extend(actions[insert_idx..].iter().cloned());
    info!(
        "plan_ensure_workspace_synthesis_default_evidence added_git={} added_text={}",
        !has_git_history, !has_text_evidence
    );
    rewritten
}

fn has_compact_structured_observation_answer_plan(actions: &[AgentAction]) -> bool {
    actions
        .iter()
        .filter(|action| action_is_compact_structured_observation(action))
        .take(2)
        .count()
        >= 2
        && has_discussion_followup_action(actions)
}

fn has_compact_structured_observation_action(actions: &[AgentAction]) -> bool {
    actions.iter().any(action_is_compact_structured_observation)
}

fn action_is_compact_structured_observation(action: &AgentAction) -> bool {
    let (AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }) =
        action
    else {
        return false;
    };
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if skill.eq_ignore_ascii_case("fs_basic") {
        return matches!(
            action_name.as_str(),
            "list_dir" | "compare_paths" | "stat_paths"
        );
    }
    if skill.eq_ignore_ascii_case("config_basic") {
        return matches!(
            action_name.as_str(),
            "read_field" | "read_fields" | "list_keys"
        );
    }
    skill.eq_ignore_ascii_case("system_basic")
        && matches!(
            action_name.as_str(),
            "count_inventory" | "compare_paths" | "path_batch_facts" | "extract_fields"
        )
}

fn action_workspace_summary_path(action: &AgentAction) -> Option<&str> {
    match action {
        AgentAction::CallSkill { skill, args } if skill == "list_dir" || skill == "read_file" => {
            args.get("path").and_then(|value| value.as_str())
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic" =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .filter(|action| {
                    matches!(
                        action.trim().to_ascii_lowercase().as_str(),
                        "list_dir" | "read_text_range" | "stat_paths"
                    )
                })
                .and_then(|_| {
                    args.get("path")
                        .or_else(|| args.get("root"))
                        .and_then(|value| value.as_str())
                })
        }
        AgentAction::CallSkill { skill, args } if skill == "system_basic" => args
            .get("action")
            .and_then(|value| value.as_str())
            .filter(|action| {
                matches!(
                    action.trim().to_ascii_lowercase().as_str(),
                    "inventory_dir" | "read_range" | "workspace_glance" | "tree_summary"
                )
            })
            .and_then(|_| {
                args.get("path")
                    .or_else(|| args.get("root"))
                    .and_then(|value| value.as_str())
            }),
        _ => None,
    }
}

fn path_matches_workspace_scope_hint(path: &str, scope_hint: &str) -> bool {
    let path = path.trim().trim_end_matches(['/', '\\']);
    let scope_hint = scope_hint.trim().trim_end_matches(['/', '\\']);
    if path.is_empty()
        || scope_hint.is_empty()
        || matches!(path, "." | "./" | "/" | "")
        || matches!(scope_hint, "." | "./" | "/" | "")
    {
        return false;
    }
    let path_lower = path.to_ascii_lowercase();
    let hint_lower = scope_hint.to_ascii_lowercase();
    if path_lower == hint_lower {
        return true;
    }
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(scope_hint))
}

fn route_requests_structured_scalar_compare(route: &RouteResult) -> bool {
    let contract = crate::TaskContract::from_route_result(route);
    !route.needs_clarify
        && !route.output_contract.delivery_required
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::QuantityComparison
                | crate::OutputSemanticKind::RecentScalarEqualityCheck
        )
        && contract
            .required_evidence_fields
            .iter()
            .any(|field| matches!(field.as_str(), "field_value" | "size_bytes"))
}

fn route_requests_path_metadata_compare(route: &RouteResult) -> bool {
    route_requests_structured_scalar_compare(route)
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison
}

fn action_scalar_compare_observation_units(action: &AgentAction) -> usize {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic" =>
        {
            match args
                .get("action")
                .and_then(|value| value.as_str())
                .map(|action| action.trim().to_ascii_lowercase())
                .as_deref()
            {
                Some("compare_paths") => {
                    let has_pair = args
                        .get("left_path")
                        .and_then(|value| value.as_str())
                        .is_some_and(|path| !path.trim().is_empty())
                        && args
                            .get("right_path")
                            .and_then(|value| value.as_str())
                            .is_some_and(|path| !path.trim().is_empty());
                    if has_pair {
                        2
                    } else {
                        string_list_from_value(args.get("paths"))
                            .into_iter()
                            .chain(string_list_from_value(args.get("targets")))
                            .take(2)
                            .count()
                    }
                }
                Some("count_entries") => {
                    args.get("path")
                        .and_then(Value::as_str)
                        .is_some_and(|path| !path.trim().is_empty()) as usize
                }
                Some("stat_paths") => string_list_from_value(args.get("paths"))
                    .into_iter()
                    .chain(string_list_from_value(args.get("targets")))
                    .chain(string_list_from_value(args.get("path")))
                    .take(2)
                    .count(),
                Some("list_dir") => {
                    args.get("path")
                        .and_then(Value::as_str)
                        .is_some_and(|path| !path.trim().is_empty()) as usize
                }
                _ => 0,
            }
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "config_basic" =>
        {
            match args
                .get("action")
                .and_then(|value| value.as_str())
                .map(|action| action.trim().to_ascii_lowercase())
                .as_deref()
            {
                Some("read_field") => 1,
                Some("read_fields") => args
                    .get("field_paths")
                    .and_then(|value| value.as_array())
                    .map(|field_paths| field_paths.len())
                    .or_else(|| {
                        args.get("fields")
                            .and_then(|value| value.as_array())
                            .map(Vec::len)
                    })
                    .unwrap_or(1),
                Some("list_keys") | Some("validate") => {
                    args.get("path")
                        .and_then(Value::as_str)
                        .is_some_and(|path| !path.trim().is_empty()) as usize
                }
                _ => 0,
            }
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "system_basic" =>
        {
            match args
                .get("action")
                .and_then(|value| value.as_str())
                .map(|action| action.trim().to_ascii_lowercase())
                .as_deref()
            {
                Some("extract_field") => 1,
                Some("extract_fields") => args
                    .get("field_paths")
                    .and_then(|value| value.as_array())
                    .map(|field_paths| field_paths.len())
                    .unwrap_or(1),
                Some("count_inventory") | Some("inventory_dir") => {
                    args.get("path")
                        .and_then(Value::as_str)
                        .is_some_and(|path| !path.trim().is_empty()) as usize
                }
                Some("compare_paths") => {
                    let has_pair = args
                        .get("left_path")
                        .and_then(|value| value.as_str())
                        .is_some_and(|path| !path.trim().is_empty())
                        && args
                            .get("right_path")
                            .and_then(|value| value.as_str())
                            .is_some_and(|path| !path.trim().is_empty());
                    if has_pair {
                        2
                    } else {
                        string_list_from_value(args.get("paths"))
                            .into_iter()
                            .chain(string_list_from_value(args.get("targets")))
                            .take(2)
                            .count()
                    }
                }
                Some("path_batch_facts") => string_list_from_value(args.get("paths"))
                    .into_iter()
                    .chain(string_list_from_value(args.get("targets")))
                    .take(2)
                    .count(),
                _ => 0,
            }
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "git_basic" =>
        {
            match args
                .get("action")
                .and_then(|value| value.as_str())
                .map(|action| action.trim().to_ascii_lowercase())
                .as_deref()
            {
                Some("current_branch" | "rev_parse") => 1,
                _ => 0,
            }
        }
        _ => 0,
    }
}

fn structured_scalar_observation_units(actions: &[AgentAction]) -> usize {
    actions
        .iter()
        .map(action_scalar_compare_observation_units)
        .sum()
}

fn action_is_single_directory_count_observation(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic" =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(|action| action.trim().to_ascii_lowercase())
                .as_deref()
                == Some("count_entries")
                && args
                    .get("path")
                    .and_then(Value::as_str)
                    .is_some_and(|path| !path.trim().is_empty())
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "system_basic" =>
        {
            matches!(
                args.get("action")
                    .and_then(Value::as_str)
                    .map(|action| action.trim().to_ascii_lowercase())
                    .as_deref(),
                Some("count_inventory")
            ) && args
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|path| !path.trim().is_empty())
        }
        _ => false,
    }
}

fn actions_satisfy_single_scalar_count(route: &RouteResult, actions: &[AgentAction]) -> bool {
    if route.output_contract.response_shape != crate::OutputResponseShape::Scalar {
        return false;
    }
    let count_actions = actions
        .iter()
        .filter(|action| action_is_single_directory_count_observation(action))
        .count();
    count_actions == 1
}

fn action_is_single_path_metadata_observation(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let action_name = args
                .get("action")
                .and_then(Value::as_str)
                .map(|action| action.trim().to_ascii_lowercase());
            let paths = string_list_from_value(args.get("paths"))
                .into_iter()
                .chain(string_list_from_value(args.get("targets")))
                .chain(string_list_from_value(args.get("path")))
                .map(|path| path.trim().to_string())
                .filter(|path| !path.is_empty())
                .collect::<Vec<_>>();
            paths.len() == 1
                && ((skill == "fs_basic" && action_name.as_deref() == Some("stat_paths"))
                    || (skill == "system_basic"
                        && action_name.as_deref() == Some("path_batch_facts")))
        }
        AgentAction::Think { .. }
        | AgentAction::CallCapability { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => false,
    }
}

fn actions_satisfy_single_path_metadata_facts(
    route: &RouteResult,
    actions: &[AgentAction],
) -> bool {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::QuantityComparison
        || route.output_contract.delivery_required
        || crate::task_contract::target_locators_for_route(route).len() > 1
    {
        return false;
    }
    let mut metadata_observations = 0usize;
    for action in actions {
        if action_is_single_path_metadata_observation(action) {
            metadata_observations += 1;
            continue;
        }
        if matches!(
            action,
            AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. }
        ) {
            continue;
        }
        return false;
    }
    metadata_observations == 1
}

fn action_is_git_scalar_field_observation(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "git_basic" =>
        {
            matches!(
                args.get("action")
                    .and_then(Value::as_str)
                    .map(|action| action.trim().to_ascii_lowercase())
                    .as_deref(),
                Some("current_branch" | "rev_parse")
            )
        }
        _ => false,
    }
}

fn actions_satisfy_current_workspace_scalar_field_observation(
    route: &RouteResult,
    actions: &[AgentAction],
) -> bool {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::RecentScalarEqualityCheck
        || route.output_contract.locator_kind != crate::OutputLocatorKind::CurrentWorkspace
    {
        return false;
    }
    let mut git_scalar_observations = 0usize;
    for action in actions {
        if action_is_git_scalar_field_observation(action) {
            git_scalar_observations += 1;
            continue;
        }
        if matches!(
            action,
            AgentAction::SynthesizeAnswer { .. }
                | AgentAction::Respond { .. }
                | AgentAction::Think { .. }
        ) {
            continue;
        }
        return false;
    }
    git_scalar_observations == 1
}

fn executed_step_scalar_compare_observation_units(
    step: &crate::executor::StepExecutionResult,
) -> usize {
    if step.is_ok() && step.skill.eq_ignore_ascii_case("git_basic") {
        return 1;
    }
    if !step.is_ok()
        || !(step.skill.eq_ignore_ascii_case("system_basic")
            || step.skill.eq_ignore_ascii_case("fs_basic"))
    {
        return 0;
    }
    let Some(value) = step
        .output
        .as_deref()
        .and_then(|output| serde_json::from_str::<Value>(output).ok())
    else {
        return 0;
    };
    match value
        .get("action")
        .and_then(Value::as_str)
        .map(|action| action.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("extract_field") => 1,
        Some("extract_fields") => value
            .get("field_paths")
            .and_then(Value::as_array)
            .map(|field_paths| field_paths.len())
            .or_else(|| value.get("fields").and_then(Value::as_array).map(Vec::len))
            .unwrap_or(1),
        Some("count_inventory") | Some("inventory_dir") | Some("count_entries") => value
            .get("path")
            .or_else(|| value.get("resolved_path"))
            .and_then(Value::as_str)
            .is_some_and(|path| !path.trim().is_empty())
            as usize,
        Some("compare_paths") => value
            .get("paths")
            .and_then(Value::as_array)
            .map(|paths| paths.len().min(2))
            .unwrap_or(2),
        Some("path_batch_facts") => value
            .get("facts")
            .and_then(Value::as_array)
            .map(|facts| facts.len().min(2))
            .unwrap_or(0),
        _ => 0,
    }
}

fn executed_structured_scalar_observation_units(loop_state: &LoopState) -> usize {
    loop_state
        .executed_step_results
        .iter()
        .map(executed_step_scalar_compare_observation_units)
        .sum()
}

fn structured_scalar_plus_text_evidence(actions: &[AgentAction]) -> bool {
    structured_scalar_observation_units(actions) >= 1
        && actions.iter().any(action_reads_workspace_text_content)
}

fn structured_scalar_compare_missing_required_extracts_for_round(
    route: &RouteResult,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    if !route_requests_structured_scalar_compare(route)
        || !has_executable_observation_or_action(actions)
    {
        return false;
    }
    let scalar_units = structured_scalar_observation_units(actions)
        + executed_structured_scalar_observation_units(loop_state);
    let has_text_evidence = has_workspace_text_content_evidence(loop_state, actions);
    if scalar_units >= 1 && has_text_evidence {
        return false;
    }
    if scalar_units == 1 && actions_satisfy_single_scalar_count(route, actions) {
        return false;
    }
    if scalar_units == 1 && actions_satisfy_single_path_metadata_facts(route, actions) {
        return false;
    }
    if scalar_units == 1
        && actions_satisfy_current_workspace_scalar_field_observation(route, actions)
    {
        return false;
    }
    scalar_units < 2
}

fn append_synthesize_answer_for_structured_scalar_compare(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route_requests_structured_scalar_compare(route)
        || actions.iter().any(|action| {
            matches!(
                action,
                AgentAction::Respond { .. } | AgentAction::SynthesizeAnswer { .. }
            )
        })
    {
        return actions;
    }
    let scalar_units = structured_scalar_observation_units(&actions);
    let has_scalar_text_evidence = structured_scalar_plus_text_evidence(&actions);
    if scalar_units < 2 && !has_scalar_text_evidence {
        return actions;
    }
    let evidence_refs = actions
        .iter()
        .enumerate()
        .filter_map(|(idx, action)| {
            (action_scalar_compare_observation_units(action) > 0
                || (has_scalar_text_evidence && action_reads_workspace_text_content(action)))
            .then(|| format!("step_{}", idx + 1))
        })
        .collect::<Vec<_>>();
    let mut rewritten = actions;
    rewritten.push(AgentAction::SynthesizeAnswer {
        evidence_refs: evidence_refs.clone(),
    });
    info!(
        "plan_append_synthesize_answer_for_structured_scalar_compare refs={}",
        evidence_refs.join(",")
    );
    rewritten
}

fn fs_basic_stat_paths_action_for_explicit_targets(targets: &[String]) -> AgentAction {
    AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "paths": targets,
            "include_missing": true,
            "fields": ["exists", "kind", "size", "modified"],
        }),
    }
}

fn rewrite_constructed_missing_stat_path_to_exact_find_entries(
    state: &AppState,
    route_result: Option<&RouteResult>,
    user_text: &str,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route_allows_constructed_stat_path_search_repair(route) || actions.len() != 1 {
        return actions;
    }
    let Some((path, call_kind)) = single_fs_basic_stat_path_candidate(&actions[0]) else {
        return actions;
    };
    let (root, basename, exact) = if let Some((root, basename)) =
        constructed_missing_stat_path_search_repair(
            &state.skill_rt.workspace_root,
            &path,
            user_text,
        ) {
        (root, basename, true)
    } else if let Some((root, pattern)) = constructed_directory_stat_path_search_repair(
        &state.skill_rt.workspace_root,
        &path,
        user_text,
    ) {
        (root, pattern, false)
    } else {
        return actions;
    };
    info!(
        "plan_rewrite_constructed_missing_stat_path_to_find_entries root={} basename={}",
        crate::truncate_for_log(&root),
        crate::truncate_for_log(&basename)
    );
    let args = serde_json::json!({
        "action": "find_entries",
        "root": root,
        "name_pattern": basename,
        "target_kind": "file",
        "exact": exact,
    });
    match call_kind {
        PlannedCallKind::Tool => vec![AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args,
        }],
        PlannedCallKind::Skill => vec![AgentAction::CallSkill {
            skill: "fs_basic".to_string(),
            args,
        }],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlannedCallKind {
    Tool,
    Skill,
}

fn route_allows_constructed_stat_path_search_repair(route: &RouteResult) -> bool {
    !route.needs_clarify
        && route.is_execute_gate()
        && !route.output_contract.delivery_required
        && route.output_contract.requires_content_evidence
}

fn single_fs_basic_stat_path_candidate(action: &AgentAction) -> Option<(String, PlannedCallKind)> {
    let (name, args, call_kind) = match action {
        AgentAction::CallTool { tool, args } => (tool.as_str(), args, PlannedCallKind::Tool),
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args, PlannedCallKind::Skill),
        _ => return None,
    };
    if !name.eq_ignore_ascii_case("fs_basic") {
        return None;
    }
    let obj = args.as_object()?;
    if obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_none_or(|action| !action.eq_ignore_ascii_case("stat_paths"))
    {
        return None;
    }
    let paths = string_list_from_value(obj.get("paths"))
        .into_iter()
        .chain(string_list_from_value(obj.get("targets")))
        .chain(string_list_from_value(obj.get("path")))
        .collect::<Vec<_>>();
    (paths.len() == 1).then(|| (paths[0].clone(), call_kind))
}

fn constructed_missing_stat_path_search_repair(
    workspace_root: &Path,
    raw_path: &str,
    user_text: &str,
) -> Option<(String, String)> {
    let raw_path = raw_path.trim();
    if raw_path.is_empty() || raw_path.contains(['*', '?', '[', ']']) {
        return None;
    }
    let candidate = resolve_workspace_path(workspace_root, raw_path);
    if candidate.exists() {
        return None;
    }
    if path_text_variants(workspace_root, raw_path, &candidate)
        .iter()
        .any(|variant| structural_token_present(user_text, variant))
    {
        return None;
    }
    let basename = candidate
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| concrete_file_basename_selector(name))?;
    if !structural_token_present(user_text, basename) {
        return None;
    }
    let parent = candidate.parent()?.to_path_buf();
    if !parent.is_dir() {
        return None;
    }
    let raw_parent = Path::new(raw_path)
        .parent()
        .and_then(|parent| parent.to_str());
    let parent_anchored = raw_parent
        .filter(|parent| !parent.trim().is_empty())
        .is_some_and(|parent| structural_token_present(user_text, parent))
        || path_text_variants(workspace_root, raw_parent.unwrap_or_default(), &parent)
            .iter()
            .any(|variant| structural_token_present(user_text, variant));
    if !parent_anchored {
        return None;
    }
    let root = raw_parent
        .map(str::trim)
        .filter(|parent| !parent.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            parent
                .strip_prefix(workspace_root)
                .ok()
                .and_then(|relative| relative.to_str())
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| parent.display().to_string());
    Some((root, basename.to_string()))
}

fn constructed_directory_stat_path_search_repair(
    workspace_root: &Path,
    raw_path: &str,
    user_text: &str,
) -> Option<(String, String)> {
    let raw_path = raw_path.trim();
    if raw_path.is_empty() || raw_path.contains(['*', '?', '[', ']']) {
        return None;
    }
    let candidate = resolve_workspace_path(workspace_root, raw_path);
    if !candidate.is_dir() {
        return None;
    }
    let directory_anchored = structural_token_present(user_text, raw_path)
        || path_text_variants(workspace_root, raw_path, &candidate)
            .iter()
            .any(|variant| structural_token_present(user_text, variant));
    if !directory_anchored {
        return None;
    }
    let pattern =
        directory_child_name_pattern_selector(workspace_root, raw_path, &candidate, user_text)?;
    Some((raw_path.to_string(), pattern))
}

fn directory_child_name_pattern_selector(
    workspace_root: &Path,
    raw_path: &str,
    directory: &Path,
    user_text: &str,
) -> Option<String> {
    let mut path_tokens = structural_selector_tokens(raw_path);
    for component in path_text_variants(workspace_root, raw_path, directory) {
        path_tokens.extend(structural_selector_tokens(&component));
    }
    let mut scores: HashMap<String, usize> = HashMap::new();
    let entries = fs::read_dir(directory).ok()?;
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name
            .to_str()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        for token in structural_selector_tokens(name) {
            if token.len() < 2 || path_tokens.contains(&token) {
                continue;
            }
            if structural_token_present(user_text, &token) {
                *scores.entry(token).or_insert(0) += 1;
            }
        }
    }
    scores
        .into_iter()
        .max_by(|(left_token, left_count), (right_token, right_count)| {
            left_count
                .cmp(right_count)
                .then_with(|| left_token.len().cmp(&right_token.len()))
                .then_with(|| right_token.cmp(left_token))
        })
        .map(|(token, _)| token)
}

fn structural_selector_tokens(text: &str) -> HashSet<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|token| token.len() >= 2)
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

fn path_text_variants(workspace_root: &Path, raw_path: &str, resolved: &Path) -> Vec<String> {
    let mut variants = Vec::new();
    push_unique_path_text_variant(&mut variants, raw_path);
    push_unique_path_text_variant(&mut variants, &raw_path.replace('\\', "/"));
    push_unique_path_text_variant(&mut variants, &resolved.display().to_string());
    if let Ok(relative) = resolved.strip_prefix(workspace_root) {
        if let Some(relative) = relative.to_str() {
            push_unique_path_text_variant(&mut variants, relative);
            push_unique_path_text_variant(&mut variants, &relative.replace('\\', "/"));
        }
    }
    variants
}

fn push_unique_path_text_variant(out: &mut Vec<String>, value: &str) {
    let trimmed = value.trim().trim_end_matches(['/', '\\']);
    if trimmed.is_empty() || out.iter().any(|existing| existing == trimmed) {
        return;
    }
    out.push(trimmed.to_string());
}

fn concrete_file_basename_selector(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty()
        && !trimmed.contains('/')
        && !trimmed.contains('\\')
        && !trimmed.contains(['*', '?', '[', ']', '(', ')', '{', '}', '|'])
        && Path::new(trimmed)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::trim)
            .is_some_and(|ext| !ext.is_empty())
}

fn planned_find_entries_directory_name(action: &AgentAction) -> Option<(String, String)> {
    let (AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }) =
        action
    else {
        return None;
    };
    if !skill.eq_ignore_ascii_case("fs_basic") {
        return None;
    }
    let obj = args.as_object()?;
    if obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|action| action.eq_ignore_ascii_case("find_entries"))?
        .is_empty()
    {
        return None;
    }
    if obj
        .get("ext")
        .or_else(|| obj.get("extension"))
        .is_some_and(has_non_empty_json_value)
    {
        return None;
    }
    let pattern = obj
        .get("pattern")
        .or_else(|| obj.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    if pattern.contains(['*', '?', '[', ']']) || pattern.contains('/') || pattern.contains('\\') {
        return None;
    }
    let root = obj
        .get("root")
        .or_else(|| obj.get("path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(".");
    Some((root.to_string(), pattern.to_string()))
}

fn rewrite_dir_compare_paths_to_unique_workspace_directories(
    state: &AppState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let max_visits = directory_name_locator_scan_limit(state);
    actions
        .into_iter()
        .map(|action| {
            let action = rewrite_dir_compare_action_paths(
                &state.skill_rt.workspace_root,
                max_visits,
                action,
            );
            rewrite_compare_paths_action_to_system_dir_compare(action)
        })
        .collect()
}

fn rewrite_dir_compare_action_paths(
    workspace_root: &Path,
    max_visits: usize,
    action: AgentAction,
) -> AgentAction {
    match action {
        AgentAction::CallSkill { skill, args } => {
            if let Some(args) =
                rewrite_dir_compare_args(workspace_root, max_visits, &skill, args.clone())
            {
                AgentAction::CallSkill { skill, args }
            } else {
                AgentAction::CallSkill { skill, args }
            }
        }
        AgentAction::CallTool { tool, args } => {
            if let Some(args) =
                rewrite_dir_compare_args(workspace_root, max_visits, &tool, args.clone())
            {
                AgentAction::CallTool { tool, args }
            } else {
                AgentAction::CallTool { tool, args }
            }
        }
        other => other,
    }
}

fn rewrite_dir_compare_args(
    workspace_root: &Path,
    max_visits: usize,
    skill: &str,
    args: Value,
) -> Option<Value> {
    if !skill.eq_ignore_ascii_case("system_basic") && !skill.eq_ignore_ascii_case("fs_basic") {
        return None;
    }
    let obj = args.as_object()?;
    let action = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|action| {
            action.eq_ignore_ascii_case("dir_compare")
                || action.eq_ignore_ascii_case("compare_paths")
        })?;
    let (left_raw, right_raw) = if action.eq_ignore_ascii_case("compare_paths") {
        let paths = obj.get("paths").and_then(Value::as_array)?;
        if paths.len() != 2 {
            return None;
        }
        (paths[0].as_str()?, paths[1].as_str()?)
    } else {
        (
            obj.get("left_path")
                .or_else(|| obj.get("left"))
                .and_then(Value::as_str)?,
            obj.get("right_path")
                .or_else(|| obj.get("right"))
                .and_then(Value::as_str)?,
        )
    };
    let left = resolve_dir_compare_path_or_unique_name(workspace_root, left_raw, max_visits)?;
    let right = resolve_dir_compare_path_or_unique_name(workspace_root, right_raw, max_visits)?;
    if left.eq_ignore_ascii_case(left_raw.trim()) && right.eq_ignore_ascii_case(right_raw.trim()) {
        return None;
    }
    let mut rewritten = obj.clone();
    rewritten.insert(
        "action".to_string(),
        Value::String("dir_compare".to_string()),
    );
    rewritten.insert("left_path".to_string(), Value::String(left.clone()));
    rewritten.insert("right_path".to_string(), Value::String(right.clone()));
    rewritten.remove("paths");
    info!(
        "plan_rewrite_dir_compare_paths left={} right={}",
        crate::truncate_for_log(&left),
        crate::truncate_for_log(&right)
    );
    Some(Value::Object(rewritten))
}

fn resolve_dir_compare_path_or_unique_name(
    workspace_root: &Path,
    raw: &str,
    max_visits: usize,
) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let candidate = Path::new(raw);
    let absolute_candidate = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace_root.join(candidate)
    };
    if absolute_candidate.is_dir() {
        return Some(
            absolute_candidate
                .canonicalize()
                .unwrap_or(absolute_candidate)
                .display()
                .to_string(),
        );
    }
    if raw.contains('/') || raw.contains('\\') {
        return None;
    }
    resolve_directory_name_under(workspace_root, ".", raw, max_visits)
        .map(|relative| workspace_root.join(relative))
        .filter(|path| path.is_dir())
        .map(|path| path.canonicalize().unwrap_or(path).display().to_string())
}

fn rewrite_compare_paths_action_to_system_dir_compare(action: AgentAction) -> AgentAction {
    match action {
        AgentAction::CallTool { tool, args }
            if tool.eq_ignore_ascii_case("fs_basic")
                && args
                    .get("action")
                    .and_then(Value::as_str)
                    .is_some_and(|action| action.eq_ignore_ascii_case("dir_compare")) =>
        {
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args,
            }
        }
        AgentAction::CallSkill { skill, args }
            if skill.eq_ignore_ascii_case("fs_basic")
                && args
                    .get("action")
                    .and_then(Value::as_str)
                    .is_some_and(|action| action.eq_ignore_ascii_case("dir_compare")) =>
        {
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args,
            }
        }
        other => other,
    }
}

fn resolve_directory_name_under(
    workspace_root: &Path,
    root_hint: &str,
    name: &str,
    max_visits: usize,
) -> Option<String> {
    let root_path = Path::new(root_hint);
    let root = if root_path.is_absolute() {
        root_path.to_path_buf()
    } else {
        workspace_root.join(root_path)
    };
    if !root.is_dir() {
        return None;
    }
    let mut stack = vec![root];
    let mut matches = Vec::new();
    let mut visits = 0usize;
    while let Some(dir) = stack.pop() {
        visits = visits.saturating_add(1);
        if visits > max_visits {
            break;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut children = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let file_type = entry.file_type().ok()?;
                file_type.is_dir().then(|| entry.path())
            })
            .collect::<Vec<_>>();
        children.sort();
        for child in children.into_iter().rev() {
            if child
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|file_name| file_name.eq_ignore_ascii_case(name))
            {
                matches.push(child.clone());
                if matches.len() > 1 {
                    return None;
                }
            }
            stack.push(child);
        }
    }
    let resolved = matches.pop()?;
    resolved
        .strip_prefix(workspace_root)
        .ok()
        .and_then(|relative| relative.to_str())
        .map(|relative| relative.trim_start_matches('/').to_string())
        .filter(|relative| !relative.is_empty())
        .or_else(|| resolved.to_str().map(ToString::to_string))
}

fn replace_directory_compare_search_plan(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify
        || !route.is_execute_gate()
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::QuantityComparison
    {
        return actions;
    }
    let executable = actions
        .iter()
        .filter(|action| {
            matches!(
                action,
                AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
            )
        })
        .collect::<Vec<_>>();
    if executable.len() != 2
        || actions.iter().any(|action| {
            matches!(
                action,
                AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. }
            )
        })
    {
        return actions;
    }
    let Some((left_root, left_name)) = planned_find_entries_directory_name(executable[0]) else {
        return actions;
    };
    let Some((right_root, right_name)) = planned_find_entries_directory_name(executable[1]) else {
        return actions;
    };
    let max_visits = directory_name_locator_scan_limit(state);
    let Some(left_path) = resolve_directory_name_under(
        &state.skill_rt.workspace_root,
        &left_root,
        &left_name,
        max_visits,
    ) else {
        return actions;
    };
    let Some(right_path) = resolve_directory_name_under(
        &state.skill_rt.workspace_root,
        &right_root,
        &right_name,
        max_visits,
    ) else {
        return actions;
    };
    if left_path.eq_ignore_ascii_case(&right_path) {
        return actions;
    }
    info!(
        "plan_replace_directory_compare_search_with_dir_compare left={} right={}",
        crate::truncate_for_log(&left_path),
        crate::truncate_for_log(&right_path)
    );
    vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "dir_compare",
            "left_path": left_path,
            "right_path": right_path,
            "recursive": true,
            "include_hidden": false,
            "max_diffs": 20,
        }),
    }]
}

fn directory_name_locator_scan_limit(state: &AppState) -> usize {
    state.skill_rt.locator_scan_max_files.max(50_000)
}

fn plan_path_matches_explicit_file_target(path: &str, target: &str) -> bool {
    let Some(path) = normalize_plan_path(path) else {
        return false;
    };
    let Some(target) = normalize_plan_path(target) else {
        return false;
    };
    let path = path.replace('\\', "/");
    let target = target.replace('\\', "/");
    if path.eq_ignore_ascii_case(&target) {
        return true;
    }
    let path_lower = path.to_ascii_lowercase();
    let target_lower = target.to_ascii_lowercase();
    path_lower.ends_with(&format!("/{target_lower}"))
        || Path::new(&path)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(&target))
}

fn action_observed_paths_for_explicit_file_targets(action: &AgentAction) -> Vec<String> {
    let Some(skill) = planned_action_skill_name(action).map(str::trim) else {
        return Vec::new();
    };
    let Some(args) = action_args(action).and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    if skill.eq_ignore_ascii_case("read_file") || skill.eq_ignore_ascii_case("doc_parse") {
        if let Some(path) = args.get("path").and_then(Value::as_str) {
            paths.push(path.to_string());
        }
        return paths;
    }
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();

    if skill.eq_ignore_ascii_case("fs_basic") {
        match action_name.as_str() {
            "stat_paths" | "find_entries" | "compare_paths" => {
                paths.extend(string_list_from_value(args.get("paths")));
                paths.extend(string_list_from_value(args.get("targets")));
                paths.extend(string_list_from_value(args.get("path")));
                if let Some(path) = args.get("left_path").and_then(Value::as_str) {
                    paths.push(path.to_string());
                }
                if let Some(path) = args.get("right_path").and_then(Value::as_str) {
                    paths.push(path.to_string());
                }
            }
            "read_text_range" => {
                if let Some(path) = args.get("path").and_then(Value::as_str) {
                    paths.push(path.to_string());
                }
            }
            _ => {}
        }
        return paths;
    }

    if skill.eq_ignore_ascii_case("config_basic") {
        match action_name.as_str() {
            "read_field" | "read_fields" | "list_keys" | "validate" => {
                if let Some(path) = args.get("path").and_then(Value::as_str) {
                    paths.push(path.to_string());
                }
            }
            _ => {}
        }
        return paths;
    }

    if skill.eq_ignore_ascii_case("system_basic") {
        match action_name.as_str() {
            "path_batch_facts" => {
                paths.extend(string_list_from_value(args.get("paths")));
                paths.extend(string_list_from_value(args.get("targets")));
                paths.extend(string_list_from_value(args.get("path")));
            }
            "compare_paths" => {
                paths.extend(string_list_from_value(args.get("paths")));
                paths.extend(string_list_from_value(args.get("targets")));
                if let Some(path) = args.get("left_path").and_then(Value::as_str) {
                    paths.push(path.to_string());
                }
                if let Some(path) = args.get("right_path").and_then(Value::as_str) {
                    paths.push(path.to_string());
                }
            }
            "read_range" | "extract_field" | "extract_fields" => {
                if let Some(path) = args.get("path").and_then(Value::as_str) {
                    paths.push(path.to_string());
                }
            }
            _ => {}
        }
    }
    paths
}

fn explicit_file_targets_covered_by_plan(actions: &[AgentAction], targets: &[String]) -> Vec<bool> {
    let mut covered = vec![false; targets.len()];
    for action in actions {
        for path in action_observed_paths_for_explicit_file_targets(action) {
            for (idx, target) in targets.iter().enumerate() {
                if !covered[idx] && plan_path_matches_explicit_file_target(&path, target) {
                    covered[idx] = true;
                }
            }
        }
    }
    covered
}

fn explicit_multi_file_metadata_plan_covers_targets(
    actions: &[AgentAction],
    targets: &[String],
) -> bool {
    actions.iter().any(|action| {
        let Some(skill) = planned_action_skill_name(action).map(str::trim) else {
            return false;
        };
        let Some(args) = action_args(action).and_then(Value::as_object) else {
            return false;
        };
        let action_name = args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        let covers_metadata = matches!(
            (
                skill.to_ascii_lowercase().as_str(),
                action_name.to_ascii_lowercase().as_str()
            ),
            ("fs_basic", "stat_paths")
                | ("fs_basic", "compare_paths")
                | ("system_basic", "path_batch_facts")
                | ("system_basic", "compare_paths")
        );
        if !covers_metadata {
            return false;
        }
        explicit_file_targets_covered_by_plan(std::slice::from_ref(action), targets)
            .into_iter()
            .all(|covered| covered)
    })
}

fn ensure_explicit_multi_file_targets_have_path_facts(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || loop_state.has_tool_or_skill_output
    {
        return actions;
    }
    let targets = structured_or_text_multi_file_targets(route, user_text)
        .into_iter()
        .take(4)
        .collect::<Vec<_>>();
    if targets.len() < 2 || explicit_multi_file_metadata_plan_covers_targets(&actions, &targets) {
        return actions;
    }

    if !route_requests_path_metadata_compare(route) {
        return actions;
    }

    if structured_scalar_plus_text_evidence(&actions) {
        return actions;
    }

    if structured_scalar_observation_units(&actions) >= 2 {
        return actions;
    }

    info!(
        "plan_replace_scalar_multi_file_read_with_fs_basic_stat_paths targets={}",
        targets.join(",")
    );
    vec![fs_basic_stat_paths_action_for_explicit_targets(&targets)]
}

fn ensure_existence_multi_file_targets_have_path_facts(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify
        || !route.is_execute_gate()
        || route.output_contract.delivery_required
        || loop_state.has_tool_or_skill_output
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::ExistenceWithPath
    {
        return actions;
    }
    let targets = structured_or_text_multi_file_targets(route, user_text)
        .into_iter()
        .take(8)
        .collect::<Vec<_>>();
    if targets.len() < 2 || explicit_multi_file_metadata_plan_covers_targets(&actions, &targets) {
        return actions;
    }
    if !actions.iter().any(planned_action_is_path_metadata_facts) {
        return actions;
    }
    info!(
        "plan_replace_incomplete_existence_multi_file_stat_paths targets={}",
        targets.join(",")
    );
    vec![fs_basic_stat_paths_action_for_explicit_targets(&targets)]
}

fn rewrite_unresolved_template_arg_multi_file_read_plan(
    route_result: Option<&RouteResult>,
    user_text: &str,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || !actions.iter().any(action_args_contain_unresolved_template)
    {
        return actions;
    }
    let file_targets = structured_or_text_multi_file_targets(route, user_text)
        .into_iter()
        .filter(|target| filename_candidate_has_document_extension(target))
        .collect::<Vec<_>>();
    if file_targets.len() < 2 {
        return actions;
    }

    let mut rewritten = Vec::new();
    for target in file_targets.iter().take(4) {
        rewritten.push(AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": target,
                "mode": "head",
                "n": 40,
            }),
        });
    }
    let evidence_refs = (1..=rewritten.len())
        .map(|idx| format!("step_{idx}"))
        .collect::<Vec<_>>();
    rewritten.push(AgentAction::SynthesizeAnswer {
        evidence_refs: evidence_refs.clone(),
    });
    rewritten.push(AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    });
    info!(
        "plan_rewrite_unresolved_template_arg_multi_file_read_plan targets={} refs={}",
        file_targets.join(","),
        evidence_refs.join(",")
    );
    rewritten
}

fn resolve_existing_file_target_from_token(state: &AppState, token: &str) -> Option<String> {
    let token = token
        .trim()
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\''
                    | '`'
                    | '<'
                    | '>'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | ','
                    | '，'
                    | ';'
                    | '；'
                    | '。'
                    | ':'
                    | '：'
                    | '\\'
            )
        })
        .trim();
    if token.is_empty() {
        return None;
    }
    let path = Path::new(token);
    if !path.is_absolute()
        && !token.starts_with("./")
        && !token.starts_with("../")
        && !(path.components().count() > 1 && path.extension().is_some())
    {
        return None;
    }
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(path)
    };
    if !resolved.is_file() {
        return None;
    }
    Some(resolved.display().to_string())
}

fn collect_existing_file_targets_from_text(state: &AppState, text: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for token in text.split(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                '"' | '\''
                    | '`'
                    | '<'
                    | '>'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | ','
                    | '，'
                    | ';'
                    | '；'
                    | '。'
                    | ':'
                    | '：'
                    | '\\'
            )
    }) {
        let Some(path) = resolve_existing_file_target_from_token(state, token) else {
            continue;
        };
        if !targets.iter().any(|existing: &String| existing == &path) {
            targets.push(path);
        }
    }
    targets
}

fn collect_file_targets_from_route_scope(
    state: &AppState,
    route: &RouteResult,
    user_text: &str,
) -> Vec<String> {
    let mut targets = Vec::new();
    for target in structured_or_text_multi_file_targets(route, user_text) {
        if let Some(path) = resolve_existing_file_target_from_token(state, &target) {
            if !targets.iter().any(|existing: &String| existing == &path) {
                targets.push(path);
            }
        }
    }
    for source in [
        route.resolved_intent.as_str(),
        route.route_reason.as_str(),
        route.output_contract.locator_hint.as_str(),
    ] {
        for path in collect_existing_file_targets_from_text(state, source) {
            if !targets.iter().any(|existing: &String| existing == &path) {
                targets.push(path);
            }
        }
    }
    targets
}

fn scoped_plan_context_file_targets(state: &AppState, plan_context: Option<&str>) -> Vec<String> {
    let Some(plan_context) = plan_context else {
        return Vec::new();
    };
    let mut targets = Vec::new();
    if let Some((_, tail)) = plan_context.split_once("### RECENT_EXECUTION_EVENTS") {
        let section = tail
            .split("\n\nDirect answer gate resolved execution intent:")
            .next()
            .unwrap_or(tail);
        let mut event_request_targets = Vec::new();
        for line in section.lines() {
            let Some((_, request_tail)) = line.split_once(" request=") else {
                continue;
            };
            let request = request_tail
                .split(" result=")
                .next()
                .unwrap_or(request_tail)
                .trim();
            for path in collect_existing_file_targets_from_text(state, request) {
                if !event_request_targets
                    .iter()
                    .any(|existing: &String| existing == &path)
                {
                    event_request_targets.push(path);
                }
            }
        }
        event_request_targets.reverse();
        for path in event_request_targets {
            if !targets.iter().any(|existing: &String| existing == &path) {
                targets.push(path);
            }
        }
    }
    for marker in [
        "Direct answer gate resolved execution intent:",
        "Resolved semantic request:",
        "Turn analysis:",
    ] {
        let Some((_, tail)) = plan_context.split_once(marker) else {
            continue;
        };
        let section = tail.split("\n\n").next().unwrap_or(tail).trim();
        for path in collect_existing_file_targets_from_text(state, section) {
            if !targets.iter().any(|existing: &String| existing == &path) {
                targets.push(path);
            }
        }
    }
    targets
}

fn replace_content_evidence_synthesize_only_with_file_reads(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    plan_context: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify
        || !route.is_execute_gate()
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || loop_state.has_tool_or_skill_output
        || actions.iter().any(|action| {
            matches!(
                action,
                AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
            )
        })
        || !actions.iter().any(|action| {
            matches!(
                action,
                AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. }
            )
        })
    {
        return actions;
    }

    let mut targets = collect_file_targets_from_route_scope(state, route, user_text);
    if targets.len() < 2 {
        for path in scoped_plan_context_file_targets(state, plan_context) {
            if !targets.iter().any(|existing| existing == &path) {
                targets.push(path);
            }
        }
    }
    if targets.len() < 2 {
        return actions;
    }
    let targets = targets.into_iter().take(4).collect::<Vec<_>>();
    let mut rewritten = Vec::new();
    for path in &targets {
        rewritten.push(AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": path,
                "mode": "head",
                "n": 60,
            }),
        });
    }
    let evidence_refs = (1..=rewritten.len())
        .map(|idx| format!("step_{idx}"))
        .collect::<Vec<_>>();
    rewritten.push(AgentAction::SynthesizeAnswer {
        evidence_refs: evidence_refs.clone(),
    });
    rewritten.push(AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    });
    info!(
        "plan_replace_synthesize_only_content_evidence_with_file_reads targets={} refs={}",
        targets.join(","),
        evidence_refs.join(",")
    );
    rewritten
}

fn strip_unresolved_template_reads_after_inventory_dir(
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let mut saw_locator_listing = false;
    let mut old_to_new: Vec<Option<usize>> = vec![None; actions.len() + 1];
    let mut stripped = Vec::new();
    let mut stripped_indices = Vec::new();

    for (idx, action) in actions.into_iter().enumerate() {
        let old_idx = idx + 1;
        if saw_locator_listing
            && is_unresolved_template_read_action(&action)
            && !is_indexed_last_output_read_action(&action)
        {
            stripped_indices.push(old_idx);
            continue;
        }
        if is_locator_listing_action(&action) {
            saw_locator_listing = true;
        }
        old_to_new[old_idx] = Some(stripped.len() + 1);
        stripped.push(action);
    }

    if stripped_indices.is_empty() {
        return stripped;
    }

    for action in &mut stripped {
        if let AgentAction::SynthesizeAnswer { evidence_refs } = action {
            *evidence_refs = rewrite_evidence_refs_after_step_strip(evidence_refs, &old_to_new);
        }
    }

    info!(
        "plan_strip_unresolved_template_reads_after_inventory_dir stripped_steps={}",
        stripped_indices
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    stripped
}

fn is_locator_listing_action(action: &AgentAction) -> bool {
    is_fs_basic_listing_action(action)
        || is_system_basic_inventory_dir_action(action)
        || is_fs_search_observation_action(action)
}

fn is_fs_basic_listing_action(action: &AgentAction) -> bool {
    matches!(
        action,
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("fs_basic")
                && args
                    .get("action")
                    .and_then(Value::as_str)
                    .is_some_and(|action| action.eq_ignore_ascii_case("list_dir"))
    )
}

fn is_system_basic_inventory_dir_action(action: &AgentAction) -> bool {
    matches!(
        system_basic_action_path(action),
        Some((action_name, _)) if action_name == "inventory_dir"
    )
}

fn is_fs_search_observation_action(action: &AgentAction) -> bool {
    matches!(
        action,
        AgentAction::CallSkill { skill, .. } | AgentAction::CallTool { tool: skill, .. }
            if skill.eq_ignore_ascii_case("fs_search")
    )
}

fn is_unresolved_template_read_action(action: &AgentAction) -> bool {
    let (AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }) =
        action
    else {
        return false;
    };
    if !value_contains_unresolved_template(args) {
        return false;
    }
    if skill == "read_file" {
        return true;
    }
    if skill == "fs_basic" {
        return args
            .as_object()
            .and_then(|obj| obj.get("action"))
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|action_name| action_name.eq_ignore_ascii_case("read_text_range"));
    }
    if skill != "system_basic" {
        return false;
    }
    args.as_object()
        .and_then(|obj| obj.get("action"))
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|action_name| action_name.eq_ignore_ascii_case("read_range"))
}

fn is_indexed_last_output_read_action(action: &AgentAction) -> bool {
    let (AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }) =
        action
    else {
        return false;
    };
    if skill != "system_basic" && skill != "fs_basic" {
        return false;
    }
    let Some(obj) = args.as_object() else {
        return false;
    };
    let is_read_range = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|action_name| {
            action_name.eq_ignore_ascii_case("read_range")
                || action_name.eq_ignore_ascii_case("read_text_range")
        });
    if !is_read_range {
        return false;
    }
    let Some(path) = obj.get("path").and_then(Value::as_str) else {
        return false;
    };
    static LAST_OUTPUT_INDEX_RE: OnceLock<Regex> = OnceLock::new();
    let re = LAST_OUTPUT_INDEX_RE.get_or_init(|| {
        Regex::new(r"\{\{\s*last_output(?:\.\d+|\[\s*\d+\s*\])\s*\}\}")
            .expect("last_output indexed placeholder regex")
    });
    re.is_match(path)
}

fn action_args_contain_unresolved_template(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { args, .. }
        | AgentAction::CallTool { args, .. }
        | AgentAction::CallCapability { args, .. } => value_contains_unresolved_template(args),
        AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => false,
    }
}

fn value_contains_unresolved_template(value: &Value) -> bool {
    match value {
        Value::String(text) => {
            let text = text.trim();
            text.contains("{{") && text.contains("}}")
        }
        Value::Array(items) => items.iter().any(value_contains_unresolved_template),
        Value::Object(map) => map.values().any(value_contains_unresolved_template),
        _ => false,
    }
}

fn explicit_document_file_targets(user_text: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for candidate in explicit_document_path_targets(user_text) {
        if document_target_already_covered(&targets, &candidate) {
            continue;
        }
        targets.push(candidate);
    }
    for candidate in crate::delivery_utils::extract_filename_candidates(user_text) {
        if !filename_candidate_has_document_extension(&candidate)
            || document_target_already_covered(&targets, &candidate)
        {
            continue;
        }
        targets.push(candidate);
    }
    targets
}

fn explicit_document_path_targets(user_text: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for token in user_text.split_whitespace() {
        let candidate = trim_structural_document_target_token(token);
        if candidate.is_empty()
            || !(candidate.contains('/') || candidate.contains('\\'))
            || candidate.contains("://")
            || crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(
                &candidate,
            )
            || !filename_candidate_has_document_extension(&candidate)
            || document_target_already_covered(&targets, &candidate)
        {
            continue;
        }
        targets.push(candidate);
    }
    targets
}

fn trim_structural_document_target_token(token: &str) -> String {
    token
        .trim()
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\''
                    | '`'
                    | ','
                    | '，'
                    | '。'
                    | ':'
                    | '：'
                    | ';'
                    | '；'
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | '《'
                    | '》'
            )
        })
        .to_string()
}

fn document_target_already_covered(targets: &[String], candidate: &str) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return true;
    }
    let candidate_basename = Path::new(candidate)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(candidate);
    targets.iter().any(|existing| {
        if existing.eq_ignore_ascii_case(candidate) {
            return true;
        }
        let existing_basename = Path::new(existing)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(existing);
        existing_basename.eq_ignore_ascii_case(candidate_basename)
            && (existing.contains('/') || existing.contains('\\'))
            && !(candidate.contains('/') || candidate.contains('\\'))
    })
}

fn structured_or_text_multi_file_targets(route: &RouteResult, user_text: &str) -> Vec<String> {
    let structured_targets = crate::task_contract::target_locators_for_route(route)
        .into_iter()
        .filter(|target| target.trim() != ".")
        .collect::<Vec<_>>();
    if structured_targets.len() >= 2 {
        return structured_targets;
    }
    // Deprecated compatibility fallback: keep this limited to structural filename
    // tokens and document-like extensions. Semantic target selection should come
    // from TaskContract/route output, not language-specific phrases.
    explicit_document_file_targets(user_text)
}

fn filename_candidate_has_document_extension(candidate: &str) -> bool {
    let Some((_, ext)) = candidate.rsplit_once('.') else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "md" | "txt" | "json" | "toml" | "yaml" | "yml" | "rs" | "log" | "sqlite" | "db" | "csv"
    )
}

fn locator_identity_token(value: &str) -> String {
    value
        .trim()
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\''
                    | '`'
                    | ','
                    | '.'
                    | '，'
                    | '。'
                    | ':'
                    | '：'
                    | ';'
                    | '；'
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '['
                    | ']'
                    | '【'
                    | '】'
            )
        })
        .to_ascii_lowercase()
}

fn locator_hint_names_workspace_root(workspace_root: &Path, locator_hint: &str) -> bool {
    let Some(root_name) = workspace_root.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let normalized_root = locator_identity_token(root_name);
    let normalized_hint = locator_identity_token(locator_hint);
    !normalized_root.is_empty() && normalized_hint == normalized_root
}

fn prune_unscoped_workspace_summary_evidence_for_scope(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    let scope_hint = route.output_contract.locator_hint.trim();
    if route.needs_clarify
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::WorkspaceProjectSummary
        || scope_hint.is_empty()
    {
        return actions;
    }
    if locator_hint_names_workspace_root(&state.skill_rt.workspace_root, scope_hint) {
        return actions;
    }
    let has_scoped_evidence = actions.iter().any(|action| {
        action_is_workspace_summary_evidence(action)
            && action_workspace_summary_path(action)
                .is_some_and(|path| path_matches_workspace_scope_hint(path, scope_hint))
    });
    if !has_scoped_evidence {
        return actions;
    }
    let original_len = actions.len();
    let pruned = actions
        .into_iter()
        .filter(|action| {
            !action_is_workspace_summary_evidence(action)
                || action_workspace_summary_path(action)
                    .is_some_and(|path| path_matches_workspace_scope_hint(path, scope_hint))
        })
        .collect::<Vec<_>>();
    if pruned.is_empty() {
        return pruned;
    }
    if pruned.len() != original_len {
        info!(
            "plan_prune_workspace_summary_unscoped_evidence scope={} removed={}",
            scope_hint,
            original_len.saturating_sub(pruned.len())
        );
    }
    pruned
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SingleFileReadActionKind {
    ReadFile,
    SystemBasicReadRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SingleStructuredFieldReadActionKind {
    ConfigBasic,
    SystemBasic,
}

fn single_file_read_action(
    actions: &[AgentAction],
) -> Option<(usize, SingleFileReadActionKind, String)> {
    let mut candidate: Option<(usize, SingleFileReadActionKind, String)> = None;
    for (idx, action) in actions.iter().enumerate() {
        match action {
            AgentAction::Think { .. }
            | AgentAction::Respond { .. }
            | AgentAction::SynthesizeAnswer { .. } => {}
            AgentAction::CallSkill { skill, args } if skill.eq_ignore_ascii_case("read_file") => {
                let Some(path) = args.get("path").and_then(|value| value.as_str()) else {
                    return None;
                };
                if candidate.is_some() {
                    return None;
                }
                candidate = Some((
                    idx,
                    SingleFileReadActionKind::ReadFile,
                    path.trim().to_string(),
                ));
            }
            AgentAction::CallSkill { skill, args }
                if skill.eq_ignore_ascii_case("system_basic")
                    && args
                        .get("action")
                        .and_then(|value| value.as_str())
                        .is_some_and(|action| action.eq_ignore_ascii_case("read_range")) =>
            {
                let Some(path) = args.get("path").and_then(|value| value.as_str()) else {
                    return None;
                };
                if candidate.is_some() {
                    return None;
                }
                candidate = Some((
                    idx,
                    SingleFileReadActionKind::SystemBasicReadRange,
                    path.trim().to_string(),
                ));
            }
            _ => return None,
        }
    }
    candidate
}

fn single_structured_field_read_action(
    actions: &[AgentAction],
) -> Option<(usize, SingleStructuredFieldReadActionKind, String)> {
    let mut candidate: Option<(usize, SingleStructuredFieldReadActionKind, String)> = None;
    for (idx, action) in actions.iter().enumerate() {
        match action {
            AgentAction::Think { .. }
            | AgentAction::Respond { .. }
            | AgentAction::SynthesizeAnswer { .. } => {}
            AgentAction::CallSkill { skill, args }
            | AgentAction::CallTool { tool: skill, args } => {
                let action_name = args
                    .get("action")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or_default();
                let kind = if skill.eq_ignore_ascii_case("config_basic")
                    && matches!(action_name, "read_field" | "read_fields")
                {
                    SingleStructuredFieldReadActionKind::ConfigBasic
                } else if skill.eq_ignore_ascii_case("system_basic")
                    && matches!(action_name, "extract_field" | "extract_fields")
                {
                    SingleStructuredFieldReadActionKind::SystemBasic
                } else {
                    return None;
                };
                let Some(path) = args.get("path").and_then(Value::as_str) else {
                    return None;
                };
                if candidate.is_some() {
                    return None;
                }
                candidate = Some((idx, kind, path.trim().to_string()));
            }
            AgentAction::CallCapability { .. } => return None,
        }
    }
    candidate
}

fn rewrite_single_target_structured_field_read_to_auto_locator(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route_result) = route_result else {
        return actions;
    };
    if route_result.needs_clarify
        || route_result.output_contract.delivery_required
        || !route_result.output_contract.requires_content_evidence
        || !matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        )
    {
        return actions;
    }
    let Some(auto_locator_path) = auto_locator_path
        .map(str::trim)
        .filter(|value| !value.is_empty() && path_has_structured_document_extension(value))
    else {
        return actions;
    };
    let auto_locator = std::path::Path::new(auto_locator_path);
    if !auto_locator.is_file() {
        return actions;
    }
    let Some((idx, kind, current_path)) = single_structured_field_read_action(&actions) else {
        return actions;
    };
    if same_existing_or_display_path(std::path::Path::new(&current_path), auto_locator) {
        return actions;
    }

    let mut rewritten = actions;
    let Some(action) = rewritten.get_mut(idx) else {
        return rewritten;
    };
    match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => {
            if let Some(obj) = args.as_object_mut() {
                obj.insert(
                    "path".to_string(),
                    Value::String(auto_locator_path.to_string()),
                );
            }
        }
        _ => return rewritten,
    }
    info!(
        "plan_rewrite_single_target_structured_field_read_to_auto_locator idx={} kind={:?} from={} to={}",
        idx, kind, current_path, auto_locator_path
    );
    rewritten
}

fn rewrite_single_target_file_read_to_auto_locator(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route_result) = route_result else {
        return actions;
    };
    if route_result.needs_clarify || route_result.output_contract.delivery_required {
        return actions;
    }
    let Some(auto_locator_path) = auto_locator_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return actions;
    };
    let auto_locator = std::path::Path::new(auto_locator_path);
    if !auto_locator.is_file() {
        return actions;
    }
    let Some((idx, kind, current_path)) = single_file_read_action(&actions) else {
        return actions;
    };
    if current_path == auto_locator_path {
        return actions;
    }

    // 当前轮 route/ordinal/auto-locator 已解析成一个具体文件时，这个路径比 LLM
    // 在厚上下文里“顺手抄到的旧文件路径”更权威。这里只在单目标读文件链路上收口，
    // 避免把多文件 read 计划错误折叠成同一个 target。
    let mut rewritten = actions;
    let Some(action) = rewritten.get_mut(idx) else {
        return rewritten;
    };
    match action {
        AgentAction::CallSkill { args, .. } => {
            if let Some(obj) = args.as_object_mut() {
                obj.insert(
                    "path".to_string(),
                    Value::String(auto_locator_path.to_string()),
                );
            }
        }
        _ => return rewritten,
    }
    info!(
        "plan_rewrite_single_target_file_read_to_auto_locator idx={} kind={:?} from={} to={}",
        idx, kind, current_path, auto_locator_path
    );
    rewritten
}

fn rewrite_extract_field_alias_args(actions: Vec<AgentAction>) -> Vec<AgentAction> {
    let mut rewritten = actions;
    for (idx, action) in rewritten.iter_mut().enumerate() {
        let AgentAction::CallSkill { skill, args } = action else {
            continue;
        };
        if !skill.eq_ignore_ascii_case("system_basic") {
            continue;
        }
        let Some(obj) = args.as_object_mut() else {
            continue;
        };
        let is_extract_field = obj
            .get("action")
            .and_then(|value| value.as_str())
            .is_some_and(|action| action.eq_ignore_ascii_case("extract_field"));
        if !is_extract_field || obj.contains_key("field_path") {
        } else if let Some(field_value) = obj.remove("field") {
            let Value::String(field_path) = field_value else {
                obj.insert("field".to_string(), field_value);
                continue;
            };
            let trimmed = field_path.trim();
            if trimmed.is_empty() {
                obj.insert("field".to_string(), Value::String(field_path));
                continue;
            }
            obj.insert("field_path".to_string(), Value::String(trimmed.to_string()));
            info!(
                "plan_rewrite_extract_field_alias idx={} alias=field canonical=field_path",
                idx
            );
        }
        if !obj.contains_key("path") {
            if let Some(path_value) = obj.remove("file_path") {
                match path_value {
                    Value::String(path) if !path.trim().is_empty() => {
                        obj.insert("path".to_string(), Value::String(path.trim().to_string()));
                        info!(
                            "plan_rewrite_extract_field_alias idx={} alias=file_path canonical=path",
                            idx
                        );
                    }
                    other => {
                        obj.insert("file_path".to_string(), other);
                    }
                }
            }
        }
        if !obj.contains_key("path") {
            if let Some(path_value) = obj.remove("target") {
                match path_value {
                    Value::String(path) if !path.trim().is_empty() => {
                        obj.insert("path".to_string(), Value::String(path.trim().to_string()));
                        info!(
                            "plan_rewrite_extract_field_alias idx={} alias=target canonical=path",
                            idx
                        );
                    }
                    other => {
                        obj.insert("target".to_string(), other);
                    }
                }
            }
        }
    }
    rewritten
}

#[derive(Debug, Clone)]
struct StructuredExtractRequest {
    path: String,
    fields: Vec<String>,
}

#[derive(Debug, Clone)]
struct StructuredFieldCandidate {
    path: PathBuf,
    depth: usize,
    package_name: Option<String>,
}

fn rewrite_extract_field_paths_to_structured_candidates(
    state: &AppState,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify || route.output_contract.delivery_required {
        return actions;
    }

    let mut rewritten = actions;
    for (idx, action) in rewritten.iter_mut().enumerate() {
        let (skill, args) = match action {
            AgentAction::CallSkill { skill, args }
            | AgentAction::CallTool { tool: skill, args } => (skill, args),
            _ => {
                continue;
            }
        };
        if !skill.eq_ignore_ascii_case("system_basic")
            && !skill.eq_ignore_ascii_case("config_basic")
        {
            continue;
        }
        let Some(request) = structured_extract_request(args) else {
            continue;
        };
        let current = resolve_workspace_path(&state.skill_rt.workspace_root, &request.path);
        if rewrite_cargo_workspace_package_fields_to_workspace_package(
            args,
            &state.skill_rt.workspace_root,
            &current,
            &request,
        ) {
            continue;
        }
        if structured_file_has_all_fields(&current, &request.fields) {
            continue;
        }
        if !should_repair_structured_extract_path(
            route,
            &state.skill_rt.workspace_root,
            &request.path,
            &current,
            auto_locator_path,
        ) {
            continue;
        }

        let Some(replacement) = find_structured_field_candidate(
            &state.skill_rt.workspace_root,
            &current,
            &request.fields,
            state.skill_rt.locator_scan_max_files,
        ) else {
            continue;
        };
        let replacement_text = replacement.display().to_string();
        let Some(obj) = args.as_object_mut() else {
            continue;
        };
        obj.insert("path".to_string(), Value::String(replacement_text.clone()));
        info!(
            "plan_rewrite_extract_field_path idx={} from={} to={} fields={:?}",
            idx, request.path, replacement_text, request.fields
        );
    }
    rewritten
}

fn rewrite_cargo_workspace_package_fields_to_workspace_package(
    args: &mut Value,
    workspace_root: &Path,
    current: &Path,
    request: &StructuredExtractRequest,
) -> bool {
    if current.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml") {
        return false;
    }
    let Some((target_path, rewritten_fields)) =
        resolve_cargo_workspace_package_fields(workspace_root, current, &request.fields)
    else {
        return false;
    };
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    let action_name = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if matches!(
        action_name.to_ascii_lowercase().as_str(),
        "extract_field" | "read_field"
    ) && rewritten_fields.len() == 1
    {
        obj.insert(
            "field_path".to_string(),
            Value::String(rewritten_fields[0].clone()),
        );
    } else if matches!(
        action_name.to_ascii_lowercase().as_str(),
        "extract_fields" | "read_fields"
    ) {
        obj.remove("fields");
        obj.insert(
            "field_paths".to_string(),
            Value::Array(
                rewritten_fields
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
    } else {
        return false;
    }
    obj.insert(
        "path".to_string(),
        Value::String(target_path.display().to_string()),
    );
    info!(
        "plan_rewrite_cargo_workspace_package_fields from={} to={} fields={:?}",
        current.display(),
        target_path.display(),
        rewritten_fields
    );
    true
}

fn resolve_cargo_workspace_package_fields(
    workspace_root: &Path,
    current: &Path,
    fields: &[String],
) -> Option<(PathBuf, Vec<String>)> {
    let current_value = parse_structured_file_value(current)?;
    let rewritten_fields = cargo_workspace_package_field_paths(fields)?;
    if lookup_structured_field_value(&current_value, "workspace").is_some()
        && lookup_structured_field_value(&current_value, "package").is_none()
        && rewritten_fields
            .iter()
            .all(|field| lookup_structured_field_value(&current_value, field).is_some())
    {
        return Some((current.to_path_buf(), rewritten_fields));
    }
    if !fields.iter().all(|field| {
        lookup_structured_field_value(&current_value, field)
            .is_some_and(is_cargo_workspace_inherited_marker)
    }) {
        return None;
    }
    let target =
        find_cargo_workspace_manifest_with_fields(workspace_root, current, &rewritten_fields)?;
    Some((target, rewritten_fields))
}

fn cargo_workspace_package_field_paths(fields: &[String]) -> Option<Vec<String>> {
    let mut rewritten = Vec::with_capacity(fields.len());
    for field in fields {
        let suffix = field.strip_prefix("package.")?;
        if suffix.trim().is_empty() {
            return None;
        }
        rewritten.push(format!("workspace.package.{suffix}"));
    }
    Some(rewritten)
}

fn is_cargo_workspace_inherited_marker(value: &Value) -> bool {
    value.as_object().is_some_and(|obj| {
        obj.len() == 1
            && obj
                .get("workspace")
                .and_then(Value::as_bool)
                .unwrap_or(false)
    })
}

fn find_cargo_workspace_manifest_with_fields(
    workspace_root: &Path,
    current: &Path,
    fields: &[String],
) -> Option<PathBuf> {
    let workspace_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let mut dir = current.parent()?.to_path_buf();
    loop {
        let manifest = dir.join("Cargo.toml");
        if !same_existing_or_display_path(&manifest, current)
            && manifest.is_file()
            && parse_structured_file_value(&manifest).is_some_and(|value| {
                lookup_structured_field_value(&value, "workspace").is_some()
                    && fields
                        .iter()
                        .all(|field| lookup_structured_field_value(&value, field).is_some())
            })
        {
            return Some(manifest);
        }
        if same_existing_or_display_path(&dir, &workspace_root) || !dir.pop() {
            break;
        }
    }
    None
}

fn structured_extract_request(args: &Value) -> Option<StructuredExtractRequest> {
    let obj = args.as_object()?;
    let action = obj.get("action").and_then(Value::as_str)?;
    let path = obj
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let mut fields = match action {
        action
            if matches!(
                action.to_ascii_lowercase().as_str(),
                "extract_field" | "read_field"
            ) =>
        {
            obj.get("field_path")
                .or_else(|| obj.get("field"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| vec![value.to_string()])
                .unwrap_or_default()
        }
        action
            if matches!(
                action.to_ascii_lowercase().as_str(),
                "extract_fields" | "read_fields"
            ) =>
        {
            string_list_from_value(obj.get("field_paths").or_else(|| obj.get("fields")))
                .into_iter()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect()
        }
        _ => Vec::new(),
    };
    fields.sort();
    fields.dedup();
    if fields.is_empty() {
        return None;
    }
    Some(StructuredExtractRequest { path, fields })
}

fn should_repair_structured_extract_path(
    route: &RouteResult,
    workspace_root: &Path,
    raw_path: &str,
    current: &Path,
    auto_locator_path: Option<&str>,
) -> bool {
    let raw = Path::new(raw_path);
    let raw_is_bare_filename = raw.components().count() == 1;
    if raw_is_bare_filename {
        return true;
    }
    if !matches!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Filename
    ) {
        return false;
    }
    if !is_workspace_root_direct_child(workspace_root, current) {
        return false;
    }
    auto_locator_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|auto_path| same_existing_or_display_path(Path::new(auto_path), current))
        .unwrap_or(true)
}

fn resolve_workspace_path(workspace_root: &Path, raw_path: &str) -> PathBuf {
    let candidate = Path::new(raw_path);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace_root.join(candidate)
    }
}

fn same_existing_or_display_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn is_workspace_root_direct_child(workspace_root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(workspace_root) else {
        return false;
    };
    relative.components().count() == 1
}

fn find_structured_field_candidate(
    workspace_root: &Path,
    current: &Path,
    fields: &[String],
    scan_max_files: usize,
) -> Option<PathBuf> {
    let file_name = current.file_name()?.to_os_string();
    let prefer_current_package = workspace_manifest_lacks_package_fields(current, fields);
    let max_files = scan_max_files.max(500);
    let mut candidates = Vec::new();
    let mut seen_files = 0usize;
    collect_structured_field_candidates(
        workspace_root,
        workspace_root,
        &file_name,
        fields,
        max_files,
        &mut seen_files,
        &mut candidates,
    );
    if prefer_current_package {
        let current_package_name = env!("CARGO_PKG_NAME");
        let preferred: Vec<_> = candidates
            .iter()
            .filter(|candidate| candidate.package_name.as_deref() == Some(current_package_name))
            .collect();
        if preferred.len() == 1 {
            return Some(preferred[0].path.clone());
        }
    }

    candidates.sort_by(|left, right| {
        left.depth
            .cmp(&right.depth)
            .then_with(|| left.path.cmp(&right.path))
    });
    let best = candidates.first()?;
    if candidates
        .get(1)
        .is_some_and(|next| next.depth == best.depth)
    {
        return None;
    }
    Some(best.path.clone())
}

fn collect_structured_field_candidates(
    workspace_root: &Path,
    dir: &Path,
    file_name: &std::ffi::OsStr,
    fields: &[String],
    max_files: usize,
    seen_files: &mut usize,
    out: &mut Vec<StructuredFieldCandidate>,
) {
    if *seen_files >= max_files {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if should_prune_structured_candidate_dir(&entry.file_name()) {
                continue;
            }
            collect_structured_field_candidates(
                workspace_root,
                &path,
                file_name,
                fields,
                max_files,
                seen_files,
                out,
            );
            if *seen_files >= max_files {
                return;
            }
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        *seen_files += 1;
        if entry.file_name() != file_name {
            continue;
        }
        let Some(root_value) = parse_structured_file_value(&path) else {
            continue;
        };
        if !fields
            .iter()
            .all(|field| lookup_structured_field_value(&root_value, field).is_some())
        {
            continue;
        }
        out.push(StructuredFieldCandidate {
            depth: path
                .strip_prefix(workspace_root)
                .ok()
                .map(|relative| relative.components().count())
                .unwrap_or(usize::MAX),
            package_name: lookup_structured_field_value(&root_value, "package.name")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            path,
        });
    }
}

fn should_prune_structured_candidate_dir(name: &std::ffi::OsStr) -> bool {
    matches!(
        name.to_string_lossy().as_ref(),
        ".git"
            | "target"
            | "node_modules"
            | ".venv"
            | "venv"
            | "dist"
            | "build"
            | ".next"
            | ".cache"
    )
}

fn structured_file_has_all_fields(path: &Path, fields: &[String]) -> bool {
    let Some(root_value) = parse_structured_file_value(path) else {
        return false;
    };
    fields
        .iter()
        .all(|field| lookup_structured_field_value(&root_value, field).is_some())
}

fn workspace_manifest_lacks_package_fields(path: &Path, fields: &[String]) -> bool {
    if path.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml") {
        return false;
    }
    if !fields
        .iter()
        .all(|field| field == "package" || field == "package.name" || field.starts_with("package."))
    {
        return false;
    }
    let Some(root_value) = parse_structured_file_value(path) else {
        return false;
    };
    lookup_structured_field_value(&root_value, "workspace").is_some()
        && lookup_structured_field_value(&root_value, "package").is_none()
}

fn parse_structured_file_value(path: &Path) -> Option<Value> {
    let contents = fs::read_to_string(path).ok()?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "json" => serde_json::from_str(&contents).ok(),
        "toml" => toml::from_str::<toml::Value>(&contents)
            .ok()
            .and_then(|value| serde_json::to_value(value).ok()),
        _ => serde_json::from_str(&contents).ok().or_else(|| {
            toml::from_str::<toml::Value>(&contents)
                .ok()
                .and_then(|value| serde_json::to_value(value).ok())
        }),
    }
}

fn lookup_structured_field_value<'a>(value: &'a Value, field_path: &str) -> Option<&'a Value> {
    let mut current = value;
    for seg in field_path.split('.') {
        if seg.is_empty() {
            return None;
        }
        if let Ok(idx) = seg.parse::<usize>() {
            current = current.as_array()?.get(idx)?;
        } else {
            current = current.get(seg)?;
        }
    }
    Some(current)
}

fn route_requests_sqlite_table_listing(route: &RouteResult) -> bool {
    matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::SqliteTableListing
            | crate::OutputSemanticKind::SqliteTableNamesOnly
            | crate::OutputSemanticKind::SqliteDatabaseKindJudgment
    )
}

fn route_requests_sqlite_schema_version(route: &RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::SqliteSchemaVersion
}

fn sqlite_locator_path_for_route(
    route: &RouteResult,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let hint = route.output_contract.locator_hint.trim();
    [
        auto_locator_path.map(str::trim),
        (!hint.is_empty()).then_some(hint),
    ]
    .into_iter()
    .flatten()
    .find(|path| {
        let lower = path.to_ascii_lowercase();
        lower.ends_with(".sqlite") || lower.ends_with(".db")
    })
    .map(ToString::to_string)
}

fn is_sqlite_database_path(path: &str) -> bool {
    let lower = path.trim().to_ascii_lowercase();
    lower.ends_with(".sqlite") || lower.ends_with(".db")
}

fn action_is_text_read_of_sqlite_path(action: &AgentAction) -> bool {
    let Some(path) = sqlite_locator_path_from_action(action) else {
        return false;
    };
    if !is_sqlite_database_path(&path) {
        return false;
    }
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let skill = skill.trim().to_ascii_lowercase();
            if matches!(skill.as_str(), "read_file" | "fs_basic") {
                return args
                    .get("action")
                    .and_then(Value::as_str)
                    .map(|action| {
                        matches!(
                            action.trim().to_ascii_lowercase().as_str(),
                            "read_range"
                                | "read_text_range"
                                | "read"
                                | "read_text"
                                | "read_file"
                                | "head"
                        )
                    })
                    .unwrap_or(skill == "read_file");
            }
            skill == "system_basic"
                && args
                    .get("action")
                    .and_then(Value::as_str)
                    .map(|action| {
                        matches!(
                            action.trim().to_ascii_lowercase().as_str(),
                            "read_range" | "read_text_range" | "read" | "read_file"
                        )
                    })
                    .unwrap_or(false)
        }
        AgentAction::CallCapability { .. }
        | AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => false,
    }
}

fn action_should_be_sqlite_table_query(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let skill = skill.trim().to_ascii_lowercase();
            if skill == "db_basic" {
                return false;
            }
            if action_is_text_read_of_sqlite_path(action) {
                return true;
            }
            if skill == "read_file" || skill == "run_cmd" {
                return true;
            }
            skill == "system_basic"
                && args
                    .get("action")
                    .and_then(Value::as_str)
                    .map(|action| {
                        matches!(
                            action.trim().to_ascii_lowercase().as_str(),
                            "read_range"
                                | "read"
                                | "read_file"
                                | "run_cmd"
                                | "extract_field"
                                | "extract_fields"
                                | "sqlite_table_names"
                                | "sqlite_tables"
                                | "list_tables"
                        )
                    })
                    .unwrap_or(false)
        }
        AgentAction::CallCapability { .. }
        | AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => false,
    }
}

fn rewrite_sqlite_table_listing_plan_to_db_basic(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    preserve_explicit_command: bool,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if preserve_explicit_command {
        return actions;
    }
    let route_requested_listing = route_result.is_some_and(route_requests_sqlite_table_listing);
    let sqlite_text_read_path = actions
        .iter()
        .find(|action| action_is_text_read_of_sqlite_path(action))
        .and_then(sqlite_locator_path_from_action);
    if !route_requested_listing && sqlite_text_read_path.is_none() {
        return actions;
    }
    let Some(db_path) = route_result
        .and_then(|route| sqlite_locator_path_for_route(route, auto_locator_path))
        .or(sqlite_text_read_path)
    else {
        return actions;
    };
    let mut rewritten = actions;
    let mut changed = false;
    for action in rewritten.iter_mut() {
        if !action_should_be_sqlite_table_query(action) {
            continue;
        }
        *action = AgentAction::CallSkill {
            skill: "db_basic".to_string(),
            args: serde_json::json!({
                "action": "list_tables",
                "db_path": db_path,
            }),
        };
        changed = true;
    }
    if changed {
        info!("plan_rewrite_sqlite_table_listing_to_db_basic");
    }
    rewritten
}

fn sqlite_locator_path_from_action(action: &AgentAction) -> Option<String> {
    match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => {
            ["db_path", "path"]
                .into_iter()
                .filter_map(|key| args.get(key).and_then(Value::as_str))
                .map(str::trim)
                .find(|path| {
                    let lower = path.to_ascii_lowercase();
                    lower.ends_with(".sqlite") || lower.ends_with(".db")
                })
                .map(ToString::to_string)
        }
        AgentAction::CallCapability { .. }
        | AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => None,
    }
}

fn action_can_serve_sqlite_schema_version_query(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let skill = skill.trim().to_ascii_lowercase();
            if skill == "db_basic" {
                return false;
            }
            if skill == "read_file" || skill == "run_cmd" {
                return true;
            }
            if skill != "system_basic" {
                return false;
            }
            args.get("action")
                .and_then(Value::as_str)
                .map(|action| {
                    matches!(
                        action.trim().to_ascii_lowercase().as_str(),
                        "read_range"
                            | "read"
                            | "read_file"
                            | "run_cmd"
                            | "extract_field"
                            | "extract_fields"
                            | "schema_version"
                            | "sqlite_schema_version"
                    )
                })
                .unwrap_or(true)
        }
        AgentAction::CallCapability { .. }
        | AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => false,
    }
}

fn value_is_schema_version_field(value: &Value) -> bool {
    value
        .as_str()
        .map(str::trim)
        .map(|field| field.eq_ignore_ascii_case("schema_version"))
        .unwrap_or(false)
}

fn action_should_be_sqlite_schema_version_query(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let skill = skill.trim().to_ascii_lowercase();
            if skill == "db_basic" {
                return false;
            }
            if skill != "system_basic" {
                return false;
            }
            let action_name = args
                .get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            match action_name.to_ascii_lowercase().as_str() {
                "schema_version" | "sqlite_schema_version" => true,
                "extract_field" => args
                    .get("field_path")
                    .is_some_and(value_is_schema_version_field),
                "extract_fields" => args
                    .get("field_paths")
                    .and_then(Value::as_array)
                    .filter(|fields| fields.len() == 1)
                    .and_then(|fields| fields.first())
                    .is_some_and(value_is_schema_version_field),
                _ => false,
            }
        }
        AgentAction::CallCapability { .. }
        | AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => false,
    }
}

fn rewrite_sqlite_schema_version_plan_to_db_basic(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    preserve_explicit_command: bool,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if preserve_explicit_command {
        return actions;
    }
    let route_path =
        route_result.and_then(|route| sqlite_locator_path_for_route(route, auto_locator_path));
    let route_requests_schema_version =
        route_result.is_some_and(route_requests_sqlite_schema_version);
    let mut rewritten = actions;
    let mut changed = false;
    for action in rewritten.iter_mut() {
        if !(action_should_be_sqlite_schema_version_query(action)
            || (route_requests_schema_version
                && action_can_serve_sqlite_schema_version_query(action)))
        {
            continue;
        }
        let Some(db_path) = route_path
            .clone()
            .or_else(|| sqlite_locator_path_from_action(action))
        else {
            continue;
        };
        *action = AgentAction::CallSkill {
            skill: "db_basic".to_string(),
            args: serde_json::json!({
                "action": "schema_version",
                "db_path": db_path,
            }),
        };
        changed = true;
    }
    if changed {
        info!("plan_rewrite_sqlite_schema_version_to_db_basic");
    }
    rewritten
}

fn rewrite_sqlite_count_query_to_requested_schema_column(
    route_result: Option<&RouteResult>,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::ScalarCount {
        return actions;
    }
    let mut rewritten = actions;
    let mut changed = false;
    for action in rewritten.iter_mut() {
        let (AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }) =
            action
        else {
            continue;
        };
        if !skill.eq_ignore_ascii_case("db_basic") {
            continue;
        }
        let action_name = args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("sqlite_query");
        if !action_name.eq_ignore_ascii_case("sqlite_query") {
            continue;
        }
        let Some(sql) = args.get("sql").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        let Some(db_path) = args.get("db_path").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        let Some((table, suffix)) = parse_sqlite_count_star_query(sql) else {
            continue;
        };
        if sqlite_count_suffix_has_grouping(&suffix) {
            continue;
        }
        let Some(column) = requested_schema_column_for_sqlite_count_rewrite(
            route,
            user_text,
            original_user_text,
            db_path,
            &table,
            &suffix,
        ) else {
            continue;
        };
        let rewritten_sql = format!(
            "SELECT {} FROM {}{}",
            quote_sqlite_identifier(&column),
            quote_sqlite_identifier(&table),
            suffix
        );
        args["sql"] = Value::String(rewritten_sql);
        changed = true;
    }
    if changed {
        info!("plan_rewrite_sqlite_count_query_to_requested_schema_column");
    }
    rewritten
}

fn rewrite_sqlite_table_probe_to_requested_schema_value(
    route_result: Option<&RouteResult>,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::ScalarCount
                | crate::OutputSemanticKind::SqliteTableListing
                | crate::OutputSemanticKind::SqliteTableNamesOnly
                | crate::OutputSemanticKind::SqliteDatabaseKindJudgment
                | crate::OutputSemanticKind::SqliteSchemaVersion
        )
    {
        return actions;
    }
    let mut rewritten = actions;
    let mut changed = false;
    for action in rewritten.iter_mut() {
        let (AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }) =
            action
        else {
            continue;
        };
        if !skill.eq_ignore_ascii_case("db_basic") {
            continue;
        }
        let action_name = args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if !action_name.eq_ignore_ascii_case("list_tables") {
            continue;
        }
        let Some(db_path) = args.get("db_path").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        let Some(sql) =
            sqlite_schema_value_query_for_route(route, user_text, original_user_text, db_path)
        else {
            continue;
        };
        args["action"] = Value::String("sqlite_query".to_string());
        args["sql"] = Value::String(sql);
        changed = true;
    }
    if changed {
        info!("plan_rewrite_sqlite_table_probe_to_requested_schema_value");
    }
    rewritten
}

fn sqlite_schema_value_query_for_route(
    route: &RouteResult,
    user_text: &str,
    original_user_text: Option<&str>,
    db_path: &str,
) -> Option<String> {
    let source = [
        route.resolved_intent.as_str(),
        route.output_contract.locator_hint.as_str(),
        user_text,
        original_user_text.unwrap_or_default(),
    ]
    .join("\n")
    .to_ascii_lowercase();
    let source_tokens = identifier_tokens(&source);
    let table = requested_sqlite_table_for_source(db_path, &source_tokens)?;
    let columns = sqlite_table_columns(db_path, &table)?;
    let filters = sqlite_source_value_filters(db_path, &table, &columns, &source_tokens);
    if filters.is_empty() {
        return None;
    }
    let filter_columns = filters
        .iter()
        .map(|(column, _)| column.to_ascii_lowercase())
        .collect::<std::collections::HashSet<_>>();
    let target_columns = columns
        .into_iter()
        .filter(|column| {
            let lower = column.to_ascii_lowercase();
            identifier_tokens_contain_schema_name(&source_tokens, &lower)
                && !filter_columns.contains(&lower)
        })
        .collect::<Vec<_>>();
    if target_columns.len() != 1 {
        return None;
    }
    let target_column = &target_columns[0];
    let where_sql = filters
        .iter()
        .map(|(column, value)| {
            format!(
                "{} = {}",
                quote_sqlite_identifier(column),
                quote_sqlite_string_literal(value)
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    let sql = format!(
        "SELECT {} FROM {} WHERE {}",
        quote_sqlite_identifier(target_column),
        quote_sqlite_identifier(&table),
        where_sql
    );
    sqlite_query_single_scalar_preview(db_path, &sql).map(|_| sql)
}

fn requested_sqlite_table_for_source(
    db_path: &str,
    source_tokens: &std::collections::HashSet<String>,
) -> Option<String> {
    let tables = sqlite_database_table_names(db_path)?;
    let candidates = tables
        .into_iter()
        .filter(|table| identifier_tokens_contain_schema_name(source_tokens, table))
        .collect::<Vec<_>>();
    (candidates.len() == 1).then(|| candidates[0].clone())
}

fn sqlite_database_table_names(db_path: &str) -> Option<Vec<String>> {
    let path = resolve_sqlite_path_for_planner(db_path);
    let conn = rusqlite::Connection::open(path).ok()?;
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .ok()?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .ok()?
        .filter_map(Result::ok)
        .filter(|table| !table.trim().is_empty())
        .collect::<Vec<_>>();
    (!rows.is_empty()).then_some(rows)
}

fn sqlite_source_value_filters(
    db_path: &str,
    table: &str,
    columns: &[String],
    source_tokens: &std::collections::HashSet<String>,
) -> Vec<(String, String)> {
    let mut filters = Vec::new();
    for column in columns {
        let lower = column.to_ascii_lowercase();
        if !identifier_tokens_contain_schema_name(source_tokens, &lower) {
            continue;
        }
        let Some(values) = sqlite_distinct_column_values(db_path, table, column, 100) else {
            continue;
        };
        let matches = values
            .into_iter()
            .filter(|value| sqlite_value_mentioned_by_source(value, source_tokens))
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            filters.push((column.clone(), matches[0].clone()));
        }
    }
    filters
}

fn sqlite_distinct_column_values(
    db_path: &str,
    table: &str,
    column: &str,
    limit: usize,
) -> Option<Vec<String>> {
    let path = resolve_sqlite_path_for_planner(db_path);
    let conn = rusqlite::Connection::open(path).ok()?;
    let sql = format!(
        "SELECT DISTINCT {} FROM {} WHERE {} IS NOT NULL LIMIT {}",
        quote_sqlite_identifier(column),
        quote_sqlite_identifier(table),
        quote_sqlite_identifier(column),
        limit.clamp(1, 500)
    );
    let mut stmt = conn.prepare(&sql).ok()?;
    let rows = stmt
        .query_map([], |row| Ok(sqlite_value_ref_to_string(row.get_ref(0)?)))
        .ok()?
        .filter_map(Result::ok)
        .flatten()
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>();
    Some(rows)
}

fn sqlite_value_mentioned_by_source(
    value: &str,
    source_tokens: &std::collections::HashSet<String>,
) -> bool {
    let value_tokens = identifier_tokens(&value.to_ascii_lowercase());
    !value_tokens.is_empty()
        && value_tokens
            .iter()
            .all(|token| source_tokens.contains(token))
}

fn sqlite_query_single_scalar_preview(db_path: &str, sql: &str) -> Option<String> {
    let path = resolve_sqlite_path_for_planner(db_path);
    let conn = rusqlite::Connection::open(path).ok()?;
    let mut stmt = conn.prepare(sql).ok()?;
    if stmt.column_count() != 1 {
        return None;
    }
    let mut rows = stmt.query([]).ok()?;
    let row = rows.next().ok()??;
    let value = sqlite_value_ref_to_string(row.get_ref(0).ok()?)?;
    if rows.next().ok()?.is_some() {
        return None;
    }
    Some(value)
}

fn sqlite_value_ref_to_string(value: rusqlite::types::ValueRef<'_>) -> Option<String> {
    match value {
        rusqlite::types::ValueRef::Null => None,
        rusqlite::types::ValueRef::Integer(value) => Some(value.to_string()),
        rusqlite::types::ValueRef::Real(value) => Some(value.to_string()),
        rusqlite::types::ValueRef::Text(value) => {
            Some(String::from_utf8_lossy(value).trim().to_string())
        }
        rusqlite::types::ValueRef::Blob(_) => None,
    }
}

fn parse_sqlite_count_star_query(sql: &str) -> Option<(String, String)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r#"(?is)^\s*select\s+count\s*\(\s*\*\s*\)(?:\s+as\s+(?:"[^"]+"|`[^`]+`|\[[^\]]+\]|[A-Za-z_][A-Za-z0-9_]*))?\s+from\s+(?:"([^"]+)"|`([^`]+)`|\[([^\]]+)\]|([A-Za-z_][A-Za-z0-9_]*))(?P<suffix>.*?)\s*;?\s*$"#,
        )
        .expect("sqlite count query regex")
    });
    let captures = re.captures(sql)?;
    let table = (1..=4)
        .filter_map(|idx| captures.get(idx).map(|m| m.as_str().trim()))
        .find(|value| !value.is_empty())?
        .to_string();
    let suffix = captures
        .name("suffix")
        .map(|m| m.as_str().trim_end_matches(';').to_string())
        .unwrap_or_default();
    Some((table, suffix))
}

fn sqlite_count_suffix_has_grouping(suffix: &str) -> bool {
    let padded = format!(" {} ", suffix.to_ascii_lowercase());
    padded.contains(" group by ") || padded.contains(" having ")
}

fn requested_schema_column_for_sqlite_count_rewrite(
    route: &RouteResult,
    user_text: &str,
    original_user_text: Option<&str>,
    db_path: &str,
    table: &str,
    sql_suffix: &str,
) -> Option<String> {
    let columns = sqlite_table_columns(db_path, table)?;
    let source = [
        route.resolved_intent.as_str(),
        route.output_contract.locator_hint.as_str(),
        user_text,
        original_user_text.unwrap_or_default(),
    ]
    .join("\n")
    .to_ascii_lowercase();
    let source_tokens = identifier_tokens(&source);
    let suffix_tokens = identifier_tokens(&sql_suffix.to_ascii_lowercase());
    let candidates = columns
        .into_iter()
        .filter(|column| {
            let lower = column.to_ascii_lowercase();
            identifier_tokens_contain_schema_name(&source_tokens, &lower)
                && !identifier_tokens_contain_schema_name(&suffix_tokens, &lower)
        })
        .collect::<Vec<_>>();
    (candidates.len() == 1).then(|| candidates[0].clone())
}

fn sqlite_table_columns(db_path: &str, table: &str) -> Option<Vec<String>> {
    let path = resolve_sqlite_path_for_planner(db_path);
    let conn = rusqlite::Connection::open(path).ok()?;
    let pragma = format!("PRAGMA table_info({})", quote_sqlite_identifier(table));
    let mut stmt = conn.prepare(&pragma).ok()?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .ok()?
        .filter_map(Result::ok)
        .filter(|column| !column.trim().is_empty())
        .collect::<Vec<_>>();
    (!rows.is_empty()).then_some(rows)
}

fn resolve_sqlite_path_for_planner(db_path: &str) -> PathBuf {
    let raw = Path::new(db_path);
    if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(raw)
    }
}

fn quote_sqlite_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn quote_sqlite_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn identifier_tokens(text: &str) -> std::collections::HashSet<String> {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn identifier_tokens_contain_schema_name(
    tokens: &std::collections::HashSet<String>,
    schema_name: &str,
) -> bool {
    let normalized = schema_name.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }
    if tokens.contains(&normalized) {
        return true;
    }
    if let Some(singular) = normalized.strip_suffix('s') {
        if singular.len() >= 3 && tokens.contains(singular) {
            return true;
        }
    }
    let plural = format!("{normalized}s");
    tokens.contains(&plural)
}

fn split_archive_locator_pair(hint: &str) -> Option<(String, String)> {
    let hint = hint.trim();
    for separator in ["|", "->", "=>"] {
        if let Some((left, right)) = hint.split_once(separator) {
            let left = left.trim();
            let right = right.trim();
            if !left.is_empty() && !right.is_empty() {
                return Some((left.to_string(), right.to_string()));
            }
        }
    }
    None
}

fn is_supported_archive_path(path: &str) -> bool {
    let path_lower = path.trim().to_ascii_lowercase();
    path_lower.ends_with(".zip") || path_lower.ends_with(".tar.gz") || path_lower.ends_with(".tgz")
}

fn archive_format_for_path(path: &str) -> &'static str {
    if path.trim().to_ascii_lowercase().ends_with(".zip") {
        "zip"
    } else {
        "tar.gz"
    }
}

fn archive_unpack_pair_for_route(route: &RouteResult) -> Option<(String, String)> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ArchiveUnpack {
        return None;
    }
    let (archive, dest) = split_archive_locator_pair(&route.output_contract.locator_hint)?;
    if !is_supported_archive_path(&archive) {
        return None;
    }
    Some((archive, dest))
}

fn rewrite_archive_unpack_run_cmd_to_archive_basic(
    route_result: Option<&RouteResult>,
    preserve_explicit_command: bool,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if preserve_explicit_command {
        return actions;
    }
    let Some(route) = route_result else {
        return actions;
    };
    let Some((archive, dest)) = archive_unpack_pair_for_route(route) else {
        return actions;
    };
    if actions.iter().any(action_is_archive_basic_unpack) {
        return actions;
    }
    let mut rewritten = actions;
    let mut changed = false;
    for action in rewritten.iter_mut() {
        let should_rewrite = action_skill_is_run_cmd(action) || action_is_archive_basic(action);
        if !should_rewrite {
            continue;
        }
        *action = AgentAction::CallSkill {
            skill: "archive_basic".to_string(),
            args: serde_json::json!({
                "action": "unpack",
                "archive": archive,
                "dest": dest,
            }),
        };
        changed = true;
        break;
    }
    if changed {
        info!("plan_rewrite_archive_unpack_plan_to_archive_basic");
    }
    rewritten
}

fn archive_pack_pair_for_route(route: &RouteResult) -> Option<(String, String)> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ArchivePack
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
    {
        return None;
    }
    let (source, archive) = split_archive_locator_pair(&route.output_contract.locator_hint)?;
    if is_supported_archive_path(&source) || !is_supported_archive_path(&archive) {
        return None;
    }
    Some((source, archive))
}

fn action_args(action: &AgentAction) -> Option<&Value> {
    match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => Some(args),
        _ => None,
    }
}

fn action_is_archive_basic(action: &AgentAction) -> bool {
    planned_action_skill_name(action)
        .map(str::trim)
        .is_some_and(|skill| skill.eq_ignore_ascii_case("archive_basic"))
}

fn action_is_archive_basic_pack(action: &AgentAction) -> bool {
    action_is_archive_basic(action)
        && action_args(action)
            .and_then(|args| args.get("action"))
            .and_then(Value::as_str)
            .is_some_and(|action| action.trim().eq_ignore_ascii_case("pack"))
}

fn action_is_archive_basic_unpack(action: &AgentAction) -> bool {
    action_is_archive_basic(action)
        && action_args(action)
            .and_then(|args| args.get("action"))
            .and_then(Value::as_str)
            .is_some_and(|action| action.trim().eq_ignore_ascii_case("unpack"))
}

fn action_skill_is_run_cmd(action: &AgentAction) -> bool {
    planned_action_skill_name(action)
        .map(str::trim)
        .is_some_and(|skill| skill.eq_ignore_ascii_case("run_cmd"))
}

fn run_cmd_command_arg(action: &AgentAction) -> Option<&str> {
    action_args(action)
        .and_then(|args| args.get("command").or_else(|| args.get("cmd")))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
}

fn shell_head_limit_from_words(words: &[String]) -> Option<u64> {
    let mut idx = 0;
    while idx < words.len() {
        if !command_basename(&words[idx]).eq_ignore_ascii_case("head") {
            idx += 1;
            continue;
        }
        let mut head_idx = idx + 1;
        while head_idx < words.len() {
            let word = words[head_idx].trim();
            if word.is_empty() {
                head_idx += 1;
                continue;
            }
            if word == "-n" || word == "--lines" {
                return words
                    .get(head_idx + 1)
                    .and_then(|value| parse_shell_line_count(value));
            }
            if let Some(value) = word.strip_prefix("-n") {
                return parse_shell_line_count(value);
            }
            if let Some(value) = word.strip_prefix("--lines=") {
                return parse_shell_line_count(value);
            }
            if let Some(value) = word.strip_prefix('-') {
                return parse_shell_line_count(value);
            }
            break;
        }
        return Some(10);
    }
    None
}

fn process_basic_ps_limit_from_command(command: &str) -> Option<u64> {
    let words = shell_like_words(command);
    let first = words.first().map(|word| command_basename(word))?;
    if !first.eq_ignore_ascii_case("ps") {
        return None;
    }

    let lower = command.to_ascii_lowercase();
    if words.iter().any(|word| {
        matches!(
            command_basename(word).to_ascii_lowercase().as_str(),
            "awk" | "sed" | "grep" | "egrep" | "fgrep" | "xargs" | "cut" | "kill" | "pkill"
        )
    }) {
        return None;
    }
    if words.iter().any(|word| {
        matches!(
            word.as_str(),
            "-p" | "--pid" | "-C" | "--ppid" | "--quick-pid"
        ) || word.starts_with("--pid=")
            || word.starts_with("--ppid=")
            || word.starts_with("--quick-pid=")
    }) {
        return None;
    }

    let head_limit = shell_head_limit_from_words(&words);
    let sorted_by_cpu = lower.contains("--sort=-%cpu")
        || lower.contains("--sort=-pcpu")
        || (lower.contains("sort") && (lower.contains("%cpu") || lower.contains("pcpu")));
    let process_cpu_columns =
        lower.contains("%cpu") || lower.contains("pcpu") || lower.contains("ps aux");
    if !sorted_by_cpu && !(head_limit.is_some() && process_cpu_columns) {
        return None;
    }

    let limit = head_limit
        .map(|rows| rows.saturating_sub(1).max(1))
        .unwrap_or(10)
        .clamp(1, 50);
    Some(limit)
}

fn rewrite_process_ps_run_cmd_to_process_basic(
    state: &AppState,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !process_basic_available_for_plan(state) {
        return actions;
    }
    let mut rewritten = actions;
    let mut changed = false;
    for action in rewritten.iter_mut() {
        if !action_skill_is_run_cmd(action) {
            continue;
        }
        let Some(args) = action_args(action) else {
            continue;
        };
        let Some(command) = run_cmd_command_arg(action) else {
            continue;
        };
        if args
            .get(super::CLAWD_LITERAL_COMMAND_ARG)
            .and_then(Value::as_bool)
            == Some(true)
            || should_preserve_user_supplied_shell_command(command, user_text, original_user_text)
        {
            continue;
        }
        let Some(limit) = process_basic_ps_limit_from_command(command) else {
            continue;
        };
        *action = AgentAction::CallSkill {
            skill: "process_basic".to_string(),
            args: serde_json::json!({
                "action": "ps",
                "limit": limit,
            }),
        };
        changed = true;
    }
    if changed {
        info!("plan_rewrite_process_ps_run_cmd_to_process_basic");
    }
    rewritten
}

fn split_shell_sequence_command_with_policy(
    command: &str,
    split_conditionals: bool,
) -> Option<Vec<String>> {
    if command.contains("<<") {
        return None;
    }
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut command_sub_depth = 0usize;
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            current.push(ch);
            if quote != Some('\'') {
                escaped = true;
            }
            continue;
        }
        if let Some(active_quote) = quote {
            current.push(ch);
            if ch == active_quote {
                quote = None;
            }
            continue;
        }
        if matches!(ch, '\'' | '"' | '`') {
            quote = Some(ch);
            current.push(ch);
            continue;
        }
        if ch == '$' && chars.peek() == Some(&'(') {
            current.push(ch);
            current.push('(');
            chars.next();
            command_sub_depth += 1;
            continue;
        }
        if command_sub_depth > 0 {
            if ch == '(' {
                command_sub_depth += 1;
            } else if ch == ')' {
                command_sub_depth = command_sub_depth.saturating_sub(1);
            }
            current.push(ch);
            continue;
        }
        if ch == '&' && chars.peek() == Some(&'&') && split_conditionals {
            let part = current.trim();
            if part.is_empty() {
                return None;
            }
            parts.push(part.to_string());
            current.clear();
            chars.next();
            continue;
        }
        if ch == ';' || ch == '\n' {
            let part = current.trim();
            if part.is_empty() {
                return None;
            }
            parts.push(part.to_string());
            current.clear();
            continue;
        }
        current.push(ch);
    }
    let part = current.trim();
    if part.is_empty() {
        return None;
    }
    parts.push(part.to_string());
    if parts.len() < 2 || !shell_sequence_parts_can_run_independently(&parts) {
        return None;
    }
    Some(parts)
}

fn planner_failure_fallback_first_command(
    command: &str,
    split_conditionals: bool,
) -> Option<String> {
    if !split_conditionals || command.contains("<<") {
        return None;
    }
    let split_at = top_level_shell_or_operator_byte_index(command)?;
    let first = command[..split_at].trim();
    let fallback = command[split_at + 2..].trim();
    if first.is_empty() || fallback.is_empty() {
        return None;
    }
    let parts = vec![first.to_string(), fallback.to_string()];
    if !shell_sequence_parts_can_run_independently(&parts) {
        return None;
    }
    Some(first.to_string())
}

fn top_level_shell_or_operator_byte_index(command: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut command_sub_depth = 0usize;
    let mut chars = command.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            if quote != Some('\'') {
                escaped = true;
            }
            continue;
        }
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            }
            continue;
        }
        if matches!(ch, '\'' | '"' | '`') {
            quote = Some(ch);
            continue;
        }
        if ch == '$' && chars.peek().is_some_and(|(_, next)| *next == '(') {
            chars.next();
            command_sub_depth += 1;
            continue;
        }
        if command_sub_depth > 0 {
            if ch == '(' {
                command_sub_depth += 1;
            } else if ch == ')' {
                command_sub_depth = command_sub_depth.saturating_sub(1);
            }
            continue;
        }
        if ch == '|' && chars.peek().is_some_and(|(_, next)| *next == '|') {
            return Some(idx);
        }
    }
    None
}

fn request_text_contains_shell_conditional_operator(text: &str) -> bool {
    text.contains("&&") || text.contains("||")
}

fn should_split_planner_introduced_shell_conditionals(
    user_text: &str,
    original_user_text: Option<&str>,
) -> bool {
    !request_text_contains_shell_conditional_operator(user_text)
        && !original_user_text.is_some_and(request_text_contains_shell_conditional_operator)
}

fn request_text_contains_command_verbatim(text: &str, command: &str) -> bool {
    let command = command.trim();
    !command.is_empty() && text.contains(command)
}

fn should_preserve_user_supplied_shell_command(
    command: &str,
    user_text: &str,
    original_user_text: Option<&str>,
) -> bool {
    request_text_contains_command_verbatim(user_text, command)
        || original_user_text
            .is_some_and(|text| request_text_contains_command_verbatim(text, command))
}

fn shell_sequence_parts_can_run_independently(parts: &[String]) -> bool {
    parts
        .iter()
        .enumerate()
        .all(|(idx, part)| shell_sequence_part_can_run_independently(part, idx + 1 == parts.len()))
}

fn shell_sequence_part_can_run_independently(part: &str, is_last: bool) -> bool {
    let words = shell_like_words(part);
    let Some(first) = words.first().map(|word| command_basename(word).trim()) else {
        return false;
    };
    let first = first.to_ascii_lowercase();
    if matches!(
        first.as_str(),
        "if" | "for"
            | "while"
            | "until"
            | "case"
            | "select"
            | "do"
            | "then"
            | "else"
            | "elif"
            | "fi"
            | "done"
            | "esac"
            | "function"
            | "{"
            | "}"
            | "("
            | ")"
    ) {
        return false;
    }
    if !is_last
        && matches!(
            first.as_str(),
            "cd" | "export" | "source" | "." | "set" | "unset" | "alias" | "unalias" | "umask"
        )
    {
        return false;
    }
    true
}

fn split_sequential_run_cmd_actions(
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let split_conditionals =
        should_split_planner_introduced_shell_conditionals(user_text, original_user_text);
    let mut changed = false;
    let mut rewritten = Vec::with_capacity(actions.len());
    for action in actions {
        match action {
            AgentAction::CallSkill { skill, args }
                if skill.trim().eq_ignore_ascii_case("run_cmd") =>
            {
                let parts = run_cmd_command_from_args(&args).and_then(|command| {
                    if should_preserve_user_supplied_shell_command(
                        command,
                        user_text,
                        original_user_text,
                    ) {
                        None
                    } else if let Some(first_attempt) =
                        planner_failure_fallback_first_command(command, split_conditionals)
                    {
                        Some(vec![first_attempt])
                    } else {
                        split_shell_sequence_command_with_policy(command, split_conditionals)
                    }
                });
                if let Some(parts) = parts {
                    let continue_on_error = parts.len() > 1;
                    for command in parts {
                        rewritten.push(AgentAction::CallSkill {
                            skill: skill.clone(),
                            args: run_cmd_args_for_rewritten_command(
                                &args,
                                command,
                                continue_on_error,
                            ),
                        });
                    }
                    changed = true;
                } else {
                    rewritten.push(AgentAction::CallSkill { skill, args });
                }
            }
            AgentAction::CallTool { tool, args } if tool.trim().eq_ignore_ascii_case("run_cmd") => {
                let parts = run_cmd_command_from_args(&args).and_then(|command| {
                    if should_preserve_user_supplied_shell_command(
                        command,
                        user_text,
                        original_user_text,
                    ) {
                        None
                    } else if let Some(first_attempt) =
                        planner_failure_fallback_first_command(command, split_conditionals)
                    {
                        Some(vec![first_attempt])
                    } else {
                        split_shell_sequence_command_with_policy(command, split_conditionals)
                    }
                });
                if let Some(parts) = parts {
                    let continue_on_error = parts.len() > 1;
                    for command in parts {
                        rewritten.push(AgentAction::CallTool {
                            tool: tool.clone(),
                            args: run_cmd_args_for_rewritten_command(
                                &args,
                                command,
                                continue_on_error,
                            ),
                        });
                    }
                    changed = true;
                } else {
                    rewritten.push(AgentAction::CallTool { tool, args });
                }
            }
            other => rewritten.push(other),
        }
    }
    if changed {
        info!("plan_split_sequential_run_cmd_actions");
    }
    rewritten
}

fn run_cmd_command_from_args(args: &Value) -> Option<&str> {
    args.get("command")
        .or_else(|| args.get("cmd"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
}

fn run_cmd_args_for_rewritten_command(
    args: &Value,
    command: String,
    continue_on_error: bool,
) -> Value {
    let mut next_args = args.clone();
    if let Some(obj) = next_args.as_object_mut() {
        let key = if obj.contains_key("command") {
            "command"
        } else {
            "cmd"
        };
        obj.insert(key.to_string(), Value::String(command));
        if continue_on_error {
            obj.insert(
                super::CLAWD_CONTINUE_ON_ERROR_ARG.to_string(),
                Value::Bool(true),
            );
        } else {
            obj.remove(super::CLAWD_CONTINUE_ON_ERROR_ARG);
            obj.remove(super::CLAWD_LITERAL_COMMAND_ARG);
            obj.remove(super::CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG);
        }
    }
    next_args
}

fn fs_basic_read_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty()
        || enabled_skills.contains("fs_basic")
        || enabled_skills.contains("system_basic")
}

fn command_has_shell_control_or_expansion(command: &str) -> bool {
    if command.contains('\n') || command.contains('\r') || command.contains('$') {
        return true;
    }
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in command.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            if quote != Some('\'') {
                escaped = true;
            }
            continue;
        }
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            }
            continue;
        }
        if matches!(ch, '\'' | '"') {
            quote = Some(ch);
            continue;
        }
        if matches!(ch, '`' | '|' | ';' | '<' | '>' | '&') {
            return true;
        }
    }
    quote.is_some()
}

fn parse_shell_line_count(raw: &str) -> Option<u64> {
    let value = raw.trim();
    if value.is_empty() || !value.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    value.parse::<u64>().ok().filter(|n| (1..=500).contains(n))
}

fn shell_file_path_token_is_safe(path: &str) -> bool {
    let path = path.trim();
    !path.is_empty()
        && path != "-"
        && !path.starts_with('~')
        && !path.contains('\0')
        && !path.contains('$')
        && !path
            .chars()
            .any(|ch| matches!(ch, '*' | '?' | '[' | ']' | '{' | '}'))
}

fn absolutize_readonly_file_path_from_run_cmd_args(path: &str, args: &Value) -> String {
    let trimmed = path.trim();
    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        return trimmed.to_string();
    }
    args.get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
        .map(|cwd| Path::new(cwd).join(candidate).display().to_string())
        .unwrap_or_else(|| trimmed.to_string())
}

fn append_text_from_shell_command(command: &str) -> Option<(String, String)> {
    if command.contains('\n') || command.contains('\r') || command.contains('\0') {
        return None;
    }
    let words = shell_like_words(command);
    let executable = words.first().map(|word| command_basename(word))?;
    if !executable.eq_ignore_ascii_case("echo") {
        return None;
    }
    let redirect_idx = words.iter().position(|word| word == ">>")?;
    if redirect_idx < 2 || redirect_idx + 2 != words.len() {
        return None;
    }
    let mut content_start = 1usize;
    let mut trailing_newline = true;
    if words.get(1).is_some_and(|word| word == "-n") {
        content_start = 2;
        trailing_newline = false;
    }
    if content_start >= redirect_idx {
        return None;
    }
    let mut content = words[content_start..redirect_idx].join(" ");
    if trailing_newline {
        content.push('\n');
    }
    let path = words.get(redirect_idx + 1)?.trim();
    if !shell_file_path_token_is_safe(path) {
        return None;
    }
    Some((content, path.to_string()))
}

fn rewrite_append_run_cmd_to_fs_basic(
    state: &AppState,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !fs_basic_read_available_for_plan(state) {
        return actions;
    }
    let mut changed = false;
    let rewritten = actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args }
                if skill.trim().eq_ignore_ascii_case("run_cmd") =>
            {
                let Some(command) = run_cmd_command_from_args(&args) else {
                    return AgentAction::CallSkill { skill, args };
                };
                if args
                    .get(super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool)
                    == Some(true)
                    || should_preserve_user_supplied_shell_command(
                        command,
                        user_text,
                        original_user_text,
                    )
                {
                    return AgentAction::CallSkill { skill, args };
                }
                let Some((content, path)) = append_text_from_shell_command(command) else {
                    return AgentAction::CallSkill { skill, args };
                };
                changed = true;
                AgentAction::CallTool {
                    tool: "fs_basic".to_string(),
                    args: serde_json::json!({
                        "action": "append_text",
                        "path": absolutize_readonly_file_path_from_run_cmd_args(&path, &args),
                        "content": content,
                    }),
                }
            }
            AgentAction::CallTool { tool, args } if tool.trim().eq_ignore_ascii_case("run_cmd") => {
                let Some(command) = run_cmd_command_from_args(&args) else {
                    return AgentAction::CallTool { tool, args };
                };
                if args
                    .get(super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool)
                    == Some(true)
                    || should_preserve_user_supplied_shell_command(
                        command,
                        user_text,
                        original_user_text,
                    )
                {
                    return AgentAction::CallTool { tool, args };
                }
                let Some((content, path)) = append_text_from_shell_command(command) else {
                    return AgentAction::CallTool { tool, args };
                };
                changed = true;
                AgentAction::CallTool {
                    tool: "fs_basic".to_string(),
                    args: serde_json::json!({
                        "action": "append_text",
                        "path": absolutize_readonly_file_path_from_run_cmd_args(&path, &args),
                        "content": content,
                    }),
                }
            }
            other => other,
        })
        .collect();
    if changed {
        info!("plan_rewrite_append_run_cmd_to_fs_basic");
    }
    rewritten
}

fn readonly_file_read_from_shell_command(command: &str) -> Option<(&'static str, u64, String)> {
    if command_has_shell_control_or_expansion(command) {
        return None;
    }
    let words = shell_like_words(command);
    let executable = words.first().map(|word| command_basename(word))?;
    let mode = match executable.to_ascii_lowercase().as_str() {
        "head" => "head",
        "tail" => "tail",
        _ => return None,
    };
    let mut n = 10;
    let mut paths = Vec::new();
    let mut idx = 1;
    while idx < words.len() {
        let word = words[idx].trim();
        if word.is_empty() {
            idx += 1;
            continue;
        }
        if word == "--" {
            paths.extend(words.iter().skip(idx + 1).cloned());
            break;
        }
        if matches!(word, "-q" | "--quiet" | "--silent") {
            idx += 1;
            continue;
        }
        if matches!(word, "-n" | "--lines") {
            let value = words.get(idx + 1)?;
            n = parse_shell_line_count(value)?;
            idx += 2;
            continue;
        }
        if let Some(value) = word.strip_prefix("-n") {
            n = parse_shell_line_count(value)?;
            idx += 1;
            continue;
        }
        if let Some(value) = word.strip_prefix("--lines=") {
            n = parse_shell_line_count(value)?;
            idx += 1;
            continue;
        }
        if let Some(value) = word.strip_prefix('-') {
            n = parse_shell_line_count(value)?;
            idx += 1;
            continue;
        }
        paths.push(word.to_string());
        idx += 1;
    }
    if paths.len() != 1 || !shell_file_path_token_is_safe(&paths[0]) {
        return None;
    }
    Some((mode, n, paths.remove(0)))
}

fn rewrite_readonly_file_read_run_cmd_to_fs_basic(
    state: &AppState,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !fs_basic_read_available_for_plan(state) {
        return actions;
    }
    let mut changed = false;
    let rewritten = actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args }
                if skill.trim().eq_ignore_ascii_case("run_cmd") =>
            {
                let Some(command) = run_cmd_command_from_args(&args) else {
                    return AgentAction::CallSkill { skill, args };
                };
                if args
                    .get(super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool)
                    == Some(true)
                    || should_preserve_user_supplied_shell_command(
                        command,
                        user_text,
                        original_user_text,
                    )
                {
                    return AgentAction::CallSkill { skill, args };
                }
                let Some((mode, n, path)) = readonly_file_read_from_shell_command(command) else {
                    return AgentAction::CallSkill { skill, args };
                };
                changed = true;
                AgentAction::CallTool {
                    tool: "fs_basic".to_string(),
                    args: serde_json::json!({
                        "action": "read_text_range",
                        "path": absolutize_readonly_file_path_from_run_cmd_args(&path, &args),
                        "mode": mode,
                        "n": n,
                    }),
                }
            }
            AgentAction::CallTool { tool, args } if tool.trim().eq_ignore_ascii_case("run_cmd") => {
                let Some(command) = run_cmd_command_from_args(&args) else {
                    return AgentAction::CallTool { tool, args };
                };
                if args
                    .get(super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool)
                    == Some(true)
                    || should_preserve_user_supplied_shell_command(
                        command,
                        user_text,
                        original_user_text,
                    )
                {
                    return AgentAction::CallTool { tool, args };
                }
                let Some((mode, n, path)) = readonly_file_read_from_shell_command(command) else {
                    return AgentAction::CallTool { tool, args };
                };
                changed = true;
                AgentAction::CallTool {
                    tool: "fs_basic".to_string(),
                    args: serde_json::json!({
                        "action": "read_text_range",
                        "path": absolutize_readonly_file_path_from_run_cmd_args(&path, &args),
                        "mode": mode,
                        "n": n,
                    }),
                }
            }
            other => other,
        })
        .collect();
    if changed {
        info!("plan_rewrite_readonly_file_read_run_cmd_to_fs_basic");
    }
    rewritten
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadonlyFindCommand {
    root: String,
    extension: String,
}

fn filesystem_find_route_prefers_structured_tool(route_result: Option<&RouteResult>) -> bool {
    route_result.is_some_and(|route| {
        !route.output_contract.delivery_required
            && matches!(
                route.output_contract.semantic_kind,
                crate::OutputSemanticKind::DirectoryNames
                    | crate::OutputSemanticKind::FileNames
                    | crate::OutputSemanticKind::FilePaths
            )
    })
}

fn simple_shell_extension_pattern(pattern: &str) -> Option<String> {
    let pattern = pattern.trim();
    let candidate = pattern
        .strip_prefix("*.")
        .or_else(|| pattern.strip_prefix('.'))
        .unwrap_or(pattern)
        .trim();
    if candidate.is_empty()
        || candidate.contains('/')
        || candidate
            .chars()
            .any(|ch| matches!(ch, '*' | '?' | '[' | ']' | '{' | '}'))
    {
        return None;
    }
    Some(candidate.to_ascii_lowercase())
}

fn readonly_find_extension_from_shell_command(command: &str) -> Option<ReadonlyFindCommand> {
    if command.contains('\n')
        || command.contains('\r')
        || command.contains('\0')
        || command.contains('`')
        || command.contains('<')
        || command.contains('>')
        || command.contains('&')
    {
        return None;
    }
    let words = shell_like_words(command);
    let pipe_index = words.iter().position(|word| word == "|");
    if let Some(index) = pipe_index {
        if !readonly_find_pipeline_suffix_is_supported(&words[index + 1..]) {
            return None;
        }
    }
    let find_words = match pipe_index {
        Some(index) => &words[..index],
        None => words.as_slice(),
    };
    if find_words
        .first()
        .map(|word| !command_basename(word).eq_ignore_ascii_case("find"))
        .unwrap_or(true)
    {
        return None;
    }
    let mut index = 1usize;
    let mut root = ".".to_string();
    if let Some(candidate) = find_words.get(index) {
        if !candidate.starts_with('-') {
            if !shell_file_path_token_is_safe(candidate) {
                return None;
            }
            root = candidate.to_string();
            index += 1;
        }
    }
    let mut extension = None;
    while index < find_words.len() {
        let word = find_words[index].as_str();
        match word {
            "-name" | "-iname" => {
                let pattern = find_words.get(index + 1)?;
                extension = Some(simple_shell_extension_pattern(pattern)?);
                index += 2;
            }
            "-type" => {
                if find_words.get(index + 1).map(String::as_str) != Some("f") {
                    return None;
                }
                index += 2;
            }
            "-maxdepth" | "-mindepth" => {
                find_words.get(index + 1)?;
                index += 2;
            }
            "-exec" => {
                let executable = find_words.get(index + 1)?;
                if !command_basename(executable).eq_ignore_ascii_case("dirname") {
                    return None;
                }
                let mut end = index + 2;
                while end < find_words.len() && find_words[end] != ";" {
                    end += 1;
                }
                if end >= find_words.len() {
                    return None;
                }
                index = end + 1;
            }
            _ => return None,
        }
    }
    Some(ReadonlyFindCommand {
        root,
        extension: extension?,
    })
}

fn readonly_find_pipeline_suffix_is_supported(words: &[String]) -> bool {
    let segments = words
        .split(|word| word == "|")
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    match segments.as_slice() {
        [single] => {
            readonly_find_suffix_is_sort_unique(single)
                || readonly_find_suffix_is_parent_projection(single)
        }
        [project, sort] => {
            readonly_find_suffix_is_parent_projection(project)
                && readonly_find_suffix_is_sort_unique(sort)
        }
        _ => false,
    }
}

fn readonly_find_suffix_is_sort_unique(words: &[String]) -> bool {
    matches!(
        words,
        [cmd, flag]
            if command_basename(cmd).eq_ignore_ascii_case("sort")
                && matches!(flag.as_str(), "-u" | "--unique")
    )
}

fn readonly_find_suffix_is_parent_projection(words: &[String]) -> bool {
    match words {
        [cmd, expr] if command_basename(cmd).eq_ignore_ascii_case("sed") => {
            readonly_sed_parent_projection_expr(expr)
        }
        [cmd, flag, expr] if command_basename(cmd).eq_ignore_ascii_case("sed") && flag == "-e" => {
            readonly_sed_parent_projection_expr(expr)
        }
        [cmd, dirname] if command_basename(cmd).eq_ignore_ascii_case("xargs") => {
            command_basename(dirname).eq_ignore_ascii_case("dirname")
        }
        [cmd, n_flag, n_value, dirname]
            if command_basename(cmd).eq_ignore_ascii_case("xargs")
                && matches!(n_flag.as_str(), "-n" | "--max-args")
                && n_value == "1" =>
        {
            command_basename(dirname).eq_ignore_ascii_case("dirname")
        }
        [cmd, n_flag, dirname]
            if command_basename(cmd).eq_ignore_ascii_case("xargs") && n_flag == "-n1" =>
        {
            command_basename(dirname).eq_ignore_ascii_case("dirname")
        }
        _ => false,
    }
}

fn readonly_sed_parent_projection_expr(expr: &str) -> bool {
    matches!(
        expr,
        "s|/[^/]*$||"
            | "s#/[^/]*$##"
            | "s,/[^/]*$,,"
            | "s|/[^/]*$|.|"
            | "s#/[^/]*$#.#"
            | "s,/[^/]*$,.,"
    )
}

fn fs_basic_find_entries_extension_from_action(action: &AgentAction) -> Option<String> {
    let (name, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        _ => return None,
    };
    if !name.eq_ignore_ascii_case("fs_basic")
        || args.get("action").and_then(Value::as_str) != Some("find_entries")
    {
        return None;
    }
    args.get("extension")
        .or_else(|| args.get("ext"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|ext| !ext.is_empty())
        .map(|ext| ext.to_ascii_lowercase())
}

fn rewrite_readonly_find_run_cmd_to_fs_basic(
    state: &AppState,
    route_result: Option<&RouteResult>,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !fs_basic_read_available_for_plan(state)
        || !filesystem_find_route_prefers_structured_tool(route_result)
    {
        return actions;
    }
    let existing_find_extensions = actions
        .iter()
        .filter_map(fs_basic_find_entries_extension_from_action)
        .collect::<Vec<_>>();
    let mut changed = false;
    let rewritten = actions
        .into_iter()
        .filter_map(|action| match action {
            AgentAction::CallSkill { skill, args }
                if skill.trim().eq_ignore_ascii_case("run_cmd") =>
            {
                let Some(command) = run_cmd_command_from_args(&args) else {
                    return Some(AgentAction::CallSkill { skill, args });
                };
                if args
                    .get(super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool)
                    == Some(true)
                    || should_preserve_user_supplied_shell_command(
                        command,
                        user_text,
                        original_user_text,
                    )
                {
                    return Some(AgentAction::CallSkill { skill, args });
                }
                let Some(find) = readonly_find_extension_from_shell_command(command) else {
                    return Some(AgentAction::CallSkill { skill, args });
                };
                if existing_find_extensions
                    .iter()
                    .any(|ext| ext.eq_ignore_ascii_case(&find.extension))
                {
                    changed = true;
                    return None;
                }
                changed = true;
                Some(AgentAction::CallTool {
                    tool: "fs_basic".to_string(),
                    args: serde_json::json!({
                        "action": "find_entries",
                        "root": find.root,
                        "extension": find.extension,
                        "files_only": true,
                        "recursive": true,
                    }),
                })
            }
            AgentAction::CallTool { tool, args } if tool.trim().eq_ignore_ascii_case("run_cmd") => {
                let Some(command) = run_cmd_command_from_args(&args) else {
                    return Some(AgentAction::CallTool { tool, args });
                };
                if args
                    .get(super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool)
                    == Some(true)
                    || should_preserve_user_supplied_shell_command(
                        command,
                        user_text,
                        original_user_text,
                    )
                {
                    return Some(AgentAction::CallTool { tool, args });
                }
                let Some(find) = readonly_find_extension_from_shell_command(command) else {
                    return Some(AgentAction::CallTool { tool, args });
                };
                if existing_find_extensions
                    .iter()
                    .any(|ext| ext.eq_ignore_ascii_case(&find.extension))
                {
                    changed = true;
                    return None;
                }
                changed = true;
                Some(AgentAction::CallTool {
                    tool: "fs_basic".to_string(),
                    args: serde_json::json!({
                        "action": "find_entries",
                        "root": find.root,
                        "extension": find.extension,
                        "files_only": true,
                        "recursive": true,
                    }),
                })
            }
            other => Some(other),
        })
        .collect();
    if changed {
        info!("plan_rewrite_readonly_find_run_cmd_to_fs_basic");
    }
    rewritten
}

fn docker_basic_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("docker_basic")
}

fn docker_readonly_action_from_command(command: &str) -> Option<&'static str> {
    let words = shell_like_words(command);
    let first = words.first().map(|word| command_basename(word))?;
    let mut index = if first.eq_ignore_ascii_case("docker") {
        1
    } else if first.eq_ignore_ascii_case("sudo")
        && words
            .get(1)
            .map(|word| command_basename(word).eq_ignore_ascii_case("docker"))
            == Some(true)
    {
        2
    } else {
        return None;
    };
    while words
        .get(index)
        .is_some_and(|word| word.starts_with('-') && word != "-")
    {
        index += 1;
    }
    let subcommand = words.get(index)?.trim().to_ascii_lowercase();
    match subcommand.as_str() {
        "ps" => Some("ps"),
        "images" => Some("images"),
        "version" => Some("version"),
        "container" => match words.get(index + 1).map(|word| word.to_ascii_lowercase()) {
            Some(next) if matches!(next.as_str(), "ls" | "list" | "ps") => Some("ps"),
            _ => None,
        },
        "image" => match words.get(index + 1).map(|word| word.to_ascii_lowercase()) {
            Some(next) if matches!(next.as_str(), "ls" | "list") => Some("images"),
            _ => None,
        },
        _ => None,
    }
}

fn rewrite_docker_readonly_run_cmd_to_docker_basic(
    state: &AppState,
    preserve_explicit_command: bool,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if preserve_explicit_command {
        return actions;
    }
    if !docker_basic_available_for_plan(state) {
        return actions;
    }
    let mut rewritten = actions;
    let mut changed = false;
    for action in rewritten.iter_mut() {
        if !action_skill_is_run_cmd(action) {
            continue;
        }
        let Some(docker_action) =
            run_cmd_command_arg(action).and_then(docker_readonly_action_from_command)
        else {
            continue;
        };
        *action = AgentAction::CallSkill {
            skill: "docker_basic".to_string(),
            args: serde_json::json!({
                "action": docker_action,
            }),
        };
        changed = true;
    }
    if changed {
        info!("plan_rewrite_docker_readonly_run_cmd_to_docker_basic");
    }
    rewritten
}

fn action_is_path_metadata_facts_for_pair(
    action: &AgentAction,
    source: &str,
    archive: &str,
) -> bool {
    if !planned_action_is_path_metadata_facts(action) {
        return false;
    }
    let Some(args) = action_args(action).and_then(Value::as_object) else {
        return false;
    };
    let mut paths = string_list_from_value(args.get("paths").or_else(|| args.get("path")));
    paths.extend(string_list_from_value(args.get("targets")));
    if let Some(path) = args.get("left_path").and_then(Value::as_str) {
        paths.push(path.to_string());
    }
    if let Some(path) = args.get("right_path").and_then(Value::as_str) {
        paths.push(path.to_string());
    }
    paths
        .iter()
        .any(|path| path.ends_with(source) || path == source)
        && paths
            .iter()
            .any(|path| path.ends_with(archive) || path == archive)
}

fn rewrite_archive_pack_plan_to_archive_basic(
    route_result: Option<&RouteResult>,
    preserve_explicit_command: bool,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if preserve_explicit_command {
        return actions;
    }
    let Some(route) = route_result else {
        return actions;
    };
    let Some((source, archive)) = archive_pack_pair_for_route(route) else {
        return actions;
    };
    if actions.iter().any(action_is_archive_basic_pack) {
        return actions;
    }

    let mut rewritten = actions;
    let mut changed = false;
    for action in rewritten.iter_mut() {
        if action_is_archive_basic(action)
            || action_is_path_metadata_facts_for_pair(action, &source, &archive)
            || action_skill_is_run_cmd(action)
        {
            *action = AgentAction::CallSkill {
                skill: "archive_basic".to_string(),
                args: serde_json::json!({
                    "action": "pack",
                    "source": source,
                    "archive": archive,
                    "format": archive_format_for_path(&archive),
                }),
            };
            changed = true;
            break;
        }
    }
    if !changed {
        return rewritten;
    }

    let mut saw_pack = false;
    let mut has_post_pack_synthesis = false;
    rewritten.retain(|action| {
        if action_is_archive_basic(action) {
            saw_pack = true;
            return true;
        }
        if saw_pack && matches!(action, AgentAction::SynthesizeAnswer { .. }) {
            has_post_pack_synthesis = true;
            return true;
        }
        if saw_pack && matches!(action, AgentAction::Respond { .. }) {
            return false;
        }
        true
    });
    if !has_post_pack_synthesis {
        rewritten.push(AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        });
    }
    info!("plan_rewrite_archive_pack_plan_to_archive_basic");
    rewritten
}

/// 检测 `respond.content` 是否是裸的 `{{last_output}}` / `{{last_output.xxx}}` /
/// `{{last_output[xxx]}}` 之类纯模板占位符。
///
/// 这种形态会被 `delivery_text_classifier` 判为 `non_informative_placeholder`，
/// 触发 `plan_missing_terminal_user_answer` 重修，进而陷入 vendor patch 都救不回来的死循环
/// （兼容模型在 short-answer 类 act 任务里可能反复踩这个坑，prompt 指令忠实度不够）。
fn is_bare_last_output_placeholder(content: &str) -> bool {
    let trimmed = content.trim();
    if !trimmed.starts_with("{{") || !trimmed.ends_with("}}") {
        return false;
    }
    let inner = trimmed[2..trimmed.len() - 2].trim();
    let lower = inner.to_ascii_lowercase();
    lower == "last_output" || lower.starts_with("last_output.") || lower.starts_with("last_output[")
}

fn is_bare_template_placeholder(content: &str) -> bool {
    let trimmed = content.trim();
    if !trimmed.starts_with("{{") || !trimmed.ends_with("}}") {
        return false;
    }
    let inner = trimmed[2..trimmed.len() - 2].trim();
    !inner.is_empty() && !inner.contains("{{") && !inner.contains("}}")
}

fn extract_output_placeholder_evidence_refs(text: &str) -> Vec<String> {
    static PLACEHOLDER_BLOCK_RE: OnceLock<Regex> = OnceLock::new();
    static PLACEHOLDER_REF_RE: OnceLock<Regex> = OnceLock::new();
    let block_re = PLACEHOLDER_BLOCK_RE
        .get_or_init(|| Regex::new(r"\{\{\s*([^{}]+?)\s*\}\}").expect("placeholder block regex"));
    let ref_re = PLACEHOLDER_REF_RE.get_or_init(|| {
        Regex::new(
            r"\b(last_output(?:[.\[][^\s{}]*)?|s\d+(?:[._]?output)?|step_?\d+(?:[._]?output)?)\b",
        )
        .expect("placeholder reference regex")
    });
    let mut refs = Vec::new();
    for block in block_re.captures_iter(text) {
        let Some(inner) = block.get(1) else {
            continue;
        };
        let mut found_ref = false;
        for captures in ref_re.captures_iter(inner.as_str()) {
            let Some(matched) = captures.get(1) else {
                continue;
            };
            let token = normalize_output_placeholder_reference(matched.as_str());
            if !refs.iter().any(|existing| existing == &token) {
                refs.push(token);
            }
            found_ref = true;
        }
        if !found_ref && !refs.iter().any(|existing| existing == "last_output") {
            refs.push("last_output".to_string());
        }
    }
    refs
}

fn normalize_output_placeholder_reference(raw: &str) -> String {
    static STEP_UNDERSCORE_OUTPUT_RE: OnceLock<Regex> = OnceLock::new();
    static STEP_BARE_RE: OnceLock<Regex> = OnceLock::new();
    static S_UNDERSCORE_OUTPUT_RE: OnceLock<Regex> = OnceLock::new();
    let lower = raw.trim().to_ascii_lowercase();
    if lower.starts_with("last_output.") || lower.starts_with("last_output[") {
        return "last_output".to_string();
    }
    let step_underscore_output_re = STEP_UNDERSCORE_OUTPUT_RE.get_or_init(|| {
        Regex::new(r"^step_?(\d+)_output$").expect("step output placeholder regex")
    });
    if let Some(captures) = step_underscore_output_re.captures(&lower) {
        if let Some(number) = captures.get(1) {
            return format!("step_{}", number.as_str());
        }
    }
    let step_bare_re =
        STEP_BARE_RE.get_or_init(|| Regex::new(r"^step_?(\d+)$").expect("step placeholder regex"));
    if let Some(captures) = step_bare_re.captures(&lower) {
        if let Some(number) = captures.get(1) {
            return format!("step_{}", number.as_str());
        }
    }
    let s_underscore_output_re = S_UNDERSCORE_OUTPUT_RE
        .get_or_init(|| Regex::new(r"^s(\d+)_output$").expect("short step output regex"));
    if let Some(captures) = s_underscore_output_re.captures(&lower) {
        if let Some(number) = captures.get(1) {
            return format!("s{}", number.as_str());
        }
    }
    lower
}

fn has_loop_observation(loop_state: &LoopState) -> bool {
    loop_state.has_tool_or_skill_output
        || !loop_state.executed_step_results.is_empty()
        || loop_state
            .last_output
            .as_deref()
            .map(str::trim)
            .is_some_and(|text| !text.is_empty())
}

fn is_concrete_final_respond_content(content: &str) -> bool {
    let trimmed = content.trim();
    !trimmed.is_empty()
        && !is_bare_last_output_placeholder(trimmed)
        && extract_output_placeholder_evidence_refs(trimmed).is_empty()
}

fn route_should_prefer_observed_terminal_synthesis(route: Option<&RouteResult>) -> bool {
    let Some(route) = route else {
        return false;
    };
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::ServiceStatus {
        return false;
    }
    route.output_contract.requires_content_evidence
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Strict
        )
}

fn textual_grounding_token_has_signal(token: &str) -> bool {
    let token = token.trim();
    if token.len() < 2 {
        return false;
    }
    let has_ascii_alpha = token.chars().any(|ch| ch.is_ascii_alphabetic());
    let uppercase_count = token.chars().filter(|ch| ch.is_ascii_uppercase()).count();
    let lowercase_count = token.chars().filter(|ch| ch.is_ascii_lowercase()).count();
    let digit_count = token.chars().filter(|ch| ch.is_ascii_digit()).count();
    if token.contains('.') || token.contains('_') || token.contains('-') || token.contains('/') {
        return token.len() >= 3;
    }
    if digit_count > 0 && has_ascii_alpha && token.len() >= 3 {
        return true;
    }
    if uppercase_count >= 2 && token.len() <= 16 {
        return true;
    }
    uppercase_count >= 1
        && lowercase_count >= 1
        && token.chars().skip(1).any(|ch| ch.is_ascii_uppercase())
}

fn push_textual_grounding_tokens(raw: &str, out: &mut Vec<String>) {
    static TOKEN_RE: OnceLock<Regex> = OnceLock::new();
    let re = TOKEN_RE.get_or_init(|| {
        Regex::new(r"[A-Za-z0-9][A-Za-z0-9._/-]{1,63}").expect("valid text token regex")
    });
    for token in re.find_iter(raw).map(|m| m.as_str()) {
        if textual_grounding_token_has_signal(token) {
            out.push(token.to_string());
        }
    }
}

fn push_structural_grounding_tokens(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Null | Value::Bool(_) => {}
        Value::Number(number) => {
            let token = number.to_string();
            if token.chars().filter(|ch| ch.is_ascii_digit()).count() >= 2 {
                out.push(token);
            }
        }
        Value::String(raw) => {
            let token = raw.trim().replace('\\', "/");
            push_textual_grounding_tokens(&token, out);
            if token.len() < 3 || token.chars().any(char::is_whitespace) && !token.contains('/') {
                return;
            }
            let has_structural_shape = token.contains('/')
                || token.contains('.')
                || token.contains('_')
                || token.contains('-')
                || token.chars().all(|ch| ch.is_ascii_digit());
            if !has_structural_shape {
                return;
            }
            out.push(token.clone());
            if let Some(basename) = token.rsplit('/').next().filter(|part| part.len() >= 3) {
                out.push(basename.to_string());
            }
        }
        Value::Array(items) => {
            for item in items {
                push_structural_grounding_tokens(item, out);
            }
        }
        Value::Object(map) => {
            for value in map.values() {
                push_structural_grounding_tokens(value, out);
            }
        }
    }
}

fn concrete_respond_has_structural_observation_anchors(
    loop_state: &LoopState,
    content: &str,
) -> bool {
    let content = content.trim();
    if content.is_empty() || content.contains("{{") {
        return false;
    }
    let haystack = content.replace('\\', "/").to_ascii_lowercase();
    let mut matched = HashSet::new();
    for output in loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| {
            step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "think" | "synthesize_answer"
                )
        })
        .filter_map(|step| step.output.as_deref())
    {
        let mut tokens = Vec::new();
        if let Ok(value) = serde_json::from_str::<Value>(output) {
            push_structural_grounding_tokens(&value, &mut tokens);
        } else {
            push_textual_grounding_tokens(output, &mut tokens);
            tokens.extend(
                output
                    .lines()
                    .map(str::trim)
                    .filter(|line| {
                        line.len() >= 3
                            && !line.chars().any(char::is_whitespace)
                            && (line.contains('/')
                                || line.contains('.')
                                || line.contains('_')
                                || line.contains('-')
                                || line.chars().all(|ch| ch.is_ascii_digit()))
                    })
                    .map(ToString::to_string),
            );
        }
        for token in tokens {
            let token = token.trim().to_ascii_lowercase();
            if token.len() >= 2 && haystack.contains(&token) {
                matched.insert(token);
                if matched.len() >= 2 {
                    return true;
                }
            }
        }
    }
    false
}

fn rewrite_observed_terminal_synthesis_concrete_respond(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if actions.len() < 2
        || !loop_state.has_tool_or_skill_output
        || !route_should_prefer_observed_terminal_synthesis(route_result)
    {
        return actions;
    }
    let last_idx = actions.len() - 1;
    if !matches!(
        actions.get(last_idx - 1),
        Some(AgentAction::SynthesizeAnswer { .. })
    ) {
        return actions;
    }
    let Some(AgentAction::Respond { content }) = actions.get(last_idx) else {
        return actions;
    };
    if !is_concrete_final_respond_content(content) {
        return actions;
    }
    let content_excerpt_contract = route_result.is_some_and(|route| {
        route
            .output_contract
            .semantic_kind
            .is_content_excerpt_summary()
    });
    if content_excerpt_contract {
        let mut rewritten = actions;
        rewritten[last_idx] = AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        };
        info!("plan_rewrite_content_excerpt_concrete_respond_after_synthesis");
        return rewritten;
    }
    if concrete_respond_has_structural_observation_anchors(loop_state, content) {
        info!("plan_keep_structurally_grounded_concrete_respond_after_synthesis");
        return actions;
    }
    let mut rewritten = actions;
    rewritten[last_idx] = AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    };
    info!("plan_rewrite_observed_terminal_synthesis_concrete_respond");
    rewritten
}

/// Planner-first shape guard: before any observation has run, a leading
/// `synthesize_answer` is redundant when a later concrete `respond` already
/// exists.
///
/// This is not a natural-language shortcut; it only repairs the plan graph so
/// a redundant synthesis node does not block an already concrete final answer.
fn strip_pre_observation_synthesize_before_concrete_respond(
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if actions.len() < 2 {
        return actions;
    }
    if loop_state.has_tool_or_skill_output || !loop_state.executed_step_results.is_empty() {
        return actions;
    }

    let mut has_future_concrete_respond = vec![false; actions.len()];
    let mut future_concrete_respond = false;
    for idx in (0..actions.len()).rev() {
        has_future_concrete_respond[idx] = future_concrete_respond;
        if let AgentAction::Respond { content } = &actions[idx] {
            future_concrete_respond |= is_concrete_final_respond_content(content);
        }
    }
    if !future_concrete_respond {
        return actions;
    }

    let mut rewritten = Vec::with_capacity(actions.len());
    let mut saw_observation_action = false;
    let mut dropped = 0usize;
    for (idx, action) in actions.into_iter().enumerate() {
        match &action {
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. } => {
                saw_observation_action = true;
                rewritten.push(action);
            }
            AgentAction::SynthesizeAnswer { .. }
                if !saw_observation_action && has_future_concrete_respond[idx] =>
            {
                dropped += 1;
            }
            _ => rewritten.push(action),
        }
    }

    if dropped > 0 {
        info!("plan_strip_pre_observation_synthesize_before_concrete_respond dropped={dropped}");
    }
    rewritten
}

/// §F1：检测「未观测先编造」的幻觉 Respond，把内容改写为 `{{last_output}}` 占位，
/// 让下游 [`inject_synthesize_answer_for_bare_placeholder_respond`] 把它包成
/// `synthesize_answer` 节点，从而在执行完上游观测步后再用真实输出生成回复。
///
/// 触发条件（必须**全部**满足）：
/// 1. `loop_state` 仍是 round-1 状态：`executed_step_results` 为空（没有任何
///    skill 实际跑过），`last_output` 为空 → 这一批 actions 全部都还没执行。
/// 2. `actions` 末尾是 `Respond` 步。
/// 3. 倒数第二步是 `CallSkill` / `CallTool`（即「先跑后说」的常见 plan 形态）。
/// 4. Respond 的 content 不包含 `{{last_output}}` 之类的占位符
///    （[`is_bare_last_output_placeholder`] 已经在主入口处理纯占位符路径），
///    并且 content 长度足够 + 含「观测过才能知道」的特征 token：
///    - 含至少一行以数字+点开头的列表项（`1. xxx` / `2. xxx` …）；或
///    - 含 3+ 行换行 + 至少一个文件路径字符（`/`、`.md`、`.toml` 等）；或
///    - 含 `result: ` / `count: ` / `size: ` 这种结构化字段标签。
///
/// 这一招针对兼容模型偶发的「planner 一次性把 list_dir + respond 编造
/// 内容写在同一个 plan，respond 直接交给用户」复现路径。
/// 不命中条件时 actions 原样返回，不破坏正确 plan。
fn rewrite_pre_observation_concrete_respond_to_placeholder(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if route_result.is_some_and(|route| {
        route.output_contract.delivery_required
            || route.output_contract.response_shape == crate::OutputResponseShape::FileToken
    }) {
        return actions;
    }
    if actions.len() < 2 {
        return actions;
    }
    if !loop_state.executed_step_results.is_empty() {
        return actions;
    }
    if loop_state
        .last_output
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
    {
        return actions;
    }
    let last_idx = actions.len() - 1;
    let respond_content = match &actions[last_idx] {
        AgentAction::Respond { content } => content.clone(),
        _ => return actions,
    };
    if crate::finalize::parse_delivery_file_token(respond_content.trim()).is_some() {
        return actions;
    }
    let prior_is_observation = matches!(
        &actions[last_idx - 1],
        AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
    );
    if !prior_is_observation {
        return actions;
    }
    if is_bare_last_output_placeholder(&respond_content) {
        return actions;
    }
    let contract_requires_observed_answer = route_result
        .map(|route| route.output_contract.requires_content_evidence)
        .unwrap_or(false);
    if !contract_requires_observed_answer
        && !has_pre_observation_structured_output_shape(&respond_content)
    {
        return actions;
    }
    let mut rewritten = actions;
    let original_len = respond_content.len();
    let respond_idx = rewritten.len() - 1;
    if let AgentAction::Respond { content } = &mut rewritten[respond_idx] {
        *content = "{{last_output}}".to_string();
    }
    info!(
        "plan_rewrite_pre_observation_concrete_respond_to_placeholder original_len={} source={}",
        original_len,
        if contract_requires_observed_answer {
            "output_contract"
        } else {
            "shape_guard"
        }
    );
    rewritten
}

fn rewrite_terminal_placeholder_respond_to_synthesize_answer(
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if actions.len() < 2 || !loop_state.executed_step_results.is_empty() {
        return actions;
    }
    if loop_state
        .last_output
        .as_deref()
        .map(|text| !text.trim().is_empty())
        .unwrap_or(false)
    {
        return actions;
    }
    let last_idx = actions.len() - 1;
    let respond_content = match &actions[last_idx] {
        AgentAction::Respond { content } => content.as_str(),
        _ => return actions,
    };
    if is_bare_last_output_placeholder(respond_content) {
        return actions;
    }
    let evidence_refs = extract_output_placeholder_evidence_refs(respond_content);
    if evidence_refs.is_empty() {
        return actions;
    }
    if mixed_last_output_respond_has_concrete_text(respond_content, &evidence_refs) {
        return actions;
    }
    let Some(previous_action) = actions[..last_idx]
        .iter()
        .rev()
        .find(|candidate| !matches!(candidate, AgentAction::Think { .. }))
    else {
        return actions;
    };
    if !matches!(
        previous_action,
        AgentAction::CallSkill { .. }
            | AgentAction::CallTool { .. }
            | AgentAction::SynthesizeAnswer { .. }
    ) {
        return actions;
    }
    let mut rewritten = actions;
    rewritten[last_idx] = AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    };
    rewritten.insert(
        last_idx,
        AgentAction::SynthesizeAnswer {
            evidence_refs: evidence_refs.clone(),
        },
    );
    info!(
        "plan_rewrite_terminal_placeholder_respond_to_synthesize_answer refs={}",
        evidence_refs.join(",")
    );
    rewritten
}

fn content_excerpt_summary_target_path(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || route.output_contract.delivery_required
        || !route
            .output_contract
            .semantic_kind
            .is_content_excerpt_summary()
    {
        return None;
    }
    auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or_else(|| {
            let hint = route.output_contract.locator_hint.trim();
            (!hint.is_empty() && Path::new(hint).is_file()).then_some(hint)
        })
        .filter(|path| Path::new(path).is_file())
        .map(ToString::to_string)
}

fn ensure_content_excerpt_summary_has_bounded_content(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output {
        return actions;
    }
    let Some(path) = content_excerpt_summary_target_path(route_result, auto_locator_path) else {
        return actions;
    };
    let mut rewritten = actions;
    if !rewritten.iter().any(action_observes_bounded_file_content) {
        let insert_at = rewritten
            .iter()
            .position(|action| {
                matches!(
                    action,
                    AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. }
                )
            })
            .unwrap_or(rewritten.len());
        rewritten.insert(
            insert_at,
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "read_text_range",
                    "path": path,
                    "mode": "head",
                    "n": 40
                }),
            },
        );
        info!("plan_insert_content_excerpt_summary_read_range");
    }
    if !rewritten
        .iter()
        .any(|action| matches!(action, AgentAction::SynthesizeAnswer { .. }))
    {
        let evidence_refs = observation_action_evidence_refs(&rewritten);
        if !evidence_refs.is_empty() {
            let insert_at = rewritten
                .iter()
                .rposition(|action| matches!(action, AgentAction::Respond { .. }))
                .unwrap_or(rewritten.len());
            rewritten.insert(
                insert_at,
                AgentAction::SynthesizeAnswer {
                    evidence_refs: evidence_refs.clone(),
                },
            );
            match rewritten.get_mut(insert_at + 1) {
                Some(AgentAction::Respond { content }) => {
                    *content = "{{last_output}}".to_string();
                }
                _ => rewritten.push(AgentAction::Respond {
                    content: "{{last_output}}".to_string(),
                }),
            }
            info!(
                "plan_insert_content_excerpt_summary_synthesis refs={}",
                evidence_refs.join(",")
            );
        }
    }
    rewritten
}

fn rewrite_terminal_synthesis_placeholder_respond(actions: Vec<AgentAction>) -> Vec<AgentAction> {
    if actions.len() < 2 {
        return actions;
    }
    let last_idx = actions.len() - 1;
    let respond_content = match &actions[last_idx] {
        AgentAction::Respond { content } => content.as_str(),
        _ => return actions,
    };
    if !is_bare_template_placeholder(respond_content) {
        return actions;
    }
    let Some(previous_action) = actions[..last_idx]
        .iter()
        .rev()
        .find(|candidate| !matches!(candidate, AgentAction::Think { .. }))
    else {
        return actions;
    };
    if !matches!(previous_action, AgentAction::SynthesizeAnswer { .. }) {
        return actions;
    }
    let mut rewritten = actions;
    rewritten[last_idx] = AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    };
    info!("plan_rewrite_terminal_synthesis_placeholder_respond");
    rewritten
}

fn route_requires_observed_synthesis_for_mixed_placeholder(route: &RouteResult) -> bool {
    route.output_contract.requires_content_evidence
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::RecentArtifactsJudgment
                | crate::OutputSemanticKind::DirectoryPurposeSummary
                | crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::ContentExcerptWithSummary
                | crate::OutputSemanticKind::ExcerptKindJudgment
                | crate::OutputSemanticKind::WorkspaceProjectSummary
                | crate::OutputSemanticKind::SqliteDatabaseKindJudgment
                | crate::OutputSemanticKind::ServiceStatus
        )
}

fn rewrite_mixed_placeholder_observed_synthesis_respond(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route_requires_observed_synthesis_for_mixed_placeholder(route)
        || actions.len() < 2
        || has_loop_observation(loop_state)
    {
        return actions;
    }
    let last_idx = actions.len() - 1;
    let respond_content = match &actions[last_idx] {
        AgentAction::Respond { content } => content.as_str(),
        _ => return actions,
    };
    let evidence_refs = extract_output_placeholder_evidence_refs(respond_content);
    if !mixed_last_output_respond_has_concrete_text(respond_content, &evidence_refs) {
        return actions;
    }
    let Some(previous_action) = actions[..last_idx]
        .iter()
        .rev()
        .find(|candidate| !matches!(candidate, AgentAction::Think { .. }))
    else {
        return actions;
    };
    if !matches!(
        previous_action,
        AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
    ) {
        return actions;
    }
    let mut rewritten = actions;
    rewritten[last_idx] = AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    };
    rewritten.insert(
        last_idx,
        AgentAction::SynthesizeAnswer {
            evidence_refs: if evidence_refs.is_empty() {
                vec!["last_output".to_string()]
            } else {
                evidence_refs.clone()
            },
        },
    );
    info!(
        "plan_rewrite_mixed_placeholder_observed_synthesis_respond refs={}",
        evidence_refs.join(",")
    );
    rewritten
}

fn action_emits_structured_output_for_placeholder(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let action_name = args
                .get("action")
                .and_then(Value::as_str)
                .map(|action| action.trim().to_ascii_lowercase());
            let action_name = action_name.as_deref();
            match skill.as_str() {
                "fs_basic" => matches!(
                    action_name,
                    Some(
                        "stat_paths"
                            | "list_dir"
                            | "count_entries"
                            | "read_text_range"
                            | "find_entries"
                            | "grep_text"
                            | "compare_paths"
                    )
                ),
                "system_basic" => matches!(
                    action_name,
                    Some(
                        "inventory_dir"
                            | "count_inventory"
                            | "workspace_glance"
                            | "tree_summary"
                            | "extract_field"
                            | "extract_fields"
                            | "structured_keys"
                            | "validate_structured"
                            | "find_path"
                            | "read_range"
                            | "compare_paths"
                            | "path_batch_facts"
                            | "diagnose_runtime"
                    )
                ),
                "config_basic" => matches!(
                    action_name,
                    Some("read_field" | "read_fields" | "list_keys" | "validate")
                ),
                _ => false,
            }
        }
        _ => false,
    }
}

fn rewrite_mixed_placeholder_structured_output_respond(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if route_explicitly_requests_raw_command_output(route_result)
        || actions.len() < 2
        || has_loop_observation(loop_state)
    {
        return actions;
    }
    let last_idx = actions.len() - 1;
    let respond_content = match &actions[last_idx] {
        AgentAction::Respond { content } => content.as_str(),
        _ => return actions,
    };
    let evidence_refs = extract_output_placeholder_evidence_refs(respond_content);
    if !mixed_last_output_respond_has_concrete_text(respond_content, &evidence_refs) {
        return actions;
    }
    let Some(previous_action) = actions[..last_idx]
        .iter()
        .rev()
        .find(|candidate| !matches!(candidate, AgentAction::Think { .. }))
    else {
        return actions;
    };
    if !action_emits_structured_output_for_placeholder(previous_action) {
        return actions;
    }
    let mut rewritten = actions;
    rewritten[last_idx] = AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    };
    rewritten.insert(
        last_idx,
        AgentAction::SynthesizeAnswer {
            evidence_refs: evidence_refs.clone(),
        },
    );
    info!(
        "plan_rewrite_mixed_placeholder_structured_output_respond refs={}",
        evidence_refs.join(",")
    );
    rewritten
}

fn mixed_last_output_respond_has_concrete_text(content: &str, evidence_refs: &[String]) -> bool {
    if evidence_refs.is_empty()
        || !evidence_refs
            .iter()
            .all(|reference| reference.trim().eq_ignore_ascii_case("last_output"))
    {
        return false;
    }
    static PLACEHOLDER_BLOCK_RE: OnceLock<Regex> = OnceLock::new();
    let block_re = PLACEHOLDER_BLOCK_RE
        .get_or_init(|| Regex::new(r"\{\{\s*([^{}]+?)\s*\}\}").expect("placeholder block regex"));
    let outside_placeholder_text = block_re.replace_all(content, "");
    !outside_placeholder_text.trim().is_empty()
}

fn has_mixed_last_output_terminal_respond(actions: &[AgentAction]) -> bool {
    let Some(AgentAction::Respond { content }) = actions.last() else {
        return false;
    };
    let evidence_refs = extract_output_placeholder_evidence_refs(content);
    mixed_last_output_respond_has_concrete_text(content, &evidence_refs)
}

fn terminal_mixed_last_output_respond_content(actions: &[AgentAction]) -> Option<String> {
    let Some(AgentAction::Respond { content }) = actions.last() else {
        return None;
    };
    let evidence_refs = extract_output_placeholder_evidence_refs(content);
    mixed_last_output_respond_has_concrete_text(content, &evidence_refs).then(|| content.clone())
}

fn restore_terminal_mixed_last_output_respond(
    route_result: Option<&RouteResult>,
    planned_content: Option<String>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !route_result.is_some_and(|route| {
        route.output_contract.response_shape == crate::OutputResponseShape::Strict
            && route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
    }) {
        return actions;
    }
    let Some(planned_content) = planned_content else {
        return actions;
    };
    if actions.len() < 3 {
        return actions;
    }
    let last_idx = actions.len() - 1;
    let synth_idx = last_idx - 1;
    let terminal_is_bare_last_output = matches!(
        &actions[last_idx],
        AgentAction::Respond { content } if is_bare_last_output_placeholder(content)
    );
    let synth_uses_last_output_only = matches!(
        &actions[synth_idx],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if !evidence_refs.is_empty()
                && evidence_refs
                    .iter()
                    .all(|reference| reference.trim().eq_ignore_ascii_case("last_output"))
    );
    if !terminal_is_bare_last_output || !synth_uses_last_output_only {
        return actions;
    }

    let mut rewritten = Vec::with_capacity(actions.len() - 1);
    rewritten.extend(actions[..synth_idx].iter().cloned());
    rewritten.push(AgentAction::Respond {
        content: planned_content,
    });
    info!("plan_restore_terminal_mixed_last_output_respond");
    rewritten
}

/// §F1 结构 guard：判断 Respond.content 是否像「未观测就编造」的工具输出形态。
///
/// 这个 guard 只看输出形态，不判断用户意图：
/// - 含至少一行以数字+点+空格开头的枚举项（最少 1 行；`1. foo` / `2. bar`）
/// - 含 3+ 行（`\n` ≥ 2）且至少含一个 `/` 或常见文件后缀
/// - 含明显结构化字段标签（`result:` / `count:` / `size:` / `path:`，大小写不敏感）
///
/// 这些形态在 round 1 尚未执行观察步骤时不应出现在直接 Respond 里；
/// 语义路由仍由 normalizer/planner 负责，不能在这里增加自然语言词面规则。
fn has_pre_observation_structured_output_shape(content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.len() < 8 {
        return false;
    }
    // 1) 数字枚举项（`\d+\. xxx`），至少 1 行。
    for line in trimmed.lines() {
        let l = line.trim_start();
        let bytes = l.as_bytes();
        if bytes.is_empty() || !bytes[0].is_ascii_digit() {
            continue;
        }
        let mut idx = 1usize;
        while idx < bytes.len() && bytes[idx].is_ascii_digit() {
            idx += 1;
        }
        if idx + 1 < bytes.len()
            && bytes[idx] == b'.'
            && (bytes[idx + 1] == b' ' || bytes[idx + 1] == b'\t')
        {
            return true;
        }
    }
    // 2) 3+ 行 + 含路径分隔符或常见后缀。
    let line_count = trimmed.lines().count();
    if line_count >= 3 {
        let lower = trimmed.to_ascii_lowercase();
        let has_pathlike = lower.contains('/')
            || lower.contains(".md")
            || lower.contains(".toml")
            || lower.contains(".json")
            || lower.contains(".rs")
            || lower.contains(".log")
            || lower.contains(".sh")
            || lower.contains(".py");
        if has_pathlike {
            return true;
        }
    }
    // 3) 结构化字段标签。
    let lower = trimmed.to_ascii_lowercase();
    for label in ["result:", "count:", "size:", "path:", "files:", "items:"] {
        if lower.contains(label) {
            return true;
        }
    }
    false
}

/// 当 plan 末尾是 `respond.content="{{last_output}}"` 这种裸 placeholder 时，
/// runtime 主动在 `respond` 之前注入一个 `synthesize_answer` 节点，
/// 把原始观察输出（命令 stdout / 文件内容 / 列表 / JSON / 错误信息）转成
/// 自然语言再交给 respond。这样 respond 拿到的 `{{last_output}}` 已经是
/// synthesize 节点产出的自然语言，能通过 `delivery_text_classifier` 的 publishable 检查。
///
/// 设计动机：
/// * Runtime 这一道兜底把证据归纳交给 `SynthesizeAnswer`，避免 planner 生成
///   只有 `{{last_output}}` 的不可发布回复。
/// * 不破坏正确 plan：仅当末尾是裸 placeholder Respond 且其前一步不是
///   `synthesize_answer` 时才注入。
fn inject_synthesize_answer_for_bare_placeholder_respond(
    actions: Vec<AgentAction>,
    _user_text: &str,
) -> Vec<AgentAction> {
    if actions.len() < 2 {
        // 只有一个 Respond 时，前面没有 observation 可供 synthesis 使用，不动。
        return actions;
    }
    let last_idx = actions.len() - 1;
    let needs_inject = match &actions[last_idx] {
        AgentAction::Respond { content } => is_bare_last_output_placeholder(content),
        _ => false,
    };
    if !needs_inject {
        return actions;
    }
    match &actions[last_idx - 1] {
        AgentAction::SynthesizeAnswer { .. } => {
            return actions;
        }
        _ => {}
    }
    let mut rewritten = actions;
    let evidence_refs = observation_action_evidence_refs(&rewritten[..last_idx]);
    let synth_step = AgentAction::SynthesizeAnswer {
        evidence_refs: if evidence_refs.is_empty() {
            vec!["last_output".to_string()]
        } else {
            evidence_refs
        },
    };
    let respond = rewritten.pop().expect("non-empty checked above");
    rewritten.push(synth_step);
    rewritten.push(respond);
    info!("plan_inject_synthesize_answer_for_bare_placeholder_respond");
    rewritten
}

fn strip_intermediate_synthesize_before_later_execution(
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if actions.len() < 2 {
        return actions;
    }
    let mut stripped = Vec::with_capacity(actions.len());
    let mut changed = false;
    for (idx, action) in actions.iter().enumerate() {
        if matches!(action, AgentAction::SynthesizeAnswer { .. })
            && actions[idx + 1..].iter().any(|later| {
                matches!(
                    later,
                    AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
                )
            })
        {
            changed = true;
            continue;
        }
        stripped.push(action.clone());
    }
    if changed {
        info!("plan_strip_intermediate_synthesize_before_later_execution");
    }
    stripped
}

fn strip_terminal_placeholder_respond_for_exact_listing_contract(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::DirectoryNames
            | crate::OutputSemanticKind::FilePaths
    ) {
        return actions;
    }
    if !actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
        )
    }) {
        return actions;
    }
    let Some(AgentAction::Respond { content }) = actions.last() else {
        return actions;
    };
    if !is_bare_last_output_placeholder(content) {
        return actions;
    }
    let mut stripped = actions;
    stripped.pop();
    info!("plan_strip_terminal_placeholder_respond_for_exact_listing_contract");
    stripped
}

pub(super) async fn plan_round_actions(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    policy: &AgentLoopGuardPolicy,
    loop_state: &LoopState,
    turn_analysis_for_prompt: Option<&crate::intent_router::TurnAnalysis>,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Result<PlanResult, String> {
    let runtime_os = runtime_os_label();
    let runtime_shell = runtime_shell_label();
    let workspace_root = state.skill_rt.workspace_root.display().to_string();
    let planning_class = classify_planning_prompt_class(route_result, user_text, loop_state);
    let original_user_text_for_policy = crate::language_policy::task_original_user_text(task)
        .unwrap_or_else(|| user_text.to_string());
    let explicit_command_request = route_allows_explicit_command_preservation(route_result)
        && explicit_command_segment(&state.policy.command_intent, &original_user_text_for_policy)
            .is_some();
    let allow_structural_deterministic_plans = !explicit_command_request
        || structural_contract_deterministic_plan_overrides_literal_command_guard(route_result);
    if let Some(plan_result) = inline_json_transform_deterministic_plan_result(
        goal,
        state,
        loop_state,
        &original_user_text_for_policy,
        route_result,
    ) {
        info!(
            "plan_deterministic_inline_json_transform task_id={} round={}",
            task.task_id, loop_state.round_no
        );
        return Ok(plan_result);
    }
    if explicit_command_request {
        if let Some(plan_result) = explicit_command_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            &original_user_text_for_policy,
        ) {
            info!(
                "plan_deterministic_explicit_command_run_cmd task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
    }
    if let Some(plan_result) = active_task_append_current_locator_deterministic_plan_result(
        goal,
        route_result,
        loop_state,
        turn_analysis_for_prompt,
        auto_locator_path,
    ) {
        info!(
            "plan_deterministic_active_task_append_current_locator task_id={} round={}",
            task.task_id, loop_state.round_no
        );
        return Ok(plan_result);
    }
    if let Some(plan_result) = contract_hint_preferred_action_deterministic_plan_result(
        state,
        goal,
        route_result,
        loop_state,
        &original_user_text_for_policy,
        auto_locator_path,
    ) {
        info!(
            "plan_deterministic_contract_hint_preferred_action task_id={} round={}",
            task.task_id, loop_state.round_no
        );
        return Ok(plan_result);
    }
    if let Some(plan_result) = package_manager_detect_deterministic_plan_result(
        state,
        goal,
        route_result,
        loop_state,
        auto_locator_path,
    ) {
        info!(
            "plan_deterministic_package_manager_detect task_id={} round={}",
            task.task_id, loop_state.round_no
        );
        return Ok(plan_result);
    }
    if let Some(plan_result) = package_manager_dry_run_deterministic_plan_result(
        state,
        goal,
        route_result,
        loop_state,
        &original_user_text_for_policy,
    ) {
        info!(
            "plan_deterministic_package_manager_dry_run task_id={} round={}",
            task.task_id, loop_state.round_no
        );
        return Ok(plan_result);
    }
    if let Some(plan_result) = runtime_status_scalar_deterministic_plan_result(
        state,
        goal,
        route_result,
        loop_state,
        turn_analysis_for_prompt,
    ) {
        info!(
            "plan_deterministic_runtime_status_scalar task_id={} round={}",
            task.task_id, loop_state.round_no
        );
        return Ok(plan_result);
    }
    if let Some(plan_result) = runtime_status_scalar_info_fallback_plan_result(
        state,
        goal,
        route_result,
        loop_state,
        turn_analysis_for_prompt,
    ) {
        info!(
            "plan_deterministic_runtime_status_scalar_info_fallback task_id={} round={}",
            task.task_id, loop_state.round_no
        );
        return Ok(plan_result);
    }
    if let Some(plan_result) = service_status_deterministic_plan_result(
        state,
        goal,
        route_result,
        loop_state,
        &original_user_text_for_policy,
    ) {
        info!(
            "plan_deterministic_service_status task_id={} round={}",
            task.task_id, loop_state.round_no
        );
        return Ok(plan_result);
    }
    if allow_structural_deterministic_plans {
        if let Some(plan_result) = directory_purpose_representative_reads_after_find_result(
            goal,
            route_result,
            loop_state,
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_directory_purpose_representative_reads task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = structured_keys_deterministic_plan_result(
            state,
            goal,
            &original_user_text_for_policy,
            route_result,
            loop_state,
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_structured_keys task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = git_repository_state_deterministic_plan_result(
            goal,
            route_result,
            loop_state,
            &original_user_text_for_policy,
        ) {
            info!(
                "plan_deterministic_git_repository_state task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = recent_scalar_file_pair_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            &original_user_text_for_policy,
            Some(&original_user_text_for_policy),
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_recent_scalar_file_pair task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = recent_scalar_current_workspace_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
        ) {
            info!(
                "plan_deterministic_recent_scalar_current_workspace task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = scalar_path_directory_locator_search_deterministic_plan_result(
            goal,
            route_result,
            loop_state,
            auto_locator_path,
            &original_user_text_for_policy,
        ) {
            info!(
                "plan_deterministic_scalar_path_directory_locator_search task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = scalar_content_auto_locator_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            user_text,
            Some(&original_user_text_for_policy),
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_scalar_content_auto_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = scalar_path_auto_locator_deterministic_plan_result(
            goal,
            route_result,
            loop_state,
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_scalar_path_auto_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = quantity_compare_pair_locator_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
        ) {
            info!(
                "plan_deterministic_quantity_compare_pair_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = file_facts_auto_locator_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            user_text,
            Some(&original_user_text_for_policy),
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_file_facts_auto_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = existence_with_path_locator_deterministic_plan_result(
            goal,
            route_result,
            loop_state,
            auto_locator_path,
            &original_user_text_for_policy,
        ) {
            info!(
                "plan_deterministic_existence_with_path_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = file_paths_locator_deterministic_plan_result(
            goal,
            route_result,
            loop_state,
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_file_paths_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = directory_compare_locator_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
        ) {
            info!(
                "plan_deterministic_directory_compare_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = directory_entry_groups_auto_locator_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            user_text,
            Some(&original_user_text_for_policy),
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_directory_entry_groups_auto_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = directory_purpose_extension_inventory_deterministic_plan_result(
            goal,
            route_result,
            loop_state,
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_directory_purpose_extension_inventory task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = directory_tree_auto_locator_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            user_text,
            Some(&original_user_text_for_policy),
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_directory_tree_auto_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = archive_read_deterministic_plan_result(
            goal,
            state,
            route_result,
            loop_state,
            auto_locator_path,
            &original_user_text_for_policy,
        ) {
            info!(
                "plan_deterministic_archive_read task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) =
            archive_unpack_deterministic_plan_result(goal, state, route_result, loop_state)
        {
            info!(
                "plan_deterministic_archive_unpack task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = archive_list_auto_locator_deterministic_plan_result(
            goal,
            state,
            route_result,
            loop_state,
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_archive_list_auto_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = content_excerpt_summary_auto_locator_deterministic_plan_result(
            goal,
            route_result,
            loop_state,
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_content_excerpt_summary_auto_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
    }
    let recent_assistant_replies = if matches!(planning_class, PlanningPromptClass::OpenPlanning) {
        crate::memory::build_recent_assistant_replies_context(
            state,
            task.user_key.as_deref(),
            task.user_id,
            task.chat_id,
            3,
            220,
        )
    } else {
        "<omitted: lightweight_execution>".to_string()
    };
    let skill_playbooks = if matches!(planning_class, PlanningPromptClass::OpenPlanning) {
        build_skill_playbooks_text(state, task)
    } else {
        build_lightweight_skill_playbooks_text(state, task)
    };
    let skill_quick_index = if matches!(planning_class, PlanningPromptClass::OpenPlanning) {
        build_skill_quick_index_text(state, task)
    } else {
        build_lightweight_skill_quick_index_text(state, task)
    };
    let tool_spec_template = if matches!(planning_class, PlanningPromptClass::OpenPlanning) {
        crate::bootstrap::load_required_prompt_template_for_state(state, AGENT_TOOL_SPEC_PATH)
            .map_err(|e| e.to_string())?
            .0
    } else {
        build_lightweight_tool_spec(route_result, auto_locator_path)
    };
    let turn_analysis = build_turn_analysis_prompt_block(turn_analysis_for_prompt, route_result);
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_text);
    let user_request_for_prompt =
        crate::language_policy::task_user_request_for_prompt(task, user_text);
    let attempt_ledger = build_attempt_ledger_compact(loop_state);
    let (prompt_name, prompt_source, prompt_version, prompt_text) = if loop_state.round_no <= 1 {
        let (prompt_name, prompt_logical_path) = round1_prompt_spec_for_class(planning_class);
        let resolved = crate::bootstrap::load_required_prompt_template_for_state_with_meta(
            state,
            prompt_logical_path,
        )
        .map_err(|e| e.to_string())?;
        (prompt_name, resolved.source, resolved.version, {
            let mut prompt = build_single_plan_prompt(
                &resolved.template,
                &user_request_for_prompt,
                goal,
                &turn_analysis,
                &tool_spec_template,
                &skill_playbooks,
                &recent_assistant_replies,
                &request_language_hint,
                &state.policy.command_intent.default_locale,
                &runtime_os,
                &runtime_shell,
                &workspace_root,
            );
            if matches!(planning_class, PlanningPromptClass::OpenPlanning) {
                prompt.push_str(
                        "\n\n## Skill Quick Index (first-round routing hint)\nGoal: reduce misclassification while minimizing avoidable extra rounds.\n- Do NOT end round-1 with a generic chat-style final answer when a skill might be relevant.\n- In round-1, prioritize intent classification + missing-slot check, but finish immediately when one bounded resolution/current-runtime step can already complete the request safely.\n- Ask one concise clarification only when safe completion is truly blocked after current-turn text, immediate context, and bounded resolution/default inference have been used.\n- Use immediate `call_skill` in round-1 whenever intent is clear or can be completed by one bounded resolution/current-runtime step.\n",
                    );
                prompt.push_str(&skill_quick_index);
                prompt.push('\n');
            }
            prompt
        })
    } else {
        let history_compact = build_loop_history_compact(loop_state);
        // Phase 3.3 / observation history regression fix:
        // 之前这里只读 delivery_messages.last()。delivery_messages 仅承载最终 respond/交付
        // 文本，observation-only 步骤（fs_search/list_dir/read_file/run_cmd 等）的输出从不
        // 写入这里。结果是 round N+1 的 loop planner 看到 "Last round output: (none)"，
        // 完全看不到 round N 的工具输出，于是会重复同一观察步骤，最终触发 plan_unactionable
        // 兜底（i18n 模板被误用作 "provider unavailable" 文案）。
        // 真正记录每步输出的字段是 LoopState.last_output（agent_engine.rs 中
        // register_step_output / register_failed_step_output 都会维护）。优先使用它，
        // 仅在确无 step output 时回退到 delivery_messages，最后退化到占位符。
        let last_output = loop_state
            .last_output
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| crate::truncate_for_log(s))
            .or_else(|| loop_state.delivery_messages.last().cloned())
            .unwrap_or_else(|| "(none)".to_string());
        let resolved = crate::bootstrap::load_required_prompt_template_for_state_with_meta(
            state,
            LOOP_INCREMENTAL_PLAN_PROMPT_LOGICAL_PATH,
        )
        .map_err(|e| e.to_string())?;
        (
            "loop_incremental_plan_prompt",
            resolved.source,
            resolved.version,
            build_incremental_plan_prompt(
                &resolved.template,
                &user_request_for_prompt,
                goal,
                &turn_analysis,
                &tool_spec_template,
                &skill_playbooks,
                &recent_assistant_replies,
                &request_language_hint,
                &state.policy.command_intent.default_locale,
                loop_state.round_no,
                &history_compact,
                &attempt_ledger,
                &last_output,
                &runtime_os,
                &runtime_shell,
                &workspace_root,
            ),
        )
    };
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        prompt_name,
        &prompt_source,
        prompt_version.as_deref(),
        Some(loop_state.round_no),
    );
    info!(
        "{} loop_round_plan task_id={} round={} max_rounds={} max_steps={} multi_round_enabled={}",
        crate::highlight_tag("loop"),
        task.task_id,
        loop_state.round_no,
        policy.max_rounds,
        policy.max_steps,
        policy.multi_round_enabled
    );
    info!(
        "plan_llm_request task_id={} round={} planning_class={} prompt_chars={} tool_spec_chars={} playbooks_chars={} recent_replies_chars={} user_request={}",
        task.task_id,
        loop_state.round_no,
        planning_class.as_str(),
        prompt_text.chars().count(),
        tool_spec_template.chars().count(),
        skill_playbooks.chars().count(),
        recent_assistant_replies.chars().count(),
        crate::truncate_for_log(user_text)
    );
    ensure_required_contract_block_present(route_result, &prompt_text)?;
    let plan_raw = llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &prompt_text,
        &prompt_source,
    )
    .await?;
    info!(
        "plan_llm_response task_id={} round={} raw={}",
        task.task_id,
        loop_state.round_no,
        crate::truncate_for_log(&plan_raw)
    );
    let parsed_actions = parse_single_plan_actions(&plan_raw, state, task).await;
    let initial_actions = parsed_actions
        .or_else(|| {
            let fallback = scalar_path_directory_locator_search_observation_plan(
                route_result,
                auto_locator_path,
                &original_user_text_for_policy,
            );
            if fallback.is_some() {
                warn!(
                    "plan_parse_failed_using_scalar_path_directory_locator_search_plan task_id={} round={}",
                    task.task_id, loop_state.round_no
                );
            }
            fallback
        })
        .or_else(|| {
            let fallback =
                scalar_content_auto_locator_observation_plan(route_result, auto_locator_path);
            if fallback.is_some() {
                warn!(
                    "plan_parse_failed_using_scalar_content_auto_locator_plan task_id={} round={}",
                    task.task_id, loop_state.round_no
                );
            }
            fallback
        })
        .or_else(|| {
            let fallback =
                scalar_path_auto_locator_observation_plan(route_result, auto_locator_path);
            if fallback.is_some() {
                warn!(
                    "plan_parse_failed_using_auto_locator_observation_plan task_id={} round={}",
                    task.task_id, loop_state.round_no
                );
            }
            fallback
        })
        .or_else(|| {
            let fallback =
                file_facts_auto_locator_observation_plan(route_result, auto_locator_path);
            if fallback.is_some() {
                warn!(
                    "plan_parse_failed_using_file_facts_auto_locator_plan task_id={} round={}",
                    task.task_id, loop_state.round_no
                );
            }
            fallback
        })
        .or_else(|| {
            let fallback =
                generic_directory_auto_locator_observation_plan(route_result, auto_locator_path);
            if fallback.is_some() {
                warn!(
                    "plan_parse_failed_using_generic_directory_auto_locator_plan task_id={} round={}",
                    task.task_id, loop_state.round_no
                );
            }
            fallback
        })
        .or_else(|| {
            let route = route_result?;
            if loop_state.has_tool_or_skill_output
                || !route_needs_workspace_respond_only_default_evidence(route)
            {
                return None;
            }
            warn!(
                "plan_parse_failed_using_workspace_default_evidence_plan task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            Some(workspace_summary_default_evidence_actions())
        })
        .map(|actions| {
            normalize_planned_actions_with_original_and_context(
                state,
                route_result,
                loop_state,
                user_text,
                Some(&original_user_text_for_policy),
                Some(goal),
                auto_locator_path,
                actions,
            )
        });
    let needs_repair = match initial_actions.as_ref() {
        Some(actions) => {
            should_force_actionable_plan_repair(state, route_result, loop_state, actions)
        }
        None => true,
    };
    let (plan_actions, plan_kind, raw_plan_text) = if needs_repair {
        let repair_reason =
            plan_repair_reason(state, route_result, loop_state, initial_actions.as_deref());
        warn!(
            "plan_repair_required task_id={} round={} reason={}",
            task.task_id, loop_state.round_no, repair_reason
        );
        // Planner-first: do not synthesize semantic repair plans from local keyword rules.
        // Repair either comes from the LLM repair prompt or, if safe, from the original
        // executable plan that the model already produced.
        match repair_plan_actions(
            state,
            task,
            goal,
            &turn_analysis,
            user_text,
            repair_reason,
            &tool_spec_template,
            &skill_playbooks,
            &attempt_ledger,
            &plan_raw,
            loop_state.round_no,
        )
        .await
        {
            Ok(repaired) => {
                let repaired_actions =
                    parse_single_plan_actions(&repaired, state, task)
                        .await
                        .map(|actions| {
                            normalize_planned_actions_with_original_and_context(
                                state,
                                route_result,
                                loop_state,
                                user_text,
                                Some(&original_user_text_for_policy),
                                Some(goal),
                                auto_locator_path,
                                actions,
                            )
                        });
                match repaired_actions {
                    Some(actions)
                        if !should_force_actionable_plan_repair(
                            state,
                            route_result,
                            loop_state,
                            &actions,
                        ) =>
                    {
                        (actions, PlanKind::Repair, repaired)
                    }
                    Some(actions) => {
                        let second_repair_reason =
                            plan_repair_reason(state, route_result, loop_state, Some(&actions));
                        warn!(
                            "plan_repair_still_invalid task_id={} round={} reason={}",
                            task.task_id, loop_state.round_no, second_repair_reason
                        );
                        let second_repaired = repair_plan_actions(
                            state,
                            task,
                            goal,
                            &turn_analysis,
                            user_text,
                            second_repair_reason,
                            &tool_spec_template,
                            &skill_playbooks,
                            &attempt_ledger,
                            &repaired,
                            loop_state.round_no,
                        )
                        .await?;
                        let second_repaired_actions =
                            parse_single_plan_actions(&second_repaired, state, task)
                                .await
                                .map(|actions| {
                                    normalize_planned_actions_with_original_and_context(
                                        state,
                                        route_result,
                                        loop_state,
                                        user_text,
                                        Some(&original_user_text_for_policy),
                                        Some(goal),
                                        auto_locator_path,
                                        actions,
                                    )
                                });
                        match second_repaired_actions {
                            Some(second_actions)
                                if !should_force_actionable_plan_repair(
                                    state,
                                    route_result,
                                    loop_state,
                                    &second_actions,
                                ) =>
                            {
                                (second_actions, PlanKind::Repair, second_repaired)
                            }
                            Some(_) => {
                                let fallback_actions = initial_actions.as_ref().filter(|actions| {
                                    can_fallback_to_initial_plan_after_repair_failure(
                                        state,
                                        route_result,
                                        loop_state,
                                        actions,
                                    )
                                });
                                if let Some(actions) = fallback_actions {
                                    warn!(
                                        "plan_second_repair_invalid_fallback_to_initial task_id={} round={}",
                                        task.task_id, loop_state.round_no
                                    );
                                    (
                                        actions.clone(),
                                        if loop_state.round_no <= 1 {
                                            PlanKind::Single
                                        } else {
                                            PlanKind::Incremental
                                        },
                                        plan_raw.clone(),
                                    )
                                } else {
                                    return Err(
                                        "repair plan still non-actionable after second repair"
                                            .to_string(),
                                    );
                                }
                            }
                            None => {
                                let fallback_actions = initial_actions.as_ref().filter(|actions| {
                                    can_fallback_to_initial_plan_after_repair_failure(
                                        state,
                                        route_result,
                                        loop_state,
                                        actions,
                                    )
                                });
                                if let Some(actions) = fallback_actions {
                                    warn!(
                                        "plan_second_repair_parse_failed_fallback_to_initial task_id={} round={}",
                                        task.task_id, loop_state.round_no
                                    );
                                    (
                                        actions.clone(),
                                        if loop_state.round_no <= 1 {
                                            PlanKind::Single
                                        } else {
                                            PlanKind::Incremental
                                        },
                                        plan_raw.clone(),
                                    )
                                } else {
                                    return Err(
                                        "second repair plan parser failed: no executable steps"
                                            .to_string(),
                                    );
                                }
                            }
                        }
                    }
                    None => {
                        let fallback_actions = initial_actions.as_ref().filter(|actions| {
                            can_fallback_to_initial_plan_after_repair_failure(
                                state,
                                route_result,
                                loop_state,
                                actions,
                            )
                        });
                        if let Some(actions) = fallback_actions {
                            warn!(
                                "plan_repair_parse_failed_fallback_to_initial task_id={} round={}",
                                task.task_id, loop_state.round_no
                            );
                            (
                                actions.clone(),
                                if loop_state.round_no <= 1 {
                                    PlanKind::Single
                                } else {
                                    PlanKind::Incremental
                                },
                                plan_raw.clone(),
                            )
                        } else {
                            return Err(
                                "single plan parser failed: no executable steps".to_string()
                            );
                        }
                    }
                }
            }
            Err(err) => {
                let fallback_actions = initial_actions.as_ref().filter(|actions| {
                    can_fallback_to_initial_plan_after_repair_failure(
                        state,
                        route_result,
                        loop_state,
                        actions,
                    )
                });
                if let Some(actions) = fallback_actions {
                    warn!(
                        "plan_repair_llm_failed_fallback_to_initial task_id={} round={} error={}",
                        task.task_id,
                        loop_state.round_no,
                        crate::truncate_for_log(&err)
                    );
                    (
                        actions.clone(),
                        if loop_state.round_no <= 1 {
                            PlanKind::Single
                        } else {
                            PlanKind::Incremental
                        },
                        plan_raw.clone(),
                    )
                } else {
                    return Err(err);
                }
            }
        }
    } else {
        (
            initial_actions.expect("checked Some above"),
            if loop_state.round_no <= 1 {
                PlanKind::Single
            } else {
                PlanKind::Incremental
            },
            plan_raw.clone(),
        )
    };
    let plan_result = build_plan_result(goal, &raw_plan_text, plan_kind, &plan_actions);
    let labels = plan_result.step_labels();
    info!(
        "act_split_trace task_id={} round={} split_steps={}",
        task.task_id,
        loop_state.round_no,
        serde_json::to_string(&labels).unwrap_or_else(|_| "[]".to_string())
    );
    Ok(plan_result)
}

#[cfg(test)]
#[path = "planning_tests.rs"]
mod tests;
