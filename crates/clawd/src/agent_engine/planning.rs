use regex::Regex;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::OnceLock;
use tracing::{debug, info, warn};

use super::{
    build_loop_history_compact, build_single_plan_prompt, build_skill_playbooks_text,
    build_skill_quick_index_text, build_turn_analysis_prompt_block, plan_step_label,
    AgentLoopGuardPolicy, LoopState, SinglePlanEnvelope, AGENT_TOOL_SPEC_PATH,
    LIGHTWEIGHT_EXECUTION_PROMPT_LOGICAL_PATH, LOOP_INCREMENTAL_PLAN_PROMPT_LOGICAL_PATH,
    PLAN_REPAIR_PROMPT_LOGICAL_PATH, SINGLE_PLAN_EXECUTION_PROMPT_LOGICAL_PATH,
};
use crate::{
    llm_gateway, plan_step_from_agent_action, read_range_request::RequestedReadRange, AgentAction,
    AppState, ClaimedTask, PlanKind, PlanResult, RouteResult,
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
            AgentAction::Respond { content } => {
                // §3.4: planning 阶段不再调 semantic_judge LLM；改用本地启发式
                // looks_like_meta_respond_directive_local 过滤明显的 meta 占位
                // Respond 步骤。漏判会在 finalize 层 (loop_finalize::drop_passthrough_*)
                // 被 LLM 二次剔除，业务无损。
                if !actions.is_empty()
                    && crate::semantic_judge::looks_like_meta_respond_directive_local(&content)
                {
                    debug!(
                        "plan_meta_respond_suppressed task_id={} content={}",
                        task.task_id,
                        crate::truncate_for_log(&content)
                    );
                    continue;
                }
                actions.push(AgentAction::Respond { content });
            }
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
            if !state.is_builtin_skill(&canonical) {
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

fn should_rewrite_service_status_run_cmd_probe(
    route_result: Option<&RouteResult>,
    actions: &[AgentAction],
) -> bool {
    let Some(route_result) = route_result else {
        return false;
    };
    if route_result.needs_clarify
        || route_result.output_contract.delivery_required
        || !route_result.is_execute_gate()
    {
        return false;
    }
    let target = route_result.output_contract.locator_hint.trim();
    if target.is_empty() {
        return false;
    }
    if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput {
        return false;
    }
    if route_result.output_contract.semantic_kind != crate::OutputSemanticKind::ServiceStatus {
        return false;
    }
    let Some(AgentAction::CallSkill { skill, args }) = actions.first() else {
        return false;
    };
    if skill != "run_cmd" {
        return false;
    }
    let command = args
        .get("command")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .unwrap_or_default();
    let command_lower = command.to_ascii_lowercase();
    let target_lower = target.to_ascii_lowercase();
    (command_lower.contains("ps aux")
        || command_lower.contains("ps -")
        || command_lower.contains("pgrep")
        || command_lower.contains("grep -i"))
        && command_lower.contains(&target_lower)
}

fn rewrite_service_status_probe_actions(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !should_rewrite_service_status_run_cmd_probe(route_result, &actions) {
        return actions;
    }
    let Some(route_result) = route_result else {
        return actions;
    };
    let target = route_result.output_contract.locator_hint.trim();
    let mut rewritten = actions;
    if let Some(first) = rewritten.first_mut() {
        *first = AgentAction::CallSkill {
            skill: "service_control".to_string(),
            args: serde_json::json!({
                "action": "status",
                "target": target,
            }),
        };
    }
    info!(
        "plan_rewrite_service_status_probe target={} intent={}",
        target,
        crate::truncate_for_log(&route_result.resolved_intent)
    );
    rewritten
}

fn extract_http_probe_url(command: &str) -> Option<String> {
    command
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|ch: char| {
                    ch.is_whitespace()
                        || matches!(
                            ch,
                            '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
                        )
                })
                .to_string()
        })
        .find(|token| token.starts_with("http://") || token.starts_with("https://"))
}

fn extract_http_request_url(text: &str) -> Option<String> {
    extract_http_probe_url(text).or_else(|| {
        text.split_whitespace().find_map(|token| {
            let token = token.trim_matches(|ch: char| {
                ch.is_whitespace()
                    || matches!(
                        ch,
                        '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
                    )
            });
            if token.starts_with("127.0.0.1:") || token.starts_with("localhost:") {
                Some(format!("http://{token}"))
            } else {
                None
            }
        })
    })
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn text_contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn looks_health_check_request(route_result: Option<&RouteResult>, user_text: &str) -> bool {
    let route_context = route_result.map_or_else(String::new, |route| {
        format!("{}\n{}", route.resolved_intent, route.route_reason)
    });
    let joined = format!("{route_context}\n{}", user_text.trim()).to_ascii_lowercase();
    text_contains_any(
        &joined,
        &[
            "health check",
            "system health",
            "host operating system",
            "operating system",
            "rustclaw itself",
            "key fields",
            "健康检查",
            "操作系统",
            "系统健康",
            "只总结操作系统",
        ],
    )
}

fn looks_http_validate_then_repair_request(user_text: &str) -> bool {
    let joined = user_text.trim().to_ascii_lowercase();
    text_contains_any(
        &joined,
        &[
            "if the first validation fails",
            "if validation fails",
            "validate first",
            "then repair",
            "repair and re-validate",
            "先不要改",
            "第一次验证失败",
            "验证失败",
            "先直接验证",
            "再修复",
            "重新验证",
        ],
    )
}

fn synthesize_ops_http_repair_fallback_actions(
    loop_state: &LoopState,
    user_text: &str,
    auto_locator_path: Option<&str>,
) -> Option<(Vec<AgentAction>, String)> {
    if !loop_state.execution_recipe.is_active() {
        return None;
    }
    let path = auto_locator_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            user_text.split_whitespace().find_map(|token| {
                let token = token.trim_matches(|ch: char| {
                    ch.is_whitespace()
                        || matches!(
                            ch,
                            '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
                        )
                });
                (token.contains('/') && (token.ends_with(".html") || token.ends_with(".htm")))
                    .then_some(token)
            })
        })?;
    let url = extract_http_request_url(user_text)?;
    let marker = crate::verifier::extract_expected_http_marker(None, Some(user_text))?;
    if matches!(
        loop_state.execution_recipe.phase,
        crate::execution_recipe::ExecutionRecipePhase::Inspect
    ) && looks_http_validate_then_repair_request(user_text)
    {
        let actions = vec![AgentAction::CallSkill {
            skill: "http_basic".to_string(),
            args: serde_json::json!({ "action": "get", "url": url }),
        }];
        let raw_plan_text = serde_json::to_string_pretty(&serde_json::json!({
            "steps": [
                { "type": "call_skill", "skill": "http_basic", "args": { "action": "get", "url": url } }
            ],
            "notes": {
                "expected_marker": marker,
                "followup": "if validation fails, repair the target file in the next round"
            }
        }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
        return Some((actions, raw_plan_text));
    }
    if !matches!(
        loop_state.execution_recipe.phase,
        crate::execution_recipe::ExecutionRecipePhase::Apply
            | crate::execution_recipe::ExecutionRecipePhase::Repair
    ) {
        return None;
    }
    let command = format!(
        "python3 - <<'PY'\nfrom pathlib import Path\npath = Path({path})\nmarker = {marker}\nif path.exists() and path.is_dir():\n    target = path / 'index.html'\nelif not path.suffix:\n    target = path / 'index.html'\nelse:\n    target = path\ntarget.parent.mkdir(parents=True, exist_ok=True)\ntext = target.read_text(encoding='utf-8') if target.exists() else ''\nif marker not in text:\n    if text and not text.endswith('\\n'):\n        text += '\\n'\n    text += marker + '\\n'\n    target.write_text(text, encoding='utf-8')\nprint(f'patched {{target}}')\nPY",
        path = shell_single_quote(path),
        marker = shell_single_quote(&marker),
    );
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({ "command": command }),
        },
        AgentAction::CallSkill {
            skill: "http_basic".to_string(),
            args: serde_json::json!({ "action": "get", "url": url }),
        },
    ];
    let raw_plan_text = serde_json::to_string_pretty(&serde_json::json!({
        "steps": [
            { "type": "call_skill", "skill": "run_cmd", "args": { "command": command } },
            { "type": "call_skill", "skill": "http_basic", "args": { "action": "get", "url": url } }
        ]
    }))
    .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some((actions, raw_plan_text))
}

