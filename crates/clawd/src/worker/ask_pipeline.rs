use anyhow::Result;
use serde_json::Value;
use tracing::info;

use super::*;

pub(super) struct PreparedAskFlow {
    pub(super) context_bundle_summary: String,
    pub(super) route_result: crate::RouteResult,
    pub(super) execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    pub(super) turn_analysis: Option<crate::intent_router::TurnAnalysis>,
    pub(super) clarify_fallback_source: Option<crate::fallback::ClarifyFallbackSource>,
    pub(super) auto_locator_path: Option<String>,
    pub(super) chat_prompt_context: String,
    pub(super) resolved_prompt_for_execution: String,
    pub(super) prompt_with_memory_for_execution: String,
    pub(super) recent_execution_context: String,
    pub(super) agent_mode: bool,
    /// Phase 3.2：合并 routed_mode + direct_resume_*
    /// 后的最终模式，从 PreparedAskRouting.ask_mode 复制而来。
    /// dispatch 内部所有分支决策走 ask_mode 谓词方法。
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

fn clarify_fallback_source_or_default(
    source: Option<crate::fallback::ClarifyFallbackSource>,
) -> crate::fallback::ClarifyFallbackSource {
    source.unwrap_or(crate::fallback::ClarifyFallbackSource::IntentUnresolved)
}

fn clarify_reason_allows_route_question_reuse(
    clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind,
) -> bool {
    matches!(
        clarify_reason_kind,
        crate::post_route_policy::ClarifyReasonKind::RouteReasonText
    )
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
        && route_result.output_contract.locator_hint.trim().is_empty()
}

fn should_reuse_route_clarify_question(
    route_result: &crate::RouteResult,
    clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind,
    fuzzy_locator_suggestions: &[String],
) -> bool {
    !should_suppress_recent_execution_in_clarify_context(route_result, fuzzy_locator_suggestions)
        && fuzzy_locator_suggestions.is_empty()
        && clarify_reason_allows_route_question_reuse(clarify_reason_kind)
}

fn structured_fuzzy_locator_clarify_context(
    fuzzy_locator_suggestions: &[String],
) -> Option<String> {
    if fuzzy_locator_suggestions.is_empty() {
        return None;
    }
    let candidate_block = fuzzy_locator_suggestions
        .iter()
        .enumerate()
        .map(|(idx, value)| format!("candidate_{}: {}", idx + 1, value))
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!(
        "clarify_case: fuzzy_locator_candidates\nexact_target_found: false\ncandidate_count: {}\n{}",
        fuzzy_locator_suggestions.len(),
        candidate_block
    ))
}

fn structured_missing_locator_clarify_context(
    route_result: &crate::RouteResult,
    fuzzy_locator_suggestions: &[String],
) -> Option<String> {
    if !route_result.needs_clarify {
        return None;
    }
    if let Some(context) = structured_fuzzy_locator_clarify_context(fuzzy_locator_suggestions) {
        return Some(context);
    }
    if !route_result.output_contract.locator_hint.trim().is_empty() {
        return None;
    }
    let clarify_case = if matches!(
        route_result.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::DirectoryLookup
    ) {
        Some("missing_directory_locator")
    } else if matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::ScalarCount
    ) {
        Some("missing_count_target")
    } else if matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::ServiceStatus
    ) {
        Some("missing_service_target")
    } else if route_result.output_contract.delivery_required {
        Some("missing_file_locator")
    } else if route_result.output_contract.requires_content_evidence
        && matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
                | crate::OutputResponseShape::Free
                | crate::OutputResponseShape::OneSentence
        )
    {
        Some("missing_read_target")
    } else {
        None
    }?;
    let mut lines = vec![
        format!("clarify_case: {clarify_case}"),
        format!(
            "locator_kind: {}",
            route_result.output_contract.locator_kind.as_str()
        ),
        format!(
            "response_shape: {}",
            route_result.output_contract.response_shape.as_str()
        ),
        format!(
            "semantic_kind: {}",
            route_result.output_contract.semantic_kind.as_str()
        ),
        format!(
            "delivery_required: {}",
            route_result.output_contract.delivery_required
        ),
        format!(
            "requires_content_evidence: {}",
            route_result.output_contract.requires_content_evidence
        ),
    ];
    let route_question = route_result.clarify_question.trim();
    if !route_question.is_empty() {
        lines.push(format!(
            "normalizer_clarify_question_candidate: {}",
            crate::truncate_for_agent_trace(route_question)
        ));
    }
    Some(lines.join("\n"))
}

