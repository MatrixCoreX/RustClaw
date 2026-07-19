use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, info};

mod arg_resolver;
mod async_start_checkpoint;
mod attempt_ledger;
#[allow(dead_code)]
mod context_compaction;
mod dispatch_support;
mod execution_loop;
mod explicit_machine_command;
mod filesystem_lifecycle_contract;
pub(crate) mod loop_control;
mod loop_state_contract_evidence;
mod loop_state_seed;
mod media_artifact_plan;
mod mutation_ledger;
pub(crate) mod observed_output;
mod planner_skill_context;
mod planning;
pub(crate) use explicit_machine_command::explicit_machine_syntax_command_segment;
mod planning_action_normalization;
mod planning_actions;
#[path = "agent_engine/planning_output_contract.rs"]
mod planning_output_contract;
mod planning_parse;
mod planning_prompt;
mod planning_repair;
mod prepare_round;
mod skill_execution;
mod skill_quick_index;
mod subagent_runtime;
mod support;
mod user_output_path;

use self::arg_resolver::{
    normalize_skill_arg_aliases, resolve_arg_string, resolve_arg_value,
    rewrite_args_with_auto_locator_path, rewrite_run_cmd_with_written_aliases,
    rewrite_tool_path_with_written_aliases,
};
use self::dispatch_support::{classify_skill_failure_recovery, dispatch_round_action};

pub(crate) fn normalize_resolved_planner_action_for_verifier(
    state: &AppState,
    action: AgentAction,
) -> AgentAction {
    self::planning_action_normalization::normalize_resolved_executable_action(state, action)
}

pub(crate) fn local_code_strict_json_projection_from_machine_evidence(
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    self::dispatch_support::local_code_strict_json_projection_from_machine_evidence(
        user_text,
        loop_state,
        agent_run_context,
    )
}

pub(crate) fn local_code_strict_json_answer_satisfies_request(
    user_text: &str,
    answer: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    self::dispatch_support::local_code_strict_json_answer_satisfies_request(
        user_text,
        answer,
        agent_run_context,
    )
}

pub(crate) fn local_code_strict_json_projection_should_defer_finalizer_fallback(
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    self::dispatch_support::local_code_strict_json_projection_should_defer_observed_synthesis(
        user_text,
        loop_state,
        agent_run_context,
    ) || self::dispatch_support::local_code_strict_json_projection_should_defer_until_validation(
        user_text,
        loop_state,
        agent_run_context,
    )
}
pub(crate) use self::context_compaction::run_model_assisted_context_compaction;
use self::execution_loop::execute_actions_once;
pub(crate) use self::filesystem_lifecycle_contract::{
    enrich_scratch_filesystem_cleanup_runtime_args,
    scratch_filesystem_lifecycle_observed_steps_match,
};
use self::loop_control::{
    run_agent_with_loop_direct_plan, run_agent_with_loop_seeded,
    run_agent_with_loop_seeded_direct_plan, run_agent_with_loop_with_initial_observations,
};
use self::loop_state_contract_evidence::{
    active_plan_file_targets_for_loop_seed, boundary_observation_needs_clarify_for_loop_seed,
    contract_repair_candidate_evidence_for_loop_seed,
    default_main_config_contract_evidence_for_loop_seed, first_string_field,
    pending_user_boundary_present_for_loop_seed, pre_loop_clarify_candidates_for_loop_seed,
    registry_capability_contract_evidence_for_loop_seed, registry_capability_contract_refs,
};
use self::planner_skill_context::build_planner_skill_context;
use self::prepare_round::{prepare_round_actions, push_round_trace};
use self::skill_execution::execute_prepared_skill_action;
use self::support::{
    action_fingerprint_for_policy, append_progress_hint, build_safe_skill_args_summary,
    encode_progress_i18n, load_agent_loop_guard_policy, maybe_publish_execution_recipe_phase_hint,
    registry_idempotency_guard_attribution, AgentLoopGuardPolicy, PROGRESS_ARGS_SUMMARY_MAX_LEN,
};
pub(crate) use self::support::{
    append_delivery_message, publish_agent_loop_user_input_checkpoint_progress,
};
pub(crate) use self::user_output_path::CLAWD_USER_NAMED_OUTPUT_PATH_ARG;

use crate::{repo, AgentAction, AppState, AskReply, ClaimedTask, PlanKind, PlanResult};

pub(crate) fn answer_verifier_enforce_required_enabled(state: &AppState) -> bool {
    load_agent_loop_guard_policy(state).answer_verifier_required_evidence_enabled()
}

