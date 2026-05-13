use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, info, warn};

mod arg_resolver;
mod attempt_ledger;
mod dispatch_support;
mod execution_loop;
pub(crate) mod loop_control;
pub(crate) mod observed_output;
mod planning;
mod prepare_round;
mod skill_execution;
mod support;

use self::arg_resolver::{
    normalize_skill_arg_aliases, resolve_arg_string, resolve_arg_value,
    rewrite_args_with_auto_locator_path, rewrite_run_cmd_with_written_aliases,
    rewrite_tool_path_with_written_aliases,
};
use self::dispatch_support::{classify_skill_failure_recovery, dispatch_round_action};
use self::execution_loop::execute_actions_once;
use self::loop_control::run_agent_with_loop;
use self::prepare_round::{prepare_round_actions, push_round_trace};

// Phase 3.3 Stage 2.3：loop_finalize.rs 已物理搬移到 `crate::finalize::loop_reply`。
// observed_output 暴露 synthesize_answer_from_observed_output 给 finalize facade。
pub(crate) use self::observed_output::synthesize_answer_from_observed_output;
use self::skill_execution::execute_prepared_skill_action;
pub(crate) use self::support::append_delivery_message;
use self::support::{
    action_fingerprint, append_progress_hint, build_safe_skill_args_summary, encode_progress_i18n,
    load_agent_loop_guard_policy, maybe_publish_execution_recipe_phase_hint, AgentLoopGuardPolicy,
    PROGRESS_ARGS_SUMMARY_MAX_LEN,
};

use crate::{repo, AgentAction, AppState, AskReply, ClaimedTask};

const AGENT_TOOL_SPEC_PATH: &str = "prompts/agent_tool_spec.md";
const CLAWD_CONTINUE_ON_ERROR_ARG: &str = "_clawd_continue_on_error";
const CLAWD_LITERAL_COMMAND_ARG: &str = "_clawd_literal_command";
const CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG: &str = "_clawd_literal_failure_repairable";
const CLAWD_MISSING_TARGET_REPAIRABLE_ARG: &str = "_clawd_missing_target_repairable";
const SINGLE_PLAN_EXECUTION_PROMPT_LOGICAL_PATH: &str = "prompts/single_plan_execution_prompt.md";
const LIGHTWEIGHT_EXECUTION_PROMPT_LOGICAL_PATH: &str = "prompts/lightweight_execution_prompt.md";
const LOOP_INCREMENTAL_PLAN_PROMPT_LOGICAL_PATH: &str = "prompts/loop_incremental_plan_prompt.md";
const PLAN_REPAIR_PROMPT_LOGICAL_PATH: &str = "prompts/plan_repair_prompt.md";
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
fn build_skill_playbooks_text(state: &AppState, task: &ClaimedTask) -> String {
    let enabled = state.planner_available_skills_for_task(task);
    let enabled_count = enabled.len();
    let agent_id = state.task_agent_id(task);
    info!(
        "planner skill playbooks: agent_id={} planner_visible_skills_count={} skills=[{}]",
        agent_id,
        enabled_count,
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
fn build_skill_quick_index_text(state: &AppState, task: &ClaimedTask) -> String {
    let enabled = state.planner_available_skills_for_task(task);
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
                "- {skill}: {summary}; planner_kind: {}",
                manifest.planner_kind.as_token()
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
            ("__RUNTIME_OS__", runtime_os),
            ("__RUNTIME_SHELL__", runtime_shell),
            ("__WORKSPACE_ROOT__", workspace_root),
        ],
    )
}

fn build_turn_analysis_prompt_block(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    route_result: Option<&crate::RouteResult>,
) -> String {
    let mut lines = Vec::new();
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
        lines.push(crate::TaskContract::from_route_result(route).compact_prompt_line());
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
    pub(crate) execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState,
    pub(crate) last_recipe_progress_phase: Option<crate::execution_recipe::ExecutionRecipePhase>,
    pub(crate) last_recipe_progress_scope:
        Option<crate::execution_recipe::ExecutionRecipeTargetScope>,
    pub(crate) recipe_scope_ready_hint_sent: bool,
    /// §7.1 output_contract 贯穿全链：normalizer 已经在 RouteResult.output_contract
    /// 里给出了 response_shape / semantic_kind / locator_hint 等 answer-shape spec；
    /// 下游 synthesis/finalize 必须看见这些字段，否则容易把 normalizer 标出的
    /// existence_with_path / scalar / file_token 契约答成自由段落。把契约挂到
    /// LoopState 上，由 runtime synthesis 和 finalize verifier 共同使用。
    /// 默认 None：测试与不走 RouteResult 的 ad-hoc 路径保持向后兼容。
    pub(crate) output_contract: Option<crate::IntentOutputContract>,
}

