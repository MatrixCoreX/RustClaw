use regex::Regex;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tracing::{info, warn};

use super::{
    build_loop_history_compact, build_single_plan_prompt, build_skill_playbooks_text,
    build_skill_quick_index_text, build_turn_analysis_prompt_block, plan_step_label,
    AgentLoopGuardPolicy, LoopState, SinglePlanEnvelope, AGENT_TOOL_SPEC_PATH,
    LIGHTWEIGHT_EXECUTION_PROMPT_LOGICAL_PATH, LOOP_INCREMENTAL_PLAN_PROMPT_LOGICAL_PATH,
    PLAN_REPAIR_PROMPT_LOGICAL_PATH, SINGLE_PLAN_EXECUTION_PROMPT_LOGICAL_PATH,
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
            ("__LAST_ROUND_OUTPUT__", last_round_output),
            ("__RUNTIME_OS__", runtime_os),
            ("__RUNTIME_SHELL__", runtime_shell),
            ("__WORKSPACE_ROOT__", workspace_root),
        ],
    )
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
        lines.push(format!(
            "- routed_mode={} response_shape={} semantic_kind={} locator_kind={}",
            route.routed_mode.as_str(),
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
    if parts.is_empty() {
        None
    } else {
        Some(format!("Registry metadata: {}", parts.join("; ")))
    }
}

fn build_lightweight_skill_playbooks_text(state: &AppState, task: &ClaimedTask) -> String {
    let visible = state.planner_visible_skills_for_task(task);
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
    let visible = state.planner_visible_skills_for_task(task);
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

fn parse_minimax_parameter_value(raw: &str) -> Value {
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

fn extract_minimax_tool_call_steps(raw: &str) -> Vec<Value> {
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
                parse_minimax_parameter_value(&body[value_start..value_end]),
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
        step_values.extend(extract_minimax_tool_call_steps(raw));
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
                | AgentAction::SynthesizeAnswer { .. }
        )
    })
}

fn has_tool_or_skill_observation(actions: &[AgentAction]) -> bool {
    actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
        )
    })
}

fn planned_action_skill_name(action: &AgentAction) -> Option<&str> {
    match action {
        AgentAction::CallSkill { skill, .. } => Some(skill.as_str()),
        AgentAction::CallTool { tool, .. } => Some(tool.as_str()),
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
        })
    }) {
        return false;
    }
    actions.iter().any(|action| {
        let Some(skill) = planned_action_skill_name(action) else {
            return false;
        };
        let canonical = state.resolve_canonical_skill_name(skill);
        !(canonical.eq_ignore_ascii_case("run_cmd")
            && action_has_internal_literal_command_marker(action))
    })
}

fn action_has_internal_literal_command_marker(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => args
            .get(super::CLAWD_LITERAL_COMMAND_ARG)
            .and_then(Value::as_bool)
            .unwrap_or(false),
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
        | AgentAction::Think { .. } => false,
    })
}

fn is_discussion_followup_action(action: &AgentAction) -> bool {
    match action {
        AgentAction::Respond { .. } => true,
        AgentAction::SynthesizeAnswer { .. } => true,
        AgentAction::CallSkill { .. }
        | AgentAction::CallTool { .. }
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
            if canonical == "run_cmd" && route_explicitly_requests_raw_command_output(route_result)
            {
                return true;
            }
            if canonical == "process_basic" {
                return false;
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
        AgentAction::Respond { .. } | AgentAction::Think { .. } => false,
    }
}

fn observation_only_plan_can_finalize_from_direct_output(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: &[AgentAction],
) -> bool {
    last_executable_action(actions)
        .is_some_and(|action| action_supports_direct_observed_finalize(state, route_result, action))
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
            | crate::OutputSemanticKind::ServiceStatus
            | crate::OutputSemanticKind::ScalarPathOnly
            | crate::OutputSemanticKind::ExistenceWithPath
            | crate::OutputSemanticKind::GitCommitSubject
            | crate::OutputSemanticKind::StructuredKeys
            | crate::OutputSemanticKind::ArchiveList
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

fn replace_workspace_synthesis_respond_only_plan(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if loop_state.has_tool_or_skill_output
        || !route_needs_workspace_summary_default_evidence(route)
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
    vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "workspace_glance",
                "path": ".",
                "max_entries": 30,
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "extract_fields",
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
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "read_range",
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
        AgentAction::Respond { .. } | AgentAction::Think { .. } => false,
    }
}

fn is_non_mutating_run_cmd_action(state: &AppState, action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill, args)
        }
        AgentAction::SynthesizeAnswer { .. }
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
        AgentAction::SynthesizeAnswer { .. }
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
    if structured_scalar_compare_missing_required_extracts(route_result, actions) {
        return true;
    }
    if actions_use_ad_hoc_command_without_route_preferred_skill(state, route_result, actions) {
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
    if structured_scalar_compare_missing_required_extracts(route_result, actions) {
        return "structured_scalar_compare_requires_extract_fields";
    }
    if actions_use_ad_hoc_command_without_route_preferred_skill(state, route_result, actions) {
        return "preferred_skill_required_for_semantic_route";
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
    !route_result.needs_clarify
        && !loop_state.has_tool_or_skill_output
        && !contains_unavailable_skill_action(state, actions)
        && !structured_scalar_compare_missing_required_extracts(route_result, actions)
        && !actions_use_ad_hoc_command_without_route_preferred_skill(state, route_result, actions)
        && has_executable_observation_or_action(actions)
        && !has_discussion_followup_action(actions)
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
    let path = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
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
    Some(vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "path_batch_facts",
            "paths": [path],
            "include_missing": true,
        }),
    }])
}

fn scalar_path_auto_locator_fast_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let actions = scalar_path_auto_locator_observation_plan(route_result, auto_locator_path)?;
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
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "path_batch_facts",
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
    let Some(path) = scalar_count_locator_path(route_result, auto_locator_path) else {
        return actions;
    };
    info!("plan_replace_scalar_count_plan_with_count_inventory");
    vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "count_inventory",
            "path": path,
        }),
    }]
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
    [
        auto_locator_path
            .map(str::trim)
            .filter(|path| !path.is_empty()),
        (!hint.is_empty()).then_some(hint),
        (current_workspace_fallback || hint.is_empty()).then_some("."),
    ]
    .into_iter()
    .flatten()
    .find(|path| Path::new(path).is_dir())
    .map(ToString::to_string)
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
    vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "inventory_dir",
            "path": path,
            "include_hidden": true,
            "names_only": true,
            "max_entries": 1000,
        }),
    }]
}

fn route_requests_service_status(route: &RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::ServiceStatus
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
        AgentAction::Think { .. }
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
    if loop_state.has_tool_or_skill_output
        || actions.iter().any(|action| {
            matches!(
                action,
                AgentAction::CallSkill { skill, args }
                    | AgentAction::CallTool { tool: skill, args }
                    if skill.eq_ignore_ascii_case("system_basic")
                        && args.get("action").and_then(Value::as_str) == Some("structured_keys")
            )
        })
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
                if skill.eq_ignore_ascii_case("system_basic")
                    && matches!(
                        args.get("action").and_then(Value::as_str),
                        Some("read_range" | "extract_field" | "extract_fields")
                    )
        )
    }) {
        return actions;
    }
    info!("plan_replace_structured_keys_read_plan_with_structured_keys");
    vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "structured_keys",
            "path": path,
            "max_keys": 1000,
        }),
    }]
}

fn action_observes_bounded_file_content(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let skill = skill.trim();
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            (skill.eq_ignore_ascii_case("system_basic")
                && action.eq_ignore_ascii_case("read_range"))
                || skill.eq_ignore_ascii_case("read_file")
                || (skill.eq_ignore_ascii_case("doc_parse")
                    && action.eq_ignore_ascii_case("parse_doc"))
        }
        AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => false,
    }
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
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "read_range",
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

fn request_has_configured_explicit_command(
    runtime: &crate::CommandIntentRuntime,
    request: &str,
) -> bool {
    let request = request.trim();
    if request.is_empty() {
        return false;
    }
    let request_lower = request.to_ascii_lowercase();
    if runtime.negative_markers.iter().any(|marker| {
        let marker = marker.trim();
        !marker.is_empty() && request_lower.contains(&marker.to_ascii_lowercase())
    }) {
        return false;
    }
    runtime
        .execute_prefixes
        .iter()
        .filter_map(|prefix| strip_configured_command_prefix(request, prefix))
        .any(looks_like_concrete_command_tail)
}

fn explicit_command_segment_before_followup(tail: &str) -> Option<&str> {
    let tail = trim_leading_command_delimiters(tail);
    let boundary = tail.char_indices().find_map(|(idx, ch)| {
        (idx > 0 && matches!(ch, ',' | '，' | ';' | '；' | '。' | '\n')).then_some(idx)
    })?;
    Some(&tail[..boundary])
}