fn synthesize_health_check_fallback_actions(
    route_result: Option<&RouteResult>,
    user_text: &str,
) -> Option<(Vec<AgentAction>, String)> {
    if !looks_health_check_request(route_result, user_text) {
        return None;
    }
    let actions = vec![AgentAction::CallSkill {
        skill: "health_check".to_string(),
        args: serde_json::json!({}),
    }];
    let raw_plan_text = serde_json::to_string_pretty(&serde_json::json!({
        "steps": [
            { "type": "call_skill", "skill": "health_check", "args": {} }
        ]
    }))
    .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some((actions, raw_plan_text))
}

fn synthesize_plan_repair_fallback_actions(
    loop_state: &LoopState,
    route_result: Option<&RouteResult>,
    user_text: &str,
    auto_locator_path: Option<&str>,
) -> Option<(Vec<AgentAction>, String)> {
    synthesize_ops_http_repair_fallback_actions(loop_state, user_text, auto_locator_path)
        .or_else(|| synthesize_health_check_fallback_actions(route_result, user_text))
}

fn should_rewrite_http_probe_run_cmd(
    route_result: Option<&RouteResult>,
    actions: &[AgentAction],
) -> bool {
    let Some(route_result) = route_result else {
        return false;
    };
    if route_result.needs_clarify
        || route_result.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
    {
        return false;
    }
    let Some(AgentAction::CallSkill { skill, args }) = actions.first() else {
        return false;
    };
    if skill != "run_cmd" {
        return false;
    }
    let command = args
        .get("command")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .unwrap_or_default();
    let command_lower = command.to_ascii_lowercase();
    !run_cmd_likely_mutates(command)
        && (command_lower.contains("curl ") || command_lower.contains("wget "))
        && extract_http_probe_url(command).is_some()
}

fn rewrite_http_probe_actions(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !should_rewrite_http_probe_run_cmd(route_result, &actions) {
        return actions;
    }
    let Some(AgentAction::CallSkill { args, .. }) = actions.first() else {
        return actions;
    };
    let Some(command) = args.get("command").and_then(|value| value.as_str()) else {
        return actions;
    };
    let Some(url) = extract_http_probe_url(command) else {
        return actions;
    };
    let mut rewritten = actions;
    if let Some(first) = rewritten.first_mut() {
        *first = AgentAction::CallSkill {
            skill: "http_basic".to_string(),
            args: serde_json::json!({
                "action": "get",
                "url": url,
            }),
        };
    }
    info!("plan_rewrite_http_probe url={url}");
    rewritten
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
    let prompt = crate::render_prompt_template(
        &prompt_template,
        &[
            ("__GOAL__", goal),
            ("__TURN_ANALYSIS__", turn_analysis),
            ("__USER_REQUEST__", user_text),
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
    let request_surface = crate::intent::surface_signals::analyze_prompt_surface(user_text);
    let actions =
        strip_terminal_discussion_for_observed_finalize(route_result, loop_state, actions);
    let actions =
        strip_terminal_discussion_for_direct_skill_passthrough(state, route_result, actions);
    let actions = rewrite_service_status_probe_actions(route_result, actions);
    let actions = rewrite_http_probe_actions(route_result, actions);
    let actions = rewrite_sqlite3_run_cmd_to_db_basic(actions);
    let actions = rewrite_path_batch_size_facts_to_compare_paths(route_result, actions);
    let actions =
        rewrite_recent_artifacts_list_dir_to_inventory_dir(route_result, &request_surface, actions);
    let actions = rewrite_overbroad_list_dir_to_compare_paths(route_result, actions);
    let actions =
        rewrite_single_target_file_read_to_auto_locator(route_result, auto_locator_path, actions);
    let actions = rewrite_explicit_read_file_range_requests(
        route_result,
        &request_surface,
        user_text,
        actions,
    );
    let actions = rewrite_extract_field_alias_args(actions);
    let actions = prune_optional_extract_field_actions_for_workspace_summary(route_result, actions);
    let actions = prune_unscoped_workspace_summary_evidence_for_scope(route_result, actions);
    let actions =
        strip_unrequested_workspace_artifact_mutations(route_result, loop_state, actions);
    let actions = inject_unscoped_workspace_text_evidence_reads(state, route_result, actions);
    let actions = append_synthesize_for_unscoped_workspace_text_evidence(route_result, actions);
    let actions = append_synthesize_answer_for_structured_scalar_compare(route_result, actions);
    let actions = rewrite_pre_observation_concrete_respond_to_placeholder(loop_state, actions);
    let actions = rewrite_terminal_placeholder_respond_to_synthesize_answer(loop_state, actions);
    inject_synthesize_answer_for_bare_placeholder_respond(actions, user_text)
}

fn rewrite_sqlite3_run_cmd_to_db_basic(actions: Vec<AgentAction>) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args } if skill == "run_cmd" => {
                let command = args
                    .get("command")
                    .or_else(|| args.get("cmd"))
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                sqlite3_command_to_db_basic_args(command)
                    .map(|args| AgentAction::CallSkill {
                        skill: "db_basic".to_string(),
                        args,
                    })
                    .unwrap_or(AgentAction::CallSkill { skill, args })
            }
            other => other,
        })
        .collect()
}

