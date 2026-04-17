use anyhow::Result;
use serde_json::Value;
use tracing::info;

use super::*;

pub(super) struct PreparedAskFlow {
    pub(super) context_bundle_summary: String,
    pub(super) route_result: crate::RouteResult,
    pub(super) execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    pub(super) auto_locator_path: Option<String>,
    pub(super) chat_prompt_context: String,
    pub(super) resolved_prompt_for_execution: String,
    pub(super) prompt_with_memory_for_execution: String,
    pub(super) recent_execution_context: String,
    pub(super) agent_mode: bool,
    pub(super) direct_resume_execution: bool,
    pub(super) direct_resume_discussion: bool,
    pub(super) classifier_direct_mode: bool,
    /// Phase 3.2 Stage C4：从 PreparedAskRouting.ask_mode 复制而来，
    /// 与上面 3 个 bool flag 双轨。dispatch 内部读这个字段做分支决策，
    /// Stage D 删除旧 bool。
    pub(super) ask_mode: crate::AskMode,
    pub(super) clarify_reason: String,
    pub(super) clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind,
    pub(super) fuzzy_locator_suggestions: Vec<String>,
    pub(super) should_route_schedule_direct: bool,
}

struct AppliedAskPostRoute {
    execution_route_result: crate::RouteResult,
    auto_locator_path: Option<String>,
    resolved_prompt_for_execution: String,
    prompt_with_memory_for_execution: String,
    clarify_reason: String,
    clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind,
    fuzzy_locator_suggestions: Vec<String>,
}

fn should_allow_classifier_direct(route_result: &crate::RouteResult) -> bool {
    !route_result.ask_mode.is_act()
}

fn normalize_brief_route_text(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clarify_reason_allows_route_question_reuse(
    clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind,
) -> bool {
    matches!(
        clarify_reason_kind,
        crate::post_route_policy::ClarifyReasonKind::RouteReasonText
    )
}

fn resolved_intent_inherits_prior_operation(prompt: &str, resolved_prompt: &str) -> bool {
    let prompt_norm = normalize_brief_route_text(prompt.trim());
    let resolved_norm = normalize_brief_route_text(resolved_prompt.trim());
    !prompt_norm.is_empty()
        && !resolved_norm.is_empty()
        && prompt_norm != resolved_norm
        && resolved_norm.len() > prompt_norm.len()
}

fn should_preserve_original_inline_structured_input(
    prompt: &str,
    resolved_prompt_for_execution: &str,
) -> bool {
    let prompt_trimmed = prompt.trim();
    let resolved_trimmed = resolved_prompt_for_execution.trim();
    if prompt_trimmed.is_empty()
        || resolved_trimmed.is_empty()
        || prompt_trimmed == resolved_trimmed
    {
        return false;
    }
    let Some(inline_value) = crate::extract_first_json_value_any(prompt_trimmed) else {
        return false;
    };
    !resolved_trimmed.contains(inline_value.trim())
}

fn execution_user_request<'a>(prompt: &'a str, resolved_prompt_for_execution: &'a str) -> &'a str {
    if should_preserve_original_inline_structured_input(prompt, resolved_prompt_for_execution) {
        prompt
    } else {
        resolved_prompt_for_execution
    }
}

fn build_locator_fuzzy_clarify_context(
    recent_execution_context: &str,
    fuzzy_locator_suggestions: &[String],
    include_recent_execution_context: bool,
) -> String {
    if fuzzy_locator_suggestions.is_empty() {
        return if include_recent_execution_context {
            recent_execution_context.to_string()
        } else {
            "<none>".to_string()
        };
    }
    let candidate_block = fuzzy_locator_suggestions
        .iter()
        .map(|v| format!("- {v}"))
        .collect::<Vec<_>>()
        .join("\n");
    let fuzzy_notice = "Exact target was not found. The following are only similar locator candidates for confirmation; they are not confirmed matches to the requested file.";
    if !include_recent_execution_context
        || recent_execution_context.trim().is_empty()
        || recent_execution_context.trim() == "<none>"
    {
        format!("### LOCATOR_FUZZY_CANDIDATES\n{fuzzy_notice}\n{candidate_block}\n")
    } else {
        format!(
            "{}\n\n### LOCATOR_FUZZY_CANDIDATES\n{}\n{}\n",
            recent_execution_context, fuzzy_notice, candidate_block
        )
    }
}

fn should_suppress_recent_execution_in_clarify_context(
    prompt: &str,
    route_result: &crate::RouteResult,
    fuzzy_locator_suggestions: &[String],
) -> bool {
    fuzzy_locator_suggestions.is_empty()
        && route_result.needs_clarify
        && matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::Url
        )
        && !super::has_concrete_locator_hint(prompt)
}

