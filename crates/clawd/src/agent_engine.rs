use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use tracing::{debug, info, warn};

mod arg_resolver;
mod attempt_ledger;
mod dispatch_support;
mod execution_loop;
pub(crate) mod loop_control;
pub(crate) mod migration_class;
pub(crate) mod observed_output;
mod planning;
mod planning_actions;
mod planning_followup;
mod planning_numeric_limits;
mod planning_parse;
mod planning_path_metadata;
mod planning_prompt;
mod planning_recent_artifacts;
mod planning_registry_preference;
mod planning_route_markers;
mod planning_structured_field_exact;
mod prepare_round;
mod skill_execution;
mod support;

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
use self::execution_loop::execute_actions_once;
use self::loop_control::run_agent_with_loop;
use self::prepare_round::{prepare_round_actions, push_round_trace};

use self::skill_execution::execute_prepared_skill_action;
pub(crate) use self::support::append_delivery_message;
use self::support::{
    action_fingerprint_for_policy, append_progress_hint, build_safe_skill_args_summary,
    encode_progress_i18n, load_agent_loop_guard_policy, maybe_publish_execution_recipe_phase_hint,
    registry_idempotency_guard_attribution, AgentLoopGuardPolicy, PROGRESS_ARGS_SUMMARY_MAX_LEN,
};

use crate::{repo, AgentAction, AppState, AskReply, ClaimedTask};

pub(crate) fn answer_verifier_enforce_required_enabled(state: &AppState) -> bool {
    load_agent_loop_guard_policy(state).answer_verifier_enforce_required
}

pub(crate) fn agent_loop_semantic_authority_enabled(state: &AppState) -> bool {
    load_agent_loop_guard_policy(state).uses_agent_loop_semantic_authority()
}

pub(crate) fn agent_loop_authority_selected_migration_class(
    state: &AppState,
    route: &crate::RouteResult,
) -> Option<&'static str> {
    let policy = load_agent_loop_guard_policy(state);
    agent_loop_authority_selected_migration_class_for_policy(&policy, route)
}

pub(in crate::agent_engine) fn agent_loop_authority_selected_migration_class_for_policy(
    policy: &AgentLoopGuardPolicy,
    route: &crate::RouteResult,
) -> Option<&'static str> {
    if !policy.uses_agent_loop_semantic_authority()
        || route.risk_ceiling == crate::RiskCeiling::High
        || route.schedule_kind != crate::ScheduleKind::None
    {
        return None;
    }
    let eligible = migration_class::agent_decides_eligible_migration_class(route);
    let selected = policy.selected_migration_class_for_eligible(eligible);
    if selected != "none" {
        Some(selected)
    } else {
        None
    }
}

pub(crate) fn agent_decides_shadow_snapshot_for_route(
    state: &AppState,
    task: &ClaimedTask,
    agent_run_context: Option<&AgentRunContext>,
    route: &crate::RouteResult,
) -> Option<crate::task_journal::TaskJournalRolloutAttribution> {
    let policy = load_agent_loop_guard_policy(state);
    if !policy.records_agent_decides_attribution() {
        return None;
    }
    let budget_profile = AgentLoopGuardPolicy::budget_profile_for_context(
        crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        Some(route),
    );
    Some(
        crate::task_journal::TaskJournalRolloutAttribution::agent_decides_shadow_snapshot(
            route,
            budget_profile.as_str(),
            Some(loop_control::boundary_context_snapshot_json(
                task,
                &policy,
                agent_run_context,
                Some(route),
                budget_profile,
            )),
        ),
    )
}

const AGENT_TOOL_SPEC_PATH: &str = "prompts/agent_tool_spec.md";
const CLAWD_CONTINUE_ON_ERROR_ARG: &str = "_clawd_continue_on_error";
const CLAWD_LITERAL_COMMAND_ARG: &str = "_clawd_literal_command";
const CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG: &str = "_clawd_literal_failure_repairable";
const CLAWD_MISSING_TARGET_REPAIRABLE_ARG: &str = "_clawd_missing_target_repairable";
const CLAWD_USER_NAMED_OUTPUT_PATH_ARG: &str = "_clawd_user_named_output_path";
const SINGLE_PLAN_EXECUTION_PROMPT_LOGICAL_PATH: &str = "prompts/single_plan_execution_prompt.md";
const LIGHTWEIGHT_EXECUTION_PROMPT_LOGICAL_PATH: &str = "prompts/lightweight_execution_prompt.md";
const LOOP_INCREMENTAL_PLAN_PROMPT_LOGICAL_PATH: &str = "prompts/loop_incremental_plan_prompt.md";
const PLAN_REPAIR_PROMPT_LOGICAL_PATH: &str = "prompts/plan_repair_prompt.md";
pub(crate) const TASK_CANCELED_ERR: &str = "__TASK_CANCELED_BY_USER__";

