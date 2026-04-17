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
    pub(super) resolved_prompt: String,
    pub(super) agent_mode: bool,
    pub(super) immediate_prior_turn_was_clarify: bool,
    /// Phase 3.2：合并 routed_mode + classifier_direct + direct_resume_*
    /// 后的最终模式。Stage D 已删除原 3 个 bool 字段（classifier_direct_mode /
    /// direct_resume_discussion / direct_resume_execution），全部判断走
    /// ask_mode 谓词方法（is_classifier_direct / is_resume_discussion /
    /// resume_execution）。
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

fn context_contains_immediate_locator_anchor(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed == "<none>" {
        return false;
    }
    if trimmed.starts_with("### RECENT_ASSISTANT_REPLIES") {
        let Some(immediate_line) = trimmed
            .lines()
            .find(|line| line.contains("turn_id=assistant[-1]"))
        else {
            return false;
        };
        return !crate::extract_delivery_file_tokens(immediate_line).is_empty()
            || crate::delivery_utils::has_concrete_locator_input(immediate_line);
    }
    !crate::extract_delivery_file_tokens(trimmed).is_empty()
        || crate::delivery_utils::has_concrete_locator_input(trimmed)
}

fn immediate_last_turn_was_clarify(text: &str) -> bool {
    text.contains("[clarification_requested]")
}

fn extract_last_turn_user_text(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix("User: "))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
}

fn prompt_is_inline_json_value(prompt: &str) -> bool {
    let trimmed = prompt.trim();
    !trimmed.is_empty()
        && crate::extract_first_json_value_any(trimmed).is_some_and(|value| value.trim() == trimmed)
}

fn prompt_looks_like_clarify_target_only(prompt: &str) -> bool {
    let trimmed = prompt.trim();
    !trimmed.is_empty()
        && (prompt_is_inline_json_value(trimmed)
            || super::has_concrete_locator_hint(trimmed)
            || super::has_explicit_path_or_url_locator_hint(trimmed))
}

fn clarify_followup_routing_prompt(prompt: &str, last_turn_full: &str) -> Option<String> {
    if !immediate_last_turn_was_clarify(last_turn_full)
        || !prompt_looks_like_clarify_target_only(prompt)
    {
        return None;
    }
    let previous_user_request = extract_last_turn_user_text(last_turn_full)?;
    Some(format!(
        "Continue the previous request that was waiting for clarification: {}\nUser now provides the missing target/content: {}",
        previous_user_request.trim(),
        prompt.trim()
    ))
}

fn should_force_clarify_for_fresh_delivery_deictic(
    route_result: &crate::RouteResult,
    prompt: &str,
    last_turn_full: &str,
    recent_assistant_replies: &str,
) -> bool {
    route_result.wants_file_delivery
        && !route_result.needs_clarify
        && !super::has_concrete_locator_hint(prompt)
        && !context_contains_immediate_locator_anchor(last_turn_full)
        && !context_contains_immediate_locator_anchor(recent_assistant_replies)
}

fn should_force_clarify_for_fresh_scalar_deictic(
    route_result: &crate::RouteResult,
    prompt: &str,
    last_turn_full: &str,
    recent_assistant_replies: &str,
) -> bool {
    !route_result.needs_clarify
        && route_result.output_contract.requires_content_evidence
        && matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
        )
        && matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::Url
        )
        && !super::has_concrete_locator_hint(prompt)
        && !context_contains_immediate_locator_anchor(last_turn_full)
        && !context_contains_immediate_locator_anchor(recent_assistant_replies)
}

fn should_force_clarify_for_fresh_content_deictic(
    route_result: &crate::RouteResult,
    prompt: &str,
    last_turn_full: &str,
    recent_assistant_replies: &str,
) -> bool {
    !route_result.needs_clarify
        && route_result.output_contract.requires_content_evidence
        && matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
        )
        && matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::Url
        )
        && !super::has_concrete_locator_hint(prompt)
        && !context_contains_immediate_locator_anchor(last_turn_full)
        && !context_contains_immediate_locator_anchor(recent_assistant_replies)
}