fn should_reuse_route_clarify_question(
    prompt: &str,
    route_result: &crate::RouteResult,
    clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind,
    fuzzy_locator_suggestions: &[String],
) -> bool {
    !should_suppress_recent_execution_in_clarify_context(
        prompt,
        route_result,
        fuzzy_locator_suggestions,
    ) && fuzzy_locator_suggestions.is_empty()
        && clarify_reason_allows_route_question_reuse(clarify_reason_kind)
}

fn structured_missing_locator_clarify_question(
    state: &AppState,
    route_result: &crate::RouteResult,
) -> Option<String> {
    if !route_result.needs_clarify || !route_result.output_contract.locator_hint.trim().is_empty() {
        return None;
    }
    if route_result.output_contract.delivery_required {
        return Some(crate::i18n_t_with_default(
            state,
            "clawd.msg.clarify_missing_file_locator",
            "Please provide the specific file name or path.",
        ));
    }
    if route_result.output_contract.requires_content_evidence
        && matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
        )
    {
        return Some(crate::i18n_t_with_default(
            state,
            "clawd.msg.clarify_missing_read_target",
            "Please provide the specific file name or path to read.",
        ));
    }
    None
}

fn should_short_circuit_structured_clarify(route_result: &crate::RouteResult) -> bool {
    route_result.needs_clarify
        && route_result.output_contract.locator_hint.trim().is_empty()
        && route_result.output_contract.requires_content_evidence
        && matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
        )
}

