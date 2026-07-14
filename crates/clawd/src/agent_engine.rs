use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use tracing::{debug, info, warn};

mod arg_resolver;
mod async_start_checkpoint;
mod attempt_ledger;
#[allow(dead_code)]
mod context_compaction;
mod dispatch_support;
mod execution_loop;
mod filesystem_lifecycle_contract;
pub(crate) mod loop_control;
mod loop_state_contract_evidence;
pub(crate) mod migration_class;
pub(crate) mod observed_output;
mod planning;
mod planning_actions;
mod planning_followup;
mod planning_parse;
mod planning_path_metadata;
mod planning_prompt;
mod planning_recent_artifacts;
mod planning_registry_preference;
mod planning_route_markers;
mod planning_structured_field_exact;
mod prepare_round;
pub(crate) mod service_probe_contract;
mod skill_execution;
mod skill_quick_index;
mod subagent_runtime;
mod support;
mod user_output_path;

pub(crate) fn explicit_command_segment_for_policy(
    runtime: &crate::CommandIntentRuntime,
    request: &str,
) -> Option<String> {
    planning::explicit_command_segment(runtime, request)
}

pub(crate) fn explicit_execution_command_segment_for_policy(
    runtime: &crate::CommandIntentRuntime,
    request: &str,
) -> Option<String> {
    planning::explicit_execution_command_segment(runtime, request)
}

use self::arg_resolver::{
    normalize_skill_arg_aliases, resolve_arg_string, resolve_arg_value,
    rewrite_args_with_auto_locator_path, rewrite_run_cmd_with_written_aliases,
    rewrite_tool_path_with_written_aliases,
};
use self::dispatch_support::{classify_skill_failure_recovery, dispatch_round_action};

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
use self::execution_loop::execute_actions_once;
use self::loop_control::{run_agent_with_loop, run_agent_with_loop_seeded};
use self::loop_state_contract_evidence::{
    active_plan_file_targets_for_loop_seed, boundary_observation_needs_clarify_for_loop_seed,
    contract_repair_candidate_evidence_for_loop_seed,
    default_main_config_contract_evidence_for_loop_seed, first_string_field,
    pending_user_boundary_present_for_loop_seed, pre_loop_clarify_candidates_for_loop_seed,
    registry_capability_contract_evidence_for_loop_seed, registry_capability_contract_refs,
};
use self::prepare_round::{prepare_round_actions, push_round_trace};
use self::skill_quick_index::{
    output_contract as quick_index_output_contract,
    output_contract_metadata as quick_index_output_contract_metadata,
    planner_capabilities as quick_index_planner_capabilities,
    planner_capabilities_metadata as quick_index_planner_capabilities_metadata,
};

pub(crate) use self::filesystem_lifecycle_contract::{
    effective_filesystem_cleanup_recovery_output_contract_for_plan_steps,
    effective_filesystem_lifecycle_output_contract_for_plan_steps,
    enrich_scratch_filesystem_cleanup_runtime_args, route_can_upgrade_scratch_filesystem_lifecycle,
    scratch_filesystem_cleanup_recovery_action_allowed,
    scratch_filesystem_lifecycle_action_allowed, scratch_filesystem_lifecycle_observed_steps_match,
    scratch_filesystem_lifecycle_plan_actions_match, scratch_filesystem_lifecycle_plan_steps_match,
};
use self::skill_execution::execute_prepared_skill_action;
pub(crate) use self::support::append_delivery_message;
use self::support::{
    action_fingerprint_for_policy, append_progress_hint, build_safe_skill_args_summary,
    encode_progress_i18n, load_agent_loop_guard_policy, maybe_publish_execution_recipe_phase_hint,
    publish_agent_loop_user_input_checkpoint_progress, registry_idempotency_guard_attribution,
    AgentLoopGuardPolicy, PROGRESS_ARGS_SUMMARY_MAX_LEN,
};
pub(crate) use self::user_output_path::{
    action_has_user_named_output_path_marker, action_is_user_named_new_workspace_write,
    CLAWD_USER_NAMED_OUTPUT_PATH_ARG,
};

use crate::{repo, AgentAction, AppState, AskReply, ClaimedTask};

pub(crate) fn answer_verifier_enforce_required_enabled_for_route(
    state: &AppState,
    route_result: Option<&crate::RouteResult>,
) -> bool {
    load_agent_loop_guard_policy(state)
        .answer_verifier_required_evidence_enabled_for_route(route_result)
}

