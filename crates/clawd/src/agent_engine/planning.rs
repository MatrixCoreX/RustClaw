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
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
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
const MAX_NATIVE_RESPONSE_FIELDS: usize = 64;
const MAX_NATIVE_RESPONSE_SOURCE_PATH: usize = 160;
const MAX_NATIVE_TOOL_NAME_BYTES: usize = 64;

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

    fn tool_spec(&self, loop_state: &LoopState) -> Result<String, String> {
        let capability_map = crate::capability_map::build_scoped_compact_capability_map_for_task(
            self.state,
            self.task,
            &loop_state.loaded_capability_skills,
            &loop_state.loaded_mcp_capabilities,
        );
        Ok(format!("runtime_capability_map_v2\n{capability_map}"))
    }

    fn callable_capability_names(&self, loop_state: &LoopState) -> Vec<String> {
        crate::capability_map::planner_callable_capability_names_for_task_with_mcp(
            self.state,
            self.task,
            &loop_state.loaded_mcp_capabilities,
        )
    }

    fn mcp_capability_argument_schemas(&self, loop_state: &LoopState) -> BTreeMap<String, Value> {
        crate::capability_map::planner_mcp_tools_for_task(
            self.state,
            self.task,
            &loop_state.loaded_mcp_capabilities,
        )
        .into_iter()
        .map(|tool| (tool.capability, tool.input_schema))
        .collect()
    }

    fn all_native_capability_groups(
        &self,
    ) -> Vec<crate::capability_map::PlannerNativeCapabilityGroup> {
        crate::capability_map::planner_native_capability_groups_for_task(self.state, self.task)
    }

    fn disclosed_native_capability_groups(
        &self,
        loop_state: &LoopState,
    ) -> Vec<crate::capability_map::PlannerNativeCapabilityGroup> {
        crate::capability_map::planner_disclosed_native_capability_groups_for_task(
            self.state,
            self.task,
            &loop_state.loaded_capability_skills,
        )
    }

    fn loadable_capability_group_names(&self, loop_state: &LoopState) -> Vec<String> {
        crate::capability_map::planner_loadable_capability_group_names_for_task(
            self.state,
            self.task,
            &loop_state.loaded_capability_skills,
        )
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
    let tool_spec_template = planner_tool_library.tool_spec(loop_state)?;
    let turn_analysis =
        build_turn_analysis_prompt_block(turn_analysis_for_prompt, boundary_envelope_for_prompt);
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_text);
    let user_request_for_prompt =
        crate::language_policy::task_user_request_for_prompt(task, user_text);
    let attempt_ledger = build_attempt_ledger_compact(loop_state);
    info!(
        "{} loop_round_plan task_id={} round={} max_actions_per_turn={}",
        crate::highlight_tag("loop"),
        task.task_id,
        loop_state.round_no,
        policy.max_actions_per_turn
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
    info!(
        "plan_llm_request task_id={} round={} planner_mode=native_tools prompt_chars={} tool_spec_chars={} skill_context_chars={} skill_context_mode={} selected_skills={} quick_index_chars={} playbook_chars={} recent_replies_chars={} user_request={}",
        task.task_id,
        loop_state.round_no,
        native_prompt.chars().count(),
        tool_spec_template.chars().count(),
        skill_playbooks.chars().count(),
        skill_context.disclosure_mode,
        skill_context.selected_skills.join(","),
        skill_context.quick_index_chars,
        skill_context.playbook_chars,
        recent_assistant_replies.chars().count(),
        crate::truncate_for_log(user_text)
    );
    let provider_timeout_seconds = loop_state
        .task_budget_slice
        .as_ref()
        .map(crate::task_budget_contract::TaskBudgetSlice::provider_call_timeout_seconds);
    let callable_capability_names = planner_tool_library.callable_capability_names(loop_state);
    let mcp_capability_argument_schemas =
        planner_tool_library.mcp_capability_argument_schemas(loop_state);
    let all_native_capability_groups = planner_tool_library.all_native_capability_groups();
    let native_capability_groups =
        planner_tool_library.disclosed_native_capability_groups(loop_state);
    let mut native_capability_argument_schemas = mcp_capability_argument_schemas.clone();
    for group in &native_capability_groups {
        native_capability_argument_schemas.extend(group.capability_argument_schemas.clone());
    }
    let loadable_capability_group_names =
        planner_tool_library.loadable_capability_group_names(loop_state);
    let native_capability_group_map = native_capability_tool_map(&native_capability_groups);
    let native_callable_capability_names = disclosed_callable_capability_names(
        &callable_capability_names,
        &all_native_capability_groups,
        &native_capability_groups,
    );
    let selected_native_group_count = native_capability_groups
        .iter()
        .filter(|group| {
            loop_state
                .loaded_capability_skills
                .contains(&group.skill_name)
        })
        .count();
    let eager_native_group_count = native_capability_groups
        .len()
        .saturating_sub(selected_native_group_count);
    let native_request = native_planner_request(
        &native_system_prompt,
        &native_user_prompt,
        provider_timeout_seconds,
        &callable_capability_names,
        &mcp_capability_argument_schemas,
        &all_native_capability_groups,
        &native_capability_groups,
        &loadable_capability_group_names,
    );
    crate::prompt_budget::publish_model_tool_surface_budget_report(
        state,
        task,
        "agent_loop_planner",
        &native_request.tools,
        native_callable_capability_names.len(),
        eager_native_group_count,
        selected_native_group_count,
        skill_context.disclosure_mode,
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
            match actions_from_native_turn_with_schemas(
                &native_turn,
                &native_callable_capability_names,
                &native_capability_group_map,
                &native_capability_argument_schemas,
                Some(loop_state),
            ) {
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
                    let repair_signal = native_contract_repair_signal_for_turn(
                        &error_code,
                        &native_turn,
                        &native_request,
                        &native_capability_group_map,
                        &native_capability_argument_schemas,
                        Some(loop_state),
                        &native_callable_capability_names,
                    );
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
            actions_from_native_turn_with_schemas(
                &native_turn,
                &callable_capability_names,
                &native_capability_group_map,
                &native_capability_argument_schemas,
                Some(loop_state),
            )?,
        );
        let raw_plan_text =
            serde_json::to_string(&native_turn).map_err(|error| error.to_string())?;
        let plan_result = build_plan_result_with_notes(
            Some(state),
            goal,
            &raw_plan_text,
            PlanKind::Native,
            &plan_actions,
            &planner_notes,
        );
        log_plan_split(task, loop_state, &plan_result);
        return Ok(plan_result);
    }
    let (prompt_name, prompt_source, prompt_version, prompt_text) = if loop_state.round_no <= 1 {
        let (prompt_name, prompt_logical_path) = round1_prompt_spec();
        let resolved = crate::bootstrap::load_required_prompt_template_for_state_with_meta(
            state,
            prompt_logical_path,
        )
        .map_err(|error| error.to_string())?;
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
                skill_playbooks,
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
        .map_err(|error| error.to_string())?;
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
                skill_playbooks,
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
        "plan_llm_request task_id={} round={} planner_mode=text_json_fallback prompt_chars={} tool_spec_chars={} skill_context_chars={} skill_context_mode={} selected_skills={} quick_index_chars={} playbook_chars={} recent_replies_chars={} user_request={}",
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
        Some(state),
        goal,
        &raw_plan_text,
        plan_kind,
        &plan_actions,
        &planner_notes,
    );
    log_plan_split(task, loop_state, &plan_result);
    Ok(plan_result)
}

