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
    prebind_workspace_child_locator_from_current_request(state, prompt, &mut route_result);
    prebind_clarify_workspace_child_locator_from_current_request(state, prompt, &mut route_result);
    prebind_workspace_child_locator_from_resolved_prompt(state, resolved_prompt, &mut route_result);
    prebind_workspace_root_locator_from_resolved_prompt(state, resolved_prompt, &mut route_result);
    prebind_active_bound_target_from_matching_locator_hint(&mut route_result, &session_snapshot);
    prebind_quantity_compare_directory_pair_from_current_request(state, prompt, &mut route_result);
    if background_only_locator_route_should_force_clarify(
        state,
        prompt,
        resolved_prompt,
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
    promote_locatorless_status_query_to_service_status(
        state,
        prompt,
        &mut route_result,
        turn_analysis,
    );
    promote_locatorless_git_capability_to_repository_state(&mut route_result);
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
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = path;
    append_route_reason(
        route_result,
        "direct_file_delivery_locator_prebound_before_deictic_guard",
    );
    true
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
    if semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind)
        && !(route_result.output_contract.semantic_kind
            == crate::OutputSemanticKind::RawCommandOutput
            && !raw_command_output_has_explicit_command(state, prompt))
    {
        return false;
    }
    route_introduces_unmentioned_distinctive_context_target(prompt, route_result)
}

fn unbound_targeted_evidence_route_should_force_clarify(
    prompt: &str,
    route_result: &crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if current_request_has_concrete_locator_surface(prompt)
        || current_request_has_self_contained_structured_payload(prompt)
        || session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot)
        || route_result.needs_clarify
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
    if surface.has_deictic_reference() && !surface.has_explicit_path_or_url() {
        return false;
    }
    current_request_has_concrete_locator_surface(prompt)
        || (route_result.output_contract.requires_content_evidence
            && !semantic_kind_can_execute_without_locator(
                route_result.output_contract.semantic_kind,
            )
            && current_request_resolves_workspace_child_locator(state, prompt).is_some())
}

fn prebind_workspace_child_locator_from_current_request(
    state: &AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if route_result.needs_clarify
        || !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind)
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

fn prebind_clarify_workspace_child_locator_from_current_request(
    state: &AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.needs_clarify
        || route_result.output_contract.delivery_required
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind)
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
    if !semantic_quantity_comparison
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
    let Some((left, right)) =
        workspace_directory_pair_from_current_request(state, prompt, !semantic_quantity_comparison)
    else {
        return false;
    };
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
            "quantity_compare_directory_pair_prebound_from_current_request"
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
    contract_locator
        || crate::worker::has_explicit_path_or_url_locator_hint(resolved_prompt)
        || crate::worker::has_explicit_path_or_url_locator_hint(&route_result.resolved_intent)
}

fn background_only_locator_route_should_force_clarify(
    state: &AppState,
    prompt: &str,
    resolved_prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if execute_route_without_input_locator_should_plan(route_result)
        || state_patch_allows_deictic_locator_guard_bypass(turn_analysis)
        || session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot)
        || route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
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

fn semantic_kind_can_execute_without_locator(kind: crate::OutputSemanticKind) -> bool {
    matches!(
        kind,
        crate::OutputSemanticKind::RawCommandOutput
            | crate::OutputSemanticKind::ServiceStatus
            | crate::OutputSemanticKind::HiddenEntriesCheck
            | crate::OutputSemanticKind::WorkspaceProjectSummary
            | crate::OutputSemanticKind::RecentScalarEqualityCheck
            | crate::OutputSemanticKind::GitCommitSubject
            | crate::OutputSemanticKind::GitRepositoryState
            | crate::OutputSemanticKind::PackageManagerDetection
            | crate::OutputSemanticKind::DockerPs
            | crate::OutputSemanticKind::DockerImages
            | crate::OutputSemanticKind::DockerLogs
            | crate::OutputSemanticKind::DockerContainerLifecycle
    )
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
    route_result
        .visible_skill_candidates
        .iter()
        .any(|skill| skill == "git_basic")
        || ascii_token_present(&route_result.resolved_intent, "git")
        || ascii_token_present(&route_result.route_reason, "git")
}

fn ascii_token_present(text: &str, token: &str) -> bool {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
        .any(|candidate| candidate.eq_ignore_ascii_case(token))
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
        || !route_result.is_execute_gate()
        || route_result.needs_clarify
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
    if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
        && raw_command_output_has_explicit_command(state, prompt)
    {
        return false;
    }

    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    append_route_reason(
        route_result,
        "locatorless_status_query_promoted_to_service_status",
    );
    true
}

fn raw_command_output_has_explicit_command(state: &AppState, prompt: &str) -> bool {
    crate::agent_engine::explicit_command_segment_for_policy(
        &state.policy.command_intent,
        prompt.trim(),
    )
    .is_some()
}

fn locatorless_observation_route_should_force_clarify(
    state: &AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || current_request_has_structural_locator_surface_for_route(state, prompt, route_result)
        || current_request_has_self_contained_structured_payload(prompt)
        || state_patch_allows_deictic_locator_guard_bypass(turn_analysis)
        || session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot)
        || active_session_has_structured_observation_anchor(session_snapshot)
    {
        return false;
    }
    if semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind) {
        return route_result.output_contract.semantic_kind
            == crate::OutputSemanticKind::RawCommandOutput
            && !raw_command_output_has_explicit_command(state, prompt);
    }

    true
}