const AGENT_TOOL_SPEC_PATH: &str = "prompts/agent_tool_spec.md";
const CLAWD_CONTINUE_ON_ERROR_ARG: &str = "_clawd_continue_on_error";
const CLAWD_LITERAL_COMMAND_ARG: &str = "_clawd_literal_command";
const CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG: &str = "_clawd_literal_failure_repairable";
const CLAWD_MISSING_TARGET_REPAIRABLE_ARG: &str = "_clawd_missing_target_repairable";
pub(crate) const CLAWD_RUNTIME_ASYNC_JOB_START_ARG: &str = "_clawd_runtime_async_job_start";
const SINGLE_PLAN_EXECUTION_PROMPT_LOGICAL_PATH: &str = "prompts/single_plan_execution_prompt.md";
const LIGHTWEIGHT_EXECUTION_PROMPT_LOGICAL_PATH: &str = "prompts/lightweight_execution_prompt.md";
const LOOP_INCREMENTAL_PLAN_PROMPT_LOGICAL_PATH: &str = "prompts/loop_incremental_plan_prompt.md";
const LIGHTWEIGHT_INCREMENTAL_PLAN_PROMPT_LOGICAL_PATH: &str =
    "prompts/lightweight_incremental_plan_prompt.md";
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

/// Phase 2+: Planner 可见技能按 task/agent 动态收敛：
/// （execution-enabled）∩（agent allowed_skills）。
/// 每个可见技能需在 registry 中提供 skill prompt 的逻辑路径配置，才会注入 playbook。
fn planner_available_skills_for_task_scoped(
    state: &AppState,
    task: &ClaimedTask,
    skill_scope: Option<&BTreeSet<String>>,
) -> Vec<String> {
    let mut enabled = state.planner_available_skills_for_task(task);
    if let Some(skill_scope) = skill_scope {
        enabled.retain(|skill| skill_scope.contains(skill));
    }
    enabled
}

fn build_skill_playbooks_text_scoped(
    state: &AppState,
    task: &ClaimedTask,
    skill_scope: Option<&BTreeSet<String>>,
) -> String {
    let enabled = planner_available_skills_for_task_scoped(state, task, skill_scope);
    let enabled_count = enabled.len();
    let agent_id = state.task_agent_id(task);
    info!(
        "planner skill playbooks: agent_id={} planner_visible_skills_count={} scoped={} skills=[{}]",
        agent_id,
        enabled_count,
        skill_scope.is_some(),
        enabled.join(", ")
    );

    let mut sections = Vec::new();
    let mut skipped_no_prompt: Vec<String> = Vec::new();

    for skill in &enabled {
        let Some(registry_prompt_rel_path) = state.skill_registry_prompt_rel_path(skill) else {
            warn!(
                "planner skill playbook: skill={} registry prompt_file missing, skipping",
                skill
            );
            skipped_no_prompt.push(skill.clone());
            continue;
        };

        let prompt_body =
            crate::load_prompt_template_for_state(state, &registry_prompt_rel_path, "").0;

        debug!(
            "planner skill playbook: skill={} prompt_logical_path={} source=registry",
            skill, registry_prompt_rel_path
        );

        let trimmed = prompt_body.trim();
        if trimmed.is_empty() {
            continue;
        }
        let metadata = state
            .skill_manifest(skill)
            .map(|manifest| {
                let mut parts = vec![format!(
                    "planner_kind: {}",
                    manifest.planner_kind.as_token()
                )];
                parts.extend(crate::skill_availability::availability_metadata_parts(
                    &crate::skill_availability::evaluate_manifest_availability(&manifest),
                ));
                if let Some(capabilities) = quick_index_planner_capabilities_metadata(&manifest) {
                    parts.push(capabilities);
                }
                parts.push(quick_index_output_contract_metadata(&manifest));
                format!("Registry metadata: {}", parts.join("; "))
            })
            .unwrap_or_default();
        if metadata.is_empty() {
            sections.push(format!("### {skill}\n{trimmed}"));
        } else {
            sections.push(format!("### {skill}\n{trimmed}\n{metadata}"));
        }
    }

    if !skipped_no_prompt.is_empty() {
        warn!(
            "planner skill playbooks: skipped_no_prompt_count={} skills=[{}]",
            skipped_no_prompt.len(),
            skipped_no_prompt.join(", ")
        );
    }

    let included_count = sections.len();
    info!(
        "planner skill playbooks: included_count={} (enabled={} skipped={})",
        included_count,
        enabled_count,
        enabled_count.saturating_sub(included_count)
    );

    if sections.is_empty() {
        "No skill playbooks configured.".to_string()
    } else {
        sections.join("\n\n")
    }
}

fn first_non_heading_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line.starts_with('#')
                && !line.starts_with("```")
                && !line.starts_with("<!--")
        })
        .map(|line| {
            let mut out = line.to_string();
            if out.chars().count() > 90 {
                out = out.chars().take(90).collect::<String>() + "...";
            }
            out
        })
}