fn disclosed_callable_capability_names(
    callable_capability_names: &[String],
    all_native_capability_groups: &[crate::capability_map::PlannerNativeCapabilityGroup],
    disclosed_native_capability_groups: &[crate::capability_map::PlannerNativeCapabilityGroup],
) -> Vec<String> {
    let registry_capabilities = all_native_capability_groups
        .iter()
        .flat_map(|group| group.capability_names.iter().cloned())
        .collect::<BTreeSet<_>>();
    let mut disclosed = callable_capability_names
        .iter()
        .filter(|name| !registry_capabilities.contains(*name))
        .cloned()
        .collect::<BTreeSet<_>>();
    disclosed.extend(
        disclosed_native_capability_groups
            .iter()
            .flat_map(|group| group.capability_names.iter().cloned()),
    );
    disclosed.into_iter().collect()
}

fn native_planner_request(
    system_prompt: &str,
    user_prompt: &str,
    provider_timeout_seconds: Option<u64>,
    callable_capability_names: &[String],
    ungrouped_capability_argument_schemas: &BTreeMap<String, Value>,
    all_native_capability_groups: &[crate::capability_map::PlannerNativeCapabilityGroup],
    native_capability_groups: &[crate::capability_map::PlannerNativeCapabilityGroup],
    loadable_capability_group_names: &[String],
) -> ModelTurnRequest {
    let mut metadata = std::collections::BTreeMap::new();
    if let Some(timeout_seconds) = provider_timeout_seconds {
        metadata.insert(
            "provider_timeout_seconds".to_string(),
            Value::Number(timeout_seconds.into()),
        );
    }
    let grouped_capability_names = all_native_capability_groups
        .iter()
        .flat_map(|group| group.capability_names.iter().cloned())
        .collect::<BTreeSet<_>>();
    let ungrouped_capability_names = callable_capability_names
        .iter()
        .filter(|name| !grouped_capability_names.contains(*name))
        .cloned()
        .collect::<Vec<_>>();
    let mut tools = Vec::new();
    if !ungrouped_capability_names.is_empty() {
        tools.push(native_capability_tool_definition(
            NATIVE_CALL_CAPABILITY_TOOL,
            "schema:runtime_ungrouped_capability_catalog_v1",
            &ungrouped_capability_names,
            ungrouped_capability_argument_schemas,
        ));
    }
    if !loadable_capability_group_names.is_empty() {
        tools.push(native_capability_loader_tool_definition(
            loadable_capability_group_names,
        ));
    }
    for group in native_capability_groups {
        tools.extend(native_group_tool_definitions(group));
    }
    tools.push(ModelToolDefinition {
        name: NATIVE_RESPOND_TOOL.to_string(),
        description: "Submit the final user-visible response after required observations. This tool formats answers; it does not execute or simulate runtime capabilities. Runtime-owned provider/config/permission, domain parse/normalize/validate/preview, dry-run, artifact/job, checkpoint, diff, verification, repair, and rewind fields require a prior matching capability result. A lower-level environment observation is supporting context, not a substitute for the disclosed domain capability that owns those fields. Use free_text for prose/scalars, list for exact items, object for model-authored exact named fields, or observed_object to copy selected JSON fields from successful capability results without re-serializing their values. Each object value_json contains one complete serialized JSON value and is validated before delivery; JSON string values include their surrounding JSON quotes.".to_string(),
        input_schema: json!({
            "type": "object",
            "required": [
                "shape",
                "content",
                "items",
                "exact_item_count",
                "fields",
                "observed_fields",
                "exact_field_count"
            ],
            "properties": {
                "shape": {
                    "type": "string",
                    "enum": ["free_text", "list", "object", "observed_object"]
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
                },
                "fields": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["name", "value_json"],
                        "properties": {
                            "name": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": 128
                            },
                            "value_json": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": 65536,
                                "description": "schema:complete_serialized_json_value_v1; json_string_requires_surrounding_quotes=true; malformed_json=rejected; example_json_string=\"\\\"text\\\"\"; scalar_and_composite_encoding=standard_json"
                            }
                        },
                        "additionalProperties": false
                    },
                    "maxItems": MAX_NATIVE_RESPONSE_FIELDS
                },
                "observed_fields": {
                    "type": "array",
                    "description": "schema:successful_capability_result_field_references_v1; value_source=runtime_copy; model_value_copy=forbidden",
                    "items": {
                        "type": "object",
                        "required": ["name", "capability", "path"],
                        "properties": {
                            "name": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": 128
                            },
                            "capability": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": MAX_NATIVE_RESPONSE_SOURCE_PATH,
                                "description": "source=current_loop_success; selector=exact_capability_token"
                            },
                            "path": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": MAX_NATIVE_RESPONSE_SOURCE_PATH,
                                "description": "selector=machine_dotted_json_path; roots=data,data.extra,data.output"
                            }
                        },
                        "additionalProperties": false
                    },
                    "maxItems": MAX_NATIVE_RESPONSE_FIELDS
                },
                "exact_field_count": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": MAX_NATIVE_RESPONSE_FIELDS
                }
            },
            "additionalProperties": false
        }),
        strict: true,
    });
    ModelTurnRequest {
        messages: vec![
            ModelMessage::text(ModelRole::System, system_prompt),
            ModelMessage::text(ModelRole::User, user_prompt),
        ],
        tools,
        tool_choice: ModelToolChoice::Auto,
        response_schema: None,
        stream: true,
        metadata,
    }
}

