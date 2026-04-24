use serde_json::{json, Value};
use tracing::info;

use crate::{schedule_service, AppState};

pub(super) struct PreparedAskExecutionContext {
    pub(super) context_bundle: crate::task_context_builder::TaskContextBundle,
    pub(super) chat_prompt_context: String,
    pub(super) resolved_prompt_for_execution: String,
    pub(super) prompt_with_memory_for_execution: String,
    pub(super) recent_execution_context: String,
}

pub(super) struct PreparedAskRouting {
    pub(super) route_result: crate::RouteResult,
    pub(super) execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    pub(super) turn_analysis: Option<crate::intent_router::TurnAnalysis>,
    pub(super) resolved_prompt: String,
    pub(super) agent_mode: bool,
    pub(super) immediate_prior_turn_was_clarify: bool,
    /// Phase 3.2：合并 routed_mode + direct_resume_* 后的最终模式。
    /// Stage D 已删除原 direct_resume_discussion / direct_resume_execution bool 字段，
    /// 全部判断走 ask_mode 谓词方法（is_resume_discussion / resume_execution）。
    pub(super) ask_mode: crate::AskMode,
}

pub(super) struct PreparedAskInput {
    pub(super) prompt: String,
    pub(super) source: String,
}

pub(super) struct PreparedRunSkillInput {
    pub(super) skill_name: String,
    pub(super) args: Value,
}

fn apply_fresh_deictic_clarify_guard(
    state: &AppState,
    prompt: &str,
    prompt_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    route_result: &mut crate::RouteResult,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    last_turn_full: &str,
    recent_assistant_replies: &str,
) {
    let Some(decision) =
        crate::intent::continuation_resolver::resolve_fresh_deictic_clarify_guard_with_surface(
            route_result,
            prompt,
            session_snapshot,
            last_turn_full,
            recent_assistant_replies,
            prompt_surface,
        )
    else {
        return;
    };
    route_result.set_routed_mode(crate::RoutedMode::AskClarify);
    route_result.needs_clarify = true;
    route_result.resolved_intent = prompt.trim().to_string();
    route_result.clarify_question =
        crate::i18n_t_with_default(state, decision.question_i18n_key, decision.default_question);
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route_result.output_contract.locator_hint.clear();
    if route_result.route_reason.trim().is_empty() {
        route_result.route_reason = decision.reason.to_string();
    } else if !route_result.route_reason.contains(decision.reason) {
        route_result.route_reason.push(';');
        route_result.route_reason.push_str(decision.reason);
    }
}