fn structured_write_path_arg(normalized_skill: &str, args: &Value) -> Option<String> {
    let obj = args.as_object()?;
    match normalized_skill {
        "write_file" => obj.get("path").and_then(Value::as_str),
        "fs_basic" => {
            let action = obj.get("action").and_then(Value::as_str)?.trim();
            if matches!(action, "write_text" | "append_text") {
                obj.get("path").and_then(Value::as_str)
            } else {
                None
            }
        }
        _ => None,
    }
    .map(str::trim)
    .filter(|path| !path.is_empty())
    .map(ToString::to_string)
}

fn structured_write_has_content_arg(args: &Value) -> bool {
    let Some(obj) = args.as_object() else {
        return false;
    };
    ["content", "text", "data", "body"].iter().any(|key| {
        obj.get(*key)
            .and_then(Value::as_str)
            .is_some_and(|v| !v.is_empty())
    })
}

fn resolve_workspace_candidate_path(
    workspace_root: &Path,
    raw_path: &str,
) -> Option<std::path::PathBuf> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let raw = Path::new(trimmed);
    if raw
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return None;
    }
    let root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let candidate = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        root.join(raw)
    };
    candidate.starts_with(&root).then_some(candidate)
}

fn request_surface_names_user_output_path(request_text: &str, path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let file_name = file_name.trim();
    if file_name.is_empty() {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request_text);
    surface
        .filename_candidates_excluding_field_selectors()
        .into_iter()
        .any(|candidate| {
            let trimmed = candidate.trim();
            let candidate_file_name = Path::new(trimmed)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(trimmed)
                .trim();
            !candidate_file_name.is_empty() && candidate_file_name.eq_ignore_ascii_case(file_name)
        })
}

pub(crate) fn action_is_user_named_new_workspace_write(
    workspace_root: &Path,
    request_text: &str,
    normalized_skill: &str,
    args: &Value,
) -> bool {
    if !structured_write_has_content_arg(args) {
        return false;
    }
    let Some(raw_path) = structured_write_path_arg(normalized_skill, args) else {
        return false;
    };
    let Some(candidate) = resolve_workspace_candidate_path(workspace_root, &raw_path) else {
        return false;
    };
    !candidate.exists() && request_surface_names_user_output_path(request_text, &candidate)
}

pub(crate) fn action_has_user_named_output_path_marker(args: &Value) -> bool {
    args.get(CLAWD_USER_NAMED_OUTPUT_PATH_ARG)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

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

fn quick_index_planner_capabilities(manifest: &claw_core::skill_registry::SkillManifest) -> String {
    let tokens = manifest
        .planner_capabilities
        .iter()
        .take(6)
        .map(|capability| {
            let name = capability.name.trim();
            let mut attrs = Vec::new();
            if let Some(action) = capability.action.as_deref() {
                if !action.trim().is_empty() {
                    attrs.push(format!("action={}", action.trim()));
                }
            }
            if let Some(effect) = capability.effect {
                attrs.push(format!("effect={}", effect.as_token()));
            }
            if !capability.required.is_empty() {
                attrs.push(format!("required={}", capability.required.join("|")));
            }
            if attrs.is_empty() {
                name.to_string()
            } else {
                format!("{name}({})", attrs.join(","))
            }
        })
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        String::new()
    } else {
        format!("; planner_capabilities: {}", tokens.join("; "))
    }
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
                "- {skill}: {summary}; planner_kind: {}{}",
                manifest.planner_kind.as_token(),
                quick_index_planner_capabilities(&manifest)
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
        if let Some(contract_matrix_line) =
            crate::contract_matrix::compact_prompt_line_for_route(route)
        {
            lines.push(contract_matrix_line);
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
    pub(crate) session_alias_bindings: Vec<crate::conversation_state::SessionAliasBinding>,
    pub(crate) auto_locator_path: Option<String>,
    pub(crate) has_authoritative_deictic_anchor: bool,
    pub(crate) fuzzy_locator_suggestions: Vec<String>,
    pub(crate) original_user_request: Option<String>,
    pub(crate) user_request: Option<String>,
    pub(crate) semantic_answer_candidate_draft: Option<String>,
    /// Memory context rendered before the current request is appended.
    /// This is used only for runtime-bound scalar checks; it must not include
    /// model-produced answer candidates from the current turn.
    pub(crate) memory_context_for_execution: Option<String>,
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
#[path = "agent_engine_tests.rs"]
mod tests;
