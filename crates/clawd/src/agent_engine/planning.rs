use super::planning_action_normalization::normalize_planned_actions;
use super::planning_actions::build_plan_result_with_notes;
use super::planning_parse::parse_single_plan_actions;
use super::planning_prompt::{
    build_incremental_plan_prompt, incremental_prompt_spec, round1_prompt_spec, runtime_os_label,
    runtime_shell_label,
};
use super::planning_repair::repair_plan_actions;
use claw_core::model_turn::{
    ModelMessage, ModelRole, ModelToolCall, ModelToolDefinition, ModelTurnRequest,
    ModelTurnResponse,
};
use serde_json::{json, Value};
use tracing::{info, warn};

use super::{
    attempt_ledger::build_attempt_ledger_compact, build_loop_history_compact,
    build_planner_skill_context, build_single_plan_prompt, build_turn_analysis_prompt_block,
    AgentLoopGuardPolicy, LoopState,
};
use crate::{llm_gateway, AgentAction, AppState, ClaimedTask, PlanKind, PlanResult};

const NATIVE_ACTION_PROTOCOL_PROMPT_LOGICAL_PATH: &str = "prompts/native_action_protocol.md";

/// Planner-visible tool and skill inventory for one loop round.
///
/// This helper only prepares prompt/tool-library material. It must not build a
/// `PlanResult`, choose a capability, or short-circuit the planner LLM.
struct PlannerToolLibrary<'a> {
    state: &'a AppState,
    task: &'a ClaimedTask,
}

impl<'a> PlannerToolLibrary<'a> {
    fn new(state: &'a AppState, task: &'a ClaimedTask) -> Self {
        Self { state, task }
    }

    fn skill_context(
        &self,
        loop_state: &LoopState,
    ) -> super::planner_skill_context::PlannerSkillContext {
        build_planner_skill_context(self.state, self.task, loop_state)
    }

    fn tool_spec(&self) -> Result<String, String> {
        let capability_map =
            crate::capability_map::build_compact_capability_map_for_task(self.state, self.task);
        Ok(format!("runtime_capability_map_v2\n{capability_map}"))
    }
}