fn merged_prompt_from_task_turn_analysis(
    prior_primary_task_prompt: Option<&str>,
    prior_primary_task_output: Option<&str>,
    current_prompt: &str,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> Option<String> {
    let prior = prior_primary_task_prompt
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let current = current_prompt.trim();
    if current.is_empty() || current == prior || current.contains(prior) {
        return None;
    }
    let analysis = turn_analysis?;
    let policy = analysis.target_task_policy?;
    let turn_type = analysis.turn_type?;
    let structured_patch = analysis
        .state_patch
        .as_ref()
        .and_then(render_task_state_patch);
    let include_prior_output = matches!(
        (turn_type, policy),
        (
            crate::intent_router::TurnType::TaskAppend
                | crate::intent_router::TurnType::TaskCorrect
                | crate::intent_router::TurnType::TaskScopeUpdate,
            crate::intent_router::TargetTaskPolicy::ReuseActive,
        )
    );
    let prior_output = if include_prior_output {
        prior_primary_task_output
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(truncate_task_output_for_merge)
    } else {
        None
    };
    match (turn_type, policy) {
        (
            crate::intent_router::TurnType::TaskAppend,
            crate::intent_router::TargetTaskPolicy::ReuseActive,
        ) => Some(merged_reuse_active_prompt(
            prior,
            prior_output.as_deref(),
            current,
            structured_patch.as_deref(),
            "Keep the same task and append this new instruction.",
        )),
        (
            crate::intent_router::TurnType::TaskCorrect,
            crate::intent_router::TargetTaskPolicy::ReuseActive,
        ) => Some(merged_reuse_active_prompt(
            prior,
            prior_output.as_deref(),
            current,
            structured_patch.as_deref(),
            "Keep the same task, but treat the new instruction as a correction that overrides conflicting earlier details.",
        )),
        (
            crate::intent_router::TurnType::TaskScopeUpdate,
            crate::intent_router::TargetTaskPolicy::ReuseActive,
        ) => Some(merged_reuse_active_prompt(
            prior,
            prior_output.as_deref(),
            current,
            structured_patch.as_deref(),
            "Keep the same task, but update its scope, priorities, or boundaries using the new instruction. Treat conceptual scope words such as module/topic/section/audience as content constraints, not filesystem targets, unless the user explicitly asks to inspect files, code, or logs. If the updated scope is enough to produce a useful generic draft/plan/answer, produce that scoped result now instead of asking for optional platform/system subtype details.",
        )),
        (
            crate::intent_router::TurnType::TaskReplace,
            crate::intent_router::TargetTaskPolicy::ReplaceActive,
        ) => Some(merged_replace_active_prompt(
            prior,
            prior_output.as_deref(),
            current,
            structured_patch.as_deref(),
        )),
        _ => None,
    }
}

fn truncate_task_output_for_merge(output: &str) -> String {
    const MAX_CHARS: usize = 2000;
    let trimmed = output.trim();
    if trimmed.chars().count() <= MAX_CHARS {
        return trimmed.to_string();
    }
    trimmed.chars().take(MAX_CHARS).collect::<String>()
}

fn render_task_state_patch(state_patch: &Value) -> Option<String> {
    match state_patch {
        Value::Null => None,
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        other => serde_json::to_string(other)
            .ok()
            .filter(|serialized| !serialized.is_empty()),
    }
}

fn merged_reuse_active_prompt(
    prior: &str,
    prior_output: Option<&str>,
    current: &str,
    structured_patch: Option<&str>,
    merge_instruction: &str,
) -> String {
    let recent_output_block = prior_output
        .map(|output| format!("\n\nMost recent generated output:\n{output}"))
        .unwrap_or_default();
    let continuity_rules = "\n\nContinuity rules:\n- Preserve all active prior subject, scope, audience, tone, key facts, and safety constraints unless the new instruction explicitly overrides them.\n- Treat the latest output-shape constraints as highest priority: exact bullet/table row counts, word/character limits, and output-only/body-only requests must be followed.\n- For table requests, row counts mean data rows only, excluding the header and separator. A two-row table must contain exactly two data rows.\n- When the latest instruction specifies a table, bullet count, final sentence, body-only, or another exact output shape, emit only that requested shape; do not append explanatory notes or summaries outside it.\n- A format/count-only change must not broaden a narrowed scope. If an exact count needs more items than the recent output has, split, combine, or elaborate within the current scope instead of adding unrelated categories.\n- Style or quality feedback means rewrite the deliverable itself. Do not answer with meta-commentary like \"it already meets that\" unless the user explicitly asks for evaluation.\n- Do not invent unobserved project setup commands, package names, dependency lines, version numbers, paths, or configuration values. If such details are not provided or observed, keep them neutral/generic or say to follow the repo's documented setup path.\n- For a project-specific setup/deployment note with no observed setup evidence, do not include command blocks, backticked command invocations, package names, fake CLI steps, settings-file claims, or assigned installer roles. If recent output already contains unsupported setup commands or setup artifacts, remove or replace them with neutral documented-path wording instead of preserving them.\n- When rewriting setup/deployment/onboarding text for a simpler audience, do not introduce alternate OS scripts, download methods, websites, ports, Bot platforms, API-key locations, installer roles, or launch commands unless they already appear in recent output or authoritative context. Do not convert shell scripts (.sh) into GUI actions such as double-clicking unless that GUI flow was explicitly observed; the words double-click/双击 must not appear for shell-script setup rewrites unless observed. Simplify by replacing commands with neutral documented-step wording, not by inventing easier-looking steps.\n- When shortening, reformatting, or asking for the final sentence/body, synthesize a complete standalone answer from the current task and recent output. Do not return only a heading, label, dangling fragment, or trailing sentence if that would drop required facts.\n- If the recent output is a clarification question and the new instruction only adds constraints without answering the missing slot, do not repeat the same clarification indefinitely. For low-risk writing or chat-only drafting tasks, produce a best-effort draft using a neutral, reasonable assumption. For file, code, command, system, credential, delivery, or other concrete-action tasks, keep clarifying instead of guessing.";
    match structured_patch {
        Some(patch) => format!(
            "Current task:\n{prior}{recent_output_block}{continuity_rules}\n\nStructured task updates:\n{patch}\n\n{merge_instruction}\nNew user instruction:\n{current}"
        ),
        None => format!(
            "Current task:\n{prior}{recent_output_block}{continuity_rules}\n\n{merge_instruction}\nNew user instruction:\n{current}"
        ),
    }
}

fn merged_replace_active_prompt(
    prior: &str,
    prior_output: Option<&str>,
    current: &str,
    structured_patch: Option<&str>,
) -> String {
    let recent_output_block = prior_output
        .map(|output| format!("\n\nMost recent generated output:\n{output}"))
        .unwrap_or_default();
    match structured_patch {
        Some(patch) => format!(
            "Previous task:\n{prior}{recent_output_block}\n\nStructured replacement details:\n{patch}\n\nDiscard that task and replace it with this new goal. Preserve the prior subject/topic unless the new instruction explicitly changes it, and treat the replacement as a deliverable/style update rather than a filesystem lookup unless the user explicitly asks to inspect files, code, or logs:\n{current}"
        ),
        None => format!(
            "Previous task:\n{prior}{recent_output_block}\n\nDiscard that task and replace it with this new goal. Preserve the prior subject/topic unless the new instruction explicitly changes it, and treat the replacement as a deliverable/style update rather than a filesystem lookup unless the user explicitly asks to inspect files, code, or logs:\n{current}"
        ),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn should_probe_transcript_for_clarify_fallback(
    prompt: &str,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    should_probe_transcript_for_clarify_fallback_with_surface(session_snapshot, &surface)
}

fn should_probe_transcript_for_clarify_fallback_with_surface(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    if session_snapshot
        .conversation_state
        .as_ref()
        .and_then(|state| state.last_primary_task_prompt.as_deref())
        .is_some_and(|prompt| !prompt.trim().is_empty())
    {
        return false;
    }
    if session_snapshot.active_clarify_state.is_some()
        || session_snapshot.active_followup_frame.is_some()
        || session_snapshot.active_observed_facts.is_some()
    {
        return false;
    }
    if surface.looks_like_locator_only_reply() {
        return true;
    }
    crate::intent::continuation_resolver::prompt_looks_like_clarify_target_only_with_surface(
        &surface,
    ) && surface.requested_read_range.is_none()
        && surface.field_selector_count == 0
        && surface.requested_listing_limit.is_none()
}

fn log_ask_memory_snapshot(
    task: &crate::ClaimedTask,
    long_term_log: &str,
    preferences_log: &str,
    trigger_log: &str,
    fact_log: &str,
    related_log: &str,
    recalled_count: usize,
    recalled_log: &str,
) {
    info!(
        "worker_once: ask memory task_id={} memory.long_term_summary={} memory.preferences={} memory.similar_triggers={} memory.relevant_facts={} memory.related_events={} memory.recalled_recent_count={} memory.recalled_recent={}",
        task.task_id,
        long_term_log,
        preferences_log,
        trigger_log,
        fact_log,
        related_log,
        recalled_count,
        recalled_log,
    );
}

pub(super) async fn prepare_ask_execution_context(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    route_result: &crate::RouteResult,
    resolved_prompt: &str,
) -> anyhow::Result<PreparedAskExecutionContext> {
    let chat_memory_budget_chars =
        crate::dynamic_chat_memory_budget_chars(state, task, resolved_prompt);
    let mut context_bundle = crate::task_context_builder::build_execution_task_context_bundle(
        state,
        task,
        route_result,
        resolved_prompt,
        chat_memory_budget_chars,
    );
    let execution_view = context_bundle
        .execution_view
        .as_ref()
        .expect("execution context bundle should include execution_view");
    let long_term_summary = execution_view.memory_ctx.long_term_summary.clone();
    let preferences = execution_view.memory_ctx.preferences.clone();
    let recalled = execution_view.memory_ctx.recalled.clone();
    let similar_triggers = execution_view.memory_ctx.similar_triggers.clone();
    let relevant_facts = execution_view.memory_ctx.relevant_facts.clone();
    let recent_related_events = execution_view.memory_ctx.recent_related_events.clone();
    let prompt_with_memory = execution_view.memory_ctx.prompt_with_memory.clone();
    let mut chat_prompt_context = execution_view.memory_ctx.chat_prompt_context.clone();
    let mut resolved_prompt_for_execution = resolved_prompt.to_string();
    let mut prompt_with_memory_for_execution = prompt_with_memory.clone();
    let recent_execution_context = execution_view.recent_execution_context.clone();
    if let Some(image_context) =
        crate::analyze_attached_images_for_ask(state, task, payload, resolved_prompt).await?
    {
        crate::task_context_builder::set_execution_image_context(
            &mut context_bundle,
            Some(image_context),
        );
    }
    crate::task_context_builder::apply_execution_context_to_prompts(
        &context_bundle,
        &mut chat_prompt_context,
        &mut resolved_prompt_for_execution,
        &mut prompt_with_memory_for_execution,
    );
    let long_term_log = long_term_summary
        .as_deref()
        .map(crate::truncate_for_log)
        .unwrap_or_else(|| "<none>".to_string());
    let recalled_log = if recalled.is_empty() {
        "<none>".to_string()
    } else {
        let merged = recalled
            .iter()
            .map(|(role, content)| format!("{role}:{content}"))
            .collect::<Vec<_>>()
            .join(" | ");
        crate::truncate_for_log(&merged)
    };
    let preferences_log = if preferences.is_empty() {
        "<none>".to_string()
    } else {
        let merged = preferences
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" | ");
        crate::truncate_for_log(&merged)
    };
    let trigger_log = if similar_triggers.is_empty() {
        "<none>".to_string()
    } else {
        crate::truncate_for_log(
            &similar_triggers
                .iter()
                .map(|v| v.text.clone())
                .collect::<Vec<_>>()
                .join(" | "),
        )
    };
    let fact_log = if relevant_facts.is_empty() {
        "<none>".to_string()
    } else {
        crate::truncate_for_log(
            &relevant_facts
                .iter()
                .map(|v| v.text.clone())
                .collect::<Vec<_>>()
                .join(" | "),
        )
    };
    let related_log = if recent_related_events.is_empty() {
        "<none>".to_string()
    } else {
        crate::truncate_for_log(
            &recent_related_events
                .iter()
                .map(|v| v.text.clone())
                .collect::<Vec<_>>()
                .join(" | "),
        )
    };
    log_ask_memory_snapshot(
        task,
        &long_term_log,
        &preferences_log,
        &trigger_log,
        &fact_log,
        &related_log,
        recalled.len(),
        &recalled_log,
    );
    Ok(PreparedAskExecutionContext {
        context_bundle,
        chat_prompt_context,
        resolved_prompt_for_execution,
        prompt_with_memory_for_execution,
        recent_execution_context,
    })
}

pub(super) async fn prepare_ask_input(
    _state: &AppState,
    _task: &crate::ClaimedTask,
    payload: &mut Value,
) -> PreparedAskInput {
    let prompt = payload
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let source = payload
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    PreparedAskInput { prompt, source }
}

pub(super) fn prepare_run_skill_input(payload: &Value) -> PreparedRunSkillInput {
    let skill_name = payload
        .get("skill_name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let args = payload.get("args").cloned().unwrap_or_else(|| json!(""));
    PreparedRunSkillInput { skill_name, args }
}

pub(super) async fn maybe_finalize_schedule_direct_text_success(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
) -> anyhow::Result<bool> {
    let is_schedule_triggered = payload
        .get("schedule_triggered")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let schedule_task_mode = payload
        .get("schedule_task_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let schedule_force_agent = payload
        .get("schedule_force_agent")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let schedule_direct_text_mode = is_schedule_triggered
        && !schedule_force_agent
        && (schedule_task_mode.is_empty() || schedule_task_mode == "direct_text");
    if !schedule_direct_text_mode {
        return Ok(false);
    }
    let direct_text = prompt.trim();
    if direct_text.is_empty() {
        return Ok(false);
    }
    let answer_text = crate::intercept_response_text_for_delivery(direct_text);
    crate::finalize::finalize_ask_direct_success(
        state,
        task,
        payload,
        prompt,
        &answer_text,
        "schedule_direct_text",
        false,
        "",
    )
    .await?;
    Ok(true)
}

pub(super) async fn prepare_ask_routing(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
    source: &str,
) -> PreparedAskRouting {
    let agent_mode = payload
        .get("agent_mode")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let is_resume_continue = super::is_resume_continue_source(source);
    let (now_iso, timezone_str, schedule_rules) =
        schedule_service::schedule_context_for_normalizer(state);
    let session_snapshot = crate::conversation_state::load_active_session_snapshot(state, task);
    let routed_prompt = crate::conversation_state::rewrite_prompt_with_alias_bindings(
        prompt,
        Some(&session_snapshot),
    )
    .unwrap_or_else(|| prompt.to_string());
    let mut last_turn_full = None;
    let mut recent_assistant_replies = None;
    let routed_prompt_surface =
        crate::intent::surface_signals::analyze_prompt_surface(&routed_prompt);
    let mut clarify_followup_resolution =
        crate::intent::continuation_resolver::resolve_clarify_followup_from_session_with_surface(
            &routed_prompt,
            None,
            Some(&session_snapshot),
            &routed_prompt_surface,
        );
    let mut immediate_prior_turn_was_clarify = session_snapshot.active_clarify_state.is_some();
    if matches!(
        clarify_followup_resolution,
        crate::intent::continuation_resolver::ClarifyFollowupResolution::None
    ) && should_probe_transcript_for_clarify_fallback_with_surface(
        &session_snapshot,
        &routed_prompt_surface,
    ) {
        let built_last_turn_full = crate::memory::build_last_turn_full_context(
            state,
            task.user_key.as_deref(),
            task.user_id,
            task.chat_id,
            400,
            1200,
        );
        immediate_prior_turn_was_clarify =
            crate::intent::continuation_resolver::immediate_prior_turn_was_clarify(
                &built_last_turn_full,
            );
        clarify_followup_resolution =
            crate::intent::continuation_resolver::resolve_clarify_followup_from_session_with_surface(
                &routed_prompt,
                Some(&built_last_turn_full),
                Some(&session_snapshot),
                &routed_prompt_surface,
            );
        last_turn_full = Some(built_last_turn_full);
    }
    let normalizer_prompt = match &clarify_followup_resolution {
        crate::intent::continuation_resolver::ClarifyFollowupResolution::NormalizerRewrite {
            rewritten_prompt,
            ..
        } => rewritten_prompt.clone(),
        crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
            hit,
        ) => {
            crate::clarify_followup::emit_clarify_rewrite_event(&task.task_id, hit);
            info!(
                "{} worker_once: ask clarify_locator_reply_rewrite task_id={} reason={} normalizer_rewrite=true",
                crate::highlight_tag("routing"),
                task.task_id,
                hit.reason.as_metric_label()
            );
            hit.resolved_intent.clone()
        }
        _ => routed_prompt.clone(),
    };
    let explicit_resume_binding =
        crate::intent::resume_policy::explicit_resume_context_binding(payload, is_resume_continue);
    let recent_failed_resume_binding = crate::intent::resume_policy::recent_failed_resume_candidate(
        state,
        task,
        explicit_resume_binding.is_some(),
    );
    let resume_binding = explicit_resume_binding
        .clone()
        .or_else(|| recent_failed_resume_binding.clone());
    let binding_context_value = crate::intent::resume_policy::binding_context_json(
        source,
        is_resume_continue,
        resume_binding.as_ref(),
    );
    let normalizer_out = crate::intent_router::run_intent_normalizer(
        state,
        task,
        &normalizer_prompt,
        Some(&session_snapshot),
        resume_binding
            .as_ref()
            .map(|binding| &binding.resume_context),
        Some(&binding_context_value),
        &now_iso,
        &timezone_str,
        &schedule_rules,
    )
    .await;
    // Phase 0.4: 若 normalizer 已给出 schedule_intent，缓存起来，后续
    // `schedule.compile` 技能可以直接复用，避免对同一段文本再跑一次
    // `schedule_intent_prompt` LLM 调用。
    if let Some(intent) = normalizer_out.schedule_intent.as_ref() {
        state.cache_task_schedule_intent(&task.task_id, &normalizer_prompt, intent);
    }
    let turn_analysis = normalizer_out.turn_analysis.clone();
    let mut execution_recipe_hint = normalizer_out.execution_recipe_hint;
    let mut route_result =
        crate::intent_router::route_result_from_normalizer(state, task, &normalizer_out);
    let needs_last_turn_full_after_normalizer = !immediate_prior_turn_was_clarify
        || crate::intent::continuation_resolver::fresh_deictic_guard_needs_recent_assistant_probe_with_surface(
            &route_result,
            &routed_prompt,
            Some(&session_snapshot),
            "<none>",
            &routed_prompt_surface,
        );
    let last_turn_full = last_turn_full.unwrap_or_else(|| {
        if !needs_last_turn_full_after_normalizer {
            return "<none>".to_string();
        }
        crate::memory::build_last_turn_full_context(
            state,
            task.user_key.as_deref(),
            task.user_id,
            task.chat_id,
            400,
            1200,
        )
    });
    if !immediate_prior_turn_was_clarify && last_turn_full != "<none>" {
        immediate_prior_turn_was_clarify =
            crate::intent::continuation_resolver::immediate_prior_turn_was_clarify(&last_turn_full);
    }
    let recent_assistant_replies = if crate::intent::continuation_resolver::
        fresh_deictic_guard_needs_recent_assistant_probe_with_surface(
            &route_result,
            &routed_prompt,
            Some(&session_snapshot),
            &last_turn_full,
            &routed_prompt_surface,
        )
    {
        recent_assistant_replies.get_or_insert_with(|| {
            crate::memory::build_recent_assistant_replies_context(
                state,
                task.user_key.as_deref(),
                task.user_id,
                task.chat_id,
                3,
                220,
            )
        })
    } else {
        recent_assistant_replies.get_or_insert_with(|| "<none>".to_string())
    };
    apply_fresh_deictic_clarify_guard(
        state,
        &routed_prompt,
        &routed_prompt_surface,
        &mut route_result,
        Some(&session_snapshot),
        &last_turn_full,
        &recent_assistant_replies,
    );
    let resume_runtime_binding = crate::intent::resume_policy::select_resume_runtime_binding(
        &route_result,
        resume_binding.as_ref(),
    );
    info!(
        "worker_once: ask raw_message task_id={} user_id={} chat_id={} text={}",
        task.task_id,
        task.user_id,
        task.chat_id,
        crate::truncate_for_log(prompt)
    );
    let resume_runtime = crate::intent::resume_policy::resolve_resume_runtime_prompt(
        state,
        task,
        payload,
        prompt,
        &route_result,
        resume_runtime_binding,
    );
    let mut runtime_prompt = resume_runtime.runtime_prompt;
    if let Some(merged_prompt) = merged_prompt_from_task_turn_analysis(
        session_snapshot
            .conversation_state
            .as_ref()
            .and_then(|state| state.last_primary_task_prompt.as_deref()),
        session_snapshot
            .conversation_state
            .as_ref()
            .and_then(|state| state.last_primary_task_output.as_deref()),
        prompt,
        turn_analysis.as_ref(),
    ) {
        info!(
            "{} worker_once: ask task_turn_merge task_id={} turn_type={:?} target_task_policy={:?} merged_prompt={}",
            crate::highlight_tag("routing"),
            task.task_id,
            turn_analysis.as_ref().and_then(|analysis| analysis.turn_type),
            turn_analysis
                .as_ref()
                .and_then(|analysis| analysis.target_task_policy),
            crate::truncate_for_log(&merged_prompt)
        );
        runtime_prompt = merged_prompt;
        route_result.resolved_intent = runtime_prompt.clone();
        if !route_result.route_reason.contains("task_turn_merge") {
            route_result.route_reason.push_str(";task_turn_merge");
        }
    }
    info!(
        "worker_once: ask received_message task_id={} user_id={} chat_id={} text={}",
        task.task_id,
        task.user_id,
        task.chat_id,
        crate::truncate_for_log(&runtime_prompt)
    );
    let context_resolution = crate::intent_router::ContextResolution {
        resolved_user_intent: runtime_prompt.clone(),
        needs_clarify: route_result.needs_clarify,
        confidence: route_result.route_confidence,
        reason: route_result.route_reason.clone(),
    };
    let resolved_prompt = context_resolution.resolved_user_intent.clone();
    if route_result.needs_clarify || !route_result.ask_mode.is_execute_gate() {
        execution_recipe_hint = None;
    }
    crate::intent::safety_class::apply_route_risk_ceiling(
        &mut route_result,
        execution_recipe_hint.as_ref(),
    );
    info!(
        "{} worker_once: ask resolved_message task_id={} needs_clarify={} confidence={} reason={} resolved_text={}",
        crate::highlight_tag("routing"),
        task.task_id,
        context_resolution.needs_clarify,
        context_resolution.confidence.unwrap_or(-1.0),
        crate::truncate_for_log(&context_resolution.reason),
        crate::truncate_for_log(&resolved_prompt)
    );
    if let Some(analysis) = turn_analysis.as_ref() {
        info!(
            "{} worker_once: ask turn_analysis task_id={} turn_type={:?} target_task_policy={:?} should_interrupt_active_run={} has_state_patch={} attachment_processing_required={}",
            crate::highlight_tag("routing"),
            task.task_id,
            analysis.turn_type,
            analysis.target_task_policy,
            analysis.should_interrupt_active_run,
            analysis.state_patch.is_some(),
            analysis.attachment_processing_required
        );
    }
    let ask_mode = crate::AskMode::from_legacy(
        route_result.routed_mode,
        resume_runtime.should_discuss_context,
        resume_runtime.should_apply_context,
    );
    // 仅在没有 resume flag 主导时校验反向 round-trip；resume_continue/discussion
    // 命中时 to_routed_mode 会做"语义等价但取值不同"的折叠
    // （比如 ResumeContinue → Act 即便原 routed_mode 是 ChatAct），不等于即合理。
    if !resume_runtime.should_discuss_context && !resume_runtime.should_apply_context {
        debug_assert_eq!(
            ask_mode.to_routed_mode(),
            route_result.routed_mode,
            "ask_mode <-> routed_mode invariant broken when no flag dominates"
        );
    }
    PreparedAskRouting {
        route_result,
        execution_recipe_hint,
        turn_analysis,
        resolved_prompt,
        agent_mode,
        immediate_prior_turn_was_clarify,
        ask_mode,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        merged_prompt_from_task_turn_analysis, should_probe_transcript_for_clarify_fallback,
    };

    use serde_json::json;

    #[test]
    fn binding_context_marks_recent_failed_candidate_without_mutating_source() {
        let binding = crate::intent::resume_policy::ResumeContextBinding {
            source: crate::intent::resume_policy::ResumeContextSource::RecentFailedCandidate,
            resume_context: json!({"resume_context_id":"ctx-1"}),
            failed_ts: Some(42),
            has_newer_successful_ask_after_failed_task: true,
        };
        let value =
            crate::intent::resume_policy::binding_context_json("manual", false, Some(&binding));
        assert_eq!(
            value.get("resume_context_source").and_then(|v| v.as_str()),
            Some("recent_failed_resume_candidate")
        );
        assert_eq!(
            value
                .get("is_resume_continue_source")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            value
                .get("has_newer_successful_ask_after_failed_task")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn runtime_resume_binding_is_disabled_when_normalizer_rejects_resume() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "list current workspace".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let binding = crate::intent::resume_policy::ResumeContextBinding {
            source: crate::intent::resume_policy::ResumeContextSource::RecentFailedCandidate,
            resume_context: json!({"resume_context_id":"ctx-2"}),
            failed_ts: Some(7),
            has_newer_successful_ask_after_failed_task: false,
        };
        assert!(crate::intent::resume_policy::select_resume_runtime_binding(
            &route,
            Some(&binding)
        )
        .is_none());
    }

    #[test]
    fn fresh_delivery_deictic_without_immediate_anchor_stays_with_normalizer() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "send the referenced file".to_string(),
            needs_clarify: false,
            route_reason: "recent_context_delivery_binding".to_string(),
            route_confidence: Some(0.83),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: true,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::FileToken,
                requires_content_evidence: false,
                delivery_required: true,
                locator_kind: crate::OutputLocatorKind::Filename,
                delivery_intent: crate::OutputDeliveryIntent::FileSingle,
                semantic_kind: Default::default(),
                locator_hint: "config.toml".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        assert!(
            crate::intent::continuation_resolver::resolve_fresh_deictic_clarify_guard(
                &route,
                "把那个文件发给我",
                None,
                "<none>",
                "<none>",
            )
            .is_none(),
            "generic deictic delivery should stay on the normalizer/planner path"
        );
    }

    #[test]
    fn fresh_delivery_deictic_with_immediate_file_anchor_does_not_force_clarify() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "send the referenced file".to_string(),
            needs_clarify: false,
            route_reason: "recent_context_delivery_binding".to_string(),
            route_confidence: Some(0.83),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: true,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::FileToken,
                requires_content_evidence: false,
                delivery_required: true,
                locator_kind: crate::OutputLocatorKind::Filename,
                delivery_intent: crate::OutputDeliveryIntent::FileSingle,
                semantic_kind: Default::default(),
                locator_hint: "README.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        assert!(
            crate::intent::continuation_resolver::context_contains_immediate_locator_anchor(
                "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] short_preview=README.md",
            )
        );
        assert!(
            crate::intent::continuation_resolver::resolve_fresh_deictic_clarify_guard(
                &route,
                "把那个文件发给我",
                None,
                "<none>",
                "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] short_preview=README.md",
            )
            .is_none()
        );
    }

    #[test]
    fn immediate_locator_anchor_ignores_older_assistant_replies() {
        assert!(!crate::intent::continuation_resolver::context_contains_immediate_locator_anchor(
            "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] short_preview=好的，我来读取 has_code_block=false\n- turn_id=assistant[-2] short_preview=package.json has_code_block=false",
        ));
        assert!(crate::intent::continuation_resolver::context_contains_immediate_locator_anchor(
            "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] short_preview=package.json has_code_block=false\n- turn_id=assistant[-2] short_preview=README.md has_code_block=false",
        ));
    }

    #[test]
    fn fresh_scalar_deictic_without_immediate_anchor_stays_with_normalizer() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "读取 package.json 文件中的 name 字段，只输出该字段的值".to_string(),
            needs_clarify: false,
            route_reason: "recent_context_scalar_binding".to_string(),
            route_confidence: Some(0.83),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Filename,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "package.json".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        assert!(
            crate::intent::continuation_resolver::resolve_fresh_deictic_clarify_guard(
                &route,
                "读一下那个文件里的名字字段，只输出值",
                None,
                "<none>",
                "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] short_preview=好的，我来读取 has_code_block=false\n- turn_id=assistant[-2] short_preview=package.json has_code_block=false",
            )
            .is_none(),
            "generic deictic scalar reads should not be hard-routed by ask_prepare"
        );
    }

    #[test]
    fn fresh_scalar_deictic_with_immediate_file_anchor_does_not_force_clarify() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "读取 package.json 文件中的 name 字段，只输出该字段的值".to_string(),
            needs_clarify: false,
            route_reason: "recent_context_scalar_binding".to_string(),
            route_confidence: Some(0.83),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Filename,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "package.json".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        assert!(
            crate::intent::continuation_resolver::resolve_fresh_deictic_clarify_guard(
                &route,
                "读一下那个文件里的名字字段，只输出值",
                None,
                "<none>",
                "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] short_preview=package.json has_code_block=false\n- turn_id=assistant[-2] short_preview=README.md has_code_block=false",
            )
            .is_none()
        );
    }

    #[test]
    fn fresh_content_deictic_without_immediate_anchor_stays_with_normalizer() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "读取 model_io.log 最后 5 行".to_string(),
            needs_clarify: false,
            route_reason: "memory_established_path_binding".to_string(),
            route_confidence: Some(0.88),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Filename,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "model_io.log".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        assert!(
            crate::intent::continuation_resolver::resolve_fresh_deictic_clarify_guard(
                &route,
                "看看那个模型日志最后 5 行",
                None,
                "<none>",
                "<none>",
            )
            .is_none(),
            "generic deictic content reads should stay on the normalizer/planner path"
        );
    }

    #[test]
    fn fresh_content_deictic_with_immediate_file_anchor_does_not_force_clarify() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "读取 model_io.log 最后 5 行".to_string(),
            needs_clarify: false,
            route_reason: "memory_established_path_binding".to_string(),
            route_confidence: Some(0.88),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Filename,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "model_io.log".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        assert!(
            crate::intent::continuation_resolver::resolve_fresh_deictic_clarify_guard(
                &route,
                "看看那个模型日志最后 5 行",
                None,
                "<none>",
                "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] short_preview=model_io.log has_code_block=false",
            )
            .is_none()
        );
    }

    #[test]
    fn explicit_file_locator_never_forces_clarify() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "send README".to_string(),
            needs_clarify: false,
            route_reason: "explicit_filename".to_string(),
            route_confidence: Some(0.95),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: true,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::FileToken,
                requires_content_evidence: false,
                delivery_required: true,
                locator_kind: crate::OutputLocatorKind::Filename,
                delivery_intent: crate::OutputDeliveryIntent::FileSingle,
                semantic_kind: Default::default(),
                locator_hint: "README.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        assert!(
            crate::intent::continuation_resolver::resolve_fresh_deictic_clarify_guard(
                &route,
                "README.md",
                None,
                "<none>",
                "<none>",
            )
            .is_none()
        );
    }

    #[test]
    fn explicit_bare_filename_delivery_does_not_force_clarify() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "send README".to_string(),
            needs_clarify: false,
            route_reason: "explicit_filename".to_string(),
            route_confidence: Some(0.95),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: true,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::FileToken,
                requires_content_evidence: false,
                delivery_required: true,
                locator_kind: crate::OutputLocatorKind::Filename,
                delivery_intent: crate::OutputDeliveryIntent::FileSingle,
                semantic_kind: Default::default(),
                locator_hint: "README".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        assert!(
            crate::intent::continuation_resolver::resolve_fresh_deictic_clarify_guard(
                &route,
                "把 README 发给我",
                None,
                "<none>",
                "<none>",
            )
            .is_none()
        );
    }

    #[test]
    fn fresh_content_with_explicit_bare_filename_does_not_force_clarify() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "读取 README 前 20 行并总结".to_string(),
            needs_clarify: false,
            route_reason: "explicit_filename".to_string(),
            route_confidence: Some(0.92),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Filename,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "README".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        assert!(
            crate::intent::continuation_resolver::resolve_fresh_deictic_clarify_guard(
                &route,
                "扫一眼 README 前 20 行，再提炼成 3 句话",
                None,
                "<none>",
                "<none>",
            )
            .is_none()
        );
    }

    #[test]
    fn fresh_content_with_multiple_explicit_filenames_does_not_force_clarify() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "比较 Cargo.toml 和 Cargo.lock 哪个更大".to_string(),
            needs_clarify: false,
            route_reason: "explicit_compare_targets".to_string(),
            route_confidence: Some(0.93),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        assert!(
            crate::intent::continuation_resolver::resolve_fresh_deictic_clarify_guard(
                &route,
                "比较 Cargo.toml 和 Cargo.lock 哪个更大，顺手用一句通俗话解释原因",
                None,
                "<none>",
                "<none>",
            )
            .is_none()
        );
    }

    #[test]
    fn immediate_last_turn_clarify_placeholder_is_detected() {
        assert!(crate::intent::continuation_resolver::immediate_prior_turn_was_clarify(
            "### LAST_TURN_FULL\n[TURN -1]\nUser: 读一下那个文件里的名字字段，只输出值\nAssistant: [clarification_requested]\n[/TURN]"
        ));
        assert!(!crate::intent::continuation_resolver::immediate_prior_turn_was_clarify(
            "### LAST_TURN_FULL\n[TURN -1]\nUser: 看看那个重启脚本在不在\nAssistant: 有，路径：scripts/restart_clawd_latest.sh\n[/TURN]"
        ));
    }

    #[test]
    fn transcript_probe_is_enabled_for_locator_only_reply_without_session_state() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(should_probe_transcript_for_clarify_fallback(
            "/tmp/device_local/logs/model_io.log",
            &snapshot,
        ));
    }

    #[test]
    fn transcript_probe_is_skipped_when_session_state_already_exists() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: Some(crate::clarify_state::ClarifyState {
                missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
                pending_question: "请提供具体要读取的文件名或路径。".to_string(),
                candidate_targets: Vec::new(),
                delivery_required: false,
                output_shape: None,
                semantic_kind: None,
                source_request: "看一下那个日志最后 5 行".to_string(),
                source_task_id: "task-1".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
            }),
            active_observed_facts: None,
        };
        assert!(!should_probe_transcript_for_clarify_fallback(
            "/tmp/device_local/logs/model_io.log",
            &snapshot,
        ));
    }

    #[test]
    fn transcript_probe_is_skipped_for_regular_new_request() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(!should_probe_transcript_for_clarify_fallback(
            "读取 /tmp/device_local/logs/model_io.log 最后 5 行",
            &snapshot,
        ));
    }

    #[test]
    fn transcript_probe_is_skipped_when_primary_task_prompt_exists() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("Help me write a proposal".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(!should_probe_transcript_for_clarify_fallback(
            "It is for executives",
            &snapshot,
        ));
    }

    #[test]
    fn clarify_followup_routing_prompt_merges_previous_operation_for_non_locator_reply_target() {
        let merged = crate::intent::continuation_resolver::resolve_clarify_followup(
            "就在 scripts/restart_clawd_latest.sh",
            Some("[LAST_TURN_FULL]\nUser: 把那个重启脚本发给我\nAssistant: [clarification_requested]\n[/LAST_TURN_FULL]"),
            None,
            None,
            None,
        );
        match merged {
            crate::intent::continuation_resolver::ClarifyFollowupResolution::NormalizerRewrite {
                rewritten_prompt,
            } => {
                assert!(rewritten_prompt.contains("把那个重启脚本发给我"));
                assert!(rewritten_prompt.contains("就在 scripts/restart_clawd_latest.sh"));
            }
            other => panic!("expected normalizer rewrite, got {other:?}"),
        }
    }

    #[test]
    fn clarify_followup_routing_prompt_skips_unrelated_new_request() {
        assert!(matches!(
            crate::intent::continuation_resolver::resolve_clarify_followup(
                "今天天气怎么样",
                Some(
                    "[LAST_TURN_FULL]\nUser: 把那个 JSON 数组按 score 排一下并转成表格\nAssistant: [clarification_requested]\n[/LAST_TURN_FULL]"
                ),
                None,
                None,
                None,
            ),
            crate::intent::continuation_resolver::ClarifyFollowupResolution::None
        ));
    }

    #[test]
    fn task_append_merge_reuses_prior_primary_task_prompt() {
        let merged = merged_prompt_from_task_turn_analysis(
            Some("帮我写个方案"),
            None,
            "面向老板",
            Some(&crate::intent_router::TurnAnalysis {
                turn_type: Some(crate::intent_router::TurnType::TaskAppend),
                target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
                should_interrupt_active_run: false,
                state_patch: Some(json!({"audience":"boss"})),
                attachment_processing_required: false,
            }),
        )
        .expect("merged prompt");
        assert!(merged.contains("帮我写个方案"));
        assert!(merged.contains("面向老板"));
        assert!(merged.contains("\"audience\":\"boss\""));
        assert!(merged.contains("append this new instruction"));
        assert!(merged.contains("Continuity rules"));
        assert!(merged.contains("do not repeat the same clarification indefinitely"));
    }

    #[test]
    fn task_replace_merge_discards_prior_goal() {
        let merged = merged_prompt_from_task_turn_analysis(
            Some("别写长文，先做方案"),
            None,
            "算了，改成 X thread",
            Some(&crate::intent_router::TurnAnalysis {
                turn_type: Some(crate::intent_router::TurnType::TaskReplace),
                target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReplaceActive),
                should_interrupt_active_run: false,
                state_patch: Some(json!({"deliverable":"thread"})),
                attachment_processing_required: false,
            }),
        )
        .expect("merged prompt");
        assert!(merged.contains("别写长文，先做方案"));
        assert!(merged.contains("算了，改成 X thread"));
        assert!(merged.contains("\"deliverable\":\"thread\""));
        assert!(merged.contains("replace it with this new goal"));
    }

    #[test]
    fn task_correct_merge_marks_conflicting_details_as_overrides() {
        let merged = merged_prompt_from_task_turn_analysis(
            Some("帮我写安装说明，面向 Python 3.10"),
            None,
            "不对，不是 Python 3.10，是 Python 3.11",
            Some(&crate::intent_router::TurnAnalysis {
                turn_type: Some(crate::intent_router::TurnType::TaskCorrect),
                target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
                should_interrupt_active_run: false,
                state_patch: Some(json!({"python_version":"3.11"})),
                attachment_processing_required: false,
            }),
        )
        .expect("merged prompt");
        assert!(merged.contains("Python 3.10"));
        assert!(merged.contains("Python 3.11"));
        assert!(merged.contains("\"python_version\":\"3.11\""));
        assert!(merged.contains("overrides conflicting earlier details"));
    }

    #[test]
    fn task_append_merge_includes_recent_generated_output_when_normalizer_reuses_active() {
        let merged = merged_prompt_from_task_turn_analysis(
            Some("Write one deployment note that mentions Python 3.11"),
            Some("Deployment note: use Python 3.11."),
            "Output only that sentence",
            Some(&crate::intent_router::TurnAnalysis {
                turn_type: Some(crate::intent_router::TurnType::TaskAppend),
                target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
                should_interrupt_active_run: false,
                state_patch: None,
                attachment_processing_required: false,
            }),
        )
        .expect("merged prompt");
        assert!(merged.contains("Most recent generated output"));
        assert!(merged.contains("Deployment note: use Python 3.11."));
    }
}