const CLAWD_CONTINUE_ON_ERROR_ARG: &str = "_clawd_continue_on_error";
const CLAWD_LITERAL_COMMAND_ARG: &str = "_clawd_literal_command";
const CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG: &str = "_clawd_literal_failure_repairable";
const CLAWD_MISSING_TARGET_REPAIRABLE_ARG: &str = "_clawd_missing_target_repairable";
pub(crate) const CLAWD_RUNTIME_ASYNC_JOB_START_ARG: &str = "_clawd_runtime_async_job_start";
const SINGLE_PLAN_EXECUTION_PROMPT_LOGICAL_PATH: &str = "prompts/single_plan_execution_prompt.md";
const LOOP_INCREMENTAL_PLAN_PROMPT_LOGICAL_PATH: &str = "prompts/loop_incremental_plan_prompt.md";
const PLAN_REPAIR_PROMPT_LOGICAL_PATH: &str = "prompts/plan_repair_prompt.md";
const PLANNER_ABORT_COMPACT_RETRY_PROMPT_LOGICAL_PATH: &str =
    "prompts/planner_abort_compact_retry_prompt.md";
pub(crate) const TASK_CANCELED_ERR: &str = "__TASK_CANCELED_BY_USER__";

fn ensure_task_running(state: &AppState, task: &ClaimedTask) -> Result<(), String> {
    match repo::is_task_still_running(state, &task.task_id) {
        Ok(true) => Ok(()),
        Ok(false) => Err(TASK_CANCELED_ERR.to_string()),
        Err(err) => Err(format!("check task running state failed: {err}")),
    }
}

#[derive(Debug, Deserialize)]
struct SinglePlanEnvelope {
    #[serde(default)]
    steps: Vec<Value>,
}

fn build_single_plan_prompt(
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
            ("__RUNTIME_OS__", runtime_os),
            ("__RUNTIME_SHELL__", runtime_shell),
            ("__WORKSPACE_ROOT__", workspace_root),
        ],
    )
}

fn build_turn_analysis_prompt_block(
    turn_analysis: Option<&crate::turn_context::TurnAnalysis>,
    boundary_envelope: Option<&crate::turn_boundary_envelope::TurnBoundaryEnvelope>,
) -> String {
    let mut lines = Vec::new();
    if let Some(envelope) = boundary_envelope {
        lines.push(envelope.compact_prompt_line());
    }
    if let Some(analysis) = turn_analysis {
        let turn_type = analysis
            .turn_type
            .map(crate::turn_context::TurnType::as_str)
            .unwrap_or("none");
        let target_task_policy = analysis
            .target_task_policy
            .map(crate::turn_context::TargetTaskPolicy::as_str)
            .unwrap_or("none");
        let state_patch = analysis
            .state_patch
            .as_ref()
            .and_then(|value| serde_json::to_string(value).ok())
            .unwrap_or_else(|| "null".to_string());
        lines.push(format!("- turn_type={turn_type}"));
        lines.push(format!("- target_task_policy={target_task_policy}"));
        lines.push(format!(
            "- should_interrupt_active_run={}",
            analysis.should_interrupt_active_run
        ));
        lines.push(format!(
            "- attachment_processing_required={}",
            analysis.attachment_processing_required
        ));
        lines.push(format!("- state_patch={state_patch}"));
    }
    if lines.is_empty() {
        "<none>".to_string()
    } else {
        lines.join("\n")
    }
}