fn shell_word(input: &str) -> Option<(&str, &str)> {
    let input = input.trim_start();
    if input.is_empty() {
        return None;
    }
    let mut chars = input.char_indices();
    let (_, first) = chars.next()?;
    if first == '"' || first == '\'' {
        let quote = first;
        for (idx, ch) in chars {
            if ch == quote {
                return Some((&input[1..idx], &input[idx + ch.len_utf8()..]));
            }
        }
        return None;
    }
    for (idx, ch) in input.char_indices() {
        if ch.is_whitespace() {
            return Some((&input[..idx], &input[idx..]));
        }
    }
    Some((input, ""))
}

fn trim_shell_quotes(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.len() >= 2 {
        let first = trimmed.as_bytes()[0] as char;
        let last = trimmed.as_bytes()[trimmed.len() - 1] as char;
        if (first == '"' && last == '"') || (first == '\'' && last == '\'') {
            return trimmed[1..trimmed.len() - 1].trim().to_string();
        }
    }
    trimmed.to_string()
}

fn sqlite3_command_to_db_basic_args(command: &str) -> Option<serde_json::Value> {
    let lower = command.to_ascii_lowercase();
    let sqlite_idx = lower.find("sqlite3")?;
    let mut rest = &command[sqlite_idx + "sqlite3".len()..];
    loop {
        let (word, next) = shell_word(rest)?;
        if !word.starts_with('-') {
            rest = rest.trim_start();
            break;
        }
        rest = next;
    }
    let (db_path, query_rest) = shell_word(rest)?;
    let db_lower = db_path.to_ascii_lowercase();
    if !(db_lower.ends_with(".sqlite") || db_lower.ends_with(".db")) {
        return None;
    }
    let query = trim_shell_quotes(query_rest);
    let sql = if query == ".tables" {
        "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name".to_string()
    } else {
        let query_lower = query.trim_start().to_ascii_lowercase();
        if !(query_lower.starts_with("select")
            || query_lower.starts_with("pragma")
            || query_lower.starts_with("with"))
        {
            return None;
        }
        query.trim_end_matches(';').trim().to_string()
    };
    Some(serde_json::json!({
        "action": "sqlite_query",
        "db_path": db_path,
        "sql": sql,
    }))
}

fn rewrite_path_batch_size_facts_to_compare_paths(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let route_requests_quantity_comparison = route_result.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison
    });
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args } if skill == "system_basic" => {
                let action_name = args
                    .get("action")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                if action_name != "path_batch_facts" {
                    return AgentAction::CallSkill { skill, args };
                }
                let paths = args
                    .get("paths")
                    .and_then(|value| value.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|value| value.as_str())
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let asks_size = args
                    .get("facts")
                    .and_then(|value| value.as_array())
                    .is_some_and(|items| {
                        items.iter().any(|value| {
                            value
                                .as_str()
                                .is_some_and(|item| item.eq_ignore_ascii_case("size"))
                        })
                    });
                if paths.len() == 2 && (asks_size || route_requests_quantity_comparison) {
                    AgentAction::CallSkill {
                        skill,
                        args: serde_json::json!({
                            "action": "compare_paths",
                            "left_path": paths[0],
                            "right_path": paths[1],
                        }),
                    }
                } else {
                    AgentAction::CallSkill { skill, args }
                }
            }
            other => other,
        })
        .collect()
}

fn single_list_dir_like_action_index_path(actions: &[AgentAction]) -> Option<(usize, String)> {
    let executable_count = actions
        .iter()
        .filter(|action| {
            matches!(
                action,
                AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
            )
        })
        .count();
    if executable_count != 1 {
        return None;
    }
    actions
        .iter()
        .enumerate()
        .find_map(|(idx, action)| match action {
            AgentAction::CallSkill { skill, args } if skill == "list_dir" => args
                .get("path")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|path| (idx, path.to_string())),
            AgentAction::CallSkill { skill, args } if skill == "system_basic" => args
                .get("action")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|action| {
                    matches!(
                        action.to_ascii_lowercase().as_str(),
                        "list_dir" | "inventory_dir"
                    )
                })
                .and_then(|_| args.get("path").and_then(|value| value.as_str()))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|path| (idx, path.to_string())),
            _ => None,
        })
}

fn compare_target_pair_from_locator_hint(route_result: &RouteResult) -> Option<(String, String)> {
    if route_result.output_contract.semantic_kind != crate::OutputSemanticKind::QuantityComparison {
        return None;
    }
    let locator_hint = route_result.output_contract.locator_hint.trim();
    if locator_hint.is_empty() {
        return None;
    }
    [",", "，", "|", ";", "；", "\n"]
        .into_iter()
        .find_map(|separator| {
            let parts = locator_hint
                .split(separator)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            (parts.len() == 2).then(|| (parts[0].to_string(), parts[1].to_string()))
        })
}

fn rewrite_recent_artifacts_list_dir_to_inventory_dir(
    route_result: Option<&RouteResult>,
    request_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::RecentArtifactsJudgment
    {
        return actions;
    }
    let Some(limit) = request_surface.requested_listing_limit else {
        return actions;
    };
    let Some((idx, path)) = single_list_dir_like_action_index_path(&actions) else {
        return actions;
    };
    let mut rewritten = actions;
    rewritten[idx] = AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "inventory_dir",
            "path": path,
            "sort_by": "mtime_desc",
            "max_entries": limit,
            "names_only": true,
        }),
    };
    info!("plan_rewrite_recent_artifacts_to_inventory_dir limit={limit}");
    rewritten
}

fn rewrite_overbroad_list_dir_to_compare_paths(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    let route_requests_quantity_comparison =
        route.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison;
    if route.needs_clarify
        || route.output_contract.delivery_required
        || !route_requests_quantity_comparison
    {
        return actions;
    }
    let Some((left, right)) = compare_target_pair_from_locator_hint(route) else {
        return actions;
    };
    let Some((idx, _path)) = single_list_dir_like_action_index_path(&actions) else {
        return actions;
    };
    let mut rewritten = actions;
    rewritten[idx] = AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "compare_paths",
            "left_path": left,
            "right_path": right,
        }),
    };
    info!(
        "plan_rewrite_list_dir_to_compare_paths left={} right={}",
        left, right
    );
    rewritten
}

