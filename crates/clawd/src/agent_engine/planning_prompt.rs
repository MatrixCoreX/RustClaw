use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;

use super::{
    LoopState, LIGHTWEIGHT_EXECUTION_PROMPT_LOGICAL_PATH, SINGLE_PLAN_EXECUTION_PROMPT_LOGICAL_PATH,
};
use crate::{AppState, ClaimedTask, RouteResult};

pub(super) fn build_incremental_plan_prompt(
    prompt_template: &str,
    user_request: &str,
    goal: &str,
    turn_analysis: &str,
    tool_spec: &str,
    skill_playbooks: &str,
    recent_assistant_replies: &str,
    request_language_hint: &str,
    config_response_language: &str,
    agent_runtime_identity: &str,
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
            ("__AGENT_RUNTIME_IDENTITY__", agent_runtime_identity),
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

pub(super) fn ensure_required_contract_block_present(
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

pub(super) fn runtime_os_label() -> String {
    format!(
        "{} (family={}, arch={})",
        std::env::consts::OS,
        std::env::consts::FAMILY,
        std::env::consts::ARCH
    )
}

pub(super) fn runtime_shell_label() -> String {
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
pub(super) enum PlanningPromptClass {
    OpenPlanning,
    LightweightExecution,
}

impl PlanningPromptClass {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::OpenPlanning => "open_planning",
            Self::LightweightExecution => "lightweight_execution",
        }
    }
}

pub(super) fn classify_planning_prompt_class(
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

pub(super) fn build_lightweight_tool_spec(
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
            "- route_gate_kind={} response_shape={} semantic_kind={} locator_kind={}",
            route.gate_kind().as_str(),
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

pub(super) fn compact_skill_playbook_from_prompt(skill: &str, prompt_body: &str) -> String {
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
    matchers: &[std::collections::BTreeMap<String, Value>],
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

fn compact_json_value_token(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::Array(values) => values
            .iter()
            .map(compact_json_value_token)
            .collect::<Vec<_>>()
            .join("|"),
        _ => value.to_string(),
    }
}

pub(super) fn build_lightweight_skill_playbooks_text(
    state: &AppState,
    task: &ClaimedTask,
    skill_scope: Option<&BTreeSet<String>>,
) -> String {
    let mut visible = state.planner_available_skills_for_task(task);
    if let Some(skill_scope) = skill_scope {
        visible.retain(|skill| skill_scope.contains(skill));
    }
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

pub(super) fn build_lightweight_skill_quick_index_text(
    state: &AppState,
    task: &ClaimedTask,
    skill_scope: Option<&BTreeSet<String>>,
) -> String {
    let mut visible = state.planner_available_skills_for_task(task);
    if let Some(skill_scope) = skill_scope {
        visible.retain(|skill| skill_scope.contains(skill));
    }
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

pub(super) fn round1_prompt_spec_for_class(
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

pub(super) fn contract_scoped_planner_skill_scope(
    route_result: Option<&RouteResult>,
) -> Option<BTreeSet<String>> {
    let route = route_result?;
    if route.needs_clarify || route.output_contract.semantic_kind == crate::OutputSemanticKind::None
    {
        return None;
    }
    let skills =
        crate::contract_matrix::allowed_action_refs_for_output_contract(&route.output_contract)
            .into_iter()
            .map(|action| action.skill)
            .filter(|skill| !skill.trim().is_empty())
            .collect::<BTreeSet<_>>();
    if skills.is_empty() || skills.len() > 10 {
        None
    } else {
        Some(skills)
    }
}
