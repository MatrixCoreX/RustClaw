use anyhow::Result;
use serde_json::Value;
use tracing::info;

use super::*;

pub(super) struct PreparedAskFlow {
    pub(super) context_bundle_summary: String,
    pub(super) memory_trace: Option<Value>,
    pub(super) route_result: crate::RouteResult,
    pub(super) execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    pub(super) turn_analysis: Option<crate::intent_router::TurnAnalysis>,
    pub(super) clarify_fallback_source: Option<crate::fallback::ClarifyFallbackSource>,
    pub(super) auto_locator_path: Option<String>,
    pub(super) has_authoritative_deictic_anchor: bool,
    pub(super) chat_prompt_context: String,
    pub(super) resolved_prompt_for_execution: String,
    pub(super) prompt_with_memory_for_execution: String,
    pub(super) memory_context_for_execution: String,
    pub(super) semantic_answer_candidate_draft: Option<String>,
    pub(super) recent_execution_context: String,
    pub(super) agent_mode: bool,
    /// Final runtime ask mode copied from PreparedAskRouting.
    /// Dispatch decisions must use ask_mode predicates.
    pub(super) ask_mode: crate::AskMode,
    pub(super) clarify_reason: String,
    pub(super) clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind,
    pub(super) fuzzy_locator_suggestions: Vec<String>,
    pub(super) should_route_schedule_direct: bool,
}

struct AppliedAskPostRoute {
    execution_route_result: crate::RouteResult,
    auto_locator_path: Option<String>,
    has_authoritative_deictic_anchor: bool,
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

fn ask_reply_with_visible_process(
    _state: &crate::AppState,
    _task: &crate::ClaimedTask,
    _prompt: &str,
    text: String,
) -> crate::AskReply {
    let answer = text.trim().to_string();
    if answer.is_empty() || crate::finalize::is_execution_summary_message(&answer) {
        crate::AskReply::non_llm(text)
    } else {
        crate::AskReply::non_llm(answer)
    }
}

fn direct_existing_file_delivery_token(route_result: &crate::RouteResult) -> Option<String> {
    let contract = &route_result.output_contract;
    if route_result.needs_clarify
        || !contract.delivery_required
        || contract.response_shape != crate::OutputResponseShape::FileToken
        || contract.delivery_intent != crate::OutputDeliveryIntent::FileSingle
        || contract.locator_kind != crate::OutputLocatorKind::Path
    {
        return None;
    }
    if contract.semantic_kind == crate::OutputSemanticKind::GeneratedFileDelivery
        && route_result
            .route_reason
            .split(';')
            .map(str::trim)
            .any(|part| part == "generated_file_delivery_allows_runtime_target")
    {
        return None;
    }
    let path = contract.locator_hint.trim();
    if path.is_empty() {
        return None;
    }
    let path = std::path::Path::new(path);
    if !path.is_file() {
        return None;
    }
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    Some(format!("FILE:{}", path.display()))
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
    _clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind,
    fuzzy_locator_suggestions: &[String],
) -> bool {
    fuzzy_locator_suggestions.is_empty() && !route_result.clarify_question.trim().is_empty()
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

fn structured_missing_locator_default_question(
    state: &AppState,
    language_hint: &str,
    route_result: &crate::RouteResult,
    fuzzy_locator_suggestions: &[String],
) -> Option<String> {
    if !route_result.needs_clarify
        || !fuzzy_locator_suggestions.is_empty()
        || !route_result.output_contract.locator_hint.trim().is_empty()
    {
        return None;
    }
    let prefer_english =
        crate::fallback::fallback_prefers_english_for_language_hint(state, language_hint);
    let (key, zh, en) = if matches!(
        route_result.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::DirectoryLookup
    ) {
        (
            "clawd.msg.clarify.missing_directory_locator",
            "请提供要查看的目录完整路径。",
            "Please provide the full path of the directory to inspect.",
        )
    } else if matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::SqliteTableListing
            | crate::OutputSemanticKind::SqliteTableNamesOnly
            | crate::OutputSemanticKind::SqliteDatabaseKindJudgment
            | crate::OutputSemanticKind::SqliteSchemaVersion
    ) {
        (
            "clawd.msg.clarify.missing_sqlite_locator",
            "请提供要查询的 SQLite 数据库文件路径。",
            "Please provide the path of the SQLite database file to query.",
        )
    } else if matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::ScalarCount
    ) {
        (
            "clawd.msg.clarify.missing_count_target",
            "请提供要统计的文件或目录完整路径。",
            "Please provide the full path of the file or directory to count.",
        )
    } else if matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::ServiceStatus
    ) {
        (
            "clawd.msg.clarify.missing_service_target",
            "请说明要查询的服务或进程名称。",
            "Please specify the service or process name to check.",
        )
    } else if route_result.output_contract.delivery_required {
        (
            "clawd.msg.clarify.missing_file_delivery_locator",
            "请提供要发送的文件完整路径。",
            "Please provide the full path of the file to send.",
        )
    } else if route_result.output_contract.requires_content_evidence {
        (
            "clawd.msg.clarify.missing_read_target",
            "请提供要读取或检查的具体文件、目录或路径。",
            "Please provide the specific file, directory, or path to read or inspect.",
        )
    } else {
        return None;
    };
    Some(crate::app_helpers::bilingual_t_with_default(
        state,
        key,
        zh,
        en,
        prefer_english,
    ))
}

fn apply_ask_post_route(
    state: &AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    resolved_prompt: &str,
    recent_execution_context: &str,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    mut route_result: crate::RouteResult,
    mut resolved_prompt_for_execution: String,
    mut prompt_with_memory_for_execution: String,
) -> AppliedAskPostRoute {
    let session_snapshot = crate::conversation_state::load_active_session_snapshot(state, task);
    let has_authoritative_deictic_anchor =
        session_has_authoritative_deictic_anchor(prompt, &route_result, &session_snapshot);
    if deictic_memory_only_route_should_force_clarify(
        prompt,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route_result);
        route_result.needs_clarify = true;
        route_result.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
        route_result.clarify_question = deictic_missing_locator_question(&route_result).to_string();
        append_route_reason(&mut route_result, "deictic_memory_only_requires_clarify");
    }
    if unbound_model_context_target_route_should_force_clarify(
        state,
        prompt,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route_result);
        route_result.needs_clarify = true;
        route_result.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
        route_result.clarify_question.clear();
        append_route_reason(
            &mut route_result,
            "unbound_model_context_target_requires_clarify",
        );
    }
    if bare_topic_model_supplied_locator_route_should_force_clarify(
        prompt,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route_result);
        route_result.needs_clarify = true;
        route_result.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
        route_result.clarify_question.clear();
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route_result.output_contract.locator_hint.clear();
        append_route_reason(
            &mut route_result,
            "bare_topic_model_supplied_locator_requires_clarify",
        );
    }
    prebind_workspace_child_locator_from_current_request(state, prompt, &mut route_result);
    prebind_clarify_workspace_child_locator_from_current_request(state, prompt, &mut route_result);
    prebind_workspace_child_locator_from_resolved_prompt(state, resolved_prompt, &mut route_result);
    repair_compound_file_names_plus_content_summary_contract(&mut route_result);
    repair_summary_only_content_excerpt_with_summary_contract(&mut route_result);
    prebind_file_delivery_locator_from_recent_ordered_resolved_prompt(
        state,
        resolved_prompt,
        recent_execution_context,
        &mut route_result,
    );
    prebind_file_delivery_locator_from_resolved_prompt_path(
        state,
        resolved_prompt,
        &mut route_result,
    );
    prebind_workspace_root_locator_from_resolved_prompt(state, resolved_prompt, &mut route_result);
    prebind_quantity_compare_directory_pair_from_current_request(
        state,
        resolved_prompt,
        &mut route_result,
    );
    prebind_existing_workspace_locator_hint_from_current_request(state, prompt, &mut route_result);
    prebind_session_alias_locator_from_current_request(
        prompt,
        &mut route_result,
        &session_snapshot,
    );
    prebind_active_bound_target_from_matching_locator_hint(&mut route_result, &session_snapshot);
    prebind_active_bound_target_for_locatorless_content_evidence(
        &mut route_result,
        &session_snapshot,
    );
    if bare_topic_model_supplied_locator_route_should_force_clarify(
        prompt,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route_result);
        route_result.needs_clarify = true;
        route_result.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
        route_result.clarify_question.clear();
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route_result.output_contract.locator_hint.clear();
        append_route_reason(
            &mut route_result,
            "bare_topic_model_supplied_locator_requires_clarify",
        );
    }
    prebind_quantity_compare_directory_pair_from_current_request(state, prompt, &mut route_result);
    if background_only_locator_route_should_force_clarify(
        state,
        prompt,
        resolved_prompt,
        recent_execution_context,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route_result);
        route_result.needs_clarify = true;
        route_result.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
        route_result.clarify_question = deictic_missing_locator_question(&route_result).to_string();
        append_route_reason(&mut route_result, "background_locator_requires_clarify");
    }
    downgrade_background_locator_clarify_to_recent_observed_chat(
        &mut route_result,
        recent_execution_context,
    );
    promote_locatorless_scalar_status_query_to_runtime_info(&mut route_result, turn_analysis);
    promote_locatorless_status_query_to_service_status(
        state,
        prompt,
        &mut route_result,
        turn_analysis,
    );
    prebind_runtime_status_scalar_path_to_current_workspace(
        &mut route_result,
        turn_analysis,
        &session_snapshot,
    );
    promote_broad_current_workspace_content_summary_to_directory_purpose(prompt, &mut route_result);
    promote_locatorless_git_capability_to_repository_state(&mut route_result);
    promote_locatorless_scalar_child_metadata_to_quantity_comparison(
        state,
        prompt,
        &mut route_result,
    );
    if locatorless_observation_route_should_force_clarify(
        state,
        prompt,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route_result);
        route_result.needs_clarify = true;
        route_result.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
        route_result.clarify_question.clear();
        append_route_reason(
            &mut route_result,
            "locatorless_observation_requires_clarify",
        );
    }
    if unbound_targeted_evidence_route_should_force_clarify(
        prompt,
        &route_result,
        &session_snapshot,
    ) {
        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route_result);
        route_result.needs_clarify = true;
        route_result.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
        route_result.clarify_question.clear();
        append_route_reason(
            &mut route_result,
            "unbound_targeted_evidence_requires_clarify",
        );
    }
    if bare_topic_memory_expansion_route_should_force_clarify(
        prompt,
        &route_result,
        turn_analysis,
        &session_snapshot,
    ) {
        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route_result);
        route_result.needs_clarify = true;
        route_result.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
        route_result.clarify_question.clear();
        append_route_reason(
            &mut route_result,
            "bare_topic_context_expansion_requires_clarify",
        );
    }
    if bare_topic_clarify_question_should_drop_context_target(prompt, &route_result) {
        route_result.clarify_question.clear();
        append_route_reason(&mut route_result, "bare_topic_contextual_clarify_sanitized");
    }
    if unbound_existing_file_delivery_route_should_force_clarify(
        state,
        prompt,
        &route_result,
        has_authoritative_deictic_anchor,
    ) {
        route_result.needs_clarify = true;
        route_result.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
        route_result.clarify_question.clear();
        route_result.wants_file_delivery = true;
        route_result.output_contract.delivery_required = true;
        route_result.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        route_result.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route_result.output_contract.locator_hint.clear();
        append_route_reason(
            &mut route_result,
            "unbound_existing_file_delivery_requires_clarify",
        );
    }
    prebind_direct_file_delivery_locator_before_deictic_guard(
        state,
        recent_execution_context,
        &mut route_result,
    );
    if deictic_bare_locator_should_force_clarify(&route_result, turn_analysis) {
        route_result.needs_clarify = true;
        route_result.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
        if route_result.clarify_question.trim().is_empty() {
            route_result.clarify_question =
                deictic_missing_locator_question(&route_result).to_string();
        }
        append_route_reason(&mut route_result, "deictic_bare_locator_requires_clarify");
    }
    if direct_answer_from_structured_anchor_requires_evidence(
        prompt,
        &route_result,
        &session_snapshot,
        recent_execution_context,
        has_authoritative_deictic_anchor,
        turn_analysis,
    ) {
        promote_structured_anchor_direct_answer_to_evidence(&mut route_result);
    }
    let locator_resolution = if should_attempt_auto_locator(&route_result) {
        current_workspace_locator_resolution(&state.skill_rt.workspace_root, &route_result)
            .unwrap_or_else(|| {
                let locator_hint = route_result.output_contract.locator_hint.trim();
                if locator_hint.is_empty() {
                    return crate::post_route_policy::LocatorResolution::None;
                }
                let locator_kind = effective_auto_locator_kind(&route_result);
                match super::try_resolve_implicit_locator_path(
                    state,
                    locator_hint,
                    locator_hint,
                    locator_kind,
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
    } else if post_route.execution_route_result.ask_mode != route_result.ask_mode {
        info!(
            "{} worker_once: ask ask_mode_refined_by_auto_locator task_id={} ask_mode={} -> {} derived_route_label={} -> {}",
            crate::highlight_tag("routing"),
            task.task_id,
            route_result.ask_mode.as_str(),
            post_route.execution_route_result.ask_mode.as_str(),
            route_result.derived_route_label(),
            post_route.execution_route_result.derived_route_label()
        );
    }
    AppliedAskPostRoute {
        execution_route_result: post_route.execution_route_result,
        auto_locator_path: post_route.auto_locator_path,
        has_authoritative_deictic_anchor,
        resolved_prompt_for_execution,
        prompt_with_memory_for_execution,
        clarify_reason: post_route.clarify_reason,
        clarify_reason_kind: post_route.clarify_reason_kind,
        fuzzy_locator_suggestions: post_route.fuzzy_locator_suggestions,
    }
}

fn direct_auto_locator_path(
    state: &AppState,
    route_result: &crate::RouteResult,
    recent_execution_context: &str,
) -> Option<String> {
    if !should_attempt_auto_locator(route_result) {
        return None;
    }
    if let Some(crate::post_route_policy::LocatorResolution::Direct(path)) =
        current_workspace_locator_resolution(&state.skill_rt.workspace_root, route_result)
    {
        return Some(path);
    }
    let locator_hint = route_result.output_contract.locator_hint.trim();
    if locator_hint.is_empty() {
        return None;
    }
    let locator_kind = effective_auto_locator_kind(route_result);
    super::try_resolve_implicit_locator_path(
        state,
        locator_hint,
        locator_hint,
        locator_kind,
        Some(recent_execution_context),
    )
    .and_then(|resolution| match resolution {
        super::LocatorAutoResolution::Direct(path) => Some(path),
        super::LocatorAutoResolution::Fuzzy(_) => None,
    })
}

fn prebind_direct_file_delivery_locator_before_deictic_guard(
    state: &AppState,
    recent_execution_context: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if route_result.needs_clarify
        || !route_result.output_contract.delivery_required
        || route_result.output_contract.response_shape != crate::OutputResponseShape::FileToken
        || route_result.output_contract.delivery_intent != crate::OutputDeliveryIntent::FileSingle
        || crate::worker::has_explicit_path_or_url_locator_hint(
            route_result.output_contract.locator_hint.trim(),
        )
    {
        return false;
    }
    let Some(path) = direct_auto_locator_path(state, route_result, recent_execution_context) else {
        return false;
    };
    if locator_hint_points_to_workspace_root(&state.skill_rt.workspace_root, &path) {
        route_result.needs_clarify = true;
        route_result.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
        route_result.clarify_question.clear();
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route_result.output_contract.locator_hint.clear();
        append_route_reason(
            route_result,
            "direct_file_delivery_workspace_root_locator_rejected",
        );
        return false;
    }
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = path;
    append_route_reason(
        route_result,
        "direct_file_delivery_locator_prebound_before_deictic_guard",
    );
    true
}

fn repair_compound_file_names_plus_content_summary_contract(route_result: &mut crate::RouteResult) {
    if !(route_reason_has_marker(
        route_result,
        "llm_semantic_contract_repair:compound_request_requires_repair_to_file_names_plus_content_summary",
    ) || route_reason_has_marker_prefix(
        route_result,
        "llm_semantic_contract_repair:malformed_contract_listing_vs_content_synthesis_conflict",
    )) || route_result.output_contract.semantic_kind != crate::OutputSemanticKind::FileNames
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
    {
        return;
    }
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route_result.output_contract.response_shape = crate::OutputResponseShape::Free;
    append_route_reason(
        route_result,
        "compound_file_names_plus_content_summary_contract_repaired",
    );
}

fn repair_summary_only_content_excerpt_with_summary_contract(
    route_result: &mut crate::RouteResult,
) {
    if route_result.output_contract.semantic_kind
        != crate::OutputSemanticKind::ContentExcerptWithSummary
        || route_result.output_contract.response_shape != crate::OutputResponseShape::OneSentence
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
    {
        return;
    }
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    append_route_reason(
        route_result,
        "summary_only_content_excerpt_with_summary_contract_repaired",
    );
}

fn route_reason_has_marker_prefix(route_result: &crate::RouteResult, marker_prefix: &str) -> bool {
    route_result
        .route_reason
        .split(';')
        .map(str::trim)
        .any(|part| part == marker_prefix || part.starts_with(&format!("{marker_prefix}:")))
}

fn effective_auto_locator_kind(route_result: &crate::RouteResult) -> crate::OutputLocatorKind {
    if route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && !route_result.output_contract.locator_hint.trim().is_empty()
    {
        crate::OutputLocatorKind::Path
    } else {
        route_result.output_contract.locator_kind
    }
}

fn current_workspace_locator_resolution(
    workspace_root: &std::path::Path,
    route_result: &crate::RouteResult,
) -> Option<crate::post_route_policy::LocatorResolution> {
    if route_result.output_contract.locator_kind != crate::OutputLocatorKind::CurrentWorkspace {
        return None;
    }
    let locator_hint = route_result.output_contract.locator_hint.trim();
    if !locator_hint.is_empty()
        && !locator_hint_points_to_workspace_root(workspace_root, locator_hint)
    {
        return None;
    }
    Some(crate::post_route_policy::LocatorResolution::Direct(
        workspace_root.display().to_string(),
    ))
}

fn locator_hint_names_workspace_root(workspace_root: &std::path::Path, locator_hint: &str) -> bool {
    let Some(root_name) = workspace_root.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let normalized_root = normalize_locator_identity_token(root_name);
    let normalized_hint = normalize_locator_identity_token(locator_hint);
    !normalized_root.is_empty() && normalized_hint == normalized_root
}

fn locator_hint_points_to_workspace_root(
    workspace_root: &std::path::Path,
    locator_hint: &str,
) -> bool {
    if locator_hint_names_workspace_root(workspace_root, locator_hint) {
        return true;
    }
    let locator_hint = locator_hint.trim();
    if locator_hint.is_empty() || locator_hint.contains('\n') {
        return false;
    }
    let candidate = std::path::Path::new(locator_hint);
    let candidate = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace_root.join(candidate)
    };
    normalize_workspace_locator_path(&candidate) == normalize_workspace_locator_path(workspace_root)
}

fn normalize_workspace_locator_path(path: &std::path::Path) -> std::path::PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn normalize_locator_identity_token(value: &str) -> String {
    value
        .trim()
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\''
                    | '`'
                    | ','
                    | '.'
                    | '，'
                    | '。'
                    | ':'
                    | '：'
                    | ';'
                    | '；'
                    | ')'
                    | '('
                    | ']'
                    | '['
                    | '）'
                    | '（'
                    | '】'
                    | '【'
                    | '>'
                    | '<'
                    | '》'
                    | '《'
            )
        })
        .to_ascii_lowercase()
}