fn native_capability_loader_tool_definition(group_names: &[String]) -> ModelToolDefinition {
    ModelToolDefinition {
        name: super::capability_discovery::RUNTIME_CAPABILITY_LOADER_TOOL.to_string(),
        description: "runtime_capability_scope_loader_v2; effect=observe; selection=exact_group_token; active_set=bounded_lru; next_action=replan".to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["groups"],
            "properties": {
                "groups": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": super::capability_discovery::MAX_GROUPS_PER_LOAD,
                    "items": {
                        "type": "string",
                        "enum": group_names
                    }
                }
            },
            "additionalProperties": false
        }),
        strict: true,
    }
}

fn native_capability_tool_definition(
    tool_name: &str,
    description: &str,
    capability_names: &[String],
    capability_argument_schemas: &BTreeMap<String, Value>,
) -> ModelToolDefinition {
    if tool_name != NATIVE_CALL_CAPABILITY_TOOL {
        if let [capability] = capability_names {
            if let Some(input_schema) = capability_argument_schemas.get(capability) {
                return ModelToolDefinition {
                    name: tool_name.to_string(),
                    description: format!(
                        "{description}; schema:direct_runtime_capability_arguments_v1; capability={capability}"
                    ),
                    input_schema: input_schema.clone(),
                    strict: true,
                };
            }
        }
    }
    let input_schema = if !capability_names.is_empty()
        && capability_names
            .iter()
            .all(|name| capability_argument_schemas.contains_key(name))
    {
        let variants = capability_names
            .iter()
            .map(|capability| {
                json!({
                    "type": "object",
                    "required": ["capability", "args"],
                    "properties": {
                        "capability": {
                            "type": "string",
                            "enum": [capability]
                        },
                        "args": &capability_argument_schemas[capability]
                    },
                    "additionalProperties": false
                })
            })
            .collect::<Vec<_>>();
        json!({
            "type": "object",
            "description": "schema:discriminated_runtime_capability_call_v1",
            "oneOf": variants
        })
    } else {
        json!({
            "type": "object",
            "required": ["capability", "args"],
            "properties": {
                "capability": {
                    "type": "string",
                    "description": "runtime_callable_capability_catalog_v1.token",
                    "enum": capability_names
                },
                "args": {
                    "type": "object",
                    "description": "schema:structured_capability_arguments"
                }
            },
            "additionalProperties": false
        })
    };
    ModelToolDefinition {
        name: tool_name.to_string(),
        description: description.to_string(),
        input_schema,
        strict: true,
    }
}