/// Progress: short hints only (e.g. "Step 1/3", "Skill X completed"). For "in progress" UI. Not final content.
/// Delivery: final user-facing content only. Only respond and fallback finalizer append here. Channel consumes this.
/// Trace: step output / subtask_results / history_compact for logs and resume; not sent as final delivery.
// Phase 3.3 Stage 2.3：LoopState 字段升 pub(crate)，因 finalize/loop_reply.rs 物理搬到了
// 不同模块（`crate::finalize`），无法再通过 `pub(super)` 隐式继承；继续保持仅 `pub(crate)`，
// 不暴露给 crate 外部。改字段时请关注 crate::finalize::* 与 crate::agent_engine::* 内的写入点。
#[derive(Debug, Default, Clone)]
pub(crate) struct LoopState {
    pub(crate) round_no: usize,
    pub(crate) max_rounds: usize,
    pub(crate) tool_calls_total: usize,
    pub(crate) total_steps_executed: usize,
    /// Progress hints only; published to task progress for "processing..." display. Must not contain full raw output.
    pub(crate) progress_messages: Vec<String>,
    /// Final delivery to user. Only respond and fallback finalizer write here. Sole source for AskReply.messages.
    pub(crate) delivery_messages: Vec<String>,
    pub(crate) subtask_results: Vec<String>,
    pub(crate) history_compact: Vec<String>,
    pub(crate) attempt_ledger_entries: Vec<self::attempt_ledger::AttemptLedgerEntry>,
    pub(crate) last_actions_fingerprint: Option<String>,
    pub(crate) repeat_action_counts: HashMap<String, usize>,
    pub(crate) successful_action_fingerprints: HashMap<String, usize>,
    pub(crate) consecutive_no_progress: usize,
    pub(crate) recoverable_failure_extra_rounds_used: usize,
    pub(crate) last_output: Option<String>,
    pub(crate) output_vars: HashMap<String, String>,
    pub(crate) has_tool_or_skill_output: bool,
    pub(crate) has_recoverable_failure_context: bool,
    pub(crate) last_stop_signal: Option<String>,
    pub(crate) written_file_aliases: HashMap<String, String>,
    pub(crate) last_written_file_path: Option<String>,
    /// Last user-visible respond text (final or publishable). Used when delivery_messages was not filled so we do not fall back to subtask summary.
    pub(crate) last_user_visible_respond: Option<String>,
    /// Last publishable runtime synthesis output. Prefer this over generic finalization when no explicit respond was emitted.
    pub(crate) last_publishable_synthesis_output: Option<String>,
    /// A tool/skill returned a structured user-input request. Finalize as clarify and do not
    /// treat the answer as incomplete execution output.
    pub(crate) pending_user_input_required: bool,
    pub(crate) executed_step_results: Vec<crate::executor::StepExecutionResult>,
    pub(crate) round_traces: Vec<crate::task_journal::TaskJournalRoundTrace>,
    pub(crate) rollout_attribution: Vec<crate::task_journal::TaskJournalRolloutAttribution>,
    pub(crate) execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState,
    pub(crate) latest_validation_result: Option<Value>,
    pub(crate) task_lifecycle: Option<Value>,
    pub(crate) task_checkpoint: Option<Value>,
    pub(crate) task_observations: Vec<Value>,
    pub(crate) last_recipe_progress_phase: Option<crate::execution_recipe::ExecutionRecipePhase>,
    pub(crate) last_recipe_progress_scope:
        Option<crate::execution_recipe::ExecutionRecipeTargetScope>,
    pub(crate) recipe_scope_ready_hint_sent: bool,
    /// §7.1 output_contract 贯穿全链：IntentOutputContract 里的 output_contract 与 route marker
    /// 合并成 effective machine contract 后挂到 LoopState 上。下游 synthesis/finalize
    /// 和无 IntentOutputContract 入参的 preflight 必须看见这些字段，否则容易把结构化
    /// existence_with_path / scalar / file_token 契约答成自由段落。
    /// 默认 None：测试与不走 IntentOutputContract 的 ad-hoc 路径保持向后兼容。
    pub(crate) output_contract: Option<crate::IntentOutputContract>,
    /// True only while executing the current round's verifier-approved actions.
    /// Preflight uses this as a narrow bridge from legacy route guards to the
    /// agent-loop authority model; it must be cleared immediately after action
    /// dispatch returns.
    pub(crate) verified_action_window_active: bool,
    /// Machine boundary fact from AGENT_LOOP_BOUNDARY_OBSERVATIONS. This lets
    /// the loop accept a terminal clarification even when legacy IntentOutputContract
    /// fields were normalized back to an execution gate for planner entry.
    pub(crate) boundary_observation_needs_clarify: bool,
    /// Machine boundary fact for an active waiting user-input turn. Respond-only
    /// planner outputs may be terminal clarify/update turns instead of broken
    /// execution plans.
    pub(crate) pending_user_boundary_present: bool,
}

impl LoopState {
    pub(crate) fn new(max_rounds: usize) -> Self {
        Self {
            max_rounds,
            ..Self::default()
        }
    }
}

#[cfg(test)]
use self::loop_state_seed::seed_loop_state_from_agent_context;
#[cfg(test)]
pub(crate) use self::loop_state_seed::seed_loop_state_from_task_checkpoint;
pub(crate) use self::loop_state_seed::{
    seed_loop_state_for_agent_run, LoopStateCheckpointSeedReport,
};