fn should_attempt_auto_locator(route_result: &crate::RouteResult) -> bool {
    if route_result.needs_clarify && route_result.output_contract.locator_hint.trim().is_empty() {
        return false;
    }
    if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison
        && route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && route_result.output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    matches!(
        route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
            | crate::OutputLocatorKind::CurrentWorkspace
            | crate::OutputLocatorKind::Filename
    )
}

fn execute_route_without_input_locator_should_plan(route_result: &crate::RouteResult) -> bool {
    route_result.is_execute_gate()
        && route_result.output_contract.requires_content_evidence
        && route_result.output_contract.locator_kind == crate::OutputLocatorKind::None
        && route_result.output_contract.locator_hint.trim().is_empty()
        && !route_result.wants_file_delivery
        && !route_result.output_contract.delivery_required
        && !matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
}

fn active_observed_facts_have_bound_target(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    session_snapshot
        .active_observed_facts
        .as_ref()
        .and_then(|facts| facts.bound_target.as_deref())
        .map(str::trim)
        .is_some_and(|target| !target.is_empty())
}

fn active_bound_targets(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> Vec<&str> {
    let mut targets = Vec::new();
    if let Some(facts) = session_snapshot.active_observed_facts.as_ref() {
        if let Some(target) = facts
            .bound_target
            .as_deref()
            .map(str::trim)
            .filter(|target| !target.is_empty())
        {
            targets.push(target);
        }
    }
    if let Some(frame) = session_snapshot.active_followup_frame.as_ref() {
        if matches!(
            frame.op_kind,
            crate::followup_frame::FollowupOpKind::Read
                | crate::followup_frame::FollowupOpKind::List
        ) {
            if let Some(target) = frame
                .bound_target
                .as_deref()
                .map(str::trim)
                .filter(|target| !target.is_empty())
            {
                targets.push(target);
            }
        }
    }
    targets
}

fn single_component_locator_hint(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\''
                | '`'
                | ','
                | '，'
                | '。'
                | ':'
                | '：'
                | ';'
                | '；'
                | '('
                | ')'
                | '（'
                | '）'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '《'
                | '》'
        )
    });
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || std::path::Path::new(trimmed).is_absolute()
        || std::path::Path::new(trimmed).components().count() != 1
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn path_basename_eq(path: &str, basename: &str) -> bool {
    let Some(candidate) = single_component_locator_hint(basename) else {
        return false;
    };
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case(&candidate))
        .unwrap_or(false)
}

fn active_bound_target_for_locator_hint<'a>(
    session_snapshot: &'a crate::conversation_state::ActiveSessionSnapshot,
    locator_hint: &str,
) -> Option<&'a str> {
    let hint = single_component_locator_hint(locator_hint)?;
    active_bound_targets(session_snapshot)
        .into_iter()
        .find(|target| path_basename_eq(target, &hint))
}

fn prebind_active_bound_target_from_matching_locator_hint(
    route_result: &mut crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if route_result.needs_clarify
        || !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind)
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        return false;
    }
    let locator_hint = route_result.output_contract.locator_hint.trim();
    let Some(target) = active_bound_target_for_locator_hint(session_snapshot, locator_hint) else {
        return false;
    };
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = target.to_string();
    append_route_reason(
        route_result,
        "active_bound_target_prebound_from_matching_locator_hint",
    );
    true
}

fn active_bound_target_semantic_kind_can_prebind(kind: crate::OutputSemanticKind) -> bool {
    matches!(
        kind,
        crate::OutputSemanticKind::ContentExcerptSummary
            | crate::OutputSemanticKind::ContentExcerptWithSummary
            | crate::OutputSemanticKind::ContentPresenceCheck
            | crate::OutputSemanticKind::DirectoryPurposeSummary
            | crate::OutputSemanticKind::ExcerptKindJudgment
    )
}

fn locator_kind_for_bound_target(target: &str) -> crate::OutputLocatorKind {
    if target.starts_with("http://") || target.starts_with("https://") {
        crate::OutputLocatorKind::Url
    } else {
        crate::OutputLocatorKind::Path
    }
}

fn prebind_active_bound_target_for_locatorless_content_evidence(
    route_result: &mut crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if route_result.needs_clarify
        || !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        || !matches!(
            route_result.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        )
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || !active_bound_target_semantic_kind_can_prebind(
            route_result.output_contract.semantic_kind,
        )
    {
        return false;
    }
    let Some(target) = active_bound_targets(session_snapshot)
        .into_iter()
        .next()
        .map(ToString::to_string)
    else {
        return false;
    };
    route_result.output_contract.locator_kind = locator_kind_for_bound_target(&target);
    route_result.output_contract.locator_hint = target;
    append_route_reason(
        route_result,
        "active_bound_target_prebound_for_locatorless_content_evidence",
    );
    true
}

const SESSION_ALIAS_LOCATOR_PREBOUND_FROM_CURRENT_REQUEST: &str =
    "session_alias_locator_prebound_from_current_request";

fn prebind_session_alias_locator_from_current_request(
    prompt: &str,
    route_result: &mut crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    let should_bind_locator = route_result.needs_clarify
        || route_result.is_execute_gate()
        || route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        );
    if !should_bind_locator
        || semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind)
    {
        return false;
    }
    let Some(conversation_state) = session_snapshot.conversation_state.as_ref() else {
        return false;
    };
    let Some(binding) = crate::conversation_state::single_alias_binding_mentioned_in_prompt(
        &conversation_state.alias_bindings,
        prompt,
    ) else {
        return false;
    };
    let target = binding.target.trim();
    if target.is_empty() {
        return false;
    }
    let locator_kind = if target.starts_with("http://") || target.starts_with("https://") {
        crate::OutputLocatorKind::Url
    } else {
        crate::OutputLocatorKind::Path
    };
    if route_result.needs_clarify || route_result.is_chat_gate() {
        return promote_clarify_observation_to_execute_with_locator(
            route_result,
            locator_kind,
            target.to_string(),
            SESSION_ALIAS_LOCATOR_PREBOUND_FROM_CURRENT_REQUEST,
        );
    }
    route_result.output_contract.locator_kind = locator_kind;
    route_result.output_contract.locator_hint = target.to_string();
    append_route_reason(
        route_result,
        SESSION_ALIAS_LOCATOR_PREBOUND_FROM_CURRENT_REQUEST,
    );
    true
}

