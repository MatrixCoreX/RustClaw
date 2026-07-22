use super::planning_action_normalization::normalize_planned_actions;
use super::planning_actions::build_plan_result_with_notes;
use super::planning_parse::parse_single_plan_actions;
use super::planning_prompt::{
    build_incremental_plan_prompt, incremental_prompt_spec, round1_prompt_spec, runtime_os_label,
    runtime_shell_label,
};
use super::planning_repair::repair_plan_actions;
use claw_core::model_turn::{
    ModelMessage, ModelRole, ModelToolCall, ModelToolChoice, ModelToolDefinition, ModelTurnRequest,
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
const NATIVE_TURN_CONTEXT_PROMPT_LOGICAL_PATH: &str = "prompts/native_turn_context.md";
const NATIVE_CALL_CAPABILITY_TOOL: &str = "call_capability";
const NATIVE_RESPOND_TOOL: &str = "respond";
const MAX_NATIVE_CONTRACT_REPAIR_ATTEMPTS: usize = 2;
const MAX_NATIVE_RESPONSE_ITEMS: usize = 64;

fn planner_last_observation(loop_state: &LoopState) -> String {
    super::observed_output::latest_structured_capability_observation(loop_state)
        .or_else(|| {
            loop_state
                .last_output
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(crate::truncate_for_log)
        })
        .or_else(|| loop_state.delivery_messages.last().cloned())
        .unwrap_or_default()
}

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

    fn callable_capability_names(&self) -> Vec<String> {
        crate::capability_map::planner_callable_capability_names_for_task(self.state, self.task)
    }

    fn callable_leaf_contracts(&self) -> String {
        crate::capability_map::planner_callable_leaf_contracts_for_task(self.state, self.task)
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
        let last_output = {
            let observation = planner_last_observation(loop_state);
            if observation.is_empty() {
                "(none)".to_string()
            } else {
                observation
            }
        };
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
        "{} loop_round_plan task_id={} round={} max_steps={}",
        crate::highlight_tag("loop"),
        task.task_id,
        loop_state.round_no,
        policy.max_steps
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
    let native_turn_context = crate::bootstrap::load_required_prompt_template_for_state_with_meta(
        state,
        NATIVE_TURN_CONTEXT_PROMPT_LOGICAL_PATH,
    )
    .map_err(|error| error.to_string())?;
    let native_system_prompt = crate::render_prompt_template(
        &native_protocol.template,
        &[
            ("__TOOL_SPEC__", &tool_spec_template),
            ("__SKILL_PLAYBOOKS__", skill_playbooks),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.policy.command_intent.default_locale,
            ),
            ("__AGENT_RUNTIME_IDENTITY__", &agent_runtime_identity),
            ("__RUNTIME_OS__", &runtime_os),
            ("__RUNTIME_SHELL__", &runtime_shell),
            ("__WORKSPACE_ROOT__", &workspace_root),
        ],
    );
    let native_history = if loop_state.round_no > 1 {
        build_loop_history_compact(loop_state)
    } else {
        String::new()
    };
    let native_last_output = planner_last_observation(loop_state);
    let native_user_prompt = crate::render_prompt_template(
        &native_turn_context.template,
        &[
            ("__USER_REQUEST__", &user_request_for_prompt),
            ("__GOAL__", goal),
            ("__TURN_ANALYSIS__", &turn_analysis),
            ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
            ("__ROUND__", &loop_state.round_no.to_string()),
            ("__HISTORY_COMPACT__", &native_history),
            ("__ATTEMPT_LEDGER__", &attempt_ledger),
            ("__LAST_ROUND_OUTPUT__", &native_last_output),
            ("__RECENT_ASSISTANT_REPLIES__", &recent_assistant_replies),
        ],
    );
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "native_action_protocol",
        &native_protocol.source,
        native_protocol.version.as_deref(),
        Some(loop_state.round_no),
    );
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "native_turn_context",
        &native_turn_context.source,
        native_turn_context.version.as_deref(),
        Some(loop_state.round_no),
    );
    crate::prompt_budget::publish_prompt_section_budget_report(
        state,
        task,
        "agent_loop_planner",
        &[
            crate::prompt_budget::PromptSection {
                name: "native_protocol_template",
                text: &native_protocol.template,
                cacheability: "stable_prefix",
                provenance: "prompt_registry",
                omission_reason: None,
            },
            crate::prompt_budget::PromptSection {
                name: "tool_spec",
                text: &tool_spec_template,
                cacheability: "registry_snapshot",
                provenance: "capability_registry",
                omission_reason: None,
            },
            crate::prompt_budget::PromptSection {
                name: "skill_quick_index",
                text: &skill_context.quick_index_text,
                cacheability: "registry_snapshot",
                provenance: "skill_registry",
                omission_reason: None,
            },
            crate::prompt_budget::PromptSection {
                name: "selected_skill_playbooks",
                text: &skill_context.playbook_text,
                cacheability: "task_scoped",
                provenance: "generated_skill_prompts",
                omission_reason: skill_context
                    .playbook_text
                    .is_empty()
                    .then_some("not_selected"),
            },
            crate::prompt_budget::PromptSection {
                name: "turn_context",
                text: &native_user_prompt,
                cacheability: "dynamic_turn",
                provenance: "agent_loop_state",
                omission_reason: None,
            },
        ],
    );
    let native_prompt = format!("{native_system_prompt}\n\n{native_user_prompt}");
    let native_prompt_source = format!("{}+{}", native_protocol.source, native_turn_context.source);
    let provider_timeout_seconds = loop_state
        .task_budget_slice
        .as_ref()
        .map(crate::task_budget_contract::TaskBudgetSlice::provider_call_timeout_seconds);
    let callable_capability_names = planner_tool_library.callable_capability_names();
    let callable_leaf_contracts = planner_tool_library.callable_leaf_contracts();
    let native_request = native_planner_request(
        &native_system_prompt,
        &native_user_prompt,
        provider_timeout_seconds,
        &callable_capability_names,
        &callable_leaf_contracts,
    );
    if let Some(native_turn) = llm_gateway::run_native_model_turn_with_fallback(
        state,
        task,
        &native_prompt,
        &native_prompt_source,
        &native_request,
    )
    .await?
    {
        let mut native_turn = native_turn;
        let mut repair_reason_codes = Vec::new();
        loop {
            match actions_from_native_turn(&native_turn, &callable_capability_names) {
                Ok(_) => break,
                Err(error_code) => {
                    if repair_reason_codes.len() >= MAX_NATIVE_CONTRACT_REPAIR_ATTEMPTS {
                        return Err(error_code);
                    }
                    warn!(
                        "native_plan_contract_retry task_id={} round={} attempt={} error_code={}",
                        task.task_id,
                        loop_state.round_no,
                        repair_reason_codes.len() + 1,
                        error_code
                    );
                    let repair_signal = native_contract_repair_signal(&error_code);
                    let repair_request =
                        native_contract_retry_request(&native_request, &repair_signal);
                    let repair_prompt = format!("{native_prompt}\n\n{repair_signal}");
                    let repair_source =
                        format!("{native_prompt_source}+inline:native_plan_contract_repair");
                    repair_reason_codes.push(error_code);
                    native_turn = llm_gateway::run_native_model_turn_with_fallback(
                        state,
                        task,
                        &repair_prompt,
                        &repair_source,
                        &repair_request,
                    )
                    .await?
                    .ok_or_else(|| "native_plan_contract_repair_unavailable".to_string())?;
                }
            }
        }
        let planner_notes = native_contract_repair_notes(&repair_reason_codes);
        let plan_actions = normalize_planned_actions(
            state,
            actions_from_native_turn(&native_turn, &callable_capability_names)?,
        );
        let raw_plan_text =
            serde_json::to_string(&native_turn).map_err(|error| error.to_string())?;
        let plan_result = build_plan_result_with_notes(
            goal,
            &raw_plan_text,
            PlanKind::Native,
            &plan_actions,
            &planner_notes,
        );
        log_plan_split(task, loop_state, &plan_result);
        return Ok(plan_result);
    }
    let plan_raw = llm_gateway::run_with_fallback_with_hints(
        state,
        task,
        &prompt_text,
        &prompt_source,
        crate::ChatRequestHints {
            timeout_seconds: provider_timeout_seconds,
            ..Default::default()
        },
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
            provider_timeout_seconds,
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

fn native_planner_request(
    system_prompt: &str,
    user_prompt: &str,
    provider_timeout_seconds: Option<u64>,
    callable_capability_names: &[String],
    callable_leaf_contracts: &str,
) -> ModelTurnRequest {
    let mut metadata = std::collections::BTreeMap::new();
    if let Some(timeout_seconds) = provider_timeout_seconds {
        metadata.insert(
            "provider_timeout_seconds".to_string(),
            Value::Number(timeout_seconds.into()),
        );
    }
    let mut capability_description = "runtime_callable_capability_catalog_v1.token".to_string();
    if !callable_leaf_contracts.is_empty() {
        capability_description.push_str("; runtime_leaf_capability_contracts_v1=");
        capability_description.push_str(callable_leaf_contracts);
    }
    let mut capability_schema = json!({
        "type": "string",
        "description": capability_description
    });
    if !callable_capability_names.is_empty() {
        capability_schema["enum"] = json!(callable_capability_names);
    }
    ModelTurnRequest {
        messages: vec![
            ModelMessage::text(ModelRole::System, system_prompt),
            ModelMessage::text(ModelRole::User, user_prompt),
        ],
        tools: vec![
            ModelToolDefinition {
                name: NATIVE_CALL_CAPABILITY_TOOL.to_string(),
                description: "Select a runtime capability. The runtime resolves, verifies, authorizes, and executes the call.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "required": ["capability", "args"],
                    "properties": {
                        "capability": capability_schema,
                        "args": {
                            "type": "object",
                            "description": "Structured capability arguments."
                        }
                    },
                    "additionalProperties": false
                }),
                strict: true,
            },
            ModelToolDefinition {
                name: NATIVE_RESPOND_TOOL.to_string(),
                description: "Submit the final user-visible response with a machine-verifiable response shape. For free_text, put the answer in content and use an empty items array with exact_item_count=0. For list, leave content empty and provide exactly the final list items plus their exact count.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "required": ["shape", "content", "items", "exact_item_count"],
                    "properties": {
                        "shape": {
                            "type": "string",
                            "enum": ["free_text", "list"]
                        },
                        "content": {
                            "type": "string",
                            "description": "free_text_content_or_empty_list_content"
                        },
                        "items": {
                            "type": "array",
                            "items": {"type": "string"},
                            "maxItems": MAX_NATIVE_RESPONSE_ITEMS
                        },
                        "exact_item_count": {
                            "type": "integer",
                            "minimum": 0,
                            "maximum": MAX_NATIVE_RESPONSE_ITEMS
                        }
                    },
                    "additionalProperties": false
                }),
                strict: true,
            },
        ],
        tool_choice: ModelToolChoice::Auto,
        response_schema: None,
        stream: true,
        metadata,
    }
}