/// 首轮路由提示：给 LLM 一份“技能速览”，降低误判成纯 chat 的概率。
fn build_skill_quick_index_text_scoped(
    state: &AppState,
    task: &ClaimedTask,
    skill_scope: Option<&BTreeSet<String>>,
) -> String {
    let enabled = planner_available_skills_for_task_scoped(state, task, skill_scope);
    if enabled.is_empty() {
        return "- (no enabled skills)".to_string();
    }
    let mut lines = Vec::new();
    for skill in &enabled {
        let summary =
            if let Some(registry_prompt_rel_path) = state.skill_registry_prompt_rel_path(skill) {
                let prompt_body =
                    crate::load_prompt_template_for_state(state, &registry_prompt_rel_path, "").0;
                first_non_heading_line(&prompt_body).unwrap_or_else(|| {
                    "use this skill when user intent matches its capability".to_string()
                })
            } else {
                "skill enabled but registry prompt_file logical path missing".to_string()
            };
        if let Some(manifest) = state.skill_manifest(skill) {
            lines.push(format!(
                "- skill={skill}; summary={summary}; planner_kind={}{}{}",
                manifest.planner_kind.as_token(),
                quick_index_planner_capabilities(&manifest),
                quick_index_output_contract(&manifest)
            ));
        } else {
            lines.push(format!("- {skill}: {summary}"));
        }
    }
    lines.join("\n")
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
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    boundary_envelope: Option<&crate::intent_router::BoundaryEnvelope>,
    route_result: Option<&crate::RouteResult>,
) -> String {
    let mut lines = Vec::new();
    if let Some(envelope) = boundary_envelope {
        lines.push(envelope.compact_prompt_line());
    }
    if let Some(analysis) = turn_analysis {
        let turn_type = analysis
            .turn_type
            .map(crate::intent_router::TurnType::as_str)
            .unwrap_or("none");
        let target_task_policy = analysis
            .target_task_policy
            .map(crate::intent_router::TargetTaskPolicy::as_str)
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
    if let Some(route) = route_result {
        lines.push(crate::evidence_policy::evidence_policy_context_prompt_line_for_route(route));
        if let Some(evidence_policy_line) =
            crate::evidence_policy::compact_prompt_line_for_route(route)
        {
            lines.push(evidence_policy_line);
        }
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
    /// §7.1 output_contract 贯穿全链：RouteResult 里的 output_contract 与 route marker
    /// 合并成 effective machine contract 后挂到 LoopState 上。下游 synthesis/finalize
    /// 和无 RouteResult 入参的 preflight 必须看见这些字段，否则容易把结构化
    /// existence_with_path / scalar / file_token 契约答成自由段落。
    /// 默认 None：测试与不走 RouteResult 的 ad-hoc 路径保持向后兼容。
    pub(crate) output_contract: Option<crate::IntentOutputContract>,
    /// Route-level policy context keeps planner capability refs available to
    /// preflight paths that otherwise only see `output_contract`.
    pub(crate) route_policy_context: Option<crate::RouteResult>,
    /// True only while executing the current round's verifier-approved actions.
    /// Preflight uses this as a narrow bridge from legacy route guards to the
    /// agent-loop authority model; it must be cleared immediately after action
    /// dispatch returns.
    pub(crate) verified_action_window_active: bool,
    /// Machine boundary fact from AGENT_LOOP_BOUNDARY_OBSERVATIONS. This lets
    /// the loop accept a terminal clarification even when legacy RouteResult
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoopStateCheckpointSeedReport {
    pub(crate) checkpoint_id: String,
    pub(crate) resume_entrypoint: crate::task_lifecycle::ResumeEntrypoint,
    pub(crate) restored_round: usize,
    pub(crate) restored_step: usize,
    pub(crate) restored_tool_calls: usize,
    pub(crate) completed_side_effect_count: usize,
    pub(crate) observation_count: usize,
}

pub(crate) fn seed_loop_state_from_task_checkpoint(
    loop_state: &mut LoopState,
    checkpoint: &crate::task_lifecycle::TaskCheckpoint,
) -> LoopStateCheckpointSeedReport {
    let restored_round = checkpoint.budget.round as usize;
    let restored_step = checkpoint.budget.step as usize;
    let restored_tool_calls = checkpoint.budget.tool_calls as usize;
    loop_state.round_no = loop_state.round_no.max(restored_round);
    loop_state.total_steps_executed = loop_state.total_steps_executed.max(restored_step);
    loop_state.tool_calls_total = loop_state.tool_calls_total.max(restored_tool_calls);

    let mut completed_side_effect_count = 0usize;
    for fingerprint in &checkpoint.completed_side_effect_refs {
        let fingerprint = fingerprint.trim();
        if fingerprint.is_empty() {
            continue;
        }
        completed_side_effect_count += 1;
        loop_state
            .successful_action_fingerprints
            .entry(fingerprint.to_string())
            .or_insert(1);
    }

    if !checkpoint.observations.is_empty() {
        loop_state.has_tool_or_skill_output = true;
    }
    let changed_files = checkpoint
        .artifact_refs
        .iter()
        .filter_map(|artifact_ref| artifact_ref.trim().strip_prefix("changed_file:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if let Some(last) = changed_files.last() {
        loop_state.last_written_file_path = Some(last.clone());
        loop_state
            .output_vars
            .insert("last_written_file_path".to_string(), last.clone());
    }
    if !changed_files.is_empty() {
        if let Ok(serialized) = serde_json::to_string(&changed_files) {
            loop_state.output_vars.insert(
                "agent_loop.resume_changed_files_json".to_string(),
                serialized,
            );
        }
    }
    loop_state.task_checkpoint = Some(checkpoint.to_machine_json());
    let resume_entrypoint = checkpoint_resume_entrypoint_token(&checkpoint.resume_entrypoint);
    loop_state.output_vars.insert(
        "agent_loop.resume_checkpoint_id".to_string(),
        checkpoint.checkpoint_id.clone(),
    );
    loop_state.output_vars.insert(
        "agent_loop.resume_entrypoint".to_string(),
        resume_entrypoint.to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.resume_completed_side_effect_count".to_string(),
        completed_side_effect_count.to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.resume_observation_count".to_string(),
        checkpoint.observations.len().to_string(),
    );
    if let Some(attempt_ledger) = checkpoint
        .attempt_ledger
        .as_ref()
        .filter(|value| value.is_array() || value.is_object())
    {
        loop_state.output_vars.insert(
            "agent_loop.resume_attempt_ledger_present".to_string(),
            "true".to_string(),
        );
        if let Ok(snapshot) = serde_json::to_string(attempt_ledger) {
            loop_state
                .history_compact
                .push(format!("checkpoint_attempt_ledger_json={snapshot}"));
        }
    }

    loop_state.history_compact.push(format!(
        "checkpoint_resume checkpoint_id={} entrypoint={} round={} step={} tool_calls={} side_effects={} observations={}",
        checkpoint.checkpoint_id,
        resume_entrypoint,
        restored_round,
        restored_step,
        restored_tool_calls,
        completed_side_effect_count,
        checkpoint.observations.len()
    ));

    LoopStateCheckpointSeedReport {
        checkpoint_id: checkpoint.checkpoint_id.clone(),
        resume_entrypoint: checkpoint.resume_entrypoint.clone(),
        restored_round,
        restored_step,
        restored_tool_calls,
        completed_side_effect_count,
        observation_count: checkpoint.observations.len(),
    }
}

fn checkpoint_resume_entrypoint_token(
    entrypoint: &crate::task_lifecycle::ResumeEntrypoint,
) -> &'static str {
    match entrypoint {
        crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound => "next_planner_round",
        crate::task_lifecycle::ResumeEntrypoint::PollAsyncJob => "poll_async_job",
        crate::task_lifecycle::ResumeEntrypoint::AwaitUserInput => "await_user_input",
        crate::task_lifecycle::ResumeEntrypoint::VerifyAndFinalize => "verify_and_finalize",
    }
}

fn seed_loop_state_from_agent_context(
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) {
    let Some(ctx) = agent_run_context else {
        return;
    };
    if let Some(path) = ctx
        .auto_locator_path
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        loop_state
            .output_vars
            .insert("auto_locator_path".to_string(), path.to_string());
    }
    if let Some(route) = ctx.route_result.as_ref() {
        loop_state.output_vars.insert(
            "route_locator_kind".to_string(),
            route.output_contract.locator_kind.as_str().to_string(),
        );
        loop_state.output_contract = Some(route.effective_output_contract());
        loop_state.route_policy_context = Some(route.clone());
    }
    if let Some(cross_turn_ctx) = ctx
        .cross_turn_recent_execution_context
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "<none>")
    {
        loop_state.output_vars.insert(
            "cross_turn_recent_execution_context".to_string(),
            cross_turn_ctx.to_string(),
        );
    }
    let alias_bindings = session_alias_bindings_for_loop_seed(ctx);
    let alias_request_texts = [
        ctx.original_user_request.as_deref(),
        ctx.user_request
            .as_deref()
            .map(alias_mention_request_surface),
    ];
    let mut required_alias_targets = Vec::new();
    for alias_request_text in alias_request_texts.into_iter().flatten() {
        required_alias_targets.extend(
            crate::conversation_state::alias_bindings_mentioned_in_prompt(
                &alias_bindings,
                alias_request_text,
            )
            .into_iter()
            .filter_map(|binding| {
                let target = binding.target.trim();
                (!target.is_empty()).then_some(target.to_string())
            }),
        );
    }
    required_alias_targets.sort();
    required_alias_targets.dedup();
    if !required_alias_targets.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&required_alias_targets) {
            loop_state
                .output_vars
                .insert("required_session_alias_targets".to_string(), encoded);
        }
    }
    let active_bound_targets = active_bound_targets_for_loop_seed(ctx);
    if !active_bound_targets.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&active_bound_targets) {
            loop_state
                .output_vars
                .insert("active_bound_targets".to_string(), encoded);
        }
    }
    let file_delivery_target_candidates = file_delivery_target_candidates_for_loop_seed(ctx);
    if !file_delivery_target_candidates.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&file_delivery_target_candidates) {
            loop_state
                .output_vars
                .insert("file_delivery_target_candidates".to_string(), encoded);
        }
    }
    let active_listing_bound_targets = active_listing_bound_targets_for_loop_seed(ctx);
    if !active_listing_bound_targets.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&active_listing_bound_targets) {
            loop_state
                .output_vars
                .insert("active_listing_bound_targets".to_string(), encoded);
        }
    }
    let current_workspace_scalar_count_targets =
        current_workspace_scalar_count_targets_for_loop_seed(ctx);
    if !current_workspace_scalar_count_targets.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&current_workspace_scalar_count_targets) {
            loop_state.output_vars.insert(
                "current_workspace_scalar_count_targets".to_string(),
                encoded,
            );
        }
    }
    let active_plan_file_targets = active_plan_file_targets_for_loop_seed(ctx);
    if !active_plan_file_targets.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&active_plan_file_targets) {
            loop_state
                .output_vars
                .insert("active_plan_file_targets".to_string(), encoded);
        }
    }
    if boundary_observation_needs_clarify_for_loop_seed(ctx) {
        loop_state.boundary_observation_needs_clarify = true;
        loop_state.output_vars.insert(
            "agent_loop.boundary_observation_needs_clarify".to_string(),
            "true".to_string(),
        );
    }
    if pending_user_boundary_present_for_loop_seed(ctx) {
        loop_state.pending_user_boundary_present = true;
        loop_state.output_vars.insert(
            "agent_loop.pending_user_boundary_present".to_string(),
            "true".to_string(),
        );
    }
    let current_request_locator_evidence = current_request_locator_evidence_for_loop_seed(ctx);
    if !current_request_locator_evidence.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&current_request_locator_evidence) {
            loop_state
                .output_vars
                .insert("current_request_locator_evidence".to_string(), encoded);
        }
    }
    let current_request_resolved_workspace_child_targets =
        current_request_resolved_workspace_child_targets(&current_request_locator_evidence);
    if !current_request_resolved_workspace_child_targets.is_empty() {
        if let Ok(encoded) =
            serde_json::to_string(&current_request_resolved_workspace_child_targets)
        {
            loop_state.output_vars.insert(
                "current_request_resolved_workspace_child_targets".to_string(),
                encoded,
            );
        }
    }
    let default_main_config_contract_evidence =
        default_main_config_contract_evidence_for_loop_seed(ctx);
    if !default_main_config_contract_evidence.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&default_main_config_contract_evidence) {
            loop_state
                .output_vars
                .insert("default_main_config_contract_evidence".to_string(), encoded);
        }
        if let Some(logical_path) =
            first_string_field(&default_main_config_contract_evidence, "logical_path")
        {
            loop_state.output_vars.insert(
                "default_main_config_contract_logical_path".to_string(),
                logical_path,
            );
        }
        if let Some(workspace_path) =
            first_string_field(&default_main_config_contract_evidence, "workspace_path")
        {
            loop_state.output_vars.insert(
                "default_main_config_contract_workspace_path".to_string(),
                workspace_path,
            );
        }
    }
    let registry_capability_contract_evidence =
        registry_capability_contract_evidence_for_loop_seed(ctx);
    if !registry_capability_contract_evidence.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&registry_capability_contract_evidence) {
            loop_state
                .output_vars
                .insert("registry_capability_contract_evidence".to_string(), encoded);
        }
        let refs = registry_capability_contract_refs(&registry_capability_contract_evidence);
        if !refs.is_empty() {
            if let Ok(encoded) = serde_json::to_string(&refs) {
                loop_state
                    .output_vars
                    .insert("registry_capability_contract_refs".to_string(), encoded);
            }
        }
    }
    let contract_repair_candidate_evidence = contract_repair_candidate_evidence_for_loop_seed(ctx);
    if !contract_repair_candidate_evidence.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&contract_repair_candidate_evidence) {
            loop_state
                .output_vars
                .insert("contract_repair_candidate_evidence".to_string(), encoded);
        }
    }
    let pre_loop_clarify_candidates = pre_loop_clarify_candidates_for_loop_seed(ctx);
    if !pre_loop_clarify_candidates.is_empty() {
        if let Ok(encoded) = serde_json::to_string(&pre_loop_clarify_candidates) {
            loop_state
                .output_vars
                .insert("pre_loop_clarify_candidates".to_string(), encoded);
        }
    }
    if let Some(spec) = ctx.execution_recipe_hint {
        loop_state.output_vars.insert(
            "route_execution_recipe_kind".to_string(),
            spec.kind.as_str().to_string(),
        );
        loop_state.output_vars.insert(
            "route_execution_recipe_profile".to_string(),
            spec.profile.as_str().to_string(),
        );
        loop_state.output_vars.insert(
            "route_execution_recipe_target_scope".to_string(),
            spec.target_scope.as_str().to_string(),
        );
    }
    if let Some(plan_hint) = ctx.execution_recipe_plan_hint.as_ref() {
        if !plan_hint.kind.trim().is_empty() {
            loop_state.output_vars.insert(
                "route_execution_recipe_plan_kind".to_string(),
                plan_hint.kind.trim().to_string(),
            );
        }
        if let Some(command) = plan_hint
            .command
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            loop_state.output_vars.insert(
                "route_execution_recipe_plan_command".to_string(),
                command.to_string(),
            );
        }
        if let Some(mode) = plan_hint
            .execution_mode
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            loop_state.output_vars.insert(
                "route_execution_recipe_plan_execution_mode".to_string(),
                mode.to_string(),
            );
        }
        if let Some(adapter_kind) = plan_hint
            .async_adapter_kind
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            loop_state.output_vars.insert(
                "route_execution_recipe_plan_async_adapter_kind".to_string(),
                adapter_kind.to_string(),
            );
        }
    }
}