fn native_capability_leaf_tool_name(capability: &str) -> String {
    let readable = capability
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let direct_name = format!("call_{readable}");
    if direct_name.len() <= MAX_NATIVE_TOOL_NAME_BYTES {
        return direct_name;
    }
    let digest = Sha256::digest(capability.as_bytes());
    let suffix = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let suffix = format!("__{suffix}");
    let mut prefix = direct_name;
    prefix.truncate(MAX_NATIVE_TOOL_NAME_BYTES.saturating_sub(suffix.len()));
    format!("{prefix}{suffix}")
}

fn native_group_leaf_tool_name(
    group: &crate::capability_map::PlannerNativeCapabilityGroup,
    capability: &str,
) -> String {
    if group.capability_names.len() == 1 {
        group.tool_name.clone()
    } else {
        native_capability_leaf_tool_name(capability)
    }
}

fn native_group_tool_definitions(
    group: &crate::capability_map::PlannerNativeCapabilityGroup,
) -> Vec<ModelToolDefinition> {
    group
        .capability_names
        .iter()
        .map(|capability| {
            let description = if group.capability_names.len() == 1 {
                group.description.clone()
            } else {
                format!(
                    "runtime_capability_leaf_v1; source_group={}; capability={capability}; dispatch=resolver_verifier",
                    group.skill_name
                )
            };
            native_capability_tool_definition(
                &native_group_leaf_tool_name(group, capability),
                &description,
                std::slice::from_ref(capability),
                &group.capability_argument_schemas,
            )
        })
        .collect()
}

fn native_capability_tool_map(
    groups: &[crate::capability_map::PlannerNativeCapabilityGroup],
) -> BTreeMap<String, BTreeSet<String>> {
    groups
        .iter()
        .flat_map(|group| {
            group.capability_names.iter().map(|capability| {
                (
                    native_group_leaf_tool_name(group, capability),
                    BTreeSet::from([capability.clone()]),
                )
            })
        })
        .collect()
}

#[cfg(test)]
fn native_contract_repair_signal(error_code: &str) -> String {
    native_contract_repair_signal_with_context(error_code, None, None, &[])
}