fn apply_fresh_delivery_deictic_clarify_guard(
    state: &AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
    last_turn_full: &str,
    recent_assistant_replies: &str,
) {
    if !should_force_clarify_for_fresh_delivery_deictic(
        route_result,
        prompt,
        last_turn_full,
        recent_assistant_replies,
    ) {
        return;
    }
    route_result.routed_mode = crate::RoutedMode::AskClarify;
    route_result.needs_clarify = true;
    route_result.resolved_intent = prompt.trim().to_string();
    route_result.clarify_question = crate::i18n_t_with_default(
        state,
        "clawd.msg.clarify_missing_file_locator",
        "Please provide the specific file name or path.",
    );
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route_result.output_contract.locator_hint.clear();
    if route_result.route_reason.trim().is_empty() {
        route_result.route_reason = "fresh_delivery_deictic_requires_locator".to_string();
    } else if !route_result
        .route_reason
        .contains("fresh_delivery_deictic_requires_locator")
    {
        route_result
            .route_reason
            .push_str(";fresh_delivery_deictic_requires_locator");
    }
}

fn apply_fresh_scalar_deictic_clarify_guard(
    state: &AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
    last_turn_full: &str,
    recent_assistant_replies: &str,
) {
    if !should_force_clarify_for_fresh_scalar_deictic(
        route_result,
        prompt,
        last_turn_full,
        recent_assistant_replies,
    ) {
        return;
    }
    route_result.routed_mode = crate::RoutedMode::AskClarify;
    route_result.needs_clarify = true;
    route_result.resolved_intent = prompt.trim().to_string();
    route_result.clarify_question = crate::i18n_t_with_default(
        state,
        "clawd.msg.clarify_missing_read_target",
        "Please provide the specific file name or path to read.",
    );
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route_result.output_contract.locator_hint.clear();
    if route_result.route_reason.trim().is_empty() {
        route_result.route_reason = "fresh_scalar_deictic_requires_locator".to_string();
    } else if !route_result
        .route_reason
        .contains("fresh_scalar_deictic_requires_locator")
    {
        route_result
            .route_reason
            .push_str(";fresh_scalar_deictic_requires_locator");
    }
}

fn apply_fresh_content_deictic_clarify_guard(
    state: &AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
    last_turn_full: &str,
    recent_assistant_replies: &str,
) {
    if !should_force_clarify_for_fresh_content_deictic(
        route_result,
        prompt,
        last_turn_full,
        recent_assistant_replies,
    ) {
        return;
    }
    route_result.routed_mode = crate::RoutedMode::AskClarify;
    route_result.needs_clarify = true;
    route_result.resolved_intent = prompt.trim().to_string();
    route_result.clarify_question = crate::i18n_t_with_default(
        state,
        "clawd.msg.clarify_missing_read_target",
        "Please provide the specific file name or path to read.",
    );
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route_result.output_contract.locator_hint.clear();
    if route_result.route_reason.trim().is_empty() {
        route_result.route_reason = "fresh_content_deictic_requires_locator".to_string();
    } else if !route_result
        .route_reason
        .contains("fresh_content_deictic_requires_locator")
    {
        route_result
            .route_reason
            .push_str(";fresh_content_deictic_requires_locator");
    }
}

fn direct_classifier_route_result(prompt: &str) -> crate::RouteResult {
    crate::RouteResult {
        routed_mode: crate::RoutedMode::Chat,
        ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Chat),
        resolved_intent: prompt.trim().to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "classifier_direct_source".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
        direct_reply_candidate: String::new(),
        direct_reply_confidence: 0.0,
    }
}

#[derive(Clone)]
enum ResumeContextSource {
    ExplicitContinue,
    RecentFailedCandidate,
}