pub(crate) fn seed_loop_state_for_agent_run(
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    resume_checkpoint: Option<&crate::task_lifecycle::TaskCheckpoint>,
) -> Option<LoopStateCheckpointSeedReport> {
    let checkpoint_seed_report = resume_checkpoint
        .map(|checkpoint| seed_loop_state_from_task_checkpoint(loop_state, checkpoint));
    seed_loop_state_from_agent_context(loop_state, agent_run_context);
    checkpoint_seed_report
}

fn alias_mention_request_surface(text: &str) -> &str {
    text.split("### SESSION_ALIAS_BINDINGS")
        .next()
        .unwrap_or(text)
}

fn session_alias_bindings_for_loop_seed(
    ctx: &AgentRunContext,
) -> Vec<crate::conversation_state::SessionAliasBinding> {
    let mut bindings = ctx.session_alias_bindings.clone();
    if let Some(summary) = ctx.context_bundle_summary.as_deref() {
        bindings.extend(session_alias_bindings_from_context_summary(summary));
    }
    let mut seen = std::collections::BTreeSet::new();
    bindings.retain(|binding| {
        let alias = binding.alias.trim();
        let target = binding.target.trim();
        if alias.is_empty() || target.is_empty() {
            return false;
        }
        seen.insert((alias.to_string(), target.to_string()))
    });
    bindings
}

