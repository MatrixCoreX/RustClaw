use anyhow::Result;
use serde_json::Value;
use std::collections::{HashSet, VecDeque};
use tracing::info;

use super::*;

pub(super) struct PreparedAskFlow {
    pub(super) context_bundle_summary: String,
    pub(super) route_result: crate::RouteResult,
    pub(super) execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    pub(super) turn_analysis: Option<crate::intent_router::TurnAnalysis>,
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

fn clarify_fallback_source_for_route(
    route_result: &crate::RouteResult,
) -> crate::fallback::ClarifyFallbackSource {
    if route_result.route_reason.contains("llm_failed") {
        crate::fallback::ClarifyFallbackSource::LlmUnavailable
    } else {
        crate::fallback::ClarifyFallbackSource::IntentUnresolved
    }
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

fn structured_fuzzy_locator_clarify_question(
    state: &AppState,
    fuzzy_locator_suggestions: &[String],
) -> Option<String> {
    if fuzzy_locator_suggestions.is_empty() {
        return None;
    }
    let candidate_block = fuzzy_locator_suggestions
        .iter()
        .enumerate()
        .map(|(idx, value)| format!("{}. {}", idx + 1, value))
        .collect::<Vec<_>>()
        .join("\n");
    let header = if state
        .policy
        .command_intent
        .default_locale
        .trim()
        .to_ascii_lowercase()
        .starts_with("en")
    {
        "I found multiple matching candidates. Reply with the number or the full path:"
    } else {
        "我找到了多个匹配候选，请直接回复序号或完整路径："
    };
    Some(format!("{header}\n{candidate_block}"))
}

fn structured_missing_locator_clarify_question(
    state: &AppState,
    route_result: &crate::RouteResult,
    fuzzy_locator_suggestions: &[String],
) -> Option<String> {
    if !route_result.needs_clarify {
        return None;
    }
    if let Some(clarify) =
        structured_fuzzy_locator_clarify_question(state, fuzzy_locator_suggestions)
    {
        return Some(clarify);
    }
    if !route_result.output_contract.locator_hint.trim().is_empty() {
        return None;
    }
    let route_question = route_result.clarify_question.trim();
    if !route_question.is_empty() {
        return Some(route_question.to_string());
    }
    if matches!(
        route_result.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::DirectoryLookup
    ) {
        return Some(crate::i18n_t_with_default(
            state,
            "clawd.msg.clarify_missing_directory_locator",
            "Please provide the specific directory name or path to inspect.",
        ));
    }
    if matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::ScalarCount
    ) {
        return Some(crate::i18n_t_with_default(
            state,
            "clawd.msg.clarify_missing_count_target",
            "Please provide the specific directory or path to count.",
        ));
    }
    if matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::ServiceStatus
    ) {
        return Some(crate::i18n_t_with_default(
            state,
            "clawd.msg.clarify_missing_service_target",
            "Please tell me which service you want to check, for example a service name or process name.",
        ));
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
                | crate::OutputResponseShape::Free
                | crate::OutputResponseShape::OneSentence
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

fn should_short_circuit_structured_clarify(
    route_result: &crate::RouteResult,
    fuzzy_locator_suggestions: &[String],
) -> bool {
    let fuzzy_locator_clarify = !fuzzy_locator_suggestions.is_empty();
    let missing_locator_structured_clarify =
        route_result.output_contract.locator_hint.trim().is_empty()
            && (route_result.output_contract.delivery_required
                || (route_result.output_contract.requires_content_evidence
                    && matches!(
                        route_result.output_contract.response_shape,
                        crate::OutputResponseShape::Scalar
                            | crate::OutputResponseShape::Free
                            | crate::OutputResponseShape::OneSentence
                    )));
    route_result.needs_clarify && (fuzzy_locator_clarify || missing_locator_structured_clarify)
}

fn structured_doc_filename_scalar_locator_resolution(
    workspace_root: &std::path::Path,
    route_result: &crate::RouteResult,
    max_depth: usize,
    _max_files: usize,
) -> Option<crate::post_route_policy::LocatorResolution> {
    if !route_supports_structured_doc_filename_scalar_resolution(route_result)
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::Filename
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.response_shape != crate::OutputResponseShape::Scalar
    {
        return None;
    }
    let locator_hint = route_result.output_contract.locator_hint.trim();
    let (file_name, field_path, format) = if locator_hint.eq_ignore_ascii_case("Cargo.toml") {
        ("Cargo.toml", "package.name", "toml")
    } else if locator_hint.eq_ignore_ascii_case("package.json") {
        ("package.json", "name", "json")
    } else {
        return None;
    };

    let candidates =
        find_exact_filename_matches_with_depth(workspace_root, file_name, max_depth, 16);
    choose_structured_doc_scalar_candidate(workspace_root, candidates, field_path, format)
}

fn route_supports_structured_doc_filename_scalar_resolution(
    route_result: &crate::RouteResult,
) -> bool {
    crate::route_reason_starts_with_route_contract(
        &route_result.route_reason,
        "generic_filename_scalar_extract",
    )
}

fn find_exact_filename_matches_with_depth(
    root: &std::path::Path,
    file_name: &str,
    max_depth: usize,
    max_matches: usize,
) -> Vec<std::path::PathBuf> {
    if !root.is_dir() || file_name.trim().is_empty() || max_matches == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut queue = VecDeque::from([(root.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries = read_dir
            .filter_map(std::result::Result::ok)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            let Ok(meta) = entry.file_type() else {
                continue;
            };
            if meta.is_dir() {
                if depth < max_depth
                    && !should_skip_structured_doc_dir(
                        path.file_name()
                            .and_then(|value| value.to_str())
                            .unwrap_or(""),
                    )
                {
                    queue.push_back((path, depth + 1));
                }
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            let Some(current_name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if current_name.eq_ignore_ascii_case(file_name) {
                out.push(path);
                if out.len() >= max_matches {
                    return out;
                }
            }
        }
    }
    out
}

fn should_skip_structured_doc_dir(name: &str) -> bool {
    matches!(
        name,
        "node_modules" | "target" | ".git" | "dist" | "build" | ".next"
    )
}

fn choose_structured_doc_scalar_candidate(
    workspace_root: &std::path::Path,
    candidates: Vec<std::path::PathBuf>,
    field_path: &str,
    format: &str,
) -> Option<crate::post_route_policy::LocatorResolution> {
    if candidates.is_empty() {
        return None;
    }
    let field_bearing = candidates
        .iter()
        .filter(|path| structured_doc_has_field(path, field_path, format))
        .cloned()
        .collect::<Vec<_>>();

    if field_bearing.is_empty() {
        return if candidates.len() == 1 {
            Some(crate::post_route_policy::LocatorResolution::Direct(
                candidates[0].display().to_string(),
            ))
        } else {
            Some(crate::post_route_policy::LocatorResolution::Fuzzy(
                candidates
                    .into_iter()
                    .take(3)
                    .map(|path| path.display().to_string())
                    .collect(),
            ))
        };
    }

    let Some(min_depth) = field_bearing
        .iter()
        .map(|path| relative_depth(workspace_root, path))
        .min()
    else {
        return None;
    };
    let best = field_bearing
        .into_iter()
        .filter(|path| relative_depth(workspace_root, path) == min_depth)
        .collect::<Vec<_>>();

    if best.len() == 1 {
        Some(crate::post_route_policy::LocatorResolution::Direct(
            best[0].display().to_string(),
        ))
    } else {
        Some(crate::post_route_policy::LocatorResolution::Fuzzy(
            best.into_iter()
                .take(3)
                .map(|path| path.display().to_string())
                .collect(),
        ))
    }
}

fn relative_depth(workspace_root: &std::path::Path, path: &std::path::Path) -> usize {
    path.strip_prefix(workspace_root)
        .ok()
        .map(|relative| relative.components().count())
        .unwrap_or_else(|| path.components().count())
}

fn structured_doc_has_field(path: &std::path::Path, field_path: &str, format: &str) -> bool {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    let root = match format {
        "json" => serde_json::from_str::<serde_json::Value>(&raw).ok(),
        "toml" => toml::from_str::<toml::Value>(&raw)
            .ok()
            .and_then(|value| serde_json::to_value(value).ok()),
        _ => None,
    };
    root.as_ref()
        .and_then(|value| lookup_scalar_field_value(value, field_path))
        .is_some()
}

fn lookup_scalar_field_value<'a>(
    value: &'a serde_json::Value,
    field_path: &str,
) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for seg in field_path.split('.') {
        current = match current {
            serde_json::Value::Object(map) => map.get(seg)?,
            serde_json::Value::Array(items) => items.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    (!current.is_null()).then_some(current)
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
    let locator_resolution = if should_attempt_auto_locator(&route_result)
        && !has_multiple_locator_targets(prompt, resolved_prompt)
    {
        current_workspace_locator_resolution(&state.skill_rt.workspace_root, &route_result)
            .or_else(|| {
                structured_doc_filename_scalar_locator_resolution(
                    &state.skill_rt.workspace_root,
                    &route_result,
                    state.skill_rt.locator_scan_max_depth,
                    state.skill_rt.locator_scan_max_files,
                )
            })
            .unwrap_or_else(|| {
                let locator_from_hint =
                    if route_result.output_contract.locator_hint.trim().is_empty() {
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
                match locator_from_hint
                    .map(|resolution| match resolution {
                        super::LocatorAutoResolution::Direct(path) => {
                            crate::post_route_policy::LocatorResolution::Direct(path)
                        }
                        super::LocatorAutoResolution::Fuzzy(candidates) => {
                            crate::post_route_policy::LocatorResolution::Fuzzy(candidates)
                        }
                    })
                    .or_else(|| {
                        super::try_resolve_implicit_locator_path(
                            state,
                            prompt,
                            resolved_prompt,
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
                        })
                    }) {
                    Some(resolution) => resolution,
                    None => crate::post_route_policy::LocatorResolution::None,
                }
            })
    } else {
        crate::post_route_policy::LocatorResolution::None
    };
    let request_surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    let post_route = crate::post_route_policy::apply_post_route_policy_with_surface(
        route_result.clone(),
        &request_surface,
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

fn has_multiple_locator_targets(prompt: &str, resolved_prompt: &str) -> bool {
    fn filtered_filename_candidates(text: &str) -> Vec<String> {
        crate::intent::surface_signals::analyze_prompt_surface(text)
            .filename_candidates_excluding_field_selectors()
    }

    fn explicit_locator_candidates(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for token in text.split_whitespace() {
            let trimmed = token
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
                .split(|ch: char| matches!(ch, ',' | '，' | '。' | ';' | '；' | ')' | '）'))
                .next()
                .unwrap_or_default()
                .trim();
            if trimmed.is_empty() || !super::has_explicit_path_or_url_locator_hint(trimmed) {
                continue;
            }
            if seen.insert(trimmed.to_string()) {
                out.push(trimmed.to_string());
            }
        }
        out
    }

    let mut candidates = explicit_locator_candidates(prompt);
    candidates.extend(explicit_locator_candidates(resolved_prompt));
    candidates.extend(filtered_filename_candidates(prompt));
    candidates.extend(filtered_filename_candidates(resolved_prompt));
    candidates.sort();
    candidates.dedup();
    candidates.len() >= 2
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
        prepared_routing.immediate_prior_turn_was_clarify,
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
            prompt,
            route_result,
            fuzzy_locator_suggestions,
        );
        let clarify_context = build_locator_fuzzy_clarify_context(
            recent_execution_context,
            fuzzy_locator_suggestions,
            !suppress_recent_execution_context,
        );
        let structured_clarify_question = structured_missing_locator_clarify_question(
            state,
            route_result,
            fuzzy_locator_suggestions,
        );
        if should_short_circuit_structured_clarify(route_result, fuzzy_locator_suggestions) {
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
        let fallback_source = clarify_fallback_source_for_route(route_result);
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
        clarify_fallback_source_for_route, execution_user_request, has_multiple_locator_targets,
        resolved_intent_inherits_prior_operation, should_attempt_auto_locator,
        should_preserve_original_inline_structured_input, should_reuse_route_clarify_question,
        should_short_circuit_structured_clarify,
        should_suppress_recent_execution_in_clarify_context,
        structured_doc_filename_scalar_locator_resolution,
        structured_missing_locator_clarify_question,
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
    fn llm_failed_route_uses_llm_unavailable_fallback_source() {
        let mut route = clarify_route(crate::OutputLocatorKind::None);
        route.route_reason = "llm_failed_safe_clarify; normalizer_llm_failed".to_string();
        assert_eq!(
            clarify_fallback_source_for_route(&route),
            crate::fallback::ClarifyFallbackSource::LlmUnavailable
        );
    }

    #[test]
    fn unresolved_route_uses_intent_unresolved_fallback_source() {
        let mut route = clarify_route(crate::OutputLocatorKind::None);
        route.route_reason = "clarify_target_missing".to_string();
        assert_eq!(
            clarify_fallback_source_for_route(&route),
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
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
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
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
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
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
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
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        assert!(!should_attempt_auto_locator(&route));
    }

    #[test]
    fn multi_locator_prompt_is_detected_before_auto_locator() {
        assert!(has_multiple_locator_targets(
            "读一下乙的开头，然后顺手说甲是干什么的",
            "读取 /tmp/service_notes.md 的开头部分，然后解释 /tmp/README.md 的用途",
        ));
    }

    #[test]
    fn dotted_field_selector_does_not_count_as_second_locator_target() {
        assert!(!has_multiple_locator_targets(
            "读取 Cargo.toml 的 package.name，只输出值",
            "读取 Cargo.toml 的 package.name，只输出值",
        ));
    }

    #[test]
    fn cargo_filename_scalar_locator_resolution_prefers_clarify_when_multiple_manifests_exist() {
        let root = make_temp_root("cargo_manifest_multi");
        std::fs::write(root.join("Cargo.toml"), "[workspace]\n").expect("root manifest");
        std::fs::create_dir_all(root.join("crates/clawd")).expect("crate dir");
        std::fs::create_dir_all(root.join("crates/claw-core")).expect("second crate dir");
        std::fs::write(
            root.join("crates/clawd/Cargo.toml"),
            "[package]\nname = \"clawd\"\n",
        )
        .expect("crate manifest");
        std::fs::write(
            root.join("crates/claw-core/Cargo.toml"),
            "[package]\nname = \"claw-core\"\n",
        )
        .expect("second crate manifest");

        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "读取 Cargo.toml 的 package.name，只输出值".to_string(),
            needs_clarify: false,
            route_reason: "route_contract:generic_filename_scalar_extract".to_string(),
            route_confidence: Some(0.98),
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
                locator_hint: "Cargo.toml".to_string(),
                requires_content_evidence: true,
                response_shape: crate::OutputResponseShape::Scalar,
                ..Default::default()
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };

        let resolution = structured_doc_filename_scalar_locator_resolution(&root, &route, 4, 1000);
        assert!(matches!(
            resolution,
            Some(crate::post_route_policy::LocatorResolution::Fuzzy(candidates))
                if candidates.len() >= 2
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cargo_filename_scalar_locator_resolution_keeps_unique_manifest_direct() {
        let root = make_temp_root("cargo_manifest_single");
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"single\"\n")
            .expect("single manifest");

        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "读取 Cargo.toml 的 package.name，只输出值".to_string(),
            needs_clarify: false,
            route_reason: "route_contract:generic_filename_scalar_extract".to_string(),
            route_confidence: Some(0.98),
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
                locator_hint: "Cargo.toml".to_string(),
                requires_content_evidence: true,
                response_shape: crate::OutputResponseShape::Scalar,
                ..Default::default()
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };

        let resolution = structured_doc_filename_scalar_locator_resolution(&root, &route, 4, 1000);
        assert!(matches!(
            resolution,
            Some(crate::post_route_policy::LocatorResolution::Direct(path))
                if path.ends_with("Cargo.toml")
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cargo_filename_scalar_locator_resolution_accepts_generic_filename_route() {
        let root = make_temp_root("cargo_manifest_single_generic");
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"single\"\n")
            .expect("single manifest");

        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "读取 Cargo.toml 的 package.name，只输出值".to_string(),
            needs_clarify: false,
            route_reason: "route_contract:generic_filename_scalar_extract".to_string(),
            route_confidence: Some(0.98),
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
                locator_hint: "Cargo.toml".to_string(),
                requires_content_evidence: true,
                response_shape: crate::OutputResponseShape::Scalar,
                ..Default::default()
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };

        let resolution = structured_doc_filename_scalar_locator_resolution(&root, &route, 4, 1000);
        assert!(matches!(
            resolution,
            Some(crate::post_route_policy::LocatorResolution::Direct(path))
                if path.ends_with("Cargo.toml")
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn package_json_filename_scalar_resolution_prefers_unique_shallow_name_bearing_candidate() {
        let root = make_temp_root("package_json_prefer_ui");
        std::fs::write(
            root.join("package.json"),
            "{\"dependencies\":{\"x\":\"1.0.0\"}}",
        )
        .expect("root package");
        std::fs::create_dir_all(root.join("UI")).expect("ui dir");
        std::fs::write(
            root.join("UI/package.json"),
            "{\"name\":\"react-example\",\"version\":\"0.0.0\"}",
        )
        .expect("ui package");
        std::fs::create_dir_all(root.join("services/wa-web-bridge")).expect("service dir");
        std::fs::write(
            root.join("services/wa-web-bridge/package.json"),
            "{\"name\":\"wa-web-bridge\",\"version\":\"0.1.0\"}",
        )
        .expect("service package");
        std::fs::create_dir_all(root.join("node_modules/pkg")).expect("vendor dir");
        std::fs::write(
            root.join("node_modules/pkg/package.json"),
            "{\"name\":\"ignored-vendor\"}",
        )
        .expect("vendor package");

        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "读取 package.json 里的 name 字段，只输出值".to_string(),
            needs_clarify: false,
            route_reason: "route_contract:generic_filename_scalar_extract".to_string(),
            route_confidence: Some(0.98),
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
                locator_hint: "package.json".to_string(),
                requires_content_evidence: true,
                response_shape: crate::OutputResponseShape::Scalar,
                ..Default::default()
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };

        let resolution = structured_doc_filename_scalar_locator_resolution(&root, &route, 4, 1000);
        assert!(matches!(
            resolution,
            Some(crate::post_route_policy::LocatorResolution::Direct(path))
                if path.ends_with("UI/package.json")
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn package_json_filename_scalar_resolution_accepts_generic_filename_route() {
        let root = make_temp_root("package_json_prefer_ui_generic");
        std::fs::write(
            root.join("package.json"),
            "{\"dependencies\":{\"x\":\"1.0.0\"}}",
        )
        .expect("root package");
        std::fs::create_dir_all(root.join("UI")).expect("ui dir");
        std::fs::write(
            root.join("UI/package.json"),
            "{\"name\":\"react-example\",\"version\":\"0.0.0\"}",
        )
        .expect("ui package");

        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "去 package.json 里找 name，只把值给我".to_string(),
            needs_clarify: false,
            route_reason: "route_contract:generic_filename_scalar_extract".to_string(),
            route_confidence: Some(0.98),
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
                locator_hint: "package.json".to_string(),
                requires_content_evidence: true,
                response_shape: crate::OutputResponseShape::Scalar,
                ..Default::default()
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };

        let resolution = structured_doc_filename_scalar_locator_resolution(&root, &route, 4, 1000);
        assert!(matches!(
            resolution,
            Some(crate::post_route_policy::LocatorResolution::Direct(path))
                if path.ends_with("UI/package.json")
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn package_json_filename_scalar_resolution_clarifies_when_shallow_name_bearing_candidates_tie()
    {
        let root = make_temp_root("package_json_tie");
        std::fs::create_dir_all(root.join("UI")).expect("ui dir");
        std::fs::create_dir_all(root.join("web")).expect("web dir");
        std::fs::write(
            root.join("UI/package.json"),
            "{\"name\":\"react-example\",\"version\":\"0.0.0\"}",
        )
        .expect("ui package");
        std::fs::write(
            root.join("web/package.json"),
            "{\"name\":\"web-console\",\"version\":\"0.1.0\"}",
        )
        .expect("web package");

        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "读取 package.json 里的 name 字段，只输出值".to_string(),
            needs_clarify: false,
            route_reason: "route_contract:generic_filename_scalar_extract".to_string(),
            route_confidence: Some(0.98),
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
                locator_hint: "package.json".to_string(),
                requires_content_evidence: true,
                response_shape: crate::OutputResponseShape::Scalar,
                ..Default::default()
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };

        let resolution = structured_doc_filename_scalar_locator_resolution(&root, &route, 4, 1000);
        assert!(matches!(
            resolution,
            Some(crate::post_route_policy::LocatorResolution::Fuzzy(candidates))
                if candidates.len() >= 2
        ));
        let _ = std::fs::remove_dir_all(root);
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
        assert!(should_short_circuit_structured_clarify(&route, &[]));
    }

    #[test]
    fn structured_missing_locator_prefers_explicit_route_question() {
        let mut route = clarify_route(crate::OutputLocatorKind::Filename);
        route.clarify_question = "请提供具体要查看的目录名或路径。".to_string();
        route.output_contract.delivery_intent = crate::OutputDeliveryIntent::DirectoryLookup;
        let state = crate::AppState::test_default_with_fixture_provider();
        let clarify = structured_missing_locator_clarify_question(&state, &route, &[])
            .expect("clarify question");
        assert_eq!(clarify, "请提供具体要查看的目录名或路径。");
    }

    #[test]
    fn content_missing_locator_clarify_short_circuits_to_structured_prompt() {
        let mut route = clarify_route(crate::OutputLocatorKind::Path);
        route.clarify_question.clear();
        route.output_contract.response_shape = crate::OutputResponseShape::Free;
        assert!(should_short_circuit_structured_clarify(&route, &[]));
        let state = crate::AppState::test_default_with_fixture_provider();
        let clarify = structured_missing_locator_clarify_question(&state, &route, &[])
            .expect("content clarify question");
        assert!(
            clarify.contains("路径")
                || clarify.to_ascii_lowercase().contains("path")
                || clarify.contains("文件名")
                || clarify.to_ascii_lowercase().contains("file name")
        );
    }

    #[test]
    fn directory_lookup_missing_locator_uses_directory_specific_question() {
        let mut route = clarify_route(crate::OutputLocatorKind::Filename);
        route.clarify_question.clear();
        route.output_contract.response_shape = crate::OutputResponseShape::Free;
        route.output_contract.delivery_intent = crate::OutputDeliveryIntent::DirectoryLookup;
        let state = crate::AppState::test_default_with_fixture_provider();
        let clarify = structured_missing_locator_clarify_question(&state, &route, &[])
            .expect("directory clarify question");
        assert!(
            clarify.contains("目录") || clarify.to_ascii_lowercase().contains("directory"),
            "unexpected clarify question: {clarify}"
        );
    }

    #[test]
    fn scalar_count_missing_locator_uses_count_specific_question() {
        let mut route = clarify_route(crate::OutputLocatorKind::Filename);
        route.clarify_question.clear();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        let state = crate::AppState::test_default_with_fixture_provider();
        let clarify = structured_missing_locator_clarify_question(&state, &route, &[])
            .expect("count clarify question");
        assert!(
            clarify.contains("统计") || clarify.to_ascii_lowercase().contains("count"),
            "unexpected clarify question: {clarify}"
        );
    }

    #[test]
    fn delivery_missing_locator_clarify_short_circuits_to_structured_prompt() {
        let mut route = clarify_route(crate::OutputLocatorKind::Filename);
        route.clarify_question.clear();
        route.output_contract.requires_content_evidence = false;
        route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        route.output_contract.delivery_required = true;
        route.wants_file_delivery = true;
        assert!(should_short_circuit_structured_clarify(&route, &[]));
        let state = crate::AppState::test_default_with_fixture_provider();
        let clarify = structured_missing_locator_clarify_question(&state, &route, &[])
            .expect("delivery clarify question");
        assert!(
            clarify.contains("路径")
                || clarify.to_ascii_lowercase().contains("path")
                || clarify.contains("文件名")
                || clarify.to_ascii_lowercase().contains("file name")
        );
    }

    #[test]
    fn fuzzy_locator_candidates_short_circuit_to_structured_question() {
        let route = clarify_route(crate::OutputLocatorKind::Filename);
        let state = crate::AppState::test_default_with_fixture_provider();
        let candidates = vec![
            "/tmp/a/Cargo.toml".to_string(),
            "/tmp/b/Cargo.toml".to_string(),
        ];
        assert!(should_short_circuit_structured_clarify(&route, &candidates));
        let clarify = structured_missing_locator_clarify_question(&state, &route, &candidates)
            .expect("fuzzy clarify question");
        assert!(clarify.contains("/tmp/a/Cargo.toml"));
        assert!(clarify.contains("/tmp/b/Cargo.toml"));
        assert!(clarify.contains("1."));
        assert!(clarify.contains("2."));
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