fn native_contract_repair_signal_for_turn(
    error_code: &str,
    turn: &ModelTurnResponse,
    request: &ModelTurnRequest,
    native_capability_groups: &BTreeMap<String, BTreeSet<String>>,
    capability_argument_schemas: &BTreeMap<String, Value>,
    loop_state: Option<&LoopState>,
    callable_capability_names: &[String],
) -> String {
    let failed_call = turn.tool_calls.iter().find(|call| {
        action_from_native_tool_call_with_schemas(
            call,
            native_capability_groups,
            capability_argument_schemas,
            loop_state,
        )
        .err()
        .as_deref()
            == Some(error_code)
            || (error_code == "native_plan_capability_not_in_runtime_catalog"
                && call
                    .arguments
                    .get("capability")
                    .and_then(Value::as_str)
                    .is_some_and(|capability| {
                        !callable_capability_names
                            .iter()
                            .any(|name| name == capability)
                    }))
    });
    let expected_schema = failed_call.and_then(|call| {
        request
            .tools
            .iter()
            .find(|tool| tool.name == call.name)
            .map(|tool| exact_schema_branch_for_call(&tool.input_schema, call))
    });
    let available_tool_names = if error_code == "native_plan_unknown_tool" {
        request
            .tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    native_contract_repair_signal_with_context(
        error_code,
        failed_call,
        expected_schema,
        &available_tool_names,
    )
}

fn exact_schema_branch_for_call(schema: &Value, call: &ModelToolCall) -> Value {
    let capability = call.arguments.get("capability").and_then(Value::as_str);
    schema
        .get("oneOf")
        .and_then(Value::as_array)
        .and_then(|branches| {
            branches.iter().find(|branch| {
                let Some(capability) = capability else {
                    return false;
                };
                branch
                    .pointer("/properties/capability/enum")
                    .and_then(Value::as_array)
                    .is_some_and(|values| {
                        values
                            .iter()
                            .any(|value| value.as_str() == Some(capability))
                    })
            })
        })
        .cloned()
        .unwrap_or_else(|| schema.clone())
}

fn native_contract_repair_signal_with_context(
    error_code: &str,
    failed_call: Option<&ModelToolCall>,
    expected_schema: Option<Value>,
    available_tool_names: &[String],
) -> String {
    let respond_contract_error = error_code.starts_with("native_respond_")
        || error_code == "native_plan_respond_tool_required";
    let loader_contract_error = error_code.starts_with("native_capability_group_load_");
    let unknown_tool_error = error_code == "native_plan_unknown_tool";
    let (default_tool_name, mut required_argument_fields, next_action) = if respond_contract_error {
        (
            NATIVE_RESPOND_TOOL,
            vec![
                "shape",
                "content",
                "items",
                "exact_item_count",
                "fields",
                "observed_fields",
                "exact_field_count",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>(),
            "retry_native_respond_call",
        )
    } else if loader_contract_error {
        (
            super::capability_discovery::RUNTIME_CAPABILITY_LOADER_TOOL,
            vec!["groups".to_string()],
            "retry_native_capability_group_load",
        )
    } else if unknown_tool_error {
        (
            NATIVE_CALL_CAPABILITY_TOOL,
            Vec::new(),
            "retry_with_available_native_tool",
        )
    } else {
        (
            NATIVE_CALL_CAPABILITY_TOOL,
            vec!["capability".to_string(), "args".to_string()],
            "retry_native_tool_call",
        )
    };
    let exact_failed_tool = failed_call.is_some() && expected_schema.is_some();
    let failed_tool_name = failed_call.map(|call| call.name.as_str());
    let tool_name = if unknown_tool_error {
        None
    } else {
        Some(failed_tool_name.unwrap_or(default_tool_name))
    };
    let mut argument_constraints = Map::new();
    if error_code == "native_respond_object_field_json_invalid" {
        argument_constraints.insert(
            "fields[].value_json".to_string(),
            json!({
                "encoding": "complete_serialized_json_value",
                "json_string_requires_surrounding_quotes": true,
                "malformed_json": "rejected"
            }),
        );
    }
    if error_code == "native_plan_args_not_object" {
        argument_constraints.insert(
            "args".to_string(),
            json!({
                "type": "object",
                "encoding": "json_object",
                "empty_object": {},
                "string_value": "rejected"
            }),
        );
    }
    if exact_failed_tool && !respond_contract_error && !loader_contract_error {
        if let Some(required) = expected_schema
            .as_ref()
            .and_then(|schema| schema.get("required"))
            .and_then(Value::as_array)
        {
            required_argument_fields = required
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
        }
    }
    if let Some(expected_schema) = expected_schema {
        argument_constraints.insert("exact_call_schema".to_string(), expected_schema);
    }
    json!({
        "protocol_observation": {
            "status": "error",
            "error_code": error_code,
            "tool_name": tool_name,
            "failed_tool_name": failed_tool_name,
            "exact_failed_tool": exact_failed_tool,
            "available_tool_names": available_tool_names,
            "required_argument_fields": required_argument_fields,
            "argument_constraints": Value::Object(argument_constraints),
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
    let repair_observation = serde_json::from_str::<Value>(repair_signal).ok();
    let error_code = repair_observation
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/protocol_observation/error_code")
                .and_then(Value::as_str)
        })
        .unwrap_or_default();
    let repair_tool_name = repair_observation.as_ref().and_then(|value| {
        value
            .pointer("/protocol_observation/tool_name")
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    let exact_failed_tool = repair_observation
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/protocol_observation/exact_failed_tool")
                .and_then(Value::as_bool)
        })
        .unwrap_or(false);
    if error_code == "native_plan_unknown_tool" {
        let available_tool_names = repair_observation
            .as_ref()
            .and_then(|value| {
                value
                    .pointer("/protocol_observation/available_tool_names")
                    .and_then(Value::as_array)
            })
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<BTreeSet<_>>();
        request
            .tools
            .retain(|tool| available_tool_names.contains(tool.name.as_str()));
    } else if let Some(repair_tool_name) = repair_tool_name {
        if exact_failed_tool {
            request.tools.retain(|tool| tool.name == repair_tool_name);
        } else if repair_tool_name == NATIVE_CALL_CAPABILITY_TOOL {
            request
                .tools
                .retain(|tool| tool.name != NATIVE_RESPOND_TOOL);
        } else {
            request.tools.retain(|tool| tool.name == repair_tool_name);
        }
    }
    request.tool_choice = ModelToolChoice::Required;
    request
        .messages
        .push(ModelMessage::text(ModelRole::User, repair_signal));
    request
}

#[cfg(test)]
fn actions_from_native_turn(
    turn: &ModelTurnResponse,
    callable_capability_names: &[String],
) -> Result<Vec<AgentAction>, String> {
    actions_from_native_turn_with_groups(turn, callable_capability_names, &BTreeMap::new(), None)
}

#[cfg(test)]
fn actions_from_native_turn_with_groups(
    turn: &ModelTurnResponse,
    callable_capability_names: &[String],
    native_capability_groups: &BTreeMap<String, BTreeSet<String>>,
    loop_state: Option<&LoopState>,
) -> Result<Vec<AgentAction>, String> {
    actions_from_native_turn_with_schemas(
        turn,
        callable_capability_names,
        native_capability_groups,
        &BTreeMap::new(),
        loop_state,
    )
}

fn actions_from_native_turn_with_schemas(
    turn: &ModelTurnResponse,
    callable_capability_names: &[String],
    native_capability_groups: &BTreeMap<String, BTreeSet<String>>,
    capability_argument_schemas: &BTreeMap<String, Value>,
    loop_state: Option<&LoopState>,
) -> Result<Vec<AgentAction>, String> {
    if !turn.tool_calls.is_empty() {
        let actions = turn
            .tool_calls
            .iter()
            .map(|call| {
                action_from_native_tool_call_with_schemas(
                    call,
                    native_capability_groups,
                    capability_argument_schemas,
                    loop_state,
                )
            })
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

fn action_from_native_tool_call_with_schemas(
    call: &ModelToolCall,
    native_capability_groups: &BTreeMap<String, BTreeSet<String>>,
    capability_argument_schemas: &BTreeMap<String, Value>,
    loop_state: Option<&LoopState>,
) -> Result<AgentAction, String> {
    match call.name.as_str() {
        NATIVE_CALL_CAPABILITY_TOOL => {
            action_from_native_capability_call(call, capability_argument_schemas)
        }
        NATIVE_RESPOND_TOOL => action_from_native_respond_call(call, loop_state),
        super::capability_discovery::RUNTIME_CAPABILITY_LOADER_TOOL => {
            action_from_native_capability_group_load(call)
        }
        tool_name if native_capability_groups.contains_key(tool_name) => {
            let allowed = &native_capability_groups[tool_name];
            let Some(capability) = allowed.iter().next().filter(|_| allowed.len() == 1) else {
                return Err("native_plan_group_action_invalid".to_string());
            };
            if !call.arguments.is_object() {
                return Err("native_plan_args_not_object".to_string());
            }
            if capability_argument_schemas
                .get(capability)
                .is_some_and(|schema| schema_has_missing_required_fields(schema, &call.arguments))
            {
                return Err("native_plan_required_args_missing".to_string());
            }
            Ok(AgentAction::CallCapability {
                capability: capability.clone(),
                args: call.arguments.clone(),
            })
        }
        _ => Err("native_plan_unknown_tool".to_string()),
    }
}

fn action_from_native_capability_group_load(call: &ModelToolCall) -> Result<AgentAction, String> {
    let arguments = call
        .arguments
        .as_object()
        .ok_or_else(|| "native_capability_group_load_arguments_not_object".to_string())?;
    let groups = arguments
        .get("groups")
        .and_then(Value::as_array)
        .filter(|groups| {
            !groups.is_empty() && groups.len() <= super::capability_discovery::MAX_GROUPS_PER_LOAD
        })
        .ok_or_else(|| "native_capability_group_load_groups_invalid".to_string())?;
    if groups.iter().any(|group| {
        group
            .as_str()
            .is_none_or(|token| !super::capability_discovery::is_capability_group_token(token))
    }) {
        return Err("native_capability_group_load_token_invalid".to_string());
    }
    Ok(AgentAction::CallTool {
        tool: super::capability_discovery::RUNTIME_CAPABILITY_LOADER_TOOL.to_string(),
        args: json!({"groups": groups}),
    })
}

fn action_from_native_capability_call(
    call: &ModelToolCall,
    capability_argument_schemas: &BTreeMap<String, Value>,
) -> Result<AgentAction, String> {
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
    let args = match arguments.get("args") {
        Some(args) if args.is_object() => args.clone(),
        Some(Value::String(args))
            if args.is_empty()
                && capability_argument_schemas
                    .get(capability)
                    .is_some_and(schema_accepts_empty_object) =>
        {
            json!({})
        }
        _ => return Err("native_plan_args_not_object".to_string()),
    };
    Ok(AgentAction::CallCapability {
        capability: capability.to_string(),
        args,
    })
}

fn schema_accepts_empty_object(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("object")
        && schema
            .get("required")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty)
        && schema.get("additionalProperties").and_then(Value::as_bool) == Some(false)
}

fn schema_has_missing_required_fields(schema: &Value, arguments: &Value) -> bool {
    let Some(arguments) = arguments.as_object() else {
        return true;
    };
    schema
        .get("required")
        .and_then(Value::as_array)
        .is_some_and(|required| {
            required.iter().filter_map(Value::as_str).any(|field| {
                arguments
                    .get(field)
                    .is_none_or(|value| !native_required_value_is_present(value))
            })
        })
}

fn native_required_value_is_present(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(value) => !value.trim().is_empty(),
        Value::Array(values) => values.iter().any(native_required_value_is_present),
        Value::Object(values) => !values.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
    }
}

fn action_from_native_respond_call(
    call: &ModelToolCall,
    loop_state: Option<&LoopState>,
) -> Result<AgentAction, String> {
    let arguments = call
        .arguments
        .as_object()
        .ok_or_else(|| "native_respond_arguments_not_object".to_string())?;
    let shape = arguments
        .get("shape")
        .and_then(Value::as_str)
        .ok_or_else(|| "native_respond_shape_missing".to_string())?;
    let content = match arguments.get("content") {
        Some(value) => value
            .as_str()
            .ok_or_else(|| "native_respond_content_missing".to_string())?,
        None => "",
    };
    let items = match arguments.get("items") {
        Some(value) => value
            .as_array()
            .map(Vec::as_slice)
            .ok_or_else(|| "native_respond_items_not_array".to_string())?,
        None => &[],
    };
    let exact_item_count = match arguments.get("exact_item_count") {
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value <= MAX_NATIVE_RESPONSE_ITEMS)
                .ok_or_else(|| "native_respond_exact_item_count_invalid".to_string())?,
        ),
        None => None,
    };
    let fields = match arguments.get("fields") {
        Some(value) => value
            .as_array()
            .map(Vec::as_slice)
            .ok_or_else(|| "native_respond_fields_not_array".to_string())?,
        None => &[],
    };
    let observed_fields = match arguments.get("observed_fields") {
        Some(value) => value
            .as_array()
            .map(Vec::as_slice)
            .ok_or_else(|| "native_respond_observed_fields_not_array".to_string())?,
        None => &[],
    };
    let exact_field_count = match arguments.get("exact_field_count") {
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value <= MAX_NATIVE_RESPONSE_FIELDS)
                .ok_or_else(|| "native_respond_exact_field_count_invalid".to_string())?,
        ),
        None => None,
    };

    match shape {
        "free_text" => {
            if content.trim().is_empty() {
                return Err("native_respond_free_text_empty".to_string());
            }
            if !items.is_empty()
                || exact_item_count.unwrap_or(0) != 0
                || !fields.is_empty()
                || !observed_fields.is_empty()
                || exact_field_count.unwrap_or(0) != 0
            {
                return Err("native_respond_free_text_contract_mismatch".to_string());
            }
            Ok(AgentAction::Respond {
                content: content.trim().to_string(),
            })
        }
        "list" => {
            let exact_item_count = exact_item_count
                .ok_or_else(|| "native_respond_exact_item_count_invalid".to_string())?;
            if !content.trim().is_empty() {
                return Err("native_respond_list_content_not_empty".to_string());
            }
            if !fields.is_empty()
                || !observed_fields.is_empty()
                || exact_field_count.unwrap_or(0) != 0
            {
                return Err("native_respond_list_fields_not_empty".to_string());
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
        "object" => {
            let exact_field_count = exact_field_count
                .ok_or_else(|| "native_respond_exact_field_count_invalid".to_string())?;
            if !items.is_empty() || exact_item_count.unwrap_or(0) != 0 {
                return Err("native_respond_object_non_field_payload".to_string());
            }
            if !observed_fields.is_empty() {
                return Err("native_respond_object_observed_fields_not_empty".to_string());
            }
            if exact_field_count == 0 || fields.len() != exact_field_count {
                return Err("native_respond_object_count_mismatch".to_string());
            }
            let mut object = Map::new();
            for field in fields {
                let field = field
                    .as_object()
                    .ok_or_else(|| "native_respond_object_field_invalid".to_string())?;
                let name = field
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|name| {
                        !name.is_empty()
                            && name.len() <= 128
                            && !name.contains('\r')
                            && !name.contains('\n')
                    })
                    .ok_or_else(|| "native_respond_object_field_name_invalid".to_string())?;
                let value_json = field
                    .get("value_json")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty() && value.len() <= 65536)
                    .ok_or_else(|| "native_respond_object_field_value_invalid".to_string())?;
                let value = serde_json::from_str(value_json)
                    .map_err(|_| "native_respond_object_field_json_invalid".to_string())?;
                if object.insert(name.to_string(), value).is_some() {
                    return Err("native_respond_object_field_duplicate".to_string());
                }
            }
            let object = Value::Object(object);
            if !content.trim().is_empty() {
                let redundant_content: Value = serde_json::from_str(content.trim())
                    .map_err(|_| "native_respond_object_non_field_payload".to_string())?;
                if redundant_content != object {
                    return Err("native_respond_object_non_field_payload".to_string());
                }
            }
            let object =
                project_exact_object_from_observations(&object, loop_state).unwrap_or(object);
            let content = serde_json::to_string(&object)
                .map_err(|_| "native_respond_object_serialize_failed".to_string())?;
            Ok(AgentAction::Respond { content })
        }
        "observed_object" => {
            if !content.trim().is_empty()
                || !items.is_empty()
                || exact_item_count.unwrap_or(0) != 0
                || !fields.is_empty()
            {
                return Err("native_respond_observed_object_non_reference_payload".to_string());
            }
            let exact_field_count = match exact_field_count {
                None | Some(0) => observed_fields.len(),
                Some(count) => count,
            };
            if exact_field_count == 0 || observed_fields.len() != exact_field_count {
                return Err("native_respond_observed_object_count_mismatch".to_string());
            }
            let loop_state =
                loop_state.ok_or_else(|| "native_respond_observation_state_missing".to_string())?;
            let mut object = Map::new();
            for field in observed_fields {
                let field = field
                    .as_object()
                    .ok_or_else(|| "native_respond_observed_field_invalid".to_string())?;
                let name = machine_response_field_name(field.get("name"))?;
                let capability = machine_observation_reference(
                    field.get("capability"),
                    "native_respond_observed_capability_invalid",
                )?;
                let path = machine_observation_reference(
                    field.get("path"),
                    "native_respond_observed_path_invalid",
                )?;
                let result = loop_state
                    .capability_results
                    .iter()
                    .rev()
                    .find(|result| {
                        result.status == claw_core::capability_result::CapabilityResultStatus::Ok
                            && result.capability == capability
                    })
                    .ok_or_else(|| {
                        "native_respond_observed_capability_result_missing".to_string()
                    })?;
                let value = crate::capability_result::selected_result_value(result, path)
                    .cloned()
                    .ok_or_else(|| "native_respond_observed_path_missing".to_string())?;
                if object.insert(name.to_string(), value).is_some() {
                    return Err("native_respond_object_field_duplicate".to_string());
                }
            }
            let content = serde_json::to_string(&Value::Object(object))
                .map_err(|_| "native_respond_object_serialize_failed".to_string())?;
            Ok(AgentAction::Respond { content })
        }
        _ => Err("native_respond_shape_unsupported".to_string()),
    }
}

