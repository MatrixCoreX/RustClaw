use regex::Regex;
use serde_json::Value;
use std::path::Path;
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
        "- Prefer system_basic/fs_search style actions for explicit local targets and simple existence/read/list/extract requests.".to_string(),
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

fn build_lightweight_skill_playbooks_text() -> String {
    [
        "### system_basic",
        "- Use for bounded local reads, line ranges, directory listings, field extraction, and path_batch_facts.",
        "### fs_search",
        "- Use only when the user is asking to find a file by basename or locate a path.",
    ]
    .join("\n")
}

fn build_lightweight_skill_quick_index_text() -> String {
    [
        "- system_basic: bounded local read/list/extract/existence actions",
        "- fs_search: basename lookup only when target path is still missing",
    ]
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
            | crate::OutputSemanticKind::ScalarPathOnly
            | crate::OutputSemanticKind::ExistenceWithPath
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
    command.contains('>')
        || lower.contains(" tee ")
        || lower.contains(" sed -i")
        || lower.contains(" perl -0pi")
        || lower.contains(" perl -pi")
        || lower.contains(" printf ")
        || lower.contains(" echo ")
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
        && has_executable_observation_or_action(actions)
        && !has_discussion_followup_action(actions)
}

fn normalize_planned_actions(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let actions =
        strip_terminal_discussion_for_observed_finalize(route_result, loop_state, actions);
    let actions =
        strip_terminal_discussion_for_direct_skill_passthrough(state, route_result, actions);
    let actions = normalize_system_basic_schema_aliases(actions);
    let actions = enforce_output_contract_tool_args(route_result, actions);
    let actions =
        rewrite_single_target_file_read_to_auto_locator(route_result, auto_locator_path, actions);
    let actions = rewrite_extract_field_alias_args(actions);
    let actions = prune_unscoped_workspace_summary_evidence_for_scope(route_result, actions);
    let actions = strip_unrequested_workspace_artifact_mutations(route_result, loop_state, actions);
    let actions = append_synthesize_for_unscoped_workspace_text_evidence(route_result, actions);
    let actions = append_synthesize_answer_for_structured_scalar_compare(route_result, actions);
    let actions = strip_pre_observation_synthesize_before_concrete_respond(loop_state, actions);
    let actions = rewrite_pre_observation_concrete_respond_to_placeholder(loop_state, actions);
    let actions = rewrite_terminal_placeholder_respond_to_synthesize_answer(loop_state, actions);
    let actions = inject_synthesize_answer_for_bare_placeholder_respond(actions, user_text);
    append_synthesize_for_observation_only_terminal_answer(state, route_result, loop_state, actions)
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
        }
        "path_batch_facts" => {
            if !obj.contains_key("paths") {
                if let Some(paths) = obj.remove("targets").or_else(|| obj.remove("target_paths")) {
                    obj.insert("paths".to_string(), paths);
                } else if let Some(path) = obj.remove("path") {
                    obj.insert("paths".to_string(), Value::Array(vec![path]));
                }
            }
        }
        "inventory_dir" => {
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

fn enforce_output_contract_tool_args(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::HiddenEntriesCheck {
        return actions;
    }

    actions
        .into_iter()
        .map(|mut action| {
            match &mut action {
                AgentAction::CallSkill { skill, args }
                | AgentAction::CallTool { tool: skill, args }
                    if skill.eq_ignore_ascii_case("system_basic") =>
                {
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
                _ => {}
            }
            action
        })
        .collect()
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

fn route_disallows_unrequested_workspace_artifact_mutation(
    route: &RouteResult,
    loop_state: &LoopState,
) -> bool {
    route_needs_unscoped_workspace_text_evidence(route)
        && !route.wants_file_delivery
        && route.output_contract.delivery_intent == crate::OutputDeliveryIntent::None
        && !loop_state.execution_recipe.is_active()
}

fn strip_unrequested_workspace_artifact_mutations(
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

fn append_synthesize_for_unscoped_workspace_text_evidence(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route_needs_unscoped_workspace_text_evidence(route)
        || has_discussion_followup_action(&actions)
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

fn extract_output_placeholder_evidence_refs(text: &str) -> Vec<String> {
    static PLACEHOLDER_RE: OnceLock<Regex> = OnceLock::new();
    let re = PLACEHOLDER_RE.get_or_init(|| {
        Regex::new(r"\{\{\s*(last_output|s\d+\.output)\s*\}\}").expect("output placeholder regex")
    });
    let mut refs = Vec::new();
    for captures in re.captures_iter(text) {
        let Some(matched) = captures.get(1) else {
            continue;
        };
        let token = matched.as_str().trim().to_ascii_lowercase();
        if !refs.iter().any(|existing| existing == &token) {
            refs.push(token);
        }
    }
    refs
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
    if !has_pre_observation_structured_output_shape(&respond_content) {
        return actions;
    }
    let mut rewritten = actions;
    let original_len = respond_content.len();
    let respond_idx = rewritten.len() - 1;
    if let AgentAction::Respond { content } = &mut rewritten[respond_idx] {
        *content = "{{last_output}}".to_string();
    }
    info!(
        "plan_rewrite_pre_observation_concrete_respond_to_placeholder original_len={}",
        original_len
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
    let synth_step = AgentAction::SynthesizeAnswer {
        evidence_refs: vec!["last_output".to_string()],
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
        build_lightweight_skill_playbooks_text()
    };
    let skill_quick_index = if matches!(planning_class, PlanningPromptClass::OpenPlanning) {
        build_skill_quick_index_text(state, task)
    } else {
        build_lightweight_skill_quick_index_text()
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
    let initial_actions = parse_single_plan_actions(&plan_raw, state, task)
        .await
        .map(|actions| {
            normalize_planned_actions(
                state,
                route_result,
                loop_state,
                user_text,
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
                            normalize_planned_actions(
                                state,
                                route_result,
                                loop_state,
                                user_text,
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
                                    normalize_planned_actions(
                                        state,
                                        route_result,
                                        loop_state,
                                        user_text,
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
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    use claw_core::config::{AgentConfig, ToolsConfig};

    use super::{
        build_lightweight_tool_spec, can_fallback_to_initial_plan_after_repair_failure,
        classify_planning_prompt_class, has_pre_observation_structured_output_shape,
        inject_synthesize_answer_for_bare_placeholder_respond, is_bare_last_output_placeholder,
        normalize_planned_actions, normalize_system_basic_schema_aliases, plan_repair_reason,
        rewrite_extract_field_alias_args, rewrite_pre_observation_concrete_respond_to_placeholder,
        rewrite_terminal_placeholder_respond_to_synthesize_answer, round1_prompt_spec_for_class,
        should_force_actionable_plan_repair,
        strip_terminal_discussion_for_direct_skill_passthrough,
        strip_terminal_discussion_for_observed_finalize, LoopState, PlanningPromptClass,
    };
    use crate::{
        AgentAction, AgentRuntimeConfig, AppState, IntentOutputContract, OutputLocatorKind,
        OutputResponseShape, OutputSemanticKind, ResumeBehavior, RiskCeiling, RouteResult,
        RoutedMode, ScheduleKind, SkillViewsSnapshot, ToolsPolicy, DEFAULT_AGENT_ID,
    };
    use serde_json::json;

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
    fn code_change_profile_with_cargo_check_keeps_plan() {
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
            args: serde_json::json!({ "command": "cargo check" }),
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
            args: serde_json::json!({ "command": "cargo check -p clawd" }),
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
        let out = rewrite_pre_observation_concrete_respond_to_placeholder(&loop_state, actions);
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
        let after = rewrite_pre_observation_concrete_respond_to_placeholder(&loop_state, actions);
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
                rewrite_pre_observation_concrete_respond_to_placeholder(&loop_state, actions);
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