fn deictic_memory_only_route_should_force_clarify(
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if execute_route_without_input_locator_should_plan(route_result) {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if !state_patch_requires_deictic_locator_clarify(turn_analysis)
        || surface.has_concrete_locator_hint()
        || surface.has_delivery_token_reference()
    {
        return false;
    }
    if state_patch_allows_deictic_locator_guard_bypass(turn_analysis) {
        return false;
    }
    if session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot) {
        return false;
    }
    if route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && !active_observed_facts_have_bound_target(session_snapshot)
    {
        return false;
    }
    route_result.is_execute_gate()
        || route_result.output_contract.requires_content_evidence
        || route_result.wants_file_delivery
        || route_result.output_contract.delivery_required
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
}

fn unbound_model_context_target_route_should_force_clarify(
    state: &AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
        )
        || raw_command_output_without_locator_can_plan_via_contract(state, prompt, route_result)
        || command_observation_route_has_runtime_evidence(state, prompt, route_result)
        || current_request_has_structural_locator_surface_for_route(state, prompt, route_result)
        || current_request_has_self_contained_structured_payload(prompt)
        || state_patch_allows_deictic_locator_guard_bypass(turn_analysis)
        || session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot)
        || active_session_has_structured_observation_anchor(session_snapshot)
    {
        return false;
    }
    if route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && route_result.output_contract.semantic_kind == crate::OutputSemanticKind::None
    {
        return false;
    }
    if route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && route_result.output_contract.semantic_kind
            == crate::OutputSemanticKind::DirectoryPurposeSummary
    {
        return false;
    }
    if current_workspace_listing_search_route_can_skip_unbound_context_guard(route_result) {
        return false;
    }
    if current_workspace_route_can_skip_unbound_context_guard(prompt, route_result)
        && !route_introduces_unmentioned_distinctive_context_target_except_workspace_root(
            prompt,
            route_result,
            &state.skill_rt.workspace_root,
        )
    {
        return false;
    }
    if semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind)
        && !(route_result.output_contract.semantic_kind
            == crate::OutputSemanticKind::RawCommandOutput
            && !raw_command_output_has_explicit_command(state, prompt)
            && !route_reason_has_marker(
                route_result,
                "execution_recipe_scalar_runtime_tool_observation",
            ))
    {
        return false;
    }
    route_introduces_unmentioned_distinctive_context_target(prompt, route_result)
}

fn current_workspace_listing_search_route_can_skip_unbound_context_guard(
    route_result: &crate::RouteResult,
) -> bool {
    route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && route_result.output_contract.locator_hint.trim().is_empty()
        && !route_result.output_contract.delivery_required
        && matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::FilePaths
                | crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::DirectoryNames
                | crate::OutputSemanticKind::DirectoryEntryGroups
                | crate::OutputSemanticKind::HiddenEntriesCheck
        )
}

fn raw_command_output_without_locator_can_plan_via_contract(
    state: &AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    if !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || route_result.output_contract.response_shape == crate::OutputResponseShape::Scalar
        || route_result.output_contract.semantic_kind != crate::OutputSemanticKind::RawCommandOutput
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
        )
        || raw_command_output_has_explicit_command(state, prompt)
    {
        return false;
    }
    crate::contract_matrix::final_answer_shape_for_output_contract(&route_result.output_contract)
        .is_some_and(|shape| shape.allows_model_language())
}

fn current_workspace_route_can_skip_unbound_context_guard(
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && !is_bare_topic_only_prompt(prompt)
        && super::semantic_kind_can_bind_workspace_child_locator(
            route_result.output_contract.semantic_kind,
        )
}

fn unbound_targeted_evidence_route_should_force_clarify(
    prompt: &str,
    route_result: &crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if current_workspace_target_binding_should_force_clarify(prompt, route_result, session_snapshot)
    {
        return true;
    }
    if current_request_has_concrete_locator_surface(prompt)
        || current_request_has_self_contained_structured_payload(prompt)
        || session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot)
        || route_result.needs_clarify
        || route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        return false;
    }
    if route_result.output_contract.requires_content_evidence
        && !semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind)
    {
        if route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
            && route_result.output_contract.semantic_kind
                == crate::OutputSemanticKind::DirectoryPurposeSummary
        {
            return false;
        }
        if route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
            && route_result.output_contract.semantic_kind == crate::OutputSemanticKind::None
            && !route_introduces_unmentioned_distinctive_context_target(prompt, route_result)
        {
            return false;
        }
        return true;
    }
    matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::ScalarCount
    ) || matches!(
        route_result.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::DirectoryLookup
            | crate::OutputDeliveryIntent::DirectoryBatchFiles
    )
}

fn current_workspace_target_binding_should_force_clarify(
    prompt: &str,
    route_result: &crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if route_result.needs_clarify
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::CurrentWorkspace
        || !route_result.output_contract.requires_content_evidence
        || current_request_has_concrete_locator_surface(prompt)
        || current_request_has_self_contained_structured_payload(prompt)
        || session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot)
        || semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind)
        || current_workspace_scope_observation_can_execute_without_locator(prompt, route_result)
    {
        return false;
    }
    matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::ScalarCount
            | crate::OutputSemanticKind::ContentExcerptSummary
            | crate::OutputSemanticKind::ContentExcerptWithSummary
    ) || matches!(
        route_result.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::DirectoryLookup
            | crate::OutputDeliveryIntent::DirectoryBatchFiles
    )
}

fn current_workspace_scope_observation_can_execute_without_locator(
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    if route_introduces_unmentioned_distinctive_context_target(prompt, route_result) {
        return false;
    }
    if route_result.output_contract.locator_hint.trim().is_empty()
        && matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::ScalarCount
                | crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::ContentExcerptWithSummary
        )
    {
        return false;
    }
    matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
            | crate::OutputSemanticKind::ScalarCount
            | crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::DirectoryNames
            | crate::OutputSemanticKind::DirectoryEntryGroups
            | crate::OutputSemanticKind::FilePaths
            | crate::OutputSemanticKind::WorkspaceProjectSummary
            | crate::OutputSemanticKind::DirectoryPurposeSummary
            | crate::OutputSemanticKind::HiddenEntriesCheck
            | crate::OutputSemanticKind::RecentScalarEqualityCheck
    )
}

fn promote_broad_current_workspace_content_summary_to_directory_purpose(
    prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.is_execute_gate()
        || route_result.needs_clarify
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::CurrentWorkspace
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
        )
        || !matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::ContentExcerptWithSummary
        )
        || current_request_has_concrete_locator_surface(prompt)
        || current_request_has_self_contained_structured_payload(prompt)
        || route_introduces_unmentioned_distinctive_context_target(prompt, route_result)
    {
        return false;
    }
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    append_route_reason(
        route_result,
        "broad_current_workspace_content_summary_repaired_to_directory_purpose_summary",
    );
    true
}

fn current_request_has_concrete_locator_surface(prompt: &str) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    surface.has_explicit_path_or_url()
        || surface.has_single_filename_candidate()
        || surface.has_filename_candidates()
        || surface.locator_target_pair.is_some()
        || surface.has_delivery_token_reference()
        || surface.is_structural_locator_only_reply()
}

fn route_requires_existing_file_delivery(route_result: &crate::RouteResult) -> bool {
    if matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::GeneratedFileDelivery
    ) {
        return false;
    }
    route_result.wants_file_delivery
        || route_result.output_contract.delivery_required
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        || matches!(
            route_result.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::FileSingle
        )
}

fn current_request_has_file_delivery_locator_binding(state: &AppState, prompt: &str) -> bool {
    current_request_has_concrete_locator_surface(prompt)
        || current_request_resolves_workspace_child_locator(state, prompt).is_some()
}

fn unbound_existing_file_delivery_route_should_force_clarify(
    state: &AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
    has_authoritative_deictic_anchor: bool,
) -> bool {
    if route_result.needs_clarify
        || has_authoritative_deictic_anchor
        || route_reason_has_marker(
            route_result,
            "file_delivery_locator_prebound_from_resolved_prompt_path",
        )
        || route_reason_has_marker(
            route_result,
            "file_delivery_locator_prebound_from_recent_ordered_resolved_prompt",
        )
        || route_reason_has_marker(
            route_result,
            "direct_file_delivery_locator_prebound_before_deictic_guard",
        )
        || !route_requires_existing_file_delivery(route_result)
        || current_request_has_file_delivery_locator_binding(state, prompt)
    {
        return false;
    }
    true
}

fn current_request_resolves_workspace_child_locator(
    state: &AppState,
    prompt: &str,
) -> Option<String> {
    let explicit_path_locators =
        crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(prompt)
            .into_iter()
            .filter(|locator| matches!(locator.locator_kind, crate::OutputLocatorKind::Path))
            .collect::<Vec<_>>();
    if !explicit_path_locators.is_empty() {
        return explicit_path_locators.into_iter().find_map(|locator| {
            resolve_existing_workspace_locator_hint(state, &locator.locator_hint)
        });
    }
    super::try_resolve_workspace_child_locator_from_text(
        &state.skill_rt.workspace_root,
        &state.skill_rt.default_locator_search_dir,
        prompt,
    )
}

fn current_request_has_structural_locator_surface_for_route(
    state: &AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    let has_concrete_locator_surface = current_request_has_concrete_locator_surface(prompt);
    if surface.has_deictic_reference() && !has_concrete_locator_surface {
        return false;
    }
    has_concrete_locator_surface
        || (route_result.output_contract.requires_content_evidence
            && !command_observation_route_has_runtime_evidence(state, prompt, route_result)
            && workspace_child_locator_surface_can_bind_route(route_result)
            && current_request_resolves_workspace_child_locator(state, prompt).is_some())
}

fn workspace_child_locator_surface_can_bind_route(route_result: &crate::RouteResult) -> bool {
    !semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind)
        || scalar_equality_route_requests_workspace_child_locator(route_result)
}

fn scalar_equality_route_requests_workspace_child_locator(
    route_result: &crate::RouteResult,
) -> bool {
    route_result.output_contract.requires_content_evidence
        && route_result.output_contract.semantic_kind
            == crate::OutputSemanticKind::RecentScalarEqualityCheck
        && locator_kind_accepts_workspace_child_prebind(route_result.output_contract.locator_kind)
}

fn route_should_skip_workspace_child_prebind(route_result: &crate::RouteResult) -> bool {
    semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind)
        && !scalar_equality_route_requests_workspace_child_locator(route_result)
}

fn prebind_workspace_child_locator_from_current_request(
    state: &AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if route_result.needs_clarify
        || !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || is_bare_topic_only_prompt(prompt)
        || !locator_kind_accepts_workspace_child_prebind(route_result.output_contract.locator_kind)
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || route_should_skip_workspace_child_prebind(route_result)
        || command_observation_route_has_runtime_evidence(state, prompt, route_result)
        || archive_unpack_requires_structural_locator_pair(route_result)
    {
        return false;
    }
    let Some(path) = current_request_resolves_workspace_child_locator(state, prompt) else {
        return false;
    };
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = path;
    append_route_reason(
        route_result,
        "workspace_child_locator_prebound_from_current_request",
    );
    true
}

fn locator_kind_accepts_workspace_child_prebind(kind: crate::OutputLocatorKind) -> bool {
    matches!(
        kind,
        crate::OutputLocatorKind::None
            | crate::OutputLocatorKind::Path
            | crate::OutputLocatorKind::Filename
            | crate::OutputLocatorKind::CurrentWorkspace
    )
}

const WORKSPACE_LOCATOR_HINT_PREBOUND_FROM_CURRENT_REQUEST: &str =
    "workspace_locator_hint_prebound_from_current_request";

fn prebind_existing_workspace_locator_hint_from_current_request(
    state: &AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if route_result.needs_clarify
        || !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
        )
        || route_should_skip_workspace_child_prebind(route_result)
        || (!super::semantic_kind_can_bind_workspace_child_locator(
            route_result.output_contract.semantic_kind,
        ) && !scalar_equality_route_requests_workspace_child_locator(route_result))
    {
        return false;
    }
    let locator_hint = route_result.output_contract.locator_hint.trim();
    if locator_hint.is_empty() || !locator_hint_token_present_in_prompt(prompt, locator_hint) {
        return false;
    }
    if !crate::worker::has_explicit_path_or_url_locator_hint(prompt)
        && locator_hint_token_ambiguous_in_workspace(state, locator_hint)
    {
        return false;
    }
    let Some(path) =
        resolve_existing_or_direct_child_stem_workspace_locator_hint(state, locator_hint)
    else {
        return false;
    };
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = path;
    append_route_reason(
        route_result,
        WORKSPACE_LOCATOR_HINT_PREBOUND_FROM_CURRENT_REQUEST,
    );
    true
}