pub(super) async fn plan_round_actions(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    policy: &AgentLoopGuardPolicy,
    loop_state: &LoopState,
    turn_analysis_for_prompt: Option<&crate::turn_context::TurnAnalysis>,
    boundary_envelope_for_prompt: Option<&crate::turn_boundary_envelope::TurnBoundaryEnvelope>,
    _auto_locator_path: Option<&str>,
) -> Result<PlanResult, String> {
    let runtime_os = runtime_os_label();
    let runtime_shell = runtime_shell_label();
    let workspace_root = state.skill_rt.workspace_root.display().to_string();
    let agent_runtime_identity = state.agent_runtime_identity_label().to_string();
    let recent_assistant_replies = crate::memory::build_recent_assistant_replies_context(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        3,
        220,
    );
    let planner_tool_library = PlannerToolLibrary::new(state, task);
    let skill_context = planner_tool_library.skill_context(loop_state);
    let skill_playbooks = &skill_context.text;
    let tool_spec_template = planner_tool_library.tool_spec()?;
    let turn_analysis =
        build_turn_analysis_prompt_block(turn_analysis_for_prompt, boundary_envelope_for_prompt);
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_text);
    let user_request_for_prompt =
        crate::language_policy::task_user_request_for_prompt(task, user_text);
    let attempt_ledger = build_attempt_ledger_compact(loop_state);
    let (prompt_name, prompt_source, prompt_version, prompt_text) = if loop_state.round_no <= 1 {
        let (prompt_name, prompt_logical_path) = round1_prompt_spec();
        let resolved = crate::bootstrap::load_required_prompt_template_for_state_with_meta(
            state,
            prompt_logical_path,
        )
        .map_err(|e| e.to_string())?;
        (
            prompt_name,
            resolved.source,
            resolved.version,
            build_single_plan_prompt(
                &resolved.template,
                &user_request_for_prompt,
                goal,
                &turn_analysis,
                &tool_spec_template,
                &skill_playbooks,
                &recent_assistant_replies,
                &request_language_hint,
                &state.policy.command_intent.default_locale,
                &agent_runtime_identity,
                &runtime_os,
                &runtime_shell,
                &workspace_root,
            ),
        )
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
        let (prompt_name, prompt_logical_path) = incremental_prompt_spec();
        let resolved = crate::bootstrap::load_required_prompt_template_for_state_with_meta(
            state,
            prompt_logical_path,
        )
        .map_err(|e| e.to_string())?;
        (
            prompt_name,
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
                &agent_runtime_identity,
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
        "plan_llm_request task_id={} round={} planner_mode=agent_loop prompt_chars={} tool_spec_chars={} skill_context_chars={} skill_context_mode={} selected_skills={} quick_index_chars={} playbook_chars={} recent_replies_chars={} user_request={}",
        task.task_id,
        loop_state.round_no,
        prompt_text.chars().count(),
        tool_spec_template.chars().count(),
        skill_playbooks.chars().count(),
        skill_context.disclosure_mode,
        skill_context.selected_skills.join(","),
        skill_context.quick_index_chars,
        skill_context.playbook_chars,
        recent_assistant_replies.chars().count(),
        crate::truncate_for_log(user_text)
    );
    let native_protocol = crate::bootstrap::load_required_prompt_template_for_state_with_meta(
        state,
        NATIVE_ACTION_PROTOCOL_PROMPT_LOGICAL_PATH,
    )
    .map_err(|error| error.to_string())?;
    let native_prompt = format!("{prompt_text}\n\n{}", native_protocol.template);
    let native_request = native_planner_request(&native_prompt);
    if let Some(native_turn) = llm_gateway::run_native_model_turn_with_fallback(
        state,
        task,
        &native_prompt,
        &prompt_source,
        &native_request,
    )
    .await?
    {
        let plan_actions =
            normalize_planned_actions(state, actions_from_native_turn(&native_turn)?);
        let raw_plan_text =
            serde_json::to_string(&native_turn).map_err(|error| error.to_string())?;
        let plan_result =
            build_plan_result_with_notes(goal, &raw_plan_text, PlanKind::Native, &plan_actions, "");
        log_plan_split(task, loop_state, &plan_result);
        return Ok(plan_result);
    }
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
        .map(|actions| normalize_planned_actions(state, actions));
    let (plan_actions, plan_kind, raw_plan_text, planner_notes) = if initial_actions.is_none() {
        let repair_reason = "plan_parse_failed";
        warn!(
            "plan_repair_required task_id={} round={} reason={}",
            task.task_id, loop_state.round_no, repair_reason
        );
        let repaired = repair_plan_actions(
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
        .await?;
        let repaired_actions = parse_single_plan_actions(&repaired, state, task)
            .await
            .map(|actions| normalize_planned_actions(state, actions));
        if let Some(actions) = repaired_actions {
            (
                actions,
                PlanKind::Repair,
                repaired,
                planner_notes_for_repair_success(repair_reason),
            )
        } else {
            return Err("plan_parse_failed_no_executable_steps".to_string());
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
            String::new(),
        )
    };
    let plan_result = build_plan_result_with_notes(
        goal,
        &raw_plan_text,
        plan_kind,
        &plan_actions,
        &planner_notes,
    );
    log_plan_split(task, loop_state, &plan_result);
    Ok(plan_result)
}

fn native_planner_request(prompt: &str) -> ModelTurnRequest {
    ModelTurnRequest {
        messages: vec![ModelMessage::text(ModelRole::User, prompt)],
        tools: vec![ModelToolDefinition {
            name: "call_capability".to_string(),
            description: "Select a runtime capability. The runtime resolves, verifies, authorizes, and executes the call.".to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["capability", "args"],
                "properties": {
                    "capability": {
                        "type": "string",
                        "description": "Registry capability name from the supplied runtime capability map."
                    },
                    "args": {
                        "type": "object",
                        "description": "Structured capability arguments."
                    }
                },
                "additionalProperties": false
            }),
            strict: true,
        }],
        response_schema: None,
        stream: true,
        metadata: Default::default(),
    }
}

fn actions_from_native_turn(turn: &ModelTurnResponse) -> Result<Vec<AgentAction>, String> {
    if !turn.tool_calls.is_empty() {
        return turn
            .tool_calls
            .iter()
            .map(action_from_native_tool_call)
            .collect();
    }
    let content = turn.text.trim();
    if content.is_empty() {
        return Err("native_plan_empty".to_string());
    }
    Ok(vec![AgentAction::Respond {
        content: content.to_string(),
    }])
}

fn action_from_native_tool_call(call: &ModelToolCall) -> Result<AgentAction, String> {
    if call.name != "call_capability" {
        return Err("native_plan_unknown_tool".to_string());
    }
    let arguments = call
        .arguments
        .as_object()
        .ok_or_else(|| "native_plan_arguments_not_object".to_string())?;
    let capability = arguments
        .get("capability")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "native_plan_capability_missing".to_string())?;
    let args = arguments
        .get("args")
        .cloned()
        .filter(Value::is_object)
        .ok_or_else(|| "native_plan_args_not_object".to_string())?;
    Ok(AgentAction::CallCapability {
        capability: capability.to_string(),
        args,
    })
}

fn log_plan_split(task: &ClaimedTask, loop_state: &LoopState, plan_result: &PlanResult) {
    let labels = plan_result.step_labels();
    info!(
        "act_split_trace task_id={} round={} split_steps={}",
        task.task_id,
        loop_state.round_no,
        serde_json::to_string(&labels).unwrap_or_else(|_| "[]".to_string())
    );
}

fn planner_notes_for_repair_success(repair_reason: &str) -> String {
    format!("repair_reason_code={repair_reason}")
}

#[cfg(test)]
#[path = "planning_native_tests.rs"]
mod native_tests;