fn current_request_has_self_contained_structured_payload(prompt: &str) -> bool {
    crate::intent::surface_signals::inline_json_transform_request(prompt)
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
    embedded_normalizer_answer_candidate(&route_result.resolved_intent).is_none_or(|candidate| {
        !normalizer_answer_candidate_is_grounded_in_structured_observation(
            candidate,
            session_snapshot,
        )
    })
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
                let target = binding.target.trim();
                (!alias.is_empty()
                    && crate::conversation_state::alias_surface_matches_prompt(prompt, alias))
                    || (!target.is_empty()
                        && (route_result.resolved_intent.contains(target)
                            || route_result.output_contract.locator_hint.contains(target)))
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
mod tests {
    use super::{
        background_only_locator_route_should_force_clarify,
        bare_topic_clarify_question_should_drop_context_target,
        bare_topic_memory_expansion_route_should_force_clarify, clarify_fallback_source_or_default,
        current_workspace_locator_resolution, deictic_bare_locator_should_force_clarify,
        deictic_memory_only_route_should_force_clarify, deictic_missing_locator_question,
        direct_answer_from_structured_anchor_requires_evidence, effective_auto_locator_kind,
        execution_user_request, locatorless_observation_route_should_force_clarify,
        prebind_active_bound_target_from_matching_locator_hint,
        prebind_clarify_workspace_child_locator_from_current_request,
        prebind_direct_file_delivery_locator_before_deictic_guard,
        prebind_quantity_compare_directory_pair_from_current_request,
        prebind_workspace_child_locator_from_current_request,
        prebind_workspace_child_locator_from_resolved_prompt,
        prebind_workspace_root_locator_from_resolved_prompt,
        preserve_scalar_shape_from_normalizer_candidate_for_clarify,
        promote_locatorless_git_capability_to_repository_state,
        promote_locatorless_status_query_to_service_status,
        promote_structured_anchor_direct_answer_to_evidence, should_attempt_auto_locator,
        should_preserve_original_inline_structured_input, should_reuse_route_clarify_question,
        should_suppress_recent_execution_in_clarify_context,
        structured_missing_locator_clarify_context, structured_missing_locator_default_question,
        unbound_existing_file_delivery_route_should_force_clarify,
        unbound_model_context_target_route_should_force_clarify,
        unbound_targeted_evidence_route_should_force_clarify,
    };
    use crate::{AgentRuntimeConfig, AppState, SkillViewsSnapshot};
    use claw_core::config::{AgentConfig, ToolsConfig};
    use std::collections::{HashMap, HashSet};
    use std::{
        path::PathBuf,
        sync::{Arc, RwLock},
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

    fn test_state_with_root(root: PathBuf) -> AppState {
        let agents_by_id = HashMap::from([(
            crate::DEFAULT_AGENT_ID.to_string(),
            AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
        )]);
        AppState {
            core: crate::CoreServices {
                agents_by_id: Arc::new(agents_by_id),
                skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                    registry: None,
                    skills_list: Arc::new(HashSet::new()),
                }))),
                ..crate::CoreServices::test_default()
            },
            skill_rt: crate::SkillRuntime {
                workspace_root: root.clone(),
                default_locator_search_dir: root,
                locator_scan_max_depth: 2,
                locator_scan_max_files: 100,
                tools_policy: Arc::new(
                    crate::ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
                ),
                ..crate::SkillRuntime::test_default()
            },
            policy: crate::PolicyConfig::test_default(),
            worker: crate::WorkerConfig::test_default(),
            metrics: crate::TaskMetricsRegistry::default(),
            channels: crate::ChannelConfig::default(),
            reload_ctx: crate::ReloadContext::default(),
            ask_states: crate::AskStateRegistry::default(),
        }
    }

    fn executable_filename_route() -> crate::RouteResult {
        crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "读取 README 开头并总结".to_string(),
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
                exact_sentence_count: None,
                locator_kind: crate::OutputLocatorKind::Filename,
                locator_hint: "README.md".to_string(),
                requires_content_evidence: true,
                ..Default::default()
            },
        }
    }

    fn unresolved_deictic_analysis() -> crate::intent_router::TurnAnalysis {
        crate::intent_router::TurnAnalysis {
            turn_type: None,
            target_task_policy: None,
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "deictic_reference": {"target": "unresolved_prior_object"}
            })),
            attachment_processing_required: false,
        }
    }

    fn turn_analysis_with_state_patch(
        state_patch: serde_json::Value,
    ) -> crate::intent_router::TurnAnalysis {
        crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskAppend),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(state_patch),
            attachment_processing_required: false,
        }
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
            ask_mode: crate::AskMode::planner_execute_plain(),
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
                exact_sentence_count: None,
                locator_kind: crate::OutputLocatorKind::Path,
                requires_content_evidence: false,
                ..Default::default()
            },
        };
        assert!(should_attempt_auto_locator(&route));
    }

    #[test]
    fn deictic_bare_locator_forces_clarify_before_auto_locator() {
        let route = executable_filename_route();
        let analysis = turn_analysis_with_state_patch(
            serde_json::json!({"deictic_reference":{"target":"unresolved_prior_object"}}),
        );
        assert!(deictic_bare_locator_should_force_clarify(
            &route,
            Some(&analysis)
        ));
    }

    #[test]
    fn deictic_directory_scope_with_target_filename_forces_clarify() {
        let mut route = executable_filename_route();
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint = "case_only".to_string();
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        let analysis = turn_analysis_with_state_patch(
            serde_json::json!({"deictic_reference":{"target":"unresolved_prior_object"}}),
        );
        assert!(deictic_bare_locator_should_force_clarify(
            &route,
            Some(&analysis)
        ));
    }

    #[test]
    fn deictic_synthesized_relative_path_forces_clarify() {
        let mut route = executable_filename_route();
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint = "reports/report.md".to_string();
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        let analysis = turn_analysis_with_state_patch(
            serde_json::json!({"deictic_reference":{"target":"unresolved_prior_object"}}),
        );
        assert!(deictic_bare_locator_should_force_clarify(
            &route,
            Some(&analysis)
        ));
    }

    #[test]
    fn deictic_forced_clarify_question_names_missing_locator() {
        let mut route = executable_filename_route();
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint = "reports/report.md".to_string();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        assert_eq!(
            deictic_missing_locator_question(&route),
            "请提供要搜索的目录或目标文件的具体路径。"
        );
    }

    #[test]
    fn deictic_file_locator_with_filename_hint_still_allows_auto_locator() {
        let route = executable_filename_route();
        assert!(!deictic_bare_locator_should_force_clarify(&route, None));
    }

    #[test]
    fn direct_bare_locator_still_allows_auto_locator() {
        let route = executable_filename_route();
        assert!(!deictic_bare_locator_should_force_clarify(&route, None));
    }

    #[test]
    fn deictic_explicit_path_still_allows_auto_locator() {
        let route = executable_filename_route();
        assert!(!deictic_bare_locator_should_force_clarify(&route, None));
    }

    #[test]
    fn deictic_memory_only_execute_route_requires_clarify_without_session_anchor() {
        let mut route = executable_filename_route();
        route.output_contract.locator_hint =
            "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs".to_string();
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        let analysis = unresolved_deictic_analysis();
        assert!(deictic_memory_only_route_should_force_clarify(
            "看看那个目录下面都有什么",
            &route,
            Some(&analysis),
            &snapshot,
        ));
    }

    #[test]
    fn unbound_current_workspace_count_requires_clarify_without_anchor() {
        let mut route = executable_filename_route();
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint.clear();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        route.output_contract.requires_content_evidence = true;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(unbound_targeted_evidence_route_should_force_clarify(
            "count the requested target's direct children and output only the number",
            &route,
            &snapshot,
        ));
        assert_eq!(
            deictic_missing_locator_question(&route),
            "请提供要计数的具体目录或路径。"
        );
    }

    #[test]
    fn bound_current_workspace_count_does_not_trigger_unbound_fallback_guard() {
        let mut route = executable_filename_route();
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint = "/tmp/rustclaw".to_string();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        route.output_contract.requires_content_evidence = true;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(!unbound_targeted_evidence_route_should_force_clarify(
            "count direct children in the current workspace and output only the number",
            &route,
            &snapshot,
        ));
    }

    #[test]
    fn current_workspace_hidden_entries_check_does_not_trigger_unbound_fallback_guard() {
        let mut route = executable_filename_route();
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint.clear();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::HiddenEntriesCheck;
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(!unbound_targeted_evidence_route_should_force_clarify(
            "check hidden entries in the current workspace and list examples",
            &route,
            &snapshot,
        ));
    }

    #[test]
    fn current_workspace_scalar_equality_check_does_not_trigger_unbound_fallback_guard() {
        let mut route = executable_filename_route();
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint.clear();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(!unbound_targeted_evidence_route_should_force_clarify(
            "check whether the current git branch equals main",
            &route,
            &snapshot,
        ));
    }

    #[test]
    fn active_bound_target_prebinds_matching_basename_locator_hint() {
        let mut route = executable_filename_route();
        route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
        route.output_contract.locator_hint = "test_bundle.zip".to_string();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ArchiveList;
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: Some(crate::followup_frame::FollowupFrame {
                op_kind: crate::followup_frame::FollowupOpKind::Read,
                bound_target: Some(
                    "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
                ),
                ..crate::followup_frame::FollowupFrame::default()
            }),
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(prebind_active_bound_target_from_matching_locator_hint(
            &mut route, &snapshot,
        ));
        assert_eq!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
        );
        assert_eq!(
            route.output_contract.locator_hint,
            "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"
        );
        assert!(route
            .route_reason
            .contains("active_bound_target_prebound_from_matching_locator_hint"));
    }

    #[test]
    fn unbound_scalar_count_without_locator_requires_clarify() {
        let mut route = executable_filename_route();
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        route.output_contract.requires_content_evidence = true;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(unbound_targeted_evidence_route_should_force_clarify(
            "count direct children and output only the number",
            &route,
            &snapshot,
        ));
    }

    #[test]
    fn unbound_current_workspace_file_summary_requires_clarify_without_anchor() {
        let mut route = executable_filename_route();
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint.clear();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
        route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(unbound_targeted_evidence_route_should_force_clarify(
            "read the beginning of the requested documentation and summarize it in one sentence",
            &route,
            &snapshot,
        ));
    }

    #[test]
    fn unbound_current_workspace_project_summary_still_allows_execution() {
        let mut route = executable_filename_route();
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint.clear();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
        route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(!unbound_targeted_evidence_route_should_force_clarify(
            "summarize the current workspace structure in one sentence",
            &route,
            &snapshot,
        ));
    }

    #[test]
    fn unbound_current_workspace_semantic_none_allows_self_scoped_observation() {
        let mut route = executable_filename_route();
        route.resolved_intent =
            "Detect which package manager is present in the current workspace.".to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint.clear();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(!unbound_targeted_evidence_route_should_force_clarify(
            "Which package manager is detected for this workspace?",
            &route,
            &snapshot,
        ));
    }

    #[test]
    fn unbound_model_context_allows_current_workspace_generic_observation() {
        let state = test_state_with_root(make_temp_root("unbound_model_current_workspace_generic"));
        let mut route = executable_filename_route();
        route.resolved_intent =
            "Preview how files in the current workspace could be categorized.".to_string();
        route.route_reason =
            "The request needs observing the current workspace before summarizing.".to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        route.output_contract.response_shape = crate::OutputResponseShape::Free;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(!unbound_model_context_target_route_should_force_clarify(
            &state,
            "preview the current workspace categories",
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn background_only_locator_rewrite_requires_clarify_without_session_anchor() {
        let state = test_state_with_root(make_temp_root("background_locator_requires_clarify"));
        let mut route = executable_filename_route();
        route.resolved_intent =
            "读取 /home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/README.md 前 3 行"
                .to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint =
            "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/README.md".to_string();
        route.output_contract.requires_content_evidence = true;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(background_only_locator_route_should_force_clarify(
            &state,
            "读一下那个文件前 3 行",
            &route.resolved_intent,
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn model_supplied_manifest_locator_from_deictic_prompt_requires_clarify() {
        let root = make_temp_root("background_manifest_locator_requires_clarify");
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\n").expect("manifest");
        let state = test_state_with_root(root);
        let mut route = executable_filename_route();
        route.resolved_intent =
            "Extract the name field from the package manifest (Cargo.toml).".to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint = "Cargo.toml".to_string();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(background_only_locator_route_should_force_clarify(
            &state,
            "extract name from that package file",
            &route.resolved_intent,
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn unbound_model_context_target_requires_clarify_before_planner_guess() {
        let state = test_state_with_root(make_temp_root("unbound_model_context_target"));
        let mut route = executable_filename_route();
        route.resolved_intent =
            "Extract the name field from the package manifest (Cargo.toml).".to_string();
        route.route_reason =
            "User requests the package name from the package file; Cargo.toml can be read directly."
                .to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(unbound_model_context_target_route_should_force_clarify(
            &state,
            "extract name from that package file",
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn unbound_model_context_target_allows_inline_csv_transform_payload() {
        let state = test_state_with_root(make_temp_root("unbound_model_context_inline_csv"));
        let mut route = executable_filename_route();
        route.resolved_intent =
            "Sort the embedded CSV records and render a markdown table.".to_string();
        route.route_reason = "inline structured records can be transformed directly".to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(!unbound_model_context_target_route_should_force_clarify(
            &state,
            "这个 CSV 按 score 降序输出 markdown 表格：name,score\\nli,3\\nwang,8\\nzhao,5",
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn unbound_model_context_target_allows_configured_raw_command_without_locator() {
        let mut state = test_state_with_root(make_temp_root("unbound_model_context_raw_command"));
        state.policy.command_intent.execute_prefixes = vec!["run ".to_string()];
        state.policy.command_intent.standalone_commands = vec!["pwd".to_string()];
        let mut route = executable_filename_route();
        route.resolved_intent = "Get current working directory path".to_string();
        route.route_reason = "command_payload_requires_raw_output_execution".to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
        route.output_contract.response_shape = crate::OutputResponseShape::Free;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(!unbound_model_context_target_route_should_force_clarify(
            &state,
            "Run pwd and output only the raw result.",
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn unbound_model_context_target_ignores_reason_only_example_targets() {
        let state = test_state_with_root(make_temp_root("unbound_model_context_reason_example"));
        let mut route = executable_filename_route();
        route.resolved_intent =
            "Detect which package manager is used in this workspace.".to_string();
        route.route_reason =
            "Observation may inspect examples such as Cargo.toml or package.json.".to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(!unbound_model_context_target_route_should_force_clarify(
            &state,
            "Which package manager is detected for this workspace?",
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn unbound_model_context_target_allows_current_turn_locator() {
        let root = make_temp_root("unbound_model_context_current_locator");
        let readme = root.join("README.md");
        std::fs::write(&readme, "# Demo\n").expect("readme");
        let state = test_state_with_root(root);
        let mut route = executable_filename_route();
        route.resolved_intent = "Read README.md and summarize it.".to_string();
        route.route_reason = "README.md is the requested target.".to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(!unbound_model_context_target_route_should_force_clarify(
            &state,
            "README.md",
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn background_only_locator_rewrite_allows_current_turn_filename() {
        let state = test_state_with_root(make_temp_root("background_locator_filename"));
        let mut route = executable_filename_route();
        route.resolved_intent = "读取 README.md 前 3 行".to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
        route.output_contract.locator_hint = "README.md".to_string();
        route.output_contract.requires_content_evidence = true;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(!background_only_locator_route_should_force_clarify(
            &state,
            "README.md",
            &route.resolved_intent,
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn background_only_locator_rewrite_allows_active_ordered_anchor() {
        let state = test_state_with_root(make_temp_root("background_locator_anchor"));
        let mut route = executable_filename_route();
        route.resolved_intent = "读取 /tmp/work/crates/larkd 前 3 行".to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint = "/tmp/work/crates/larkd".to_string();
        route.output_contract.requires_content_evidence = true;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: Some(crate::followup_frame::FollowupFrame {
                op_kind: crate::followup_frame::FollowupOpKind::List,
                bound_target: Some("/tmp/work/crates".to_string()),
                ordered_entries: vec!["larkd".to_string()],
                ..crate::followup_frame::FollowupFrame::default()
            }),
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(!background_only_locator_route_should_force_clarify(
            &state,
            "看最后一个的基本信息",
            &route.resolved_intent,
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn locatorless_observation_requires_clarify_without_session_anchor() {
        let state = test_state_with_root(make_temp_root("locatorless_requires_clarify"));
        let mut route = executable_filename_route();
        route.resolved_intent = "读取该文件的前 3 行".to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(locatorless_observation_route_should_force_clarify(
            &state,
            "读一下那个文件前 3 行",
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn locatorless_inline_structured_payload_does_not_require_external_locator() {
        let state = test_state_with_root(make_temp_root("locatorless_inline_payload"));
        let mut route = executable_filename_route();
        route.resolved_intent = r#"Count inline JSON array records."#.to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(!locatorless_observation_route_should_force_clarify(
            &state,
            r#"统计这个 JSON 数组中对象数量，只输出数字：[{"x":1},{"x":2}]"#,
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn quantity_compare_prebinds_two_workspace_directories_from_current_request() {
        let root = make_temp_root("quantity_dir_pair_prebind");
        std::fs::create_dir_all(root.join("fixtures/tmp/bundle_src")).expect("left");
        std::fs::create_dir_all(root.join("fixtures/tmp/dynamic_guard_unpack_case"))
            .expect("right");
        let state = test_state_with_root(root.clone());
        let mut route = executable_filename_route();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint = "bundle_src vs dynamic_guard_unpack_case".to_string();
        route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;

        assert!(
            prebind_quantity_compare_directory_pair_from_current_request(
                &state,
                "bundle_src 와 dynamic_guard_unpack_case 를 재귀 비교하고 차이가 있는지 짧게 답해.",
                &mut route,
            )
        );

        assert!(!route.needs_clarify);
        assert_eq!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
        );
        assert!(route.output_contract.locator_hint.contains("bundle_src"));
        assert!(route
            .output_contract
            .locator_hint
            .contains("dynamic_guard_unpack_case"));

        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(!background_only_locator_route_should_force_clarify(
            &state,
            "bundle_src 와 dynamic_guard_unpack_case 를 재귀 비교하고 차이가 있는지 짧게 답해.",
            &route.resolved_intent,
            &route,
            None,
            &snapshot,
        ));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn directory_pair_prebinds_missing_locator_without_forcing_semantic_kind() {
        let root = make_temp_root("directory_pair_missing_locator_prebind");
        std::fs::create_dir_all(root.join("fixtures/tmp/bundle_src")).expect("left");
        std::fs::create_dir_all(root.join("fixtures/tmp/dynamic_guard_unpack_case"))
            .expect("right");
        let state = test_state_with_root(root.clone());
        let mut route = executable_filename_route();
        route.needs_clarify = true;
        route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint.clear();
        route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;

        assert!(
            prebind_quantity_compare_directory_pair_from_current_request(
                &state,
                "bundle_src 와 dynamic_guard_unpack_case 를 재귀 비교하고 차이가 있는지 짧게 답해.",
                &mut route,
            )
        );

        assert!(!route.needs_clarify);
        assert!(route.is_execute_gate());
        assert_eq!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
        );
        assert!(route
            .output_contract
            .locator_hint
            .contains("fixtures/tmp/bundle_src"));
        assert!(route
            .output_contract
            .locator_hint
            .contains("fixtures/tmp/dynamic_guard_unpack_case"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn directory_pair_prebind_skips_archive_locator_pair_contract() {
        let root = make_temp_root("directory_pair_archive_locator_pair");
        std::fs::create_dir_all(root.join("scripts/nl_tests/fixtures/device_local/tmp"))
            .expect("fixture dirs");
        std::fs::create_dir_all(root.join("tmp/contract_matrix_unpacked")).expect("dest dir");
        let state = test_state_with_root(root.clone());
        let mut route = executable_filename_route();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint.clear();
        route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;

        assert!(!prebind_quantity_compare_directory_pair_from_current_request(
            &state,
            concat!(
                "把 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 解压到 tmp/contract_matrix_unpacked，并简短说明结果。",
                "\n[CONTRACT_TEST_HINT]\n",
                "candidate_wrong_action_ref=fs_basic.write_text\n",
                "policy_expectation=runtime_must_reject_or_replace_disallowed_action\n",
                "[/CONTRACT_TEST_HINT]"
            ),
            &mut route,
        ));

        assert_eq!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::CurrentWorkspace
        );
        assert!(route.output_contract.locator_hint.is_empty());
        assert!(!route
            .route_reason
            .contains("directory_pair_prebound_from_current_request"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn directory_pair_prebind_skips_explicit_content_excerpt_contract() {
        let root = make_temp_root("directory_pair_content_excerpt");
        std::fs::create_dir_all(root.join("scripts/nl_tests/fixtures/device_local/docs"))
            .expect("fixture dirs");
        std::fs::create_dir_all(root.join(".git/objects/20")).expect("numeric dir");
        let state = test_state_with_root(root.clone());
        let mut route = executable_filename_route();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint =
            "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md".to_string();
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;

        assert!(!prebind_quantity_compare_directory_pair_from_current_request(
            &state,
            concat!(
                "读取 scripts/nl_tests/fixtures/device_local/docs/release_checklist.md 前 20 行，并用三句话总结。",
                "\n[CONTRACT_TEST_HINT]\n",
                "preferred_action_ref=archive_basic.read\n",
                "policy_expectation=use_allowed_action_with_required_evidence\n",
                "[/CONTRACT_TEST_HINT]"
            ),
            &mut route,
        ));

        assert_eq!(
            route.output_contract.locator_hint,
            "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
        );
        assert!(!route
            .route_reason
            .contains("directory_pair_prebound_from_current_request"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn directory_pair_prebind_scan_reaches_late_structural_directory_tokens() {
        let root = make_temp_root("directory_pair_late_structural_scan");
        for idx in 0..2500 {
            std::fs::create_dir_all(root.join(format!("aaa_filler_{idx:04}"))).expect("filler");
        }
        std::fs::create_dir_all(root.join("zz_fixture/tmp/bundle_src")).expect("left");
        std::fs::create_dir_all(root.join("zz_fixture/tmp/dynamic_guard_unpack_case"))
            .expect("right");
        let mut state = test_state_with_root(root.clone());
        state.skill_rt.locator_scan_max_files = 10;
        let mut route = executable_filename_route();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint.clear();
        route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;

        assert!(
            prebind_quantity_compare_directory_pair_from_current_request(
                &state,
                "bundle_src 와 dynamic_guard_unpack_case 를 재귀 비교하고 차이가 있는지 짧게 답해.",
                &mut route,
            )
        );

        assert!(!route.needs_clarify);
        assert_eq!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
        );
        assert!(route
            .output_contract
            .locator_hint
            .contains("zz_fixture/tmp/bundle_src"));
        assert!(route
            .output_contract
            .locator_hint
            .contains("zz_fixture/tmp/dynamic_guard_unpack_case"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn locatorless_service_status_observation_does_not_clarify() {
        let state = test_state_with_root(make_temp_root("locatorless_service_status"));
        let mut route = executable_filename_route();
        route.resolved_intent =
            "Check whether the requested daemon process is currently running.".to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(!locatorless_observation_route_should_force_clarify(
            &state,
            "check whether telegramd is currently running",
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn locatorless_status_query_promotes_to_service_status_before_clarify_guards() {
        let state = test_state_with_root(make_temp_root("locatorless_runtime_status_query"));
        let mut route = executable_filename_route();
        route.resolved_intent =
            "Provide a brief runtime diagnostics overview from fresh system observation."
                .to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        let analysis = crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::StatusQuery),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        };
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(promote_locatorless_status_query_to_service_status(
            &state,
            "status overview",
            &mut route,
            Some(&analysis),
        ));
        assert_eq!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::ServiceStatus
        );
        assert!(!locatorless_observation_route_should_force_clarify(
            &state,
            "status overview",
            &route,
            Some(&analysis),
            &snapshot,
        ));
        assert!(!unbound_targeted_evidence_route_should_force_clarify(
            "status overview",
            &route,
            &snapshot,
        ));
    }

    #[test]
    fn locatorless_raw_status_query_promotes_when_no_literal_command() {
        let state = test_state_with_root(make_temp_root("locatorless_raw_status_query"));
        let mut route = executable_filename_route();
        route.resolved_intent =
            "Check whether the local clawd process is present and summarize matches.".to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        let analysis = crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::StatusQuery),
            target_task_policy: None,
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        };
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(promote_locatorless_status_query_to_service_status(
            &state,
            "check whether the local clawd process is present",
            &mut route,
            Some(&analysis),
        ));
        assert_eq!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::ServiceStatus
        );
        assert!(!locatorless_observation_route_should_force_clarify(
            &state,
            "check whether the local clawd process is present",
            &route,
            Some(&analysis),
            &snapshot,
        ));
    }

    #[test]
    fn locatorless_git_capability_promotes_to_repository_state_before_clarify_guards() {
        let state = test_state_with_root(make_temp_root("locatorless_git_capability"));
        let mut route = executable_filename_route();
        route.resolved_intent =
            "Observe git repository state from the current workspace.".to_string();
        route.route_reason = "This requires git_basic readonly observation.".to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(promote_locatorless_git_capability_to_repository_state(
            &mut route,
        ));
        assert_eq!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::GitRepositoryState
        );
        assert!(!locatorless_observation_route_should_force_clarify(
            &state, "git", &route, None, &snapshot,
        ));
        assert!(!unbound_targeted_evidence_route_should_force_clarify(
            "git", &route, &snapshot,
        ));
    }

    #[test]
    fn locatorless_observation_binds_existing_workspace_child_from_current_request() {
        let root = make_temp_root("locatorless_workspace_child");
        let prompts_dir = root.join("prompts");
        std::fs::create_dir_all(&prompts_dir).expect("prompts dir");
        let state = test_state_with_root(root);
        let mut route = executable_filename_route();
        route.resolved_intent = "用户希望查看 prompts 目录下前 5 个条目的名称".to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(prebind_workspace_child_locator_from_current_request(
            &state,
            "先列出 prompts 目录下前 5 个条目名称",
            &mut route,
        ));
        assert_eq!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
        );
        assert_eq!(
            route.output_contract.locator_hint,
            prompts_dir
                .canonicalize()
                .expect("canonical prompts")
                .display()
                .to_string()
        );
        assert!(!locatorless_observation_route_should_force_clarify(
            &state,
            "先列出 prompts 目录下前 5 个条目名称",
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn clarify_observation_binds_existing_workspace_child_from_current_request() {
        let root = make_temp_root("clarify_current_workspace_child");
        let configs_dir = root.join("configs");
        std::fs::create_dir_all(&configs_dir).expect("configs dir");
        let state = test_state_with_root(root);
        let mut route = executable_filename_route();
        route.needs_clarify = true;
        route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;

        assert!(
            prebind_clarify_workspace_child_locator_from_current_request(
                &state,
                "先列出 configs 目录下前 5 个条目名称",
                &mut route,
            )
        );
        assert!(!route.needs_clarify);
        assert!(route.is_execute_gate());
        assert_eq!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
        );
        assert_eq!(
            route.output_contract.locator_hint,
            configs_dir
                .canonicalize()
                .expect("canonical configs")
                .display()
                .to_string()
        );
    }

    #[test]
    fn clarify_observation_binds_workspace_child_when_semantic_kind_is_generic() {
        let root = make_temp_root("clarify_generic_workspace_child");
        let document_dir = root.join("document");
        std::fs::create_dir_all(&document_dir).expect("document dir");
        let state = test_state_with_root(root);
        let mut route = executable_filename_route();
        route.needs_clarify = true;
        route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;

        assert!(
            prebind_clarify_workspace_child_locator_from_current_request(
                &state,
                "列出 document 目录最近修改的 2 个文件名，只输出文件名",
                &mut route,
            )
        );
        assert!(!route.needs_clarify);
        assert!(route.is_execute_gate());
        assert_eq!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
        );
        assert_eq!(
            route.output_contract.locator_hint,
            document_dir
                .canonicalize()
                .expect("canonical document")
                .display()
                .to_string()
        );
    }

    #[test]
    fn clarify_observation_binds_existing_workspace_file_from_current_request_path() {
        let root = make_temp_root("clarify_current_workspace_file");
        let schema_path = root
            .join("prompts")
            .join("schemas")
            .join("direct_answer_gate.schema.json");
        std::fs::create_dir_all(schema_path.parent().expect("schema parent")).expect("schema dir");
        std::fs::write(&schema_path, "{}").expect("schema file");
        let state = test_state_with_root(root);
        let mut route = executable_filename_route();
        route.needs_clarify = true;
        route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;

        assert!(
            prebind_clarify_workspace_child_locator_from_current_request(
                &state,
                "prompts/schemas/direct_answer_gate.schema.json",
                &mut route,
            )
        );
        assert!(!route.needs_clarify);
        assert!(route.is_execute_gate());
        assert_eq!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
        );
        assert_eq!(
            route.output_contract.locator_hint,
            schema_path
                .canonicalize()
                .expect("canonical schema")
                .display()
                .to_string()
        );
    }

    #[test]
    fn clarify_observation_binds_existing_workspace_child_from_resolved_prompt() {
        let root = make_temp_root("clarify_resolved_workspace_child");
        let configs_dir = root.join("configs");
        std::fs::create_dir_all(&configs_dir).expect("configs dir");
        let state = test_state_with_root(root);
        let mut route = executable_filename_route();
        route.needs_clarify = true;
        route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;

        assert!(prebind_workspace_child_locator_from_resolved_prompt(
            &state,
            &format!("列出 {} 目录下前 5 个条目名称", configs_dir.display()),
            &mut route,
        ));
        assert!(!route.needs_clarify);
        assert!(route.is_execute_gate());
        assert_eq!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
        );
        assert_eq!(
            route.output_contract.locator_hint,
            configs_dir
                .canonicalize()
                .expect("canonical configs")
                .display()
                .to_string()
        );
    }

    #[test]
    fn clarify_observation_binds_existing_workspace_file_from_resolved_prompt_path() {
        let root = make_temp_root("clarify_resolved_workspace_file");
        let schema_path = root
            .join("prompts")
            .join("schemas")
            .join("direct_answer_gate.schema.json");
        std::fs::create_dir_all(schema_path.parent().expect("schema parent")).expect("schema dir");
        std::fs::write(&schema_path, "{}").expect("schema file");
        let state = test_state_with_root(root);
        let mut route = executable_filename_route();
        route.needs_clarify = true;
        route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;

        assert!(prebind_workspace_child_locator_from_resolved_prompt(
            &state,
            &format!("查看 {} 中的 target enum", schema_path.display()),
            &mut route,
        ));
        assert!(!route.needs_clarify);
        assert!(route.is_execute_gate());
        assert_eq!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
        );
        assert_eq!(
            route.output_contract.locator_hint,
            schema_path
                .canonicalize()
                .expect("canonical schema")
                .display()
                .to_string()
        );
    }

    #[test]
    fn locatorless_observation_binds_workspace_root_from_resolved_prompt_path() {
        let root = make_temp_root("locatorless_workspace_root");
        let state = test_state_with_root(root.clone());
        let mut route = executable_filename_route();
        route.resolved_intent = format!(
            "List the first 10 entry names in the repository root {} without explanation.",
            root.display()
        );
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(prebind_workspace_root_locator_from_resolved_prompt(
            &state,
            &route.resolved_intent.clone(),
            &mut route,
        ));
        assert_eq!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::CurrentWorkspace
        );
        assert_eq!(
            route.output_contract.locator_hint,
            root.display().to_string()
        );
        assert!(!locatorless_observation_route_should_force_clarify(
            &state,
            "列出当前仓库根目录前 10 个条目名称，不要解释",
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn unclassified_workspace_child_keyword_does_not_bypass_background_locator_guard() {
        let root = make_temp_root("workspace_child_keyword_not_locator");
        std::fs::create_dir_all(root.join("target")).expect("target dir");
        let state = test_state_with_root(root);
        let mut route = executable_filename_route();
        route.resolved_intent = "Inspect schema target enum".to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint = "target".to_string();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let analysis = unresolved_deictic_analysis();

        assert!(background_only_locator_route_should_force_clarify(
            &state,
            "inspect schema target enum",
            &route.resolved_intent,
            &route,
            Some(&analysis),
            &snapshot,
        ));
    }

    #[test]
    fn locatorless_raw_command_output_still_allows_execution() {
        let mut state = test_state_with_root(make_temp_root("locatorless_raw_command"));
        state.policy.command_intent.execute_prefixes = vec!["execute ".to_string()];
        let mut route = executable_filename_route();
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(!locatorless_observation_route_should_force_clarify(
            &state,
            "execute pwd, then explain what the path means in one sentence",
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn locatorless_raw_command_without_explicit_command_requires_clarify() {
        let state = test_state_with_root(make_temp_root("locatorless_raw_without_command"));
        let mut route = executable_filename_route();
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(locatorless_observation_route_should_force_clarify(
            &state,
            "list directory contents",
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn bare_topic_raw_command_with_unmentioned_context_target_forces_clarify() {
        let mut route = executable_filename_route();
        route.resolved_intent = "View logs from the ops_http_repair test suite".to_string();
        route.route_reason = "User typed a bare topic and route context mentioned scripts/nl_suite_logs/ops_http_repair".to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(bare_topic_memory_expansion_route_should_force_clarify(
            "logs", &route, None, &snapshot,
        ));
    }

    #[test]
    fn bare_topic_raw_command_without_unmentioned_context_target_stays_executable() {
        let mut route = executable_filename_route();
        route.resolved_intent = "execute pwd command".to_string();
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(!bare_topic_memory_expansion_route_should_force_clarify(
            "pwd", &route, None, &snapshot,
        ));
    }

    #[test]
    fn bare_topic_clarify_question_with_unmentioned_context_target_is_sanitized() {
        let mut route = executable_filename_route();
        route.needs_clarify = true;
        route.resolved_intent = "View logs".to_string();
        route.clarify_question =
            "Which logs: ops_http_repair, a specific file path, or current process logs?"
                .to_string();

        assert!(bare_topic_clarify_question_should_drop_context_target(
            "logs", &route
        ));
    }

    #[test]
    fn chinese_deictic_delivery_sentence_is_not_treated_as_bare_topic() {
        let mut route = executable_filename_route();
        route.needs_clarify = true;
        route.resolved_intent = "把最近提到的文件发给用户".to_string();
        route.clarify_question = "请提供目标文件或目录的具体路径。".to_string();

        assert!(!bare_topic_clarify_question_should_drop_context_target(
            "把那个文件发给我",
            &route
        ));
    }

    #[test]
    fn locatorless_observation_allows_active_structured_anchor() {
        let state = test_state_with_root(make_temp_root("locatorless_active_anchor"));
        let mut route = executable_filename_route();
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: Some(crate::followup_frame::FollowupFrame {
                op_kind: crate::followup_frame::FollowupOpKind::Read,
                bound_target: Some("/tmp/work/README.md".to_string()),
                ..crate::followup_frame::FollowupFrame::default()
            }),
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(!locatorless_observation_route_should_force_clarify(
            &state,
            "再看前 3 行",
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn deictic_memory_only_command_output_reference_does_not_force_clarify() {
        let mut route = executable_filename_route();
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.delivery_required = false;
        route.wants_file_delivery = false;
        route.output_contract.response_shape = crate::OutputResponseShape::Free;
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(!deictic_memory_only_route_should_force_clarify(
            "执行 pwd，然后用一句话解释这个路径代表什么",
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn deictic_forced_clarify_preserves_only_scalar_shape_from_answer_candidate() {
        let mut route = executable_filename_route();
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        route.resolved_intent =
            "Count direct child items in that directory\nanswer_candidate: 6".to_string();

        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route);

        assert_eq!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
        );
        assert_eq!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
        );
        assert!(route.resolved_intent.contains("answer_candidate: 6"));
    }

    #[test]
    fn scalar_shape_preservation_ignores_list_like_answer_candidate() {
        let mut route = executable_filename_route();
        route.output_contract.response_shape = crate::OutputResponseShape::Free;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        route.resolved_intent =
            "List candidate files\nanswer_candidate: a.log, b.log, c.log".to_string();

        preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route);

        assert_eq!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free
        );
    }

    #[test]
    fn deictic_memory_only_guard_allows_current_session_alias_binding() {
        let mut route = executable_filename_route();
        route.output_contract.locator_hint = "/tmp/docs".to_string();
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
                    alias: "那个目录".to_string(),
                    target: "/tmp/docs".to_string(),
                    updated_at_ts: 1,
                }],
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(!deictic_memory_only_route_should_force_clarify(
            "看看那个目录下面都有什么",
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn deictic_memory_only_guard_allows_session_alias_target_resolved_by_normalizer() {
        let mut route = executable_filename_route();
        route.resolved_intent = "Read the first 10 lines of /tmp/device/README.md".to_string();
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
                    alias: "that_file".to_string(),
                    target: "/tmp/device/README.md".to_string(),
                    updated_at_ts: 1,
                }],
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(!deictic_memory_only_route_should_force_clarify(
            "把那个文件开头读 10 行",
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn deictic_memory_only_guard_rejects_stale_observed_target_without_route_match() {
        let mut route = executable_filename_route();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::SqliteSchemaVersion;
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint.clear();
        route.resolved_intent =
            "Query the current workspace SQLite database schema version".to_string();
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: Some(crate::observed_facts::ObservedFacts {
                bound_target: Some(
                    "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/archive"
                        .to_string(),
                ),
                ..Default::default()
            }),
        };

        let analysis = unresolved_deictic_analysis();
        assert!(deictic_memory_only_route_should_force_clarify(
            "看一下那个 sqlite 的 schema version 是多少",
            &route,
            Some(&analysis),
            &snapshot,
        ));
    }

    #[test]
    fn deictic_memory_only_guard_allows_active_clarify_anchor() {
        let route = executable_filename_route();
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: Some(crate::clarify_state::ClarifyState {
                missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
                pending_question: "Which file?".to_string(),
                candidate_targets: Vec::new(),
                delivery_required: true,
                output_shape: Some(crate::OutputResponseShape::FileToken.as_str().to_string()),
                semantic_kind: None,
                source_request: "Send the file".to_string(),
                source_task_id: "task-1".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
            }),
            active_observed_facts: None,
        };

        assert!(!deictic_memory_only_route_should_force_clarify(
            "把那个文件发给我",
            &route,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn deictic_context_bound_path_still_allows_execution() {
        let mut route = executable_filename_route();
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint =
            "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite"
                .to_string();
        assert!(!deictic_bare_locator_should_force_clarify(&route, None));
    }

    #[test]
    fn direct_file_delivery_locator_prebinds_directory_before_deictic_guard() {
        let root = make_temp_root("delivery_dir_prebind");
        std::fs::create_dir_all(root.join("document")).expect("document dir");
        let state = test_state_with_root(root.clone());
        let mut route = executable_filename_route();
        route.ask_mode = crate::AskMode::planner_execute_plain();
        route.resolved_intent =
            "send the last file in the document directory, rejecting the previous file".to_string();
        route.wants_file_delivery = true;
        route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        route.output_contract.delivery_required = true;
        route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint = "document directory".to_string();

        assert!(prebind_direct_file_delivery_locator_before_deictic_guard(
            &state, "", &mut route
        ));

        assert!(!deictic_bare_locator_should_force_clarify(&route, None));
        assert_eq!(
            route.output_contract.locator_hint,
            root.join("document")
                .canonicalize()
                .expect("canonical document")
                .display()
                .to_string()
        );
        assert!(route
            .route_reason
            .contains("direct_file_delivery_locator_prebound_before_deictic_guard"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unbound_existing_file_delivery_with_model_locator_forces_clarify() {
        let root = make_temp_root("unbound_delivery_model_locator");
        let state = test_state_with_root(root.clone());
        let mut route = executable_filename_route();
        route.ask_mode = crate::AskMode::planner_execute_plain();
        route.wants_file_delivery = true;
        route.output_contract.requires_content_evidence = false;
        route.output_contract.delivery_required = true;
        route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint = "configs/config.toml".to_string();

        assert!(unbound_existing_file_delivery_route_should_force_clarify(
            &state,
            "please send the referenced local configuration as a file",
            &route,
            false,
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unbound_existing_file_delivery_allows_current_request_locator() {
        let root = make_temp_root("delivery_current_locator");
        let state = test_state_with_root(root.clone());
        let mut route = executable_filename_route();
        route.ask_mode = crate::AskMode::planner_execute_plain();
        route.wants_file_delivery = true;
        route.output_contract.requires_content_evidence = false;
        route.output_contract.delivery_required = true;
        route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint = "configs/config.toml".to_string();

        assert!(!unbound_existing_file_delivery_route_should_force_clarify(
            &state,
            "please send configs/config.toml as a file",
            &route,
            false,
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unbound_existing_file_delivery_allows_authoritative_anchor() {
        let root = make_temp_root("delivery_authoritative_anchor");
        let state = test_state_with_root(root.clone());
        let mut route = executable_filename_route();
        route.ask_mode = crate::AskMode::planner_execute_plain();
        route.wants_file_delivery = true;
        route.output_contract.requires_content_evidence = false;
        route.output_contract.delivery_required = true;
        route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint = "configs/config.toml".to_string();

        assert!(!unbound_existing_file_delivery_route_should_force_clarify(
            &state,
            "please send it as a file",
            &route,
            true,
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unbound_existing_file_delivery_allows_generated_file_delivery() {
        let root = make_temp_root("delivery_generated_file");
        let state = test_state_with_root(root.clone());
        let mut route = executable_filename_route();
        route.ask_mode = crate::AskMode::planner_execute_plain();
        route.wants_file_delivery = true;
        route.output_contract.requires_content_evidence = false;
        route.output_contract.delivery_required = true;
        route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFileDelivery;

        assert!(!unbound_existing_file_delivery_route_should_force_clarify(
            &state,
            "generate a small report and send it as a file",
            &route,
            false,
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unbound_existing_file_delivery_allows_resolved_workspace_child() {
        let root = make_temp_root("delivery_workspace_child");
        std::fs::create_dir_all(root.join("document")).expect("document dir");
        let state = test_state_with_root(root.clone());
        let mut route = executable_filename_route();
        route.ask_mode = crate::AskMode::planner_execute_plain();
        route.wants_file_delivery = true;
        route.output_contract.requires_content_evidence = false;
        route.output_contract.delivery_required = true;
        route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint = "document".to_string();

        assert!(!unbound_existing_file_delivery_route_should_force_clarify(
            &state,
            "please send document as a file",
            &route,
            false,
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn structured_anchor_direct_answer_with_derived_candidate_requires_evidence() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("list document entries".to_string()),
                last_primary_task_output: Some("hello.sh".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: Some(crate::followup_frame::FollowupFrame {
                source_request: "list document entries".to_string(),
                op_kind: crate::followup_frame::FollowupOpKind::List,
                bound_target: Some("document".to_string()),
                ordered_entries: vec!["hello.sh".to_string()],
                source_task_id: "task-1".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
                ..crate::followup_frame::FollowupFrame::default()
            }),
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let mut route = executable_filename_route();
        route.ask_mode = crate::AskMode::direct_answer();
        route.resolved_intent = concat!(
            "User wants path and type for the observed entry hello.sh.\n",
            "answer_candidate: {\"path\":\"/tmp/hello.sh\",\"type\":\"file\"}"
        )
        .to_string();
        route.output_contract = crate::IntentOutputContract::default();

        assert!(direct_answer_from_structured_anchor_requires_evidence(
            "What is the path and type for that entry?",
            &route,
            &snapshot,
            true,
            None
        ));

        promote_structured_anchor_direct_answer_to_evidence(&mut route);
        assert!(route.is_execute_gate());
        assert!(route.output_contract.requires_content_evidence);
        assert_eq!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::CurrentWorkspace
        );
        assert!(route
            .route_reason
            .contains("structured_anchor_direct_answer_requires_evidence"));
    }

    #[test]
    fn structured_anchor_direct_answer_with_exact_observed_candidate_stays_chat() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("list document entries".to_string()),
                last_primary_task_output: Some("hello.sh".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: Some(crate::followup_frame::FollowupFrame {
                source_request: "list document entries".to_string(),
                op_kind: crate::followup_frame::FollowupOpKind::List,
                bound_target: Some("document".to_string()),
                ordered_entries: vec!["hello.sh".to_string()],
                source_task_id: "task-1".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
                ..crate::followup_frame::FollowupFrame::default()
            }),
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let mut route = executable_filename_route();
        route.ask_mode = crate::AskMode::direct_answer();
        route.resolved_intent =
            "User wants the observed entry name.\nanswer_candidate: hello.sh".to_string();
        route.output_contract = crate::IntentOutputContract::default();

        assert!(!direct_answer_from_structured_anchor_requires_evidence(
            "What is that entry name?",
            &route,
            &snapshot,
            true,
            None
        ));
    }

    #[test]
    fn active_text_mutation_with_structured_anchor_stays_chat() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("GET http://127.0.0.1:8787/v1/health".to_string()),
                last_primary_task_output: Some("Service status: reachable (HTTP 200).".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: Some(crate::followup_frame::FollowupFrame {
                source_request: "GET http://127.0.0.1:8787/v1/health".to_string(),
                op_kind: crate::followup_frame::FollowupOpKind::Read,
                bound_target: Some("http://127.0.0.1:8787/v1/health".to_string()),
                source_task_id: "task-1".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
                ..crate::followup_frame::FollowupFrame::default()
            }),
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let analysis = crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskScopeUpdate),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        };
        let mut route = executable_filename_route();
        route.ask_mode = crate::AskMode::direct_answer();
        route.resolved_intent = "Clarify the current request without reading files.".to_string();
        route.output_contract = crate::IntentOutputContract::default();

        assert!(!direct_answer_from_structured_anchor_requires_evidence(
            "A concept label without a concrete target.",
            &route,
            &snapshot,
            true,
            Some(&analysis)
        ));
    }

    #[test]
    fn deictic_result_reference_with_two_named_files_allows_execution() {
        let mut route = executable_filename_route();
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint.clear();
        route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        let analysis = turn_analysis_with_state_patch(
            serde_json::json!({"deictic_reference":{"target":"comparison_result"}}),
        );
        assert!(!deictic_bare_locator_should_force_clarify(
            &route,
            Some(&analysis)
        ));
    }

    #[test]
    fn deictic_result_reference_after_command_allows_execution() {
        let mut route = executable_filename_route();
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint.clear();
        route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
        let analysis = turn_analysis_with_state_patch(
            serde_json::json!({"deictic_reference":{"target":"current_action_result"}}),
        );

        assert!(!deictic_bare_locator_should_force_clarify(
            &route,
            Some(&analysis)
        ));
    }

    #[test]
    fn deictic_target_before_followup_still_forces_clarify() {
        let route = executable_filename_route();
        let analysis = turn_analysis_with_state_patch(
            serde_json::json!({"deictic_reference":{"target":"unresolved_prior_object"}}),
        );
        assert!(deictic_bare_locator_should_force_clarify(
            &route,
            Some(&analysis)
        ));
    }

    #[test]
    fn deictic_directory_reference_after_named_folder_allows_execution() {
        let mut route = executable_filename_route();
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint = "docs".to_string();
        route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        let analysis = turn_analysis_with_state_patch(
            serde_json::json!({"deictic_reference":{"target":"current_turn_locator"}}),
        );

        assert!(!deictic_bare_locator_should_force_clarify(
            &route,
            Some(&analysis)
        ));
    }

    #[test]
    fn auto_locator_skips_non_path_locators() {
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
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
                exact_sentence_count: None,
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
            ask_mode: crate::AskMode::planner_execute_plain(),
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
                exact_sentence_count: None,
                locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
                requires_content_evidence: false,
                ..Default::default()
            },
        };
        assert!(should_attempt_auto_locator(&route));
    }

    #[test]
    fn auto_locator_skips_clarify_with_unbound_workspace_scope() {
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::clarify(),
            resolved_intent: "检查当前目录".to_string(),
            needs_clarify: true,
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
                exact_sentence_count: None,
                locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
                requires_content_evidence: true,
                ..Default::default()
            },
        };
        assert!(!should_attempt_auto_locator(&route));

        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint = "docs".to_string();
        assert!(should_attempt_auto_locator(&route));

        route.output_contract.locator_hint.clear();
        assert!(!should_attempt_auto_locator(&route));
    }

    #[test]
    fn quantity_comparison_current_workspace_does_not_auto_locator_to_root() {
        let mut route = executable_filename_route();
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint = "/tmp/repo".to_string();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;

        assert!(!should_attempt_auto_locator(&route));
    }

    #[test]
    fn current_workspace_locator_resolution_prefers_workspace_root() {
        let root = make_temp_root("current_workspace_locator_root");
        std::fs::create_dir_all(root.join("rustclaw")).expect("nested rustclaw dir");
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
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
                exact_sentence_count: None,
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
    fn current_workspace_locator_resolution_accepts_absolute_workspace_hint() {
        let root = make_temp_root("current_workspace_locator_abs_root");
        std::fs::write(root.join("rustclaw"), "#!/usr/bin/env bash\n").expect("launcher file");
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "Introduce RustClaw as the current project".to_string(),
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
                exact_sentence_count: None,
                locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
                locator_hint: root.display().to_string(),
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
    fn current_workspace_locator_hint_naming_root_resolves_to_workspace_root() {
        let parent = make_temp_root("current_workspace_locator_root_name");
        let root = parent.join("rustclaw");
        std::fs::create_dir_all(&root).expect("workspace root");
        std::fs::write(root.join("rustclaw"), "#!/usr/bin/env bash\n").expect("same-name child");
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "Introduce the current RustClaw project".to_string(),
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
                exact_sentence_count: None,
                locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
                locator_hint: "RustClaw".to_string(),
                requires_content_evidence: true,
                ..Default::default()
            },
        };

        assert!(matches!(
            super::current_workspace_locator_resolution(&root, &route),
            Some(crate::post_route_policy::LocatorResolution::Direct(path))
                if path == root.display().to_string()
        ));
        let _ = std::fs::remove_dir_all(parent);
    }

    #[test]
    fn current_workspace_locator_hint_with_target_name_does_not_resolve_to_root() {
        let root = make_temp_root("current_workspace_locator_named_hint");
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "列出 archive 目录下的所有条目".to_string(),
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
                exact_sentence_count: None,
                locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
                locator_hint: "archive".to_string(),
                requires_content_evidence: true,
                ..Default::default()
            },
        };

        assert!(current_workspace_locator_resolution(&root, &route).is_none());
        assert_eq!(
            effective_auto_locator_kind(&route),
            crate::OutputLocatorKind::Path
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn current_workspace_empty_locator_hint_resolves_to_root() {
        let root = make_temp_root("current_workspace_locator_empty_hint");
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "列出当前工作区".to_string(),
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
                exact_sentence_count: None,
                locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
                requires_content_evidence: true,
                ..Default::default()
            },
        };

        assert!(matches!(
            current_workspace_locator_resolution(&root, &route),
            Some(crate::post_route_policy::LocatorResolution::Direct(path))
                if path == root.display().to_string()
        ));
        assert_eq!(
            effective_auto_locator_kind(&route),
            crate::OutputLocatorKind::CurrentWorkspace
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn auto_locator_attempts_for_filename_locators() {
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
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
                exact_sentence_count: None,
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
            ask_mode: crate::AskMode::clarify(),
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
                exact_sentence_count: None,
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
            ask_mode: crate::AskMode::clarify(),
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
                exact_sentence_count: None,
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
            ask_mode: crate::AskMode::clarify(),
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
                exact_sentence_count: None,
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
    fn structured_missing_file_locator_default_question_is_specific() {
        let root = make_temp_root("missing_file_question");
        let state = test_state_with_root(root.clone());
        let mut route = clarify_route(crate::OutputLocatorKind::Filename);
        route.clarify_question.clear();
        route.output_contract.requires_content_evidence = false;
        route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        route.output_contract.delivery_required = true;
        route.wants_file_delivery = true;

        let question = structured_missing_locator_default_question(&state, "zh-CN", &route, &[])
            .expect("structured default question");

        assert!(question.contains("文件完整路径"));
        assert!(!question.contains("没看出"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn structured_missing_sqlite_locator_default_question_uses_semantic_kind() {
        let root = make_temp_root("missing_sqlite_question");
        let state = test_state_with_root(root.clone());
        let mut route = clarify_route(crate::OutputLocatorKind::Path);
        route.clarify_question.clear();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::SqliteTableListing;

        let question = structured_missing_locator_default_question(&state, "en", &route, &[])
            .expect("structured default question");

        assert!(question.contains("SQLite database file"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn structured_missing_locator_default_question_skips_fuzzy_candidates() {
        let root = make_temp_root("missing_fuzzy_question");
        let state = test_state_with_root(root.clone());
        let route = clarify_route(crate::OutputLocatorKind::Filename);
        let candidates = vec!["/tmp/a/Cargo.toml".to_string()];

        assert!(
            structured_missing_locator_default_question(&state, "en", &route, &candidates)
                .is_none()
        );
        let _ = std::fs::remove_dir_all(root);
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
    fn filename_clarify_without_locator_reuses_specific_router_question() {
        let route = clarify_route(crate::OutputLocatorKind::Filename);
        assert!(should_reuse_route_clarify_question(
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
