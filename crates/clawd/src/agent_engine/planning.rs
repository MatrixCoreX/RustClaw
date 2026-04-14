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

fn observation_only_plan_missing_user_answer(
    route_result: &RouteResult,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    if should_prefer_observed_finalize(Some(route_result), loop_state) {
        return false;
    }
    has_executable_observation_or_action(actions)
        && !has_discussion_followup_action(actions)
        && route_expects_terminal_user_answer(route_result)
        && !has_authoritative_delivery(loop_state)
}

fn should_force_actionable_plan_repair(
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
    if observation_only_plan_missing_user_answer(route_result, loop_state, actions) {
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
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    initial_actions: Option<&[AgentAction]>,
) -> &'static str {
    let Some(actions) = initial_actions else {
        return "plan_parse_failed";
    };
    let Some(route_result) = route_result else {
        return "non_actionable_plan_for_current_route";
    };
    if observation_only_plan_missing_user_answer(route_result, loop_state, actions) {
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

pub(super) async fn plan_round_actions(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    policy: &AgentLoopGuardPolicy,
    loop_state: &LoopState,
    route_result: Option<&RouteResult>,
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
        .map(|actions| {
            strip_terminal_discussion_for_observed_finalize(route_result, loop_state, actions)
        })
        .map(|actions| rewrite_service_status_probe_actions(route_result, actions));
    let needs_repair = match initial_actions.as_ref() {
        Some(actions) => should_force_actionable_plan_repair(route_result, loop_state, actions),
        None => true,
    };
    let (plan_actions, plan_kind, raw_plan_text) = if needs_repair {
        let repair_reason =
            plan_repair_reason(route_result, loop_state, initial_actions.as_deref());
        warn!(
            "plan_repair_required task_id={} round={} reason={}",
            task.task_id, loop_state.round_no, repair_reason
        );
        match repair_plan_actions(
            state,
            task,
            goal,
            user_text,
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
                            let actions = strip_terminal_discussion_for_observed_finalize(
                                route_result,
                                loop_state,
                                actions,
                            );
                            rewrite_service_status_probe_actions(route_result, actions)
                        });
                match repaired_actions {
                    Some(actions) => (actions, PlanKind::Repair, repaired),
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
    use super::{
        rewrite_service_status_probe_actions, should_force_actionable_plan_repair,
        strip_terminal_discussion_for_observed_finalize, LoopState,
    };
    use crate::{
        AgentAction, IntentOutputContract, OutputLocatorKind, OutputResponseShape,
        OutputSemanticKind, ResumeBehavior, RiskCeiling, RouteResult, RoutedMode, ScheduleKind,
    };
    use serde_json::json;

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
    fn actionable_route_repairs_respond_only_plan_before_any_observation() {
        let loop_state = LoopState::new(2);
        let actions = vec![AgentAction::Respond {
            content: "final answer".to_string(),
        }];
        assert!(should_force_actionable_plan_repair(
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
        assert!(should_force_actionable_plan_repair(
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
        assert!(!should_force_actionable_plan_repair(
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
        assert!(!should_force_actionable_plan_repair(
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
    fn chat_act_route_repairs_observation_only_plan_before_any_observation() {
        let loop_state = LoopState::new(2);
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({ "command": "ls -l Cargo.toml Cargo.lock" }),
        }];
        assert!(should_force_actionable_plan_repair(
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
        assert!(!should_force_actionable_plan_repair(
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
    fn non_scalar_route_still_repairs_after_prior_observation_when_delivery_is_empty() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({ "command": "ls -l Cargo.toml Cargo.lock" }),
        }];
        assert!(should_force_actionable_plan_repair(
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
        assert!(!should_force_actionable_plan_repair(
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
    fn file_delivery_route_allows_plain_not_found_terminal_reply() {
        let loop_state = LoopState::new(2);
        let actions = vec![AgentAction::Respond {
            content: "未找到该文件。".to_string(),
        }];
        assert!(!should_force_actionable_plan_repair(
            Some(&delivery_route_result()),
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
        assert!(!should_force_actionable_plan_repair(
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