fn session_alias_bindings_from_context_summary(
    summary: &str,
) -> Vec<crate::conversation_state::SessionAliasBinding> {
    let mut out = session_alias_bindings_from_context_alias_block(summary);
    out.extend(session_alias_bindings_from_boundary_observation_blocks(
        summary,
    ));
    out
}

fn session_alias_bindings_from_context_alias_block(
    summary: &str,
) -> Vec<crate::conversation_state::SessionAliasBinding> {
    let marker = "### SESSION_ALIAS_BINDINGS";
    let Some((_, tail)) = summary.split_once(marker) else {
        return Vec::new();
    };
    let block = tail.split("\n### ").next().unwrap_or(tail);
    let mut current_alias: Option<String> = None;
    let mut out = Vec::new();
    for line in block.lines() {
        let trimmed = line.trim();
        let alias = trimmed
            .strip_prefix("- alias:")
            .or_else(|| trimmed.strip_prefix("alias:"))
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(alias) = alias {
            current_alias = Some(alias.to_string());
            continue;
        }
        let target = trimmed
            .strip_prefix("target:")
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let (Some(alias), Some(target)) = (current_alias.take(), target) {
            out.push(crate::conversation_state::SessionAliasBinding {
                alias,
                target: target.to_string(),
                updated_at_ts: 0,
            });
        }
    }
    out
}