#[derive(Clone)]
struct ResumeContextBinding {
    source: ResumeContextSource,
    resume_context: Value,
    failed_ts: Option<i64>,
    has_newer_successful_ask_after_failed_task: bool,
}

fn explicit_resume_context_binding(
    payload: &Value,
    is_resume_continue: bool,
) -> Option<ResumeContextBinding> {
    if !is_resume_continue {
        return None;
    }
    Some(ResumeContextBinding {
        source: ResumeContextSource::ExplicitContinue,
        resume_context: payload.get("resume_context").cloned()?,
        failed_ts: payload
            .get("failed_resume_context_ts")
            .and_then(|v| v.as_i64()),
        has_newer_successful_ask_after_failed_task: payload
            .get("has_newer_successful_ask_after_failed_task")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

fn recent_failed_resume_candidate(
    state: &AppState,
    task: &crate::ClaimedTask,
    explicit_binding_present: bool,
) -> Option<ResumeContextBinding> {
    if explicit_binding_present {
        return None;
    }
    let candidate = crate::find_recent_failed_resume_context(state, task.user_id, task.chat_id)?;
    Some(ResumeContextBinding {
        source: ResumeContextSource::RecentFailedCandidate,
        resume_context: candidate.resume_context,
        failed_ts: Some(candidate.failed_ts),
        has_newer_successful_ask_after_failed_task: candidate
            .has_newer_successful_ask_after_failed_task,
    })
}

fn binding_context_json(
    source: &str,
    is_resume_continue: bool,
    resume_binding: Option<&ResumeContextBinding>,
) -> Value {
    let (
        resume_context_source,
        failed_resume_context_ts,
        has_newer_successful_ask_after_failed_task,
    ) = match resume_binding {
        Some(binding) => (
            match binding.source {
                ResumeContextSource::ExplicitContinue => "explicit_continue_source",
                ResumeContextSource::RecentFailedCandidate => "recent_failed_resume_candidate",
            },
            binding.failed_ts.map(Value::from).unwrap_or(Value::Null),
            binding.has_newer_successful_ask_after_failed_task,
        ),
        None => ("none", Value::Null, false),
    };
    json!({
        "source": source.trim(),
        "is_resume_continue_source": is_resume_continue,
        "resume_context_source": resume_context_source,
        "failed_resume_context_ts": failed_resume_context_ts,
        "has_newer_successful_ask_after_failed_task": has_newer_successful_ask_after_failed_task,
    })
}

fn select_resume_runtime_binding<'a>(
    route_result: &crate::RouteResult,
    resume_binding: Option<&'a ResumeContextBinding>,
) -> Option<&'a ResumeContextBinding> {
    (!matches!(route_result.resume_behavior, crate::ResumeBehavior::None))
        .then_some(resume_binding)
        .flatten()
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
    resolved_prompt: &str,
) -> anyhow::Result<PreparedAskExecutionContext> {
    let chat_memory_budget_chars =
        crate::dynamic_chat_memory_budget_chars(state, task, resolved_prompt);
    let mut context_bundle = crate::task_context_builder::build_execution_task_context_bundle(
        state,
        task,
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
    super::finalize_ask_direct_success(
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
    let normalized_source = source.trim().to_ascii_lowercase();
    let classifier_direct_mode = crate::CLASSIFIER_DIRECT_SOURCES
        .iter()
        .any(|value| *value == normalized_source);
    let agent_mode = payload
        .get("agent_mode")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if classifier_direct_mode {
        info!(
            "{} worker_once: ask classifier_direct_route_bypass task_id={} source={} normalizer_skipped=true",
            crate::highlight_tag("routing"),
            task.task_id,
            source
        );
        let direct_route = direct_classifier_route_result(prompt);
        let ask_mode = crate::AskMode::from_legacy(
            direct_route.routed_mode,
            true,
            false,
            false,
            Some(&normalized_source),
        );
        return PreparedAskRouting {
            route_result: direct_route,
            execution_recipe_hint: None,
            resolved_prompt: prompt.trim().to_string(),
            agent_mode,
            immediate_prior_turn_was_clarify: false,
            ask_mode,
        };
    }
    let is_resume_continue = super::is_resume_continue_source(source);
    let (now_iso, timezone_str, schedule_rules) =
        schedule_service::schedule_context_for_normalizer(state);
    let last_turn_full = crate::memory::build_last_turn_full_context(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        400,
        1200,
    );
    let recent_assistant_replies = crate::memory::build_recent_assistant_replies_context(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        3,
        220,
    );
    let immediate_prior_turn_was_clarify = immediate_last_turn_was_clarify(&last_turn_full);
    let normalizer_prompt = clarify_followup_routing_prompt(prompt, &last_turn_full)
        .unwrap_or_else(|| prompt.to_string());
    let explicit_resume_binding = explicit_resume_context_binding(payload, is_resume_continue);
    let recent_failed_resume_binding =
        recent_failed_resume_candidate(state, task, explicit_resume_binding.is_some());
    let resume_binding = explicit_resume_binding
        .clone()
        .or_else(|| recent_failed_resume_binding.clone());
    let binding_context_value =
        binding_context_json(source, is_resume_continue, resume_binding.as_ref());
    let normalizer_out = crate::intent_router::run_intent_normalizer(
        state,
        task,
        &normalizer_prompt,
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
    let mut execution_recipe_hint = normalizer_out.execution_recipe_hint;
    let mut route_result =
        crate::intent_router::route_result_from_normalizer(state, task, &normalizer_out);
    apply_fresh_delivery_deictic_clarify_guard(
        state,
        prompt,
        &mut route_result,
        &last_turn_full,
        &recent_assistant_replies,
    );
    apply_fresh_scalar_deictic_clarify_guard(
        state,
        prompt,
        &mut route_result,
        &last_turn_full,
        &recent_assistant_replies,
    );
    apply_fresh_content_deictic_clarify_guard(
        state,
        prompt,
        &mut route_result,
        &last_turn_full,
        &recent_assistant_replies,
    );
    let resume_runtime_binding =
        select_resume_runtime_binding(&route_result, resume_binding.as_ref());
    let resume_should_apply_context = resume_runtime_binding.is_some()
        && route_result.resume_behavior == crate::ResumeBehavior::ResumeExecute;
    let resume_should_discuss_context = resume_runtime_binding.is_some()
        && route_result.resume_behavior == crate::ResumeBehavior::ResumeDiscuss;
    info!(
        "worker_once: ask raw_message task_id={} user_id={} chat_id={} text={}",
        task.task_id,
        task.user_id,
        task.chat_id,
        crate::truncate_for_log(prompt)
    );
    let runtime_prompt = if resume_should_apply_context {
        match resume_runtime_binding {
            Some(ResumeContextBinding {
                source: ResumeContextSource::ExplicitContinue,
                ..
            }) => crate::build_resume_continue_execute_prompt(state, payload, prompt),
            Some(binding) => crate::ask_flow::build_resume_continue_execute_prompt_from_context(
                state,
                prompt,
                &binding.resume_context,
            ),
            None => route_result.resolved_intent.clone(),
        }
    } else if resume_should_discuss_context {
        match resume_runtime_binding {
            Some(ResumeContextBinding {
                source: ResumeContextSource::ExplicitContinue,
                ..
            }) => crate::build_resume_followup_discussion_prompt(state, payload, prompt),
            Some(binding) => crate::ask_flow::build_resume_followup_discussion_prompt_from_context(
                state,
                prompt,
                &binding.resume_context,
            ),
            None => route_result.resolved_intent.clone(),
        }
    } else {
        route_result.resolved_intent.clone()
    };
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
    if route_result.needs_clarify
        || !matches!(
            route_result.routed_mode,
            crate::RoutedMode::Act | crate::RoutedMode::ChatAct
        )
    {
        execution_recipe_hint = None;
    }
    info!(
        "{} worker_once: ask resolved_message task_id={} needs_clarify={} confidence={} reason={} resolved_text={}",
        crate::highlight_tag("routing"),
        task.task_id,
        context_resolution.needs_clarify,
        context_resolution.confidence.unwrap_or(-1.0),
        crate::truncate_for_log(&context_resolution.reason),
        crate::truncate_for_log(&resolved_prompt)
    );
    let ask_mode = crate::AskMode::from_legacy(
        route_result.routed_mode,
        classifier_direct_mode,
        resume_should_discuss_context,
        resume_should_apply_context,
        if classifier_direct_mode {
            Some(normalized_source.as_str())
        } else {
            None
        },
    );
    // 仅在没有任何 flag 主导时校验反向 round-trip；resume_continue/discussion/
    // classifier_direct 命中时 to_routed_mode 会做"语义等价但取值不同"的折叠
    // （比如 ResumeContinue → Act 即便原 routed_mode 是 ChatAct），不等于即合理。
    if !classifier_direct_mode && !resume_should_discuss_context && !resume_should_apply_context {
        debug_assert_eq!(
            ask_mode.to_routed_mode(),
            route_result.routed_mode,
            "ask_mode <-> routed_mode invariant broken when no flag dominates"
        );
    }
    PreparedAskRouting {
        route_result,
        execution_recipe_hint,
        resolved_prompt,
        agent_mode,
        immediate_prior_turn_was_clarify,
        ask_mode,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        binding_context_json, clarify_followup_routing_prompt,
        context_contains_immediate_locator_anchor, direct_classifier_route_result,
        immediate_last_turn_was_clarify, select_resume_runtime_binding,
        should_force_clarify_for_fresh_content_deictic,
        should_force_clarify_for_fresh_delivery_deictic,
        should_force_clarify_for_fresh_scalar_deictic, ResumeContextBinding, ResumeContextSource,
    };

    #[test]
    fn binding_context_marks_recent_failed_candidate_without_mutating_source() {
        let binding = ResumeContextBinding {
            source: ResumeContextSource::RecentFailedCandidate,
            resume_context: json!({"resume_context_id":"ctx-1"}),
            failed_ts: Some(42),
            has_newer_successful_ask_after_failed_task: true,
        };
        let value = binding_context_json("manual", false, Some(&binding));
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
        let binding = ResumeContextBinding {
            source: ResumeContextSource::RecentFailedCandidate,
            resume_context: json!({"resume_context_id":"ctx-2"}),
            failed_ts: Some(7),
            has_newer_successful_ask_after_failed_task: false,
        };
        assert!(select_resume_runtime_binding(&route, Some(&binding)).is_none());
    }

    #[test]
    fn direct_classifier_route_uses_chat_without_clarify() {
        let route = direct_classifier_route_result("detect voice mode");
        assert_eq!(route.routed_mode, crate::RoutedMode::Chat);
        assert!(!route.needs_clarify);
        assert_eq!(route.resolved_intent, "detect voice mode");
        assert_eq!(route.route_reason, "classifier_direct_source");
    }

    #[test]
    fn fresh_delivery_deictic_without_immediate_anchor_forces_clarify() {
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
        assert!(should_force_clarify_for_fresh_delivery_deictic(
            &route,
            "把那个文件发给我",
            "<none>",
            "<none>",
        ));
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
        assert!(context_contains_immediate_locator_anchor(
            "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] short_preview=README.md",
        ));
        assert!(!should_force_clarify_for_fresh_delivery_deictic(
            &route,
            "把那个文件发给我",
            "<none>",
            "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] short_preview=README.md",
        ));
    }

    #[test]
    fn immediate_locator_anchor_ignores_older_assistant_replies() {
        assert!(!context_contains_immediate_locator_anchor(
            "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] short_preview=好的，我来读取 has_code_block=false\n- turn_id=assistant[-2] short_preview=package.json has_code_block=false",
        ));
        assert!(context_contains_immediate_locator_anchor(
            "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] short_preview=package.json has_code_block=false\n- turn_id=assistant[-2] short_preview=README.md has_code_block=false",
        ));
    }

    #[test]
    fn fresh_scalar_deictic_without_immediate_anchor_forces_clarify() {
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
        assert!(should_force_clarify_for_fresh_scalar_deictic(
            &route,
            "读一下那个文件里的名字字段，只输出值",
            "<none>",
            "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] short_preview=好的，我来读取 has_code_block=false\n- turn_id=assistant[-2] short_preview=package.json has_code_block=false",
        ));
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
        assert!(!should_force_clarify_for_fresh_scalar_deictic(
            &route,
            "读一下那个文件里的名字字段，只输出值",
            "<none>",
            "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] short_preview=package.json has_code_block=false\n- turn_id=assistant[-2] short_preview=README.md has_code_block=false",
        ));
    }

    #[test]
    fn fresh_content_deictic_without_immediate_anchor_forces_clarify() {
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
        assert!(should_force_clarify_for_fresh_content_deictic(
            &route,
            "看看那个模型日志最后 5 行",
            "<none>",
            "<none>",
        ));
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
        assert!(!should_force_clarify_for_fresh_content_deictic(
            &route,
            "看看那个模型日志最后 5 行",
            "<none>",
            "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] short_preview=model_io.log has_code_block=false",
        ));
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
        assert!(!should_force_clarify_for_fresh_delivery_deictic(
            &route,
            "README.md",
            "<none>",
            "<none>",
        ));
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
        assert!(!should_force_clarify_for_fresh_delivery_deictic(
            &route,
            "把 README 发给我",
            "<none>",
            "<none>",
        ));
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
        assert!(!should_force_clarify_for_fresh_content_deictic(
            &route,
            "扫一眼 README 前 20 行，再提炼成 3 句话",
            "<none>",
            "<none>",
        ));
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
        assert!(!should_force_clarify_for_fresh_content_deictic(
            &route,
            "比较 Cargo.toml 和 Cargo.lock 哪个更大，顺手用一句通俗话解释原因",
            "<none>",
            "<none>",
        ));
    }

    #[test]
    fn immediate_last_turn_clarify_placeholder_is_detected() {
        assert!(immediate_last_turn_was_clarify(
            "### LAST_TURN_FULL\n[TURN -1]\nUser: 读一下那个文件里的名字字段，只输出值\nAssistant: [clarification_requested]\n[/TURN]"
        ));
        assert!(!immediate_last_turn_was_clarify(
            "### LAST_TURN_FULL\n[TURN -1]\nUser: 看看那个重启脚本在不在\nAssistant: 有，路径：scripts/restart_clawd_latest.sh\n[/TURN]"
        ));
    }

    #[test]
    fn clarify_followup_routing_prompt_merges_previous_operation_for_json_payload() {
        let merged = clarify_followup_routing_prompt(
            "[{\"name\":\"alpha\",\"score\":7}]",
            "[LAST_TURN_FULL]\nUser: 把那个 JSON 数组按 score 排一下并转成表格\nAssistant: [clarification_requested]\n[/LAST_TURN_FULL]",
        )
        .expect("merged clarify follow-up prompt");
        assert!(merged.contains("把那个 JSON 数组按 score 排一下并转成表格"));
        assert!(merged.contains("[{\"name\":\"alpha\",\"score\":7}]"));
    }

    #[test]
    fn clarify_followup_routing_prompt_skips_unrelated_new_request() {
        assert!(clarify_followup_routing_prompt(
            "今天天气怎么样",
            "[LAST_TURN_FULL]\nUser: 把那个 JSON 数组按 score 排一下并转成表格\nAssistant: [clarification_requested]\n[/LAST_TURN_FULL]",
        )
        .is_none());
    }
}