fn project_exact_object_from_observations(
    object: &Value,
    loop_state: Option<&LoopState>,
) -> Option<Value> {
    let Some(object) = object.as_object().filter(|object| !object.is_empty()) else {
        return None;
    };
    let Some(loop_state) = loop_state else {
        return None;
    };
    object
        .iter()
        .map(|(name, expected)| {
            loop_state
                .capability_results
                .iter()
                .rev()
                .filter(|result| {
                    result.status == claw_core::capability_result::CapabilityResultStatus::Ok
                })
                .find_map(|result| observed_named_value(&result.data, name, expected, 0))
                .cloned()
                .map(|value| (name.clone(), value))
        })
        .collect::<Option<Map<String, Value>>>()
        .map(Value::Object)
}

fn observed_named_value<'a>(
    current: &'a Value,
    field_name: &str,
    expected: &Value,
    depth: usize,
) -> Option<&'a Value> {
    if depth > 12 {
        return None;
    }
    let Some(object) = current.as_object() else {
        return None;
    };
    object.iter().find_map(|(name, value)| {
        if name == field_name && value == expected {
            Some(value)
        } else {
            observed_named_value(value, field_name, expected, depth + 1)
        }
    })
}

fn machine_response_field_name(value: Option<&Value>) -> Result<&str, String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| {
            !name.is_empty() && name.len() <= 128 && !name.contains('\r') && !name.contains('\n')
        })
        .ok_or_else(|| "native_respond_object_field_name_invalid".to_string())
}

fn machine_observation_reference<'a>(
    value: Option<&'a Value>,
    error_code: &str,
) -> Result<&'a str, String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| {
            value.len() <= MAX_NATIVE_RESPONSE_SOURCE_PATH
                && claw_core::capability_result::is_machine_ref(value)
        })
        .ok_or_else(|| error_code.to_string())
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