fn session_alias_bindings_from_boundary_observation_blocks(
    summary: &str,
) -> Vec<crate::conversation_state::SessionAliasBinding> {
    const START: &str = "### AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    const END: &str = "### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    let mut out = Vec::new();
    for tail in summary.split(START).skip(1) {
        let block = tail.split(END).next().unwrap_or(tail).trim();
        let Ok(value) = serde_json::from_str::<Value>(block) else {
            continue;
        };
        let Some(bindings) = value
            .get("session_alias_bindings")
            .and_then(Value::as_array)
        else {
            continue;
        };
        for binding in bindings {
            let alias = binding
                .get("alias")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let target = binding
                .get("target")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            if let (Some(alias), Some(target)) = (alias, target) {
                out.push(crate::conversation_state::SessionAliasBinding {
                    alias: alias.to_string(),
                    target: target.to_string(),
                    updated_at_ts: 0,
                });
            }
        }
    }
    out
}

fn active_bound_targets_for_loop_seed(ctx: &AgentRunContext) -> Vec<String> {
    let mut targets = Vec::new();
    for summary in [
        ctx.user_request.as_deref(),
        ctx.context_bundle_summary.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        targets.extend(active_bound_targets_from_boundary_observation_blocks(
            summary,
        ));
    }
    targets.sort();
    targets.dedup();
    targets
}