impl LoopState {
    pub(crate) fn new(max_rounds: usize) -> Self {
        Self {
            max_rounds,
            ..Self::default()
        }
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
        // §7.1: 把 normalizer 算出的 output_contract 挂到 loop 上，让 synthesis/finalize
        // 能拿到判定依据。
        // clone 因为 RouteResult 跨 await 共享，loop 内部要独立可写。
        loop_state.output_contract = Some(route.output_contract.clone());
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
    pub(crate) turn_analysis: Option<crate::intent_router::TurnAnalysis>,
    pub(crate) context_bundle_summary: Option<String>,
    pub(crate) auto_locator_path: Option<String>,
    pub(crate) fuzzy_locator_suggestions: Vec<String>,
    pub(crate) original_user_request: Option<String>,
    pub(crate) user_request: Option<String>,
    /// Cross-turn recent execution context (rendered by routing_context::build_recent_execution_context).
    /// Used by runtime synthesis/planning so the LLM can see prior turns' outputs (file content,
    /// list_dir results, alias bindings, etc.) when the current turn references "上一个文件 / 上上个 /
    /// 那个文件 / 甲 / 乙" or asks to compare/relate prior outputs.
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
    loop_state.output_vars.insert(
        "failed_step.action".to_string(),
        failed_action.trim().to_string(),
    );
    loop_state
        .output_vars
        .insert("failed_step.index".to_string(), round_step.to_string());
}

type WriteFileEffectivePath = (String, String, String);

fn plan_step_label(action: &AgentAction) -> String {
    match action {
        // LEGACY: CallTool shown as skill for unified capability view.
        AgentAction::CallTool { tool, .. } => format!("skill:{tool}"),
        AgentAction::CallSkill { skill, .. } => format!("skill:{skill}"),
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

fn user_safe_step_error(err: &str, prefer_english: bool) -> String {
    let trimmed = err.trim();
    if trimmed.is_empty() {
        return if prefer_english {
            "Execution failed without a clear reason.".to_string()
        } else {
            "执行失败，但没有返回明确原因".to_string()
        };
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
    let failed_index_text = failed_index.to_string();
    let safe_err = user_safe_step_error(err, prefer_english);
    let fallback_user_error = if has_remaining_actions {
        crate::bilingual_t_with_default_vars(
            state,
            "clawd.msg.resume_step_failed_with_remaining",
            "第 {failed_index} 步未完成：{err}。后续步骤已暂停，你可以回复“继续”来执行剩余步骤。",
            "Step {failed_index} could not be completed: {err}. Remaining steps were paused. Reply \"continue\" to run them.",
            prefer_english,
            &[
                ("failed_index", &failed_index_text),
                ("failed_action", failed_action),
                ("err", &safe_err),
            ],
        )
    } else {
        crate::bilingual_t_with_default_vars(
            state,
            "clawd.msg.resume_step_failed_no_remaining",
            "第 {failed_index} 步未完成：{err}",
            "Step {failed_index} could not be completed: {err}",
            prefer_english,
            &[
                ("failed_index", &failed_index_text),
                ("failed_action", failed_action),
                ("err", &safe_err),
            ],
        )
    };
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
        "Do not expose raw resume_context JSON, internal action schema, stack traces, or prompt names."
            .to_string(),
        "Do not claim the failed step succeeded.".to_string(),
        "Keep the reply focused on the failed step and the immediate recovery path.".to_string(),
    ];
    if has_remaining_actions {
        policy_boundary.push(
            "Mention that remaining steps are paused and the user can reply continue to resume them."
                .to_string(),
        );
    } else {
        policy_boundary.push("Do not invent remaining work or a continuation option.".to_string());
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
    let prefer_english = language_hint.to_ascii_lowercase().starts_with("en");
    let fallback_user_error = crate::bilingual_t_with_default_vars(
        state,
        "clawd.msg.resume_confirmation_required",
        "这一步需要你先明确确认，我还不会直接执行。你可以回复“继续”来执行剩余步骤。\n原因：{detail}",
        "This step needs your explicit confirmation before I execute it. Reply \"continue\" to run the remaining steps.\nReason: {detail}",
        prefer_english,
        &[("detail", detail)],
    );
    let contract = crate::fallback::UserResponseContract::verifier_gate(
        "resume_confirmation_required",
        user_request,
        goal,
        vec!["explicit_user_confirmation".to_string()],
        vec![
            format!(
                "verification_detail: {}",
                crate::truncate_for_agent_trace(detail)
            ),
            format!("remaining_steps_count: {remaining_steps_count}"),
            "needs_confirmation: true".to_string(),
        ],
        "brief_failure_with_continue_option",
        &language_hint,
    );
    let user_error = crate::fallback::compose_user_response_from_contract_with_default(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::VerifyRejected,
        &fallback_user_error,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- build_safe_skill_args_summary: progress hint args must be whitelisted and safe ---
    #[test]
    fn test_build_safe_skill_args_summary_whitelist_order() {
        let args = json!({
            "symbol": "DOGEUSDT",
            "action": "open_orders",
            "exchange": "binance"
        });
        let s = build_safe_skill_args_summary(&args, 160);
        assert!(s.contains("action=open_orders"));
        assert!(s.contains("exchange=binance"));
        assert!(s.contains("symbol=DOGEUSDT"));
        assert!(s.starts_with("action="));
    }

    #[test]
    fn test_build_safe_skill_args_summary_sensitive_omitted() {
        let args = json!({
            "action": "trade_submit",
            "symbol": "DOGEUSDT",
            "api_key": "secret123",
            "api_secret": "never-show"
        });
        let s = build_safe_skill_args_summary(&args, 160);
        assert!(!s.contains("api_key"));
        assert!(!s.contains("api_secret"));
        assert!(!s.contains("secret123"));
        assert!(s.contains("action=trade_submit"));
        assert!(s.contains("symbol=DOGEUSDT"));
    }

    #[test]
    fn test_build_safe_skill_args_summary_truncate() {
        let args = json!({
            "action": "trade_history",
            "symbol": "DOGEUSDT",
            "limit": 10
        });
        let s = build_safe_skill_args_summary(&args, 30);
        assert!(s.len() <= 33);
        assert!(s.ends_with("...") || s.len() <= 30);
    }

    #[test]
    fn test_build_safe_skill_args_summary_empty_object() {
        let args = json!({});
        let s = build_safe_skill_args_summary(&args, 160);
        assert!(s.is_empty());
    }

    #[test]
    fn register_step_output_indexes_inventory_names_for_followup_paths() {
        let mut loop_state = LoopState::new(1);
        register_step_output(
            &mut loop_state,
            1,
            1,
            "step_1",
            r#"{"action":"inventory_dir","names":["act_plan.log","clawd.log","clawd.run.log"],"path":"logs"}"#,
        );

        assert_eq!(
            loop_state
                .output_vars
                .get("last_output.0")
                .map(String::as_str),
            Some("act_plan.log")
        );
        assert_eq!(
            loop_state
                .output_vars
                .get("last_output.1")
                .map(String::as_str),
            Some("clawd.log")
        );
        assert_eq!(
            loop_state.output_vars.get("s1.names.2").map(String::as_str),
            Some("clawd.run.log")
        );
        assert_eq!(
            loop_state.output_vars.get("step_1[2]").map(String::as_str),
            Some("clawd.run.log")
        );
    }

    // --- build_final_delivery_with_priority: last_respond has priority over delivery_messages ---
    #[test]
    fn test_final_delivery_last_respond_priority_when_different() {
        let delivery = vec!["early answer".to_string()];
        let last_respond = "final answer".to_string();
        let (deduped, final_text, used) =
            crate::finalize::build_final_delivery_with_priority(&delivery, Some(&last_respond));
        assert!(used);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0], "early answer");
        assert_eq!(deduped[1], "final answer");
        assert_eq!(final_text, "final answer");
    }

    #[test]
    fn test_final_delivery_last_respond_same_as_delivery_no_duplicate() {
        let delivery = vec!["same text".to_string()];
        let last_respond = "same text".to_string();
        let (deduped, final_text, used) =
            crate::finalize::build_final_delivery_with_priority(&delivery, Some(&last_respond));
        assert!(used);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0], "same text");
        assert_eq!(final_text, "same text");
    }

    #[test]
    fn test_final_delivery_no_last_respond_uses_delivery() {
        let delivery = vec!["only delivery".to_string()];
        let (deduped, final_text, used) =
            crate::finalize::build_final_delivery_with_priority(&delivery, None);
        assert!(!used);
        assert_eq!(deduped.len(), 1);
        assert_eq!(final_text, "only delivery");
    }

    #[test]
    fn test_final_delivery_both_empty() {
        let delivery: Vec<String> = vec![];
        let (deduped, final_text, used) =
            crate::finalize::build_final_delivery_with_priority(&delivery, None);
        assert!(!used);
        assert!(deduped.is_empty());
        assert!(final_text.is_empty());
    }

    #[test]
    fn test_final_delivery_strips_subtask_prefix_from_user_visible_messages() {
        let delivery = vec!["subtask#1 skill(run_cmd): success\ntestuser".to_string()];
        let (deduped, final_text, used) =
            crate::finalize::build_final_delivery_with_priority(&delivery, None);
        assert!(!used);
        assert_eq!(deduped, vec!["testuser".to_string()]);
        assert_eq!(final_text, "testuser");
    }

    #[test]
    fn test_normalize_user_visible_text_strips_inline_subtask_prefix() {
        assert_eq!(
            crate::finalize::normalize_user_visible_text(
                "subtask#1 skill(run_cmd): success testuser",
            ),
            "testuser"
        );
    }

    #[test]
    fn test_final_delivery_preserves_failed_message_body() {
        let delivery = vec!["subtask#1 skill(run_cmd): failed\npermission denied".to_string()];
        let (deduped, final_text, used) =
            crate::finalize::build_final_delivery_with_priority(&delivery, None);
        assert!(!used);
        assert_eq!(deduped, vec!["permission denied".to_string()]);
        assert_eq!(final_text, "permission denied");
    }

    #[test]
    fn test_normalize_user_visible_text_strips_inline_failed_prefix() {
        assert_eq!(
            crate::finalize::normalize_user_visible_text(
                "subtask#1 skill(run_cmd): failed permission denied"
            ),
            "permission denied"
        );
    }

    #[test]
    fn test_extract_latest_successful_read_file_output_prefers_content_body() {
        let mut loop_state = LoopState::default();
        loop_state
            .executed_step_results
            .push(crate::executor::StepExecutionResult {
                step_id: "subtask#2".to_string(),
                skill: "read_file".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output: Some("# Test Workspace\nThis directory is reserved.".to_string()),
                error: None,
                started_at: 0,
                finished_at: 0,
            });
        let observed =
            super::observed_output::extract_latest_successful_read_file_output(&loop_state);
        assert_eq!(
            observed.as_deref(),
            Some("# Test Workspace\nThis directory is reserved.")
        );
    }

    #[test]
    fn test_extract_latest_successful_list_dir_output_prefers_content_body() {
        let mut loop_state = LoopState::default();
        loop_state
            .executed_step_results
            .push(crate::executor::StepExecutionResult {
                step_id: "subtask#1".to_string(),
                skill: "list_dir".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output: Some("file1.txt\nsubdir/\nfile2.md".to_string()),
                error: None,
                started_at: 0,
                finished_at: 0,
            });
        let observed =
            super::observed_output::extract_latest_successful_list_dir_output(&loop_state);
        assert_eq!(observed.as_deref(), Some("file1.txt\nsubdir/\nfile2.md"));
    }

    #[test]
    fn test_extract_latest_generic_successful_output_prefers_non_read_non_list_skill() {
        let mut loop_state = LoopState::default();
        loop_state
            .executed_step_results
            .push(crate::executor::StepExecutionResult {
                step_id: "subtask#1".to_string(),
                skill: "read_file".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output: Some("hello".to_string()),
                error: None,
                started_at: 0,
                finished_at: 0,
            });
        loop_state
            .executed_step_results
            .push(crate::executor::StepExecutionResult {
                step_id: "subtask#2".to_string(),
                skill: "run_cmd".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output: Some("testuser".to_string()),
                error: None,
                started_at: 0,
                finished_at: 0,
            });
        let observed =
            super::observed_output::extract_latest_generic_successful_output(&loop_state)
                .expect("generic observed output");
        assert!(observed.action_label.contains("skill(run_cmd): success"));
        assert_eq!(observed.body, "testuser");
    }

    #[test]
    fn test_extract_latest_generic_successful_output_skips_non_content() {
        let mut loop_state = LoopState::default();
        loop_state
            .executed_step_results
            .push(crate::executor::StepExecutionResult {
                step_id: "subtask#1".to_string(),
                skill: "chat".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output: Some("FILE:/tmp/demo.txt".to_string()),
                error: None,
                started_at: 0,
                finished_at: 0,
            });
        assert!(
            super::observed_output::extract_latest_generic_successful_output(&loop_state).is_none()
        );
    }

    #[test]
    fn test_normalized_observed_listing_trims_blank_lines() {
        let observed = "\n file1.txt \n\n subdir/ \n";
        let out = super::observed_output::normalized_observed_listing(observed);
        assert_eq!(out.as_deref(), Some("file1.txt\nsubdir/"));
    }

    #[test]
    fn test_finalizer_schema_answer_parse_ok() {
        let raw = r#"{"answer":"hello","completion_ok":true,"grounded_ok":true,"format_ok":true,"needs_clarify":false,"confidence":0.9,"used_evidence_ids":["E1"]}"#;
        let (answer, schema) = crate::finalize::finalizer_schema_answer(raw).expect("schema parse");
        assert_eq!(answer, "hello");
        assert!(crate::finalize::finalizer_contract_ok(&schema));
    }

    #[test]
    fn test_finalizer_schema_answer_parse_fail_non_json() {
        assert!(crate::finalize::finalizer_schema_answer("plain text").is_none());
    }

    #[test]
    fn test_finalizer_contract_not_ok_when_grounding_false() {
        let raw = r#"{"answer":"hello","completion_ok":true,"grounded_ok":false,"format_ok":true}"#;
        let (_answer, schema) =
            crate::finalize::finalizer_schema_answer(raw).expect("schema parse");
        assert!(!crate::finalize::finalizer_contract_ok(&schema));
        assert!(matches!(
            crate::finalize::finalizer_contract_disposition(&schema),
            crate::finalize::FinalizerDisposition::MustFail
        ));
    }

    #[test]
    fn test_finalizer_contract_disposition_allows_fallback_on_format_only_failure() {
        let raw = r#"{"answer":"hello","completion_ok":true,"grounded_ok":true,"format_ok":false}"#;
        let (_answer, schema) =
            crate::finalize::finalizer_schema_answer(raw).expect("schema parse");
        assert!(matches!(
            crate::finalize::finalizer_contract_disposition(&schema),
            crate::finalize::FinalizerDisposition::AllowFallback
        ));
    }

    #[test]
    fn test_finalizer_contract_disposition_must_fail_on_needs_clarify() {
        let raw = r#"{"answer":"need info","completion_ok":false,"grounded_ok":true,"format_ok":true,"needs_clarify":true}"#;
        let (_answer, schema) =
            crate::finalize::finalizer_schema_answer(raw).expect("schema parse");
        assert!(matches!(
            crate::finalize::finalizer_contract_disposition(&schema),
            crate::finalize::FinalizerDisposition::MustFail
        ));
    }

    #[test]
    fn test_internal_trace_artifact_detected() {
        assert!(crate::finalize::looks_like_internal_trace_artifact(
            "subtask#1 skill(run_cmd): success"
        ));
    }

    #[test]
    fn test_structured_blob_detected() {
        assert!(crate::finalize::looks_like_structured_blob(
            "{\"answer\":\"x\"}"
        ));
        assert!(crate::finalize::looks_like_structured_blob("[1,2,3]"));
        assert!(!crate::finalize::looks_like_structured_blob("plain text"));
    }

    #[test]
    fn test_infer_file_target_kind_classifies_extension_backed_files() {
        assert_eq!(
            crate::finalize::infer_file_target_kind("/tmp/app.log"),
            crate::finalize::FileTargetKind::LogFile
        );
        assert_eq!(
            crate::finalize::infer_file_target_kind("/tmp/data.json"),
            crate::finalize::FileTargetKind::JsonFile
        );
        assert_eq!(
            crate::finalize::infer_file_target_kind("/tmp/archive.tar.gz"),
            crate::finalize::FileTargetKind::ArchiveFile
        );
    }

    #[test]
    fn test_infer_file_target_kind_distinguishes_directory_from_plain_file() {
        assert_eq!(
            crate::finalize::infer_file_target_kind("/tmp/output"),
            crate::finalize::FileTargetKind::Directory
        );
        assert_eq!(
            crate::finalize::infer_file_target_kind("/tmp/output.txt"),
            crate::finalize::FileTargetKind::File
        );
    }

    #[test]
    fn test_parse_delivery_token_normalizes_supported_prefixes() {
        let (kind, payload) =
            crate::finalize::parse_delivery_token(" IMAGE_FILE:/tmp/demo.png ").expect("token");
        assert_eq!(kind, crate::finalize::DeliveryTokenKind::ImageFile);
        assert_eq!(payload.trim(), "/tmp/demo.png");
        assert_eq!(kind.canonical_prefix(), "FILE:");

        let (kind, payload) =
            crate::finalize::parse_delivery_token("MEDIA_URL:https://example.com/a.mp4")
                .expect("token");
        assert_eq!(kind, crate::finalize::DeliveryTokenKind::MediaUrl);
        assert_eq!(payload.trim(), "https://example.com/a.mp4");
    }

    #[test]
    fn test_classify_planner_artifact_detects_tool_call_and_action_json() {
        assert!(matches!(
            crate::finalize::classify_planner_artifact("[TOOL_CALL]run_cmd[/TOOL_CALL]"),
            Some(crate::finalize::PlannerArtifactKind::ToolCallTag)
        ));
        assert!(matches!(
            crate::finalize::classify_planner_artifact(
                r#"{"type":"call_tool","tool":"read_file"}"#
            ),
            Some(
                crate::finalize::PlannerArtifactKind::ActionObject
                    | crate::finalize::PlannerArtifactKind::PlannerObject
            )
        ));
    }

    #[test]
    fn test_user_safe_step_error_preserves_sanitized_error_excerpt() {
        assert_eq!(
            user_safe_step_error(
                "synthesize_answer could not produce a grounded publishable answer",
                false,
            ),
            "synthesize_answer could not produce a grounded publishable answer"
        );
        assert_eq!(
            user_safe_step_error("unknown action: read; allowed: info|inventory_dir", true),
            "unknown action: read; allowed: info|inventory_dir"
        );
        assert_eq!(
            user_safe_step_error("", false),
            "执行失败，但没有返回明确原因"
        );
        assert_eq!(
            user_safe_step_error("  ", true),
            "Execution failed without a clear reason."
        );
    }

    #[test]
    fn test_extract_single_explicit_path_from_request_ok() {
        let text = "先读 /home/guagua/test/README.md 开头，再用一句话总结";
        assert_eq!(
            crate::finalize::extract_single_explicit_path_from_request(text).as_deref(),
            Some("/home/guagua/test/README.md")
        );
    }

    #[test]
    fn test_observed_quotes_grounded_requires_exact_match() {
        let observed =
            "# Test Workspace\nThis directory is reserved for NL regression test artifacts.";
        let schema = crate::finalize::FinalizerSchemaOut {
            answer: "summary".to_string(),
            completion_ok: true,
            grounded_ok: true,
            format_ok: true,
            needs_clarify: false,
            confidence: 0.8,
            used_evidence_ids: vec!["E1".to_string()],
            evidence_quotes: vec!["NL regression test artifacts".to_string()],
        };
        assert!(crate::finalize::observed_quotes_grounded(&schema, observed));

        let bad = crate::finalize::FinalizerSchemaOut {
            evidence_quotes: vec!["high-performance distributed scheduler".to_string()],
            ..schema
        };
        assert!(!crate::finalize::observed_quotes_grounded(&bad, observed));
    }

    #[test]
    fn test_observed_read_path_matches_request() {
        let ws = Path::new("/tmp/workspace");
        let user_text = "Read /home/guagua/test/README.md and summarize.";
        assert!(crate::finalize::observed_read_path_matches_request(
            ws,
            user_text,
            Some("/home/guagua/test/README.md")
        ));
        assert!(!crate::finalize::observed_read_path_matches_request(
            ws,
            user_text,
            Some("/home/guagua/git_upload/README.md")
        ));
    }
}