#[derive(Debug, Clone)]
struct RoundOutcome {
    executed_actions: usize,
    had_error: bool,
    stop_signal: Option<String>,
    next_goal_hint: Option<String>,
    no_progress: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AgentRunContext {
    pub(crate) output_contract: Option<crate::IntentOutputContract>,
    pub(crate) execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    pub(crate) execution_recipe_plan_hint: Option<crate::execution_recipe::ExecutionRecipePlanHint>,
    pub(crate) turn_analysis: Option<crate::turn_context::TurnAnalysis>,
    pub(crate) boundary_envelope: Option<crate::turn_boundary_envelope::TurnBoundaryEnvelope>,
    pub(crate) context_bundle_summary: Option<String>,
    pub(crate) session_alias_bindings: Vec<crate::conversation_state::SessionAliasBinding>,
    pub(crate) auto_locator_path: Option<String>,
    pub(crate) original_user_request: Option<String>,
    pub(crate) user_request: Option<String>,
    /// Cross-turn recent execution context for planner access to prior observed outputs,
    /// aliases, and targets when the current turn references previous turns.
    pub(crate) cross_turn_recent_execution_context: Option<String>,
}

impl AgentRunContext {
    pub(crate) fn output_contract(&self) -> Option<&crate::IntentOutputContract> {
        self.output_contract.as_ref()
    }
}

struct RespondActionOutcome {
    ended_with_user_visible_output: bool,
    stop_signal: Option<String>,
    should_stop: bool,
}

struct SkillActionOutcome {
    ended_with_user_visible_output: bool,
    stop_signal: Option<String>,
    continue_in_round: bool,
}

enum ActionLoopDecision {
    NextAction,
    ContinueRound,
    StopRound(String),
}

fn build_loop_history_compact(loop_state: &LoopState) -> String {
    if loop_state.history_compact.is_empty() {
        "(empty)".to_string()
    } else {
        loop_state.history_compact.join("\n")
    }
}

/// Trace only: updates loop_state for planner/resume. Does not write to progress or delivery.
fn register_step_output(
    loop_state: &mut LoopState,
    global_step: usize,
    round_step: usize,
    key_prefix: &str,
    output: &str,
) {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return;
    }
    debug!(
        "trace_only step_output key={} global_step={} round_step={}",
        key_prefix, global_step, round_step
    );
    let value = trimmed.to_string();
    let output_kind = crate::finalize::classify_observed_output_kind(trimmed);
    let content_status = crate::finalize::classify_observed_content_status(trimmed);
    loop_state.last_output = Some(value.clone());
    loop_state
        .output_vars
        .insert("last_output".to_string(), value.clone());
    loop_state.output_vars.insert(
        "last_output_kind".to_string(),
        output_kind.as_str().to_string(),
    );
    loop_state.output_vars.insert(
        "last_content_status".to_string(),
        content_status.as_str().to_string(),
    );
    loop_state
        .output_vars
        .insert(format!("s{global_step}.output"), value.clone());
    loop_state
        .output_vars
        .insert(format!("s{round_step}.output"), value.clone());
    loop_state.output_vars.insert(
        format!("s{global_step}.output_kind"),
        output_kind.as_str().to_string(),
    );
    loop_state.output_vars.insert(
        format!("s{round_step}.output_kind"),
        output_kind.as_str().to_string(),
    );
    loop_state.output_vars.insert(
        format!("s{global_step}.content_status"),
        content_status.as_str().to_string(),
    );
    loop_state.output_vars.insert(
        format!("s{round_step}.content_status"),
        content_status.as_str().to_string(),
    );
    loop_state
        .output_vars
        .insert(format!("{key_prefix}.last_output"), value);
    loop_state.output_vars.insert(
        format!("{key_prefix}.output_kind"),
        output_kind.as_str().to_string(),
    );
    loop_state.output_vars.insert(
        format!("{key_prefix}.content_status"),
        content_status.as_str().to_string(),
    );
    register_structured_indexed_output_vars(
        loop_state,
        global_step,
        round_step,
        key_prefix,
        trimmed,
    );
}

fn register_structured_indexed_output_vars(
    loop_state: &mut LoopState,
    global_step: usize,
    round_step: usize,
    key_prefix: &str,
    output: &str,
) {
    let Ok(value) = serde_json::from_str::<Value>(output) else {
        return;
    };
    let Some(names) = value.get("names").and_then(Value::as_array) else {
        return;
    };
    for (idx, item) in names.iter().enumerate() {
        let Some(name) = item.as_str().map(str::trim).filter(|name| !name.is_empty()) else {
            continue;
        };
        let name = name.to_string();
        for base in [
            "last_output".to_string(),
            format!("s{global_step}"),
            format!("s{round_step}"),
            key_prefix.to_string(),
        ] {
            loop_state
                .output_vars
                .insert(format!("{base}.{idx}"), name.clone());
            loop_state
                .output_vars
                .insert(format!("{base}[{idx}]"), name.clone());
            loop_state
                .output_vars
                .insert(format!("{base}.names.{idx}"), name.clone());
            loop_state
                .output_vars
                .insert(format!("{base}.names[{idx}]"), name.clone());
        }
    }
}

fn remember_written_file_alias(
    loop_state: &mut LoopState,
    original_path: &str,
    effective_path: &str,
) {
    let original = original_path.trim();
    let effective = effective_path.trim();
    if original.is_empty() || effective.is_empty() || original == effective {
        return;
    }
    loop_state
        .written_file_aliases
        .insert(original.to_string(), effective.to_string());
    if let Some(name) = Path::new(original).file_name().and_then(|v| v.to_str()) {
        loop_state
            .written_file_aliases
            .entry(name.to_string())
            .or_insert_with(|| effective.to_string());
    }
    loop_state.last_written_file_path = Some(effective.to_string());
}

fn register_file_path_output(
    loop_state: &mut LoopState,
    global_step: usize,
    round_step: usize,
    key_prefix: &str,
    path: &str,
) {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return;
    }
    let value = trimmed.to_string();
    loop_state
        .output_vars
        .insert("last_file_path".to_string(), value.clone());
    loop_state
        .output_vars
        .insert("last_saved_file_path".to_string(), value.clone());
    loop_state
        .output_vars
        .insert("last_written_file_path".to_string(), value.clone());
    loop_state
        .output_vars
        .insert(format!("s{global_step}.path"), value.clone());
    loop_state
        .output_vars
        .insert(format!("s{round_step}.path"), value.clone());
    loop_state
        .output_vars
        .insert(format!("{key_prefix}.path"), value);
    loop_state.output_vars.insert(
        "last_file_kind".to_string(),
        crate::finalize::infer_file_target_kind(trimmed)
            .as_str()
            .to_string(),
    );
}

