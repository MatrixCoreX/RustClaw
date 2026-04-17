use serde_json::Value;
use tracing::{debug, info, warn};

use super::{
    build_loop_history_compact, build_single_plan_prompt, build_skill_playbooks_text,
    build_skill_quick_index_text, plan_step_label, AgentLoopGuardPolicy, LoopState,
    SinglePlanEnvelope, AGENT_TOOL_SPEC_PATH, AGENT_TOOL_SPEC_TEMPLATE,
    LOOP_INCREMENTAL_PLAN_PROMPT_LOGICAL_PATH, LOOP_INCREMENTAL_PLAN_PROMPT_TEMPLATE,
    PLAN_REPAIR_PROMPT_LOGICAL_PATH, PLAN_REPAIR_PROMPT_TEMPLATE,
    SINGLE_PLAN_EXECUTION_PROMPT_LOGICAL_PATH, SINGLE_PLAN_EXECUTION_PROMPT_TEMPLATE,
};
use crate::{
    llm_gateway, plan_step_from_agent_action, AgentAction, AppState, ClaimedTask, PlanKind,
    PlanResult, RouteResult, RoutedMode,
};

fn build_incremental_plan_prompt(
    prompt_template: &str,
    user_request: &str,
    goal: &str,
    tool_spec: &str,
    skill_playbooks: &str,
    recent_assistant_replies: &str,
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
            ("__TOOL_SPEC__", tool_spec),
            ("__SKILL_PLAYBOOKS__", skill_playbooks),
            ("__RECENT_ASSISTANT_REPLIES__", recent_assistant_replies),
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
    if let Some(value) = crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<Value>(raw) {
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
                if !actions.is_empty()
                    && crate::semantic_judge::is_meta_respond_instruction(state, task, &content)
                        .await
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
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
        )
    })
}

fn has_discussion_followup_action(actions: &[AgentAction]) -> bool {
    actions.iter().any(|action| match action {
        AgentAction::Respond { .. } => true,
        AgentAction::CallSkill { skill, .. } | AgentAction::CallTool { tool: skill, .. } => {
            skill.eq_ignore_ascii_case("chat")
        }
        AgentAction::Think { .. } => false,
    })
}

