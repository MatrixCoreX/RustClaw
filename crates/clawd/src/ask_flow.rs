use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

use crate::{ActFinalizeStyle, AppState, AskReply, ClaimedTask};

const DIRECT_ANSWER_GATE_PROMPT_LOGICAL_PATH: &str = "prompts/direct_answer_gate_prompt.md";
const RUSTCLAW_MAIN_CONFIG_LOGICAL_PATH: &str = "configs/config.toml";
const VOICE_CHAT_PROMPT_LOGICAL_PATH: &str = "prompts/voice_chat_prompt.md";
const DEFAULT_VOICE_CHAT_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/layers/overlays/voice_chat_prompt.md");

#[derive(Debug, Clone, Deserialize)]
struct DirectAnswerGateOut {
    #[serde(default)]
    decision: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    confidence: f64,
    #[serde(default)]
    clarify_question: String,
    #[serde(default)]
    resolved_user_intent: String,
    #[serde(default)]
    reference_resolution: DirectAnswerGateReferenceResolutionOut,
    output_contract: DirectAnswerGateContractOut,
    #[serde(default)]
    state_patch: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct DirectAnswerGateContractOut {
    #[serde(default)]
    response_shape: String,
    #[serde(default)]
    exact_sentence_count: Option<usize>,
    #[serde(default)]
    requires_content_evidence: bool,
    #[serde(default)]
    delivery_required: bool,
    #[serde(default)]
    locator_kind: String,
    #[serde(default)]
    delivery_intent: String,
    #[serde(default)]
    semantic_kind: String,
    #[serde(default)]
    locator_hint: String,
    #[serde(default)]
    self_extension: DirectAnswerGateSelfExtensionOut,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct DirectAnswerGateSelfExtensionOut {
    #[serde(default)]
    mode: String,
    #[serde(default)]
    trigger: String,
    #[serde(default)]
    execute_now: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct DirectAnswerGateReferenceResolutionOut {
    #[serde(default)]
    target: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ActiveTaskFactualRewriteReview {
    #[serde(default)]
    pass: bool,
    #[serde(default)]
    unsupported_claims: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectAnswerGateDecision {
    DirectAnswer,
    PlannerExecute,
    Clarify,
}

enum DirectAnswerPreflight {
    DirectAnswer,
    PlannerExecute(crate::agent_engine::AgentRunContext, &'static str),
    Clarify(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecentCountObservation {
    target_label: String,
    total: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecentCountComparisonDirection {
    More,
    Less,
}

#[path = "ask_flow_resume.rs"]
mod resume;
use resume::*;

#[path = "ask_flow_active_context.rs"]
mod active_context;
pub(crate) use active_context::active_ordered_entries_count_direct_answer_candidate;
use active_context::*;

#[path = "ask_flow_recent_count.rs"]
mod recent_count;
use recent_count::*;

#[path = "ask_flow_gate_contract.rs"]
mod gate_contract;
use gate_contract::*;

#[path = "ask_flow_gate_policy.rs"]
mod gate_policy;
pub(crate) use gate_policy::route_allows_agent_loop_pure_chat_submode;
use gate_policy::*;

#[path = "ask_flow_gate_execution.rs"]
mod gate_execution;
use gate_execution::*;

#[path = "ask_flow_chat_helpers.rs"]
mod chat_helpers;
#[cfg(test)]
use chat_helpers::ActiveTaskReplacementPair;
use chat_helpers::*;
pub(crate) use chat_helpers::{
    build_resume_continue_execute_prompt, build_resume_continue_execute_prompt_from_context,
    build_resume_followup_discussion_prompt, build_resume_followup_discussion_prompt_from_context,
};

#[path = "ask_flow_pre_planner_exit.rs"]
mod pre_planner_exit;
use pre_planner_exit::*;

fn task_payload_text(task: &ClaimedTask) -> Option<String> {
    crate::task_payload_value(task)?
        .get("text")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

async fn execute_via_planner_loop(
    state: &AppState,
    task: &ClaimedTask,
    prompt_with_memory: &str,
    execution_user_request: &str,
    ask_mode: &crate::AskMode,
    agent_run_context: Option<crate::agent_engine::AgentRunContext>,
) -> Result<AskReply, String> {
    let planner_goal = if ask_mode.finalize_chat_wrapped() {
        chat_wrapped_execution_goal_from_prompt(prompt_with_memory)
    } else {
        prompt_with_memory.to_string()
    };
    crate::agent_engine::run_agent_with_tools(
        state,
        task,
        &planner_goal,
        execution_user_request,
        agent_run_context,
    )
    .await
}

fn direct_answer_gate_promoted_prompt_with_memory(
    prompt_with_memory: &str,
    resolved_intent: &str,
) -> String {
    let prompt_with_memory = prompt_with_memory.trim();
    let resolved_intent = resolved_intent.trim();
    if resolved_intent.is_empty() || resolved_intent == prompt_with_memory {
        return prompt_with_memory.to_string();
    }
    let gate_context = json!({
        "direct_answer_gate": {
            "resolved_intent": resolved_intent,
        }
    });
    format!("{}\n\n{}", prompt_with_memory, gate_context)
}

pub(crate) async fn execute_ask_routed(
    state: &AppState,
    task: &ClaimedTask,
    chat_prompt_context: &str,
    prompt_with_memory: &str,
    resolved_prompt: &str,
    execution_user_request: &str,
    agent_mode: bool,
    resume_force_chat: bool,
    route_ask_mode: Option<crate::AskMode>,
    agent_run_context: Option<crate::agent_engine::AgentRunContext>,
) -> Result<AskReply, String> {
    // Callers pass the first-layer AskMode directly. If it is missing, choose a
    // conservative local fallback instead of starting another routing LLM round.
    let route_ask_mode_for_log = route_ask_mode.clone();
    let (ask_mode, override_reason) = if resume_force_chat {
        (crate::AskMode::direct_answer(), Some("resume_force_chat"))
    } else if let Some(mode) = route_ask_mode {
        (mode, None)
    } else if agent_mode {
        (
            crate::AskMode::clarify(),
            Some("route_ask_mode=None and agent_mode=true"),
        )
    } else {
        (
            crate::AskMode::direct_answer(),
            Some("route_ask_mode=None and agent_mode=false"),
        )
    };
    let legacy_route_label = ask_mode.legacy_route_label_for_trace();
    tracing::info!(
        "{} worker_once: ask task_id={} route_gate_kind={} ask_mode={} legacy_route_label={} agent_mode={} override={}",
        crate::highlight_tag("routing"),
        task.task_id,
        ask_mode.gate_kind().as_str(),
        route_ask_mode_for_log
            .as_ref()
            .map(crate::AskMode::as_str)
            .unwrap_or("none"),
        legacy_route_label,
        agent_mode,
        override_reason.unwrap_or("")
    );
    let mut agent_run_context = agent_run_context;
    let mut effective_ask_mode = ask_mode.clone();
    let current_turn_user_request_for_process =
        task_payload_text(task).unwrap_or_else(|| execution_user_request.to_string());
    if effective_ask_mode.is_clarify_only()
        && promote_clarify_recent_execution_judgment_context_to_chat(
            state,
            agent_run_context.as_mut(),
        )
    {
        effective_ask_mode = crate::AskMode::direct_answer();
        tracing::info!(
            "{} worker_once: ask clarify_recent_execution_judgment_to_chat task_id={}",
            crate::highlight_tag("routing"),
            task.task_id
        );
    }
    if effective_ask_mode.is_clarify_only()
        && promote_clarify_config_risk_assessment_default_config_to_planner(
            state,
            &current_turn_user_request_for_process,
            agent_run_context.as_mut(),
        )
    {
        effective_ask_mode = crate::AskMode::planner_execute_chat_wrapped();
        tracing::info!(
            "{} worker_once: ask config_risk_default_main_config_to_planner task_id={}",
            crate::highlight_tag("routing"),
            task.task_id
        );
    }
    if effective_ask_mode.act_finalize_style().is_some()
        && promote_active_anchor_observed_judgment_to_chat(
            &current_turn_user_request_for_process,
            agent_run_context.as_mut(),
        )
    {
        effective_ask_mode = crate::AskMode::direct_answer();
        tracing::info!(
            "{} worker_once: ask active_anchor_observed_judgment_to_chat task_id={}",
            crate::highlight_tag("routing"),
            task.task_id
        );
    }
    if let Some(reply) = crate::self_extension::maybe_handle_ask_self_extension(
        state,
        task,
        resolved_prompt,
        execution_user_request,
        agent_run_context.as_ref(),
    )
    .await?
    {
        return Ok(with_pre_planner_exit_snapshot(
            state,
            task,
            &current_turn_user_request_for_process,
            reply,
            agent_run_context.as_ref(),
            "self_extension_boundary",
        ));
    }
    let process_language_hint = crate::language_policy::task_response_language_hint(
        state,
        task,
        &current_turn_user_request_for_process,
    );
    if let Some(reply) = structural_alias_binding_ack(
        state,
        agent_run_context.as_ref(),
        &current_turn_user_request_for_process,
        execution_user_request,
        &process_language_hint,
    ) {
        tracing::info!(
            "{} worker_once: ask structural_alias_binding_ack task_id={}",
            crate::highlight_tag("routing"),
            task.task_id
        );
        return Ok(with_pre_planner_exit_snapshot(
            state,
            task,
            &current_turn_user_request_for_process,
            reply,
            agent_run_context.as_ref(),
            "structural_alias_binding_ack",
        ));
    }
    if let Some(candidate) = active_ordered_entries_count_direct_answer_candidate(
        &current_turn_user_request_for_process,
        agent_run_context.as_ref(),
    ) {
        tracing::info!(
            "{} worker_once: ask active_ordered_entries_count_direct_answer task_id={} answer={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(&candidate)
        );
        return Ok(with_pre_planner_exit_snapshot(
            state,
            task,
            &current_turn_user_request_for_process,
            ask_reply_with_chat_process(candidate, &process_language_hint),
            agent_run_context.as_ref(),
            "active_ordered_entries_count_direct_answer",
        ));
    }
    if let Some(candidate) = recent_count_comparison_direct_answer(
        state,
        task,
        &current_turn_user_request_for_process,
        agent_run_context.as_ref(),
    ) {
        tracing::info!(
            "{} worker_once: ask recent_count_comparison_direct_answer task_id={} answer={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(&candidate)
        );
        return Ok(with_pre_planner_exit_snapshot(
            state,
            task,
            &current_turn_user_request_for_process,
            ask_reply_with_chat_process(candidate, &process_language_hint),
            agent_run_context.as_ref(),
            "recent_count_comparison_direct_answer",
        ));
    }
    if let Some(candidate) = runtime_approval_wait_status_direct_answer_candidate(
        state,
        agent_run_context.as_ref(),
        &process_language_hint,
    ) {
        tracing::info!(
            "{} worker_once: ask runtime_approval_wait_status_direct_answer task_id={} answer={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(&candidate)
        );
        return Ok(with_pre_planner_exit_snapshot(
            state,
            task,
            &current_turn_user_request_for_process,
            ask_reply_with_chat_process(candidate, &process_language_hint),
            agent_run_context.as_ref(),
            "runtime_approval_wait_status_direct_answer",
        ));
    }
    if let Some(candidate) = session_alias_target_direct_answer_candidate(
        state,
        task,
        &current_turn_user_request_for_process,
        agent_run_context.as_ref(),
    ) {
        tracing::info!(
            "{} worker_once: ask session_alias_target_direct_answer task_id={} answer={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(&candidate)
        );
        return Ok(with_pre_planner_exit_snapshot(
            state,
            task,
            &current_turn_user_request_for_process,
            ask_reply_with_chat_process(candidate, &process_language_hint),
            agent_run_context.as_ref(),
            "session_alias_target_direct_answer",
        ));
    }
    if let Some(candidate) = active_file_basename_direct_answer(state, agent_run_context.as_ref()) {
        tracing::info!(
            "{} worker_once: ask active_file_basename_direct_answer task_id={} answer={}",
            crate::highlight_tag("routing"),
            task.task_id,
            candidate.answer
        );
        let mut reply = with_pre_planner_exit_snapshot(
            state,
            task,
            &current_turn_user_request_for_process,
            ask_reply_with_chat_process(candidate.answer.clone(), &process_language_hint),
            agent_run_context.as_ref(),
            "active_file_basename_direct_answer",
        );
        let journal = reply.task_journal.get_or_insert_with(|| {
            crate::task_journal::TaskJournal::for_task(
                &task.task_id,
                "ask",
                &current_turn_user_request_for_process,
            )
        });
        journal.push_task_observation(candidate.observed_evidence());
        journal.record_used_evidence_ids_count(1);
        return Ok(reply);
    }
    if let Some(candidate) =
        runtime_scalar_path_direct_answer_candidate(state, agent_run_context.as_ref())
    {
        tracing::info!(
            "{} worker_once: ask runtime_scalar_path_direct_answer task_id={} len={}",
            crate::highlight_tag("routing"),
            task.task_id,
            candidate.len()
        );
        return Ok(with_pre_planner_exit_snapshot(
            state,
            task,
            &current_turn_user_request_for_process,
            ask_reply_with_chat_process(candidate, &process_language_hint),
            agent_run_context.as_ref(),
            "runtime_scalar_path_direct_answer",
        ));
    }
    let current_turn_user_request =
        task_payload_text(task).unwrap_or_else(|| execution_user_request.to_string());
    match effective_ask_mode {
        crate::AskMode::ClarifyOrChat {
            entry:
                crate::ChatEntryStrategy::NormalizerThenChat
                | crate::ChatEntryStrategy::ResumeFollowupDiscussion,
        } => {
            let chat_prompt_context = chat_prompt_context_with_route_resolution(
                chat_prompt_context,
                agent_run_context.as_ref(),
            );
            let resolved_chat_prompt =
                crate::bootstrap::load_required_prompt_template_for_state_with_meta(
                    state,
                    crate::CHAT_RESPONSE_PROMPT_LOGICAL_PATH,
                )
                .map_err(|e| e.to_string())?;
            let chat_prompt_template = resolved_chat_prompt.template;
            let chat_prompt_source = resolved_chat_prompt.source;
            let chat_prompt_version = resolved_chat_prompt.version;
            crate::log_prompt_render_with_version(
                state,
                &task.task_id,
                "chat_response_prompt",
                &chat_prompt_source,
                chat_prompt_version.as_deref(),
                None,
            );
            let task_persona_prompt = state.task_persona_prompt(task);
            let chat_user_request = chat_user_request(resolved_prompt, execution_user_request);
            let request_language_hint = crate::language_policy::task_response_language_hint(
                state,
                task,
                &current_turn_user_request,
            );
            if let Some(reply) = structural_alias_binding_ack(
                state,
                agent_run_context.as_ref(),
                &current_turn_user_request,
                execution_user_request,
                &request_language_hint,
            ) {
                tracing::info!(
                    "{} worker_once: ask structural_alias_binding_ack task_id={}",
                    crate::highlight_tag("routing"),
                    task.task_id
                );
                return Ok(with_pre_planner_exit_snapshot(
                    state,
                    task,
                    &current_turn_user_request,
                    reply,
                    agent_run_context.as_ref(),
                    "structural_alias_binding_ack",
                ));
            }
            if contract_test_hint_should_enter_planner_loop(
                &current_turn_user_request,
                agent_run_context.as_ref(),
            ) {
                tracing::info!(
                    "{} worker_once: ask contract_test_hint_promoted_to_planner task_id={}",
                    crate::highlight_tag("routing"),
                    task.task_id
                );
                let reply = execute_via_planner_loop(
                    state,
                    task,
                    prompt_with_memory,
                    execution_user_request,
                    &crate::AskMode::planner_execute_chat_wrapped(),
                    agent_run_context.clone(),
                )
                .await?;
                return Ok(with_pre_planner_exit_snapshot(
                    state,
                    task,
                    &current_turn_user_request,
                    reply,
                    agent_run_context.as_ref(),
                    "contract_test_hint_promoted_to_planner",
                ));
            }
            if crate::agent_engine::agent_loop_semantic_authority_enabled(state)
                && agent_run_context
                    .as_ref()
                    .and_then(|ctx| ctx.route_result.as_ref())
                    .is_some_and(route_allows_agent_loop_pure_chat_submode)
            {
                let mut loop_ctx = agent_run_context.clone();
                if let Some(route) = loop_ctx.as_mut().and_then(|ctx| ctx.route_result.as_mut()) {
                    route.set_planner_execute_finalize(ActFinalizeStyle::ChatWrapped);
                    append_route_reason(route, "pure_chat_agent_loop_submode");
                }
                tracing::info!(
                    "{} worker_once: ask pure_chat_agent_loop_submode task_id={}",
                    crate::highlight_tag("routing"),
                    task.task_id
                );
                let reply = execute_via_planner_loop(
                    state,
                    task,
                    prompt_with_memory,
                    execution_user_request,
                    &crate::AskMode::planner_execute_chat_wrapped(),
                    loop_ctx.clone(),
                )
                .await?;
                return Ok(with_pre_planner_exit_snapshot(
                    state,
                    task,
                    &current_turn_user_request,
                    reply,
                    loop_ctx.as_ref(),
                    "pure_chat_agent_loop_submode",
                ));
            }
            let mut direct_answer_gate_approved = false;
            let skip_direct_answer_gate =
                direct_answer_gate_can_skip_for_self_contained_payload(
                    &current_turn_user_request,
                    agent_run_context
                        .as_ref()
                        .and_then(|ctx| ctx.route_result.as_ref()),
                ) || direct_answer_gate_can_skip_for_pure_chat_draft(
                    state,
                    &current_turn_user_request,
                    agent_run_context
                        .as_ref()
                        .and_then(|ctx| ctx.route_result.as_ref()),
                ) || direct_answer_gate_can_skip_for_boundary_clean_chat(
                    state,
                    &current_turn_user_request,
                    agent_run_context
                        .as_ref()
                        .and_then(|ctx| ctx.route_result.as_ref()),
                ) || direct_answer_gate_can_skip_for_standalone_freeform_repair(
                    agent_run_context
                        .as_ref()
                        .and_then(|ctx| ctx.route_result.as_ref()),
                ) || direct_answer_gate_can_skip_for_active_task_text_mutation(
                    &current_turn_user_request,
                    agent_run_context.as_ref(),
                ) || direct_answer_gate_can_skip_for_active_observed_output_chat_repair(
                    agent_run_context.as_ref(),
                ) || direct_answer_gate_can_skip_for_recent_execution_judgment_context(
                    agent_run_context.as_ref(),
                ) || direct_answer_gate_can_skip_for_recent_count_context(
                    state,
                    task,
                    agent_run_context.as_ref(),
                );
            if skip_direct_answer_gate {
                tracing::info!(
                    "{} worker_once: ask direct_answer_gate_skipped task_id={}",
                    crate::highlight_tag("routing"),
                    task.task_id
                );
            } else if let Some(mut gate_ctx) = agent_run_context.clone() {
                if let Some(gate) =
                    run_direct_answer_gate(state, task, &current_turn_user_request, Some(&gate_ctx))
                        .await
                {
                    match apply_direct_answer_gate_outcome(
                        state,
                        &mut gate_ctx,
                        &current_turn_user_request,
                        gate,
                    ) {
                        DirectAnswerPreflight::DirectAnswer => {
                            direct_answer_gate_approved = true;
                            if let Some(candidate) = recent_count_comparison_direct_answer(
                                state,
                                task,
                                &current_turn_user_request,
                                Some(&gate_ctx),
                            ) {
                                tracing::info!(
                                    "{} worker_once: ask direct_answer_gate_recent_count_comparison task_id={} answer={}",
                                    crate::highlight_tag("routing"),
                                    task.task_id,
                                    crate::truncate_for_log(&candidate)
                                );
                                return Ok(with_pre_planner_exit_snapshot(
                                    state,
                                    task,
                                    &current_turn_user_request,
                                    ask_reply_with_chat_process(candidate, &request_language_hint),
                                    Some(&gate_ctx),
                                    "direct_answer_gate_recent_count_comparison",
                                ));
                            }
                            if direct_answer_gate_direct_answer_should_enter_agent_loop(
                                state,
                                gate_ctx.route_result.as_ref(),
                            ) {
                                if let Some(route) = gate_ctx.route_result.as_mut() {
                                    route.set_planner_execute_finalize(
                                        ActFinalizeStyle::ChatWrapped,
                                    );
                                    append_route_reason(
                                        route,
                                        "direct_answer_gate_direct_answer_deferred_to_agent_loop",
                                    );
                                }
                                tracing::info!(
                                    "{} worker_once: ask direct_answer_gate_direct_answer_deferred_to_agent_loop task_id={}",
                                    crate::highlight_tag("routing"),
                                    task.task_id
                                );
                                let promoted_prompt_with_memory = gate_ctx
                                    .route_result
                                    .as_ref()
                                    .map(|route| route.resolved_intent.trim())
                                    .map(|intent| {
                                        direct_answer_gate_promoted_prompt_with_memory(
                                            prompt_with_memory,
                                            intent,
                                        )
                                    })
                                    .unwrap_or_else(|| prompt_with_memory.to_string());
                                let reply = execute_via_planner_loop(
                                    state,
                                    task,
                                    &promoted_prompt_with_memory,
                                    execution_user_request,
                                    &crate::AskMode::planner_execute_chat_wrapped(),
                                    Some(gate_ctx.clone()),
                                )
                                .await?;
                                return Ok(with_pre_planner_exit_snapshot(
                                    state,
                                    task,
                                    &current_turn_user_request,
                                    reply,
                                    Some(&gate_ctx),
                                    "direct_answer_gate_agent_loop_activation",
                                ));
                            }
                        }
                        DirectAnswerPreflight::Clarify(question) => {
                            tracing::info!(
                                "{} worker_once: ask direct_answer_gate_clarify task_id={}",
                                crate::highlight_tag("routing"),
                                task.task_id
                            );
                            let question = if question.trim().is_empty() {
                                let clarify_reason = gate_ctx
                                    .route_result
                                    .as_ref()
                                    .map(|route| route.route_reason.as_str())
                                    .unwrap_or("direct_answer_gate_requires_clarify");
                                let structured_clarify_context =
                                    route_structured_clarify_context(Some(&gate_ctx));
                                crate::finalize::render_clarify_question(
                                    state,
                                    task,
                                    crate::finalize::ClarifyRenderRequest {
                                        user_request: &current_turn_user_request,
                                        resolver_reason: clarify_reason,
                                        candidate_context: structured_clarify_context.as_deref(),
                                        preferred_question: None,
                                        policy: crate::intent_router::ClarifyQuestionPolicy::SafeFallback,
                                        fallback_source:
                                            crate::fallback::ClarifyFallbackSource::IntentUnresolved,
                                    },
                                )
                                .await
                            } else {
                                question
                            };
                            return Ok(with_pre_planner_exit_snapshot(
                                state,
                                task,
                                &current_turn_user_request,
                                ask_reply_with_clarify_process(
                                    task,
                                    &current_turn_user_request,
                                    question,
                                    gate_ctx.route_result.as_ref(),
                                ),
                                Some(&gate_ctx),
                                "direct_answer_gate_clarify",
                            ));
                        }
                        DirectAnswerPreflight::PlannerExecute(promoted_ctx, reason_code) => {
                            tracing::info!(
                                "{} worker_once: ask {} task_id={}",
                                crate::highlight_tag("routing"),
                                reason_code,
                                task.task_id
                            );
                            let promoted_prompt_with_memory = promoted_ctx
                                .route_result
                                .as_ref()
                                .map(|route| route.resolved_intent.trim())
                                .map(|intent| {
                                    direct_answer_gate_promoted_prompt_with_memory(
                                        prompt_with_memory,
                                        intent,
                                    )
                                })
                                .unwrap_or_else(|| prompt_with_memory.to_string());
                            let reply = execute_via_planner_loop(
                                state,
                                task,
                                &promoted_prompt_with_memory,
                                execution_user_request,
                                &crate::AskMode::planner_execute_chat_wrapped(),
                                Some(promoted_ctx.clone()),
                            )
                            .await?;
                            return Ok(match reason_code {
                                "direct_answer_gate_agent_loop_activation" => {
                                    with_pre_planner_exit_snapshot(
                                        state,
                                        task,
                                        &current_turn_user_request,
                                        reply,
                                        Some(&promoted_ctx),
                                        "direct_answer_gate_agent_loop_activation",
                                    )
                                }
                                "direct_answer_gate_contract_boundary_execute" => {
                                    with_pre_planner_exit_snapshot(
                                        state,
                                        task,
                                        &current_turn_user_request,
                                        reply,
                                        Some(&promoted_ctx),
                                        "direct_answer_gate_contract_boundary_execute",
                                    )
                                }
                                "direct_answer_gate_evidence_projection_execute" => {
                                    with_pre_planner_exit_snapshot(
                                        state,
                                        task,
                                        &current_turn_user_request,
                                        reply,
                                        Some(&promoted_ctx),
                                        "direct_answer_gate_evidence_projection_execute",
                                    )
                                }
                                "contract_test_hint_promoted_to_planner" => {
                                    with_pre_planner_exit_snapshot(
                                        state,
                                        task,
                                        &current_turn_user_request,
                                        reply,
                                        Some(&promoted_ctx),
                                        "contract_test_hint_promoted_to_planner",
                                    )
                                }
                                _ => with_pre_planner_exit_snapshot(
                                    state,
                                    task,
                                    &current_turn_user_request,
                                    reply,
                                    Some(&promoted_ctx),
                                    "direct_answer_gate_promoted_to_planner",
                                ),
                            });
                        }
                    }
                }
            }
            let chat_user_request = direct_answer_chat_user_request(
                chat_user_request,
                &current_turn_user_request,
                direct_answer_gate_approved,
            );
            let request_for_chat_prompt =
                chat_request_for_prompt(&current_turn_user_request, &chat_user_request);
            let chat_prompt = crate::render_prompt_template(
                &chat_prompt_template,
                &[
                    ("__PERSONA_PROMPT__", &task_persona_prompt),
                    (
                        "__AGENT_RUNTIME_IDENTITY__",
                        state.agent_runtime_identity_label(),
                    ),
                    ("__CONTEXT__", &chat_prompt_context),
                    (
                        "__CONFIG_RESPONSE_LANGUAGE__",
                        &state.policy.command_intent.default_locale,
                    ),
                    ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
                    ("__REQUEST__", &request_for_chat_prompt),
                ],
            );
            let raw_answer = crate::llm_gateway::run_with_fallback_with_prompt_source(
                state,
                task,
                &chat_prompt,
                &chat_prompt_source,
            )
            .await
            .map_err(|e| e.to_string())?;
            let mut answer = ensure_active_task_required_visible_literals(
                raw_answer,
                agent_run_context.as_ref(),
            );
            if active_task_text_rewrite_context(agent_run_context.as_ref()) {
                let repair_prompt = active_task_rewrite_conservation_prompt(&chat_prompt, &answer);
                let repaired_answer = crate::llm_gateway::run_with_fallback_with_prompt_source(
                    state,
                    task,
                    &repair_prompt,
                    &chat_prompt_source,
                )
                .await
                .map_err(|e| e.to_string())?;
                answer = ensure_active_task_required_visible_literals(
                    repaired_answer,
                    agent_run_context.as_ref(),
                );
            }
            answer = repair_active_task_factual_rewrite_if_needed(
                state,
                task,
                &chat_prompt,
                &chat_prompt_source,
                answer,
                agent_run_context.as_ref(),
            )
            .await?;
            if direct_chat_answer_needs_repair(&answer) {
                tracing::warn!(
                    "{} worker_once: ask direct_chat_answer_repair task_id={} rejected={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    crate::truncate_for_log(&answer)
                );
                let repair_prompt = direct_chat_answer_repair_prompt(&chat_prompt, &answer);
                let repaired_answer = crate::llm_gateway::run_with_fallback_with_prompt_source(
                    state,
                    task,
                    &repair_prompt,
                    &chat_prompt_source,
                )
                .await
                .map_err(|e| e.to_string())?;
                let repaired_answer = ensure_active_task_required_visible_literals(
                    repaired_answer,
                    agent_run_context.as_ref(),
                );
                if direct_chat_answer_needs_repair(&repaired_answer) {
                    return Err(format!(
                        "direct chat answer remained malformed after repair: {}",
                        crate::truncate_for_log(&repaired_answer)
                    ));
                }
                answer = repaired_answer;
            }
            Ok(with_pre_planner_exit_snapshot(
                state,
                task,
                &current_turn_user_request,
                ask_reply_with_chat_process(answer, &request_language_hint),
                agent_run_context.as_ref(),
                "direct_answer_gate_chat_fallback",
            ))
        }
        mode @ crate::AskMode::Act { .. } => {
            let effective_ask_mode = agent_run_context
                .as_ref()
                .and_then(|ctx| ctx.route_result.as_ref())
                .map(|route| route.ask_mode.clone())
                .filter(|mode| mode.act_finalize_style().is_some())
                .unwrap_or(mode);
            execute_via_planner_loop(
                state,
                task,
                prompt_with_memory,
                execution_user_request,
                &effective_ask_mode,
                agent_run_context.clone(),
            )
            .await
        }
        crate::AskMode::ClarifyOrChat {
            entry: crate::ChatEntryStrategy::NormalizerThenClarify,
        } => {
            let clarify_reason = agent_run_context
                .as_ref()
                .and_then(|ctx| ctx.route_result.as_ref())
                .map(|route| route.route_reason.as_str())
                .unwrap_or("router_selected_clarify");
            let preferred_clarify = preferred_route_clarify_question(agent_run_context.as_ref());
            let structured_clarify_context =
                route_structured_clarify_context(agent_run_context.as_ref());
            let clarify_policy = if structured_clarify_context.is_some()
                || (preferred_clarify.is_none()
                    && agent_run_context
                        .as_ref()
                        .and_then(|ctx| ctx.route_result.as_ref())
                        .is_some_and(|route| route.clarify_question.trim().is_empty()))
            {
                crate::intent_router::ClarifyQuestionPolicy::SafeFallback
            } else {
                crate::intent_router::ClarifyQuestionPolicy::AllowModel
            };
            let clarify = crate::finalize::render_clarify_question(
                state,
                task,
                crate::finalize::ClarifyRenderRequest {
                    user_request: resolved_prompt,
                    resolver_reason: clarify_reason,
                    candidate_context: structured_clarify_context.as_deref(),
                    preferred_question: preferred_clarify.as_deref(),
                    policy: clarify_policy,
                    // §7.2: ask_flow 路由到 AskClarify 但 route_result 也没给 clarify_question
                    // → IntentUnresolved（与 ask_pipeline 同语义）。
                    fallback_source: crate::fallback::ClarifyFallbackSource::IntentUnresolved,
                },
            )
            .await;
            Ok(with_pre_planner_exit_snapshot(
                state,
                task,
                &current_turn_user_request,
                ask_reply_with_chat_process(clarify, &process_language_hint),
                agent_run_context.as_ref(),
                "router_selected_clarify",
            ))
        }
    }
}

pub(crate) async fn analyze_attached_images_for_ask(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    resolved_prompt: &str,
) -> anyhow::Result<Option<String>> {
    let Some(images) = payload.get("images").and_then(|v| v.as_array()) else {
        return Ok(None);
    };
    if images.is_empty() {
        return Ok(None);
    }
    let mut args = json!({
        "action": "describe",
        "images": images,
    });
    let instruction = resolved_prompt.trim();
    if let Some(obj) = args.as_object_mut() {
        if !instruction.is_empty() {
            obj.insert(
                "instruction".to_string(),
                Value::String(instruction.to_string()),
            );
        }
        if let Some(language) = payload
            .get("response_language")
            .or_else(|| payload.get("language"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            obj.insert(
                "response_language".to_string(),
                Value::String(language.to_string()),
            );
        }
    }
    crate::skills::run_skill_with_runner(state, task, "image_vision", args)
        .await
        .map_err(anyhow::Error::msg)
        .map(Some)
}

pub(crate) async fn transcribe_attached_audio_for_ask(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    typed_prompt: &str,
) -> anyhow::Result<Option<String>> {
    let Some(audio) = payload.get("audio") else {
        return Ok(None);
    };
    let Some(audio_arg) = audio_arg_from_payload(audio) else {
        return Ok(None);
    };
    let outcome = crate::skills::run_skill_with_runner_outcome(
        state,
        task,
        "audio_transcribe",
        json!({ "audio": audio_arg }),
    )
    .await
    .map_err(anyhow::Error::msg)?;
    let transcript = outcome.text.trim();
    if transcript.is_empty() {
        return Err(anyhow::anyhow!("audio_transcript_empty"));
    }
    let template = crate::load_prompt_template_for_state(
        state,
        VOICE_CHAT_PROMPT_LOGICAL_PATH,
        DEFAULT_VOICE_CHAT_PROMPT_TEMPLATE,
    )
    .0;
    let mut prompt = template.replace("__TRANSCRIPT__", transcript);
    let typed_prompt = typed_prompt.trim();
    if !typed_prompt.is_empty() {
        prompt.push_str("\n\n[RUSTCLAW_TYPED_TEXT]\n");
        prompt.push_str(typed_prompt);
        prompt.push_str("\n[/RUSTCLAW_TYPED_TEXT]");
    }
    Ok(Some(prompt))
}

fn audio_arg_from_payload(audio: &Value) -> Option<Value> {
    if audio.get("path").and_then(Value::as_str).is_some()
        || audio.get("url").and_then(Value::as_str).is_some()
    {
        return Some(audio.clone());
    }
    if let Some(path) = audio.as_str().map(str::trim).filter(|v| !v.is_empty()) {
        return Some(json!({ "path": path }));
    }
    None
}

#[cfg(test)]
#[path = "ask_flow_tests.rs"]
mod tests;