fn register_failed_step_output(
    loop_state: &mut LoopState,
    global_step: usize,
    round_step: usize,
    key_prefix: &str,
    failed_action: &str,
    err: &str,
) {
    loop_state.has_recoverable_failure_context = true;
    let trimmed = err.trim();
    let failed_status = crate::finalize::ObservedContentStatus::Failed
        .as_str()
        .to_string();
    if !trimmed.is_empty() {
        loop_state.last_output = Some(trimmed.to_string());
        loop_state
            .output_vars
            .insert("last_output".to_string(), trimmed.to_string());
        loop_state.output_vars.insert(
            "last_output_kind".to_string(),
            crate::finalize::ObservedOutputKind::Error
                .as_str()
                .to_string(),
        );
        loop_state.output_vars.insert(
            "last_content_status".to_string(),
            crate::finalize::ObservedContentStatus::Failed
                .as_str()
                .to_string(),
        );
        loop_state
            .output_vars
            .insert("last_error".to_string(), trimmed.to_string());
        loop_state
            .output_vars
            .insert("failed_step.error".to_string(), trimmed.to_string());
        loop_state.output_vars.insert(
            format!("s{global_step}.content_status"),
            failed_status.clone(),
        );
        loop_state.output_vars.insert(
            format!("s{round_step}.content_status"),
            failed_status.clone(),
        );
        loop_state
            .output_vars
            .insert(format!("s{global_step}.error"), trimmed.to_string());
        loop_state
            .output_vars
            .insert(format!("s{round_step}.error"), trimmed.to_string());
        loop_state
            .output_vars
            .insert(format!("{key_prefix}.error"), trimmed.to_string());
        loop_state
            .output_vars
            .insert(format!("{key_prefix}.content_status"), failed_status);
    }
    register_structured_failed_step_fields(loop_state, key_prefix, trimmed);
    loop_state.output_vars.insert(
        "failed_step.action".to_string(),
        failed_action.trim().to_string(),
    );
    loop_state
        .output_vars
        .insert("failed_step.index".to_string(), round_step.to_string());
}

fn register_structured_failed_step_fields(loop_state: &mut LoopState, key_prefix: &str, err: &str) {
    let Some(structured) = crate::skills::parse_structured_skill_error(err) else {
        return;
    };
    if !structured.skill.trim().is_empty() {
        loop_state
            .output_vars
            .insert("failed_step.skill".to_string(), structured.skill.clone());
    }
    loop_state.output_vars.insert(
        "failed_step.error_kind".to_string(),
        structured.error_kind.clone(),
    );
    loop_state.output_vars.insert(
        format!("{key_prefix}.error_kind"),
        structured.error_kind.clone(),
    );
    if let Some(extra) = structured.extra.as_ref().and_then(Value::as_object) {
        for field in ["error_code", "status_code"] {
            if let Some(value) = extra
                .get(field)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
            {
                loop_state
                    .output_vars
                    .insert(format!("failed_step.{field}"), value.to_string());
            }
        }
        if let Some(retryable) = extra.get("retryable").and_then(Value::as_bool) {
            loop_state
                .output_vars
                .insert("failed_step.retryable".to_string(), retryable.to_string());
        }
    }
}

pub(crate) fn register_failed_step_structured_error_fields(
    loop_state: &mut LoopState,
    key_prefix: &str,
    raw_err: &str,
) {
    register_structured_failed_step_fields(loop_state, key_prefix, raw_err);
}

type WriteFileEffectivePath = (String, String, String);

fn plan_step_label(action: &AgentAction) -> String {
    match action {
        // LEGACY: CallTool shown as skill for unified capability view.
        AgentAction::CallTool { tool, .. } => format!("skill:{tool}"),
        AgentAction::CallSkill { skill, .. } => format!("skill:{skill}"),
        AgentAction::CallCapability { capability, .. } => format!("capability:{capability}"),
        AgentAction::SynthesizeAnswer { evidence_refs } => {
            if evidence_refs.is_empty() {
                "synthesize_answer".to_string()
            } else {
                format!("synthesize_answer:{}", evidence_refs.join(","))
            }
        }
        AgentAction::Respond { content } => {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                "respond".to_string()
            } else {
                format!("respond:{}", crate::truncate_for_agent_trace(trimmed))
            }
        }
        AgentAction::Think { .. } => "think".to_string(),
    }
}

fn user_safe_step_error(err: &str, _prefer_english: bool) -> String {
    let trimmed = err.trim();
    if trimmed.is_empty() {
        return json!({
            "message_key": "clawd.msg.execution.step_error_missing",
            "reason_code": "execution_step_error_missing",
        })
        .to_string();
    }
    if let Some(structured) = crate::skills::parse_structured_skill_error(trimmed) {
        let skill = if structured.skill.trim().is_empty() {
            ""
        } else {
            structured.skill.as_str()
        };
        if let Some(observation) = crate::skills::skill_error_machine_observation(skill, trimmed) {
            return crate::truncate_for_agent_trace(&observation);
        }
    }
    crate::truncate_for_agent_trace(trimmed)
}

