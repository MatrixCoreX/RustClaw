use anyhow::Result;
use serde_json::Value;
use tracing::info;

use super::*;

pub(super) struct PreparedAskFlow {
    pub(super) context_bundle_summary: String,
    pub(super) route_result: crate::RouteResult,
    pub(super) auto_locator_path: Option<String>,
    pub(super) chat_prompt_context: String,
    pub(super) resolved_prompt_for_execution: String,
    pub(super) prompt_with_memory_for_execution: String,
    pub(super) recent_execution_context: String,
    pub(super) agent_mode: bool,
    pub(super) direct_resume_execution: bool,
    pub(super) direct_resume_discussion: bool,
    pub(super) classifier_direct_mode: bool,
    pub(super) clarify_reason: String,
    pub(super) fuzzy_locator_suggestions: Vec<String>,
    pub(super) should_route_schedule_direct: bool,
}

struct AppliedAskPostRoute {
    execution_route_result: crate::RouteResult,
    auto_locator_path: Option<String>,
    resolved_prompt_for_execution: String,
    prompt_with_memory_for_execution: String,
    clarify_reason: String,
    fuzzy_locator_suggestions: Vec<String>,
}

fn should_allow_classifier_direct(route_result: &crate::RouteResult) -> bool {
    !matches!(
        route_result.routed_mode,
        crate::RoutedMode::Act | crate::RoutedMode::ChatAct
    )
}

fn normalize_brief_route_text(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn resolved_intent_inherits_prior_operation(prompt: &str, resolved_prompt: &str) -> bool {
    let prompt_norm = normalize_brief_route_text(prompt.trim());
    let resolved_norm = normalize_brief_route_text(resolved_prompt.trim());
    !prompt_norm.is_empty()
        && !resolved_norm.is_empty()
        && prompt_norm != resolved_norm
        && resolved_norm.len() > prompt_norm.len()
}

fn build_locator_fuzzy_clarify_context(
    recent_execution_context: &str,
    fuzzy_locator_suggestions: &[String],
) -> String {
    if fuzzy_locator_suggestions.is_empty() {
        return recent_execution_context.to_string();
    }
    let candidate_block = fuzzy_locator_suggestions
        .iter()
        .map(|v| format!("- {v}"))
        .collect::<Vec<_>>()
        .join("\n");
    let fuzzy_notice = "Exact target was not found. The following are only similar locator candidates for confirmation; they are not confirmed matches to the requested file.";
    if recent_execution_context.trim().is_empty() || recent_execution_context.trim() == "<none>" {
        format!("### LOCATOR_FUZZY_CANDIDATES\n{fuzzy_notice}\n{candidate_block}\n")
    } else {
        format!(
            "{}\n\n### LOCATOR_FUZZY_CANDIDATES\n{}\n{}\n",
            recent_execution_context, fuzzy_notice, candidate_block
        )
    }
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
        match super::try_resolve_implicit_locator_path(
            state,
            prompt,
            resolved_prompt,
            route_result.output_contract.locator_kind,
            Some(recent_execution_context),
        ) {
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
        resolved_intent_inherits_prior_operation(prompt, resolved_prompt),
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
            "{} worker_once: ask force_clarify_by_locator_guard task_id={} reason=missing_concrete_locator_for_path_scoped_content raw_text={} resolved_text={}",
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
        auto_locator_path: applied_post_route.auto_locator_path,
        chat_prompt_context: prepared_execution.chat_prompt_context,
        resolved_prompt_for_execution: applied_post_route.resolved_prompt_for_execution,
        prompt_with_memory_for_execution: applied_post_route.prompt_with_memory_for_execution,
        recent_execution_context: prepared_execution.recent_execution_context,
        agent_mode: prepared_routing.agent_mode,
        direct_resume_execution: prepared_routing.direct_resume_execution,
        direct_resume_discussion: prepared_routing.direct_resume_discussion,
        classifier_direct_mode: prepared_routing.classifier_direct_mode,
        clarify_reason: applied_post_route.clarify_reason,
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
    fuzzy_locator_suggestions: &[String],
    classifier_direct_mode: bool,
    direct_resume_discussion: bool,
    direct_resume_execution: bool,
    should_route_schedule_direct: bool,
    agent_run_context: Option<crate::agent_engine::AgentRunContext>,
) -> Result<Option<Result<crate::AskReply, String>>> {
    if matches!(route_result.routed_mode, crate::RoutedMode::AskClarify) {
        let clarify_context = build_locator_fuzzy_clarify_context(
            recent_execution_context,
            fuzzy_locator_suggestions,
        );
        let clarify = crate::intent_router::generate_clarify_question(
            state,
            task,
            prompt,
            clarify_reason,
            Some(&clarify_context),
        )
        .await;
        return Ok(Some(Ok(crate::AskReply::non_llm(clarify))));
    }
    if direct_resume_discussion {
        let resume_prompt_source = crate::resolve_prompt_rel_path_for_vendor(
            &state.workspace_root,
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
    if direct_resume_execution {
        return Ok(Some(
            crate::agent_engine::run_agent_with_tools(
                state,
                task,
                prompt_with_memory_for_execution,
                resolved_prompt_for_execution,
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
        )
        .await?
        {
            return Ok(None);
        }
        if classifier_direct_mode
            && !direct_resume_discussion
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
                agent_mode,
                direct_resume_discussion,
                Some(route_result.routed_mode),
                agent_run_context.clone(),
            )
            .await,
        ));
    }
    if classifier_direct_mode && should_allow_classifier_direct(route_result) {
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
        resolved_intent_inherits_prior_operation, should_allow_classifier_direct,
        should_attempt_auto_locator,
    };

    #[test]
    fn auto_locator_attempts_for_path_locators_even_without_content_evidence() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            resolved_intent: "读取 Cargo.toml".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            wants_file_delivery: false,
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
            resolved_intent: "今天天气".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            wants_file_delivery: false,
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
            resolved_intent: "检查当前目录是否存在隐藏文件".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            wants_file_delivery: false,
            output_contract: crate::IntentOutputContract {
                locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
                requires_content_evidence: false,
                ..Default::default()
            },
        };
        assert!(should_attempt_auto_locator(&route));
    }

    #[test]
    fn auto_locator_attempts_for_filename_locators() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            resolved_intent: "读取 README 前 20 行".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            wants_file_delivery: false,
            output_contract: crate::IntentOutputContract {
                locator_kind: crate::OutputLocatorKind::Filename,
                requires_content_evidence: true,
                ..Default::default()
            },
        };
        assert!(should_attempt_auto_locator(&route));
    }

    #[test]
    fn classifier_direct_disabled_for_act_mode() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            resolved_intent: "读取 Cargo.toml".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            wants_file_delivery: false,
            output_contract: crate::IntentOutputContract::default(),
        };
        assert!(!should_allow_classifier_direct(&route));
    }

    #[test]
    fn classifier_direct_disabled_for_chat_act_mode() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            resolved_intent: "比较 Cargo.toml 和 Cargo.lock".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            wants_file_delivery: false,
            output_contract: crate::IntentOutputContract::default(),
        };
        assert!(!should_allow_classifier_direct(&route));
    }

    #[test]
    fn classifier_direct_kept_for_chat_mode() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Chat,
            resolved_intent: "解释一下这个概念".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            wants_file_delivery: false,
            output_contract: crate::IntentOutputContract::default(),
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
}