fn file_delivery_target_candidates_for_loop_seed(ctx: &AgentRunContext) -> Vec<String> {
    let mut targets = Vec::new();
    for summary in [
        ctx.user_request.as_deref(),
        ctx.context_bundle_summary.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        targets.extend(file_delivery_target_candidates_from_boundary_observation_blocks(summary));
    }
    targets.sort();
    targets.dedup();
    targets
}

fn active_listing_bound_targets_for_loop_seed(ctx: &AgentRunContext) -> Vec<String> {
    let mut targets = Vec::new();
    for summary in [
        ctx.user_request.as_deref(),
        ctx.context_bundle_summary.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        targets.extend(active_listing_bound_targets_from_boundary_observation_blocks(summary));
    }
    targets.sort();
    targets.dedup();
    targets
}

fn current_workspace_scalar_count_targets_for_loop_seed(ctx: &AgentRunContext) -> Vec<String> {
    let mut targets = Vec::new();
    for summary in [
        ctx.user_request.as_deref(),
        ctx.context_bundle_summary.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        targets.extend(
            current_workspace_scalar_count_targets_from_boundary_observation_blocks(summary),
        );
    }
    targets.sort();
    targets.dedup();
    targets
}

fn current_request_locator_evidence_for_loop_seed(ctx: &AgentRunContext) -> Vec<Value> {
    let mut evidence = Vec::new();
    for summary in [
        ctx.user_request.as_deref(),
        ctx.context_bundle_summary.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        evidence.extend(current_request_locator_evidence_from_boundary_observation_blocks(summary));
    }
    evidence.sort_by_key(|value| serde_json::to_string(value).unwrap_or_default());
    evidence.dedup_by(|left, right| left == right);
    evidence
}

fn current_request_locator_evidence_from_boundary_observation_blocks(summary: &str) -> Vec<Value> {
    const START: &str = "### AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    const END: &str = "### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    let mut out = Vec::new();
    for tail in summary.split(START).skip(1) {
        let block = tail.split(END).next().unwrap_or(tail).trim();
        let Ok(value) = serde_json::from_str::<Value>(block) else {
            continue;
        };
        let Some(locator) = value
            .get("current_request_locator")
            .and_then(Value::as_object)
        else {
            continue;
        };
        let has_concrete_surface = locator
            .get("has_concrete_surface")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let has_resolved_workspace_child = locator
            .get("resolved_workspace_child")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|target| !target.is_empty());
        let has_explicit_locator_hints = locator
            .get("explicit_locator_hints")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty());
        if has_concrete_surface || has_resolved_workspace_child || has_explicit_locator_hints {
            out.push(Value::Object(locator.clone()));
        }
    }
    out
}

fn current_request_resolved_workspace_child_targets(evidence: &[Value]) -> Vec<String> {
    let mut targets = evidence
        .iter()
        .filter_map(|value| {
            value
                .get("resolved_workspace_child")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|target| !target.is_empty())
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();
    targets.sort();
    targets.dedup();
    targets
}

fn active_bound_targets_from_boundary_observation_blocks(summary: &str) -> Vec<String> {
    const START: &str = "### AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    const END: &str = "### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    let mut out = Vec::new();
    for tail in summary.split(START).skip(1) {
        let block = tail.split(END).next().unwrap_or(tail).trim();
        let Ok(value) = serde_json::from_str::<Value>(block) else {
            continue;
        };
        let Some(targets) = value.get("active_bound_targets").and_then(Value::as_array) else {
            continue;
        };
        for target_value in targets {
            let Some(target) = target_value
                .get("target")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                if let Some(ordered_targets) = target_value
                    .get("ordered_targets")
                    .and_then(Value::as_array)
                {
                    out.extend(
                        ordered_targets
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(ToString::to_string),
                    );
                }
                continue;
            };
            out.push(target.to_string());
            if let Some(ordered_targets) = target_value
                .get("ordered_targets")
                .and_then(Value::as_array)
            {
                out.extend(
                    ordered_targets
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToString::to_string),
                );
            }
        }
    }
    out
}

fn file_delivery_target_candidates_from_boundary_observation_blocks(summary: &str) -> Vec<String> {
    const START: &str = "### AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    const END: &str = "### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    let mut out = Vec::new();
    for tail in summary.split(START).skip(1) {
        let block = tail.split(END).next().unwrap_or(tail).trim();
        let Ok(value) = serde_json::from_str::<Value>(block) else {
            continue;
        };
        let Some(targets) = value
            .get("file_delivery_target_candidates")
            .and_then(Value::as_array)
        else {
            continue;
        };
        out.extend(
            targets
                .iter()
                .filter_map(|target_value| target_value.get("target"))
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
        );
    }
    out
}