fn resume_step_failed_machine_payload(
    failed_index: usize,
    failed_action: &str,
    err: &str,
    has_remaining_actions: bool,
) -> String {
    json!({
        "message_key": "clawd.msg.execution.step_failed",
        "reason_code": "execution_step_failed",
        "failed_index": failed_index,
        "failed_action": failed_action,
        "error": err,
        "remaining_actions_paused": has_remaining_actions,
    })
    .to_string()
}

fn resume_context_structured_skill_error(raw_err: Option<&str>) -> Option<Value> {
    let raw = raw_err.map(str::trim).filter(|err| !err.is_empty())?;
    if let Some(path) = crate::skills::read_file_not_found_path(raw) {
        return Some(json!({
            "skill": "read_file",
            "error_kind": "not_found",
            "platform": Value::Null,
            "manager_type": Value::Null,
            "service_name": Value::Null,
            "extra": {
                "path": path,
                "error_code": "not_found",
                "reason_code": "not_found"
            }
        }));
    }
    let structured = crate::skills::parse_structured_skill_error(raw)?;
    Some(json!({
        "skill": structured.skill,
        "error_kind": structured.error_kind,
        "platform": structured.platform,
        "manager_type": structured.manager_type,
        "service_name": structured.service_name,
        "extra": structured.extra,
    }))
}