fn prebind_clarify_workspace_child_locator_from_current_request(
    state: &AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.needs_clarify
        || route_result.output_contract.delivery_required
        || !route_result.output_contract.requires_content_evidence
        || is_bare_topic_only_prompt(prompt)
        || !locator_kind_accepts_workspace_child_prebind(route_result.output_contract.locator_kind)
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || route_should_skip_workspace_child_prebind(route_result)
        || command_observation_route_has_runtime_evidence(state, prompt, route_result)
        || archive_unpack_requires_structural_locator_pair(route_result)
    {
        return false;
    }
    let Some(path) = current_request_resolves_workspace_child_locator(state, prompt) else {
        return false;
    };
    promote_clarify_observation_to_execute_with_locator(
        route_result,
        crate::OutputLocatorKind::Path,
        path,
        "workspace_child_locator_prebound_from_clarify_current_request",
    )
}

fn prebind_workspace_child_locator_from_resolved_prompt(
    state: &AppState,
    resolved_prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.needs_clarify
        || route_result.output_contract.delivery_required
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind)
        || command_observation_route_has_runtime_evidence(state, resolved_prompt, route_result)
        || archive_unpack_requires_structural_locator_pair(route_result)
    {
        return false;
    }
    let Some(path) = resolved_prompt_existing_workspace_locator(state, resolved_prompt) else {
        return false;
    };
    promote_clarify_observation_to_execute_with_locator(
        route_result,
        crate::OutputLocatorKind::Path,
        path,
        "workspace_child_locator_prebound_from_resolved_prompt",
    )
}

fn archive_unpack_requires_structural_locator_pair(route_result: &crate::RouteResult) -> bool {
    route_result.output_contract.semantic_kind == crate::OutputSemanticKind::ArchiveUnpack
}

fn prebind_file_delivery_locator_from_recent_ordered_resolved_prompt(
    state: &AppState,
    resolved_prompt: &str,
    recent_execution_context: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.needs_clarify
        || !route_result.output_contract.delivery_required
        || route_result.output_contract.response_shape != crate::OutputResponseShape::FileToken
        || route_result.output_contract.delivery_intent != crate::OutputDeliveryIntent::FileSingle
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    let Some(path) = resolve_recent_ordered_entry_target_from_resolved_prompt(
        state,
        resolved_prompt,
        recent_execution_context,
    ) else {
        return false;
    };
    route_result.wants_file_delivery = true;
    promote_clarify_observation_to_execute_with_locator(
        route_result,
        crate::OutputLocatorKind::Path,
        path,
        "file_delivery_locator_prebound_from_recent_ordered_resolved_prompt",
    )
}

fn prebind_file_delivery_locator_from_resolved_prompt_path(
    state: &AppState,
    resolved_prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.output_contract.delivery_required
        || route_result.output_contract.response_shape != crate::OutputResponseShape::FileToken
        || route_result.output_contract.delivery_intent != crate::OutputDeliveryIntent::FileSingle
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    let Some(path) = resolved_prompt_existing_workspace_locator(state, resolved_prompt)
        .or_else(|| {
            resolved_prompt_existing_workspace_locator(state, &route_result.resolved_intent)
        })
        .filter(|path| std::path::Path::new(path).is_file())
    else {
        return false;
    };
    route_result.wants_file_delivery = true;
    if route_result.needs_clarify || route_result.is_chat_gate() {
        return promote_clarify_observation_to_execute_with_locator(
            route_result,
            crate::OutputLocatorKind::Path,
            path,
            "file_delivery_locator_prebound_from_resolved_prompt_path",
        );
    }
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = path;
    append_route_reason(
        route_result,
        "file_delivery_locator_prebound_from_resolved_prompt_path",
    );
    true
}

fn resolve_recent_ordered_entry_target_from_resolved_prompt(
    state: &AppState,
    resolved_prompt: &str,
    recent_execution_context: &str,
) -> Option<String> {
    let context = recent_execution_context.trim();
    if resolved_prompt.trim().is_empty() || context.is_empty() || context == "<none>" {
        return None;
    }
    let mut sources = recent_execution_result_segments(context);
    sources.push(context.to_string());
    for source in sources {
        for entry in crate::followup_frame::extract_ordered_entries_from_text(&source) {
            if !text_mentions_locator_identity(resolved_prompt, &entry) {
                continue;
            }
            if let Some(path) = resolve_existing_workspace_locator_hint(state, &entry)
                .or_else(|| resolve_unique_workspace_file_by_entry_identity(state, &entry))
                .or_else(|| {
                    super::try_resolve_workspace_child_locator_from_text(
                        &state.skill_rt.workspace_root,
                        &state.skill_rt.default_locator_search_dir,
                        resolved_prompt,
                    )
                })
                .or_else(|| {
                    super::try_resolve_workspace_child_locator_from_text(
                        &state.skill_rt.workspace_root,
                        &state.skill_rt.default_locator_search_dir,
                        &entry,
                    )
                })
            {
                return Some(path);
            }
        }
    }
    None
}

fn resolve_unique_workspace_file_by_entry_identity(
    state: &AppState,
    entry: &str,
) -> Option<String> {
    let identities = locator_identity_candidates(entry);
    if identities.is_empty() {
        return None;
    }
    let mut roots = vec![state.skill_rt.workspace_root.clone()];
    if state.skill_rt.default_locator_search_dir != state.skill_rt.workspace_root {
        roots.push(state.skill_rt.default_locator_search_dir.clone());
    }
    let mut matches = Vec::new();
    let mut scanned = 0usize;
    for root in roots {
        scan_unique_workspace_file_by_identity(
            &root,
            &identities,
            state.skill_rt.locator_scan_max_depth,
            state.skill_rt.locator_scan_max_files,
            &mut scanned,
            &mut matches,
        );
        matches.sort();
        matches.dedup();
        if matches.len() > 1 || scanned > state.skill_rt.locator_scan_max_files {
            return None;
        }
    }
    matches.pop()
}

fn scan_unique_workspace_file_by_identity(
    root: &std::path::Path,
    identities: &[String],
    max_depth: usize,
    max_files: usize,
    scanned: &mut usize,
    matches: &mut Vec<String>,
) {
    if matches.len() > 1 || *scanned > max_files || !root.is_dir() {
        return;
    }
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if matches.len() > 1 || *scanned > max_files {
            return;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth < max_depth {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            *scanned += 1;
            if *scanned > max_files {
                return;
            }
            let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let normalized = normalize_locator_identity_token(file_name);
            if identities.iter().any(|identity| identity == &normalized) {
                let canonical = path.canonicalize().unwrap_or(path);
                matches.push(canonical.display().to_string());
                if matches.len() > 1 {
                    return;
                }
            }
        }
    }
}

fn text_mentions_locator_identity(text: &str, locator: &str) -> bool {
    let normalized_text = text.to_ascii_lowercase();
    locator_identity_candidates(locator)
        .into_iter()
        .any(|identity| identity.chars().count() >= 3 && normalized_text.contains(&identity))
}

fn promote_clarify_observation_to_execute_with_locator(
    route_result: &mut crate::RouteResult,
    locator_kind: crate::OutputLocatorKind,
    locator_hint: String,
    reason: &'static str,
) -> bool {
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.locator_kind = locator_kind;
    route_result.output_contract.locator_hint = locator_hint;
    route_result.needs_clarify = false;
    route_result.clarify_question.clear();
    route_result.set_planner_execute_finalize(
        crate::post_route_policy::content_evidence_execution_finalize_style(
            &route_result.output_contract,
            false,
        )
        .unwrap_or(crate::ActFinalizeStyle::ChatWrapped),
    );
    append_route_reason(route_result, reason);
    true
}

fn prebind_quantity_compare_directory_pair_from_current_request(
    state: &AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
    {
        return false;
    }
    let semantic_quantity_comparison =
        route_result.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison;
    let semantic_recent_scalar_comparison = route_result.output_contract.semantic_kind
        == crate::OutputSemanticKind::RecentScalarEqualityCheck;
    if !semantic_quantity_comparison
        && !semantic_recent_scalar_comparison
        && route_result.output_contract.semantic_kind != crate::OutputSemanticKind::None
    {
        return false;
    }
    if !semantic_quantity_comparison && prompt_surface_contains_archive_locator_pair(prompt) {
        return false;
    }
    if !semantic_quantity_comparison
        && !route_result.needs_clarify
        && !route_result.is_execute_gate()
    {
        return false;
    }
    let path_pair = if semantic_recent_scalar_comparison {
        workspace_path_pair_from_current_request(state, prompt)
            .filter(|(left, right)| {
                std::path::Path::new(left).is_dir() && std::path::Path::new(right).is_dir()
            })
            .or_else(|| workspace_directory_pair_from_current_request(state, prompt, false))
    } else if semantic_quantity_comparison {
        workspace_path_pair_from_current_request(state, prompt).or_else(|| {
            if route_has_single_existing_locator_hint(state, route_result) {
                None
            } else {
                workspace_directory_pair_from_current_request(
                    state,
                    prompt,
                    !semantic_quantity_comparison,
                )
            }
        })
    } else {
        workspace_directory_pair_from_current_request(state, prompt, !semantic_quantity_comparison)
    };
    let Some((left, right)) = path_pair else {
        return false;
    };
    if semantic_recent_scalar_comparison {
        route_result.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
        route_result.output_contract.response_shape = crate::OutputResponseShape::Strict;
    }
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = format!("{left} | {right}");
    route_result.output_contract.requires_content_evidence = true;
    route_result.needs_clarify = false;
    route_result.clarify_question.clear();
    route_result.set_planner_execute_finalize(
        crate::post_route_policy::content_evidence_execution_finalize_style(
            &route_result.output_contract,
            false,
        )
        .unwrap_or(crate::ActFinalizeStyle::ChatWrapped),
    );
    append_route_reason(
        route_result,
        if semantic_quantity_comparison {
            "quantity_compare_path_pair_prebound_from_current_request"
        } else if semantic_recent_scalar_comparison {
            "recent_scalar_directory_pair_promoted_to_quantity_comparison"
        } else {
            "directory_pair_prebound_from_current_request"
        },
    );
    true
}

fn prompt_surface_contains_archive_locator_pair(prompt: &str) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    let Some((left, right)) = surface.locator_target_pair.as_ref() else {
        return false;
    };
    supported_archive_locator_path(left) ^ supported_archive_locator_path(right)
}

fn supported_archive_locator_path(path: &str) -> bool {
    let path = path.trim().to_ascii_lowercase();
    path.ends_with(".zip") || path.ends_with(".tar.gz") || path.ends_with(".tgz")
}

fn route_has_single_existing_locator_hint(
    state: &AppState,
    route_result: &crate::RouteResult,
) -> bool {
    let locators = crate::task_contract::target_locators_for_route(route_result);
    if locators.len() > 1 {
        return false;
    }
    let hint = route_result.output_contract.locator_hint.trim();
    !hint.is_empty() && resolve_existing_workspace_locator_hint(state, hint).is_some()
}

fn workspace_path_pair_from_current_request(
    state: &AppState,
    prompt: &str,
) -> Option<(String, String)> {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if let Some((left, right)) = surface.locator_target_pair.as_ref() {
        let left = resolve_existing_workspace_locator_hint(state, left)?;
        let right = resolve_existing_workspace_locator_hint(state, right)?;
        return (!left.eq_ignore_ascii_case(&right)).then_some((left, right));
    }
    workspace_existing_locator_pair_from_prompt_tokens(state, prompt)
}

fn workspace_existing_locator_pair_from_prompt_tokens(
    state: &AppState,
    prompt: &str,
) -> Option<(String, String)> {
    let mut out = Vec::new();
    for token in prompt
        .split_whitespace()
        .flat_map(split_structural_locator_token)
    {
        let token = token
            .trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\''
                        | '`'
                        | ','
                        | '，'
                        | '。'
                        | ':'
                        | '：'
                        | ';'
                        | '；'
                        | '('
                        | ')'
                        | '（'
                        | '）'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '<'
                        | '>'
                        | '《'
                        | '》'
                )
            })
            .trim();
        if token.len() < 2
            || crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(token)
        {
            continue;
        }
        let Some(path) = resolve_existing_workspace_locator_hint(state, token) else {
            continue;
        };
        if out
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(&path))
        {
            continue;
        }
        out.push(path);
        if out.len() > 2 {
            return None;
        }
    }
    (out.len() == 2).then(|| (out.remove(0), out.remove(0)))
}

fn split_structural_locator_token(token: &str) -> impl Iterator<Item = &str> {
    token.split(|ch: char| {
        matches!(
            ch,
            ',' | '，'
                | '。'
                | ';'
                | '；'
                | ':'
                | '：'
                | '('
                | ')'
                | '（'
                | '）'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '《'
                | '》'
        )
    })
}