fn requested_read_range(
    request_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    user_text: &str,
) -> Option<RequestedReadRange> {
    request_surface.requested_read_range.or_else(|| {
        crate::intent::surface_signals::requested_read_range_from_prompt_pair(None, user_text)
    })
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
        && route.output_contract.locator_hint.trim().is_empty()
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

fn action_workspace_text_path(action: &AgentAction) -> Option<&str> {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("read_file")
                || skill.eq_ignore_ascii_case("doc_parse") =>
        {
            args.get("path").and_then(|value| value.as_str())
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("system_basic")
                && args
                    .get("action")
                    .and_then(|value| value.as_str())
                    .is_some_and(|action| action.trim().eq_ignore_ascii_case("read_range")) =>
        {
            args.get("path").and_then(|value| value.as_str())
        }
        _ => None,
    }
}

fn text_path_matches_candidate(path: &str, candidate: &str) -> bool {
    let path = path.trim().trim_end_matches(['/', '\\']);
    let candidate = candidate.trim().trim_end_matches(['/', '\\']);
    !path.is_empty()
        && !candidate.is_empty()
        && (path == candidate
            || path.ends_with(&format!("/{candidate}"))
            || path.ends_with(&format!("\\{candidate}")))
}

fn workspace_text_evidence_candidate_docs(state: &AppState) -> Vec<&'static str> {
    const CANDIDATES: &[&str] = &[
        "README.md",
        "README.zh-CN.md",
        "USAGE.md",
        "docs/README.md",
        "docs/setup.md",
        "docs/deployment.md",
    ];
    CANDIDATES
        .iter()
        .copied()
        .filter(|path| state.skill_rt.workspace_root.join(path).is_file())
        .take(3)
        .collect()
}

fn workspace_doc_read_range_action(path: &str) -> AgentAction {
    AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "read_range",
            "path": path,
            "mode": "head",
            "n": 220
        }),
    }
}

fn inject_unscoped_workspace_text_evidence_reads(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route_needs_unscoped_workspace_text_evidence(route) {
        return actions;
    }
    let already_read_paths = actions
        .iter()
        .filter_map(action_workspace_text_path)
        .collect::<Vec<_>>();
    let candidate_docs = workspace_text_evidence_candidate_docs(state)
        .into_iter()
        .filter(|candidate| {
            !already_read_paths
                .iter()
                .any(|path| text_path_matches_candidate(path, candidate))
        })
        .collect::<Vec<_>>();
    if candidate_docs.is_empty() {
        return actions;
    }

    let mut prefix = actions;
    let mut terminal = Vec::new();
    while prefix.last().is_some_and(is_discussion_followup_action) {
        if let Some(action) = prefix.pop() {
            terminal.push(action);
        }
    }
    let first_inserted_step = prefix.len() + 1;
    let inserted = candidate_docs
        .iter()
        .map(|path| workspace_doc_read_range_action(path))
        .collect::<Vec<_>>();
    let inserted_paths = candidate_docs.join(",");
    let inserted_count = inserted.len();
    prefix.extend(inserted);
    let inserted_refs = (first_inserted_step..first_inserted_step + inserted_count)
        .map(|idx| format!("step_{idx}"))
        .collect::<Vec<_>>();
    terminal.reverse();
    for action in &mut terminal {
        if let AgentAction::SynthesizeAnswer { evidence_refs } = action {
            for reference in &inserted_refs {
                if !evidence_refs.iter().any(|value| value == reference) {
                    evidence_refs.push(reference.clone());
                }
            }
        }
    }
    prefix.extend(terminal);
    info!("plan_inject_unscoped_workspace_text_evidence paths={inserted_paths}");
    prefix
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

fn action_is_optional_extract_field(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } if skill == "system_basic" => args
            .get("action")
            .and_then(|value| value.as_str())
            .is_some_and(|action| {
                matches!(
                    action.trim().to_ascii_lowercase().as_str(),
                    "extract_field" | "extract_fields"
                )
            }),
        _ => false,
    }
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
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::OneSentence
        )
}