fn native_contract_repair_signal(error_code: &str) -> String {
    let respond_contract_error = error_code.starts_with("native_respond_")
        || error_code == "native_plan_respond_tool_required";
    let (tool_name, required_argument_fields, next_action) = if respond_contract_error {
        (
            NATIVE_RESPOND_TOOL,
            vec!["shape", "content", "items", "exact_item_count"],
            "retry_native_respond_call",
        )
    } else {
        (
            NATIVE_CALL_CAPABILITY_TOOL,
            vec!["capability", "args"],
            "retry_native_tool_call",
        )
    };
    json!({
        "protocol_observation": {
            "status": "error",
            "error_code": error_code,
            "tool_name": tool_name,
            "required_argument_fields": required_argument_fields,
            "capability_value_source": "RUNTIME_CAPABILITY_MAP",
            "next_action": next_action
        }
    })
    .to_string()
}

fn native_contract_repair_notes(reason_codes: &[String]) -> String {
    if reason_codes.is_empty() {
        String::new()
    } else {
        format!(
            "native_contract_repair_reason_codes={}",
            reason_codes.join(",")
        )
    }
}

fn native_contract_retry_request(
    request: &ModelTurnRequest,
    repair_signal: &str,
) -> ModelTurnRequest {
    let mut request = request.clone();
    let repair_tool_name = serde_json::from_str::<Value>(repair_signal)
        .ok()
        .and_then(|value| {
            value
                .pointer("/protocol_observation/tool_name")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    if let Some(repair_tool_name) = repair_tool_name {
        request.tools.retain(|tool| tool.name == repair_tool_name);
    }
    request.tool_choice = ModelToolChoice::Required;
    request
        .messages
        .push(ModelMessage::text(ModelRole::User, repair_signal));
    request
}

fn actions_from_native_turn(
    turn: &ModelTurnResponse,
    callable_capability_names: &[String],
) -> Result<Vec<AgentAction>, String> {
    if !turn.tool_calls.is_empty() {
        let actions = turn
            .tool_calls
            .iter()
            .map(action_from_native_tool_call)
            .collect::<Result<Vec<_>, _>>()?;
        if actions.iter().any(|action| {
            matches!(
                action,
                AgentAction::CallCapability { capability, .. }
                    if !callable_capability_names.iter().any(|name| name == capability)
            )
        }) {
            return Err("native_plan_capability_not_in_runtime_catalog".to_string());
        }
        if actions
            .iter()
            .any(|action| matches!(action, AgentAction::Respond { .. }))
            && actions.len() != 1
        {
            return Err("native_respond_mixed_actions".to_string());
        }
        return Ok(actions);
    }
    let content = turn.text.trim();
    if content.is_empty() {
        return Err("native_plan_empty".to_string());
    }
    Err("native_plan_respond_tool_required".to_string())
}

fn action_from_native_tool_call(call: &ModelToolCall) -> Result<AgentAction, String> {
    match call.name.as_str() {
        NATIVE_CALL_CAPABILITY_TOOL => action_from_native_capability_call(call),
        NATIVE_RESPOND_TOOL => action_from_native_respond_call(call),
        _ => Err("native_plan_unknown_tool".to_string()),
    }
}

fn action_from_native_capability_call(call: &ModelToolCall) -> Result<AgentAction, String> {
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

fn action_from_native_respond_call(call: &ModelToolCall) -> Result<AgentAction, String> {
    let arguments = call
        .arguments
        .as_object()
        .ok_or_else(|| "native_respond_arguments_not_object".to_string())?;
    let shape = arguments
        .get("shape")
        .and_then(Value::as_str)
        .ok_or_else(|| "native_respond_shape_missing".to_string())?;
    let content = arguments
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| "native_respond_content_missing".to_string())?;
    let items = arguments
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| "native_respond_items_not_array".to_string())?;
    let exact_item_count = arguments
        .get("exact_item_count")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value <= MAX_NATIVE_RESPONSE_ITEMS)
        .ok_or_else(|| "native_respond_exact_item_count_invalid".to_string())?;

    match shape {
        "free_text" => {
            if content.trim().is_empty() {
                return Err("native_respond_free_text_empty".to_string());
            }
            if !items.is_empty() || exact_item_count != 0 {
                return Err("native_respond_free_text_contract_mismatch".to_string());
            }
            Ok(AgentAction::Respond {
                content: content.trim().to_string(),
            })
        }
        "list" => {
            if !content.trim().is_empty() {
                return Err("native_respond_list_content_not_empty".to_string());
            }
            if exact_item_count == 0 || items.len() != exact_item_count {
                return Err("native_respond_list_count_mismatch".to_string());
            }
            let items = items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::trim)
                        .filter(|item| {
                            !item.is_empty() && !item.contains('\r') && !item.contains('\n')
                        })
                        .map(ToString::to_string)
                        .ok_or_else(|| "native_respond_list_item_invalid".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let content = items
                .iter()
                .enumerate()
                .map(|(index, item)| format!("{}. {item}", index + 1))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(AgentAction::Respond { content })
        }
        _ => Err("native_respond_shape_unsupported".to_string()),
    }
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