fn workspace_directory_pair_from_current_request(
    state: &AppState,
    prompt: &str,
    require_strong_locator_tokens: bool,
) -> Option<(String, String)> {
    let mut out = Vec::new();
    for token in structural_locator_token_candidates(prompt) {
        if require_strong_locator_tokens && !strong_structural_locator_token(&token) {
            continue;
        }
        let Some(path) = resolve_unique_directory_basename_under(
            &state.skill_rt.workspace_root,
            &token,
            directory_pair_locator_scan_limit(state),
        ) else {
            continue;
        };
        if !out
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(&path))
        {
            out.push(path);
        }
        if out.len() >= 2 {
            break;
        }
    }
    (out.len() == 2).then(|| (out.remove(0), out.remove(0)))
}

fn directory_pair_locator_scan_limit(state: &AppState) -> usize {
    state.skill_rt.locator_scan_max_files.max(50_000)
}

fn strong_structural_locator_token(token: &str) -> bool {
    token.contains(['_', '-', '.']) || token.chars().any(|ch| ch.is_ascii_digit())
}

fn structural_locator_token_candidates(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
            cur.push(ch.to_ascii_lowercase());
        } else if !cur.is_empty() {
            push_structural_locator_token(&cur, &mut out);
            cur.clear();
            if out.len() >= 16 {
                break;
            }
        }
    }
    if !cur.is_empty() && out.len() < 16 {
        push_structural_locator_token(&cur, &mut out);
    }
    out
}

fn push_structural_locator_token(token: &str, out: &mut Vec<String>) {
    let token = token
        .trim_matches(|ch: char| matches!(ch, '.' | '"' | '\'' | '`'))
        .trim();
    if token.len() < 2
        || token.contains('/')
        || token.contains('\\')
        || crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(token)
        || out.iter().any(|existing| existing == token)
    {
        return;
    }
    out.push(token.to_string());
}

fn resolve_unique_directory_basename_under(
    workspace_root: &std::path::Path,
    name: &str,
    max_visits: usize,
) -> Option<String> {
    if !workspace_root.is_dir() || name.trim().is_empty() {
        return None;
    }
    let mut stack = vec![workspace_root.to_path_buf()];
    let mut matches = Vec::new();
    let mut visits = 0usize;
    while let Some(dir) = stack.pop() {
        visits = visits.saturating_add(1);
        if visits > max_visits {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut children = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let file_type = entry.file_type().ok()?;
                file_type.is_dir().then(|| entry.path())
            })
            .collect::<Vec<_>>();
        children.sort();
        for child in children.into_iter().rev() {
            if child
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|file_name| file_name.eq_ignore_ascii_case(name))
            {
                let canonical = child.canonicalize().unwrap_or(child.clone());
                matches.push(canonical.display().to_string());
                if matches.len() > 1 {
                    return None;
                }
            }
            stack.push(child);
        }
    }
    matches.pop()
}

fn resolved_prompt_existing_workspace_locator(
    state: &AppState,
    resolved_prompt: &str,
) -> Option<String> {
    crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(
        resolved_prompt,
    )
    .into_iter()
    .filter(|locator| matches!(locator.locator_kind, crate::OutputLocatorKind::Path))
    .filter_map(|locator| resolve_existing_workspace_locator_hint(state, &locator.locator_hint))
    .next()
}

fn resolve_existing_workspace_locator_hint(state: &AppState, locator_hint: &str) -> Option<String> {
    let hint = locator_hint.trim();
    if hint.is_empty() || hint.contains('\n') {
        return None;
    }
    let path = std::path::Path::new(hint);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(path)
    };
    if !path.exists() {
        return None;
    }
    let canonical_path = path.canonicalize().unwrap_or(path);
    let canonical_root = state
        .skill_rt
        .workspace_root
        .canonicalize()
        .unwrap_or_else(|_| state.skill_rt.workspace_root.clone());
    canonical_path
        .starts_with(canonical_root)
        .then(|| canonical_path.display().to_string())
}

fn resolve_existing_or_direct_child_stem_workspace_locator_hint(
    state: &AppState,
    locator_hint: &str,
) -> Option<String> {
    resolve_existing_workspace_locator_hint(state, locator_hint)
        .or_else(|| resolve_direct_child_stem_workspace_locator_hint(state, locator_hint))
}

fn resolve_direct_child_stem_workspace_locator_hint(
    state: &AppState,
    locator_hint: &str,
) -> Option<String> {
    let hint = locator_hint.trim();
    if hint.is_empty() || hint.contains('\n') {
        return None;
    }
    let hint_path = std::path::Path::new(hint);
    let file_name = locator_component_token(hint_path.file_name()?.to_str()?)?;
    if file_name.contains('.') {
        return None;
    }
    let parent = hint_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                state.skill_rt.workspace_root.join(path)
            }
        })
        .unwrap_or_else(|| state.skill_rt.workspace_root.clone());
    let canonical_root = state
        .skill_rt
        .workspace_root
        .canonicalize()
        .unwrap_or_else(|_| state.skill_rt.workspace_root.clone());
    let canonical_parent = parent.canonicalize().ok()?;
    if !canonical_parent.starts_with(&canonical_root) {
        return None;
    }
    let mut matches = std::fs::read_dir(&canonical_parent)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_file() {
                return None;
            }
            let path = entry.path();
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem.eq_ignore_ascii_case(&file_name))
                .then_some(path)
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return None;
    }
    let path = matches.pop()?;
    let canonical_path = path.canonicalize().unwrap_or(path);
    canonical_path
        .starts_with(canonical_root)
        .then(|| canonical_path.display().to_string())
}

fn locator_hint_token_present_in_prompt(prompt: &str, locator_hint: &str) -> bool {
    let hint_tokens = locator_hint_match_tokens(locator_hint);
    if hint_tokens.is_empty() {
        return false;
    }
    structural_locator_token_candidates(prompt)
        .into_iter()
        .any(|token| {
            hint_tokens
                .iter()
                .any(|hint_token| token.eq_ignore_ascii_case(hint_token))
        })
}

fn locator_hint_token_ambiguous_in_workspace(state: &AppState, locator_hint: &str) -> bool {
    let hint_tokens = locator_hint_match_tokens(locator_hint);
    if hint_tokens.is_empty() {
        return false;
    }
    let mut roots = Vec::new();
    push_unique_canonical_locator_root(&mut roots, state.skill_rt.workspace_root.clone());
    push_unique_canonical_locator_root(
        &mut roots,
        state.skill_rt.default_locator_search_dir.clone(),
    );

    let mut matches = Vec::new();
    for root in roots {
        collect_locator_token_matches(
            &root,
            &hint_tokens,
            state.skill_rt.locator_scan_max_depth,
            state.skill_rt.locator_scan_max_files,
            &mut matches,
        );
        matches.sort();
        matches.dedup();
        if matches.len() > 1 {
            return true;
        }
    }
    false
}

fn push_unique_canonical_locator_root(
    roots: &mut Vec<std::path::PathBuf>,
    root: std::path::PathBuf,
) {
    let canonical = root.canonicalize().unwrap_or(root);
    if !roots.iter().any(|existing| existing == &canonical) {
        roots.push(canonical);
    }
}

fn collect_locator_token_matches(
    root: &std::path::Path,
    hint_tokens: &[String],
    max_depth: usize,
    max_files: usize,
    out: &mut Vec<String>,
) {
    if !root.is_dir() {
        return;
    }
    let mut scanned = 0usize;
    let mut queue = std::collections::VecDeque::from([(root.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if depth < max_depth {
                    queue.push_back((path, depth + 1));
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            scanned += 1;
            if locator_path_matches_any_hint_token(&path, hint_tokens) {
                out.push(path.canonicalize().unwrap_or(path).display().to_string());
                if out.len() > 1 {
                    return;
                }
            }
            if scanned >= max_files {
                return;
            }
        }
    }
}

fn locator_path_matches_any_hint_token(path: &std::path::Path, hint_tokens: &[String]) -> bool {
    let file_name = path.file_name().and_then(|value| value.to_str());
    let file_stem = path.file_stem().and_then(|value| value.to_str());
    hint_tokens.iter().any(|token| {
        file_name.is_some_and(|name| name.eq_ignore_ascii_case(token))
            || (!token.contains('.')
                && file_stem.is_some_and(|stem| stem.eq_ignore_ascii_case(token)))
    })
}

fn locator_hint_match_tokens(locator_hint: &str) -> Vec<String> {
    let hint = locator_hint.trim();
    if hint.is_empty() || hint.contains('\n') {
        return Vec::new();
    }
    let path = std::path::Path::new(hint);
    let mut out = Vec::new();
    if let Some(file_name) = path
        .file_name()
        .and_then(|value| value.to_str())
        .and_then(locator_component_token)
    {
        push_unique_locator_hint_match_token(&mut out, file_name);
    }
    if let Some(stem) = path
        .file_stem()
        .and_then(|value| value.to_str())
        .and_then(locator_component_token)
    {
        push_unique_locator_hint_match_token(&mut out, stem);
    }
    out
}

fn push_unique_locator_hint_match_token(out: &mut Vec<String>, token: String) {
    if !out
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&token))
    {
        out.push(token);
    }
}

fn locator_component_token(value: &str) -> Option<String> {
    let token = single_component_locator_hint(value)?;
    if token.len() < 2
        || token.chars().any(char::is_whitespace)
        || !token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        return None;
    }
    Some(token)
}

fn prebind_workspace_root_locator_from_resolved_prompt(
    state: &AppState,
    resolved_prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if route_result.needs_clarify
        || !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind)
        || !super::semantic_kind_can_bind_workspace_child_locator(
            route_result.output_contract.semantic_kind,
        )
    {
        return false;
    }
    if !text_contains_workspace_root_locator(resolved_prompt, &state.skill_rt.workspace_root)
        && !text_contains_workspace_root_locator(
            &route_result.resolved_intent,
            &state.skill_rt.workspace_root,
        )
    {
        return false;
    }

    route_result.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route_result.output_contract.locator_hint = state.skill_rt.workspace_root.display().to_string();
    append_route_reason(
        route_result,
        "workspace_root_locator_prebound_from_resolved_prompt",
    );
    true
}

fn text_contains_workspace_root_locator(text: &str, workspace_root: &std::path::Path) -> bool {
    crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(text)
        .into_iter()
        .any(|locator| {
            matches!(locator.locator_kind, crate::OutputLocatorKind::Path)
                && locator_path_points_to_workspace_root(&locator.locator_hint, workspace_root)
        })
}

fn locator_path_points_to_workspace_root(
    locator_hint: &str,
    workspace_root: &std::path::Path,
) -> bool {
    let hint = locator_hint.trim();
    if hint.is_empty() || hint.contains('\n') {
        return false;
    }
    let candidate = std::path::Path::new(hint);
    let candidate = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace_root.join(candidate)
    };
    let candidate = candidate.canonicalize().unwrap_or(candidate);
    let workspace_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    candidate == workspace_root
}

fn route_has_model_supplied_concrete_locator(
    route_result: &crate::RouteResult,
    resolved_prompt: &str,
) -> bool {
    let contract = &route_result.output_contract;
    let contract_locator = matches!(
        contract.locator_kind,
        crate::OutputLocatorKind::Path
            | crate::OutputLocatorKind::Filename
            | crate::OutputLocatorKind::Url
    ) && !contract.locator_hint.trim().is_empty();
    let resolved_prompt_without_answer_candidate =
        strip_embedded_answer_candidate_lines(resolved_prompt);
    let resolved_intent_without_answer_candidate =
        strip_embedded_answer_candidate_lines(&route_result.resolved_intent);
    contract_locator
        || crate::worker::has_explicit_path_or_url_locator_hint(
            &resolved_prompt_without_answer_candidate,
        )
        || crate::worker::has_explicit_path_or_url_locator_hint(
            &resolved_intent_without_answer_candidate,
        )
}

fn strip_embedded_answer_candidate_lines(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("answer_candidate:"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn background_only_locator_route_should_force_clarify(
    state: &AppState,
    prompt: &str,
    resolved_prompt: &str,
    recent_execution_context: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if execute_route_without_input_locator_should_plan(route_result)
        || state_patch_allows_deictic_locator_guard_bypass(turn_analysis)
        || session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot)
        || recent_execution_context_has_ordered_entry_target(recent_execution_context, route_result)
        || route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        || route_reason_has_marker(
            route_result,
            WORKSPACE_LOCATOR_HINT_PREBOUND_FROM_CURRENT_REQUEST,
        )
        || !route_has_model_supplied_concrete_locator(route_result, resolved_prompt)
    {
        return false;
    }
    if !state_patch_requires_deictic_locator_clarify(turn_analysis)
        && current_request_has_structural_locator_surface_for_route(state, prompt, route_result)
    {
        return false;
    }
    if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison
        && workspace_directory_pair_from_current_request(state, prompt, false).is_some()
    {
        return false;
    }

    route_result.is_execute_gate()
        || route_result.output_contract.requires_content_evidence
        || route_result.wants_file_delivery
        || route_result.output_contract.delivery_required
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
}

fn downgrade_background_locator_clarify_to_recent_observed_chat(
    route_result: &mut crate::RouteResult,
    recent_execution_context: &str,
) -> bool {
    if !route_result.needs_clarify
        || !route_reason_has_marker(route_result, "background_locator_requires_clarify")
        || route_result.wants_file_delivery
        || route_result.output_contract.delivery_required
        || route_result.output_contract.semantic_kind != crate::OutputSemanticKind::FileNames
        || recent_execution_result_segments(recent_execution_context).len() < 2
    {
        return false;
    }
    route_result.needs_clarify = false;
    route_result.set_first_layer_decision(crate::FirstLayerDecision::DirectAnswer);
    route_result.clarify_question.clear();
    route_result.output_contract = crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::Scalar,
        ..Default::default()
    };
    append_route_reason(route_result, "active_observed_output_chat_repair");
    append_route_reason(
        route_result,
        "recent_observed_results_background_locator_chat_repair",
    );
    true
}