fn configured_explicit_command_segment(
    runtime: &crate::CommandIntentRuntime,
    request: &str,
) -> Option<String> {
    if !request_has_configured_explicit_command(runtime, request) {
        return None;
    }
    runtime
        .execute_prefixes
        .iter()
        .filter_map(|prefix| strip_configured_command_prefix(request, prefix))
        .filter_map(explicit_command_segment_before_followup)
        .map(|segment| crate::bootstrap::config_loaders::trim_command_text(segment.to_string()))
        .find(|segment| looks_like_concrete_command_tail(segment))
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
    if !request_has_configured_explicit_command(&state.policy.command_intent, original_user_text) {
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
    let exact_command =
        configured_explicit_command_segment(&state.policy.command_intent, original_user_text);
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

fn normalize_planned_actions_with_original(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
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
            request_has_configured_explicit_command(&state.policy.command_intent, text)
        });
    let defer_legacy_semantic_rewrites = !explicit_command_request
        && route_result.is_some_and(|route| {
            actions_use_ad_hoc_command_without_route_preferred_skill(state, route, &actions)
        });
    if defer_legacy_semantic_rewrites {
        info!("plan_defer_legacy_semantic_rewrite_to_registry_repair");
    }
    let skip_legacy_semantic_rewrites = explicit_command_request || defer_legacy_semantic_rewrites;
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
    let actions =
        strip_terminal_discussion_for_observed_finalize(route_result, loop_state, actions);
    let actions =
        strip_terminal_discussion_for_scalar_path_observation(route_result, loop_state, actions);
    let actions =
        strip_terminal_discussion_for_direct_skill_passthrough(state, route_result, actions);
    let actions = normalize_doc_parse_schema_aliases(actions);
    let actions = normalize_system_basic_schema_aliases(actions);
    let actions = fill_missing_read_range_path_from_route_locator(route_result, actions);
    let actions = rewrite_filtered_list_dir_to_inventory_dir(route_result, actions);
    let actions = normalize_archive_basic_schema_aliases(route_result, actions);
    let actions = strip_file_lines_count_before_tail_read_range(actions);
    let actions = strip_directory_read_range_after_inventory_dir(actions);
    let actions = enforce_output_contract_tool_args(route_result, actions);
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
    let actions =
        rewrite_single_target_file_read_to_auto_locator(route_result, auto_locator_path, actions);
    let actions = replace_workspace_synthesis_respond_only_plan(route_result, loop_state, actions);
    let actions = rewrite_extract_field_alias_args(actions);
    let actions = rewrite_extract_field_paths_to_structured_candidates(
        state,
        route_result,
        auto_locator_path,
        actions,
    );
    let actions = prune_unscoped_workspace_summary_evidence_for_scope(route_result, actions);
    let actions =
        strip_unrequested_workspace_artifact_mutations(state, route_result, loop_state, actions);
    let actions =
        ensure_workspace_synthesis_has_default_text_evidence(route_result, loop_state, actions);
    let actions =
        append_synthesize_for_unscoped_workspace_text_evidence(route_result, loop_state, actions);
    let actions = append_synthesize_answer_for_structured_scalar_compare(route_result, actions);
    let actions =
        rewrite_unresolved_template_arg_multi_file_read_plan(route_result, user_text, actions);
    let actions = strip_unresolved_template_reads_after_inventory_dir(actions);
    let actions =
        strip_workspace_synthesis_without_text_evidence(route_result, loop_state, actions);
    let actions = strip_pre_observation_synthesize_before_concrete_respond(loop_state, actions);
    let actions =
        rewrite_pre_observation_concrete_respond_to_placeholder(route_result, loop_state, actions);
    let actions =
        rewrite_mixed_placeholder_observed_synthesis_respond(route_result, loop_state, actions);
    let actions = rewrite_terminal_synthesis_placeholder_respond(actions);
    let actions = rewrite_terminal_placeholder_respond_to_synthesize_answer(loop_state, actions);
    let actions = inject_synthesize_answer_for_bare_placeholder_respond(actions, user_text);
    let actions = append_synthesize_for_observation_only_terminal_answer(
        state,
        route_result,
        loop_state,
        actions,
    );
    let actions = strip_service_status_discussion_actions(route_result, actions);
    let actions = mark_missing_target_repairable_actions(state, route_result, actions);
    mark_non_mutating_run_cmd_sequences_continue_on_error(state, actions)
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
    let Some(range_value) = obj
        .remove("lines")
        .or_else(|| obj.remove("line_range"))
        .or_else(|| obj.remove("range"))
    else {
        return;
    };
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
            normalize_read_range_negative_bounds(obj);
            normalize_read_range_line_count_template(obj);
        }
        "read_range" => {
            normalize_read_range_line_aliases(obj);
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
        "check_exists" | "exists" | "path_exists" => {
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
        AgentAction::SynthesizeAnswer { .. }
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

fn list_dir_args_need_inventory_dir(
    route_result: Option<&RouteResult>,
    obj: &serde_json::Map<String, Value>,
) -> bool {
    route_result.is_some_and(|route| {
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
        || obj.contains_key("extensions")
}

fn inventory_dir_args_from_list_dir_args(
    route_result: Option<&RouteResult>,
    args: Value,
) -> Option<Value> {
    let mut obj = args.as_object()?.clone();
    if !list_dir_args_need_inventory_dir(route_result, &obj) {
        return None;
    }
    normalize_path_alias_to_path(
        &mut obj,
        &["dir_path", "directory_path", "directory", "dir"],
    );
    obj.insert(
        "action".to_string(),
        Value::String("inventory_dir".to_string()),
    );
    let route_semantic = route_result.map(|route| route.output_contract.semantic_kind);
    let mut dirs_only = route_semantic == Some(crate::OutputSemanticKind::DirectoryNames)
        || bool_arg_any(
            &obj,
            &[
                "dirs_only",
                "directories_only",
                "directory_only",
                "folders_only",
            ],
        )
        || string_arg_any_matches(
            &obj,
            &["kind_filter", "kind", "entry_type"],
            &[
                "dir",
                "dirs",
                "directory",
                "directories",
                "folder",
                "folders",
            ],
        );
    let mut files_only = route_semantic == Some(crate::OutputSemanticKind::FileNames)
        || bool_arg_any(&obj, &["files_only", "file_only"])
        || string_arg_any_matches(
            &obj,
            &["kind_filter", "kind", "entry_type"],
            &["file", "files"],
        );
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
    Some(Value::Object(obj))
}

fn rewrite_filtered_list_dir_to_inventory_dir(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args } if skill.eq_ignore_ascii_case("list_dir") => {
                if let Some(args) =
                    inventory_dir_args_from_list_dir_args(route_result, args.clone())
                {
                    info!("plan_rewrite_list_dir_to_inventory_dir");
                    AgentAction::CallSkill {
                        skill: "system_basic".to_string(),
                        args,
                    }
                } else {
                    AgentAction::CallSkill { skill, args }
                }
            }
            AgentAction::CallTool { tool, args } if tool.eq_ignore_ascii_case("list_dir") => {
                if let Some(args) =
                    inventory_dir_args_from_list_dir_args(route_result, args.clone())
                {
                    info!("plan_rewrite_list_dir_to_inventory_dir");
                    AgentAction::CallTool {
                        tool: "system_basic".to_string(),
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

fn normalize_archive_basic_args(mut args: Value, route_result: Option<&RouteResult>) -> Value {
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
                    "archive_path",
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
        _ => {}
    }
    args
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
                    if rewrite_inventory_ext_filter_action_to_fs_search(route, skill, args) {
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
                        enforce_directory_names_inventory_args(route, obj);
                        enforce_general_directory_inventory_args(route, obj);
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
                _ => {}
            }
            action
        })
        .collect()
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

fn enforce_directory_names_inventory_args(
    route: &RouteResult,
    obj: &mut serde_json::Map<String, Value>,
) {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::DirectoryNames {
        return;
    }
    obj.insert("files_only".to_string(), Value::Bool(false));
    obj.insert("dirs_only".to_string(), Value::Bool(true));
    obj.insert("names_only".to_string(), Value::Bool(true));
    info!("plan_contract_enforce_directory_names_inventory");
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

fn should_rewrite_inventory_ext_filter_to_fs_search(route: &RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::FilePaths
        && matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::CurrentWorkspace | crate::OutputLocatorKind::Path
        )
        && !route.output_contract.delivery_required
}

fn rewrite_inventory_ext_filter_action_to_fs_search(
    route: &RouteResult,
    skill: &mut String,
    args: &mut Value,
) -> bool {
    if !should_rewrite_inventory_ext_filter_to_fs_search(route) {
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
    if action_name != "inventory_dir" {
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
    *skill = "fs_search".to_string();
    *args = serde_json::json!({
        "action": "find_ext",
        "root": root,
        "ext": ext,
        "max_results": max_results
    });
    info!("plan_contract_rewrite_inventory_ext_filter_to_fs_search");
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
    if !obj.contains_key("pattern") {
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
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::FilePaths {
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
        AgentAction::CallSkill { skill, args } if skill == "system_basic" => args
            .get("action")
            .and_then(|value| value.as_str())
            .is_some_and(|action| {
                matches!(
                    action.trim().to_ascii_lowercase().as_str(),
                    "inventory_dir" | "read_range" | "workspace_glance" | "tree_summary"
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
                _ => false,
            }
        }
        AgentAction::SynthesizeAnswer { .. } => false,
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
            if skill.eq_ignore_ascii_case("system_basic") =>
        {
            args.get("action")
                .and_then(|value| value.as_str())
                .is_some_and(|action| action.trim().eq_ignore_ascii_case("read_range"))
        }
        AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::CallSkill { .. }
        | AgentAction::CallTool { .. } => false,
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
        return false;
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
        && !has_listing_grounded_synthesis_answer_plan(route, actions)
        && !has_workspace_text_content_evidence(loop_state, actions)
        && !has_compact_structured_observation_answer_plan(actions)
        && !has_mixed_last_output_terminal_respond(actions)
        && !has_run_cmd_observation_action(actions)
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
        rewritten.push(AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "read_range",
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
    if !skill.eq_ignore_ascii_case("system_basic") {
        return false;
    }
    args.get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|action_name| {
            matches!(
                action_name.to_ascii_lowercase().as_str(),
                "count_inventory" | "compare_paths" | "path_batch_facts" | "extract_fields"
            )
        })
}

fn action_workspace_summary_path(action: &AgentAction) -> Option<&str> {
    match action {
        AgentAction::CallSkill { skill, args } if skill == "list_dir" || skill == "read_file" => {
            args.get("path").and_then(|value| value.as_str())
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
    !route.needs_clarify
        && !route.output_contract.delivery_required
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::QuantityComparison
                | crate::OutputSemanticKind::RecentScalarEqualityCheck
        )
        && matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
                | crate::OutputResponseShape::OneSentence
                | crate::OutputResponseShape::Strict
        )
}

fn action_scalar_compare_observation_units(action: &AgentAction) -> usize {
    match action {
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
        _ => 0,
    }
}

fn structured_scalar_observation_units(actions: &[AgentAction]) -> usize {
    actions
        .iter()
        .map(action_scalar_compare_observation_units)
        .sum()
}

fn structured_scalar_compare_missing_required_extracts(
    route: &RouteResult,
    actions: &[AgentAction],
) -> bool {
    route_requests_structured_scalar_compare(route)
        && has_executable_observation_or_action(actions)
        && structured_scalar_observation_units(actions) < 2
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
    let evidence_refs = actions
        .iter()
        .enumerate()
        .filter_map(|(idx, action)| {
            (action_scalar_compare_observation_units(action) > 0)
                .then(|| format!("step_{}", idx + 1))
        })
        .collect::<Vec<_>>();
    if structured_scalar_observation_units(&actions) < 2 {
        return actions;
    }
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
    let file_targets = explicit_document_file_targets(user_text);
    if file_targets.len() < 2 {
        return actions;
    }

    let mut rewritten = Vec::new();
    for target in file_targets.iter().take(4) {
        rewritten.push(AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "read_range",
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
    is_system_basic_inventory_dir_action(action) || is_fs_search_observation_action(action)
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
    let AgentAction::CallSkill { skill, args } = action else {
        return false;
    };
    if !value_contains_unresolved_template(args) {
        return false;
    }
    if skill == "read_file" {
        return true;
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
    let AgentAction::CallSkill { skill, args } = action else {
        return false;
    };
    if skill != "system_basic" {
        return false;
    }
    let Some(obj) = args.as_object() else {
        return false;
    };
    let is_read_range = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|action_name| action_name.eq_ignore_ascii_case("read_range"));
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
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => {
            value_contains_unresolved_template(args)
        }
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
    for candidate in crate::delivery_utils::extract_filename_candidates(user_text) {
        if !filename_candidate_has_document_extension(&candidate)
            || targets
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(&candidate))
        {
            continue;
        }
        targets.push(candidate);
    }
    targets
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

fn prune_unscoped_workspace_summary_evidence_for_scope(
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
        let AgentAction::CallSkill { skill, args } = action else {
            continue;
        };
        if !skill.eq_ignore_ascii_case("system_basic") {
            continue;
        }
        let Some(request) = structured_extract_request(args) else {
            continue;
        };
        let current = resolve_workspace_path(&state.skill_rt.workspace_root, &request.path);
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
        action if action.eq_ignore_ascii_case("extract_field") => obj
            .get("field_path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| vec![value.to_string()])
            .unwrap_or_default(),
        action if action.eq_ignore_ascii_case("extract_fields") => {
            string_list_from_value(obj.get("field_paths"))
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

fn action_should_be_sqlite_table_query(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let skill = skill.trim().to_ascii_lowercase();
            if skill == "db_basic" {
                return false;
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
        AgentAction::Think { .. }
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
    let Some(route) = route_result else {
        return actions;
    };
    if !route_requests_sqlite_table_listing(route) {
        return actions;
    }
    let Some(db_path) = sqlite_locator_path_for_route(route, auto_locator_path) else {
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
        AgentAction::Think { .. }
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
        AgentAction::Think { .. }
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
        AgentAction::Think { .. }
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
    let mut rewritten = actions;
    let mut changed = false;
    for action in rewritten.iter_mut() {
        let should_rewrite = matches!(
            action,
            AgentAction::CallSkill { skill, .. } | AgentAction::CallTool { tool: skill, .. }
                if skill.trim().eq_ignore_ascii_case("run_cmd")
        );
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
    }
    if changed {
        info!("plan_rewrite_archive_unpack_run_cmd_to_archive_basic");
    }
    rewritten
}

fn archive_pack_pair_for_route(route: &RouteResult) -> Option<(String, String)> {
    if !route.is_execute_gate() || !route.output_contract.requires_content_evidence {
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

fn action_is_system_basic_path_batch_facts_for_pair(
    action: &AgentAction,
    source: &str,
    archive: &str,
) -> bool {
    if !planned_action_skill_name(action)
        .map(str::trim)
        .is_some_and(|skill| {
            skill.eq_ignore_ascii_case("system_basic") || skill.eq_ignore_ascii_case("read_file")
        })
    {
        return false;
    }
    let Some(args) = action_args(action).and_then(Value::as_object) else {
        return false;
    };
    if args.get("action").and_then(Value::as_str) != Some("path_batch_facts") {
        return false;
    }
    let paths = string_list_from_value(args.get("paths").or_else(|| args.get("path")));
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
    if actions.iter().any(action_is_archive_basic) {
        return actions;
    }

    let mut rewritten = actions;
    let mut changed = false;
    for action in rewritten.iter_mut() {
        if action_is_system_basic_path_batch_facts_for_pair(action, &source, &archive)
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
/// （MiniMax 在 short-answer 类 act 任务里会反复踩这个坑，prompt 指令忠实度不够）。
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

/// Planner-first shape guard: `synthesize_answer` is only meaningful after an
/// observation exists. If a first-round plan puts synthesis before any
/// tool/skill output but also provides a concrete final `respond`, keep the
/// concrete response and drop the impossible pre-observation synthesis.
///
/// This is not a natural-language shortcut; it only repairs the plan graph so
/// creative/chat-like deliverables do not fail by trying to summarize missing
/// evidence before returning an already concrete answer.
fn strip_pre_observation_synthesize_before_concrete_respond(
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if actions.len() < 2 || has_loop_observation(loop_state) {
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
/// 这一招专门针对 minimax 偶发的「planner 一次性把 list_dir + respond 编造
/// 内容写在同一个 plan，respond 直接交给用户」的 adversarial v1 → adv08 复现路径。
/// 不命中条件时 actions 原样返回，不破坏正确 plan。
fn rewrite_pre_observation_concrete_respond_to_placeholder(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
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
        && request_has_configured_explicit_command(
            &state.policy.command_intent,
            &original_user_text_for_policy,
        );
    if !explicit_command_request {
        if let Some(plan_result) = scalar_path_auto_locator_fast_plan_result(
            goal,
            route_result,
            loop_state,
            auto_locator_path,
        ) {
            info!(
                "plan_fast_path_scalar_path_auto_locator task_id={} round={}",
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
    let turn_analysis = build_turn_analysis_prompt_block(turn_analysis_for_prompt);
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_text);
    let user_request_for_prompt =
        crate::language_policy::task_user_request_for_prompt(task, user_text);
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
        // Phase 3.3 / minimax-fs_search regression fix:
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
        .map(|actions| {
            normalize_planned_actions_with_original(
                state,
                route_result,
                loop_state,
                user_text,
                Some(&original_user_text_for_policy),
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
                            normalize_planned_actions_with_original(
                                state,
                                route_result,
                                loop_state,
                                user_text,
                                Some(&original_user_text_for_policy),
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
                            &repaired,
                            loop_state.round_no,
                        )
                        .await?;
                        let second_repaired_actions =
                            parse_single_plan_actions(&second_repaired, state, task)
                                .await
                                .map(|actions| {
                                    normalize_planned_actions_with_original(
                                        state,
                                        route_result,
                                        loop_state,
                                        user_text,
                                        Some(&original_user_text_for_policy),
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
                                return Err("repair plan still non-actionable after second repair"
                                    .to_string());
                            }
                            None => {
                                return Err(
                                    "second repair plan parser failed: no executable steps"
                                        .to_string(),
                                );
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
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, RwLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    use claw_core::config::{AgentConfig, ToolsConfig};
    use claw_core::skill_registry::SkillsRegistry;

    use super::{
        build_lightweight_skill_playbooks_text, build_lightweight_skill_quick_index_text,
        build_lightweight_tool_spec, can_fallback_to_initial_plan_after_repair_failure,
        classify_planning_prompt_class, enforce_output_contract_tool_args,
        fill_missing_read_range_path_from_route_locator,
        has_pre_observation_structured_output_shape,
        inject_synthesize_answer_for_bare_placeholder_respond, is_bare_last_output_placeholder,
        normalize_archive_basic_schema_aliases, normalize_planned_actions,
        normalize_planned_actions_with_original, normalize_system_basic_schema_aliases,
        plan_repair_reason, replace_file_delivery_respond_only_with_path_observation,
        replace_scalar_count_plan_with_count_inventory,
        replace_scalar_path_respond_only_with_auto_locator_observation,
        rewrite_archive_pack_plan_to_archive_basic,
        rewrite_archive_unpack_run_cmd_to_archive_basic,
        rewrite_docker_readonly_run_cmd_to_docker_basic, rewrite_extract_field_alias_args,
        rewrite_pre_observation_concrete_respond_to_placeholder,
        rewrite_service_status_plan_to_service_control,
        rewrite_sqlite_schema_version_plan_to_db_basic,
        rewrite_sqlite_table_listing_plan_to_db_basic,
        rewrite_terminal_placeholder_respond_to_synthesize_answer,
        rewrite_terminal_synthesis_placeholder_respond,
        rewrite_unresolved_template_arg_multi_file_read_plan, round1_prompt_spec_for_class,
        scalar_path_auto_locator_fast_plan_result, scalar_path_auto_locator_observation_plan,
        should_force_actionable_plan_repair, strip_directory_read_range_after_inventory_dir,
        strip_file_lines_count_before_tail_read_range,
        strip_terminal_discussion_for_direct_skill_passthrough,
        strip_terminal_discussion_for_observed_finalize,
        strip_terminal_discussion_for_scalar_path_observation,
        strip_unresolved_template_reads_after_inventory_dir, LoopState, PlanningPromptClass,
    };
    use crate::{
        AgentAction, AgentRuntimeConfig, AppState, ClaimedTask, IntentOutputContract,
        OutputLocatorKind, OutputResponseShape, OutputSemanticKind, PlanKind, ResumeBehavior,
        RiskCeiling, RouteResult, RoutedMode, ScheduleKind, SkillViewsSnapshot, ToolsPolicy,
        DEFAULT_AGENT_ID,
    };
    use serde_json::{json, Value};

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(prefix: &str) -> Self {
            let mut path = std::env::temp_dir();
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time before unix epoch")
                .as_nanos();
            path.push(format!(
                "clawd_planning_{prefix}_{}_{}",
                std::process::id(),
                nanos
            ));
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn test_state() -> AppState {
        let agents_by_id = HashMap::from([(
            DEFAULT_AGENT_ID.to_string(),
            AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
        )]);
        AppState {
            core: crate::CoreServices {
                agents_by_id: Arc::new(agents_by_id),
                skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                    registry: None,
                    skills_list: Arc::new(HashSet::new()),
                }))),
                ..crate::CoreServices::test_default()
            },
            skill_rt: crate::SkillRuntime {
                locator_scan_max_files: 200,
                tools_policy: Arc::new(
                    ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
                ),
                ..crate::SkillRuntime::test_default()
            },
            policy: crate::PolicyConfig::test_default(),
            worker: crate::WorkerConfig::test_default(),
            metrics: crate::TaskMetricsRegistry::default(),
            channels: crate::ChannelConfig::default(),
            reload_ctx: crate::ReloadContext::default(),
            ask_states: crate::AskStateRegistry::default(),
        }
    }

    fn test_state_with_enabled_skills(skills: &[&str]) -> AppState {
        let state = test_state();
        let enabled: HashSet<String> = skills.iter().map(|skill| (*skill).to_string()).collect();
        *state
            .core
            .skill_views_snapshot
            .write()
            .expect("skill snapshot lock") = Arc::new(SkillViewsSnapshot {
            registry: None,
            skills_list: Arc::new(enabled),
        });
        state
    }

    fn test_state_with_registry() -> AppState {
        let state = test_state();
        let registry_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/skills_registry.toml");
        let registry = SkillsRegistry::load_from_path(&registry_path).expect("load registry");
        *state
            .core
            .skill_views_snapshot
            .write()
            .expect("skill snapshot lock") = Arc::new(SkillViewsSnapshot {
            registry: Some(Arc::new(registry)),
            skills_list: Arc::new(HashSet::new()),
        });
        state
    }

    fn test_task() -> ClaimedTask {
        ClaimedTask {
            task_id: "test-task".to_string(),
            user_id: 1,
            chat_id: 2,
            user_key: None,
            channel: "test".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        }
    }

    fn base_route_result() -> RouteResult {
        RouteResult {
            routed_mode: RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(RoutedMode::Act),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Low,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract::default(),
        }
    }

    #[test]
    fn sqlite_table_listing_route_rewrites_text_read_plan_to_db_basic_list_tables() {
        let mut route = base_route_result();
        route.output_contract.response_shape = OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableListing;
        route.output_contract.locator_hint = "/tmp/app.sqlite".to_string();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "read_range",
                    "path": "/tmp/app.sqlite",
                    "command": "sqlite3 /tmp/app.sqlite \"SELECT name FROM sqlite_master WHERE type='table';\""
                }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        let rewritten = rewrite_sqlite_table_listing_plan_to_db_basic(
            Some(&route),
            Some("/tmp/app.sqlite"),
            false,
            actions,
        );

        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "db_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("list_tables")
                );
                assert_eq!(
                    args.get("db_path").and_then(|value| value.as_str()),
                    Some("/tmp/app.sqlite")
                );
                assert!(args.get("sql").is_none());
            }
            other => panic!("expected db_basic action, got {other:?}"),
        }
        assert!(matches!(rewritten[1], AgentAction::SynthesizeAnswer { .. }));
        assert!(matches!(rewritten[2], AgentAction::Respond { .. }));
    }

    #[test]
    fn existence_path_summary_plan_inserts_bounded_content_observation() {
        let state = test_state();
        let loop_state = LoopState::new(1);
        let mut route = base_route_result();
        route.output_contract.response_shape = OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = OutputLocatorKind::Filename;
        route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPathSummary;
        route.output_contract.locator_hint = "rustclaw.service".to_string();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "path_batch_facts",
                    "paths": ["/tmp/rustclaw.service"],
                    "include_missing": true
                }),
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        let normalized = normalize_planned_actions(
            &state,
            Some(&route),
            &loop_state,
            "check service file and summarize its purpose",
            Some("/tmp/rustclaw.service"),
            actions,
        );

        assert!(normalized.iter().any(|action| {
            matches!(
                action,
                AgentAction::CallSkill { skill, args }
                    if skill == "system_basic"
                        && args.get("action").and_then(Value::as_str) == Some("read_range")
                        && args.get("path").and_then(Value::as_str) == Some("/tmp/rustclaw.service")
            )
        }));
        assert!(normalized.iter().any(|action| {
            matches!(
                action,
                AgentAction::SynthesizeAnswer { evidence_refs }
                    if evidence_refs == &vec!["step_1".to_string(), "step_2".to_string()]
            )
        }));
        assert!(matches!(
            normalized.last(),
            Some(AgentAction::Respond { content }) if content == "{{last_output}}"
        ));
    }

    #[test]
    fn sqlite_table_names_route_rewrites_system_basic_action_alias_to_db_basic_list_tables() {
        let mut route = base_route_result();
        route.output_contract.response_shape = OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableNamesOnly;
        route.output_contract.locator_hint = "/tmp/app.sqlite".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "sqlite_table_names",
                "path": "/tmp/app.sqlite"
            }),
        }];

        let rewritten =
            rewrite_sqlite_table_listing_plan_to_db_basic(Some(&route), None, false, actions);

        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "db_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("list_tables")
                );
                assert_eq!(
                    args.get("db_path").and_then(|value| value.as_str()),
                    Some("/tmp/app.sqlite")
                );
            }
            other => panic!("expected db_basic action, got {other:?}"),
        }
    }

    #[test]
    fn sqlite_table_listing_route_rewrites_text_field_extract_to_db_basic_list_tables() {
        let mut route = base_route_result();
        route.output_contract.response_shape = OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableListing;
        route.output_contract.locator_hint = "/tmp/app.sqlite".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "extract_field",
                "path": "/tmp/app.sqlite",
                "field_path": "sqlite_master.name"
            }),
        }];

        let rewritten =
            rewrite_sqlite_table_listing_plan_to_db_basic(Some(&route), None, false, actions);

        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "db_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("list_tables")
                );
                assert_eq!(
                    args.get("db_path").and_then(|value| value.as_str()),
                    Some("/tmp/app.sqlite")
                );
            }
            other => panic!("expected db_basic action, got {other:?}"),
        }
    }

    #[test]
    fn sqlite_database_kind_judgment_rewrites_run_cmd_to_db_basic_list_tables() {
        let mut route = base_route_result();
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.semantic_kind = OutputSemanticKind::SqliteDatabaseKindJudgment;
        route.output_contract.locator_hint = "/tmp/db-basic-contract.sqlite".to_string();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "run_cmd",
                    "command": "sqlite3 /tmp/db-basic-contract.sqlite \".tables\""
                }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
        ];

        let rewritten =
            rewrite_sqlite_table_listing_plan_to_db_basic(Some(&route), None, false, actions);

        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "db_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("list_tables")
                );
                assert_eq!(
                    args.get("db_path").and_then(|value| value.as_str()),
                    Some("/tmp/db-basic-contract.sqlite")
                );
                assert!(args.get("sql").is_none());
            }
            other => panic!("expected db_basic action, got {other:?}"),
        }
        assert!(matches!(rewritten[1], AgentAction::SynthesizeAnswer { .. }));
    }

    #[test]
    fn sqlite_table_listing_preserves_explicit_literal_run_cmd() {
        let mut route = base_route_result();
        route.output_contract.response_shape = OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableListing;
        route.output_contract.locator_hint = "/tmp/app.sqlite".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "sqlite3 /tmp/app.sqlite '.tables'"}),
        }];

        let rewritten =
            rewrite_sqlite_table_listing_plan_to_db_basic(Some(&route), None, true, actions);

        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "run_cmd");
                assert_eq!(
                    args.get("command").and_then(Value::as_str),
                    Some("sqlite3 /tmp/app.sqlite '.tables'")
                );
            }
            other => panic!("expected run_cmd action, got {other:?}"),
        }
    }

    #[test]
    fn sqlite_schema_version_extract_field_rewrites_to_db_basic_pragma() {
        let mut route = base_route_result();
        route.output_contract.response_shape = OutputResponseShape::Scalar;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "/tmp/app.sqlite".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "extract_field",
                "path": "/tmp/app.sqlite",
                "field_path": "schema_version"
            }),
        }];

        let rewritten =
            rewrite_sqlite_schema_version_plan_to_db_basic(Some(&route), None, false, actions);

        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "db_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("schema_version")
                );
                assert_eq!(
                    args.get("db_path").and_then(|value| value.as_str()),
                    Some("/tmp/app.sqlite")
                );
                assert!(args.get("sql").is_none());
            }
            other => panic!("expected db_basic action, got {other:?}"),
        }
    }

    #[test]
    fn sqlite_schema_version_extract_fields_rewrites_from_action_path() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "extract_fields",
                "path": "/tmp/app.db",
                "field_paths": ["schema_version"]
            }),
        }];

        let rewritten = rewrite_sqlite_schema_version_plan_to_db_basic(None, None, false, actions);

        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "db_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("schema_version")
                );
                assert_eq!(
                    args.get("db_path").and_then(|value| value.as_str()),
                    Some("/tmp/app.db")
                );
                assert!(args.get("sql").is_none());
            }
            other => panic!("expected db_basic action, got {other:?}"),
        }
    }

    #[test]
    fn sqlite_schema_version_route_rewrites_binary_text_read_to_db_basic_pragma() {
        let mut route = base_route_result();
        route.output_contract.response_shape = OutputResponseShape::Scalar;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.semantic_kind = OutputSemanticKind::SqliteSchemaVersion;
        route.output_contract.locator_hint = "/tmp/app.sqlite".to_string();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "read_range",
                    "path": "/tmp/app.sqlite",
                    "mode": "head",
                    "n": 100
                }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
        ];

        let rewritten =
            rewrite_sqlite_schema_version_plan_to_db_basic(Some(&route), None, false, actions);

        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "db_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("schema_version")
                );
                assert_eq!(
                    args.get("db_path").and_then(|value| value.as_str()),
                    Some("/tmp/app.sqlite")
                );
                assert!(args.get("sql").is_none());
            }
            other => panic!("expected db_basic action, got {other:?}"),
        }
        assert!(matches!(rewritten[1], AgentAction::SynthesizeAnswer { .. }));
    }

    #[test]
    fn file_delivery_respond_only_gets_path_observation_before_file_token() {
        let tmp = TempDirGuard::new("file_delivery_observation");
        let file_path = tmp.path.join("service_notes.md");
        fs::write(&file_path, "notes\n").expect("write file");
        let state = test_state();
        let mut route = base_route_result();
        route.wants_file_delivery = true;
        route.output_contract.response_shape = OutputResponseShape::FileToken;
        route.output_contract.delivery_required = true;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = file_path.display().to_string();
        let token = format!("FILE:{}", file_path.display());
        let actions = vec![AgentAction::Respond { content: token }];

        let rewritten = replace_file_delivery_respond_only_with_path_observation(
            &state,
            Some(&route),
            &LoopState::default(),
            actions,
        );

        assert_eq!(rewritten.len(), 2);
        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("path_batch_facts")
                );
            }
            other => panic!("expected path observation, got {other:?}"),
        }
        assert!(matches!(rewritten[1], AgentAction::Respond { .. }));
    }

    #[test]
    fn archive_unpack_route_rewrites_run_cmd_unzip_to_archive_basic() {
        let mut route = base_route_result();
        route.output_contract.response_shape = OutputResponseShape::Free;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "/tmp/bundle.zip | /tmp/out".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({
                "command": "unzip \"/tmp/bundle.zip\" -d \"/tmp/out\""
            }),
        }];

        let rewritten =
            rewrite_archive_unpack_run_cmd_to_archive_basic(Some(&route), false, actions);

        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "archive_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("unpack")
                );
                assert_eq!(
                    args.get("archive").and_then(|value| value.as_str()),
                    Some("/tmp/bundle.zip")
                );
                assert_eq!(
                    args.get("dest").and_then(|value| value.as_str()),
                    Some("/tmp/out")
                );
            }
            other => panic!("expected archive_basic action, got {other:?}"),
        }
    }

    #[test]
    fn archive_unpack_preserves_explicit_literal_run_cmd() {
        let mut route = base_route_result();
        route.output_contract.response_shape = OutputResponseShape::Free;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "/tmp/input.zip | /tmp/out".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "unzip /tmp/input.zip -d /tmp/out"}),
        }];

        let rewritten =
            rewrite_archive_unpack_run_cmd_to_archive_basic(Some(&route), true, actions);

        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "run_cmd");
                assert_eq!(
                    args.get("command").and_then(Value::as_str),
                    Some("unzip /tmp/input.zip -d /tmp/out")
                );
            }
            other => panic!("expected run_cmd action, got {other:?}"),
        }
    }

    #[test]
    fn archive_pack_route_rewrites_probe_only_plan_to_archive_basic() {
        let mut route = base_route_result();
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint =
            "scripts/skill_calls | tmp/nl_archive_case_en.zip".to_string();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "path_batch_facts",
                    "paths": [
                        "/home/guagua/rustclaw/scripts/skill_calls",
                        "/home/guagua/rustclaw/tmp/nl_archive_case_en.zip"
                    ]
                }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
            AgentAction::Respond {
                content: "Unable to create the zip archive.".to_string(),
            },
        ];

        let rewritten = rewrite_archive_pack_plan_to_archive_basic(Some(&route), false, actions);

        assert_eq!(rewritten.len(), 2);
        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "archive_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("pack")
                );
                assert_eq!(
                    args.get("source").and_then(|value| value.as_str()),
                    Some("scripts/skill_calls")
                );
                assert_eq!(
                    args.get("archive").and_then(|value| value.as_str()),
                    Some("tmp/nl_archive_case_en.zip")
                );
                assert_eq!(
                    args.get("format").and_then(|value| value.as_str()),
                    Some("zip")
                );
            }
            other => panic!("expected archive_basic action, got {other:?}"),
        }
        assert!(matches!(rewritten[1], AgentAction::SynthesizeAnswer { .. }));
    }

    #[test]
    fn archive_pack_preserves_explicit_literal_run_cmd() {
        let mut route = base_route_result();
        route.output_contract.response_shape = OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "/tmp/source | /tmp/source.tgz".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "tar -czf /tmp/source.tgz /tmp/source"}),
        }];

        let rewritten = rewrite_archive_pack_plan_to_archive_basic(Some(&route), true, actions);

        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "run_cmd");
                assert_eq!(
                    args.get("command").and_then(Value::as_str),
                    Some("tar -czf /tmp/source.tgz /tmp/source")
                );
            }
            other => panic!("expected run_cmd action, got {other:?}"),
        }
    }

    #[test]
    fn archive_basic_pack_alias_args_normalize_to_contract() {
        let mut route = base_route_result();
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint =
            "scripts/skill_calls -> tmp/nl_archive_case_en.zip".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "archive_basic".to_string(),
            args: json!({
                "action": "pack",
                "source_path": "/home/guagua/rustclaw/scripts/skill_calls",
                "archive_path": "/home/guagua/rustclaw/tmp/nl_archive_case_en.zip",
            }),
        }];

        let normalized = normalize_archive_basic_schema_aliases(Some(&route), actions);

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "archive_basic");
                assert_eq!(
                    args.get("source").and_then(Value::as_str),
                    Some("/home/guagua/rustclaw/scripts/skill_calls")
                );
                assert_eq!(
                    args.get("archive").and_then(Value::as_str),
                    Some("/home/guagua/rustclaw/tmp/nl_archive_case_en.zip")
                );
                assert_eq!(args.get("format").and_then(Value::as_str), Some("zip"));
                assert!(args.get("source_path").is_none());
                assert!(args.get("archive_path").is_none());
            }
            other => panic!("expected archive_basic action, got {other:?}"),
        }
    }

    #[test]
    fn archive_basic_list_path_alias_normalizes_to_archive_contract() {
        let actions = vec![AgentAction::CallSkill {
            skill: "archive_basic".to_string(),
            args: json!({
                "action": "list",
                "path": "/tmp/rustclaw_archive_nl_case/sample.tgz",
            }),
        }];

        let normalized = normalize_archive_basic_schema_aliases(None, actions);

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "archive_basic");
                assert_eq!(
                    args.get("archive").and_then(Value::as_str),
                    Some("/tmp/rustclaw_archive_nl_case/sample.tgz")
                );
                assert!(args.get("path").is_none());
            }
            other => panic!("expected archive_basic action, got {other:?}"),
        }
    }

    #[test]
    fn docker_ps_run_cmd_rewrites_to_docker_basic() {
        let state = test_state_with_enabled_skills(&["docker_basic", "run_cmd"]);
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "docker ps -a"}),
        }];

        let rewritten = rewrite_docker_readonly_run_cmd_to_docker_basic(&state, false, actions);

        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "docker_basic");
                assert_eq!(args.get("action").and_then(Value::as_str), Some("ps"));
            }
            other => panic!("expected docker_basic action, got {other:?}"),
        }
    }

    #[test]
    fn docker_image_ls_run_cmd_rewrites_to_docker_basic_images() {
        let state = test_state_with_enabled_skills(&["docker_basic", "run_cmd"]);
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "docker image ls"}),
        }];

        let rewritten = rewrite_docker_readonly_run_cmd_to_docker_basic(&state, false, actions);

        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "docker_basic");
                assert_eq!(args.get("action").and_then(Value::as_str), Some("images"));
            }
            other => panic!("expected docker_basic action, got {other:?}"),
        }
    }

    #[test]
    fn docker_readonly_preserves_explicit_literal_run_cmd() {
        let state = test_state_with_enabled_skills(&["docker_basic", "run_cmd"]);
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "docker ps"}),
        }];

        let rewritten = rewrite_docker_readonly_run_cmd_to_docker_basic(&state, true, actions);

        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "run_cmd");
                assert_eq!(
                    args.get("command").and_then(Value::as_str),
                    Some("docker ps")
                );
            }
            other => panic!("expected run_cmd action, got {other:?}"),
        }
    }

    #[test]
    fn doc_parse_unsupported_transform_action_normalizes_to_parse_doc() {
        let state = test_state_with_enabled_skills(&["doc_parse"]);
        let actions = vec![AgentAction::CallSkill {
            skill: "doc_parse".to_string(),
            args: json!({
                "action": "summarize",
                "file_path": "/home/guagua/rustclaw/README.md",
                "max_chars": 8000
            }),
        }];

        let normalized = normalize_planned_actions(
            &state,
            Some(&base_route_result()),
            &LoopState::default(),
            "Summarize README.md",
            None,
            actions,
        );

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "doc_parse");
                assert_eq!(
                    args.get("action").and_then(Value::as_str),
                    Some("parse_doc")
                );
                assert_eq!(
                    args.get("path").and_then(Value::as_str),
                    Some("/home/guagua/rustclaw/README.md")
                );
            }
            other => panic!("expected doc_parse action, got {other:?}"),
        }
    }

    #[test]
    fn lightweight_prompt_mentions_archive_basic_for_archive_contracts() {
        let state = test_state_with_enabled_skills(&[
            "archive_basic",
            "docker_basic",
            "config_guard",
            "doc_parse",
            "transform",
            "browser_web",
        ])
        .with_prompt_layers_installed();
        let task = test_task();
        let quick_index = build_lightweight_skill_quick_index_text(&state, &task);
        let playbooks = build_lightweight_skill_playbooks_text(&state, &task);
        assert!(quick_index.contains("archive_basic"));
        assert!(playbooks.contains("archive_basic"));
        assert!(playbooks.contains("`pack`") || playbooks.contains("packing"));
        assert!(quick_index.contains("docker_basic"));
        assert!(playbooks.contains("docker_basic"));
        assert!(quick_index.contains("config_guard"));
        assert!(playbooks.contains("config_guard"));
        assert!(quick_index.contains("doc_parse"));
        assert!(playbooks.contains("doc_parse"));
        assert!(quick_index.contains("transform"));
        assert!(playbooks.contains("transform"));
        assert!(quick_index.contains("browser_web"));
        assert!(playbooks.contains("browser_web"));
    }

    #[test]
    fn lightweight_prompt_includes_registry_planner_metadata() {
        let state = test_state_with_registry();
        let registry = state.get_skills_registry().expect("registry loaded");
        *state
            .core
            .skill_views_snapshot
            .write()
            .expect("skill snapshot lock") = Arc::new(SkillViewsSnapshot {
            registry: Some(registry),
            skills_list: Arc::new(HashSet::from([
                "archive_basic".to_string(),
                "service_control".to_string(),
            ])),
        });
        let state = state.with_prompt_layers_installed();
        let task = test_task();
        let quick_index = build_lightweight_skill_quick_index_text(&state, &task);
        let playbooks = build_lightweight_skill_playbooks_text(&state, &task);
        assert!(quick_index.contains("archive_basic"));
        assert!(quick_index.contains("semantic_tags: archive_list"));
        assert!(quick_index.contains("preferred_over_run_cmd: true"));
        assert!(quick_index.contains("validation_actions: list"));
        assert!(playbooks.contains("### archive_basic"));
        assert!(playbooks.contains("Registry metadata: semantic_tags: archive_list"));
        assert!(playbooks.contains("preferred_over_run_cmd: true"));
        assert!(playbooks.contains("validation_actions: list"));
        assert!(playbooks.contains("### service_control"));
        assert!(playbooks.contains("semantic_tags: service_status"));
    }

    #[test]
    fn planning_prompt_class_uses_lightweight_execution_for_scalar_contract() {
        let mut route = base_route_result();
        route.route_reason = "llm_contract:generic_filename_scalar_extract".to_string();
        route.resolved_intent = "读取 UI/package.json 里的 name 字段，只输出值".to_string();
        route.output_contract.response_shape = OutputResponseShape::Scalar;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = OutputLocatorKind::Filename;
        route.output_contract.locator_hint = "package.json".to_string();
        assert_eq!(
            classify_planning_prompt_class(
                Some(&route),
                &route.resolved_intent,
                &LoopState::default()
            )
            .as_str(),
            "lightweight_execution"
        );
    }

    #[test]
    fn planning_prompt_class_uses_lightweight_execution_for_generic_scalar_path_read() {
        let mut route = base_route_result();
        route.resolved_intent =
            "读取 /home/guagua/rustclaw/configs/config.toml 中的 tools.allow_sudo 配置项的值，并仅输出该值"
                .to_string();
        route.output_contract.response_shape = OutputResponseShape::Scalar;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint =
            "/home/guagua/rustclaw/configs/config.toml".to_string();
        assert_eq!(
            classify_planning_prompt_class(
                Some(&route),
                &route.resolved_intent,
                &LoopState::default()
            )
            .as_str(),
            "lightweight_execution"
        );
    }

    #[test]
    fn planning_prompt_class_uses_lightweight_execution_for_pwd_only_route() {
        let mut route = base_route_result();
        route.route_reason = "llm_contract:scalar_path_only".to_string();
        route.resolved_intent = "只输出当前工作目录的绝对路径，不要解释".to_string();
        route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
        route.output_contract.response_shape = OutputResponseShape::Scalar;
        assert_eq!(
            classify_planning_prompt_class(
                Some(&route),
                &route.resolved_intent,
                &LoopState::default()
            )
            .as_str(),
            "lightweight_execution"
        );
    }

    #[test]
    fn planning_prompt_class_uses_lightweight_execution_for_content_evidence_reads() {
        let mut route = base_route_result();
        route.routed_mode = RoutedMode::ChatAct;
        route.ask_mode = crate::AskMode::from_routed_mode(RoutedMode::ChatAct);
        route.route_reason = "llm_contract:generic_filename_read_range".to_string();
        route.resolved_intent = "先读一下 README.md 前 4 行".to_string();
        route.output_contract.response_shape = OutputResponseShape::Free;
        route.output_contract.requires_content_evidence = true;
        assert_eq!(
            classify_planning_prompt_class(
                Some(&route),
                &route.resolved_intent,
                &LoopState::default()
            )
            .as_str(),
            "lightweight_execution"
        );
    }

    #[test]
    fn planning_prompt_class_keeps_open_planning_for_chat_act_or_later_rounds() {
        let mut route = base_route_result();
        route.ask_mode = crate::AskMode::from_routed_mode(RoutedMode::ChatAct);
        route.resolved_intent = "比较这两个文件大小，然后一句话总结".to_string();
        assert_eq!(
            classify_planning_prompt_class(
                Some(&route),
                &route.resolved_intent,
                &LoopState::default()
            )
            .as_str(),
            "open_planning"
        );

        let mut scalar = base_route_result();
        scalar.route_reason = "llm_contract:generic_filename_scalar_extract".to_string();
        scalar.resolved_intent = "读取 UI/package.json 里的 name 字段，只输出值".to_string();
        scalar.output_contract.response_shape = OutputResponseShape::Scalar;
        scalar.output_contract.requires_content_evidence = true;
        scalar.output_contract.locator_kind = OutputLocatorKind::Filename;
        scalar.output_contract.locator_hint = "package.json".to_string();
        let mut round2 = LoopState::default();
        round2.round_no = 2;
        assert_eq!(
            classify_planning_prompt_class(Some(&scalar), &scalar.resolved_intent, &round2)
                .as_str(),
            "open_planning"
        );
    }

    #[test]
    fn planning_prompt_class_keeps_open_planning_for_current_workspace_drafting() {
        let mut route = base_route_result();
        route.routed_mode = RoutedMode::ChatAct;
        route.ask_mode = crate::AskMode::from_routed_mode(RoutedMode::ChatAct);
        route.resolved_intent =
            "Write a short RustClaw setup note for the current workspace project".to_string();
        route.output_contract.response_shape = OutputResponseShape::Free;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = OutputSemanticKind::None;
        route.output_contract.locator_hint = "rustclaw workspace".to_string();

        assert_eq!(
            classify_planning_prompt_class(
                Some(&route),
                &route.resolved_intent,
                &LoopState::default()
            )
            .as_str(),
            "open_planning"
        );
    }

    #[test]
    fn round1_prompt_spec_switches_to_lightweight_prompt_for_light_class() {
        assert_eq!(
            round1_prompt_spec_for_class(PlanningPromptClass::OpenPlanning),
            (
                "single_plan_execution_prompt",
                "prompts/single_plan_execution_prompt.md",
            )
        );
        assert_eq!(
            round1_prompt_spec_for_class(PlanningPromptClass::LightweightExecution),
            (
                "lightweight_execution_prompt",
                "prompts/lightweight_execution_prompt.md",
            )
        );
    }

    #[test]
    fn lightweight_tool_spec_includes_contract_and_auto_locator() {
        let mut route = base_route_result();
        route.route_reason = "llm_contract:generic_explicit_path_scalar_extract".to_string();
        route.resolved_intent = "读取 UI/package.json 里的 name 字段，只输出值".to_string();
        route.output_contract.response_shape = OutputResponseShape::Scalar;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
        route.output_contract.locator_hint = "UI/package.json".to_string();
        let rendered = build_lightweight_tool_spec(Some(&route), Some("/tmp/UI/package.json"));
        assert!(rendered.contains("planning_class=lightweight_execution"));
        assert!(rendered.contains("response_shape=scalar"));
        assert!(rendered.contains("locator_hint=UI/package.json"));
        assert!(rendered.contains("auto_locator_path=/tmp/UI/package.json"));
    }

    #[test]
    fn rewrite_extract_field_field_alias_to_field_path() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "extract_field",
                "path": "/tmp/config.toml",
                "field": "tools.allow_sudo"
            }),
        }];
        let out = rewrite_extract_field_alias_args(actions);
        match &out[0] {
            AgentAction::CallSkill { args, .. } => {
                assert_eq!(
                    args.get("field_path").and_then(|value| value.as_str()),
                    Some("tools.allow_sudo")
                );
                assert!(args.get("field").is_none());
            }
            other => panic!("expected call_skill, got {other:?}"),
        }
    }

    #[test]
    fn rewrite_extract_field_keeps_existing_field_path() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "extract_field",
                "path": "/tmp/config.toml",
                "field": "tools.allow_sudo",
                "field_path": "tools.allow_path_outside_workspace"
            }),
        }];
        let out = rewrite_extract_field_alias_args(actions);
        match &out[0] {
            AgentAction::CallSkill { args, .. } => {
                assert_eq!(
                    args.get("field_path").and_then(|value| value.as_str()),
                    Some("tools.allow_path_outside_workspace")
                );
                assert_eq!(
                    args.get("field").and_then(|value| value.as_str()),
                    Some("tools.allow_sudo")
                );
            }
            other => panic!("expected call_skill, got {other:?}"),
        }
    }

    #[test]
    fn rewrite_extract_field_file_path_alias_to_path() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "extract_field",
                "file_path": "/tmp/config.toml",
                "field_path": "tools.allow_sudo"
            }),
        }];
        let out = rewrite_extract_field_alias_args(actions);
        match &out[0] {
            AgentAction::CallSkill { args, .. } => {
                assert_eq!(
                    args.get("path").and_then(|value| value.as_str()),
                    Some("/tmp/config.toml")
                );
                assert!(args.get("file_path").is_none());
            }
            other => panic!("expected call_skill, got {other:?}"),
        }
    }

    #[test]
    fn rewrite_extract_field_target_alias_to_path() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "extract_field",
                "target": "/tmp/config.toml",
                "field_path": "tools.allow_sudo"
            }),
        }];
        let out = rewrite_extract_field_alias_args(actions);
        match &out[0] {
            AgentAction::CallSkill { args, .. } => {
                assert_eq!(
                    args.get("path").and_then(|value| value.as_str()),
                    Some("/tmp/config.toml")
                );
                assert!(args.get("target").is_none());
            }
            other => panic!("expected call_skill, got {other:?}"),
        }
    }

    #[test]
    fn extract_field_rewrites_bare_manifest_to_shallow_candidate_with_field() {
        let root = TempDirGuard::new("structured_manifest_candidate");
        fs::write(
            root.path.join("package.json"),
            r#"{"dependencies":{"left-pad":"1.0.0"}}"#,
        )
        .expect("write root package");
        fs::create_dir_all(root.path.join("UI")).expect("create ui");
        fs::write(
            root.path.join("UI/package.json"),
            r#"{"name":"react-example"}"#,
        )
        .expect("write ui package");
        fs::create_dir_all(root.path.join("services/wa-web-bridge")).expect("create service");
        fs::write(
            root.path.join("services/wa-web-bridge/package.json"),
            r#"{"name":"wa-web-bridge"}"#,
        )
        .expect("write service package");

        let mut state = test_state();
        state.skill_rt.workspace_root = root.path.clone();
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Scalar);
        route.output_contract.locator_kind = OutputLocatorKind::Filename;
        let root_package = root.path.join("package.json");
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "extract_field",
                "path": root_package.display().to_string(),
                "field_path": "name"
            }),
        }];

        let normalized = super::normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(1),
            "读取 package.json 里的 name 字段",
            Some(root_package.to_string_lossy().as_ref()),
            actions,
        );
        let AgentAction::CallSkill { args, .. } = &normalized[0] else {
            panic!("expected call skill");
        };
        assert_eq!(
            args.get("path").and_then(Value::as_str),
            Some(root.path.join("UI/package.json").to_string_lossy().as_ref())
        );
    }

    #[test]
    fn extract_field_rewrites_workspace_cargo_package_field_to_current_package_manifest() {
        let root = TempDirGuard::new("workspace_cargo_candidate");
        fs::write(
            root.path.join("Cargo.toml"),
            r#"[workspace]
members = ["crates/other", "crates/clawd"]
"#,
        )
        .expect("write workspace cargo");
        fs::create_dir_all(root.path.join("crates/other")).expect("create other");
        fs::write(
            root.path.join("crates/other/Cargo.toml"),
            r#"[package]
name = "other"
"#,
        )
        .expect("write other cargo");
        fs::create_dir_all(root.path.join("crates/clawd")).expect("create clawd");
        fs::write(
            root.path.join("crates/clawd/Cargo.toml"),
            r#"[package]
name = "clawd"
"#,
        )
        .expect("write clawd cargo");

        let mut state = test_state();
        state.skill_rt.workspace_root = root.path.clone();
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Scalar);
        route.output_contract.locator_kind = OutputLocatorKind::Filename;
        let root_cargo = root.path.join("Cargo.toml");
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "extract_field",
                "path": root_cargo.display().to_string(),
                "field_path": "package.name"
            }),
        }];

        let normalized = super::normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(1),
            "读取 Cargo.toml 的 package.name",
            Some(root_cargo.to_string_lossy().as_ref()),
            actions,
        );
        let AgentAction::CallSkill { args, .. } = &normalized[0] else {
            panic!("expected call skill");
        };
        assert_eq!(
            args.get("path").and_then(Value::as_str),
            Some(
                root.path
                    .join("crates/clawd/Cargo.toml")
                    .to_string_lossy()
                    .as_ref()
            )
        );
    }

    fn should_force_plan_repair(
        route_result: Option<&RouteResult>,
        loop_state: &LoopState,
        actions: &[AgentAction],
    ) -> bool {
        should_force_actionable_plan_repair(&test_state(), route_result, loop_state, actions)
    }

    fn repair_reason(
        route_result: Option<&RouteResult>,
        loop_state: &LoopState,
        actions: Option<&[AgentAction]>,
    ) -> &'static str {
        plan_repair_reason(&test_state(), route_result, loop_state, actions)
    }

    fn route_result(
        mode: RoutedMode,
        requires_content_evidence: bool,
        response_shape: OutputResponseShape,
    ) -> RouteResult {
        RouteResult {
            routed_mode: mode,
            ask_mode: crate::AskMode::from_routed_mode(mode),
            resolved_intent: "test".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape,
                requires_content_evidence,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: Default::default(),
                semantic_kind: OutputSemanticKind::None,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        }
    }

    fn delivery_route_result() -> RouteResult {
        let mut route = route_result(RoutedMode::Act, false, OutputResponseShape::FileToken);
        route.output_contract.delivery_required = true;
        route
    }

    #[test]
    fn actionable_route_repairs_respond_only_plan_before_any_observation() {
        let loop_state = LoopState::new(2);
        let actions = vec![AgentAction::Respond {
            content: "final answer".to_string(),
        }];
        assert!(should_force_plan_repair(
            Some(&route_result(
                RoutedMode::ChatAct,
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn content_evidence_route_repairs_respond_only_plan_even_in_chat_mode() {
        let loop_state = LoopState::new(2);
        let actions = vec![AgentAction::Respond {
            content: "guessed answer".to_string(),
        }];
        assert!(should_force_plan_repair(
            Some(&route_result(
                RoutedMode::Chat,
                true,
                OutputResponseShape::Free,
            )),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn workspace_synthesis_respond_only_plan_gets_default_evidence_actions() {
        let loop_state = LoopState::new(2);
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
        let actions = vec![AgentAction::Respond {
            content: "guessed release note".to_string(),
        }];

        let normalized = normalize_planned_actions(
            &test_state(),
            Some(&route),
            &loop_state,
            "Write a short release note for RustClaw.",
            None,
            actions,
        );

        assert!(normalized.iter().any(|action| matches!(
            action,
            AgentAction::CallSkill { skill, args }
                if skill == "git_basic"
                    && args.get("action").and_then(|value| value.as_str()) == Some("log")
        )));
        assert!(normalized.iter().any(|action| matches!(
            action,
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(|value| value.as_str())
                        == Some("read_range")
                    && args.get("path").and_then(|value| value.as_str()) == Some("README.md")
        )));
        assert!(!should_force_actionable_plan_repair(
            &test_state(),
            Some(&route),
            &loop_state,
            &normalized
        ));
    }

    #[test]
    fn workspace_synthesis_plan_adds_missing_text_evidence_and_synthesizes_all_steps() {
        let loop_state = LoopState::new(2);
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Free);
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({"action":"tree_summary","path":"."}),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action":"extract_fields",
                    "path":"Cargo.toml",
                    "field_paths":["workspace.package.version"]
                }),
            },
            AgentAction::Respond {
                content: "# Release\nSee README.md\n- guessed from Cargo.toml".to_string(),
            },
        ];

        let normalized = normalize_planned_actions(
            &test_state(),
            Some(&route),
            &loop_state,
            "Write a short release note for RustClaw.",
            None,
            actions,
        );

        assert!(normalized.iter().any(|action| matches!(
            action,
            AgentAction::CallSkill { skill, args }
                if skill == "git_basic"
                    && args.get("action").and_then(|value| value.as_str()) == Some("log")
        )));
        assert!(normalized.iter().any(|action| matches!(
            action,
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(|value| value.as_str())
                        == Some("read_range")
                    && args.get("path").and_then(|value| value.as_str()) == Some("README.md")
        )));
        let synth_refs = normalized.iter().find_map(|action| match action {
            AgentAction::SynthesizeAnswer { evidence_refs } => Some(evidence_refs),
            _ => None,
        });
        assert_eq!(
            synth_refs,
            Some(&vec![
                "step_1".to_string(),
                "step_2".to_string(),
                "step_3".to_string(),
                "step_4".to_string(),
            ])
        );
        assert!(!should_force_actionable_plan_repair(
            &test_state(),
            Some(&route),
            &loop_state,
            &normalized
        ));
    }

    #[test]
    fn workspace_discovery_only_plan_waits_for_text_evidence_before_synthesis() {
        let loop_state = LoopState::new(2);
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = OutputSemanticKind::None;
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({"action": "workspace_glance", "path": ".", "max_entries": 30}),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({"action": "find_path", "name": "README.md", "target_kind": "file"}),
            },
        ];

        let normalized = normalize_planned_actions(
            &test_state(),
            Some(&route),
            &loop_state,
            "Write a deployment note for the current project.",
            None,
            actions,
        );

        assert_eq!(normalized.len(), 2);
        assert!(normalized.iter().all(|action| {
            !matches!(
                action,
                AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. }
            )
        }));
    }

    #[test]
    fn workspace_text_read_observation_can_append_synthesis() {
        let loop_state = LoopState::new(2);
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = OutputSemanticKind::None;
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "read_range", "path": "README.md", "mode": "head", "n": 40}),
        }];

        let normalized = normalize_planned_actions(
            &test_state(),
            Some(&route),
            &loop_state,
            "Write a deployment note for the current project.",
            None,
            actions,
        );

        assert_eq!(normalized.len(), 2);
        assert!(matches!(
            &normalized[1],
            AgentAction::SynthesizeAnswer { evidence_refs } if evidence_refs == &vec!["step_1".to_string()]
        ));
    }

    #[test]
    fn workspace_default_evidence_does_not_expand_mixed_last_output_answer() {
        let loop_state = LoopState::new(2);
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::OneSentence);
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = OutputSemanticKind::None;
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({"command": "pwd"}),
            },
            AgentAction::Respond {
                content: "{{last_output}} 是当前工作目录，通常对应正在操作的项目根目录。"
                    .to_string(),
            },
        ];

        let normalized = normalize_planned_actions(
            &test_state(),
            Some(&route),
            &loop_state,
            "执行 pwd，然后用一句话解释这个路径大概是什么",
            None,
            actions,
        );

        assert_eq!(normalized.len(), 2);
        assert!(normalized.iter().all(|action| {
            !matches!(
                action,
                AgentAction::CallSkill { skill, .. }
                    if skill == "git_basic" || skill == "system_basic"
            )
        }));
    }

    #[test]
    fn listing_grounded_workspace_synthesis_does_not_expand_default_text_evidence() {
        let loop_state = LoopState::new(2);
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = OutputSemanticKind::None;
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({"action": "inventory_dir", "path": ".", "names_only": true}),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["step_1".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        let normalized = normalize_planned_actions(
            &test_state(),
            Some(&route),
            &loop_state,
            "List the current directory, then answer from that listing.",
            None,
            actions,
        );

        assert_eq!(normalized.len(), 3);
        assert!(!normalized.iter().any(|action| {
            matches!(action, AgentAction::CallSkill { skill, .. } if skill == "git_basic")
                || matches!(
                    action,
                    AgentAction::CallSkill { skill, args }
                        if skill == "system_basic"
                            && args.get("action").and_then(Value::as_str) == Some("read_range")
                            && args.get("path").and_then(Value::as_str) == Some("README.md")
                )
        }));
        assert!(matches!(
            &normalized[1],
            AgentAction::SynthesizeAnswer { evidence_refs }
                if evidence_refs == &vec!["step_1".to_string()]
        ));
    }

    #[test]
    fn workspace_default_evidence_does_not_expand_structured_count_answer() {
        let loop_state = LoopState::new(2);
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::OneSentence);
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = OutputSemanticKind::None;
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({"action": "count_inventory", "path": "crates"}),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({"action": "count_inventory", "path": "crates/skills"}),
            },
            AgentAction::Respond {
                content: "{{s1.output}} | {{s2.output}}".to_string(),
            },
        ];

        let normalized = normalize_planned_actions(
            &test_state(),
            Some(&route),
            &loop_state,
            "count two directories and explain the layout",
            None,
            actions,
        );

        assert_eq!(normalized.len(), 4);
        assert!(normalized.iter().all(|action| {
            !matches!(
                action,
                AgentAction::CallSkill { skill, .. } if skill == "git_basic"
            )
        }));
        assert!(matches!(
            &normalized[2],
            AgentAction::SynthesizeAnswer { evidence_refs }
                if evidence_refs == &vec!["s1.output".to_string(), "s2.output".to_string()]
        ));
    }

    #[test]
    fn workspace_default_evidence_does_not_expand_single_structured_count_answer() {
        let loop_state = LoopState::new(1);
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::OneSentence);
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = OutputSemanticKind::None;
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "count_inventory",
                    "path": ".",
                    "kind_filter": "file",
                    "recursive": false,
                    "include_hidden": false
                }),
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        let normalized = normalize_planned_actions(
            &test_state(),
            Some(&route),
            &loop_state,
            "数一下当前目录一级有多少个普通文件，只告诉我数字和一句解释",
            None,
            actions,
        );

        assert_eq!(normalized.len(), 1);
        assert!(matches!(
            &normalized[0],
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(Value::as_str) == Some("count_inventory")
        ));
        assert!(!normalized.iter().any(|action| {
            matches!(action, AgentAction::CallSkill { skill, .. } if skill == "git_basic")
                || matches!(
                    action,
                    AgentAction::CallSkill { skill, args }
                        if skill == "system_basic"
                            && args.get("action").and_then(Value::as_str) == Some("read_range")
                )
        }));
    }

    #[test]
    fn unavailable_skill_plan_forces_repair() {
        let state = test_state_with_enabled_skills(&["run_cmd", "read_file"]);
        let loop_state = LoopState::new(2);
        let actions = vec![AgentAction::CallSkill {
            skill: "disabled_writer".to_string(),
            args: json!({ "path": "out.txt" }),
        }];
        let route = route_result(RoutedMode::ChatAct, false, OutputResponseShape::Free);

        assert!(should_force_actionable_plan_repair(
            &state,
            Some(&route),
            &loop_state,
            &actions
        ));
        assert_eq!(
            plan_repair_reason(&state, Some(&route), &loop_state, Some(&actions)),
            "unavailable_skill_requires_replan"
        );
    }

    #[test]
    fn preferred_registry_skill_route_forces_repair_from_run_cmd() {
        let state = test_state_with_registry();
        let loop_state = LoopState::new(2);
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Free);
        route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "systemctl status clawd"}),
        }];

        assert!(super::registry_preferred_skill_matches_route(
            &state, &route
        ));
        assert!(
            super::actions_use_ad_hoc_command_without_route_preferred_skill(
                &state, &route, &actions
            )
        );
        assert!(should_force_actionable_plan_repair(
            &state,
            Some(&route),
            &loop_state,
            &actions
        ));
        assert_eq!(
            plan_repair_reason(&state, Some(&route), &loop_state, Some(&actions)),
            "preferred_skill_required_for_semantic_route"
        );
        assert!(!can_fallback_to_initial_plan_after_repair_failure(
            &state,
            Some(&route),
            &loop_state,
            &actions
        ));
    }

    #[test]
    fn explicit_literal_run_cmd_marker_skips_preferred_skill_repair() {
        let state = test_state_with_registry();
        let loop_state = LoopState::new(2);
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Free);
        route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableListing;
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({
                "command": "sqlite3 data/db-basic-contract.sqlite '.tables'",
                super::super::CLAWD_LITERAL_COMMAND_ARG: true
            }),
        }];

        assert!(super::registry_preferred_skill_matches_route(
            &state, &route
        ));
        assert!(
            !super::actions_use_ad_hoc_command_without_route_preferred_skill(
                &state, &route, &actions
            )
        );
        assert!(!should_force_actionable_plan_repair(
            &state,
            Some(&route),
            &loop_state,
            &actions
        ));
        assert!(can_fallback_to_initial_plan_after_repair_failure(
            &state,
            Some(&route),
            &loop_state,
            &actions
        ));
    }

    #[test]
    fn explicit_literal_existing_run_cmd_is_marked_before_repair_checks() {
        let mut state = test_state_with_registry();
        state.policy.command_intent.execute_prefixes = vec!["执行命令 ".to_string()];
        let loop_state = LoopState::new(1);
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Free);
        route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableListing;
        route.output_contract.locator_hint = "data/db-basic-contract.sqlite".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "sqlite3 data/db-basic-contract.sqlite '.tables'"}),
        }];

        let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "执行 sqlite3 命令查询 data/db-basic-contract.sqlite 数据库中的所有表名，并返回结果。",
            Some("执行命令 sqlite3 data/db-basic-contract.sqlite \".tables\"，告诉我结果。"),
            None,
            actions,
        );

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "run_cmd");
                assert_eq!(
                    args.get(super::super::CLAWD_LITERAL_COMMAND_ARG)
                        .and_then(Value::as_bool),
                    Some(true)
                );
            }
            other => panic!("expected run_cmd action, got {other:?}"),
        }
        assert!(!should_force_actionable_plan_repair(
            &state,
            Some(&route),
            &loop_state,
            &normalized
        ));
    }

    #[test]
    fn explicit_literal_scalar_route_marks_failure_repairable() {
        let mut state = test_state_with_registry();
        state.policy.command_intent.execute_prefixes = vec!["执行 ".to_string()];
        let loop_state = LoopState::new(1);
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Scalar);
        route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "missing_probe --version"}),
        }];

        let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "执行 missing_probe --version；如果该命令不存在，则执行 which bash，并只返回 bash 的路径。",
            Some("执行 missing_probe --version；如果该命令不存在，则执行 which bash，并只返回 bash 的路径。"),
            None,
            actions,
        );

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "run_cmd");
                assert_eq!(
                    args.get(super::super::CLAWD_LITERAL_COMMAND_ARG)
                        .and_then(Value::as_bool),
                    Some(true)
                );
                assert_eq!(
                    args.get(super::super::CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG)
                        .and_then(Value::as_bool),
                    Some(true)
                );
            }
            other => panic!("expected run_cmd action, got {other:?}"),
        }
    }

    #[test]
    fn file_paths_route_marks_missing_target_repairable() {
        let state = test_state_with_registry();
        let loop_state = LoopState::new(1);
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Strict);
        route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
        let actions = vec![AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({"path": "plan/missing.md"}),
        }];

        let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "read missing, then find a related file",
            Some("read missing, then find a related file"),
            None,
            actions,
        );

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "read_file");
                assert_eq!(
                    args.get(super::super::CLAWD_MISSING_TARGET_REPAIRABLE_ARG)
                        .and_then(Value::as_bool),
                    Some(true)
                );
            }
            other => panic!("expected read_file action, got {other:?}"),
        }
    }

    #[test]
    fn raw_command_output_route_does_not_force_preferred_skill_repair() {
        let state = test_state_with_registry();
        let loop_state = LoopState::new(2);
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Free);
        route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "systemctl status clawd"}),
        }];

        assert!(!should_force_actionable_plan_repair(
            &state,
            Some(&route),
            &loop_state,
            &actions
        ));
    }

    #[test]
    fn repair_failure_does_not_fallback_to_unavailable_skill_plan() {
        let state = test_state_with_enabled_skills(&["run_cmd", "read_file"]);
        let loop_state = LoopState::new(2);
        let actions = vec![AgentAction::CallSkill {
            skill: "disabled_reader".to_string(),
            args: json!({ "path": "README.md" }),
        }];
        let route = route_result(RoutedMode::ChatAct, false, OutputResponseShape::Free);

        assert!(!can_fallback_to_initial_plan_after_repair_failure(
            &state,
            Some(&route),
            &loop_state,
            &actions
        ));
    }

    #[test]
    fn actionable_route_allows_respond_only_after_observation_exists() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        let actions = vec![AgentAction::Respond {
            content: "final answer".to_string(),
        }];
        assert!(!should_force_plan_repair(
            Some(&route_result(
                RoutedMode::ChatAct,
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn content_evidence_route_keeps_observation_only_plan_for_observed_finalize() {
        let loop_state = LoopState::new(2);
        let actions = vec![AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({ "path": "README.md" }),
        }];
        assert!(!should_force_plan_repair(
            Some(&route_result(
                RoutedMode::ChatAct,
                true,
                OutputResponseShape::Free,
            )),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn lightweight_act_route_keeps_observation_only_plan_without_repair() {
        let loop_state = LoopState::new(2);
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "read_range",
                "path": "/tmp/device_local/logs/model_io.log",
                "mode": "tail",
                "n": 4
            }),
        }];
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Free);
        route.resolved_intent = "读取 /tmp/device_local/logs/model_io.log 最后 4 行".to_string();
        route.output_contract.locator_hint = "/tmp/device_local/logs/model_io.log".to_string();
        assert!(!should_force_plan_repair(
            Some(&route),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn lightweight_route_rejects_unavailable_followup_skill() {
        let state = test_state_with_enabled_skills(&["read_file"]);
        let loop_state = LoopState::new(2);
        let actions = vec![
            AgentAction::CallSkill {
                skill: "read_file".to_string(),
                args: serde_json::json!({ "path": "README.md" }),
            },
            AgentAction::CallSkill {
                skill: "formatter".to_string(),
                args: serde_json::json!({ "text": "用一句话总结 {{last_output}}" }),
            },
        ];
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::OneSentence);
        route.route_reason = "llm_contract:generic_filename_single_read".to_string();
        route.resolved_intent = "看一下 README.md，然后一句话说它主要讲了什么".to_string();
        route.output_contract.locator_kind = OutputLocatorKind::Filename;
        route.output_contract.locator_hint = "README.md".to_string();
        assert!(should_force_actionable_plan_repair(
            &state,
            Some(&route),
            &loop_state,
            &actions,
        ));
        assert_eq!(
            plan_repair_reason(&state, Some(&route), &loop_state, Some(&actions)),
            "unavailable_skill_requires_replan"
        );
    }

    #[test]
    fn clarify_followup_tail_request_does_not_rewrite_single_read_file_from_text() {
        let state = test_state();
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Free);
        route.resolved_intent = "Continue the previous request that was waiting for clarification: 看看那个模型日志最后 5 行\nUser now provides the missing target/content: scripts/nl_tests/fixtures/device_local/logs/model_io.log".to_string();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "read_file".to_string(),
                args: json!({
                    "path": "scripts/nl_tests/fixtures/device_local/logs/model_io.log"
                }),
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        let normalized = normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(2),
            "scripts/nl_tests/fixtures/device_local/logs/model_io.log",
            None,
            actions,
        );

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "read_file");
                assert_eq!(
                    args.get("path").and_then(|value| value.as_str()),
                    Some("scripts/nl_tests/fixtures/device_local/logs/model_io.log")
                );
            }
            other => panic!("expected read_file to stay unchanged, got {other:?}"),
        }
    }

    #[test]
    fn non_range_single_read_keeps_read_file_plan() {
        let state = test_state();
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Free);
        route.resolved_intent =
            "看看 scripts/nl_tests/fixtures/device_local/logs/model_io.log".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({
                "path": "scripts/nl_tests/fixtures/device_local/logs/model_io.log"
            }),
        }];

        let normalized = normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(2),
            "scripts/nl_tests/fixtures/device_local/logs/model_io.log",
            None,
            actions,
        );

        assert!(matches!(
            &normalized[0],
            AgentAction::CallSkill { skill, .. } if skill == "read_file"
        ));
    }

    #[test]
    fn single_target_read_file_prefers_auto_locator_file_over_stale_existing_path() {
        let state = test_state();
        let root = TempDirGuard::new("single_target_read_file");
        let stale = root.path.join("stale.log");
        let current = root.path.join("clawd.log");
        fs::write(&stale, "stale\n").expect("write stale file");
        fs::write(&current, "fresh\n").expect("write current file");
        let stale_path = stale.display().to_string();
        let current_path = current.display().to_string();

        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Free);
        route.resolved_intent = format!("读取 {} 的内容", current_path);
        route.output_contract.locator_hint = current_path.clone();

        let actions = vec![
            AgentAction::CallSkill {
                skill: "read_file".to_string(),
                args: json!({ "path": stale_path }),
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        let normalized = normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(2),
            "第二个的内容",
            Some(current_path.as_str()),
            actions,
        );

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "read_file");
                assert_eq!(
                    args.get("path").and_then(|value| value.as_str()),
                    Some(current_path.as_str())
                );
            }
            other => panic!("expected read_file action, got {other:?}"),
        }
    }

    #[test]
    fn single_target_read_range_prefers_auto_locator_file_over_stale_existing_path() {
        let state = test_state();
        let root = TempDirGuard::new("single_target_read_range");
        let stale = root.path.join("hello_from_manual_test.sh");
        let current = root.path.join("clawd.log");
        fs::write(&stale, "#!/bin/bash\necho stale\n").expect("write stale file");
        fs::write(&current, "line1\nline2\n").expect("write current file");
        let stale_path = stale.display().to_string();
        let current_path = current.display().to_string();

        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Free);
        route.resolved_intent = format!("查看 {} 最后 2 行", current_path);
        route.output_contract.locator_hint = current_path.clone();

        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "read_range",
                    "path": stale_path,
                    "mode": "tail",
                    "n": 2
                }),
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        let normalized = normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(2),
            "第二个的最后 2 行",
            Some(current_path.as_str()),
            actions,
        );

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("path").and_then(|value| value.as_str()),
                    Some(current_path.as_str())
                );
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("read_range")
                );
            }
            other => panic!("expected system_basic read_range, got {other:?}"),
        }
    }

    #[test]
    fn auto_locator_file_does_not_collapse_multi_read_plan() {
        let state = test_state();
        let root = TempDirGuard::new("multi_read_preserve");
        let alpha = root.path.join("alpha.log");
        let beta = root.path.join("beta.log");
        fs::write(&alpha, "alpha\n").expect("write alpha");
        fs::write(&beta, "beta\n").expect("write beta");
        let alpha_path = alpha.display().to_string();
        let beta_path = beta.display().to_string();

        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        route.resolved_intent = "对比两个文件".to_string();

        let actions = vec![
            AgentAction::CallSkill {
                skill: "read_file".to_string(),
                args: json!({ "path": alpha_path.clone() }),
            },
            AgentAction::CallSkill {
                skill: "read_file".to_string(),
                args: json!({ "path": beta_path.clone() }),
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        let normalized = normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(2),
            "对比 alpha 和 beta",
            Some(beta_path.as_str()),
            actions,
        );

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "read_file");
                assert_eq!(
                    args.get("path").and_then(|value| value.as_str()),
                    Some(alpha_path.as_str())
                );
            }
            other => panic!("expected first read_file action, got {other:?}"),
        }
        match &normalized[1] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "read_file");
                assert_eq!(
                    args.get("path").and_then(|value| value.as_str()),
                    Some(beta_path.as_str())
                );
            }
            other => panic!("expected second read_file action, got {other:?}"),
        }
    }

    #[test]
    fn content_evidence_route_keeps_terminal_discussion_followup_for_planned_synthesis() {
        let loop_state = LoopState::new(2);
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "read_range",
                    "path": "README.md",
                    "mode": "head",
                    "n": 20
                }),
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];
        let kept = strip_terminal_discussion_for_observed_finalize(
            Some(&route_result(
                RoutedMode::ChatAct,
                true,
                OutputResponseShape::Free,
            )),
            &loop_state,
            actions.clone(),
        );
        assert_eq!(kept.len(), 2);
        assert!(matches!(
            &kept[0],
            AgentAction::CallSkill { skill, .. } if skill == "system_basic"
        ));
        assert!(matches!(
            &kept[1],
            AgentAction::Respond { content } if content == "{{last_output}}"
        ));
    }

    #[test]
    fn content_evidence_route_keeps_terminal_synthesize_followup_for_planned_synthesis() {
        let loop_state = LoopState::new(2);
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "read_range",
                    "path": "README.md",
                    "mode": "head",
                    "n": 20
                }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
        ];
        let kept = strip_terminal_discussion_for_observed_finalize(
            Some(&route_result(
                RoutedMode::ChatAct,
                true,
                OutputResponseShape::Free,
            )),
            &loop_state,
            actions.clone(),
        );
        assert_eq!(kept.len(), 2);
        assert!(matches!(
            &kept[0],
            AgentAction::CallSkill { skill, .. } if skill == "system_basic"
        ));
        assert!(matches!(
            &kept[1],
            AgentAction::SynthesizeAnswer { evidence_refs } if evidence_refs == &vec!["last_output".to_string()]
        ));
    }

    #[test]
    fn content_evidence_route_keeps_multi_evidence_synthesize_followup() {
        let loop_state = LoopState::new(2);
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "read_range",
                    "path": "service_notes.md",
                    "mode": "head",
                    "n": 20
                }),
            },
            AgentAction::CallSkill {
                skill: "read_file".to_string(),
                args: serde_json::json!({ "path": "README.md" }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["s1".to_string(), "s2".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];
        let kept = strip_terminal_discussion_for_observed_finalize(
            Some(&route_result(
                RoutedMode::ChatAct,
                true,
                OutputResponseShape::Free,
            )),
            &loop_state,
            actions.clone(),
        );
        assert_eq!(kept.len(), 4);
        assert!(matches!(
            &kept[0],
            AgentAction::CallSkill { skill, .. } if skill == "system_basic"
        ));
        assert!(matches!(
            &kept[1],
            AgentAction::CallSkill { skill, .. } if skill == "read_file"
        ));
        assert!(matches!(
            &kept[2],
            AgentAction::SynthesizeAnswer { evidence_refs }
                if evidence_refs == &vec!["s1".to_string(), "s2".to_string()]
        ));
        assert!(matches!(
            &kept[3],
            AgentAction::Respond { content } if content == "{{last_output}}"
        ));
    }

    #[test]
    fn scalar_path_observation_strips_guessed_terminal_respond() {
        let loop_state = LoopState::new(1);
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Scalar);
        route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
        route.output_contract.delivery_required = false;
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "path_batch_facts",
                    "paths": ["/workspace/stem_unique/abcd"],
                    "include_missing": true
                }),
            },
            AgentAction::Respond {
                content: "/workspace/stem_unique/abcd".to_string(),
            },
        ];

        let kept = strip_terminal_discussion_for_scalar_path_observation(
            Some(&route),
            &loop_state,
            actions,
        );
        assert_eq!(kept.len(), 1);
        assert!(matches!(
            &kept[0],
            AgentAction::CallSkill { skill, .. } if skill == "system_basic"
        ));
    }

    #[test]
    fn scalar_path_observation_does_not_strip_after_tool_output_started() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Scalar);
        route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
        route.output_contract.delivery_required = false;
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "path_batch_facts",
                    "paths": ["/workspace/stem_unique/abcd"],
                    "include_missing": true
                }),
            },
            AgentAction::Respond {
                content: "/workspace/stem_unique/abcd".to_string(),
            },
        ];

        let kept = strip_terminal_discussion_for_scalar_path_observation(
            Some(&route),
            &loop_state,
            actions.clone(),
        );
        assert_eq!(kept.len(), 2);
        assert!(matches!(
            &kept[0],
            AgentAction::CallSkill { skill, .. } if skill == "system_basic"
        ));
        assert!(matches!(
            &kept[1],
            AgentAction::Respond { content } if content == "/workspace/stem_unique/abcd"
        ));
    }

    #[test]
    fn system_basic_compare_paths_targets_alias_sets_left_and_right_paths() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "compare_paths",
                "targets": ["README.md", "AGENTS.md"],
            }),
        }];

        let normalized = normalize_system_basic_schema_aliases(actions);
        assert_eq!(normalized.len(), 1);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("compare_paths")
                );
                assert_eq!(
                    args.get("left_path").and_then(|value| value.as_str()),
                    Some("README.md")
                );
                assert_eq!(
                    args.get("right_path").and_then(|value| value.as_str()),
                    Some("AGENTS.md")
                );
            }
            other => panic!("expected system_basic compare_paths action, got {other:?}"),
        }
    }

    #[test]
    fn system_basic_compare_paths_numbered_alias_sets_left_and_right_paths() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "compare_paths",
                "path1": "Cargo.lock",
                "path2": "Cargo.toml",
            }),
        }];

        let normalized = normalize_system_basic_schema_aliases(actions);
        assert_eq!(normalized.len(), 1);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("left_path").and_then(|value| value.as_str()),
                    Some("Cargo.lock")
                );
                assert_eq!(
                    args.get("right_path").and_then(|value| value.as_str()),
                    Some("Cargo.toml")
                );
                assert!(args.get("path1").is_none());
                assert!(args.get("path2").is_none());
            }
            other => panic!("expected system_basic compare_paths action, got {other:?}"),
        }
    }

    #[test]
    fn system_basic_path_batch_facts_path_alias_becomes_paths_array() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "path_batch_facts",
                "path": "Cargo.toml",
            }),
        }];

        let normalized = normalize_system_basic_schema_aliases(actions);
        assert_eq!(normalized.len(), 1);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("path_batch_facts")
                );
                assert_eq!(args.get("paths"), Some(&json!(["Cargo.toml"])));
                assert!(args.get("path").is_none());
            }
            other => panic!("expected system_basic path_batch_facts action, got {other:?}"),
        }
    }

    #[test]
    fn system_basic_path_batch_facts_path_list_alias_becomes_paths() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "path_batch_facts",
                "path_list": ["Cargo.toml", "Cargo.lock"],
            }),
        }];

        let normalized = normalize_system_basic_schema_aliases(actions);
        assert_eq!(normalized.len(), 1);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("path_batch_facts")
                );
                assert_eq!(
                    args.get("paths"),
                    Some(&json!(["Cargo.toml", "Cargo.lock"]))
                );
                assert!(args.get("path_list").is_none());
            }
            other => panic!("expected system_basic path_batch_facts action, got {other:?}"),
        }
    }

    #[test]
    fn directory_read_range_after_inventory_is_stripped() {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "inventory_dir",
                    "path": "/workspace/docs",
                    "sort_by": "mtime_desc",
                    "max_entries": 2,
                    "names_only": false,
                }),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "read_range",
                    "path": "/workspace/docs/",
                    "mode": "head",
                    "n": 50,
                }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec![
                    "last_output".to_string(),
                    "s1".to_string(),
                    "s2".to_string(),
                ],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        let normalized = strip_directory_read_range_after_inventory_dir(actions);
        assert_eq!(normalized.len(), 3);
        assert!(normalized.iter().all(|action| {
            !matches!(
                action,
                AgentAction::CallSkill { skill, args }
                    if skill == "system_basic"
                        && args.get("action").and_then(Value::as_str) == Some("read_range")
            )
        }));
        match &normalized[1] {
            AgentAction::SynthesizeAnswer { evidence_refs } => {
                assert_eq!(
                    evidence_refs,
                    &vec!["last_output".to_string(), "s1".to_string()]
                );
            }
            other => panic!("expected synthesize_answer after inventory, got {other:?}"),
        }
    }

    #[test]
    fn child_file_read_range_after_inventory_is_kept() {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({"action": "inventory_dir", "path": "/workspace/docs"}),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "read_range",
                    "path": "/workspace/docs/README.md",
                    "mode": "head",
                    "n": 20,
                }),
            },
        ];

        let normalized = strip_directory_read_range_after_inventory_dir(actions);
        assert_eq!(normalized.len(), 2);
        assert!(matches!(
            &normalized[1],
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(Value::as_str) == Some("read_range")
        ));
    }

    #[test]
    fn unresolved_template_reads_after_inventory_are_stripped() {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "inventory_dir",
                    "path": "/workspace/docs",
                    "sort_by": "mtime_desc",
                    "max_entries": 2,
                }),
            },
            AgentAction::CallSkill {
                skill: "read_file".to_string(),
                args: json!({"path": "{{s1.entry0_path}}"}),
            },
            AgentAction::CallSkill {
                skill: "read_file".to_string(),
                args: json!({"path": "{{s1.entry1_path}}"}),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["s1".to_string(), "s2".to_string(), "s3".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        let normalized = strip_unresolved_template_reads_after_inventory_dir(actions);
        assert_eq!(normalized.len(), 3);
        assert!(normalized.iter().all(|action| {
            !matches!(
                action,
                AgentAction::CallSkill { skill, .. } if skill == "read_file"
            )
        }));
        match &normalized[1] {
            AgentAction::SynthesizeAnswer { evidence_refs } => {
                assert_eq!(evidence_refs, &vec!["s1".to_string()]);
            }
            other => panic!("expected synthesize_answer after inventory, got {other:?}"),
        }
    }

    #[test]
    fn unresolved_template_reads_after_fs_search_are_stripped() {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "fs_search".to_string(),
                args: json!({
                    "action": "find_name",
                    "pattern": "missing.txt",
                    "target_kind": "file",
                }),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "read_range",
                    "path": "{{last_output}}",
                    "mode": "head",
                    "n": 3,
                }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        let normalized = strip_unresolved_template_reads_after_inventory_dir(actions);
        assert_eq!(normalized.len(), 3);
        assert!(normalized.iter().all(|action| {
            !matches!(
                action,
                AgentAction::CallSkill { skill, args }
                    if skill == "system_basic"
                        && args.get("action").and_then(Value::as_str) == Some("read_range")
            )
        }));
        match &normalized[1] {
            AgentAction::SynthesizeAnswer { evidence_refs } => {
                assert_eq!(evidence_refs, &vec!["step_1".to_string()]);
            }
            other => panic!("expected synthesize_answer after fs_search, got {other:?}"),
        }
    }

    #[test]
    fn indexed_last_output_reads_after_inventory_are_kept() {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "inventory_dir",
                    "path": "/workspace/logs",
                    "sort_by": "mtime_desc",
                    "max_entries": 2,
                    "names_only": true,
                }),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "read_range",
                    "path": "/workspace/logs/{{last_output.0}}",
                    "mode": "head",
                    "n": 40,
                }),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "read_range",
                    "path": "/workspace/logs/{{ last_output[1] }}",
                    "mode": "head",
                    "n": 40,
                }),
            },
        ];

        let normalized = strip_unresolved_template_reads_after_inventory_dir(actions);
        assert_eq!(normalized.len(), 3);
        assert!(matches!(
            &normalized[1],
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(Value::as_str) == Some("read_range")
        ));
        assert!(matches!(
            &normalized[2],
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(Value::as_str) == Some("read_range")
        ));
    }

    #[test]
    fn scalar_path_auto_locator_file_builds_observation_plan() {
        let root = TempDirGuard::new("scalar_auto_locator");
        let report = root.path.join("Report.MD");
        fs::write(&report, "hello").expect("write report");
        let report_path = report.display().to_string();
        let route = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "只输出匹配文件路径".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ScalarPathOnly,
                locator_hint: "report.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };

        let actions =
            scalar_path_auto_locator_observation_plan(Some(&route), Some(&report_path)).unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("path_batch_facts")
                );
                assert_eq!(args.get("paths"), Some(&json!([report_path])));
            }
            other => panic!("expected system_basic path_batch_facts action, got {other:?}"),
        }
    }

    #[test]
    fn scalar_path_auto_locator_fast_plan_uses_structural_locator() {
        let root = TempDirGuard::new("scalar_auto_locator_fast_plan");
        let report = root.path.join("my_abcd.txt");
        fs::write(&report, "hello").expect("write report");
        let report_path = report.display().to_string();
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Scalar);
        route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "my_abcd.txt".to_string();
        route.output_contract.delivery_required = false;
        let mut loop_state = LoopState::default();
        loop_state.round_no = 1;

        let plan = scalar_path_auto_locator_fast_plan_result(
            "return the structurally resolved path",
            Some(&route),
            &loop_state,
            Some(&report_path),
        )
        .expect("fast plan should be available");

        assert_eq!(plan.plan_kind, PlanKind::Single);
        assert_eq!(plan.steps.len(), 1);
        assert!(plan.raw_plan_text.contains("path_batch_facts"));
        assert!(plan.raw_plan_text.contains(&report_path));
    }

    #[test]
    fn scalar_path_auto_locator_does_not_fast_path_directory_search_scope() {
        let root = TempDirGuard::new("scalar_auto_locator_search_scope");
        fs::write(root.path.join("ABCD.txt"), "hello").expect("write report");
        let root_path = root.path.display().to_string();
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Scalar);
        route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = root_path.clone();
        route.output_contract.delivery_required = false;
        let mut loop_state = LoopState::default();
        loop_state.round_no = 1;

        assert!(scalar_path_auto_locator_fast_plan_result(
            "find a named item inside the resolved directory",
            Some(&route),
            &loop_state,
            Some(&root_path),
        )
        .is_none());
    }

    #[test]
    fn scalar_path_auto_locator_directory_builds_observation_plan() {
        let root = TempDirGuard::new("scalar_auto_locator_dir");
        let root_path = root.path.display().to_string();
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Scalar);
        route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.delivery_required = false;

        let actions =
            scalar_path_auto_locator_observation_plan(Some(&route), Some(&root_path)).unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("path_batch_facts")
                );
                assert_eq!(args.get("paths"), Some(&json!([root_path])));
            }
            other => panic!("expected system_basic path_batch_facts action, got {other:?}"),
        }
    }

    #[test]
    fn scalar_path_respond_only_uses_auto_locator_observation() {
        let root = TempDirGuard::new("scalar_auto_locator_respond_only");
        let report = root.path.join("Report.MD");
        fs::write(&report, "hello").expect("write report");
        let report_path = report.display().to_string();
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Scalar);
        route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.delivery_required = false;
        let actions = vec![AgentAction::Respond {
            content: report_path.clone(),
        }];

        let normalized = replace_scalar_path_respond_only_with_auto_locator_observation(
            Some(&route),
            &LoopState::new(1),
            Some(&report_path),
            actions,
        );
        assert_eq!(normalized.len(), 1);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("path_batch_facts")
                );
                assert_eq!(args.get("paths"), Some(&json!([report_path])));
            }
            other => panic!("expected system_basic path_batch_facts action, got {other:?}"),
        }
    }

    #[test]
    fn scalar_path_respond_only_uses_loop_state_auto_locator_observation() {
        let root = TempDirGuard::new("scalar_auto_locator_loop_state");
        let report = root.path.join("Report.MD");
        fs::write(&report, "hello").expect("write report");
        let report_path = report.display().to_string();
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Scalar);
        route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
        route.output_contract.delivery_required = false;
        let actions = vec![AgentAction::Respond {
            content: report_path.clone(),
        }];
        let mut loop_state = LoopState::new(1);
        loop_state
            .output_vars
            .insert("auto_locator_path".to_string(), report_path.clone());

        let normalized = replace_scalar_path_respond_only_with_auto_locator_observation(
            Some(&route),
            &loop_state,
            None,
            actions,
        );
        assert_eq!(normalized.len(), 1);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("path_batch_facts")
                );
                assert_eq!(args.get("paths"), Some(&json!([report_path])));
            }
            other => panic!("expected system_basic path_batch_facts action, got {other:?}"),
        }
    }

    #[test]
    fn scalar_count_synthesis_only_uses_count_inventory_for_locator_dir() {
        let root = TempDirGuard::new("scalar_count_locator_dir");
        fs::write(root.path.join("a.txt"), "a").expect("write a");
        fs::write(root.path.join("b.txt"), "b").expect("write b");
        fs::create_dir_all(root.path.join("child")).expect("create child");
        let root_path = root.path.display().to_string();
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Scalar);
        route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = root_path.clone();
        route.output_contract.delivery_required = false;
        let actions = vec![
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        let normalized = replace_scalar_count_plan_with_count_inventory(
            Some(&route),
            &LoopState::new(1),
            Some(&root_path),
            actions,
        );

        assert_eq!(normalized.len(), 1);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("count_inventory")
                );
                assert_eq!(
                    args.get("path").and_then(|value| value.as_str()),
                    Some(root_path.as_str())
                );
            }
            other => panic!("expected system_basic count_inventory action, got {other:?}"),
        }
    }

    #[test]
    fn scalar_count_listing_plan_uses_count_inventory_for_locator_dir() {
        let root = TempDirGuard::new("scalar_count_listing_locator_dir");
        fs::write(root.path.join("a.txt"), "a").expect("write a");
        fs::write(root.path.join("b.txt"), "b").expect("write b");
        fs::create_dir_all(root.path.join("child")).expect("create child");
        let root_path = root.path.display().to_string();
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Scalar);
        route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = root_path.clone();
        route.output_contract.delivery_required = false;
        let actions = vec![
            AgentAction::CallSkill {
                skill: "list_dir".to_string(),
                args: json!({"path": root_path.clone()}),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        let normalized = replace_scalar_count_plan_with_count_inventory(
            Some(&route),
            &LoopState::new(1),
            Some(&root_path),
            actions,
        );

        assert_eq!(normalized.len(), 1);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("count_inventory")
                );
                assert_eq!(
                    args.get("path").and_then(|value| value.as_str()),
                    Some(root_path.as_str())
                );
            }
            other => panic!("expected system_basic count_inventory action, got {other:?}"),
        }
    }

    #[test]
    fn hidden_entries_scalar_contract_uses_inventory_dir() {
        let root = TempDirGuard::new("hidden_entries_scalar_plan");
        fs::write(root.path.join(".env"), "a").expect("write hidden");
        fs::write(root.path.join("visible.txt"), "b").expect("write visible");
        let root_path = root.path.display().to_string();
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Scalar);
        route.output_contract.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint = root_path.clone();
        route.output_contract.delivery_required = false;
        let actions = vec![
            AgentAction::CallSkill {
                skill: "list_dir".to_string(),
                args: json!({"path": root_path.clone()}),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        let state = test_state_with_enabled_skills(&["system_basic", "list_dir"]);
        let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &LoopState::new(1),
            "current workspace hidden entries check",
            None,
            Some(&root_path),
            actions,
        );

        assert_eq!(normalized.len(), 1);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("inventory_dir")
                );
                assert_eq!(
                    args.get("path").and_then(|value| value.as_str()),
                    Some(root_path.as_str())
                );
                assert_eq!(
                    args.get("include_hidden").and_then(Value::as_bool),
                    Some(true)
                );
                assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
            }
            other => panic!("expected system_basic inventory_dir action, got {other:?}"),
        }
    }

    #[test]
    fn hidden_entries_scalar_current_workspace_hint_falls_back_to_dot_inventory() {
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Scalar);
        route.output_contract.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint = "current directory".to_string();
        route.output_contract.delivery_required = false;
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({"command": "find . -maxdepth 1 -name '.*' | wc -l"}),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
        ];

        let state = test_state_with_enabled_skills(&["system_basic", "run_cmd"]);
        let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &LoopState::new(1),
            "count hidden entries in current directory",
            None,
            None,
            actions,
        );

        assert_eq!(normalized.len(), 1);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("inventory_dir")
                );
                assert_eq!(args.get("path").and_then(|value| value.as_str()), Some("."));
                assert_eq!(
                    args.get("include_hidden").and_then(Value::as_bool),
                    Some(true)
                );
            }
            other => panic!("expected system_basic inventory_dir action, got {other:?}"),
        }
    }

    #[test]
    fn service_status_contract_rewrites_pgrep_run_cmd_to_service_control_status() {
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::OneSentence);
        route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
        route.output_contract.locator_kind = OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({"command": "pgrep -x telegramd > /dev/null && echo 'running' || echo 'not running'"}),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
        ];

        let normalized =
            rewrite_service_status_plan_to_service_control(Some(&route), false, actions);

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "service_control");
                assert_eq!(args.get("action").and_then(Value::as_str), Some("status"));
                assert_eq!(
                    args.get("target").and_then(Value::as_str),
                    Some("telegramd")
                );
                assert!(args.get("manager_type").is_none());
            }
            other => panic!("expected service_control status action, got {other:?}"),
        }
        assert_eq!(normalized.len(), 1);
    }

    #[test]
    fn service_status_contract_rewrites_pgrep_script_without_trailing_shell_words() {
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::OneSentence);
        route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
        route.output_contract.locator_kind = OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "pgrep -fa telegramd 2>/dev/null; if [ $? -ne 0 ]; then echo 'telegramd is NOT currently running'; else echo 'telegramd is currently running'; fi"}),
        }];

        let normalized =
            rewrite_service_status_plan_to_service_control(Some(&route), false, actions);

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "service_control");
                assert_eq!(args.get("action").and_then(Value::as_str), Some("status"));
                assert_eq!(
                    args.get("target").and_then(Value::as_str),
                    Some("telegramd")
                );
            }
            other => panic!("expected service_control status action, got {other:?}"),
        }
    }

    #[test]
    fn service_status_contract_rewrites_systemctl_status_to_service_control_systemd() {
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::OneSentence);
        route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "run_cmd",
                "command": "systemctl is-active nginx.service"
            }),
        }];

        let normalized =
            rewrite_service_status_plan_to_service_control(Some(&route), false, actions);

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "service_control");
                assert_eq!(
                    args.get("target").and_then(Value::as_str),
                    Some("nginx.service")
                );
                assert_eq!(
                    args.get("manager_type").and_then(Value::as_str),
                    Some("systemd")
                );
            }
            other => panic!("expected service_control status action, got {other:?}"),
        }
    }

    #[test]
    fn normalize_prefers_registry_repair_over_legacy_service_rewrite() {
        let state = test_state_with_registry();
        let loop_state = LoopState::new(1);
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::OneSentence);
        route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "systemctl status clawd"}),
        }];

        let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "check clawd service status",
            None,
            None,
            actions,
        );

        assert!(matches!(
            &normalized[0],
            AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
        ));
        assert!(should_force_actionable_plan_repair(
            &state,
            Some(&route),
            &loop_state,
            &normalized
        ));
        assert_eq!(
            plan_repair_reason(&state, Some(&route), &loop_state, Some(&normalized)),
            "preferred_skill_required_for_semantic_route"
        );
    }

    #[test]
    fn normalize_prefers_registry_repair_over_legacy_sqlite_rewrite() {
        let state = test_state_with_registry();
        let loop_state = LoopState::new(1);
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Strict);
        route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableListing;
        route.output_contract.locator_hint = "data/db-basic-contract.sqlite".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({"path": "data/db-basic-contract.sqlite"}),
        }];

        let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "list sqlite tables",
            None,
            Some("data/db-basic-contract.sqlite"),
            actions,
        );

        assert!(matches!(
            &normalized[0],
            AgentAction::CallSkill { skill, .. } if skill == "read_file"
        ));
        assert_eq!(
            plan_repair_reason(&state, Some(&route), &loop_state, Some(&normalized)),
            "preferred_skill_required_for_semantic_route"
        );
    }

    #[test]
    fn normalize_prefers_registry_repair_over_legacy_docker_rewrite() {
        let state = test_state_with_registry();
        let loop_state = LoopState::new(1);
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::OneSentence);
        route.output_contract.semantic_kind = OutputSemanticKind::DockerPs;
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "docker ps"}),
        }];

        let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "show docker containers",
            None,
            None,
            actions,
        );

        assert!(matches!(
            &normalized[0],
            AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
        ));
        assert_eq!(
            plan_repair_reason(&state, Some(&route), &loop_state, Some(&normalized)),
            "preferred_skill_required_for_semantic_route"
        );
    }

    #[test]
    fn normalize_prefers_registry_repair_over_legacy_archive_unpack_rewrite() {
        let state = test_state_with_registry();
        let loop_state = LoopState::new(1);
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::OneSentence);
        route.output_contract.semantic_kind = OutputSemanticKind::ArchiveUnpack;
        route.output_contract.locator_hint = "/tmp/source.tgz | /tmp/source-unpacked".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "tar -xzf /tmp/source.tgz -C /tmp/source-unpacked"}),
        }];

        let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "unpack archive",
            None,
            None,
            actions,
        );

        assert!(matches!(
            &normalized[0],
            AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
        ));
        assert_eq!(
            plan_repair_reason(&state, Some(&route), &loop_state, Some(&normalized)),
            "preferred_skill_required_for_semantic_route"
        );
    }

    #[test]
    fn explicit_service_command_is_preserved_as_run_cmd() {
        let mut state = test_state_with_enabled_skills(&["service_control", "run_cmd"]);
        state.policy.command_intent.execute_prefixes = vec!["执行命令 ".to_string()];
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::OneSentence);
        route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
        let actions = vec![AgentAction::CallSkill {
            skill: "service_control".to_string(),
            args: json!({
                "action": "status",
                "target": "clawd"
            }),
        }];

        let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &LoopState::new(1),
            "执行命令 systemctl status clawd --no-pager，告诉我结果",
            Some("执行命令 systemctl status clawd --no-pager，告诉我结果"),
            None,
            actions,
        );

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "run_cmd");
                assert_eq!(
                    args.get("command").and_then(Value::as_str),
                    Some("systemctl status clawd --no-pager")
                );
            }
            other => panic!("expected preserved run_cmd action, got {other:?}"),
        }
    }

    #[test]
    fn observed_judgment_mixed_placeholder_respond_uses_synthesize_after_listing() {
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        route.output_contract.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "document".to_string();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "list_dir".to_string(),
                args: json!({"path": "document", "limit": 5}),
            },
            AgentAction::Respond {
                content:
                    "Here are the first files:\n{{last_output}}\nThese look more like documentation."
                        .to_string(),
            },
        ];

        let state = test_state_with_enabled_skills(&["list_dir"]);
        let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &LoopState::new(1),
            "list files and judge their role",
            None,
            None,
            actions,
        );

        assert_eq!(normalized.len(), 3);
        assert!(matches!(normalized[0], AgentAction::CallSkill { .. }));
        assert!(matches!(
            &normalized[1],
            AgentAction::SynthesizeAnswer { evidence_refs }
                if evidence_refs == &vec!["last_output".to_string()]
        ));
        assert!(matches!(
            &normalized[2],
            AgentAction::Respond { content } if content == "{{last_output}}"
        ));
    }

    #[test]
    fn scalar_count_preserves_planned_run_cmd_observation() {
        let root = TempDirGuard::new("scalar_count_run_cmd_plan");
        fs::write(root.path.join(".env"), "a").expect("write hidden");
        fs::write(root.path.join("visible.txt"), "b").expect("write visible");
        let root_path = root.path.display().to_string();
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Scalar);
        route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint = root_path.clone();
        route.output_contract.delivery_required = false;
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({"command": "printf '2\\n'"}),
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        let state = test_state_with_enabled_skills(&["system_basic", "run_cmd"]);
        let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &LoopState::new(1),
            "count current workspace entries",
            None,
            Some(&root_path),
            actions,
        );

        match normalized.iter().find(
            |action| matches!(action, AgentAction::CallSkill { skill, .. } if skill == "run_cmd"),
        ) {
            Some(AgentAction::CallSkill { skill, args }) => {
                assert_eq!(skill, "run_cmd");
                assert_eq!(
                    args.get("command").and_then(|value| value.as_str()),
                    Some("printf '2\\n'")
                );
            }
            other => panic!("expected run_cmd action, got {other:?}"),
        }
        assert!(!normalized.iter().any(|action| {
            matches!(
                action,
                AgentAction::CallSkill { skill, args }
                    if skill == "system_basic"
                        && args.get("action").and_then(Value::as_str) == Some("count_inventory")
            )
        }));
    }

    #[test]
    fn structured_keys_contract_rewrites_read_range_to_structured_keys() {
        let root = TempDirGuard::new("structured_keys_plan");
        let config_path = root.path.join("config.toml");
        fs::write(&config_path, "alpha = 1\n[beta]\nvalue = 2\n").expect("write config");
        let config_path = config_path.display().to_string();
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Strict);
        route.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = config_path.clone();
        route.output_contract.delivery_required = false;
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({"action": "read_range", "path": config_path.clone()}),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        let state = test_state_with_enabled_skills(&["system_basic"]);
        let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &LoopState::new(1),
            "list structured keys",
            None,
            Some(&config_path),
            actions,
        );

        assert_eq!(normalized.len(), 1);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(Value::as_str),
                    Some("structured_keys")
                );
                assert_eq!(
                    args.get("path").and_then(Value::as_str),
                    Some(config_path.as_str())
                );
            }
            other => panic!("expected system_basic structured_keys action, got {other:?}"),
        }
    }

    #[test]
    fn explicit_configured_command_request_rewrites_semantic_substitute_to_run_cmd() {
        let mut state = test_state_with_enabled_skills(&["run_cmd", "system_basic"]);
        state.policy.command_intent.execute_prefixes = vec!["execute ".to_string()];
        state.policy.command_intent.negative_markers = vec!["what is this command".to_string()];
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        let loop_state = LoopState::new(1);
        let original_request = "execute ls scripts, then summarize the directory";
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "inventory_dir",
                    "path": "/workspace/scripts",
                    "names_only": true,
                }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "list scripts and summarize the directory",
            Some(original_request),
            None,
            actions,
        );

        assert_eq!(normalized.len(), 3);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "run_cmd");
                assert_eq!(
                    args.get("request_text").and_then(Value::as_str),
                    Some(original_request)
                );
                assert!(args
                    .get("cwd")
                    .and_then(Value::as_str)
                    .is_some_and(|cwd| !cwd.trim().is_empty()));
                assert_eq!(
                    args.get("command").and_then(Value::as_str),
                    Some("ls scripts")
                );
            }
            other => panic!("expected run_cmd action, got {other:?}"),
        }
        assert!(matches!(
            &normalized[1],
            AgentAction::SynthesizeAnswer { evidence_refs } if evidence_refs == &vec!["last_output".to_string()]
        ));
    }

    #[test]
    fn explicit_command_rewrite_uses_configured_negative_markers() {
        let mut state = test_state_with_enabled_skills(&["run_cmd", "system_basic"]);
        state.policy.command_intent.execute_prefixes = vec!["execute ".to_string()];
        state.policy.command_intent.negative_markers = vec!["what is this command".to_string()];
        let route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        let loop_state = LoopState::new(1);
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "inventory_dir",
                "path": "/workspace/scripts",
                "names_only": true,
            }),
        }];

        let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "explain a command",
            Some("execute ls scripts: what is this command"),
            None,
            actions,
        );

        assert!(matches!(
            &normalized[0],
            AgentAction::CallSkill { skill, .. } if skill == "system_basic"
        ));
    }

    #[test]
    fn scalar_path_route_treats_fs_search_query_as_name_pattern_when_action_missing() {
        let root = TempDirGuard::new("fs_search_name_contract");
        let root_path = root.path.display().to_string();
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Scalar);
        route.output_contract.response_shape = OutputResponseShape::Scalar;
        route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
        route.output_contract.delivery_required = false;
        let actions = vec![AgentAction::CallSkill {
            skill: "fs_search".to_string(),
            args: json!({
                "path": root_path,
                "query": "abcd",
            }),
        }];

        let normalized = enforce_output_contract_tool_args(Some(&route), actions);
        assert_eq!(normalized.len(), 1);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "fs_search");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("find_name")
                );
                assert_eq!(
                    args.get("pattern").and_then(|value| value.as_str()),
                    Some("abcd")
                );
                assert_eq!(
                    args.get("root").and_then(|value| value.as_str()),
                    Some(root_path.as_str())
                );
            }
            other => panic!("expected fs_search action, got {other:?}"),
        }
    }

    #[test]
    fn system_basic_read_alias_is_normalized_to_read_range() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read",
                "path": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
            }),
        }];

        let normalized = normalize_system_basic_schema_aliases(actions);
        assert_eq!(normalized.len(), 1);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("read_range")
                );
                assert_eq!(
                    args.get("path").and_then(|value| value.as_str()),
                    Some("scripts/nl_tests/fixtures/device_local/docs/release_checklist.md")
                );
            }
            other => panic!("expected system_basic read_range action, got {other:?}"),
        }
    }

    #[test]
    fn system_basic_find_name_alias_is_normalized_to_find_path() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "find_name",
                "pattern": "missing.md",
                "max_results": 5,
            }),
        }];

        let normalized = normalize_system_basic_schema_aliases(actions);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("find_path")
                );
                assert_eq!(
                    args.get("name").and_then(|value| value.as_str()),
                    Some("missing.md")
                );
            }
            other => panic!("expected system_basic find_path action, got {other:?}"),
        }
    }

    #[test]
    fn system_basic_check_exists_alias_is_normalized_to_path_batch_facts() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "check_exists",
                "path": "plan/extra_missing_repair_probe.md",
            }),
        }];

        let normalized = normalize_system_basic_schema_aliases(actions);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("path_batch_facts")
                );
                assert_eq!(
                    args.get("paths").and_then(|value| value.as_array()),
                    Some(&vec![json!("plan/extra_missing_repair_probe.md")])
                );
                assert!(args.get("path").is_none());
            }
            other => panic!("expected system_basic path_batch_facts action, got {other:?}"),
        }
    }

    #[test]
    fn system_basic_check_exists_target_alias_keeps_batch_shape() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "check_exists",
                "target_path": "README.md",
            }),
        }];

        let normalized = normalize_system_basic_schema_aliases(actions);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("path_batch_facts")
                );
                assert_eq!(
                    args.get("paths").and_then(|value| value.as_array()),
                    Some(&vec![json!("README.md")])
                );
                assert!(args.get("target_path").is_none());
            }
            other => panic!("expected system_basic path_batch_facts action, got {other:?}"),
        }
    }

    #[test]
    fn missing_read_range_path_uses_route_locator_hint() {
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        route.output_contract.locator_kind = OutputLocatorKind::Filename;
        route.output_contract.locator_hint = "definitely_missing_system_basic_case.txt".to_string();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "find_path",
                    "name": "definitely_missing_system_basic_case.txt",
                }),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "read_range",
                    "mode": "head",
                    "n": 3,
                }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
        ];

        let normalized = fill_missing_read_range_path_from_route_locator(Some(&route), actions);
        match &normalized[1] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("path").and_then(|value| value.as_str()),
                    Some("definitely_missing_system_basic_case.txt")
                );
            }
            other => panic!("expected system_basic read_range action, got {other:?}"),
        }
    }

    #[test]
    fn system_basic_read_range_lines_alias_becomes_range_bounds() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "README.md",
                "lines": "1-3",
            }),
        }];

        let normalized = normalize_system_basic_schema_aliases(actions);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("read_range")
                );
                assert_eq!(
                    args.get("mode").and_then(|value| value.as_str()),
                    Some("range")
                );
                assert_eq!(
                    args.get("start_line").and_then(|value| value.as_u64()),
                    Some(1)
                );
                assert_eq!(
                    args.get("end_line").and_then(|value| value.as_u64()),
                    Some(3)
                );
                assert_eq!(args.get("n").and_then(|value| value.as_u64()), Some(3));
                assert!(args.get("lines").is_none());
            }
            other => panic!("expected system_basic read_range action, got {other:?}"),
        }
    }

    #[test]
    fn system_basic_read_alias_with_lines_becomes_range_bounds() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read",
                "path": "README.md",
                "lines": [2, 4],
            }),
        }];

        let normalized = normalize_system_basic_schema_aliases(actions);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("read_range")
                );
                assert_eq!(
                    args.get("mode").and_then(|value| value.as_str()),
                    Some("range")
                );
                assert_eq!(
                    args.get("start_line").and_then(|value| value.as_u64()),
                    Some(2)
                );
                assert_eq!(
                    args.get("end_line").and_then(|value| value.as_u64()),
                    Some(4)
                );
                assert_eq!(args.get("n").and_then(|value| value.as_u64()), Some(3));
                assert!(args.get("lines").is_none());
            }
            other => panic!("expected system_basic read_range action, got {other:?}"),
        }
    }

    #[test]
    fn system_basic_read_range_negative_bounds_becomes_tail_count() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "logs/app.log",
                "start_line": -12,
                "end_line": -1,
            }),
        }];

        let normalized = normalize_system_basic_schema_aliases(actions);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("mode").and_then(|value| value.as_str()),
                    Some("tail")
                );
                assert_eq!(args.get("n").and_then(|value| value.as_u64()), Some(12));
                assert!(args.get("start_line").is_none());
                assert!(args.get("end_line").is_none());
            }
            other => panic!("expected system_basic read_range action, got {other:?}"),
        }
    }

    #[test]
    fn system_basic_read_range_line_count_template_becomes_tail_count() {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "file_lines_count",
                    "path": "logs/model_io.log",
                }),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "read_range",
                    "path": "logs/model_io.log",
                    "start_line": "{{s1.result.line_count - 4}}",
                    "end_line": "{{s1.result.line_count}}",
                }),
            },
        ];

        let normalized = strip_file_lines_count_before_tail_read_range(
            normalize_system_basic_schema_aliases(actions),
        );
        assert_eq!(normalized.len(), 1);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("mode").and_then(|value| value.as_str()),
                    Some("tail")
                );
                assert_eq!(args.get("n").and_then(|value| value.as_u64()), Some(5));
                assert!(args.get("start_line").is_none());
                assert!(args.get("end_line").is_none());
            }
            other => panic!("expected system_basic read_range action, got {other:?}"),
        }
    }

    #[test]
    fn system_basic_list_dir_alias_is_normalized_to_inventory_dir() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "list_dir",
                "path": "scripts/nl_tests/fixtures/device_local/docs",
            }),
        }];

        let normalized = normalize_system_basic_schema_aliases(actions);
        assert_eq!(normalized.len(), 1);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("inventory_dir")
                );
                assert_eq!(
                    args.get("names_only").and_then(|value| value.as_bool()),
                    Some(true)
                );
            }
            other => panic!("expected system_basic inventory_dir action, got {other:?}"),
        }
    }

    #[test]
    fn system_basic_inventory_dir_dir_path_alias_becomes_path() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "inventory_dir",
                "dir_path": "scripts/nl_tests/fixtures/device_local/docs",
                "sort_by": "name",
            }),
        }];

        let normalized = normalize_system_basic_schema_aliases(actions);
        assert_eq!(normalized.len(), 1);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("inventory_dir")
                );
                assert_eq!(
                    args.get("path").and_then(|value| value.as_str()),
                    Some("scripts/nl_tests/fixtures/device_local/docs")
                );
                assert!(args.get("dir_path").is_none());
            }
            other => panic!("expected system_basic inventory_dir action, got {other:?}"),
        }
    }

    #[test]
    fn system_basic_count_dir_alias_is_normalized_to_count_inventory() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "count_dir",
                "directory_path": "document",
            }),
        }];

        let normalized = normalize_system_basic_schema_aliases(actions);
        assert_eq!(normalized.len(), 1);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("count_inventory")
                );
                assert_eq!(
                    args.get("path").and_then(|value| value.as_str()),
                    Some("document")
                );
                assert!(args.get("directory_path").is_none());
            }
            other => panic!("expected system_basic count_inventory action, got {other:?}"),
        }
    }

    #[test]
    fn system_basic_inventory_dir_extension_filter_implies_files_only() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "inventory_dir",
                "path": "document",
                "ext_filter": ".md",
                "names_only": true,
            }),
        }];

        let normalized = normalize_system_basic_schema_aliases(actions);
        assert_eq!(normalized.len(), 1);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("inventory_dir")
                );
                assert_eq!(
                    args.get("files_only").and_then(|value| value.as_bool()),
                    Some(true)
                );
            }
            other => panic!("expected system_basic inventory_dir action, got {other:?}"),
        }
    }

    #[test]
    fn system_basic_inventory_dir_normalizes_size_sort_aliases() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "inventory_dir",
                "path": "logs",
                "files_only": true,
                "sort_by": "size",
                "sort_order": "desc",
                "max_entries": 3,
            }),
        }];

        let normalized = normalize_system_basic_schema_aliases(actions);
        assert_eq!(normalized.len(), 1);
        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("inventory_dir")
                );
                assert_eq!(
                    args.get("sort_by").and_then(|value| value.as_str()),
                    Some("size_desc")
                );
            }
            other => panic!("expected system_basic inventory_dir action, got {other:?}"),
        }
    }

    #[test]
    fn hidden_entries_contract_forces_inventory_dir_include_hidden() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "inventory_dir",
                "path": ".",
                "names_only": true,
            }),
        }];
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Strict);
        route.output_contract.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;

        let normalized = normalize_planned_actions(
            &test_state(),
            Some(&route),
            &LoopState::new(1),
            "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子",
            None,
            actions,
        );

        assert!(matches!(
            &normalized[0],
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(|value| value.as_str()) == Some("inventory_dir")
                    && args.get("include_hidden").and_then(|value| value.as_bool()) == Some(true)
        ));
    }

    #[test]
    fn structured_scalar_compare_plan_appends_synthesize_answer() {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "extract_fields",
                    "path": "UI/package.json",
                    "field_paths": ["name"]
                }),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "extract_field",
                    "path": "crates/clawd/Cargo.toml",
                    "field_path": "package.name"
                }),
            },
        ];
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Scalar);
        route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
        route.resolved_intent =
            "UI/package.json 里的 name 和 crates/clawd/Cargo.toml 里的 package.name 一样吗？只回答一样或不一样"
                .to_string();

        let normalized = super::normalize_planned_actions(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            &route.resolved_intent,
            None,
            actions,
        );
        assert_eq!(normalized.len(), 3);
        assert!(matches!(
            &normalized[2],
            AgentAction::SynthesizeAnswer { evidence_refs }
                if evidence_refs
                    == &vec!["step_1".to_string(), "step_2".to_string()]
        ));
    }

    #[test]
    fn structured_scalar_compare_repairs_whole_file_read_plan() {
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Strict);
        route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
        let actions = vec![
            AgentAction::CallSkill {
                skill: "read_file".to_string(),
                args: serde_json::json!({ "path": "UI/package.json" }),
            },
            AgentAction::CallSkill {
                skill: "read_file".to_string(),
                args: serde_json::json!({ "path": "crates/clawd/Cargo.toml" }),
            },
        ];
        let normalized = super::normalize_planned_actions(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            "读取两个字段并比较",
            None,
            actions,
        );

        assert!(should_force_actionable_plan_repair(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            &normalized
        ));
        assert_eq!(
            plan_repair_reason(
                &test_state(),
                Some(&route),
                &LoopState::new(2),
                Some(&normalized)
            ),
            "structured_scalar_compare_requires_extract_fields"
        );
    }

    #[test]
    fn structured_scalar_compare_keeps_two_structured_extracts_for_strict_shape() {
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Strict);
        route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "extract_fields",
                    "path": "UI/package.json",
                    "field_paths": ["name"]
                }),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "extract_fields",
                    "path": "crates/clawd/Cargo.toml",
                    "field_paths": ["package.name"]
                }),
            },
        ];
        let normalized = super::normalize_planned_actions(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            "读取两个字段并比较",
            None,
            actions,
        );

        assert_eq!(normalized.len(), 3);
        assert!(matches!(
            normalized.last(),
            Some(AgentAction::SynthesizeAnswer { evidence_refs })
                if evidence_refs == &vec!["step_1".to_string(), "step_2".to_string()]
        ));
        assert!(!should_force_actionable_plan_repair(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            &normalized
        ));
    }

    #[test]
    fn structured_scalar_compare_accepts_two_directory_inventory_observations() {
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::OneSentence);
        route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "list_dir",
                    "path": "scripts/nl_tests/fixtures/device_local/docs"
                }),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "list_dir",
                    "path": "scripts/nl_tests/fixtures/device_local/logs"
                }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string(), "s1".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        let normalized = super::normalize_planned_actions(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            "先数 docs 直接子项数量，再数 logs 直接子项数量，最后一句中文说哪个更多",
            None,
            actions,
        );

        assert!(matches!(
            &normalized[0],
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(|value| value.as_str()) == Some("inventory_dir")
        ));
        assert!(matches!(
            &normalized[1],
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(|value| value.as_str()) == Some("inventory_dir")
        ));
        assert!(!should_force_actionable_plan_repair(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            &normalized
        ));
    }

    #[test]
    fn structured_scalar_compare_accepts_path_batch_facts_for_file_metadata() {
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Scalar);
        route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "path_batch_facts",
                "paths": ["Cargo.lock", "Cargo.toml"]
            }),
        }];
        let normalized = super::normalize_planned_actions(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            "比较 Cargo.lock 和 Cargo.toml 的大小",
            None,
            actions,
        );

        assert_eq!(normalized.len(), 2);
        assert!(matches!(
            normalized.last(),
            Some(AgentAction::SynthesizeAnswer { evidence_refs })
                if evidence_refs == &vec!["step_1".to_string()]
        ));
        assert!(!should_force_actionable_plan_repair(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            &normalized
        ));
    }

    #[test]
    fn structured_scalar_compare_accepts_compare_paths_for_file_metadata() {
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Scalar);
        route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "compare_paths",
                "left_path": "Cargo.lock",
                "right_path": "Cargo.toml"
            }),
        }];
        let normalized = super::normalize_planned_actions(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            "比较 Cargo.lock 和 Cargo.toml 的大小",
            None,
            actions,
        );

        assert_eq!(normalized.len(), 2);
        assert!(!should_force_actionable_plan_repair(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            &normalized
        ));
    }

    #[test]
    fn observation_only_terminal_answer_appends_synthesis_for_builtin_observation() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "inventory_dir",
                "path": "logs",
                "files_only": true,
                "sort_by": "mtime_desc",
                "max_entries": 2
            }),
        }];
        let mut route = route_result(RoutedMode::Act, false, OutputResponseShape::Free);
        route.output_contract.semantic_kind = OutputSemanticKind::None;

        let normalized = super::normalize_planned_actions(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            "列出 logs 最近修改的 2 个文件名，并判断更像运行日志还是测试残留",
            None,
            actions,
        );

        assert_eq!(normalized.len(), 3);
        assert!(matches!(
            &normalized[0],
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(|value| value.as_str()) == Some("inventory_dir")
        ));
        assert!(matches!(
            &normalized[1],
            AgentAction::SynthesizeAnswer { evidence_refs }
                if evidence_refs == &vec!["last_output".to_string()]
        ));
        assert!(matches!(
            &normalized[2],
            AgentAction::Respond { content } if content == "{{last_output}}"
        ));
    }

    #[test]
    fn observation_only_terminal_answer_keeps_file_names_runtime_finalizer() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "inventory_dir",
                "path": "logs",
                "files_only": true,
                "sort_by": "mtime_desc",
                "max_entries": 2
            }),
        }];
        let mut route = route_result(RoutedMode::Act, false, OutputResponseShape::Free);
        route.output_contract.semantic_kind = OutputSemanticKind::FileNames;

        let normalized = super::normalize_planned_actions(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            "只输出 logs 最近修改的 2 个文件名",
            None,
            actions,
        );

        assert_eq!(normalized.len(), 1);
        assert!(matches!(
            &normalized[0],
            AgentAction::CallSkill { skill, .. } if skill == "system_basic"
        ));
    }

    #[test]
    fn general_directory_inventory_clears_file_only_filter() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "inventory_dir",
                "path": "/workspace/docs",
                "files_only": true,
                "names_only": true
            }),
        }];
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Free);
        route.output_contract.semantic_kind = OutputSemanticKind::None;

        let normalized = super::normalize_planned_actions(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            "show the directory contents",
            None,
            actions,
        );

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(false));
                assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
                assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(false));
            }
            other => panic!("expected system_basic inventory_dir action, got {other:?}"),
        }
    }

    #[test]
    fn directory_lookup_inventory_clears_file_only_even_with_file_names_semantic() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "inventory_dir",
                "path": "/workspace/docs",
                "files_only": true,
                "names_only": true
            }),
        }];
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Strict);
        route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
        route.output_contract.delivery_intent = crate::OutputDeliveryIntent::DirectoryLookup;

        let normalized = super::normalize_planned_actions(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            "inspect the directory contents",
            None,
            actions,
        );

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(false));
                assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
                assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
            }
            other => panic!("expected system_basic inventory_dir action, got {other:?}"),
        }
    }

    #[test]
    fn file_names_directory_inventory_preserves_file_only_filter() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "inventory_dir",
                "path": "/workspace/docs",
                "files_only": true,
                "names_only": true
            }),
        }];
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Strict);
        route.output_contract.semantic_kind = OutputSemanticKind::FileNames;

        let normalized = super::normalize_planned_actions(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            "output file names only",
            None,
            actions,
        );

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
                assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
            }
            other => panic!("expected system_basic inventory_dir action, got {other:?}"),
        }
    }

    #[test]
    fn directory_names_contract_enforces_dirs_only_inventory() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "inventory_dir",
                "path": "/workspace",
                "files_only": true,
                "names_only": false
            }),
        }];
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Strict);
        route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;

        let normalized = super::normalize_planned_actions(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            "list top-level directory names only",
            None,
            actions,
        );

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(false));
                assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(true));
                assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
            }
            other => panic!("expected system_basic inventory_dir action, got {other:?}"),
        }
    }

    #[test]
    fn directory_names_contract_rewrites_filtered_list_dir_to_inventory() {
        let actions = vec![AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: serde_json::json!({
                "path": "/workspace",
                "dirs_only": true
            }),
        }];
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Strict);
        route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;

        let normalized = super::normalize_planned_actions(
            &test_state_with_enabled_skills(&["list_dir", "system_basic"]),
            Some(&route),
            &LoopState::new(2),
            "list top-level directory names only",
            None,
            actions,
        );

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("inventory_dir")
                );
                assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(false));
                assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(true));
                assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
                assert!(args.get("kind_filter").is_none());
            }
            other => panic!("expected system_basic inventory_dir action, got {other:?}"),
        }
    }

    #[test]
    fn list_dir_kind_filter_file_rewrites_to_inventory_file_names() {
        let actions = vec![AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: serde_json::json!({
                "path": "/workspace",
                "kind_filter": "file",
                "limit": 3
            }),
        }];

        let normalized = super::normalize_planned_actions(
            &test_state_with_enabled_skills(&["list_dir", "system_basic"]),
            None,
            &LoopState::new(2),
            "list file names",
            None,
            actions,
        );

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("inventory_dir")
                );
                assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
                assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
                assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
                assert_eq!(args.get("limit").and_then(Value::as_u64), Some(3));
            }
            other => panic!("expected system_basic inventory_dir action, got {other:?}"),
        }
    }

    #[test]
    fn file_paths_contract_rewrites_extension_inventory_to_fs_search() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "inventory_dir",
                "path": ".",
                "files_only": true,
                "names_only": true,
                "ext_filter": ".toml",
                "max_entries": 5
            }),
        }];
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Strict);
        route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;

        let normalized = super::normalize_planned_actions(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            "return five representative TOML file paths from the repository",
            None,
            actions,
        );

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "fs_search");
                assert_eq!(args.get("action").and_then(Value::as_str), Some("find_ext"));
                assert_eq!(args.get("root").and_then(Value::as_str), Some("."));
                assert_eq!(args.get("ext").and_then(Value::as_str), Some("toml"));
                assert_eq!(args.get("max_results").and_then(Value::as_u64), Some(5));
            }
            other => panic!("expected fs_search find_ext action, got {other:?}"),
        }
    }

    #[test]
    fn file_paths_contract_preserves_planned_synthesis_selection() {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "fs_search".to_string(),
                args: serde_json::json!({
                    "action": "find_name",
                    "root": ".",
                    "name": "*.toml",
                    "max_results": 50
                }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Strict);
        route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;

        let normalized = super::normalize_planned_actions(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            "return five representative TOML file paths from the repository",
            None,
            actions,
        );

        assert_eq!(normalized.len(), 3);
        assert!(matches!(
            &normalized[0],
            AgentAction::CallSkill { skill, .. } if skill == "fs_search"
        ));
        assert!(matches!(
            &normalized[1],
            AgentAction::SynthesizeAnswer { evidence_refs } if evidence_refs == &vec!["last_output".to_string()]
        ));
        assert!(matches!(
            &normalized[2],
            AgentAction::Respond { content } if content == "{{last_output}}"
        ));
    }

    #[test]
    fn file_paths_contract_normalizes_fs_search_glob_extension_args() {
        let root = TempDirGuard::new("fs_search_file_paths_contract");
        let root_path = root.path.display().to_string();
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::Strict);
        route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        let actions = vec![AgentAction::CallSkill {
            skill: "fs_search".to_string(),
            args: json!({
                "action": "find_name",
                "basename_pattern": "*.toml",
                "search_root": root_path,
                "type": "file",
                "max_results": 5
            }),
        }];

        let normalized = super::normalize_planned_actions(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            "return five representative TOML file paths from the repository",
            None,
            actions,
        );

        match &normalized[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "fs_search");
                assert_eq!(args.get("action").and_then(Value::as_str), Some("find_ext"));
                assert_eq!(
                    args.get("root").and_then(Value::as_str),
                    Some(root_path.as_str())
                );
                assert_eq!(args.get("ext").and_then(Value::as_str), Some("toml"));
                assert_eq!(args.get("max_results").and_then(Value::as_u64), Some(5));
            }
            other => panic!("expected normalized fs_search action, got {other:?}"),
        }
    }

    #[test]
    fn observation_only_terminal_answer_keeps_raw_command_runtime_finalizer() {
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({ "command": "pwd" }),
        }];
        let mut route = route_result(RoutedMode::Act, false, OutputResponseShape::Free);
        route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;

        let normalized = super::normalize_planned_actions(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            "执行 pwd，直接输出命令结果",
            None,
            actions,
        );

        assert_eq!(normalized.len(), 1);
        assert!(matches!(
            &normalized[0],
            AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
        ));
    }

    #[test]
    fn workspace_summary_keeps_requested_structured_field_evidence() {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "list_dir".to_string(),
                args: serde_json::json!({ "path": "." }),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "extract_field",
                    "path": "UI/package.json",
                    "field_path": "name"
                }),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "read_range",
                    "path": "README.md",
                    "mode": "head",
                    "n": 10
                }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec![
                    "step_1".to_string(),
                    "step_2".to_string(),
                    "step_3".to_string(),
                ],
            },
        ];
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::OneSentence);
        route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
        route.resolved_intent =
            "先看顶层目录，再读 UI/package.json 的 name，最后一句话判断 UI 定位".to_string();

        let normalized = super::normalize_planned_actions(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            &route.resolved_intent,
            None,
            actions,
        );
        assert!(normalized.iter().any(|action| matches!(
            action,
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(|value| value.as_str()) == Some("extract_field")
        )));
    }

    #[test]
    fn workspace_summary_with_scope_prunes_sibling_evidence() {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "list_dir".to_string(),
                args: serde_json::json!({ "path": "UI" }),
            },
            AgentAction::CallSkill {
                skill: "list_dir".to_string(),
                args: serde_json::json!({ "path": "pi_app" }),
            },
        ];
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
        route.output_contract.locator_hint = "UI".to_string();
        route.resolved_intent = "Summarize only the UI part of this repository".to_string();

        let pruned =
            super::prune_unscoped_workspace_summary_evidence_for_scope(Some(&route), actions);
        assert_eq!(pruned.len(), 1);
        match &pruned[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "list_dir");
                assert_eq!(
                    args.get("path").and_then(|value| value.as_str()),
                    Some("UI")
                );
            }
            other => panic!("expected scoped UI list_dir action, got {other:?}"),
        }
    }

    #[test]
    fn unscoped_workspace_evidence_appends_synthesis_after_existing_text_read_plan() {
        let root = TempDirGuard::new("workspace_text_evidence_existing");
        fs::write(root.path.join("README.md"), "# RustClaw").expect("write README");
        let mut state = test_state();
        state.skill_rt.workspace_root = root.path.clone();
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = OutputSemanticKind::None;
        route.output_contract.locator_hint.clear();
        let actions = vec![AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({"path":"README.md"}),
        }];

        let normalized = super::normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(1),
            &route.resolved_intent,
            None,
            actions,
        );
        assert_eq!(normalized.len(), 2);
        assert!(matches!(
            &normalized[0],
            AgentAction::CallSkill { skill, .. } if skill == "read_file"
        ));
        assert!(matches!(
            &normalized[1],
            AgentAction::SynthesizeAnswer { evidence_refs }
                if evidence_refs == &vec!["step_1".to_string()]
        ));
    }

    #[test]
    fn unscoped_workspace_text_answer_strips_unrequested_file_artifact_plan() {
        let root = TempDirGuard::new("workspace_text_evidence_no_artifact");
        fs::write(
            root.path.join("README.md"),
            "# RustClaw\n\nUse the documented installer",
        )
        .expect("write README");
        let mut state = test_state();
        state.skill_rt.workspace_root = root.path.clone();
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = OutputSemanticKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.delivery_required = false;
        route.wants_file_delivery = false;
        route.resolved_intent = "Write a short RustClaw setup note".to_string();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "read_file".to_string(),
                args: json!({"path":"Cargo.toml"}),
            },
            AgentAction::CallSkill {
                skill: "write_file".to_string(),
                args: json!({
                    "path":"document/SETUP_NOTE.md",
                    "content":"# RustClaw Setup Note\n"
                }),
            },
            AgentAction::Respond {
                content: "FILE:/home/guagua/rustclaw/document/SETUP_NOTE.md".to_string(),
            },
        ];

        let normalized = super::normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(1),
            &route.resolved_intent,
            None,
            actions,
        );
        assert!(normalized.iter().all(|action| {
            !matches!(
                action,
                AgentAction::CallSkill { skill, .. } if skill == "write_file"
            )
        }));
        assert!(normalized
            .iter()
            .all(|action| !matches!(action, AgentAction::Respond { .. })));
        assert!(!normalized.iter().any(|action| {
            matches!(action, AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(|value| value.as_str()) == Some("read_range"))
        }));
        assert!(matches!(
            normalized.last(),
            Some(AgentAction::SynthesizeAnswer { evidence_refs })
                if evidence_refs == &vec!["step_1".to_string()]
        ));
    }

    #[test]
    fn active_execution_recipe_keeps_workspace_file_mutation_plan() {
        let root = TempDirGuard::new("workspace_text_evidence_recipe_mutation");
        fs::write(root.path.join("README.md"), "# RustClaw").expect("write README");
        let mut state = test_state();
        state.skill_rt.workspace_root = root.path.clone();
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = OutputSemanticKind::None;
        route.output_contract.locator_hint.clear();
        let mut loop_state = LoopState::new(1);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            ..Default::default()
        };
        let actions = vec![AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: json!({
                "path":"document/SETUP_NOTE.md",
                "content":"# RustClaw Setup Note\n"
            }),
        }];

        let normalized = super::normalize_planned_actions(
            &state,
            Some(&route),
            &loop_state,
            &route.resolved_intent,
            None,
            actions,
        );
        assert!(normalized.iter().any(|action| {
            matches!(
                action,
                AgentAction::CallSkill { skill, .. } if skill == "write_file"
            )
        }));
    }

    #[test]
    fn explicit_workspace_file_locator_keeps_requested_file_mutation_plan() {
        let root = TempDirGuard::new("workspace_text_evidence_requested_mutation");
        let mut state = test_state();
        state.skill_rt.workspace_root = root.path.clone();
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::OneSentence);
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = OutputSemanticKind::None;
        route.output_contract.locator_hint = "plan/p2_expand_test.md".to_string();
        route.resolved_intent = "Create plan/p2_expand_test.md and write p2 hello".to_string();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "make_dir".to_string(),
                args: json!({"path":"plan"}),
            },
            AgentAction::CallSkill {
                skill: "write_file".to_string(),
                args: json!({
                    "path":"plan/p2_expand_test.md",
                    "content":"p2 hello"
                }),
            },
            AgentAction::Respond {
                content: "created".to_string(),
            },
        ];

        let normalized = super::normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(1),
            &route.resolved_intent,
            None,
            actions,
        );
        assert!(normalized.iter().any(|action| {
            matches!(
                action,
                AgentAction::CallSkill { skill, .. } if skill == "make_dir"
            )
        }));
        assert!(normalized.iter().any(|action| {
            matches!(
                action,
                AgentAction::CallSkill { skill, .. } if skill == "write_file"
            )
        }));
    }

    #[test]
    fn free_route_strips_terminal_discussion_after_runner_skill() {
        let state = test_state();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "crypto".to_string(),
                args: serde_json::json!({ "action": "quote", "symbol": "BTCUSDT" }),
            },
            AgentAction::Respond {
                content: "下面是我帮你整理后的结果。".to_string(),
            },
        ];

        let stripped = strip_terminal_discussion_for_direct_skill_passthrough(
            &state,
            Some(&route_result(
                RoutedMode::Act,
                false,
                OutputResponseShape::Free,
            )),
            actions,
        );
        assert_eq!(stripped.len(), 1);
        assert!(matches!(
            &stripped[0],
            AgentAction::CallSkill { skill, .. } if skill == "crypto"
        ));
    }

    #[test]
    fn process_basic_port_list_keeps_terminal_discussion_followup() {
        let state = test_state();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "process_basic".to_string(),
                args: serde_json::json!({ "action": "port_list" }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        let kept = strip_terminal_discussion_for_direct_skill_passthrough(
            &state,
            Some(&route_result(
                RoutedMode::ChatAct,
                false,
                OutputResponseShape::Free,
            )),
            actions.clone(),
        );
        assert_eq!(kept.len(), 3);
        assert!(matches!(
            &kept[0],
            AgentAction::CallSkill { skill, args }
                if skill == "process_basic"
                    && args.get("action").and_then(|value| value.as_str()) == Some("port_list")
        ));
        assert!(matches!(
            &kept[1],
            AgentAction::SynthesizeAnswer { evidence_refs }
                if evidence_refs == &vec!["last_output".to_string()]
        ));
        assert!(matches!(
            &kept[2],
            AgentAction::Respond { content } if content == "{{last_output}}"
        ));
    }

    #[test]
    fn direct_passthrough_keeps_mixed_placeholder_terminal_respond() {
        let state = test_state();
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: serde_json::json!({ "command": "pwd" }),
            },
            AgentAction::Respond {
                content: "{{last_output}}\n\nworkspace ready".to_string(),
            },
        ];

        let kept =
            strip_terminal_discussion_for_direct_skill_passthrough(&state, Some(&route), actions);
        assert_eq!(kept.len(), 2);
        assert!(matches!(
            &kept[0],
            AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
        ));
        assert!(matches!(
            &kept[1],
            AgentAction::Respond { content } if content.contains("workspace ready")
        ));
    }

    #[test]
    fn runner_skill_only_plan_does_not_require_terminal_respond() {
        let loop_state = LoopState::new(2);
        let actions = vec![AgentAction::CallSkill {
            skill: "crypto".to_string(),
            args: serde_json::json!({ "action": "quote", "symbol": "BTCUSDT" }),
        }];
        assert!(!should_force_plan_repair(
            Some(&route_result(
                RoutedMode::Act,
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn chat_act_route_repairs_observation_only_plan_before_any_observation() {
        let loop_state = LoopState::new(2);
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({ "command": "ls -l Cargo.toml Cargo.lock" }),
        }];
        assert!(should_force_plan_repair(
            Some(&route_result(
                RoutedMode::ChatAct,
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn chat_act_route_repairs_observation_plus_unavailable_followup_plan() {
        let state = test_state_with_enabled_skills(&["run_cmd"]);
        let loop_state = LoopState::new(2);
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: serde_json::json!({ "command": "ls -l Cargo.toml Cargo.lock" }),
            },
            AgentAction::CallSkill {
                skill: "formatter".to_string(),
                args: serde_json::json!({ "text": "explain {{last_output}}" }),
            },
        ];
        let route = route_result(RoutedMode::ChatAct, false, OutputResponseShape::Free);
        assert!(should_force_actionable_plan_repair(
            &state,
            Some(&route),
            &loop_state,
            &actions
        ));
        assert_eq!(
            plan_repair_reason(&state, Some(&route), &loop_state, Some(&actions)),
            "unavailable_skill_requires_replan"
        );
    }

    #[test]
    fn chat_act_route_keeps_observation_plus_synthesize_followup_plan() {
        let loop_state = LoopState::new(2);
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: serde_json::json!({ "command": "ls -l Cargo.toml Cargo.lock" }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
        ];
        assert!(!should_force_plan_repair(
            Some(&route_result(
                RoutedMode::ChatAct,
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn chat_act_route_keeps_health_check_observation_only_plan() {
        let loop_state = LoopState::new(2);
        let actions = vec![AgentAction::CallSkill {
            skill: "health_check".to_string(),
            args: serde_json::json!({}),
        }];
        assert!(!should_force_plan_repair(
            Some(&route_result(
                RoutedMode::ChatAct,
                false,
                OutputResponseShape::OneSentence,
            )),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn non_scalar_route_still_repairs_after_prior_observation_when_delivery_is_empty() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({ "command": "ls -l Cargo.toml Cargo.lock" }),
        }];
        assert!(should_force_plan_repair(
            Some(&route_result(
                RoutedMode::ChatAct,
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn scalar_route_keeps_single_observation_plan_without_followup() {
        let loop_state = LoopState::new(2);
        let actions = vec![AgentAction::CallSkill {
            skill: "git_basic".to_string(),
            args: serde_json::json!({ "action": "current_branch" }),
        }];
        assert!(!should_force_plan_repair(
            Some(&route_result(
                RoutedMode::Act,
                false,
                OutputResponseShape::Scalar,
            )),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn raw_command_output_route_keeps_single_run_cmd_plan_without_followup() {
        let loop_state = LoopState::new(2);
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({ "command": "ls", "cwd": "/tmp/rustclaw-workspace" }),
        }];
        let mut route = route_result(RoutedMode::Act, false, OutputResponseShape::Free);
        route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
        assert!(!should_force_plan_repair(
            Some(&route),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn file_delivery_route_allows_plain_not_found_terminal_reply() {
        let loop_state = LoopState::new(2);
        let actions = vec![AgentAction::Respond {
            content: "未找到该文件。".to_string(),
        }];
        assert!(!should_force_plan_repair(
            Some(&delivery_route_result()),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn ops_recipe_apply_phase_without_mutation_forces_plan_repair() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            ..Default::default()
        };
        let actions = vec![AgentAction::CallSkill {
            skill: "http_basic".to_string(),
            args: serde_json::json!({ "action": "get", "url": "http://127.0.0.1:60703/" }),
        }];
        assert!(should_force_plan_repair(
            Some(&route_result(
                RoutedMode::Act,
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn ops_recipe_apply_phase_without_mutation_uses_specific_repair_reason() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            ..Default::default()
        };
        let actions = vec![AgentAction::CallSkill {
            skill: "http_basic".to_string(),
            args: serde_json::json!({ "action": "get", "url": "http://127.0.0.1:60703/" }),
        }];
        assert_eq!(
            repair_reason(
                Some(&route_result(
                    RoutedMode::Act,
                    false,
                    OutputResponseShape::Free,
                )),
                &loop_state,
                Some(&actions),
            ),
            "ops_closed_loop_apply_requires_mutation"
        );
    }

    #[test]
    fn ops_recipe_apply_phase_with_mutation_keeps_plan() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            ..Default::default()
        };
        let actions = vec![
            AgentAction::CallSkill {
                skill: "read_file".to_string(),
                args: serde_json::json!({ "path": "document/index.html" }),
            },
            AgentAction::CallSkill {
                skill: "write_file".to_string(),
                args: serde_json::json!({ "path": "document/index.html", "content": "ops-repair-ok\n" }),
            },
        ];
        assert!(!should_force_plan_repair(
            Some(&route_result(
                RoutedMode::Act,
                false,
                OutputResponseShape::Scalar,
            )),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn config_change_profile_without_post_change_validation_forces_repair() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::ConfigChange,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            ..Default::default()
        };
        let actions = vec![
            AgentAction::CallSkill {
                skill: "read_file".to_string(),
                args: serde_json::json!({ "path": "configs/config.toml" }),
            },
            AgentAction::CallSkill {
                skill: "write_file".to_string(),
                args: serde_json::json!({ "path": "configs/config.toml", "content": "[tools]\nallow_sudo=false\n" }),
            },
        ];
        assert!(should_force_plan_repair(
            Some(&route_result(
                RoutedMode::Act,
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            &actions,
        ));
        assert_eq!(
            repair_reason(
                Some(&route_result(
                    RoutedMode::Act,
                    false,
                    OutputResponseShape::Free,
                )),
                &loop_state,
                Some(&actions),
            ),
            "config_change_requires_post_change_validation"
        );
    }

    #[test]
    fn skill_authoring_profile_requires_integration_validation_not_readback_only() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::SkillAuthoring,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            ..Default::default()
        };
        let actions = vec![
            AgentAction::CallSkill {
                skill: "read_file".to_string(),
                args: serde_json::json!({ "path": "crates/skills/foo/INTERFACE.md" }),
            },
            AgentAction::CallSkill {
                skill: "write_file".to_string(),
                args: serde_json::json!({ "path": "crates/skills/foo/INTERFACE.md", "content": "# Foo\n" }),
            },
            AgentAction::CallSkill {
                skill: "http_basic".to_string(),
                args: serde_json::json!({ "action": "get", "url": "http://127.0.0.1:62078/" }),
            },
        ];
        assert!(should_force_plan_repair(
            Some(&route_result(
                RoutedMode::Act,
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            &actions,
        ));
        assert_eq!(
            repair_reason(
                Some(&route_result(
                    RoutedMode::Act,
                    false,
                    OutputResponseShape::Free,
                )),
                &loop_state,
                Some(&actions),
            ),
            "skill_authoring_requires_integration_validation"
        );
    }

    #[test]
    fn code_change_profile_requires_verification_not_readback_only() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            ..Default::default()
        };
        let actions = vec![
            AgentAction::CallSkill {
                skill: "read_file".to_string(),
                args: serde_json::json!({ "path": "crates/clawd/src/main.rs" }),
            },
            AgentAction::CallSkill {
                skill: "write_file".to_string(),
                args: serde_json::json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
            },
            AgentAction::CallSkill {
                skill: "read_file".to_string(),
                args: serde_json::json!({ "path": "crates/clawd/src/main.rs" }),
            },
        ];
        assert!(should_force_plan_repair(
            Some(&route_result(
                RoutedMode::Act,
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            &actions,
        ));
        assert_eq!(
            repair_reason(
                Some(&route_result(
                    RoutedMode::Act,
                    false,
                    OutputResponseShape::Free,
                )),
                &loop_state,
                Some(&actions),
            ),
            "code_change_requires_verification"
        );
    }

    #[test]
    fn code_change_profile_with_structured_cargo_check_keeps_plan() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            ..Default::default()
        };
        let actions = vec![
            AgentAction::CallSkill {
                skill: "write_file".to_string(),
                args: serde_json::json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
            },
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: serde_json::json!({
                    "command": "cargo check -p clawd",
                    "_clawd_validation": {
                        "profile": "code_change",
                        "validator_type": "build",
                        "validated_target": "clawd"
                    }
                }),
            },
        ];
        assert!(!should_force_plan_repair(
            Some(&route_result(
                RoutedMode::Act,
                false,
                OutputResponseShape::Scalar,
            )),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn code_change_profile_with_unstructured_cargo_check_forces_repair() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            ..Default::default()
        };
        let actions = vec![
            AgentAction::CallSkill {
                skill: "write_file".to_string(),
                args: serde_json::json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
            },
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: serde_json::json!({ "command": "cargo check -p clawd" }),
            },
        ];
        assert!(should_force_plan_repair(
            Some(&route_result(
                RoutedMode::Act,
                false,
                OutputResponseShape::Scalar,
            )),
            &loop_state,
            &actions,
        ));
        assert_eq!(
            repair_reason(
                Some(&route_result(
                    RoutedMode::Act,
                    false,
                    OutputResponseShape::Scalar,
                )),
                &loop_state,
                Some(&actions),
            ),
            "code_change_requires_verification"
        );
    }

    #[test]
    fn current_repo_scope_rejects_external_absolute_path() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            ..Default::default()
        };
        let actions = vec![
            AgentAction::CallSkill {
                skill: "write_file".to_string(),
                args: serde_json::json!({ "path": "/opt/other-project/main.rs", "content": "fn main() {}\n" }),
            },
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: serde_json::json!({
                    "command": "cargo check -p clawd",
                    "_clawd_validation": {
                        "profile": "code_change",
                        "validator_type": "build",
                        "validated_target": "tools/demo"
                    }
                }),
            },
        ];
        assert!(should_force_plan_repair(
            Some(&route_result(
                RoutedMode::Act,
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            &actions,
        ));
        assert_eq!(
            repair_reason(
                Some(&route_result(
                    RoutedMode::Act,
                    false,
                    OutputResponseShape::Free,
                )),
                &loop_state,
                Some(&actions),
            ),
            "current_repo_scope_rejects_external_target"
        );
    }

    #[test]
    fn external_workspace_scope_requires_explicit_external_target() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            ..Default::default()
        };
        let actions = vec![
            AgentAction::CallSkill {
                skill: "write_file".to_string(),
                args: serde_json::json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
            },
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: serde_json::json!({ "command": "cargo check -p clawd" }),
            },
        ];
        assert!(should_force_plan_repair(
            Some(&route_result(
                RoutedMode::Act,
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            &actions,
        ));
        assert_eq!(
            repair_reason(
                Some(&route_result(
                    RoutedMode::Act,
                    false,
                    OutputResponseShape::Free,
                )),
                &loop_state,
                Some(&actions),
            ),
            "external_workspace_requires_explicit_target"
        );
    }

    #[test]
    fn greenfield_scope_requires_creation_step_before_validation() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            ..Default::default()
        };
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({ "command": "cargo check -p clawd" }),
        }];
        assert!(should_force_plan_repair(
            Some(&route_result(
                RoutedMode::Act,
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            &actions,
        ));
        assert_eq!(
            repair_reason(
                Some(&route_result(
                    RoutedMode::Act,
                    false,
                    OutputResponseShape::Free,
                )),
                &loop_state,
                Some(&actions),
            ),
            "greenfield_requires_artifact_creation"
        );
    }

    #[test]
    fn greenfield_scope_with_make_dir_and_write_file_keeps_plan() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            ..Default::default()
        };
        let actions = vec![
            AgentAction::CallSkill {
                skill: "make_dir".to_string(),
                args: serde_json::json!({ "path": "tools/demo" }),
            },
            AgentAction::CallSkill {
                skill: "write_file".to_string(),
                args: serde_json::json!({ "path": "tools/demo/main.rs", "content": "fn main() {}\n" }),
            },
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: serde_json::json!({ "command": "cargo check -p clawd" }),
            },
        ];
        assert!(!should_force_plan_repair(
            Some(&route_result(
                RoutedMode::Act,
                false,
                OutputResponseShape::Scalar,
            )),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn external_workspace_scope_persists_across_rounds_without_repeating_path() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
            phase: crate::execution_recipe::ExecutionRecipePhase::Validate,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_external_target: true,
            ..Default::default()
        };
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({
                "command": "cargo check",
                "_clawd_validation": {
                    "profile": "code_change",
                    "validator_type": "build",
                    "validated_target": "external_workspace"
                }
            }),
        }];
        assert!(!should_force_plan_repair(
            Some(&route_result(
                RoutedMode::Act,
                false,
                OutputResponseShape::Scalar,
            )),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn greenfield_scope_persists_creation_across_rounds() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
            phase: crate::execution_recipe::ExecutionRecipePhase::Validate,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_greenfield_creation: true,
            ..Default::default()
        };
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({
                "command": "cargo check -p clawd",
                "_clawd_validation": {
                    "profile": "code_change",
                    "validator_type": "build",
                    "validated_target": "greenfield_project"
                }
            }),
        }];
        assert!(!should_force_plan_repair(
            Some(&route_result(
                RoutedMode::Act,
                false,
                OutputResponseShape::Scalar,
            )),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn content_evidence_route_allows_respond_only_after_prior_observation() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        let actions = vec![AgentAction::Respond {
            content: "grounded final answer".to_string(),
        }];
        assert!(!should_force_plan_repair(
            Some(&route_result(
                RoutedMode::ChatAct,
                true,
                OutputResponseShape::Free,
            )),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn extracts_minimax_call_skill_markup_into_step_values() {
        let raw = r#"<minimax:tool_call>
<invoke name="call_skill">
<parameter name="skill">list_dir</parameter>
<parameter name="args">{"path": "/tmp"}</parameter>
</invoke>
</minimax:tool_call>"#;
        assert_eq!(
            super::extract_minimax_tool_call_steps(raw),
            vec![json!({
                "type": "call_skill",
                "skill": "list_dir",
                "args": { "path": "/tmp" }
            })]
        );
    }

    #[test]
    fn extracts_minimax_direct_skill_invoke_markup_into_step_values() {
        let raw = r#"<minimax:tool_call>
<invoke name="fs_search">
<parameter name="args">{"action":"find_name","pattern":"README"}</parameter>
</invoke>
</minimax:tool_call>"#;
        assert_eq!(
            super::extract_minimax_tool_call_steps(raw),
            vec![json!({
                "type": "call_skill",
                "skill": "fs_search",
                "args": { "action": "find_name", "pattern": "README" }
            })]
        );
    }

    // ---------- inject_synthesize_answer_for_bare_placeholder_respond ----------
    // 见函数 doc：runtime 兜底，把 minimax 偶发吐出的裸 placeholder respond 注入
    // 一个 synthesize_answer 节点，关掉裸 placeholder 导致的死循环。

    #[test]
    fn detects_bare_last_output_placeholder_variants() {
        assert!(is_bare_last_output_placeholder("{{last_output}}"));
        assert!(is_bare_last_output_placeholder("  {{ last_output }}  "));
        assert!(is_bare_last_output_placeholder("{{last_output.hostname}}"));
        assert!(is_bare_last_output_placeholder("{{last_output.foo.bar}}"));
        assert!(is_bare_last_output_placeholder("{{LAST_OUTPUT}}"));
        assert!(is_bare_last_output_placeholder("{{last_output[\"x\"]}}"));
    }

    #[test]
    fn rejects_non_bare_placeholder_content() {
        assert!(!is_bare_last_output_placeholder(
            "hostname is {{last_output}}"
        ));
        assert!(!is_bare_last_output_placeholder("当前用户是 root"));
        assert!(!is_bare_last_output_placeholder(""));
        assert!(!is_bare_last_output_placeholder("{{other}}"));
        assert!(!is_bare_last_output_placeholder("{{lastoutput}}"));
        // last_output 后接非 . / [ 的字符不算同一占位
        assert!(!is_bare_last_output_placeholder("{{last_output_extra}}"));
    }

    #[test]
    fn injects_synthesize_answer_when_respond_is_bare_placeholder() {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({ "command": "whoami" }),
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];
        let out = inject_synthesize_answer_for_bare_placeholder_respond(
            actions,
            "只输出当前用户名，不要解释",
        );
        assert_eq!(out.len(), 3, "should insert exactly one synth step");
        assert!(matches!(
            &out[0],
            AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
        ));
        match &out[1] {
            AgentAction::SynthesizeAnswer { evidence_refs } => {
                assert_eq!(
                    evidence_refs,
                    &vec!["last_output".to_string()],
                    "synthesize step should point at last_output by default"
                );
            }
            _ => panic!("expected synthesize_answer at index 1, got {:?}", out[1]),
        }
        assert!(matches!(
            &out[2],
            AgentAction::Respond { content } if content == "{{last_output}}"
        ));
    }

    fn actions_as_json(actions: &[AgentAction]) -> serde_json::Value {
        serde_json::to_value(actions).expect("serialize")
    }

    #[test]
    fn injection_is_idempotent_when_synthesize_already_precedes_respond() {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({ "command": "whoami" }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];
        let before = actions_as_json(&actions);
        let out = inject_synthesize_answer_for_bare_placeholder_respond(actions, "x");
        assert_eq!(
            actions_as_json(&out),
            before,
            "should not re-inject when synthesize_answer already precedes respond"
        );
    }

    #[test]
    fn terminal_synthesis_placeholder_respond_uses_last_output() {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "read_file".to_string(),
                args: json!({ "path": "README.md" }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["step_1".to_string()],
            },
            AgentAction::Respond {
                content: "{{synthesized}}".to_string(),
            },
        ];

        let out = rewrite_terminal_synthesis_placeholder_respond(actions);
        assert_eq!(out.len(), 3);
        assert!(matches!(
            &out[2],
            AgentAction::Respond { content } if content == "{{last_output}}"
        ));
    }

    #[test]
    fn injection_no_op_when_respond_content_is_concrete() {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({ "command": "whoami" }),
            },
            AgentAction::Respond {
                content: "guagua".to_string(),
            },
        ];
        let before = actions_as_json(&actions);
        let out = inject_synthesize_answer_for_bare_placeholder_respond(actions, "x");
        assert_eq!(actions_as_json(&out), before);
    }

    #[test]
    fn injection_no_op_when_only_one_action() {
        let actions = vec![AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        }];
        let before = actions_as_json(&actions);
        let out = inject_synthesize_answer_for_bare_placeholder_respond(actions, "x");
        assert_eq!(
            actions_as_json(&out),
            before,
            "no observation step before respond → cannot meaningfully inject"
        );
    }

    #[test]
    fn injection_no_op_when_last_action_is_not_respond() {
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({ "command": "ls" }),
        }];
        let before = actions_as_json(&actions);
        let out = inject_synthesize_answer_for_bare_placeholder_respond(actions, "x");
        assert_eq!(actions_as_json(&out), before);
    }

    #[test]
    fn normalizer_drops_pre_observation_synthesize_when_concrete_respond_exists() {
        let state = test_state();
        let loop_state = LoopState::new(2);
        let route = route_result(RoutedMode::Chat, false, OutputResponseShape::Free);
        let actions = vec![
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
            AgentAction::Respond {
                content: "早出晚归皆是梦，\n一杯咖啡换人间。".to_string(),
            },
        ];

        let normalized = normalize_planned_actions(
            &state,
            Some(&route),
            &loop_state,
            "写一首两句的打工人短诗",
            None,
            actions,
        );

        assert_eq!(normalized.len(), 1);
        assert!(matches!(
            &normalized[0],
            AgentAction::Respond { content }
                if content == "早出晚归皆是梦，\n一杯咖啡换人间。"
        ));
    }

    #[test]
    fn normalizer_keeps_observation_backed_synthesize_before_respond() {
        let state = test_state();
        let loop_state = LoopState::new(2);
        let route = route_result(RoutedMode::Act, false, OutputResponseShape::Free);
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({ "command": "pwd" }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];
        let before = actions_as_json(&actions);

        let normalized =
            normalize_planned_actions(&state, Some(&route), &loop_state, "执行 pwd", None, actions);

        assert_eq!(actions_as_json(&normalized), before);
    }

    /// §F1：`has_pre_observation_structured_output_shape` 结构形态检测覆盖。
    #[test]
    fn pre_observation_structured_output_shape_recognizes_listing_shapes() {
        // 真实 adv08 复现：list_dir 还没跑，respond 编出 5 行 numbered 列表 + 路径。
        let adv08 = "prompts 目录前 5 个文件名：\n1. prompts/skills\n2. prompts/agents\n3. prompts/system\n4. prompts/user\n5. prompts/layers";
        assert!(has_pre_observation_structured_output_shape(adv08));

        // 多行 + 文件后缀，但没编号。
        let multi_paths = "Cargo.toml\nCargo.lock\nREADME.md\nLICENSE";
        assert!(has_pre_observation_structured_output_shape(multi_paths));

        // 结构化字段标签。
        assert!(has_pre_observation_structured_output_shape(
            "result: 42\ncount: 3"
        ));

        // 一句正常文本 → 不命中。
        assert!(!has_pre_observation_structured_output_shape(
            "好的，正在查询，请稍候。"
        ));
        // {{last_output}} 占位符 → 不命中（应由 synthesize 注入兜底处理）。
        assert!(!has_pre_observation_structured_output_shape(
            "{{last_output}}"
        ));
        // 只有一行短回复 → 不命中。
        assert!(!has_pre_observation_structured_output_shape("yes"));
    }

    /// §F1：rewrite 触发条件 —— round 1 + 上一步 CallSkill + Respond 含枚举。
    #[test]
    fn rewrite_pre_observation_rewrites_concrete_respond_after_call_skill() {
        let loop_state = LoopState::new(2);
        assert!(loop_state.executed_step_results.is_empty());
        assert!(loop_state.last_output.is_none());

        let actions = vec![
            AgentAction::CallSkill {
                skill: "list_dir".to_string(),
                args: json!({"path": "/home/guagua/rustclaw/prompts"}),
            },
            AgentAction::Respond {
                content: "prompts 目录前 5 个文件名：\n1. prompts/skills\n2. prompts/agents\n3. prompts/system\n4. prompts/user\n5. prompts/layers".to_string(),
            },
        ];
        let out =
            rewrite_pre_observation_concrete_respond_to_placeholder(None, &loop_state, actions);
        match out.last().expect("should have a last action") {
            AgentAction::Respond { content } => {
                assert_eq!(
                    content, "{{last_output}}",
                    "concrete content must be replaced with placeholder"
                );
            }
            other => panic!("last action should remain Respond, got: {:?}", other),
        }
    }

    #[test]
    fn rewrite_pre_observation_uses_output_contract_without_shape_matching() {
        let loop_state = LoopState::new(2);
        let route = route_result(RoutedMode::Act, true, OutputResponseShape::Free);
        let actions = vec![
            AgentAction::CallSkill {
                skill: "service_control".to_string(),
                args: json!({"action": "status", "service": "rustclaw"}),
            },
            AgentAction::Respond {
                content: "服务运行正常，可以继续使用。".to_string(),
            },
        ];

        let out = rewrite_pre_observation_concrete_respond_to_placeholder(
            Some(&route),
            &loop_state,
            actions,
        );

        assert!(matches!(
            out.last(),
            Some(AgentAction::Respond { content }) if content == "{{last_output}}"
        ));
    }

    /// §F1：执行过任何 step 后不再触发（避免误改 round 2+ 的合法 grounded respond）。
    #[test]
    fn rewrite_pre_observation_no_op_after_any_step_executed() {
        use crate::executor::{StepExecutionResult, StepExecutionStatus};
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "s1".to_string(),
            skill: "list_dir".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some("foo\nbar".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        loop_state.last_output = Some("foo\nbar".to_string());

        let actions = vec![
            AgentAction::CallSkill {
                skill: "list_dir".to_string(),
                args: json!({"path": "/x"}),
            },
            AgentAction::Respond {
                content: "1. foo\n2. bar".to_string(),
            },
        ];
        let before = actions.clone();
        let after =
            rewrite_pre_observation_concrete_respond_to_placeholder(None, &loop_state, actions);
        assert_eq!(actions_as_json(&before), actions_as_json(&after));
    }

    /// §F1：Respond 内容是合法占位符或短确认时不触发。
    #[test]
    fn rewrite_pre_observation_no_op_for_placeholder_or_short_ack() {
        let loop_state = LoopState::new(2);
        for content in ["{{last_output}}", "好的", "稍候，正在执行"] {
            let actions = vec![
                AgentAction::CallSkill {
                    skill: "run_cmd".to_string(),
                    args: json!({"command": "ls"}),
                },
                AgentAction::Respond {
                    content: content.to_string(),
                },
            ];
            let before = actions.clone();
            let after =
                rewrite_pre_observation_concrete_respond_to_placeholder(None, &loop_state, actions);
            assert_eq!(
                actions_as_json(&before),
                actions_as_json(&after),
                "should not rewrite for content={:?}",
                content
            );
        }
    }

    #[test]
    fn rewrite_terminal_placeholder_respond_inserts_synthesize_answer() {
        let loop_state = LoopState::new(2);
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({
                    "action": "read_range",
                    "path": "service_notes.md",
                    "mode": "head",
                    "n": 20
                }),
            },
            AgentAction::CallSkill {
                skill: "read_file".to_string(),
                args: json!({ "path": "README.md" }),
            },
            AgentAction::Respond {
                content: "先看 {{s1.output}}，再说明 {{s2.output}} 的作用".to_string(),
            },
        ];

        let rewritten =
            rewrite_terminal_placeholder_respond_to_synthesize_answer(&loop_state, actions);

        assert_eq!(rewritten.len(), 4);
        assert!(matches!(
            &rewritten[0],
            AgentAction::CallSkill { skill, .. } if skill == "system_basic"
        ));
        assert!(matches!(
            &rewritten[1],
            AgentAction::CallSkill { skill, .. } if skill == "read_file"
        ));
        assert!(matches!(
            &rewritten[2],
            AgentAction::SynthesizeAnswer { evidence_refs }
                if evidence_refs.as_slice()
                    == ["s1.output".to_string(), "s2.output".to_string()].as_slice()
        ));
        assert!(matches!(
            &rewritten[3],
            AgentAction::Respond { content } if content == "{{last_output}}"
        ));
    }

    #[test]
    fn normalized_multi_command_failure_summary_preserves_all_observations() {
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        let loop_state = LoopState::new(1);
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({"command": "echo THINK_BREAK_CN"}),
            },
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({"command": "definitely_missing_command_minimax_think_24690"}),
            },
            AgentAction::Respond {
                content: "执行结果总结：\n\n- **echo THINK_BREAK_CN** -> 成功，输出：{{s1.output}}\n- **definitely_missing_command_minimax_think_24690** -> 失败，输出：{{s2.output}}"
                    .to_string(),
            },
        ];

        let state = test_state_with_enabled_skills(&["run_cmd"]);
        let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "先执行 echo THINK_BREAK_CN，再执行 definitely_missing_command_minimax_think_24690，然后总结成功和失败分别是什么",
            None,
            Some("/home/guagua/rustclaw"),
            actions,
        );

        assert_eq!(
            actions_as_json(&normalized),
            json!([
                {
                    "type": "call_skill",
                    "skill": "run_cmd",
                    "args": {
                        "command": "echo THINK_BREAK_CN",
                        "_clawd_continue_on_error": true
                    }
                },
                {
                    "type": "call_skill",
                    "skill": "run_cmd",
                    "args": {
                        "command": "definitely_missing_command_minimax_think_24690",
                        "_clawd_continue_on_error": true
                    }
                },
                {
                    "type": "synthesize_answer",
                    "evidence_refs": ["s1.output", "s2.output"]
                },
                {
                    "type": "respond",
                    "content": "{{last_output}}"
                }
            ])
        );
        assert_eq!(normalized.len(), 4);
        assert!(matches!(
            &normalized[0],
            AgentAction::CallSkill { skill, args }
                if skill == "run_cmd"
                    && args.get("command").and_then(Value::as_str) == Some("echo THINK_BREAK_CN")
        ));
        assert!(matches!(
            &normalized[1],
            AgentAction::CallSkill { skill, args }
                if skill == "run_cmd"
                    && args.get("command").and_then(Value::as_str)
                        == Some("definitely_missing_command_minimax_think_24690")
        ));
        assert_eq!(
            super::action_args(&normalized[0])
                .and_then(|args| args.get("_clawd_continue_on_error"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            super::action_args(&normalized[1])
                .and_then(|args| args.get("_clawd_continue_on_error"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert!(matches!(
            &normalized[2],
            AgentAction::SynthesizeAnswer { evidence_refs }
                if evidence_refs.as_slice()
                    == ["s1.output".to_string(), "s2.output".to_string()].as_slice()
        ));
        assert!(matches!(
            &normalized[3],
            AgentAction::Respond { content } if content == "{{last_output}}"
        ));
    }

    #[test]
    fn normalized_run_cmd_observation_sequence_marks_continue_on_error() {
        let route = route_result(RoutedMode::Act, true, OutputResponseShape::Free);
        let loop_state = LoopState::new(1);
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({"command": "printenv PATH"}),
            },
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({"command": "definitely_absent_command_for_sequence_marker"}),
            },
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({"command": "uname -s"}),
            },
        ];

        let state = test_state_with_enabled_skills(&["run_cmd"]);
        let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "Run the listed command sequence and report each result.",
            None,
            Some("/home/guagua/rustclaw"),
            actions,
        );

        assert!(normalized.len() >= 3);
        for action in normalized.iter().take(3) {
            let args = super::action_args(action).expect("run_cmd args");
            assert_eq!(
                args.get("_clawd_continue_on_error")
                    .and_then(Value::as_bool),
                Some(true)
            );
        }
    }

    #[test]
    fn normalized_run_cmd_mutation_sequence_does_not_mark_continue_on_error() {
        let route = route_result(RoutedMode::Act, true, OutputResponseShape::Free);
        let loop_state = LoopState::new(1);
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({"command": "mkdir tmp_sequence_marker"}),
            },
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({"command": "pwd"}),
            },
        ];

        let state = test_state_with_enabled_skills(&["run_cmd"]);
        let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "Run this setup command and then inspect the current directory.",
            None,
            Some("/home/guagua/rustclaw"),
            actions,
        );

        assert!(normalized.len() >= 2);
        for action in normalized.iter().take(2) {
            let args = super::action_args(action).expect("run_cmd args");
            assert_eq!(args.get("_clawd_continue_on_error"), None);
        }
    }

    #[test]
    fn normalized_single_sequential_run_cmd_splits_for_step_status_evidence() {
        let route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        let loop_state = LoopState::new(1);
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({
                "command": "echo THINK_BREAK_CN; definitely_missing_command_minimax_think_24690",
                "cwd": "/home/guagua/rustclaw"
            }),
        }];

        let state = test_state_with_enabled_skills(&["run_cmd"]);
        let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "执行两个命令：echo THINK_BREAK_CN 和 definitely_missing_command_minimax_think_24690，然后总结哪些成功、哪些失败",
            None,
            Some("/home/guagua/rustclaw"),
            actions,
        );

        assert_eq!(
            actions_as_json(&normalized),
            json!([
                {
                    "type": "call_skill",
                    "skill": "run_cmd",
                    "args": {
                        "command": "echo THINK_BREAK_CN",
                        "cwd": "/home/guagua/rustclaw",
                        "_clawd_continue_on_error": true
                    }
                },
                {
                    "type": "call_skill",
                    "skill": "run_cmd",
                    "args": {
                        "command": "definitely_missing_command_minimax_think_24690",
                        "cwd": "/home/guagua/rustclaw",
                        "_clawd_continue_on_error": true
                    }
                },
                {
                    "type": "synthesize_answer",
                    "evidence_refs": ["step_1", "step_2"]
                },
                {
                    "type": "respond",
                    "content": "{{last_output}}"
                }
            ])
        );
    }

    #[test]
    fn normalized_planner_introduced_and_sequence_splits_for_step_status_evidence() {
        let route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::OneSentence);
        let loop_state = LoopState::new(1);
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({
                "command": "echo BEFORE_BREAK && definitely_missing_command_rustclaw_user_ops_13579"
            }),
        }];

        let state = test_state_with_enabled_skills(&["run_cmd"]);
        let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "执行两个命令：先 echo BEFORE_BREAK，再 definitely_missing_command_rustclaw_user_ops_13579，报告哪一步失败了",
            Some("先执行 echo BEFORE_BREAK，再执行 definitely_missing_command_rustclaw_user_ops_13579，只告诉我哪一步挂了"),
            Some("/home/guagua/rustclaw"),
            actions,
        );

        assert_eq!(
            actions_as_json(&normalized),
            json!([
                {
                    "type": "call_skill",
                    "skill": "run_cmd",
                    "args": {
                        "command": "echo BEFORE_BREAK",
                        "_clawd_continue_on_error": true
                    }
                },
                {
                    "type": "call_skill",
                    "skill": "run_cmd",
                    "args": {
                        "command": "definitely_missing_command_rustclaw_user_ops_13579",
                        "_clawd_continue_on_error": true
                    }
                },
                {
                    "type": "synthesize_answer",
                    "evidence_refs": ["step_1", "step_2"]
                },
                {
                    "type": "respond",
                    "content": "{{last_output}}"
                }
            ])
        );
    }

    #[test]
    fn user_supplied_and_operator_is_preserved_as_one_command() {
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "echo BEFORE_BREAK && echo AFTER_BREAK"}),
        }];

        let rewritten = super::split_sequential_run_cmd_actions(
            "Run `echo BEFORE_BREAK && echo AFTER_BREAK` exactly.",
            Some("Run `echo BEFORE_BREAK && echo AFTER_BREAK` exactly."),
            actions,
        );

        assert_eq!(rewritten.len(), 1);
        assert!(matches!(
            &rewritten[0],
            AgentAction::CallSkill { args, .. }
                if args.get("command").and_then(Value::as_str)
                    == Some("echo BEFORE_BREAK && echo AFTER_BREAK")
        ));
    }

    #[test]
    fn user_supplied_or_operator_is_preserved_as_one_command() {
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "missing_probe --version || which bash"}),
        }];

        let rewritten = super::split_sequential_run_cmd_actions(
            "Run `missing_probe --version || which bash` exactly.",
            Some("Run `missing_probe --version || which bash` exactly."),
            actions,
        );

        assert_eq!(rewritten.len(), 1);
        assert!(matches!(
            &rewritten[0],
            AgentAction::CallSkill { args, .. }
                if args.get("command").and_then(Value::as_str)
                    == Some("missing_probe --version || which bash")
        ));
    }

    #[test]
    fn user_supplied_semicolon_command_is_preserved_as_one_command() {
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "printf problem >&2; exit 7"}),
        }];

        let rewritten = super::split_sequential_run_cmd_actions(
            "执行命令 `printf problem >&2; exit 7`，报告退出码和 stderr 错误输出。",
            Some("执行命令 printf problem >&2; exit 7，告诉我退出码和错误输出。"),
            actions,
        );

        assert_eq!(rewritten.len(), 1);
        assert!(matches!(
            &rewritten[0],
            AgentAction::CallSkill { args, .. }
                if args.get("command").and_then(Value::as_str)
                    == Some("printf problem >&2; exit 7")
        ));
    }

    #[test]
    fn planner_introduced_or_operator_becomes_first_visible_attempt() {
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({
                "command": "missing_probe --version 2>/dev/null || which bash",
                "_clawd_continue_on_error": true,
                "_clawd_literal_command": true
            }),
        }];

        let rewritten = super::split_sequential_run_cmd_actions(
            "Run missing_probe --version. If it is missing, run which bash.",
            Some("Run missing_probe --version. If it is missing, run which bash."),
            actions,
        );

        assert_eq!(rewritten.len(), 1);
        assert!(matches!(
            &rewritten[0],
            AgentAction::CallSkill { args, .. }
                if args.get("command").and_then(Value::as_str)
                    == Some("missing_probe --version 2>/dev/null")
                    && args.get("_clawd_continue_on_error").is_none()
                    && args.get("_clawd_literal_command").is_none()
        ));
    }

    #[test]
    fn planner_introduced_and_operator_can_split_when_user_did_not_supply_it() {
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "echo BEFORE_BREAK && echo AFTER_BREAK"}),
        }];

        let rewritten = super::split_sequential_run_cmd_actions(
            "Run echo BEFORE_BREAK, then run echo AFTER_BREAK.",
            Some("Run echo BEFORE_BREAK, then run echo AFTER_BREAK."),
            actions,
        );

        assert_eq!(rewritten.len(), 2);
        assert!(matches!(
            &rewritten[0],
            AgentAction::CallSkill { args, .. }
                if args.get("command").and_then(Value::as_str) == Some("echo BEFORE_BREAK")
        ));
        assert!(matches!(
            &rewritten[1],
            AgentAction::CallSkill { args, .. }
                if args.get("command").and_then(Value::as_str) == Some("echo AFTER_BREAK")
        ));
    }

    #[test]
    fn shell_sequence_splitter_ignores_quoted_semicolons_and_stateful_prefixes() {
        assert_eq!(
            super::split_shell_sequence_command_with_policy("echo a; echo b", false),
            Some(vec!["echo a".to_string(), "echo b".to_string()])
        );
        assert_eq!(
            super::split_shell_sequence_command_with_policy("printf 'a;b\\n'", false),
            None
        );
        assert_eq!(
            super::split_shell_sequence_command_with_policy("cd /tmp; pwd", false),
            None
        );
    }

    #[test]
    fn rewrite_terminal_expression_placeholder_respond_inserts_synthesize_answer() {
        let loop_state = LoopState::new(2);
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({"action": "extract_field", "path": "package.json", "field_path": "name"}),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({"action": "extract_field", "path": "Cargo.toml", "field_path": "package.name"}),
            },
            AgentAction::Respond {
                content: "name={{s1}}; crate={{s2}}; same={{s1 == s2 ? 'yes' : 'no'}}".to_string(),
            },
        ];

        let rewritten =
            rewrite_terminal_placeholder_respond_to_synthesize_answer(&loop_state, actions);

        assert_eq!(rewritten.len(), 4);
        assert!(matches!(
            &rewritten[2],
            AgentAction::SynthesizeAnswer { evidence_refs }
                if evidence_refs.as_slice() == ["s1".to_string(), "s2".to_string()].as_slice()
        ));
        assert!(matches!(
            &rewritten[3],
            AgentAction::Respond { content } if content == "{{last_output}}"
        ));
    }

    #[test]
    fn rewrite_terminal_step_output_alias_placeholder_inserts_synthesize_answer() {
        let loop_state = LoopState::new(2);
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({"action": "inventory_dir", "path": "docs"}),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({"action": "read_range", "path": "docs/release_checklist.md"}),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
            AgentAction::Respond {
                content: "{{step1_output}} and {{step3_output}}".to_string(),
            },
        ];

        let rewritten =
            rewrite_terminal_placeholder_respond_to_synthesize_answer(&loop_state, actions);

        assert_eq!(rewritten.len(), 5);
        assert!(matches!(
            &rewritten[3],
            AgentAction::SynthesizeAnswer { evidence_refs }
                if evidence_refs.as_slice() == ["step_1".to_string(), "step_3".to_string()].as_slice()
        ));
        assert!(matches!(
            &rewritten[4],
            AgentAction::Respond { content } if content == "{{last_output}}"
        ));
    }

    #[test]
    fn rewrite_terminal_placeholder_preserves_mixed_last_output_respond() {
        let loop_state = LoopState::new(2);
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({"command": "pwd"}),
            },
            AgentAction::Respond {
                content:
                    "{{last_output}}\n\n这个路径是当前工作目录，通常对应正在操作的项目根目录。"
                        .to_string(),
            },
        ];

        let rewritten =
            rewrite_terminal_placeholder_respond_to_synthesize_answer(&loop_state, actions);

        assert_eq!(rewritten.len(), 2);
        assert!(matches!(
            &rewritten[1],
            AgentAction::Respond { content }
                if content.contains("{{last_output}}") && content.contains("当前工作目录")
        ));
    }

    #[test]
    fn unresolved_template_arg_multi_file_read_plan_uses_direct_file_reads() {
        let route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::OneSentence);
        let actions = vec![
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({"action": "read_range", "path": "README.md", "mode": "head", "n": 40}),
            },
            AgentAction::CallSkill {
                skill: "fs_search".to_string(),
                args: json!({"action": "find_name", "name": "AGENTS.md"}),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: json!({"action": "read_range", "path": "{{s1_match}}", "mode": "head", "n": 40}),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["s0".to_string(), "s2".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        let rewritten = rewrite_unresolved_template_arg_multi_file_read_plan(
            Some(&route),
            "read the opening section of README.md, then read the opening section of AGENTS.md",
            actions,
        );

        assert_eq!(rewritten.len(), 4);
        assert!(matches!(
            &rewritten[0],
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(|value| value.as_str()) == Some("read_range")
                    && args.get("path").and_then(|value| value.as_str()) == Some("README.md")
        ));
        assert!(matches!(
            &rewritten[1],
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(|value| value.as_str()) == Some("read_range")
                    && args.get("path").and_then(|value| value.as_str()) == Some("AGENTS.md")
        ));
        assert!(matches!(
            &rewritten[2],
            AgentAction::SynthesizeAnswer { evidence_refs }
                if evidence_refs.as_slice() == ["step_1".to_string(), "step_2".to_string()].as_slice()
        ));
        assert!(matches!(
            &rewritten[3],
            AgentAction::Respond { content } if content == "{{last_output}}"
        ));
    }

    /// §D2.a：plan_result schema 与 `AgentAction` enum / `SinglePlanEnvelope` 漂移检查。
    ///
    /// 校验内容：
    /// 1. `prompts/schemas/plan_result.schema.json` 是合法 JSON 且为 object schema；
    /// 2. envelope 顶层 required 含 `steps`；
    /// 3. `$defs/AgentAction.oneOf` 必须正好覆盖 5 个 variant：think / call_skill /
    ///    call_tool / synthesize_answer / respond（与 `AgentAction` enum 一一对应）；
    /// 4. 每个 variant 的 `type` const 必须是 snake_case 的 variant 名；
    /// 5. 每个 variant 的 required 字段必须 ⊇ `AgentAction` 该 variant 的非空字段；
    /// 6. 完整性闭环：把每个 variant 的最小合法实例 round-trip
    ///    `serde_json::from_value::<AgentAction>` 必须成功。
    #[test]
    fn plan_result_schema_drift() {
        const SCHEMA_RAW: &str =
            include_str!("../../../../prompts/schemas/plan_result.schema.json");
        let schema: serde_json::Value =
            serde_json::from_str(SCHEMA_RAW).expect("plan_result.schema.json must be valid JSON");
        assert_eq!(
            schema.get("type").and_then(|v| v.as_str()),
            Some("object"),
            "schema root must be object"
        );
        assert_eq!(
            schema.get("additionalProperties"),
            Some(&json!(false)),
            "schema root must reject unknown envelope fields after canonicalization"
        );
        let required = schema
            .get("required")
            .and_then(|v| v.as_array())
            .expect("schema must have `required`");
        assert!(
            required.iter().any(|v| v.as_str() == Some("steps")),
            "envelope must require `steps`"
        );
        let defs = schema
            .get("$defs")
            .and_then(|v| v.as_object())
            .expect("schema must declare $defs");
        let action = defs
            .get("AgentAction")
            .expect("$defs.AgentAction must exist");
        let one_of = action
            .get("oneOf")
            .and_then(|v| v.as_array())
            .expect("AgentAction must be a oneOf union");

        // 期望与 `AgentAction` enum 完全对齐：think / call_skill / call_tool /
        // synthesize_answer / respond
        let expected: HashSet<&str> = [
            "think",
            "call_skill",
            "call_tool",
            "synthesize_answer",
            "respond",
        ]
        .into_iter()
        .collect();
        let mut actual: HashSet<String> = HashSet::new();
        for entry in one_of {
            let ref_path = entry
                .get("$ref")
                .and_then(|v| v.as_str())
                .expect("oneOf entry must use $ref");
            let def_name = ref_path
                .strip_prefix("#/$defs/")
                .expect("$ref must point under #/$defs/");
            let def = defs.get(def_name).expect("referenced def must exist");
            assert_eq!(
                def.get("additionalProperties"),
                Some(&json!(false)),
                "variant `{}` must reject unknown action fields after canonicalization",
                def_name
            );
            let type_const = def
                .get("properties")
                .and_then(|v| v.get("type"))
                .and_then(|v| v.get("const"))
                .and_then(|v| v.as_str())
                .expect("variant must declare `properties.type.const`");
            actual.insert(type_const.to_string());
        }
        let actual_refs: HashSet<&str> = actual.iter().map(String::as_str).collect();
        assert_eq!(
            actual_refs, expected,
            "plan_result.schema.json AgentAction oneOf must cover exactly {:?}, got {:?}",
            expected, actual_refs
        );

        // §D2.a 步骤 6：每个 variant 的最小合法实例必须能反序列化进 AgentAction。
        let probes: &[(&str, serde_json::Value)] = &[
            ("think", json!({"type": "think", "content": "x"})),
            (
                "call_skill",
                json!({"type": "call_skill", "skill": "run_cmd", "args": {}}),
            ),
            (
                "call_tool",
                json!({"type": "call_tool", "tool": "read_file", "args": {}}),
            ),
            (
                "synthesize_answer",
                json!({"type": "synthesize_answer", "evidence_refs": ["last_output"]}),
            ),
            ("respond", json!({"type": "respond", "content": "ok"})),
        ];
        for (label, value) in probes {
            serde_json::from_value::<AgentAction>(value.clone()).unwrap_or_else(|err| {
                panic!(
                    "AgentAction variant `{}` failed to deserialize from schema-conformant minimum payload: {}",
                    label, err
                )
            });
        }
    }
}