async fn build_resume_context_error(
    state: &AppState,
    task: &ClaimedTask,
    actions: &[AgentAction],
    plan_steps: &[String],
    user_request: &str,
    goal: &str,
    subtask_results: &[String],
    delivery_messages: &[String],
    failed_index: usize,
    failed_action: &str,
    err: &str,
    raw_err: Option<&str>,
) -> String {
    let completed_messages_for_ctx: Vec<String> = if delivery_messages.is_empty() {
        subtask_results.to_vec()
    } else {
        delivery_messages.to_vec()
    };
    let completed_steps = if failed_index <= 1 {
        Vec::new()
    } else {
        subtask_results
            .iter()
            .take(failed_index.saturating_sub(1))
            .cloned()
            .collect::<Vec<_>>()
    };
    let remaining_steps = if plan_steps.len() > failed_index {
        plan_steps
            .iter()
            .skip(failed_index)
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let remaining_actions = if actions.len() > failed_index {
        actions
            .iter()
            .skip(failed_index)
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let mut resume_context = json!({
        "resume_context_id": format!("ctx-{}", uuid::Uuid::new_v4()),
        "user_request": user_request,
        "goal": goal,
        "plan_steps": plan_steps,
        "completed_steps": completed_steps,
        "completed_messages": completed_messages_for_ctx,
        "failed_step": {
            "index": failed_index,
            "action": failed_action,
            "error": crate::truncate_for_agent_trace(err),
        },
        "remaining_steps": remaining_steps,
        "remaining_actions": remaining_actions,
        "hint": "LLM should infer continuation from resume context and user follow-up."
    });
    if let Some(structured_error) = resume_context_structured_skill_error(raw_err) {
        if let Some(failed_step) = resume_context
            .get_mut("failed_step")
            .and_then(|value| value.as_object_mut())
        {
            failed_step.insert("structured_error".to_string(), structured_error);
        }
    }
    let has_remaining_actions = resume_context
        .get("remaining_actions")
        .and_then(|v| v.as_array())
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_request);
    let prefer_english = language_hint.to_ascii_lowercase().starts_with("en");
    let safe_err = user_safe_step_error(err, prefer_english);
    let fallback_user_error = resume_step_failed_machine_payload(
        failed_index,
        failed_action,
        &safe_err,
        has_remaining_actions,
    );
    let mut observed_facts = vec![
        format!("failed_step_index: {failed_index}"),
        format!("failed_action: {failed_action}"),
        format!("error_summary: {safe_err}"),
        format!("remaining_steps_count: {}", remaining_steps.len()),
    ];
    if !completed_steps.is_empty() {
        observed_facts.push(format!("completed_steps_count: {}", completed_steps.len()));
    }
    let mut policy_boundary = vec![
        "expose_internal_details=false".to_string(),
        "failed_step_success_claim_allowed=false".to_string(),
        "response_focus=failed_step_and_recovery_path".to_string(),
    ];
    if has_remaining_actions {
        policy_boundary.push("remaining_steps_state=paused".to_string());
        policy_boundary.push("resume_available=true".to_string());
    } else {
        policy_boundary.push("remaining_work_claim_allowed=false".to_string());
        policy_boundary.push("continuation_option_claim_allowed=false".to_string());
    }
    let contract = crate::fallback::UserResponseContract::tool_failure(
        if has_remaining_actions {
            "resume_step_failed_with_remaining"
        } else {
            "resume_step_failed_no_remaining"
        },
        user_request,
        goal,
        observed_facts,
        policy_boundary,
        if has_remaining_actions {
            "brief_failure_with_continue_option"
        } else {
            "brief_failure"
        },
        &language_hint,
    );
    let user_error = crate::fallback::compose_user_response_from_contract_with_default(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::ExecutionFailedPartial,
        &fallback_user_error,
    )
    .await;
    let payload = json!({
        "user_error": user_error,
        "resume_context": resume_context
    });
    format!("{}{}", crate::RESUME_CONTEXT_ERROR_PREFIX, payload)
}

fn confirmation_remaining_step_labels(steps: &[crate::PlanStep]) -> Vec<String> {
    steps
        .iter()
        .map(|step| match step.action_type.as_str() {
            "respond" => "respond".to_string(),
            "think" => "think".to_string(),
            "call_capability" => format!("capability({})", step.skill),
            "call_tool" => format!("tool({})", step.skill),
            _ => format!("skill({})", step.skill),
        })
        .collect()
}

pub(crate) async fn build_confirmation_required_resume_context(
    state: &AppState,
    task: &ClaimedTask,
    steps: &[crate::PlanStep],
    user_request: &str,
    goal: &str,
    subtask_results: &[String],
    delivery_messages: &[String],
    detail: &str,
    confirmation_step_ids: &[String],
) -> (String, serde_json::Value) {
    let completed_messages_for_ctx: Vec<String> = if delivery_messages.is_empty() {
        subtask_results.to_vec()
    } else {
        delivery_messages.to_vec()
    };
    let remaining_steps = confirmation_remaining_step_labels(steps);
    let remaining_steps_count = remaining_steps.len();
    let remaining_actions = steps
        .iter()
        .filter_map(crate::PlanStep::to_agent_action)
        .collect::<Vec<_>>();
    let permission_evaluation = crate::agent_hooks::lifecycle_stage_outcome_for_state(
        state,
        &task.task_id,
        crate::agent_hooks::HookStage::PermissionRequest,
        "agent_loop.verifier_permission_request",
        json!({
            "request_source": "plan_verifier",
            "remaining_step_count": remaining_steps_count,
            "completed_step_count": subtask_results.len(),
            "delivery_message_count": delivery_messages.len(),
            "confirmation_step_count": confirmation_step_ids.len(),
        }),
    )
    .await;
    let permission_decision = permission_evaluation
        .outcome
        .decision_kind()
        .unwrap_or(crate::policy_decision::PolicyDecision::Deny);
    let confirmation_can_proceed = matches!(
        permission_decision,
        crate::policy_decision::PolicyDecision::Allow
            | crate::policy_decision::PolicyDecision::RequireConfirmation
    );
    let required_decision = if confirmation_can_proceed {
        crate::policy_decision::PolicyDecision::RequireConfirmation
    } else {
        permission_decision
    };
    let message_key = match required_decision {
        crate::policy_decision::PolicyDecision::Deny => {
            "clawd.agent_hook.permission_request_blocked"
        }
        crate::policy_decision::PolicyDecision::BackgroundWait => {
            "clawd.agent_hook.permission_request_background_wait"
        }
        _ => "clawd.approval.explicit_confirmation_required",
    };
    let mut resume_context = json!({
        "resume_context_id": format!("ctx-{}", uuid::Uuid::new_v4()),
        "user_request": user_request,
        "goal": goal,
        "plan_steps": remaining_steps,
        "completed_steps": subtask_results,
        "completed_messages": completed_messages_for_ctx,
        "failed_step": {
            "index": 0,
            "action": "confirmation_required",
            "error": crate::truncate_for_agent_trace(detail),
        },
        "remaining_steps": remaining_steps,
        "remaining_actions": remaining_actions,
        "message_key": message_key,
        "required_decision": required_decision.as_token(),
        "permission_hook_decision": permission_decision.as_token(),
        "agent_hook_events": permission_evaluation.machine_observations("plan_verifier"),
    });
    if confirmation_can_proceed {
        if let Some(binding) = crate::approval_grant::binding_for_confirmation_steps(
            state,
            steps,
            confirmation_step_ids,
        ) {
            resume_context["approval_request"] =
                crate::approval_grant::pending_approval_request_json(
                    &task.task_id,
                    &binding,
                    crate::now_ts_u64() as i64,
                );
        }
    }
    let language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_request);
    let response_contract_kind = match required_decision {
        crate::policy_decision::PolicyDecision::Deny => "resume_permission_request_denied",
        crate::policy_decision::PolicyDecision::BackgroundWait => {
            "resume_permission_request_background_wait"
        }
        _ => "resume_confirmation_required",
    };
    let contract = crate::fallback::UserResponseContract::verifier_gate(
        response_contract_kind,
        user_request,
        goal,
        vec![json!({"required_decision": required_decision.as_token()}).to_string()],
        vec![
            json!({"verification_issue_kind": "confirmation_required"}).to_string(),
            json!({"remaining_steps_count": remaining_steps_count}).to_string(),
            json!({"needs_confirmation": confirmation_can_proceed}).to_string(),
            json!({"permission_hook_decision": permission_decision.as_token()}).to_string(),
            json!({"confirmation_detail_present": !detail.trim().is_empty()}).to_string(),
        ],
        "brief_failure_with_continue_option",
        &language_hint,
    );
    let user_error = crate::fallback::compose_user_response_from_contract(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::VerifyRejected,
    )
    .await;
    (user_error, resume_context)
}

