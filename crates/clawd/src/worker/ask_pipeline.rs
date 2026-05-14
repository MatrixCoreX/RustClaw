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
        clarify_fallback_source_or_default, current_workspace_locator_resolution,
        deictic_bare_locator_should_force_clarify, deictic_missing_locator_question,
        effective_auto_locator_kind, execution_user_request,
        prebind_direct_file_delivery_locator_before_deictic_guard, should_attempt_auto_locator,
        should_preserve_original_inline_structured_input, should_reuse_route_clarify_question,
        should_suppress_recent_execution_in_clarify_context,
        structured_missing_locator_clarify_context,
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
        route.routed_mode = crate::RoutedMode::Act;
        route.ask_mode = crate::AskMode::from_routed_mode(crate::RoutedMode::Act);
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
                exact_sentence_count: None,
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