fn apply_ask_post_route(
    state: &AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    resolved_prompt: &str,
    recent_execution_context: &str,
    immediate_prior_turn_was_clarify: bool,
    route_result: crate::RouteResult,
    mut resolved_prompt_for_execution: String,
    mut prompt_with_memory_for_execution: String,
) -> AppliedAskPostRoute {
    let locator_resolution = if should_attempt_auto_locator(&route_result) {
        let locator_from_hint = if route_result.output_contract.locator_hint.trim().is_empty() {
            None
        } else {
            super::try_resolve_implicit_locator_path(
                state,
                route_result.output_contract.locator_hint.trim(),
                route_result.output_contract.locator_hint.trim(),
                route_result.output_contract.locator_kind,
                Some(recent_execution_context),
            )
        };
        match locator_from_hint.or_else(|| {
            super::try_resolve_implicit_locator_path(
                state,
                prompt,
                resolved_prompt,
                route_result.output_contract.locator_kind,
                Some(recent_execution_context),
            )
        }) {
            Some(super::LocatorAutoResolution::Direct(path)) => {
                crate::post_route_policy::LocatorResolution::Direct(path)
            }
            Some(super::LocatorAutoResolution::Fuzzy(candidates)) => {
                crate::post_route_policy::LocatorResolution::Fuzzy(candidates)
            }
            None => crate::post_route_policy::LocatorResolution::None,
        }
    } else {
        crate::post_route_policy::LocatorResolution::None
    };
    let post_route = crate::post_route_policy::apply_post_route_policy(
        route_result.clone(),
        super::has_concrete_locator_hint(prompt),
        super::has_concrete_locator_hint(resolved_prompt),
        super::has_explicit_path_or_url_locator_hint(prompt),
        super::has_explicit_path_or_url_locator_hint(resolved_prompt),
        resolved_intent_inherits_prior_operation(prompt, resolved_prompt),
        immediate_prior_turn_was_clarify,
        locator_resolution,
    );
    if let Some(hint) = post_route.auto_locator_hint.as_deref() {
        resolved_prompt_for_execution.push_str(hint);
        prompt_with_memory_for_execution.push_str(hint);
    }
    if let Some(path) = post_route.auto_locator_path.as_deref() {
        info!(
            "{} worker_once: ask auto_locator_resolved task_id={} path={} raw_text={} resolved_text={}",
            crate::highlight_tag("routing"),
            task.task_id,
            path,
            crate::truncate_for_log(prompt),
            crate::truncate_for_log(resolved_prompt)
        );
    }
    if !post_route.fuzzy_locator_suggestions.is_empty() {
        info!(
            "{} worker_once: ask auto_locator_fuzzy_candidates task_id={} candidates={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(&post_route.fuzzy_locator_suggestions.join(" | "))
        );
    }
    if post_route.missing_locator_for_path_scoped_content {
        info!(
            "{} worker_once: ask force_clarify_by_locator_guard task_id={} reason=locator_required_for_path_scoped_content raw_text={} resolved_text={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(prompt),
            crate::truncate_for_log(resolved_prompt)
        );
    }
    if post_route.execution_route_result.routed_mode != route_result.routed_mode {
        info!(
            "{} worker_once: ask routed_mode_override_by_auto_locator task_id={} mode={:?}->{:?}",
            crate::highlight_tag("routing"),
            task.task_id,
            route_result.routed_mode,
            post_route.execution_route_result.routed_mode
        );
    }
    AppliedAskPostRoute {
        execution_route_result: post_route.execution_route_result,
        auto_locator_path: post_route.auto_locator_path,
        resolved_prompt_for_execution,
        prompt_with_memory_for_execution,
        clarify_reason: post_route.clarify_reason,
        clarify_reason_kind: post_route.clarify_reason_kind,
        fuzzy_locator_suggestions: post_route.fuzzy_locator_suggestions,
    }
}

fn should_attempt_auto_locator(route_result: &crate::RouteResult) -> bool {
    matches!(
        route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
            | crate::OutputLocatorKind::CurrentWorkspace
            | crate::OutputLocatorKind::Filename
    )
}

pub(super) async fn prepare_ask_flow(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
    source: &str,
) -> Result<PreparedAskFlow> {
    let prepared_routing = super::prepare_ask_routing(state, task, payload, prompt, source).await;
    let prepared_execution = super::prepare_ask_execution_context(
        state,
        task,
        payload,
        &prepared_routing.resolved_prompt,
    )
    .await?;
    let applied_post_route = apply_ask_post_route(
        state,
        task,
        prompt,
        &prepared_routing.resolved_prompt,
        &prepared_execution.recent_execution_context,
        prepared_routing.immediate_prior_turn_was_clarify,
        prepared_routing.route_result,
        prepared_execution.resolved_prompt_for_execution,
        prepared_execution.prompt_with_memory_for_execution,
    );
    let has_schedule_intent =
        applied_post_route.execution_route_result.schedule_kind != crate::ScheduleKind::None;
    let should_route_schedule_direct = has_schedule_intent
        && !prepared_routing.direct_resume_execution
        && !prepared_routing.direct_resume_discussion;
    Ok(PreparedAskFlow {
        context_bundle_summary: prepared_execution.context_bundle.summary(),
        route_result: applied_post_route.execution_route_result,
        execution_recipe_hint: prepared_routing.execution_recipe_hint,
        auto_locator_path: applied_post_route.auto_locator_path,
        chat_prompt_context: prepared_execution.chat_prompt_context,
        resolved_prompt_for_execution: applied_post_route.resolved_prompt_for_execution,
        prompt_with_memory_for_execution: applied_post_route.prompt_with_memory_for_execution,
        recent_execution_context: prepared_execution.recent_execution_context,
        agent_mode: prepared_routing.agent_mode,
        direct_resume_execution: prepared_routing.direct_resume_execution,
        direct_resume_discussion: prepared_routing.direct_resume_discussion,
        classifier_direct_mode: prepared_routing.classifier_direct_mode,
        ask_mode: prepared_routing.ask_mode.clone(),
        clarify_reason: applied_post_route.clarify_reason,
        clarify_reason_kind: applied_post_route.clarify_reason_kind,
        fuzzy_locator_suggestions: applied_post_route.fuzzy_locator_suggestions,
        should_route_schedule_direct,
    })
}