fn recent_execution_context_has_ordered_entry_target(
    recent_execution_context: &str,
    route_result: &crate::RouteResult,
) -> bool {
    let context = recent_execution_context.trim();
    if context.is_empty() || context == "<none>" {
        return false;
    }
    let target_identities =
        locator_identity_candidates(route_result.output_contract.locator_hint.trim());
    if target_identities.is_empty() {
        return false;
    }
    let mut sources = recent_execution_result_segments(context);
    sources.push(context.to_string());
    sources.into_iter().any(|source| {
        crate::followup_frame::extract_ordered_entries_from_text(&source)
            .into_iter()
            .any(|entry| {
                locator_identity_candidates(&entry)
                    .into_iter()
                    .any(|entry_identity| target_identities.contains(&entry_identity))
            })
    })
}

fn recent_execution_result_segments(context: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current: Option<String> = None;
    for line in context.lines() {
        if let Some((_, result)) = line.split_once(" result=") {
            if let Some(segment) = current.take().filter(|value| !value.trim().is_empty()) {
                segments.push(segment);
            }
            current = Some(result.trim().to_string());
            continue;
        }
        if line.trim_start().starts_with("- ts=") || line.trim_start().starts_with("### ") {
            if let Some(segment) = current.take().filter(|value| !value.trim().is_empty()) {
                segments.push(segment);
            }
            continue;
        }
        if let Some(segment) = current.as_mut() {
            if !line.trim().is_empty() {
                segment.push('\n');
                segment.push_str(line.trim());
            }
        }
    }
    if let Some(segment) = current.take().filter(|value| !value.trim().is_empty()) {
        segments.push(segment);
    }
    segments
}

fn locator_identity_candidates(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    push_locator_identity(&mut out, trimmed);
    if let Some(name) = std::path::Path::new(trimmed)
        .file_name()
        .and_then(|name| name.to_str())
    {
        push_locator_identity(&mut out, name);
    }
    out
}

fn push_locator_identity(out: &mut Vec<String>, value: &str) {
    let normalized = normalize_locator_identity_token(value);
    if !normalized.is_empty() && !out.contains(&normalized) {
        out.push(normalized);
    }
}

fn semantic_kind_can_execute_without_locator(kind: crate::OutputSemanticKind) -> bool {
    matches!(
        kind,
        crate::OutputSemanticKind::RawCommandOutput
            | crate::OutputSemanticKind::CommandOutputSummary
            | crate::OutputSemanticKind::ServiceStatus
            | crate::OutputSemanticKind::HiddenEntriesCheck
            | crate::OutputSemanticKind::WorkspaceProjectSummary
            | crate::OutputSemanticKind::RecentScalarEqualityCheck
            | crate::OutputSemanticKind::GitCommitSubject
            | crate::OutputSemanticKind::GitRepositoryState
            | crate::OutputSemanticKind::RssNewsFetch
            | crate::OutputSemanticKind::WebSearchSummary
            | crate::OutputSemanticKind::WeatherQuery
            | crate::OutputSemanticKind::MarketQuote
            | crate::OutputSemanticKind::ImageUnderstanding
            | crate::OutputSemanticKind::PublishingPreview
            | crate::OutputSemanticKind::PackageManagerDetection
            | crate::OutputSemanticKind::DockerPs
            | crate::OutputSemanticKind::DockerImages
            | crate::OutputSemanticKind::DockerLogs
            | crate::OutputSemanticKind::DockerContainerLifecycle
    )
}

fn promote_locatorless_scalar_child_metadata_to_quantity_comparison(
    state: &AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if !route_result.is_execute_gate()
        || route_result.needs_clarify
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || surface.token_count <= 1
        || surface.has_explicit_path_or_url()
        || surface.has_filename_candidates()
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
        )
        || !matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
                | crate::OutputResponseShape::OneSentence
                | crate::OutputResponseShape::Strict
        )
        || !matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::None | crate::OutputSemanticKind::RawCommandOutput
        )
        || (route_result.output_contract.semantic_kind
            == crate::OutputSemanticKind::RawCommandOutput
            && route_reason_has_marker(
                route_result,
                "command_payload_requires_raw_output_execution",
            ))
        || raw_command_output_has_explicit_command(state, prompt)
    {
        return false;
    }
    let Some(path) = current_request_resolves_workspace_child_locator(state, prompt) else {
        return false;
    };
    let resolved_path = std::path::Path::new(&path);
    if resolved_path.is_file()
        && route_result.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
    {
        return false;
    }
    if resolved_path.is_file()
        && route_result.output_contract.semantic_kind == crate::OutputSemanticKind::None
        && route_result.output_contract.response_shape != crate::OutputResponseShape::Scalar
    {
        return false;
    }
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = path;
    route_result.output_contract.delivery_required = false;
    route_result.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    route_result.output_contract.requires_content_evidence = true;
    route_result.set_planner_execute_finalize(
        crate::post_route_policy::content_evidence_execution_finalize_style(
            &route_result.output_contract,
            false,
        )
        .unwrap_or(crate::ActFinalizeStyle::ChatWrapped),
    );
    append_route_reason(
        route_result,
        "locatorless_scalar_child_metadata_promoted_to_quantity_comparison",
    );
    true
}

fn promote_locatorless_git_capability_to_repository_state(
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.is_execute_gate()
        || route_result.needs_clarify
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
        )
        || route_result.output_contract.semantic_kind != crate::OutputSemanticKind::None
        || !route_mentions_git_capability(route_result)
    {
        return false;
    }

    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::GitRepositoryState;
    append_route_reason(
        route_result,
        "locatorless_git_capability_promoted_to_git_repository_state",
    );
    true
}

fn route_mentions_git_capability(route_result: &crate::RouteResult) -> bool {
    ascii_token_present(&route_result.resolved_intent, "git")
        || ascii_token_present(&route_result.route_reason, "git")
}

fn ascii_token_present(text: &str, token: &str) -> bool {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
        .any(|candidate| candidate.eq_ignore_ascii_case(token))
}

fn prebind_runtime_status_scalar_path_to_current_workspace(
    route_result: &mut crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if !route_result.is_execute_gate()
        || route_result.needs_clarify
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || route_result.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || route_result.output_contract.semantic_kind != crate::OutputSemanticKind::ScalarPathOnly
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    let runtime_status_kind = turn_analysis
        .and_then(|analysis| analysis.state_patch.as_ref())
        .and_then(|patch| patch.get("runtime_status_query"))
        .and_then(serde_json::Value::as_object)
        .and_then(|query| query.get("kind"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim);
    let structured_cwd_query = runtime_status_kind.is_some_and(|kind| {
        matches!(
            kind,
            "current_working_directory" | "current_process_cwd" | "process_cwd"
        )
    });
    let status_query_scalar_path = turn_analysis.is_some_and(|analysis| {
        analysis.turn_type == Some(crate::intent_router::TurnType::StatusQuery)
    });
    if !structured_cwd_query
        && !status_query_scalar_path
        && active_ordered_entries_without_structured_ref(session_snapshot, turn_analysis)
    {
        append_route_reason(
            route_result,
            "scalar_path_only_missing_ordered_entry_ref_not_bound_to_current_workspace",
        );
        return false;
    }
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route_result.output_contract.locator_hint.clear();
    let reason = if structured_cwd_query || status_query_scalar_path {
        "runtime_status_scalar_path_bound_to_current_workspace"
    } else {
        "scalar_path_only_without_locator_bound_to_current_workspace"
    };
    append_route_reason(route_result, reason);
    true
}

fn active_ordered_entries_without_structured_ref(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let has_active_ordered_entries = session_snapshot
        .active_followup_frame
        .as_ref()
        .is_some_and(|frame| !frame.ordered_entries.is_empty());
    if !has_active_ordered_entries {
        return false;
    }
    !turn_analysis
        .and_then(|analysis| analysis.state_patch.as_ref())
        .is_some_and(|state_patch| {
            state_patch.get("ordered_entry_ref").is_some()
                || state_patch.get("ordered_entry_reference").is_some()
        })
}

fn promote_locatorless_status_query_to_service_status(
    state: &AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let Some(turn_analysis) = turn_analysis else {
        return false;
    };
    if turn_analysis.turn_type != Some(crate::intent_router::TurnType::StatusQuery)
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
        )
        || !matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::None | crate::OutputSemanticKind::RawCommandOutput
        )
    {
        return false;
    }
    let promotable_gate = route_result.is_execute_gate()
        || (route_result.needs_clarify && route_result.clarify_question.trim().is_empty());
    if !promotable_gate {
        return false;
    }
    if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
        && route_reason_has_marker(
            route_result,
            "command_payload_requires_raw_output_execution",
        )
    {
        return false;
    }
    if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
        && turn_analysis_has_runtime_status_query(turn_analysis)
    {
        return false;
    }
    if route_result.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && route_reason_has_marker(
            route_result,
            "execution_recipe_scalar_runtime_tool_observation",
        )
    {
        return false;
    }
    if raw_command_output_has_explicit_command(state, prompt) {
        return false;
    }

    route_result.needs_clarify = false;
    route_result.clarify_question.clear();
    route_result.set_first_layer_decision(crate::FirstLayerDecision::PlannerExecute);
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    append_route_reason(
        route_result,
        "locatorless_status_query_promoted_to_service_status",
    );
    true
}

fn promote_locatorless_scalar_status_query_to_runtime_info(
    route_result: &mut crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let Some(turn_analysis) = turn_analysis else {
        return false;
    };
    if turn_analysis.turn_type != Some(crate::intent_router::TurnType::StatusQuery)
        || turn_analysis_has_runtime_status_query(turn_analysis)
        || !route_result.is_execute_gate()
        || route_result.needs_clarify
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || route_result.output_contract.semantic_kind != crate::OutputSemanticKind::None
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        return false;
    }
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route_result.output_contract.locator_hint.clear();
    append_route_reason(
        route_result,
        "execution_recipe_scalar_runtime_tool_observation",
    );
    true
}