fn apply_ask_post_route(
    state: &AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    resolved_prompt: &str,
    recent_execution_context: &str,
    route_result: crate::RouteResult,
    mut resolved_prompt_for_execution: String,
    mut prompt_with_memory_for_execution: String,
) -> AppliedAskPostRoute {
    let locator_resolution = if should_attempt_auto_locator(&route_result) {
        current_workspace_locator_resolution(&state.skill_rt.workspace_root, &route_result)
            .unwrap_or_else(|| {
                let locator_hint = route_result.output_contract.locator_hint.trim();
                if locator_hint.is_empty() {
                    return crate::post_route_policy::LocatorResolution::None;
                }
                match super::try_resolve_implicit_locator_path(
                    state,
                    locator_hint,
                    locator_hint,
                    route_result.output_contract.locator_kind,
                    Some(recent_execution_context),
                )
                .map(|resolution| match resolution {
                    super::LocatorAutoResolution::Direct(path) => {
                        crate::post_route_policy::LocatorResolution::Direct(path)
                    }
                    super::LocatorAutoResolution::Fuzzy(candidates) => {
                        crate::post_route_policy::LocatorResolution::Fuzzy(candidates)
                    }
                }) {
                    Some(resolution) => resolution,
                    None => crate::post_route_policy::LocatorResolution::None,
                }
            })
    } else {
        crate::post_route_policy::LocatorResolution::None
    };
    let post_route =
        crate::post_route_policy::apply_post_route_policy(route_result.clone(), locator_resolution);
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
    if post_route.execution_route_result.gate_kind() != route_result.gate_kind() {
        info!(
            "{} worker_once: ask route_gate_override_by_auto_locator task_id={} gate={:?}->{:?}",
            crate::highlight_tag("routing"),
            task.task_id,
            route_result.gate_kind(),
            post_route.execution_route_result.gate_kind()
        );
    } else if post_route.execution_route_result.routed_mode != route_result.routed_mode {
        info!(
            "{} worker_once: ask routed_mode_refined_by_auto_locator task_id={} mode={:?}->{:?}",
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

fn current_workspace_locator_resolution(
    workspace_root: &std::path::Path,
    route_result: &crate::RouteResult,
) -> Option<crate::post_route_policy::LocatorResolution> {
    (route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace).then(
        || {
            crate::post_route_policy::LocatorResolution::Direct(
                workspace_root.display().to_string(),
            )
        },
    )
}

fn should_attempt_auto_locator(route_result: &crate::RouteResult) -> bool {
    if route_result.needs_clarify {
        return false;
    }
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
        &prepared_routing.route_result,
        &prepared_routing.resolved_prompt,
    )
    .await?;
    let applied_post_route = apply_ask_post_route(
        state,
        task,
        prompt,
        &prepared_routing.resolved_prompt,
        &prepared_execution.recent_execution_context,
        prepared_routing.route_result,
        prepared_execution.resolved_prompt_for_execution,
        prepared_execution.prompt_with_memory_for_execution,
    );
    let has_schedule_intent =
        applied_post_route.execution_route_result.schedule_kind != crate::ScheduleKind::None;
    let should_route_schedule_direct = has_schedule_intent
        && !prepared_routing.ask_mode.resume_execution()
        && !prepared_routing.ask_mode.is_resume_discussion();
    Ok(PreparedAskFlow {
        context_bundle_summary: prepared_execution.context_bundle.summary(),
        route_result: applied_post_route.execution_route_result,
        execution_recipe_hint: prepared_routing.execution_recipe_hint,
        turn_analysis: prepared_routing.turn_analysis,
        clarify_fallback_source: prepared_routing.clarify_fallback_source,
        auto_locator_path: applied_post_route.auto_locator_path,
        chat_prompt_context: prepared_execution.chat_prompt_context,
        resolved_prompt_for_execution: applied_post_route.resolved_prompt_for_execution,
        prompt_with_memory_for_execution: applied_post_route.prompt_with_memory_for_execution,
        recent_execution_context: prepared_execution.recent_execution_context,
        agent_mode: prepared_routing.agent_mode,
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
    clarify_fallback_source: Option<crate::fallback::ClarifyFallbackSource>,
    fuzzy_locator_suggestions: &[String],
    ask_mode: &crate::AskMode,
    should_route_schedule_direct: bool,
    agent_run_context: Option<crate::agent_engine::AgentRunContext>,
) -> Result<Option<Result<crate::AskReply, String>>> {
    let execution_user_request = execution_user_request(prompt, resolved_prompt_for_execution);
    if route_result.ask_mode.is_clarify_only() {
        crate::log_ask_transition(
            state,
            &task.task_id,
            Some(crate::AskState::Routing),
            crate::AskState::Clarifying,
            "ask_mode_is_clarify_only",
            None,
        );
        let suppress_recent_execution_context = should_suppress_recent_execution_in_clarify_context(
            route_result,
            fuzzy_locator_suggestions,
        );
        let clarify_context = build_locator_fuzzy_clarify_context(
            recent_execution_context,
            fuzzy_locator_suggestions,
            !suppress_recent_execution_context,
        );
        let structured_clarify_context =
            structured_missing_locator_clarify_context(route_result, fuzzy_locator_suggestions);
        let clarify_context = match structured_clarify_context.as_deref() {
            Some(context)
                if clarify_context.trim().is_empty() || clarify_context.trim() == "<none>" =>
            {
                format!("### STRUCTURED_CLARIFY_CONTEXT\n{context}")
            }
            Some(context) => {
                format!("{clarify_context}\n\n### STRUCTURED_CLARIFY_CONTEXT\n{context}")
            }
            None => clarify_context,
        };
        let preferred_clarify_question = if should_reuse_route_clarify_question(
            route_result,
            clarify_reason_kind,
            fuzzy_locator_suggestions,
        ) {
            let route_question = route_result.clarify_question.trim();
            (!route_question.is_empty()).then_some(route_question)
        } else {
            None
        };
        let structured_context_requires_llm =
            structured_clarify_context.is_some() && preferred_clarify_question.is_none();
        let clarify_policy = if structured_context_requires_llm
            || (preferred_clarify_question.is_none()
                && route_result.clarify_question.trim().is_empty()
                && !matches!(
                    clarify_reason_kind,
                    crate::post_route_policy::ClarifyReasonKind::FuzzyLocatorCandidates
                )) {
            crate::intent_router::ClarifyQuestionPolicy::SafeFallback
        } else {
            crate::intent_router::ClarifyQuestionPolicy::AllowModel
        };
        let fallback_source = clarify_fallback_source_or_default(clarify_fallback_source);
        let clarify = crate::intent_router::generate_or_reuse_clarify_question(
            state,
            task,
            prompt,
            clarify_reason,
            Some(&clarify_context),
            preferred_clarify_question,
            clarify_policy,
            // §7.2: 路由阶段没拿到可用 clarify_question + 非 fuzzy_locator 触发的 SafeFallback。
            // normalizer LLM 失败必须暴露为 LlmUnavailable，不能伪装成“我没看懂”。
            fallback_source,
        )
        .await;
        return Ok(Some(Ok(crate::AskReply::non_llm(clarify))));
    }
    if ask_mode.is_resume_discussion() {
        crate::log_ask_transition(
            state,
            &task.task_id,
            Some(crate::AskState::Routing),
            crate::AskState::ResumeDiscussing,
            "ask_mode_resume_discussion",
            None,
        );
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
        crate::log_ask_transition(
            state,
            &task.task_id,
            Some(crate::AskState::Routing),
            crate::AskState::ResumeExecuting,
            "ask_mode_resume_execution",
            None,
        );
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
        crate::log_ask_transition(
            state,
            &task.task_id,
            Some(crate::AskState::Routing),
            crate::AskState::ScheduleDirect,
            "schedule_direct_route",
            None,
        );
        if crate::finalize::try_finalize_schedule_direct_success(
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
        let routed_to_execute = route_result.is_execute_gate();
        let target_state = if routed_to_execute {
            crate::AskState::Executing
        } else {
            crate::AskState::Chatting
        };
        crate::log_ask_transition(
            state,
            &task.task_id,
            Some(crate::AskState::Routing),
            target_state,
            "execute_ask_routed_in_schedule_branch",
            None,
        );
        return Ok(Some(
            crate::execute_ask_routed(
                state,
                task,
                chat_prompt_context,
                prompt_with_memory_for_execution,
                resolved_prompt_for_execution,
                execution_user_request,
                agent_mode,
                ask_mode.is_resume_discussion(),
                Some(route_result.ask_mode.clone()),
                agent_run_context.clone(),
            )
            .await,
        ));
    }
    let routed_to_execute = route_result.is_execute_gate();
    let target_state = if routed_to_execute {
        crate::AskState::Executing
    } else {
        crate::AskState::Chatting
    };
    crate::log_ask_transition(
        state,
        &task.task_id,
        Some(crate::AskState::Routing),
        target_state,
        if routed_to_execute {
            "execute_ask_routed_act"
        } else {
            "execute_ask_routed_chat"
        },
        None,
    );
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
            Some(route_result.ask_mode.clone()),
            agent_run_context,
        )
        .await,
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        clarify_fallback_source_or_default, execution_user_request, should_attempt_auto_locator,
        should_preserve_original_inline_structured_input, should_reuse_route_clarify_question,
        should_suppress_recent_execution_in_clarify_context,
        structured_missing_locator_clarify_context,
    };
    use std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn make_temp_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rustclaw_ask_pipeline_{label}_{}_{}",
            std::process::id(),
            nonce
        ));
        std::fs::create_dir_all(&path).expect("temp root");
        path
    }

    #[test]
    fn llm_failed_normalizer_source_uses_llm_unavailable_fallback_source() {
        assert_eq!(
            clarify_fallback_source_or_default(Some(
                crate::fallback::ClarifyFallbackSource::LlmUnavailable
            )),
            crate::fallback::ClarifyFallbackSource::LlmUnavailable
        );
    }

    #[test]
    fn absent_normalizer_fallback_source_uses_intent_unresolved() {
        assert_eq!(
            clarify_fallback_source_or_default(None),
            crate::fallback::ClarifyFallbackSource::IntentUnresolved
        );
    }

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
        };
        assert!(should_attempt_auto_locator(&route));
    }

    #[test]
    fn current_workspace_locator_resolution_prefers_workspace_root() {
        let root = make_temp_root("current_workspace_locator_root");
        std::fs::create_dir_all(root.join("rustclaw")).expect("nested rustclaw dir");
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "Write a long introduction for RustClaw".to_string(),
            needs_clarify: false,
            route_reason: "workspace summary".to_string(),
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
                requires_content_evidence: true,
                ..Default::default()
            },
        };
        assert!(matches!(
            super::current_workspace_locator_resolution(&root, &route),
            Some(crate::post_route_policy::LocatorResolution::Direct(path))
                if path == root.display().to_string()
        ));
        let _ = std::fs::remove_dir_all(root);
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
        };
        assert!(should_attempt_auto_locator(&route));
    }

    #[test]
    fn auto_locator_skips_clarify_routes() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::AskClarify,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::AskClarify),
            resolved_intent: "读一下那个 README 开头，然后一句话总结".to_string(),
            needs_clarify: true,
            route_reason: "normalizer requested clarification before execution".to_string(),
            route_confidence: Some(0.95),
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
                requires_content_evidence: true,
                response_shape: crate::OutputResponseShape::OneSentence,
                ..Default::default()
            },
        };
        assert!(!should_attempt_auto_locator(&route));
    }

    #[test]
    fn auto_locator_skips_stateful_ordered_entry_clarify_routes() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::AskClarify,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::AskClarify),
            resolved_intent: "看第二个".to_string(),
            needs_clarify: true,
            route_reason: "stateful_ordered_entry_ambiguous_clarify:content_read:entries=4"
                .to_string(),
            route_confidence: Some(0.97),
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
                response_shape: crate::OutputResponseShape::Free,
                ..Default::default()
            },
        };
        assert!(!should_attempt_auto_locator(&route));
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
        }
    }

    #[test]
    fn scalar_missing_locator_attaches_structured_context() {
        let route = clarify_route(crate::OutputLocatorKind::Path);
        let context = structured_missing_locator_clarify_context(&route, &[])
            .expect("structured clarify context");
        assert!(context.contains("clarify_case: missing_read_target"));
        assert!(context.contains("locator_kind: path"));
    }

    #[test]
    fn structured_missing_locator_records_explicit_route_question_as_context() {
        let mut route = clarify_route(crate::OutputLocatorKind::Filename);
        route.clarify_question = "LOCATOR_CLARIFY_PROMPT".to_string();
        route.output_contract.delivery_intent = crate::OutputDeliveryIntent::DirectoryLookup;
        let context = structured_missing_locator_clarify_context(&route, &[])
            .expect("structured clarify context");
        assert!(context.contains("clarify_case: missing_directory_locator"));
        assert!(context.contains("normalizer_clarify_question_candidate"));
    }

    #[test]
    fn content_missing_locator_attaches_structured_context() {
        let mut route = clarify_route(crate::OutputLocatorKind::Path);
        route.clarify_question.clear();
        route.output_contract.response_shape = crate::OutputResponseShape::Free;
        let context = structured_missing_locator_clarify_context(&route, &[])
            .expect("structured clarify context");
        assert!(context.contains("clarify_case: missing_read_target"));
        assert!(context.contains("response_shape: free"));
    }

    #[test]
    fn directory_lookup_missing_locator_records_directory_case() {
        let mut route = clarify_route(crate::OutputLocatorKind::Filename);
        route.clarify_question.clear();
        route.output_contract.response_shape = crate::OutputResponseShape::Free;
        route.output_contract.delivery_intent = crate::OutputDeliveryIntent::DirectoryLookup;
        let context = structured_missing_locator_clarify_context(&route, &[])
            .expect("structured clarify context");
        assert!(context.contains("clarify_case: missing_directory_locator"));
    }

    #[test]
    fn scalar_count_missing_locator_records_count_case() {
        let mut route = clarify_route(crate::OutputLocatorKind::Filename);
        route.clarify_question.clear();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        let context = structured_missing_locator_clarify_context(&route, &[])
            .expect("structured clarify context");
        assert!(context.contains("clarify_case: missing_count_target"));
        assert!(context.contains("semantic_kind: scalar_count"));
    }

    #[test]
    fn delivery_missing_locator_attaches_structured_context() {
        let mut route = clarify_route(crate::OutputLocatorKind::Filename);
        route.clarify_question.clear();
        route.output_contract.requires_content_evidence = false;
        route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        route.output_contract.delivery_required = true;
        route.wants_file_delivery = true;
        let context = structured_missing_locator_clarify_context(&route, &[])
            .expect("structured clarify context");
        assert!(context.contains("clarify_case: missing_file_locator"));
        assert!(context.contains("delivery_required: true"));
    }

    #[test]
    fn fuzzy_locator_candidates_attach_structured_context() {
        let route = clarify_route(crate::OutputLocatorKind::Filename);
        let candidates = vec![
            "/tmp/a/Cargo.toml".to_string(),
            "/tmp/b/Cargo.toml".to_string(),
        ];
        let context = structured_missing_locator_clarify_context(&route, &candidates)
            .expect("structured clarify context");
        assert!(context.contains("clarify_case: fuzzy_locator_candidates"));
        assert!(context.contains("candidate_1: /tmp/a/Cargo.toml"));
        assert!(context.contains("candidate_2: /tmp/b/Cargo.toml"));
    }

    #[test]
    fn path_scoped_clarify_without_locator_suppresses_recent_execution_context() {
        let route = clarify_route(crate::OutputLocatorKind::Path);
        assert!(should_suppress_recent_execution_in_clarify_context(
            &route,
            &[],
        ));
    }

    #[test]
    fn filename_clarify_without_locator_does_not_reuse_router_question() {
        let route = clarify_route(crate::OutputLocatorKind::Filename);
        assert!(!should_reuse_route_clarify_question(
            &route,
            crate::post_route_policy::ClarifyReasonKind::MissingPathScopedLocator,
            &[],
        ));
    }

    #[test]
    fn route_reason_text_can_reuse_router_question_when_structured_locator_exists() {
        let mut route = clarify_route(crate::OutputLocatorKind::Filename);
        route.output_contract.locator_hint = "README.md".to_string();
        assert!(should_reuse_route_clarify_question(
            &route,
            crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
            &[],
        ));
    }

    #[test]
    fn clarify_with_fuzzy_candidates_keeps_recent_context_available() {
        let route = clarify_route(crate::OutputLocatorKind::Filename);
        assert!(!should_suppress_recent_execution_in_clarify_context(
            &route,
            &["/tmp/a".to_string()],
        ));
    }
}