fn current_workspace_scalar_count_targets_from_boundary_observation_blocks(
    summary: &str,
) -> Vec<String> {
    const START: &str = "### AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    const END: &str = "### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    let mut out = Vec::new();
    for tail in summary.split(START).skip(1) {
        let block = tail.split(END).next().unwrap_or(tail).trim();
        let Ok(value) = serde_json::from_str::<Value>(block) else {
            continue;
        };
        let Some(scope) = value.get("current_workspace_scope") else {
            continue;
        };
        if !current_workspace_scope_marks_scalar_count(scope) {
            continue;
        }
        let Some(target) = scope
            .get("target")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        out.push(target.to_string());
    }
    out
}

fn current_workspace_scope_marks_scalar_count(scope: &Value) -> bool {
    const SCALAR_COUNT_MARKER: &str = "scalar_count";
    ["task_shape", "contract_marker", "output_contract_marker"]
        .into_iter()
        .filter_map(|key| scope.get(key).and_then(Value::as_str))
        .map(str::trim)
        .any(|value| value == SCALAR_COUNT_MARKER)
}

fn active_listing_bound_targets_from_boundary_observation_blocks(summary: &str) -> Vec<String> {
    const START: &str = "### AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    const END: &str = "### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    let mut out = Vec::new();
    for tail in summary.split(START).skip(1) {
        let block = tail.split(END).next().unwrap_or(tail).trim();
        let Ok(value) = serde_json::from_str::<Value>(block) else {
            continue;
        };
        let Some(targets) = value.get("active_bound_targets").and_then(Value::as_array) else {
            continue;
        };
        for target in targets {
            if !active_target_observation_has_listing_evidence(target) {
                continue;
            }
            let Some(target) = target
                .get("target")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            out.push(target.to_string());
        }
    }
    out
}

fn active_target_observation_has_listing_evidence(value: &Value) -> bool {
    value
        .get("op_kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind == "list")
        || value
            .get("ordered_entry_count")
            .and_then(Value::as_u64)
            .is_some_and(|count| count > 0)
        || value
            .get("observed_entry_count")
            .and_then(Value::as_u64)
            .is_some()
}

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
    pub(crate) route_result: Option<crate::RouteResult>,
    pub(crate) execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    pub(crate) execution_recipe_plan_hint: Option<crate::intent_router::ExecutionRecipePlanHint>,
    pub(crate) turn_analysis: Option<crate::intent_router::TurnAnalysis>,
    pub(crate) boundary_envelope: Option<crate::intent_router::BoundaryEnvelope>,
    pub(crate) context_bundle_summary: Option<String>,
    pub(crate) session_alias_bindings: Vec<crate::conversation_state::SessionAliasBinding>,
    pub(crate) auto_locator_path: Option<String>,
    pub(crate) original_user_request: Option<String>,
    pub(crate) user_request: Option<String>,
    /// Cross-turn recent execution context for planner access to prior observed outputs,
    /// aliases, and targets when the current turn references previous turns.
    pub(crate) cross_turn_recent_execution_context: Option<String>,
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
        return crate::truncate_for_agent_trace(&crate::skills::normalize_skill_error_for_user(
            skill, trimmed,
        ));
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
    let structured = raw_err
        .map(str::trim)
        .filter(|err| !err.is_empty())
        .and_then(crate::skills::parse_structured_skill_error)?;
    Some(json!({
        "skill": structured.skill,
        "error_kind": structured.error_kind,
        "error_text": structured.error_text,
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
    let resume_context = json!({
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
        "hint": "User must explicitly confirm before executing the remaining actions."
    });
    let language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_request);
    let contract = crate::fallback::UserResponseContract::verifier_gate(
        "resume_confirmation_required",
        user_request,
        goal,
        vec!["explicit_user_confirmation".to_string()],
        vec![
            "verification_issue_kind: ConfirmationRequired".to_string(),
            format!("remaining_steps_count: {remaining_steps_count}"),
            "needs_confirmation: true".to_string(),
            format!("confirmation_detail_present: {}", !detail.trim().is_empty()),
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
) -> Result<AskReply, String> {
    info!(
        "run_agent_with_tools: task_id={} user_id={} chat_id={} goal={}",
        task.task_id,
        task.user_id,
        task.chat_id,
        crate::truncate_for_log(goal)
    );
    let user_text = user_request.trim();
    if !user_text.is_empty() {
        return run_agent_with_loop(state, task, goal, user_text, agent_run_context.as_ref()).await;
    }
    return Ok(AskReply::non_llm(String::new()));
}

pub(crate) async fn run_agent_with_tools_seeded(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_request: &str,
    agent_run_context: Option<AgentRunContext>,
    resume_checkpoint: &crate::task_lifecycle::TaskCheckpoint,
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
        )
        .await;
    }
    Ok(AskReply::non_llm(String::new()))
}

#[cfg(test)]
#[path = "agent_engine_tests.rs"]
mod tests;