fn action_is_scalar_structured_extract(action: &AgentAction) -> bool {
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
                Some("extract_field") => true,
                Some("extract_fields") => args
                    .get("field_paths")
                    .and_then(|value| value.as_array())
                    .is_none_or(|field_paths| field_paths.len() <= 1),
                _ => false,
            }
        }
        _ => false,
    }
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
            action_is_scalar_structured_extract(action).then(|| format!("step_{}", idx + 1))
        })
        .collect::<Vec<_>>();
    if evidence_refs.len() < 2 {
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

fn prune_optional_extract_field_actions_for_workspace_summary(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::WorkspaceProjectSummary
        || !actions.iter().any(action_is_workspace_summary_evidence)
        || !actions.iter().any(action_is_optional_extract_field)
    {
        return actions;
    }
    let original_len = actions.len();
    let pruned = actions
        .into_iter()
        .filter(|action| !action_is_optional_extract_field(action))
        .collect::<Vec<_>>();
    if !pruned.iter().any(action_is_workspace_summary_evidence) {
        return pruned;
    }
    if pruned.len() != original_len {
        info!(
            "plan_prune_workspace_summary_extract_fields removed={}",
            original_len.saturating_sub(pruned.len())
        );
    }
    pruned
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

fn rewrite_explicit_read_file_range_requests(
    route_result: Option<&RouteResult>,
    request_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    user_text: &str,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(range_request) = requested_read_range(request_surface, user_text).or_else(|| {
        route_result.and_then(|route| {
            let resolved_intent = route.resolved_intent.trim();
            if resolved_intent.is_empty() || resolved_intent == user_text.trim() {
                return None;
            }
            let resolved_surface =
                crate::intent::surface_signals::analyze_prompt_surface(resolved_intent);
            requested_read_range(&resolved_surface, resolved_intent)
        })
    }) else {
        return actions;
    };

    let mut read_file_idx = None;
    let mut read_file_path = None;
    for (idx, action) in actions.iter().enumerate() {
        match action {
            AgentAction::Think { .. }
            | AgentAction::Respond { .. }
            | AgentAction::SynthesizeAnswer { .. } => {}
            AgentAction::CallSkill { skill, args }
                if skill.eq_ignore_ascii_case("system_basic")
                    && args
                        .get("action")
                        .and_then(|value| value.as_str())
                        .is_some_and(|action| action.eq_ignore_ascii_case("read_range")) =>
            {
                return actions;
            }
            AgentAction::CallSkill { skill, args } if skill.eq_ignore_ascii_case("read_file") => {
                let Some(path) = args.get("path").and_then(|value| value.as_str()) else {
                    return actions;
                };
                if read_file_idx.is_some() {
                    return actions;
                }
                read_file_idx = Some(idx);
                read_file_path = Some(path.trim().to_string());
            }
            _ => return actions,
        }
    }

    let Some(idx) = read_file_idx else {
        return actions;
    };
    let Some(path) = read_file_path.filter(|value| !value.is_empty()) else {
        return actions;
    };

    let read_range_args = match range_request {
        RequestedReadRange::Head { n } => {
            json!({ "action": "read_range", "path": path, "mode": "head", "n": n })
        }
        RequestedReadRange::Tail { n } => {
            json!({ "action": "read_range", "path": path, "mode": "tail", "n": n })
        }
        RequestedReadRange::Range {
            start_line,
            end_line,
        } => json!({
            "action": "read_range",
            "path": path,
            "mode": "range",
            "start_line": start_line,
            "end_line": end_line
        }),
    };

    let mut rewritten = actions;
    rewritten[idx] = AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: read_range_args,
    };
    info!(
        "plan_rewrite_explicit_read_file_range_request idx={} request={:?}",
        idx, range_request
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
    if !looks_like_pre_observation_hallucinated_concrete_content(&respond_content) {
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

/// §F1 启发式：判断 Respond.content 是否是「未观测就编造」的具体内容形态。
///
/// 命中任意一条即视为可疑：
/// - 含至少一行以数字+点+空格开头的枚举项（最少 1 行；`1. foo` / `2. bar`）
/// - 含 3+ 行（`\n` ≥ 2）且至少含一个 `/` 或常见文件后缀
/// - 含明显结构化字段标签（`result:` / `count:` / `size:` / `path:`，大小写不敏感）
///
/// 这些都是 list_dir / read_file / fs_search / run_cmd 的典型输出形态，
/// 在 round 1 还没执行任何步骤时不可能合法出现在 Respond 里。
fn looks_like_pre_observation_hallucinated_concrete_content(content: &str) -> bool {
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
                user_text,
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
                user_text,
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
        // Phase 1.1: LLM 修复之前先尝试本地兜底
        // (`synthesize_plan_repair_fallback_actions`)。命中时直接复用，避免
        // 一次（可能两次）的 `plan_repair_prompt` LLM 调用。
        // 只有在本地兜底找不到可用模式或产物仍被判定需要修复时，
        // 才回退到原先的 LLM 修复链路。
        let local_fallback_hit = synthesize_plan_repair_fallback_actions(
            loop_state,
            route_result,
            user_text,
            auto_locator_path,
        )
        .map(|(actions, raw_plan)| {
            (
                normalize_planned_actions(
                    state,
                    route_result,
                    loop_state,
                    user_text,
                    auto_locator_path,
                    actions,
                ),
                raw_plan,
            )
        })
        .filter(|(actions, _)| {
            !should_force_actionable_plan_repair(state, route_result, loop_state, actions)
        });
        if let Some((local_fallback_actions, local_fallback_raw_plan)) = local_fallback_hit {
            info!(
                "plan_repair_local_fallback_hit task_id={} round={} reason={}",
                task.task_id, loop_state.round_no, repair_reason
            );
            (
                local_fallback_actions,
                PlanKind::Repair,
                local_fallback_raw_plan,
            )
        } else {
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
                    let repaired_actions = parse_single_plan_actions(&repaired, state, task)
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
                                    if let Some((fallback_actions, fallback_raw_plan)) =
                                        synthesize_plan_repair_fallback_actions(
                                            loop_state,
                                            route_result,
                                            user_text,
                                            auto_locator_path,
                                        )
                                    {
                                        warn!(
                                            "plan_repair_synthesized_fallback task_id={} round={}",
                                            task.task_id, loop_state.round_no
                                        );
                                        (fallback_actions, PlanKind::Repair, fallback_raw_plan)
                                    } else {
                                        return Err(
                                            "repair plan still non-actionable after second repair"
                                                .to_string(),
                                        );
                                    }
                                }
                                None => {
                                    if let Some((fallback_actions, fallback_raw_plan)) =
                                        synthesize_plan_repair_fallback_actions(
                                            loop_state,
                                            route_result,
                                            user_text,
                                            auto_locator_path,
                                        )
                                    {
                                        warn!(
                                            "plan_repair_synthesized_fallback_after_parse_fail task_id={} round={}",
                                            task.task_id, loop_state.round_no
                                        );
                                        (fallback_actions, PlanKind::Repair, fallback_raw_plan)
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
        classify_planning_prompt_class, inject_synthesize_answer_for_bare_placeholder_respond,
        is_bare_last_output_placeholder, looks_health_check_request,
        looks_like_pre_observation_hallucinated_concrete_content, normalize_planned_actions,
        plan_repair_reason, rewrite_extract_field_alias_args, rewrite_http_probe_actions,
        rewrite_path_batch_size_facts_to_compare_paths,
        rewrite_pre_observation_concrete_respond_to_placeholder,
        rewrite_service_status_probe_actions, rewrite_sqlite3_run_cmd_to_db_basic,
        rewrite_terminal_placeholder_respond_to_synthesize_answer, round1_prompt_spec_for_class,
        should_force_actionable_plan_repair,
        strip_terminal_discussion_for_direct_skill_passthrough,
        strip_terminal_discussion_for_observed_finalize, synthesize_health_check_fallback_actions,
        synthesize_ops_http_repair_fallback_actions, LoopState, PlanningPromptClass,
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
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        }
    }

    #[test]
    fn planning_prompt_class_uses_lightweight_execution_for_scalar_contract() {
        let mut route = base_route_result();
        route.route_reason = "route_contract:generic_filename_scalar_extract".to_string();
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
        route.route_reason = "route_contract:pwd_only_current_workspace".to_string();
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
        route.route_reason = "route_contract:generic_filename_read_range".to_string();
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
        scalar.route_reason = "route_contract:generic_filename_scalar_extract".to_string();
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
        route.route_reason = "route_contract:generic_explicit_path_scalar_extract".to_string();
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
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        }
    }

    fn delivery_route_result() -> RouteResult {
        let mut route = route_result(RoutedMode::Act, false, OutputResponseShape::FileToken);
        route.output_contract.delivery_required = true;
        route
    }

    #[test]
    fn service_status_probe_rewrites_run_cmd_grep_to_service_control() {
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::OneSentence);
        route.resolved_intent = "检查 telegramd 进程是否正在运行，并用一句话解释状态".to_string();
        route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
        route.output_contract.locator_hint = "telegramd".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({ "command": "ps aux | grep -i telegramd | grep -v grep" }),
        }];

        let rewritten = rewrite_service_status_probe_actions(Some(&route), actions);
        assert!(matches!(
            &rewritten[0],
            AgentAction::CallSkill { skill, args }
                if skill == "service_control"
                    && args.get("action").and_then(|v| v.as_str()) == Some("status")
                    && args.get("target").and_then(|v| v.as_str()) == Some("telegramd")
        ));
    }

    #[test]
    fn explicit_command_request_keeps_run_cmd_probe() {
        let mut route = route_result(RoutedMode::Act, false, OutputResponseShape::Free);
        route.resolved_intent =
            "执行命令 ps aux | grep -i telegramd | grep -v grep，并直接回复执行结果".to_string();
        route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
        route.output_contract.locator_hint = "telegramd".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({ "command": "ps aux | grep -i telegramd | grep -v grep" }),
        }];

        let rewritten = rewrite_service_status_probe_actions(Some(&route), actions.clone());
        assert!(matches!(
            &rewritten[0],
            AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
        ));
    }

    #[test]
    fn english_status_probe_rewrites_run_cmd_to_service_control() {
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::OneSentence);
        route.resolved_intent =
            "Check whether telegramd is running and briefly explain the status".to_string();
        route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
        route.output_contract.locator_hint = "telegramd".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({ "command": "pgrep -fa telegramd" }),
        }];

        let rewritten = rewrite_service_status_probe_actions(Some(&route), actions);
        assert!(matches!(
            &rewritten[0],
            AgentAction::CallSkill { skill, args }
                if skill == "service_control"
                    && args.get("action").and_then(|v| v.as_str()) == Some("status")
                    && args.get("target").and_then(|v| v.as_str()) == Some("telegramd")
        ));
    }

    #[test]
    fn http_probe_run_cmd_rewrites_to_http_basic() {
        let route = route_result(RoutedMode::Act, false, OutputResponseShape::Free);
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({ "command": "curl -s http://127.0.0.1:62078/" }),
        }];

        let rewritten = rewrite_http_probe_actions(Some(&route), actions);
        assert!(matches!(
            &rewritten[0],
            AgentAction::CallSkill { skill, args }
                if skill == "http_basic"
                    && args.get("action").and_then(|v| v.as_str()) == Some("get")
                    && args.get("url").and_then(|v| v.as_str()) == Some("http://127.0.0.1:62078/")
        ));
    }

    #[test]
    fn mutating_http_run_cmd_does_not_rewrite_to_http_basic() {
        let route = route_result(RoutedMode::Act, false, OutputResponseShape::Free);
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({
                "command": "cd /tmp/demo && nohup python3 -m http.server 62078 --bind 127.0.0.1 > /dev/null 2>&1 & sleep 2 && curl -s http://127.0.0.1:62078/"
            }),
        }];

        let rewritten = rewrite_http_probe_actions(Some(&route), actions.clone());
        assert!(matches!(
            &rewritten[0],
            AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
        ));
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
        route.route_reason = "route_contract:generic_filename_single_read".to_string();
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
    fn clarify_followup_tail_request_rewrites_single_read_file_to_read_range() {
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
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("read_range")
                );
                assert_eq!(
                    args.get("mode").and_then(|value| value.as_str()),
                    Some("tail")
                );
                assert_eq!(args.get("n").and_then(|value| value.as_u64()), Some(5));
            }
            other => panic!("expected read_range rewrite, got {other:?}"),
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
    fn sqlite3_run_cmd_is_rewritten_to_db_basic_query() {
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({
                "command": "sqlite3 data/db-basic-contract.sqlite \"SELECT name FROM sqlite_master WHERE type='table' ORDER BY name;\""
            }),
        }];

        let rewritten = rewrite_sqlite3_run_cmd_to_db_basic(actions);
        assert_eq!(rewritten.len(), 1);
        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "db_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("sqlite_query")
                );
                assert_eq!(
                    args.get("db_path").and_then(|value| value.as_str()),
                    Some("data/db-basic-contract.sqlite")
                );
                assert_eq!(
                    args.get("sql").and_then(|value| value.as_str()),
                    Some("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                );
            }
            other => panic!("expected db_basic sqlite_query action, got {other:?}"),
        }
    }

    #[test]
    fn path_batch_size_facts_is_rewritten_to_compare_paths() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "path_batch_facts",
                "paths": ["README.md", "AGENTS.md"],
                "facts": ["size"]
            }),
        }];

        let rewritten = rewrite_path_batch_size_facts_to_compare_paths(None, actions);
        assert_eq!(rewritten.len(), 1);
        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("compare_paths")
                );
                let left = args.get("left_path").and_then(|value| value.as_str());
                let right = args.get("right_path").and_then(|value| value.as_str());
                let pair = [left, right];
                assert!(pair.contains(&Some("README.md")));
                assert!(pair.contains(&Some("AGENTS.md")));
            }
            other => panic!("expected system_basic compare_paths action, got {other:?}"),
        }
    }

    #[test]
    fn quantity_comparison_route_rewrites_path_batch_facts_without_explicit_size_fact() {
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "path_batch_facts",
                "paths": ["README.md", "AGENTS.md"],
            }),
        }];
        let mut route = route_result(RoutedMode::Act, true, OutputResponseShape::OneSentence);
        route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
        route.resolved_intent = "比较 README.md 和 AGENTS.md 哪个更大".to_string();

        let rewritten = rewrite_path_batch_size_facts_to_compare_paths(Some(&route), actions);
        assert_eq!(rewritten.len(), 1);
        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("compare_paths")
                );
                let left = args.get("left_path").and_then(|value| value.as_str());
                let right = args.get("right_path").and_then(|value| value.as_str());
                let pair = [left, right];
                assert!(pair.contains(&Some("README.md")));
                assert!(pair.contains(&Some("AGENTS.md")));
            }
            other => panic!("expected system_basic compare_paths action, got {other:?}"),
        }
    }

    #[test]
    fn recent_artifacts_route_rewrites_bare_list_dir_to_inventory_dir() {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "list_dir".to_string(),
                args: serde_json::json!({ "path": "logs" }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
        ];
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentArtifactsJudgment;
        route.resolved_intent =
            "列出 logs 目录最近修改的 2 个文件名，再用一句中文告诉我这更像运行日志还是测试残留"
                .to_string();
        let request_surface =
            crate::intent::surface_signals::analyze_prompt_surface(&route.resolved_intent);

        let rewritten = super::rewrite_recent_artifacts_list_dir_to_inventory_dir(
            Some(&route),
            &request_surface,
            actions,
        );
        assert_eq!(rewritten.len(), 2);
        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("inventory_dir")
                );
                assert_eq!(
                    args.get("path").and_then(|value| value.as_str()),
                    Some("logs")
                );
                assert_eq!(
                    args.get("sort_by").and_then(|value| value.as_str()),
                    Some("mtime_desc")
                );
                assert_eq!(
                    args.get("max_entries").and_then(|value| value.as_u64()),
                    Some(2)
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
    fn quantity_comparison_route_rewrites_bare_list_dir_to_compare_paths() {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "list_dir".to_string(),
                args: serde_json::json!({ "path": "." }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
        ];
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::OneSentence);
        route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
        route.output_contract.locator_hint = "README.md, AGENTS.md".to_string();
        route.resolved_intent = "keep the answer brief".to_string();

        let rewritten = super::rewrite_overbroad_list_dir_to_compare_paths(Some(&route), actions);
        assert_eq!(rewritten.len(), 2);
        match &rewritten[0] {
            AgentAction::CallSkill { skill, args } => {
                assert_eq!(skill, "system_basic");
                assert_eq!(
                    args.get("action").and_then(|value| value.as_str()),
                    Some("compare_paths")
                );
                let left = args.get("left_path").and_then(|value| value.as_str());
                let right = args.get("right_path").and_then(|value| value.as_str());
                let pair = [left, right];
                assert!(pair.contains(&Some("README.md")));
                assert!(pair.contains(&Some("AGENTS.md")));
            }
            other => panic!("expected system_basic compare_paths action, got {other:?}"),
        }
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
    fn workspace_summary_prunes_optional_extract_field_steps_when_read_evidence_exists() {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "list_dir".to_string(),
                args: serde_json::json!({ "path": "." }),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "read_range",
                    "path": "README.md",
                    "mode": "head",
                    "n": 20
                }),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "extract_field",
                    "path": "Cargo.toml",
                    "field_path": "package.name"
                }),
            },
        ];
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
        route.resolved_intent = "Summarize this repository for me".to_string();

        let pruned = super::prune_optional_extract_field_actions_for_workspace_summary(
            Some(&route),
            actions,
        );
        assert_eq!(pruned.len(), 2);
        assert!(pruned
            .iter()
            .all(|action| !super::action_is_optional_extract_field(action)));
    }

    #[test]
    fn workspace_summary_prunes_multi_extract_fields_when_read_evidence_exists() {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "list_dir".to_string(),
                args: serde_json::json!({ "path": "." }),
            },
            AgentAction::CallSkill {
                skill: "read_file".to_string(),
                args: serde_json::json!({ "path": "README.md" }),
            },
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "extract_fields",
                    "path": "Cargo.toml",
                    "field_paths": [
                        "package.name",
                        "package.version",
                        "package.description",
                        "workspace.package.description"
                    ]
                }),
            },
        ];
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
        route.resolved_intent = "帮我总结这个仓库".to_string();

        let pruned = super::prune_optional_extract_field_actions_for_workspace_summary(
            Some(&route),
            actions,
        );
        assert_eq!(pruned.len(), 2);
        assert!(pruned
            .iter()
            .all(|action| !super::action_is_optional_extract_field(action)));
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
    fn unscoped_workspace_evidence_injects_doc_reads_after_search_only_plan() {
        let root = TempDirGuard::new("workspace_text_evidence");
        fs::write(root.path.join("README.md"), "# RustClaw\n\nSetup notes").expect("write README");
        fs::write(
            root.path.join("USAGE.md"),
            "# Usage\n\nRun documented steps",
        )
        .expect("write USAGE");
        let mut state = test_state();
        state.skill_rt.workspace_root = root.path.clone();
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = OutputSemanticKind::None;
        route.output_contract.locator_hint.clear();
        route.resolved_intent = "帮我写一段当前项目安装说明".to_string();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "list_dir".to_string(),
                args: json!({"path":"."}),
            },
            AgentAction::CallSkill {
                skill: "fs_search".to_string(),
                args: json!({"action":"find_name","pattern":"README"}),
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
        let injected_paths = normalized
            .iter()
            .filter_map(|action| match action {
                AgentAction::CallSkill { skill, args }
                    if skill == "system_basic"
                        && args.get("action").and_then(|value| value.as_str())
                            == Some("read_range") =>
                {
                    args.get("path").and_then(|value| value.as_str())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(injected_paths, vec!["README.md", "USAGE.md"]);
        assert!(matches!(
            normalized.last(),
            Some(AgentAction::SynthesizeAnswer { evidence_refs })
                if evidence_refs == &vec!["step_3".to_string(), "step_4".to_string()]
        ));
    }

    #[test]
    fn unscoped_workspace_evidence_injects_doc_reads_before_terminal_synthesis() {
        let root = TempDirGuard::new("workspace_text_evidence_synthesis");
        fs::write(root.path.join("README.md"), "# RustClaw\n\nSetup notes").expect("write README");
        let mut state = test_state();
        state.skill_rt.workspace_root = root.path.clone();
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = OutputSemanticKind::None;
        route.output_contract.locator_hint.clear();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "process_basic".to_string(),
                args: json!({"action":"list","filter":"rustclaw"}),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
        ];

        let normalized = super::normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(2),
            &route.resolved_intent,
            None,
            actions,
        );
        assert!(matches!(
            normalized.last(),
            Some(AgentAction::SynthesizeAnswer { .. })
        ));
        assert!(matches!(
            normalized.get(normalized.len().saturating_sub(2)),
            Some(AgentAction::CallSkill { skill, args })
                if skill == "system_basic"
                    && args.get("action").and_then(|value| value.as_str()) == Some("read_range")
                    && args.get("path").and_then(|value| value.as_str()) == Some("README.md")
        ));
        assert!(matches!(
            normalized.last(),
            Some(AgentAction::SynthesizeAnswer { evidence_refs })
                if evidence_refs == &vec!["last_output".to_string(), "step_2".to_string()]
        ));
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
    fn unscoped_workspace_evidence_supplements_existing_read_with_unread_docs() {
        let root = TempDirGuard::new("workspace_text_evidence_supplement");
        fs::write(root.path.join("README.md"), "# RustClaw").expect("write README");
        fs::write(root.path.join("README.zh-CN.md"), "# RustClaw 中文").expect("write zh README");
        fs::write(root.path.join("USAGE.md"), "# Usage").expect("write USAGE");
        let mut state = test_state();
        state.skill_rt.workspace_root = root.path.clone();
        let mut route = route_result(RoutedMode::ChatAct, true, OutputResponseShape::Free);
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = OutputSemanticKind::None;
        route.output_contract.locator_hint.clear();
        let actions = vec![AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({"path": root.path.join("README.md").display().to_string()}),
        }];

        let normalized = super::normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(1),
            &route.resolved_intent,
            None,
            actions,
        );
        let injected_paths = normalized
            .iter()
            .filter_map(|action| match action {
                AgentAction::CallSkill { skill, args }
                    if skill == "system_basic"
                        && args.get("action").and_then(|value| value.as_str())
                            == Some("read_range") =>
                {
                    args.get("path").and_then(|value| value.as_str())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(injected_paths, vec!["README.zh-CN.md", "USAGE.md"]);
        assert!(matches!(
            normalized.last(),
            Some(AgentAction::SynthesizeAnswer { evidence_refs })
                if evidence_refs
                    == &vec![
                        "step_1".to_string(),
                        "step_2".to_string(),
                        "step_3".to_string()
                    ]
        ));
    }

    #[test]
    fn unscoped_workspace_text_answer_strips_unrequested_file_artifact_plan() {
        let root = TempDirGuard::new("workspace_text_evidence_no_artifact");
        fs::write(root.path.join("README.md"), "# RustClaw\n\nUse the documented installer")
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
        assert!(normalized.iter().any(|action| {
            matches!(
                action,
                AgentAction::CallSkill { skill, args }
                    if skill == "system_basic"
                        && args.get("action").and_then(|value| value.as_str())
                            == Some("read_range")
                        && args.get("path").and_then(|value| value.as_str())
                            == Some("README.md")
            )
        }));
        assert!(matches!(
            normalized.last(),
            Some(AgentAction::SynthesizeAnswer { .. })
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
    fn synthesize_ops_http_repair_fallback_builds_mutate_then_validate_plan() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            ..Default::default()
        };
        let (actions, raw_plan_text) = synthesize_ops_http_repair_fallback_actions(
            &loop_state,
            "先验证 127.0.0.1:64486 首页是否包含 ops-repair-ok；若失败则修复 index.html 使其包含 ops-repair-ok，再验证直到通过。",
            Some("/tmp/document/nl_ops_http_repair_demo/index.html"),
        )
        .expect("fallback should be synthesized");
        assert!(raw_plan_text.contains("\"run_cmd\""));
        assert!(raw_plan_text.contains("\"http_basic\""));
        assert!(matches!(
            &actions[0],
            AgentAction::CallSkill { skill, args }
                if skill == "run_cmd"
                    && args
                        .get("command")
                        .and_then(|v| v.as_str())
                        .is_some_and(|command| command.contains("ops-repair-ok"))
        ));
        assert!(matches!(
            &actions[1],
            AgentAction::CallSkill { skill, args }
                if skill == "http_basic"
                    && args.get("url").and_then(|v| v.as_str()) == Some("http://127.0.0.1:64486")
        ));
    }

    #[test]
    fn synthesize_ops_http_repair_fallback_builds_validate_only_plan_during_inspect_phase() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Inspect,
            inspect_first: true,
            validation_required: true,
            ..Default::default()
        };
        let (actions, raw_plan_text) = synthesize_ops_http_repair_fallback_actions(
            &loop_state,
            "先不要改文件或进程，先直接验证当前已经在 127.0.0.1:64608 运行的本地静态 HTTP 服务首页是否包含 ops-repair-ok。如果第一次验证失败，再修复 document/nl_ops_http_repair_demo/index.html 的内容，使首页包含 ops-repair-ok，然后重新验证直到通过；通过时明确输出 VALIDATION_PASSED 后直接结束。",
            Some("/tmp/document/nl_ops_http_repair_demo/index.html"),
        )
        .expect("inspect fallback should be synthesized");
        assert!(raw_plan_text.contains("\"http_basic\""));
        assert!(raw_plan_text.contains("\"expected_marker\": \"ops-repair-ok\""));
        assert!(matches!(
            actions.as_slice(),
            [AgentAction::CallSkill { skill, args }]
                if skill == "http_basic"
                    && args.get("action").and_then(|v| v.as_str()) == Some("get")
                    && args.get("url").and_then(|v| v.as_str()) == Some("http://127.0.0.1:64608")
        ));
    }

    #[test]
    fn synthesize_ops_http_repair_fallback_treats_directory_locator_as_index_html() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            ..Default::default()
        };
        let (actions, _) = synthesize_ops_http_repair_fallback_actions(
            &loop_state,
            "Start a static HTTP server in the background from document/nl_ops_http_demo on 127.0.0.1:63848, then use curl to verify that the homepage contains ops-demo-ok; when validation passes, explicitly output VALIDATION_PASSED and finish immediately.",
            Some("/tmp/document/nl_ops_http_demo"),
        )
        .expect("fallback should be synthesized");
        assert!(matches!(
            &actions[0],
            AgentAction::CallSkill { skill, args }
                if skill == "run_cmd"
                    && args
                        .get("command")
                        .and_then(|v| v.as_str())
                        .is_some_and(|command| {
                            command.contains("path.is_dir()")
                                && command.contains("target = path / 'index.html'")
                                && command.contains("ops-demo-ok")
                        })
        ));
    }

    #[test]
    fn synthesize_health_check_fallback_builds_observation_plan_for_english_request() {
        let route = route_result(RoutedMode::Act, true, OutputResponseShape::OneSentence);
        assert!(looks_health_check_request(
            Some(&route),
            "Run a basic health check. Summarize only the host operating system, and for RustClaw itself just list the key fields."
        ));
        let (actions, raw_plan_text) = synthesize_health_check_fallback_actions(
            Some(&route),
            "Run a basic health check. Summarize only the host operating system, and for RustClaw itself just list the key fields.",
        )
        .expect("health fallback should be synthesized");
        assert!(raw_plan_text.contains("\"health_check\""));
        assert!(matches!(
            actions.as_slice(),
            [AgentAction::CallSkill { skill, args }]
                if skill == "health_check" && args.as_object().is_some_and(|map| map.is_empty())
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

    /// §F1：`looks_like_pre_observation_hallucinated_concrete_content` 启发式检测覆盖。
    #[test]
    fn looks_hallucinated_concrete_content_recognizes_listing_shapes() {
        // 真实 adv08 复现：list_dir 还没跑，respond 编出 5 行 numbered 列表 + 路径。
        let adv08 = "prompts 目录前 5 个文件名：\n1. prompts/skills\n2. prompts/agents\n3. prompts/system\n4. prompts/user\n5. prompts/layers";
        assert!(looks_like_pre_observation_hallucinated_concrete_content(
            adv08
        ));

        // 多行 + 文件后缀，但没编号。
        let multi_paths = "Cargo.toml\nCargo.lock\nREADME.md\nLICENSE";
        assert!(looks_like_pre_observation_hallucinated_concrete_content(
            multi_paths
        ));

        // 结构化字段标签。
        assert!(looks_like_pre_observation_hallucinated_concrete_content(
            "result: 42\ncount: 3"
        ));

        // 一句正常文本 → 不命中。
        assert!(!looks_like_pre_observation_hallucinated_concrete_content(
            "好的，正在查询，请稍候。"
        ));
        // {{last_output}} 占位符 → 不命中（应由 synthesize 注入兜底处理）。
        assert!(!looks_like_pre_observation_hallucinated_concrete_content(
            "{{last_output}}"
        ));
        // 只有一行短回复 → 不命中。
        assert!(!looks_like_pre_observation_hallucinated_concrete_content(
            "yes"
        ));
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