fn is_discussion_followup_action(action: &AgentAction) -> bool {
    match action {
        AgentAction::Respond { .. } => true,
        AgentAction::CallSkill { skill, .. } | AgentAction::CallTool { tool: skill, .. } => {
            skill.eq_ignore_ascii_case("chat")
        }
        AgentAction::Think { .. } => false,
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
            .last_publishable_chat_output
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
    !trimmed.is_empty() && crate::finalizer::parse_delivery_token(trimmed).is_none()
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
            if canonical == "run_cmd" && route_explicitly_requests_raw_command_output(route_result) {
                return true;
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
    if !should_prefer_observed_finalize(route_result, loop_state)
        || loop_state.has_tool_or_skill_output
        || !has_executable_observation_or_action(&actions)
        || !has_discussion_followup_action(&actions)
    {
        return actions;
    }
    let mut stripped = actions.clone();
    while stripped.last().is_some_and(is_discussion_followup_action) {
        stripped.pop();
    }
    if has_executable_observation_or_action(&stripped) && !has_discussion_followup_action(&stripped)
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

fn should_rewrite_service_status_run_cmd_probe(
    route_result: Option<&RouteResult>,
    actions: &[AgentAction],
) -> bool {
    let Some(route_result) = route_result else {
        return false;
    };
    if route_result.needs_clarify
        || route_result.output_contract.delivery_required
        || !matches!(
            route_result.routed_mode,
            RoutedMode::Act | RoutedMode::ChatAct
        )
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
        || observation_only_plan_can_finalize_from_direct_output(
            state,
            Some(route_result),
            actions,
        )
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
    if observation_only_plan_missing_user_answer(state, route_result, loop_state, actions) {
        return true;
    }
    if has_executable_observation_or_action(actions) {
        return false;
    }
    if has_discussion_followup_action(actions) && loop_state.has_tool_or_skill_output {
        return false;
    }
    let requires_action_before_reply = !loop_state.has_tool_or_skill_output
        && matches!(
            route_result.routed_mode,
            RoutedMode::Act | RoutedMode::ChatAct
        );
    route_result.output_contract.requires_content_evidence || requires_action_before_reply
}

async fn repair_plan_actions(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    repair_reason: &str,
    tool_spec: &str,
    skill_playbooks: &str,
    raw_plan: &str,
    round_no: usize,
) -> Result<String, String> {
    let runtime_os = runtime_os_label();
    let runtime_shell = runtime_shell_label();
    let workspace_root = state.workspace_root.display().to_string();
    let (prompt_template, prompt_source) = crate::load_prompt_template_for_state(
        state,
        PLAN_REPAIR_PROMPT_LOGICAL_PATH,
        PLAN_REPAIR_PROMPT_TEMPLATE,
    );
    let prompt = crate::render_prompt_template(
        &prompt_template,
        &[
            ("__GOAL__", goal),
            ("__USER_REQUEST__", user_text),
            ("__REPAIR_REASON__", repair_reason),
            ("__TOOL_SPEC__", tool_spec),
            ("__SKILL_PLAYBOOKS__", skill_playbooks),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.command_intent.default_locale,
            ),
            ("__RUNTIME_OS__", &runtime_os),
            ("__RUNTIME_SHELL__", &runtime_shell),
            ("__WORKSPACE_ROOT__", &workspace_root),
            ("__RAW_PLAN__", raw_plan),
        ],
    );
    crate::log_prompt_render(
        state,
        &task.task_id,
        "plan_repair_prompt",
        &prompt_source,
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
    let Some(route_result) = route_result else {
        return "non_actionable_plan_for_current_route";
    };
    if observation_only_plan_missing_user_answer(state, route_result, loop_state, actions) {
        return "plan_missing_terminal_user_answer";
    }
    "non_actionable_plan_for_current_route"
}

fn can_fallback_to_initial_plan_after_repair_failure(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    let Some(route_result) = route_result else {
        return false;
    };
    !route_result.needs_clarify
        && !loop_state.has_tool_or_skill_output
        && has_executable_observation_or_action(actions)
        && !has_discussion_followup_action(actions)
}

fn normalize_planned_actions(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let actions =
        strip_terminal_discussion_for_observed_finalize(route_result, loop_state, actions);
    let actions =
        strip_terminal_discussion_for_direct_skill_passthrough(state, route_result, actions);
    let actions = rewrite_service_status_probe_actions(route_result, actions);
    rewrite_http_probe_actions(route_result, actions)
}

pub(super) async fn plan_round_actions(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    policy: &AgentLoopGuardPolicy,
    loop_state: &LoopState,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Result<PlanResult, String> {
    let runtime_os = runtime_os_label();
    let runtime_shell = runtime_shell_label();
    let workspace_root = state.workspace_root.display().to_string();
    let recent_assistant_replies = crate::memory::build_recent_assistant_replies_context(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        3,
        220,
    );
    let skill_playbooks = build_skill_playbooks_text(state, task);
    let skill_quick_index = build_skill_quick_index_text(state, task);
    let (tool_spec_template, _) = crate::load_prompt_template_for_state(
        state,
        AGENT_TOOL_SPEC_PATH,
        AGENT_TOOL_SPEC_TEMPLATE,
    );
    let (prompt_name, prompt_source, prompt_text) = if loop_state.round_no <= 1 {
        let (prompt_template, prompt_source) = crate::load_prompt_template_for_state(
            state,
            SINGLE_PLAN_EXECUTION_PROMPT_LOGICAL_PATH,
            SINGLE_PLAN_EXECUTION_PROMPT_TEMPLATE,
        );
        (
            "single_plan_execution_prompt",
            prompt_source,
            format!(
                "{}\n\n## Skill Quick Index (first-round routing hint)\nGoal: reduce misclassification while minimizing avoidable extra rounds.\n- Do NOT end round-1 with a generic chat-style final answer when a skill might be relevant.\n- In round-1, prioritize intent classification + missing-slot check, but finish immediately when one bounded resolution/current-runtime step can already complete the request safely.\n- Ask one concise clarification only when safe completion is truly blocked after current-turn text, immediate context, and bounded resolution/default inference have been used.\n- Use immediate `call_skill` in round-1 whenever intent is clear or can be completed by one bounded resolution/current-runtime step.\n{}\n",
                build_single_plan_prompt(
                    &prompt_template,
                    user_text,
                    goal,
                    &tool_spec_template,
                    &skill_playbooks,
                    &recent_assistant_replies,
                    &state.command_intent.default_locale,
                    &runtime_os,
                    &runtime_shell,
                    &workspace_root,
                ),
                skill_quick_index
            ),
        )
    } else {
        let history_compact = build_loop_history_compact(loop_state);
        let last_output = loop_state
            .delivery_messages
            .last()
            .cloned()
            .unwrap_or_else(|| "(none)".to_string());
        let (prompt_template, prompt_source) = crate::load_prompt_template_for_state(
            state,
            LOOP_INCREMENTAL_PLAN_PROMPT_LOGICAL_PATH,
            LOOP_INCREMENTAL_PLAN_PROMPT_TEMPLATE,
        );
        (
            "loop_incremental_plan_prompt",
            prompt_source,
            build_incremental_plan_prompt(
                &prompt_template,
                user_text,
                goal,
                &tool_spec_template,
                &skill_playbooks,
                &recent_assistant_replies,
                &state.command_intent.default_locale,
                loop_state.round_no,
                &history_compact,
                &last_output,
                &runtime_os,
                &runtime_shell,
                &workspace_root,
            ),
        )
    };
    crate::log_prompt_render(
        state,
        &task.task_id,
        prompt_name,
        &prompt_source,
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
        "plan_llm_request task_id={} round={} user_request={}",
        task.task_id,
        loop_state.round_no,
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
        .map(|actions| normalize_planned_actions(state, route_result, loop_state, actions));
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
        // Phase 1.1: LLM 修复之前先尝试确定性兜底
        // (`synthesize_plan_repair_fallback_actions`)。命中时直接复用，避免
        // 一次（可能两次）的 `plan_repair_prompt` LLM 调用。
        // 只有在确定性合成"找不到可用模式"或产物仍被判定需要修复时，
        // 才回退到原先的 LLM 修复链路。
        let deterministic_hit = synthesize_plan_repair_fallback_actions(
            loop_state,
            route_result,
            user_text,
            auto_locator_path,
        )
        .map(|(actions, raw_plan)| {
            (
                normalize_planned_actions(state, route_result, loop_state, actions),
                raw_plan,
            )
        })
        .filter(|(actions, _)| {
            !should_force_actionable_plan_repair(state, route_result, loop_state, actions)
        });
        if let Some((deterministic_actions, deterministic_raw_plan)) = deterministic_hit {
            info!(
                "plan_repair_deterministic_hit task_id={} round={} reason={}",
                task.task_id, loop_state.round_no, repair_reason
            );
            (deterministic_actions, PlanKind::Repair, deterministic_raw_plan)
        } else {
            match repair_plan_actions(
            state,
            task,
            goal,
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
                            normalize_planned_actions(state, route_result, loop_state, actions)
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
    use std::sync::{Arc, Mutex, RwLock};
    use std::time::Instant;

    use claw_core::config::{
        AgentConfig, MaintenanceConfig, MemoryConfig, RoutingConfig, ToolsConfig,
    };
    use rusqlite::Connection;
    use tokio::sync::Semaphore;

    use super::{
        looks_health_check_request, plan_repair_reason, rewrite_http_probe_actions,
        rewrite_service_status_probe_actions, should_force_actionable_plan_repair,
        strip_terminal_discussion_for_direct_skill_passthrough,
        strip_terminal_discussion_for_observed_finalize, synthesize_health_check_fallback_actions,
        synthesize_ops_http_repair_fallback_actions, LoopState,
    };
    use crate::{
        AgentAction, AgentRuntimeConfig, AppState, CommandIntentRuntime, IntentOutputContract,
        OutputLocatorKind, OutputResponseShape, OutputSemanticKind, RateLimiter, ResumeBehavior,
        RiskCeiling, RouteResult, RoutedMode, ScheduleKind, ScheduleRuntime, SkillViewsSnapshot,
        ToolsPolicy, DEFAULT_AGENT_ID,
    };
    use serde_json::json;

    fn test_state() -> AppState {
        let agents_by_id = HashMap::from([(
            DEFAULT_AGENT_ID.to_string(),
            AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
        )]);
        AppState {
            started_at: Instant::now(),
            queue_limit: 1,
            db: Arc::new(Mutex::new(Connection::open_in_memory().expect("open db"))),
            llm_providers: Vec::new(),
            agents_by_id: Arc::new(agents_by_id),
            skill_timeout_seconds: 30,
            skill_runner_path: std::path::PathBuf::new(),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                registry: None,
                skills_list: Arc::new(HashSet::new()),
            }))),
            skill_semaphore: Arc::new(Semaphore::new(1)),
            rate_limiter: Arc::new(Mutex::new(RateLimiter::new(60, 30))),
            llm_calls_per_task: Arc::new(Mutex::new(HashMap::new())),
            llm_elapsed_per_task: Arc::new(Mutex::new(HashMap::new())),
            task_schedule_intent_cache: Arc::new(Mutex::new(HashMap::new())),
            maintenance: MaintenanceConfig::default(),
            memory: MemoryConfig::default(),
            workspace_root: std::env::temp_dir(),
            default_locator_search_dir: std::env::temp_dir(),
            locator_scan_max_depth: 2,
            locator_scan_max_files: 200,
            tools_policy: Arc::new(
                ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
            ),
            active_provider_type: None,
            cmd_timeout_seconds: 30,
            max_cmd_length: 4096,
            allow_path_outside_workspace: false,
            allow_sudo: false,
            worker_task_timeout_seconds: 300,
            worker_task_heartbeat_seconds: 10,
            worker_running_no_progress_timeout_seconds: 300,
            worker_running_recovery_check_interval_seconds: 30,
            last_running_recovery_check_ts: Arc::new(Mutex::new(0)),
            routing: RoutingConfig::default(),
            persona_prompt: String::new(),
            command_intent: CommandIntentRuntime {
                all_result_suffixes: Vec::new(),
                default_locale: "zh-CN".to_string(),
                verify_enforce_enabled: false,
            },
            schedule: ScheduleRuntime {
                timezone: "Asia/Shanghai".to_string(),
                intent_prompt_template: String::new(),
                intent_prompt_source: String::new(),
                intent_rules_template: String::new(),
                locale: "zh-CN".to_string(),
                i18n_dict: HashMap::new(),
            },
            telegram_bot_token: String::new(),
            telegram_configured_bot_names: Arc::new(Vec::new()),
            whatsapp_cloud_enabled: false,
            whatsapp_api_base: String::new(),
            whatsapp_access_token: String::new(),
            whatsapp_phone_number_id: String::new(),
            whatsapp_web_enabled: false,
            whatsapp_web_bridge_base_url: String::new(),
            future_adapters_enabled: Arc::new(Vec::new()),
            wechat_send_config: None,
            feishu_send_config: None,
            lark_send_config: None,
            http_client: reqwest::Client::new(),
            database_sqlite_path: std::path::PathBuf::new(),
            database_busy_timeout_ms: 5_000,
            config_path_for_reload: String::new(),
            self_extension: claw_core::config::SelfExtensionConfig::default(),
            registry_path_for_reload: None,
            skill_switches_for_reload: Arc::new(HashMap::new()),
            initial_skills_list_for_reload: Vec::new(),
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
    fn content_evidence_route_strips_terminal_discussion_followup_before_observation() {
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
            AgentAction::CallSkill {
                skill: "chat".to_string(),
                args: json!({ "text": "summarize {{last_output}}" }),
            },
        ];
        let stripped = strip_terminal_discussion_for_observed_finalize(
            Some(&route_result(
                RoutedMode::ChatAct,
                true,
                OutputResponseShape::Free,
            )),
            &loop_state,
            actions,
        );
        assert_eq!(stripped.len(), 1);
        assert!(matches!(
            &stripped[0],
            AgentAction::CallSkill { skill, .. } if skill == "system_basic"
        ));
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
    fn chat_act_route_keeps_observation_plus_chat_followup_plan() {
        let loop_state = LoopState::new(2);
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: serde_json::json!({ "command": "ls -l Cargo.toml Cargo.lock" }),
            },
            AgentAction::CallSkill {
                skill: "chat".to_string(),
                args: serde_json::json!({ "text": "explain {{last_output}}" }),
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
}