fn turn_analysis_has_runtime_status_query(
    turn_analysis: &crate::intent_router::TurnAnalysis,
) -> bool {
    turn_analysis
        .state_patch
        .as_ref()
        .and_then(|patch| patch.get("runtime_status_query"))
        .and_then(serde_json::Value::as_object)
        .and_then(|query| query.get("kind"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .is_some_and(|kind| !kind.is_empty())
}

fn route_reason_has_marker(route_result: &crate::RouteResult, marker: &str) -> bool {
    route_result
        .route_reason
        .split(';')
        .any(|part| part.trim() == marker)
}

fn raw_command_output_has_explicit_command(state: &AppState, prompt: &str) -> bool {
    crate::agent_engine::explicit_command_segment_for_policy(
        &state.policy.command_intent,
        prompt.trim(),
    )
    .is_some()
}

fn command_observation_route_has_runtime_evidence(
    state: &AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
            | crate::OutputSemanticKind::CommandOutputSummary
            | crate::OutputSemanticKind::ExecutionFailedStep
    ) && (raw_command_output_has_explicit_command(state, prompt)
        || route_reason_has_marker(
            route_result,
            "command_payload_requires_raw_output_execution",
        )
        || route_reason_has_marker(
            route_result,
            "command_payload_requires_command_output_summary_execution",
        ))
}

fn scalar_raw_runtime_observation_can_plan(route_result: &crate::RouteResult) -> bool {
    if route_result.output_contract.semantic_kind != crate::OutputSemanticKind::RawCommandOutput
        || route_result.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    let args = serde_json::json!({
        "action": "runtime_status",
        "kind": "current_user",
    });
    crate::contract_matrix::action_policy_for_output_contract(
        Some(&route_result.output_contract),
        "system_basic",
        &args,
    )
    .is_some_and(|policy| policy.is_allowed())
}

fn locatorless_observation_route_should_force_clarify(
    state: &AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    let has_self_contained_payload = current_request_has_self_contained_structured_payload(prompt);
    let has_raw_command_input_locator =
        raw_command_request_has_structural_input_locator(state, prompt);
    let has_structural_locator_surface =
        current_request_has_structural_locator_surface_for_route(state, prompt, route_result);
    let raw_command_without_input = route_result.output_contract.semantic_kind
        == crate::OutputSemanticKind::RawCommandOutput
        && !raw_command_output_has_explicit_command(state, prompt)
        && !has_self_contained_payload
        && !has_raw_command_input_locator;
    let has_structured_session_anchor =
        active_session_has_structured_observation_anchor(session_snapshot);
    let has_authoritative_deictic_anchor =
        session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot);
    if !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || (has_structural_locator_surface && !raw_command_without_input)
        || has_self_contained_payload
        || state_patch_allows_deictic_locator_guard_bypass(turn_analysis)
        || (has_authoritative_deictic_anchor && !raw_command_without_input)
        || has_structured_session_anchor
    {
        return false;
    }
    if semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind) {
        if route_result.output_contract.semantic_kind
            == crate::OutputSemanticKind::CommandOutputSummary
        {
            return false;
        }
        if raw_command_output_without_locator_can_plan_via_contract(state, prompt, route_result) {
            return false;
        }
        if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
            && route_result.output_contract.response_shape == crate::OutputResponseShape::Scalar
            && (turn_analysis.is_some_and(turn_analysis_has_runtime_status_query)
                || route_reason_has_marker(
                    route_result,
                    "execution_recipe_scalar_runtime_tool_observation",
                )
                || scalar_raw_runtime_observation_can_plan(route_result))
        {
            return false;
        }
        return route_result.output_contract.semantic_kind
            == crate::OutputSemanticKind::RawCommandOutput
            && !raw_command_output_has_explicit_command(state, prompt)
            && (!route_reason_has_marker(
                route_result,
                "command_payload_requires_raw_output_execution",
            ) || raw_command_without_input);
    }
    if command_observation_route_has_runtime_evidence(state, prompt, route_result) {
        return false;
    }

    true
}

fn raw_command_request_has_structural_input_locator(_state: &AppState, prompt: &str) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    surface.has_explicit_path_or_url()
        || surface.has_delivery_token_reference()
        || surface.is_structural_locator_only_reply()
}

fn current_request_has_self_contained_structured_payload(prompt: &str) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    surface.inline_json_shape.is_some()
        || crate::intent::surface_signals::inline_csv_record_block(prompt).is_some()
}

fn bare_topic_memory_expansion_route_should_force_clarify(
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if !is_bare_topic_only_prompt(prompt)
        || !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || !matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::RawCommandOutput
        )
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || state_patch_allows_deictic_locator_guard_bypass(turn_analysis)
        || session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot)
        || active_session_has_structured_observation_anchor(session_snapshot)
    {
        return false;
    }
    route_introduces_unmentioned_distinctive_context_target(prompt, route_result)
}

fn bare_topic_model_supplied_locator_route_should_force_clarify(
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if route_reason_has_marker(route_result, "active_clarify_locator_reply_execute") {
        return false;
    }
    if turn_analysis.is_some_and(|analysis| {
        matches!(
            analysis.target_task_policy,
            Some(crate::intent_router::TargetTaskPolicy::ReuseActive)
        )
    }) && active_session_has_structured_observation_anchor(session_snapshot)
    {
        return false;
    }
    if !is_bare_topic_only_prompt(prompt)
        || current_request_has_concrete_locator_surface(prompt)
        || (!route_result.needs_clarify && !route_result.is_execute_gate())
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::Url
        )
        || route_result.output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    true
}

fn is_bare_topic_only_prompt(prompt: &str) -> bool {
    let trimmed = prompt.trim();
    if trimmed.is_empty()
        || trimmed.split_whitespace().count() != 1
        || trimmed.contains(['/', '\\', '.', ':'])
        || !trimmed
            .chars()
            .any(|ch| ch.is_ascii_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(&ch))
        || !trimmed.chars().all(|ch| {
            ch.is_ascii_alphanumeric()
                || ('\u{4e00}'..='\u{9fff}').contains(&ch)
                || matches!(ch, '-' | '_')
        })
    {
        return false;
    }
    let signal_chars = trimmed
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(ch))
        .count();
    if trimmed.is_ascii() {
        return signal_chars <= 32;
    }
    signal_chars <= 4
}

fn route_introduces_unmentioned_distinctive_context_target(
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    let mut text = String::new();
    text.push_str(&route_result.resolved_intent);
    text.push('\n');
    text.push_str(&route_result.clarify_question);
    distinctive_context_tokens(&text)
        .into_iter()
        .any(|token| !distinctive_token_present_in_request(prompt, &token))
}

fn route_introduces_unmentioned_distinctive_context_target_except_workspace_root(
    prompt: &str,
    route_result: &crate::RouteResult,
    workspace_root: &std::path::Path,
) -> bool {
    let mut text = String::new();
    text.push_str(&route_result.resolved_intent);
    text.push('\n');
    text.push_str(&route_result.clarify_question);
    distinctive_context_tokens(&text).into_iter().any(|token| {
        distinctive_token_relevant_for_workspace_scope_guard(&token)
            && !distinctive_token_names_workspace_root(&token, workspace_root)
            && !distinctive_token_present_in_request(prompt, &token)
    })
}

fn distinctive_token_relevant_for_workspace_scope_guard(token: &str) -> bool {
    token
        .chars()
        .any(|ch| ch.is_ascii_alphanumeric() || ch.is_ascii_digit())
        || token.contains(['.', ':'])
}

fn distinctive_token_names_workspace_root(token: &str, workspace_root: &std::path::Path) -> bool {
    let normalized_token = normalize_locator_identity_token(token);
    if normalized_token.is_empty() {
        return false;
    }
    if locator_hint_names_workspace_root(workspace_root, &normalized_token) {
        return true;
    }
    let canonical_root = normalize_workspace_locator_path(workspace_root);
    let normalized_root = normalize_locator_identity_token(&canonical_root.display().to_string());
    normalized_token == normalized_root
        || normalized_token == normalized_root.trim_start_matches('/')
}

fn distinctive_context_tokens(text: &str) -> Vec<String> {
    text.split(|ch: char| {
        !(ch.is_ascii_alphanumeric()
            || ('\u{4e00}'..='\u{9fff}').contains(&ch)
            || matches!(ch, '_' | '-' | '/' | '.' | ':'))
    })
    .map(|token| token.trim_matches(|ch: char| matches!(ch, '_' | '-' | '/' | '.' | ':')))
    .filter(|token| distinctive_context_token(token))
    .map(ToOwned::to_owned)
    .collect()
}

fn distinctive_context_token(token: &str) -> bool {
    let signal_chars = token
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(ch))
        .count();
    let has_identifier_separator = token.contains(['_', '/', '.', ':']);
    let has_digit = token.chars().any(|ch| ch.is_ascii_digit());
    (signal_chars >= 4 && has_identifier_separator)
        || (signal_chars >= 8 && has_digit)
        || signal_chars >= 16
}

fn distinctive_token_present_in_request(request: &str, token: &str) -> bool {
    let request = request.to_ascii_lowercase();
    let token = token.to_ascii_lowercase();
    if request.contains(&token) {
        return true;
    }
    token
        .split(['_', '-', '/', '.', ':'])
        .filter(|part| part.len() >= 3)
        .any(|part| request.contains(part))
}

fn bare_topic_clarify_question_should_drop_context_target(
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    is_bare_topic_only_prompt(prompt)
        && route_result.needs_clarify
        && route_introduces_unmentioned_distinctive_context_target(prompt, route_result)
}

fn preserve_scalar_shape_from_normalizer_candidate_for_clarify(
    route_result: &mut crate::RouteResult,
) {
    if !route_result.is_execute_gate()
        || route_result.output_contract.semantic_kind != crate::OutputSemanticKind::None
        || !matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::Strict
        )
    {
        return;
    }
    let Some(candidate) = embedded_normalizer_answer_candidate(&route_result.resolved_intent)
    else {
        return;
    };
    if !answer_candidate_is_compact_scalar_shape(candidate) {
        return;
    }
    route_result.output_contract.response_shape = crate::OutputResponseShape::Scalar;
}

fn embedded_normalizer_answer_candidate(resolved_intent: &str) -> Option<&str> {
    resolved_intent.lines().find_map(|line| {
        line.trim_start()
            .strip_prefix("answer_candidate:")
            .map(str::trim)
            .filter(|candidate| !candidate.is_empty())
    })
}

fn active_structured_observation_values<'a>(
    session_snapshot: &'a crate::conversation_state::ActiveSessionSnapshot,
) -> Vec<&'a str> {
    let mut values = Vec::new();
    if let Some(frame) = session_snapshot.active_followup_frame.as_ref() {
        if matches!(
            frame.op_kind,
            crate::followup_frame::FollowupOpKind::Read
                | crate::followup_frame::FollowupOpKind::List
        ) {
            values.push(frame.source_request.as_str());
        }
        if let Some(target) = frame.bound_target.as_deref() {
            values.push(target);
        }
        values.extend(frame.ordered_entries.iter().map(String::as_str));
    }
    if let Some(facts) = session_snapshot.active_observed_facts.as_ref() {
        if let Some(target) = facts.bound_target.as_deref() {
            values.push(target);
        }
        values.extend(facts.ordered_entries.iter().map(String::as_str));
        values.extend(facts.delivery_targets.iter().map(String::as_str));
    }
    values
        .into_iter()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect()
}

fn normalizer_answer_candidate_is_grounded_in_structured_observation(
    candidate: &str,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() || candidate.contains('\n') {
        return false;
    }
    active_structured_observation_values(session_snapshot)
        .into_iter()
        .any(|value| value == candidate)
}

fn normalizer_answer_candidate_is_existing_context_synthesis(
    candidate: &str,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || candidate.contains('\n')
        || candidate.starts_with('{')
        || candidate.starts_with('[')
        || candidate.contains("FILE:")
    {
        return false;
    }
    session_snapshot
        .conversation_state
        .as_ref()
        .and_then(|state| state.last_primary_task_output.as_deref())
        .is_some_and(|output| !output.trim().is_empty())
}

fn answer_candidate_is_recent_execution_token(candidate: &str) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() || candidate.contains('\n') || candidate.chars().count() > 160 {
        return false;
    }
    candidate.contains('/')
        || candidate.contains('\\')
        || candidate.contains('_')
        || candidate.contains('-')
        || std::path::Path::new(candidate).extension().is_some()
}

fn normalizer_answer_candidate_matches_recent_execution_context(
    candidate: &str,
    recent_execution_context: &str,
) -> bool {
    if !answer_candidate_is_compact_scalar_shape(candidate)
        || !answer_candidate_is_recent_execution_token(candidate)
    {
        return false;
    }
    let context = recent_execution_context.trim();
    if context.is_empty() || context == "<none>" {
        return false;
    }
    let candidate = normalize_locator_identity_token(candidate);
    if candidate.chars().count() < 3 {
        return false;
    }
    context.lines().any(|line| {
        let line = normalize_locator_identity_token(line);
        line.split(|ch: char| {
            ch.is_whitespace() || matches!(ch, '=' | ',' | ';' | '|' | '，' | '；')
        })
        .any(|token| {
            let token = normalize_locator_identity_token(token);
            token == candidate
                || std::path::Path::new(&token)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(normalize_locator_identity_token)
                    .is_some_and(|basename| basename == candidate)
        })
    })
}

fn active_session_has_structured_observation_anchor(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    !active_structured_observation_values(session_snapshot).is_empty()
        || session_snapshot
            .active_followup_frame
            .as_ref()
            .is_some_and(|frame| frame.selected_entry_index.is_some() || frame.slice_spec.is_some())
        || session_snapshot
            .active_observed_facts
            .as_ref()
            .is_some_and(|facts| {
                facts.selected_entry_index.is_some()
                    || facts.observed_entry_count.is_some()
                    || facts.slice_spec.is_some()
            })
}

fn route_output_contract_requires_planner_execution(
    contract: &crate::IntentOutputContract,
) -> bool {
    contract.requires_content_evidence
        || contract.delivery_required
        || !matches!(contract.locator_kind, crate::OutputLocatorKind::None)
        || !matches!(contract.delivery_intent, crate::OutputDeliveryIntent::None)
        || !matches!(contract.semantic_kind, crate::OutputSemanticKind::None)
}

fn prompt_surface_has_current_turn_concrete_target(
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    surface.has_explicit_path_or_url()
        || surface.locator_target_pair.is_some()
        || surface.field_selector_count > 0
        || surface.dotted_field_selector.is_some()
        || surface.has_delivery_token_reference()
        || surface.has_deictic_reference()
        || surface.inline_json_shape.is_some()
        || !surface
            .filename_candidates_excluding_field_selectors()
            .is_empty()
}