pub(super) async fn execute_ask_dispatch(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
    recent_execution_context: &str,
    resolved_prompt_for_execution: &str,
    prompt_with_memory_for_execution: &str,
    chat_prompt_context: &str,
    route_result: &crate::RouteResult,
    agent_mode: bool,
    clarify_reason: &str,
    clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind,
    fuzzy_locator_suggestions: &[String],
    classifier_direct_mode: bool,
    direct_resume_discussion: bool,
    direct_resume_execution: bool,
    ask_mode: &crate::AskMode,
    should_route_schedule_direct: bool,
    agent_run_context: Option<crate::agent_engine::AgentRunContext>,
) -> Result<Option<Result<crate::AskReply, String>>> {
    let execution_user_request = execution_user_request(prompt, resolved_prompt_for_execution);
    debug_assert_eq!(
        ask_mode.is_resume_discussion(),
        direct_resume_discussion,
        "ask_mode/resume_discussion drift"
    );
    debug_assert_eq!(
        ask_mode.resume_execution(),
        direct_resume_execution,
        "ask_mode/resume_execution drift"
    );
    debug_assert_eq!(
        ask_mode.is_classifier_direct(),
        classifier_direct_mode,
        "ask_mode/classifier_direct drift"
    );
    if route_result.ask_mode.is_clarify_only() {
        let suppress_recent_execution_context = should_suppress_recent_execution_in_clarify_context(
            prompt,
            route_result,
            fuzzy_locator_suggestions,
        );
        let clarify_context = build_locator_fuzzy_clarify_context(
            recent_execution_context,
            fuzzy_locator_suggestions,
            !suppress_recent_execution_context,
        );
        let structured_clarify_question =
            structured_missing_locator_clarify_question(state, route_result);
        if should_short_circuit_structured_clarify(route_result) {
            if let Some(clarify) = structured_clarify_question {
                return Ok(Some(Ok(crate::AskReply::non_llm(clarify))));
            }
        }
        let preferred_clarify_question = structured_clarify_question.as_deref().or_else(|| {
            if should_reuse_route_clarify_question(
                prompt,
                route_result,
                clarify_reason_kind,
                fuzzy_locator_suggestions,
            ) {
                let route_question = route_result.clarify_question.trim();
                (!route_question.is_empty()).then_some(route_question)
            } else {
                None
            }
        });
        let clarify_policy = if preferred_clarify_question.is_none()
            && route_result.clarify_question.trim().is_empty()
            && !matches!(
                clarify_reason_kind,
                crate::post_route_policy::ClarifyReasonKind::FuzzyLocatorCandidates
            ) {
            crate::intent_router::ClarifyQuestionPolicy::SafeFallback
        } else {
            crate::intent_router::ClarifyQuestionPolicy::AllowModel
        };
        let clarify = crate::intent_router::generate_or_reuse_clarify_question(
            state,
            task,
            prompt,
            clarify_reason,
            Some(&clarify_context),
            preferred_clarify_question,
            clarify_policy,
        )
        .await;
        return Ok(Some(Ok(crate::AskReply::non_llm(clarify))));
    }
    if ask_mode.is_resume_discussion() {
        let resume_prompt_source = crate::resolve_prompt_rel_path_for_vendor(
            &state.skill_rt.workspace_root,
            &crate::active_prompt_vendor_name(state),
            crate::RESUME_FOLLOWUP_DISCUSSION_PROMPT_LOGICAL_PATH,
        );
        crate::log_prompt_render(
            state,
            &task.task_id,
            "resume_followup_discussion_prompt",
            &resume_prompt_source,
            None,
        );
        let reply = crate::llm_gateway::run_with_fallback_with_prompt_source(
            state,
            task,
            resolved_prompt_for_execution,
            &resume_prompt_source,
        )
        .await
        .map(|s| crate::AskReply::llm(s.trim().to_string()));
        return Ok(Some(reply));
    }
    if ask_mode.resume_execution() {
        return Ok(Some(
            crate::agent_engine::run_agent_with_tools(
                state,
                task,
                prompt_with_memory_for_execution,
                execution_user_request,
                agent_run_context.clone(),
            )
            .await,
        ));
    }
    if should_route_schedule_direct {
        if super::try_finalize_schedule_direct_success(
            state,
            task,
            payload,
            prompt,
            resolved_prompt_for_execution,
            route_result,
        )
        .await?
        {
            return Ok(None);
        }
        if ask_mode.is_classifier_direct()
            && !ask_mode.is_resume_discussion()
            && should_allow_classifier_direct(route_result)
        {
            return Ok(Some(
                super::run_classifier_direct_reply(state, task, resolved_prompt_for_execution)
                    .await,
            ));
        }
        return Ok(Some(
            crate::execute_ask_routed(
                state,
                task,
                chat_prompt_context,
                prompt_with_memory_for_execution,
                resolved_prompt_for_execution,
                execution_user_request,
                agent_mode,
                direct_resume_discussion,
                Some(route_result.routed_mode),
                agent_run_context.clone(),
            )
            .await,
        ));
    }
    if ask_mode.is_classifier_direct() && should_allow_classifier_direct(route_result) {
        return Ok(Some(
            super::run_classifier_direct_reply(state, task, resolved_prompt_for_execution).await,
        ));
    }
    Ok(Some(
        crate::execute_ask_routed(
            state,
            task,
            chat_prompt_context,
            prompt_with_memory_for_execution,
            resolved_prompt_for_execution,
            execution_user_request,
            agent_mode,
            false,
            Some(route_result.routed_mode),
            agent_run_context,
        )
        .await,
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        execution_user_request, resolved_intent_inherits_prior_operation,
        should_allow_classifier_direct, should_attempt_auto_locator,
        should_preserve_original_inline_structured_input, should_reuse_route_clarify_question,
        should_short_circuit_structured_clarify,
        should_suppress_recent_execution_in_clarify_context,
    };

    #[test]
    fn auto_locator_attempts_for_path_locators_even_without_content_evidence() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "读取 Cargo.toml".to_string(),
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
            output_contract: crate::IntentOutputContract {
                locator_kind: crate::OutputLocatorKind::Path,
                requires_content_evidence: false,
                ..Default::default()
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        assert!(should_attempt_auto_locator(&route));
    }

    #[test]
    fn auto_locator_skips_non_path_locators() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "今天天气".to_string(),
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
            output_contract: crate::IntentOutputContract {
                locator_kind: crate::OutputLocatorKind::None,
                requires_content_evidence: false,
                ..Default::default()
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        assert!(!should_attempt_auto_locator(&route));
    }

    #[test]
    fn auto_locator_attempts_for_current_workspace_locator() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "检查当前目录是否存在隐藏文件".to_string(),
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
            output_contract: crate::IntentOutputContract {
                locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
                requires_content_evidence: false,
                ..Default::default()
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        assert!(should_attempt_auto_locator(&route));
    }

    #[test]
    fn auto_locator_attempts_for_filename_locators() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "读取 README 前 20 行".to_string(),
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
            output_contract: crate::IntentOutputContract {
                locator_kind: crate::OutputLocatorKind::Filename,
                requires_content_evidence: true,
                ..Default::default()
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        assert!(should_attempt_auto_locator(&route));
    }

    #[test]
    fn classifier_direct_disabled_for_act_mode() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "读取 Cargo.toml".to_string(),
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
        assert!(!should_allow_classifier_direct(&route));
    }

    #[test]
    fn classifier_direct_disabled_for_chat_act_mode() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "比较 Cargo.toml 和 Cargo.lock".to_string(),
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
        assert!(!should_allow_classifier_direct(&route));
    }

    #[test]
    fn classifier_direct_kept_for_chat_mode() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Chat,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Chat),
            resolved_intent: "解释一下这个概念".to_string(),
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
        assert!(should_allow_classifier_direct(&route));
    }

    #[test]
    fn inherited_operation_detects_locator_handoff_rewrite() {
        assert!(resolved_intent_inherits_prior_operation(
            "document",
            "list the contents of the directory named 'document'"
        ));
    }

    #[test]
    fn inherited_operation_ignores_equivalent_prompt() {
        assert!(!resolved_intent_inherits_prior_operation(
            "读取 README 前 20 行",
            "读取 README 前 20 行"
        ));
    }

    #[test]
    fn inline_json_payload_prefers_original_user_request_for_execution() {
        let prompt = r#"把这个 JSON 数组按 score 从高到低排一下，再输出成 markdown 表格：[{"name":"alpha","score":7},{"name":"beta","score":12},{"name":"gamma","score":9}]"#;
        let resolved =
            "Sort the provided JSON array by score in descending order and output as a markdown table";
        assert!(should_preserve_original_inline_structured_input(
            prompt, resolved
        ));
        assert_eq!(execution_user_request(prompt, resolved), prompt);
    }

    #[test]
    fn non_structured_prompt_keeps_resolved_execution_request() {
        let prompt = "帮我检查 telegramd 现在是不是在运行，顺手简短解释状态";
        let resolved = "检查 telegramd 进程当前是否在运行，并简要说明其状态";
        assert!(!should_preserve_original_inline_structured_input(
            prompt, resolved
        ));
        assert_eq!(execution_user_request(prompt, resolved), resolved);
    }

    fn clarify_route(locator_kind: crate::OutputLocatorKind) -> crate::RouteResult {
        crate::RouteResult {
            routed_mode: crate::RoutedMode::AskClarify,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::AskClarify),
            resolved_intent: "读取那个文件".to_string(),
            needs_clarify: true,
            route_reason: "need concrete locator".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: "你是指哪个文件？".to_string(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                locator_kind,
                requires_content_evidence: true,
                response_shape: crate::OutputResponseShape::Scalar,
                ..Default::default()
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        }
    }

    #[test]
    fn scalar_missing_locator_clarify_short_circuits_to_structured_prompt() {
        let route = clarify_route(crate::OutputLocatorKind::Path);
        assert!(should_short_circuit_structured_clarify(&route));
    }

    #[test]
    fn path_scoped_clarify_without_locator_suppresses_recent_execution_context() {
        let route = clarify_route(crate::OutputLocatorKind::Path);
        assert!(should_suppress_recent_execution_in_clarify_context(
            "把那个配置文件发给我",
            &route,
            &[],
        ));
    }

    #[test]
    fn filename_clarify_without_locator_does_not_reuse_router_question() {
        let route = clarify_route(crate::OutputLocatorKind::Filename);
        assert!(!should_reuse_route_clarify_question(
            "读一下那个文件里的名字字段，只输出值",
            &route,
            crate::post_route_policy::ClarifyReasonKind::MissingPathScopedLocator,
            &[],
        ));
    }

    #[test]
    fn route_reason_text_can_reuse_router_question_when_no_structured_override_exists() {
        let route = clarify_route(crate::OutputLocatorKind::Filename);
        assert!(should_reuse_route_clarify_question(
            "README.md",
            &route,
            crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
            &[],
        ));
    }

    #[test]
    fn clarify_with_fuzzy_candidates_keeps_recent_context_available() {
        let route = clarify_route(crate::OutputLocatorKind::Filename);
        assert!(!should_suppress_recent_execution_in_clarify_context(
            "读一下那个文件里的名字字段，只输出值",
            &route,
            &["/tmp/a".to_string()],
        ));
    }
}