pub(crate) async fn run_agent_with_tools(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_request: &str,
    agent_run_context: Option<AgentRunContext>,
    initial_task_observations: &[Value],
) -> Result<AskReply, String> {
    info!(
        "run_agent_with_tools: task_id={} observation_count={} goal={}",
        task.task_id,
        initial_task_observations.len(),
        crate::truncate_for_log(goal)
    );
    let user_text = user_request.trim();
    if user_text.is_empty() {
        return Ok(AskReply::non_llm(String::new()));
    }
    run_agent_with_loop_with_initial_observations(
        state,
        task,
        goal,
        user_text,
        agent_run_context.as_ref(),
        initial_task_observations,
    )
    .await
}

pub(crate) async fn run_agent_with_tools_seeded(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_request: &str,
    agent_run_context: Option<AgentRunContext>,
    resume_checkpoint: &crate::task_lifecycle::TaskCheckpoint,
    initial_task_observations: &[Value],
) -> Result<AskReply, String> {
    info!(
        "run_agent_with_tools_seeded: task_id={} user_id={} chat_id={} checkpoint_id={} goal={}",
        task.task_id,
        task.user_id,
        task.chat_id,
        resume_checkpoint.checkpoint_id,
        crate::truncate_for_log(goal)
    );
    let user_text = user_request.trim();
    if !user_text.is_empty() {
        return run_agent_with_loop_seeded(
            state,
            task,
            goal,
            user_text,
            agent_run_context.as_ref(),
            Some(resume_checkpoint),
            initial_task_observations,
        )
        .await;
    }
    Ok(AskReply::non_llm(String::new()))
}

pub(crate) fn direct_capability_plan(
    _state: &AppState,
    capability: &str,
    args: Value,
) -> PlanResult {
    let requested = AgentAction::CallCapability {
        capability: capability.to_string(),
        args,
    };
    let mut actions = vec![requested];
    actions.push(AgentAction::SynthesizeAnswer {
        evidence_refs: Vec::new(),
    });
    let raw_plan = json!({
        "plan_source": "direct_capability",
        "capability": capability,
        "resolution_stage": "verifier"
    })
    .to_string();
    self::planning_actions::build_plan_result_with_notes(
        &format!("capability:{capability}"),
        &raw_plan,
        PlanKind::Single,
        &actions,
        "direct_capability",
    )
}

pub(crate) fn checkpoint_action_plan(
    tool_or_skill: &str,
    action_ref: &str,
    args: Value,
    completed_step_count: usize,
    output_contract: Option<crate::IntentOutputContract>,
) -> PlanResult {
    let actions = vec![
        AgentAction::CallCapability {
            capability: action_ref.to_string(),
            args,
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: Vec::new(),
        },
    ];
    let raw_plan = json!({
        "plan_source": "checkpoint_action",
        "action_ref": action_ref,
        "tool_or_skill": tool_or_skill,
        "resolution_stage": "persisted_checkpoint"
    })
    .to_string();
    let mut plan = self::planning_actions::build_plan_result_with_notes(
        &format!("checkpoint_action:{action_ref}"),
        &raw_plan,
        PlanKind::Single,
        &actions,
        "checkpoint_action",
    );
    let step_id_map = plan
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            (
                step.step_id.clone(),
                format!("step_{}", completed_step_count + index + 1),
            )
        })
        .collect::<HashMap<_, _>>();
    for step in &mut plan.steps {
        if let Some(step_id) = step_id_map.get(&step.step_id) {
            step.step_id = step_id.clone();
        }
        for dependency in &mut step.depends_on {
            if let Some(step_id) = step_id_map.get(dependency) {
                *dependency = step_id.clone();
            }
        }
    }
    plan.output_contract = output_contract;
    plan
}

pub(crate) async fn run_agent_with_tools_direct_plan(
    state: &AppState,
    task: &ClaimedTask,
    request_envelope: &str,
    agent_run_context: Option<AgentRunContext>,
    initial_plan: &PlanResult,
) -> Result<AskReply, String> {
    run_agent_with_loop_direct_plan(
        state,
        task,
        &initial_plan.goal,
        request_envelope,
        agent_run_context.as_ref(),
        initial_plan,
    )
    .await
}

pub(crate) async fn run_agent_with_tools_seeded_direct_plan(
    state: &AppState,
    task: &ClaimedTask,
    request_envelope: &str,
    agent_run_context: Option<AgentRunContext>,
    resume_checkpoint: &crate::task_lifecycle::TaskCheckpoint,
    initial_task_observations: &[Value],
    initial_plan: &PlanResult,
) -> Result<AskReply, String> {
    run_agent_with_loop_seeded_direct_plan(
        state,
        task,
        &initial_plan.goal,
        request_envelope,
        agent_run_context.as_ref(),
        resume_checkpoint,
        initial_plan,
        initial_task_observations,
    )
    .await
}

#[cfg(test)]
#[path = "agent_engine_tests.rs"]
mod tests;