fn active_text_mutation_can_stay_direct_answer_without_structured_anchor_evidence(
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    if route_result.needs_clarify
        || route_result.is_execute_gate()
        || route_result.wants_file_delivery
        || !matches!(route_result.schedule_kind, crate::ScheduleKind::None)
        || route_output_contract_requires_planner_execution(&route_result.output_contract)
    {
        return false;
    }
    let Some(analysis) = turn_analysis else {
        return false;
    };
    if !matches!(
        analysis.turn_type,
        Some(
            crate::intent_router::TurnType::TaskAppend
                | crate::intent_router::TurnType::TaskCorrect
                | crate::intent_router::TurnType::TaskReplace
                | crate::intent_router::TurnType::TaskScopeUpdate
        )
    ) || !matches!(
        analysis.target_task_policy,
        Some(
            crate::intent_router::TargetTaskPolicy::ReuseActive
                | crate::intent_router::TargetTaskPolicy::ReplaceActive
        )
    ) {
        return false;
    }
    if analysis.attachment_processing_required {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    !prompt_surface_has_current_turn_concrete_target(&surface)
}

fn direct_answer_from_structured_anchor_requires_evidence(
    prompt: &str,
    route_result: &crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    recent_execution_context: &str,
    has_authoritative_deictic_anchor: bool,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    if !has_authoritative_deictic_anchor
        || route_result.needs_clarify
        || route_result.is_execute_gate()
        || route_result.wants_file_delivery
        || !matches!(route_result.schedule_kind, crate::ScheduleKind::None)
        || route_output_contract_requires_planner_execution(&route_result.output_contract)
        || !active_session_has_structured_observation_anchor(session_snapshot)
    {
        return false;
    }
    if active_text_mutation_can_stay_direct_answer_without_structured_anchor_evidence(
        prompt,
        route_result,
        turn_analysis,
    ) {
        return false;
    }
    if current_request_has_self_contained_structured_payload(prompt) {
        return false;
    }
    if resolved_intent_mentions_active_target_basename(route_result, session_snapshot) {
        return false;
    }
    embedded_normalizer_answer_candidate(&route_result.resolved_intent).is_none_or(|candidate| {
        !normalizer_answer_candidate_is_grounded_in_structured_observation(
            candidate,
            session_snapshot,
        ) && !normalizer_answer_candidate_is_existing_context_synthesis(candidate, session_snapshot)
            && !normalizer_answer_candidate_matches_recent_execution_context(
                candidate,
                recent_execution_context,
            )
    })
}

fn resolved_intent_mentions_active_target_basename(
    route_result: &crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    let resolved = route_result.resolved_intent.to_ascii_lowercase();
    if resolved.trim().is_empty() {
        return false;
    }
    active_session_target_basenames(session_snapshot)
        .into_iter()
        .any(|basename| {
            let normalized = normalize_locator_identity_token(&basename);
            normalized.chars().count() >= 3 && resolved.contains(&normalized)
        })
}

fn active_session_target_basenames(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(frame) = session_snapshot.active_followup_frame.as_ref() {
        push_target_basename(&mut out, frame.bound_target.as_deref());
    }
    if let Some(facts) = session_snapshot.active_observed_facts.as_ref() {
        push_target_basename(&mut out, facts.bound_target.as_deref());
        for target in &facts.delivery_targets {
            push_target_basename(&mut out, Some(target));
        }
    }
    out
}

fn push_target_basename(out: &mut Vec<String>, target: Option<&str>) {
    let Some(target) = target.map(str::trim).filter(|target| !target.is_empty()) else {
        return;
    };
    let Some(name) = std::path::Path::new(target)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        return;
    };
    if !out.iter().any(|existing| existing == name) {
        out.push(name.to_string());
    }
}

fn promote_structured_anchor_direct_answer_to_evidence(route_result: &mut crate::RouteResult) {
    route_result.needs_clarify = false;
    route_result.set_planner_execute_finalize(crate::ActFinalizeStyle::ChatWrapped);
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.delivery_required = false;
    route_result.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    if matches!(
        route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    ) {
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    }
    if matches!(
        route_result.output_contract.response_shape,
        crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
    ) {
        route_result.output_contract.response_shape = crate::OutputResponseShape::Strict;
    }
    append_route_reason(
        route_result,
        "structured_anchor_direct_answer_requires_evidence",
    );
}

fn answer_candidate_is_compact_scalar_shape(candidate: &str) -> bool {
    let trimmed = candidate.trim();
    if trimmed.is_empty()
        || trimmed.contains('\n')
        || trimmed.chars().count() > 80
        || trimmed
            .chars()
            .any(|c| matches!(c, ',' | '，' | ';' | '；' | '|' | '[' | ']' | '{' | '}'))
    {
        return false;
    }
    let token_count = trimmed.split_whitespace().count();
    (1..=4).contains(&token_count)
}

fn session_has_authoritative_deictic_anchor(
    prompt: &str,
    route_result: &crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if session_snapshot.active_clarify_state.is_some() {
        return true;
    }
    if followup_frame_has_matching_target(
        session_snapshot.active_followup_frame.as_ref(),
        route_result,
    ) || observed_facts_have_matching_target(
        session_snapshot.active_observed_facts.as_ref(),
        route_result,
    ) {
        return true;
    }
    session_snapshot
        .conversation_state
        .as_ref()
        .is_some_and(|state| {
            state.alias_bindings.iter().any(|binding| {
                let alias = binding.alias.trim();
                !alias.is_empty()
                    && crate::conversation_state::alias_surface_matches_prompt(prompt, alias)
            })
        })
}

fn route_context_contains_target(route_result: &crate::RouteResult, target: &str) -> bool {
    let target = target.trim();
    !target.is_empty()
        && (route_result.resolved_intent.contains(target)
            || route_result.output_contract.locator_hint.contains(target))
}

fn followup_frame_has_matching_target(
    frame: Option<&crate::followup_frame::FollowupFrame>,
    route_result: &crate::RouteResult,
) -> bool {
    frame.is_some_and(|frame| {
        frame
            .bound_target
            .as_deref()
            .is_some_and(|target| route_context_contains_target(route_result, target))
            || frame
                .ordered_entries
                .iter()
                .any(|target| route_context_contains_target(route_result, target))
    })
}

fn observed_facts_have_matching_target(
    facts: Option<&crate::observed_facts::ObservedFacts>,
    route_result: &crate::RouteResult,
) -> bool {
    facts.is_some_and(|facts| {
        facts
            .bound_target
            .as_deref()
            .is_some_and(|target| route_context_contains_target(route_result, target))
            || facts
                .ordered_entries
                .iter()
                .any(|target| route_context_contains_target(route_result, target))
            || facts
                .delivery_targets
                .iter()
                .any(|target| route_context_contains_target(route_result, target))
    })
}

fn append_route_reason(route_result: &mut crate::RouteResult, reason: &'static str) {
    if route_result.route_reason.trim().is_empty() {
        route_result.route_reason = reason.to_string();
    } else if !route_result.route_reason.contains(reason) {
        route_result.route_reason.push_str("; ");
        route_result.route_reason.push_str(reason);
    }
}

fn deictic_bare_locator_should_force_clarify(
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    if state_patch_allows_deictic_locator_guard_bypass(turn_analysis) {
        return false;
    }
    if !state_patch_requires_deictic_locator_clarify(turn_analysis) {
        return false;
    }
    let locator_hint = route_result.output_contract.locator_hint.trim();
    let locator_hint_is_inferred_relative_path = locator_hint_is_relative_path_like(locator_hint);
    (!crate::worker::has_explicit_path_or_url_locator_hint(locator_hint)
        || locator_hint_is_inferred_relative_path)
        && route_result.output_contract.requires_content_evidence
        && matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::CurrentWorkspace
                | crate::OutputLocatorKind::Filename
        )
}

fn state_patch_allows_deictic_locator_guard_bypass(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let Some(state_patch) = turn_analysis.and_then(|analysis| analysis.state_patch.as_ref()) else {
        return false;
    };
    if state_patch
        .get("current_result_ref")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }
    state_patch
        .get("deictic_reference")
        .and_then(serde_json::Value::as_object)
        .and_then(|obj| obj.get("target"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|target| {
            matches!(
                target,
                "current_action_result" | "current_turn_locator" | "comparison_result"
            )
        })
}

fn state_patch_requires_deictic_locator_clarify(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    turn_analysis
        .and_then(|analysis| analysis.state_patch.as_ref())
        .and_then(|state_patch| state_patch.get("deictic_reference"))
        .and_then(serde_json::Value::as_object)
        .and_then(|obj| obj.get("target"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|target| {
            matches!(
                target,
                "unresolved_prior_object" | "missing_locator" | "ambiguous_locator"
            )
        })
}

fn locator_hint_is_relative_path_like(locator_hint: &str) -> bool {
    let hint = locator_hint.trim();
    !hint.is_empty()
        && !hint.starts_with('/')
        && !hint.starts_with("~/")
        && !hint.starts_with("http://")
        && !hint.starts_with("https://")
        && !hint.contains(":\\")
        && (hint.contains('/') || hint.contains('\\'))
}

fn deictic_missing_locator_question(route_result: &crate::RouteResult) -> &'static str {
    if matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::ScalarCount
    ) {
        return "请提供要计数的具体目录或路径。";
    }
    if matches!(
        route_result.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
            | crate::OutputDeliveryIntent::DirectoryLookup
            | crate::OutputDeliveryIntent::DirectoryBatchFiles
    ) {
        return "请提供目标文件或目录的具体路径。";
    }
    if matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::ServiceStatus
    ) {
        return "请提供要查看状态的具体服务名。";
    }
    if matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::ScalarPathOnly | crate::OutputSemanticKind::ExistenceWithPath
    ) {
        return "请提供要搜索的目录或目标文件的具体路径。";
    }
    if route_result.output_contract.requires_content_evidence {
        return "请提供要读取或检查的具体文件、目录或路径。";
    }
    "请提供具体目标或路径。"
}

pub(super) async fn prepare_ask_flow(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
    source: &str,
) -> Result<PreparedAskFlow> {
    let prepared_routing = super::prepare_ask_routing(state, task, payload, prompt, source).await;
    let semantic_answer_candidate_draft =
        embedded_normalizer_answer_candidate(&prepared_routing.route_result.resolved_intent)
            .map(ToOwned::to_owned);
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
        prepared_routing.turn_analysis.as_ref(),
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
        memory_trace: prepared_execution.context_bundle.memory_trace(),
        route_result: applied_post_route.execution_route_result,
        execution_recipe_hint: prepared_routing.execution_recipe_hint,
        turn_analysis: prepared_routing.turn_analysis,
        clarify_fallback_source: prepared_routing.clarify_fallback_source,
        auto_locator_path: applied_post_route.auto_locator_path,
        has_authoritative_deictic_anchor: applied_post_route.has_authoritative_deictic_anchor,
        chat_prompt_context: prepared_execution.chat_prompt_context,
        resolved_prompt_for_execution: applied_post_route.resolved_prompt_for_execution,
        prompt_with_memory_for_execution: applied_post_route.prompt_with_memory_for_execution,
        memory_context_for_execution: prepared_execution.memory_context_for_execution,
        semantic_answer_candidate_draft,
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
    if let Some(candidate) = crate::ask_flow::active_ordered_entries_count_direct_answer_candidate(
        prompt,
        agent_run_context.as_ref(),
    ) {
        return Ok(Some(Ok(ask_reply_with_visible_process(
            state, task, prompt, candidate,
        ))));
    }
    if let Some(delivery_token) = direct_existing_file_delivery_token(route_result) {
        crate::log_ask_transition(
            state,
            &task.task_id,
            Some(crate::AskState::Routing),
            crate::AskState::Executing,
            "direct_existing_file_delivery",
            None,
        );
        let path = delivery_token
            .strip_prefix("FILE:")
            .unwrap_or(delivery_token.as_str())
            .to_string();
        let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", prompt);
        journal.record_route_result(route_result);
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "direct_file_delivery".to_string(),
                skill: "fs_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(
                    serde_json::json!({
                        "action": "direct_file_delivery",
                        "path": path.clone(),
                        "resolved_path": path,
                    })
                    .to_string(),
                ),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });
        return Ok(Some(Ok(
            crate::AskReply::non_llm(delivery_token).with_task_journal(journal)
        )));
    }
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
        let request_language_hint =
            crate::language_policy::task_response_language_hint(state, task, prompt);
        let structured_default_question = preferred_clarify_question.is_none().then(|| {
            structured_missing_locator_default_question(
                state,
                &request_language_hint,
                route_result,
                fuzzy_locator_suggestions,
            )
        });
        let structured_default_question = structured_default_question.flatten();
        let preferred_clarify_question =
            preferred_clarify_question.or(structured_default_question.as_deref());
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
        return Ok(Some(Ok(ask_reply_with_visible_process(
            state, task, prompt, clarify,
        ))));
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
#[path = "ask_pipeline_tests.rs"]
mod tests;
