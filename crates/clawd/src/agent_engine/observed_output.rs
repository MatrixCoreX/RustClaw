use std::path::Path;

use serde::Deserialize;

use super::{AgentRunContext, LoopState};
use crate::{llm_gateway, AppState, ClaimedTask};

#[cfg(test)]
const OBSERVED_ANSWER_FALLBACK_PROMPT_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/observed_answer_fallback_prompt.md");
const OBSERVED_ANSWER_FALLBACK_PROMPT_LOGICAL_PATH: &str =
    "prompts/observed_answer_fallback_prompt.md";

fn render_observed_vars(mut text: String, vars: &[(&str, &str)]) -> String {
    for (name, value) in vars {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

fn observed_t(
    state: Option<&AppState>,
    key: &str,
    default_zh: &str,
    default_en: &str,
    prefer_english: bool,
) -> String {
    observed_t_with_vars(state, key, default_zh, default_en, prefer_english, &[])
}

fn observed_t_with_vars(
    state: Option<&AppState>,
    key: &str,
    default_zh: &str,
    default_en: &str,
    prefer_english: bool,
    vars: &[(&str, &str)],
) -> String {
    match state {
        Some(state) => crate::bilingual_t_with_default_vars(
            state,
            key,
            default_zh,
            default_en,
            prefer_english,
            vars,
        ),
        None => render_observed_vars(
            if prefer_english {
                default_en.to_string()
            } else {
                default_zh.to_string()
            },
            vars,
        ),
    }
}

fn is_internal_missing_scalar_sentinel(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed == "<missing>" || trimmed.ends_with(": <missing>")
}

#[derive(Debug, Clone)]
pub(crate) struct GenericObservedOutput {
    pub(crate) skill: String,
    #[cfg(test)]
    pub(crate) action_label: String,
    pub(crate) body: String,
}

#[derive(Debug, Deserialize)]
struct ObservedAnswerFallbackOut {
    #[serde(default)]
    answer: String,
    #[serde(default)]
    qualified: bool,
    #[serde(default)]
    needs_clarify: bool,
    #[serde(default)]
    is_meta_instruction: bool,
    #[serde(default)]
    publishable: bool,
    #[serde(default)]
    confidence: f64,
    #[serde(default, rename = "reason")]
    _reason: String,
}

fn strip_bare_json_language_prefix(raw: &str) -> &str {
    let trimmed = raw.trim();
    let Some(rest) = trimmed
        .strip_prefix("json")
        .or_else(|| trimmed.strip_prefix("JSON"))
    else {
        return trimmed;
    };
    let rest = rest.trim_start();
    if rest.starts_with('{') || rest.starts_with('[') {
        rest
    } else {
        trimmed
    }
}

fn extract_answer_from_finalizer_envelope_text(raw: &str) -> Option<String> {
    let candidate = strip_bare_json_language_prefix(raw);
    crate::prompt_utils::validate_against_schema::<ObservedAnswerFallbackOut>(
        candidate,
        crate::prompt_utils::PromptSchemaId::FinalizerOut,
    )
    .ok()
    .map(|validated| validated.value.answer.trim().to_string())
    .filter(|answer| !answer.is_empty())
}

fn non_code_markdown_text(raw: &str) -> Option<String> {
    let mut in_fence = false;
    let mut lines = Vec::new();
    for line in raw.lines() {
        let trimmed_start = line.trim_start();
        if trimmed_start.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            lines.push(trimmed.to_string());
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn latest_successful_step_index<F>(loop_state: &LoopState, predicate: F) -> Option<usize>
where
    F: Fn(&crate::executor::StepExecutionResult) -> bool,
{
    loop_state
        .executed_step_results
        .iter()
        .rposition(|step| step.is_ok() && predicate(step))
}

#[cfg(test)]
fn latest_successful_step_output<F>(loop_state: &LoopState, predicate: F) -> Option<String>
where
    F: Fn(&crate::executor::StepExecutionResult) -> bool,
{
    latest_successful_step_index(loop_state, predicate).and_then(|idx| {
        loop_state.executed_step_results[idx]
            .output
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(ToString::to_string)
    })
}

fn has_successful_step_for_skill(loop_state: &LoopState, skill_name: &str) -> bool {
    latest_successful_step_index(loop_state, |step| step.skill == skill_name && step.is_ok())
        .is_some()
}

#[cfg(test)]
pub(crate) fn extract_latest_successful_read_file_output(loop_state: &LoopState) -> Option<String> {
    latest_successful_step_output(loop_state, |step| step.skill == "read_file")
}

#[cfg(test)]
pub(crate) fn extract_latest_successful_list_dir_output(loop_state: &LoopState) -> Option<String> {
    latest_successful_step_output(loop_state, |step| step.skill == "list_dir")
}

pub(crate) fn extract_latest_generic_successful_output(
    loop_state: &LoopState,
) -> Option<GenericObservedOutput> {
    let idx = latest_successful_step_index(loop_state, |step| {
        if matches!(
            step.skill.as_str(),
            "read_file" | "list_dir" | "respond" | "synthesize_answer" | "think"
        ) {
            return false;
        }
        let body = step.output.as_deref().map(str::trim).unwrap_or_default();
        !body.is_empty()
            && (crate::finalize::classify_observed_content_status(body)
                == crate::finalize::ObservedContentStatus::ContentAvailable
                || structured_scalar_candidate(
                    None,
                    None,
                    &step.skill,
                    body,
                    None,
                    None,
                    false,
                    true,
                    false,
                )
                .is_some()
                || structured_observed_body(&step.skill, body).is_some())
            || system_basic_info_value(&step.skill, body).is_some()
            || system_basic_structured_doc_value(&step.skill, body).is_some()
            || system_basic_existence_with_path_value(&step.skill, body).is_some()
    })?;
    let step = &loop_state.executed_step_results[idx];
    let body = step
        .output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())?;
    Some(GenericObservedOutput {
        skill: step.skill.clone(),
        #[cfg(test)]
        action_label: format!("{} skill({}): success", step.step_id, step.skill),
        body: body.to_string(),
    })
}

fn latest_successful_list_dir_answer_candidate(
    loop_state: &LoopState,
    response_shape: Option<crate::OutputResponseShape>,
    auto_locator_path: Option<&str>,
    prefer_full_path: bool,
) -> Option<String> {
    if matches!(
        response_shape,
        Some(crate::OutputResponseShape::OneSentence)
    ) {
        return None;
    }
    let idx = latest_successful_step_index(loop_state, |step| step.skill == "list_dir")?;
    let step = &loop_state.executed_step_results[idx];
    if !step.is_ok() || step.skill != "list_dir" {
        return None;
    }
    let listing = normalized_observed_listing(step.output.as_deref().unwrap_or_default())?;
    if matches!(response_shape, Some(crate::OutputResponseShape::Scalar)) {
        let mut lines = listing
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty());
        let first = lines.next()?;
        if lines.next().is_some() {
            return None;
        }
        if prefer_full_path {
            if let Some(resolved) = resolve_listing_entry_full_path(first, auto_locator_path) {
                return Some(resolved);
            }
        }
        return Some(first.to_string());
    }
    Some(listing)
}

fn canonical_existing_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

fn resolve_listing_entry_full_path(entry: &str, auto_locator_path: Option<&str>) -> Option<String> {
    let entry = entry.trim().trim_end_matches('/');
    if entry.is_empty() {
        return None;
    }
    let auto_locator_path = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(Path::new)?;

    if auto_locator_path.is_file() {
        let file_name_matches = auto_locator_path
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::trim)
            .is_some_and(|name| name.eq_ignore_ascii_case(entry));
        if file_name_matches {
            return Some(canonical_existing_path(auto_locator_path));
        }
        if let Some(parent) = auto_locator_path.parent() {
            let candidate = parent.join(entry);
            if candidate.exists() {
                return Some(canonical_existing_path(&candidate));
            }
        }
    }

    if auto_locator_path.is_dir() {
        let candidate = auto_locator_path.join(entry);
        if candidate.exists() {
            return Some(canonical_existing_path(&candidate));
        }
    }

    None
}

fn normalized_listing_entry_count(listing: &str) -> usize {
    listing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .count()
}

fn normalized_listing_text(listing: &str) -> Option<String> {
    let lines = listing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn looks_like_shell_long_listing_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed == "exit=0" || trimmed.starts_with("total ") {
        return false;
    }
    let Some(first) = trimmed.chars().next() else {
        return false;
    };
    matches!(first, '-' | 'd' | 'l' | 'b' | 'c' | 'p' | 's')
        && trimmed.split_whitespace().count() >= 9
}

fn current_turn_request_text<'a>(
    route: Option<&'a crate::RouteResult>,
    agent_run_context: Option<&'a AgentRunContext>,
) -> Option<&'a str> {
    agent_run_context
        .and_then(|ctx| ctx.original_user_request.as_deref())
        .filter(|text| !text.trim().is_empty())
        .or_else(|| {
            agent_run_context
                .and_then(|ctx| ctx.user_request.as_deref())
                .filter(|text| !text.trim().is_empty())
        })
        .filter(|text| !text.trim().is_empty())
        .or_else(|| {
            route
                .map(|route| route.resolved_intent.as_str())
                .filter(|text| !text.trim().is_empty())
        })
}

fn route_requests_scalar_count(route: &crate::RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::ScalarCount
}

fn route_requests_scalar_existence(route: &crate::RouteResult) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::ExistenceWithPath
        && !route.output_contract.delivery_required
}

fn route_requests_hidden_entries_check(route: &crate::RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::HiddenEntriesCheck
}

pub(crate) fn route_prefers_direct_observed_answer_for_scalar(route: &crate::RouteResult) -> bool {
    route_requests_scalar_existence(route)
        || (route.output_contract.response_shape == crate::OutputResponseShape::Scalar
            && route_requests_hidden_entries_check(route))
}

pub(crate) fn scalar_route_prefers_structured_observed_answer(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && (route_prefers_direct_observed_answer_for_scalar(route)
            || extract_latest_generic_successful_output(loop_state).is_some_and(|observed| {
                observed.skill == "health_check"
                    || observed_output_action_is(&observed, "read_range")
            }))
}

fn observed_output_action_is(observed: &GenericObservedOutput, expected_action: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(&observed.body)
        .ok()
        .and_then(|value| {
            value
                .get("action")
                .and_then(|action| action.as_str())
                .map(|action| action == expected_action)
        })
        .unwrap_or(false)
}

fn route_allows_scalar_read_range_direct_answer(route: &crate::RouteResult) -> bool {
    matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
    ) && !route.output_contract.delivery_required
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
                | crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::RawCommandOutput
        )
}

fn route_requests_scalar_path_only(route: &crate::RouteResult) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::ScalarPathOnly
}

fn route_allows_path_batch_scalar_path_observed_answer(route: &crate::RouteResult) -> bool {
    route_requests_scalar_path_only(route)
        && !route.output_contract.requires_content_evidence
        && !route
            .route_reason
            .contains("execution_required_read_file_extract_scalar")
        && !route
            .route_reason
            .contains("request_requires_fresh_file_observation_to_extract_title")
}

fn recent_file_path_candidate_for_scalar_path(
    loop_state: &LoopState,
    route: Option<&crate::RouteResult>,
) -> Option<String> {
    if !route.is_some_and(route_requests_scalar_path_only) {
        return None;
    }
    let latest_file_step_idx = latest_successful_step_index(loop_state, |step| {
        matches!(step.skill.as_str(), "read_file" | "write_file")
    })?;
    let latest_effective_step_idx = latest_successful_step_index(loop_state, |step| {
        !matches!(
            step.skill.as_str(),
            "respond" | "synthesize_answer" | "think"
        )
    })?;
    if latest_file_step_idx < latest_effective_step_idx {
        return None;
    }
    loop_state
        .output_vars
        .get("last_file_path")
        .or_else(|| loop_state.output_vars.get("last_written_file_path"))
        .or_else(|| loop_state.last_written_file_path.as_ref())
        .or_else(|| loop_state.written_file_aliases.values().next())
        .map(String::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToString::to_string)
}

fn route_prefers_plain_fs_search_paths(route: &crate::RouteResult) -> bool {
    route_requests_scalar_path_only(route)
        || (route.output_contract.response_shape == crate::OutputResponseShape::Strict
            && route.output_contract.locator_kind == crate::OutputLocatorKind::Path
            && route.output_contract.semantic_kind == crate::OutputSemanticKind::ExistenceWithPath
            && !route.output_contract.delivery_required)
        || (matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        ) && route.output_contract.semantic_kind == crate::OutputSemanticKind::FileNames)
        || (matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        ) && route.output_contract.semantic_kind == crate::OutputSemanticKind::DirectoryNames)
        || (matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        ) && route.output_contract.semantic_kind == crate::OutputSemanticKind::FilePaths)
}

fn looks_like_plain_path_literal(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.contains('\n') || trimmed.split_whitespace().count() > 1 {
        return false;
    }
    let path = Path::new(trimmed);
    path.is_absolute()
        || trimmed.starts_with("~/")
        || trimmed.contains('/')
        || trimmed.contains('\\')
}

fn route_scalar_has_plain_path_terminal_respond(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::ExistenceWithPath
        && route.output_contract.locator_kind == crate::OutputLocatorKind::Path
        && !route.output_contract.delivery_required
        && loop_state
            .last_user_visible_respond
            .as_deref()
            .is_some_and(looks_like_plain_path_literal)
}

fn route_allows_raw_listing_direct_answer(route: Option<&crate::RouteResult>) -> bool {
    route.is_none_or(|route| {
        if !route.output_contract.requires_content_evidence {
            return true;
        }
        if !route.output_contract.delivery_required
            && route.output_contract.locator_kind == crate::OutputLocatorKind::Path
            && matches!(
                route.output_contract.semantic_kind,
                crate::OutputSemanticKind::None | crate::OutputSemanticKind::ExistenceWithPath
            )
            && route.ask_mode.is_plain_act()
        {
            return true;
        }
        matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::DirectoryNames
                | crate::OutputSemanticKind::DirectoryEntryGroups
                | crate::OutputSemanticKind::FilePaths
        )
    })
}

fn route_allows_strict_plain_observation_passthrough(route: &crate::RouteResult) -> bool {
    route.ask_mode.finalize_chat_wrapped()
        && route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::None
        && route.output_contract.response_shape == crate::OutputResponseShape::Strict
        && route.output_contract.exact_sentence_count.is_none()
}

fn strict_plain_observation_passthrough_candidate(body: &str) -> Option<String> {
    let lines = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| *line != "exit=0")
        .filter(|line| !is_ignorable_shell_warning(line))
        .collect::<Vec<_>>();
    if lines.is_empty()
        || lines.len() > 80
        || lines.iter().any(|line| {
            looks_like_shell_long_listing_line(line)
                || serde_json::from_str::<serde_json::Value>(line)
                    .map(|value| value.is_object() || value.is_array())
                    .unwrap_or(false)
        })
    {
        return None;
    }
    let candidate = lines.join("\n");
    if crate::finalize::looks_like_planner_artifact(&candidate)
        || crate::finalize::looks_like_internal_trace_artifact(&candidate)
    {
        return None;
    }
    Some(candidate)
}

fn latest_list_dir_listing(loop_state: &LoopState) -> Option<String> {
    let idx = latest_successful_step_index(loop_state, |step| step.skill == "list_dir")?;
    let step = &loop_state.executed_step_results[idx];
    if !step.is_ok() || step.skill != "list_dir" {
        return None;
    }
    normalized_observed_listing(step.output.as_deref().unwrap_or_default())
}

fn hidden_entries_from_listing(listing: &str) -> Vec<String> {
    listing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| is_user_hidden_entry(line))
        .map(ToString::to_string)
        .collect()
}

fn hidden_entries_from_entries(entries: &[String]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| entry.trim())
        .filter(|entry| !entry.is_empty())
        .filter(|entry| is_user_hidden_entry(entry))
        .map(ToString::to_string)
        .collect()
}

fn is_user_hidden_entry(entry: &str) -> bool {
    let normalized = entry.trim().trim_end_matches('/');
    normalized.starts_with('.') && normalized != "." && normalized != ".."
}

fn inventory_dir_hidden_entries(value: &serde_json::Value) -> Option<Vec<String>> {
    let action = value.get("action").and_then(|v| v.as_str())?;
    if action != "inventory_dir" {
        return None;
    }
    let mut hidden = Vec::new();
    if let Some(entries) = value.get("entries").and_then(|v| v.as_array()) {
        for entry in entries {
            let Some(obj) = entry.as_object() else {
                continue;
            };
            let is_hidden = obj
                .get("hidden")
                .and_then(|v| v.as_bool())
                .unwrap_or_else(|| {
                    obj.get("name")
                        .or_else(|| obj.get("path"))
                        .and_then(|v| v.as_str())
                        .is_some_and(is_user_hidden_entry)
                });
            if !is_hidden {
                continue;
            }
            let display = obj
                .get("path")
                .or_else(|| obj.get("name"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string);
            if let Some(display) = display {
                hidden.push(display);
            }
        }
    }
    if !hidden.is_empty() {
        hidden.sort();
        hidden.dedup();
        return Some(hidden);
    }
    let include_hidden = value
        .get("include_hidden")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if include_hidden {
        let names = inventory_dir_names(value).unwrap_or_default();
        let hidden = hidden_entries_from_entries(&names);
        if !hidden.is_empty() {
            return Some(hidden);
        }
    }
    if value
        .get("counts")
        .and_then(|v| v.get("hidden"))
        .and_then(|v| v.as_u64())
        == Some(0)
        && include_hidden
    {
        return Some(Vec::new());
    }
    None
}

fn latest_hidden_entries(loop_state: &LoopState) -> Option<Vec<String>> {
    let idx = latest_successful_step_index(loop_state, |_| true)?;
    let step = &loop_state.executed_step_results[idx];
    let body = step.output.as_deref().unwrap_or_default();
    match step.skill.as_str() {
        "system_basic" | "fs_basic" => serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|value| inventory_dir_hidden_entries(&value)),
        "list_dir" => {
            normalized_observed_listing(body).map(|listing| hidden_entries_from_listing(&listing))
        }
        "run_cmd" => run_cmd_listing_text_candidate(body, None)
            .map(|listing| hidden_entries_from_listing(&listing)),
        _ => None,
    }
}

fn hidden_entries_direct_answer(
    state: Option<&AppState>,
    route: &crate::RouteResult,
    loop_state: &LoopState,
    prefer_english: bool,
) -> Option<String> {
    if !route_requests_hidden_entries_check(route) {
        return None;
    }
    if !matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
    ) {
        return None;
    }
    let hidden_entries = latest_hidden_entries(loop_state).or_else(|| {
        latest_directory_listing_entries(loop_state, None)
            .map(|entries| hidden_entries_from_entries(&entries))
    })?;
    if route.output_contract.response_shape == crate::OutputResponseShape::Scalar {
        return Some(hidden_entries.len().to_string());
    }
    if hidden_entries.is_empty() {
        return Some(observed_t(
            state,
            "clawd.msg.hidden_entries_none",
            "未发现隐藏文件。",
            "No hidden entries found.",
            prefer_english,
        ));
    }
    Some(
        hidden_entries
            .into_iter()
            .take(8)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn listing_entries(listing: &str) -> Vec<String> {
    listing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn latest_directory_listing_entries(
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<Vec<String>> {
    let idx = latest_successful_step_index(loop_state, |_| true)?;
    let step = &loop_state.executed_step_results[idx];
    directory_listing_entries_from_step(step, auto_locator_path)
}

fn directory_listing_entries_from_step(
    step: &crate::executor::StepExecutionResult,
    auto_locator_path: Option<&str>,
) -> Option<Vec<String>> {
    if !step.is_ok() {
        return None;
    }
    let body = step.output.as_deref().unwrap_or_default();
    match step.skill.as_str() {
        "list_dir" => normalized_observed_listing(body).map(|listing| listing_entries(&listing)),
        "run_cmd" => run_cmd_listing_text_candidate(body, auto_locator_path)
            .map(|listing| listing_entries(&listing)),
        "system_basic" | "fs_basic" => {
            let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
            inventory_dir_names(&value)
        }
        _ => None,
    }
    .filter(|entries| !entries.is_empty())
}

fn count_answer_from_latest_listing(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<String> {
    if !route_requests_scalar_count(route) {
        return None;
    }
    let listing = latest_list_dir_listing(loop_state)?;
    Some(normalized_listing_entry_count(&listing).to_string())
}

fn count_answer_from_latest_fs_search(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<String> {
    if !route_requests_scalar_count(route) {
        return None;
    }
    let observed = extract_latest_generic_successful_output(loop_state)?;
    if observed.skill != "fs_search" {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(&observed.body).ok()?;
    match value.get("action").and_then(|v| v.as_str())? {
        action
            if action.eq_ignore_ascii_case("find_name")
                || action.eq_ignore_ascii_case("find_ext") =>
        {
            value.get("count").and_then(value_scalar_text)
        }
        action if action.eq_ignore_ascii_case("grep_text") => value
            .get("match_count")
            .or_else(|| value.get("count"))
            .and_then(value_scalar_text),
        _ => None,
    }
}

fn trim_for_observed_prompt(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out = trimmed.chars().take(max_chars).collect::<String>();
    out.push_str("\n...[truncated]");
    out
}

fn looks_like_structured_machine_output_line(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .map(|value| value.is_object() || value.is_array())
        .unwrap_or(false)
}

fn normalized_scalar_candidate(body: &str) -> Option<String> {
    let lines = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| *line != "exit=0")
        .filter(|line| !is_ignorable_shell_warning(line))
        .filter(|line| !looks_like_structured_machine_output_line(line))
        .collect::<Vec<_>>();
    (lines.len() == 1).then(|| lines[0].to_string())
}

fn numeric_scalar_text(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty() && trimmed.parse::<f64>().is_ok()
}

fn scalar_count_diagnostic_line_for_answer(
    answer: &str,
    route: Option<&crate::RouteResult>,
    loop_state: &LoopState,
) -> Option<String> {
    let route = route?;
    if !route_requests_scalar_count(route) || !numeric_scalar_text(answer) {
        return None;
    }
    let observed = extract_latest_generic_successful_output(loop_state)?;
    if observed.skill == "archive_basic" && archive_list_summary_from_body(&observed.body).is_some()
    {
        return None;
    }
    let lines = observed
        .body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| *line != "exit=0")
        .filter(|line| !is_ignorable_shell_warning(line))
        .filter(|line| !looks_like_structured_machine_output_line(line))
        .collect::<Vec<_>>();
    if lines.len() <= 1 {
        return None;
    }
    lines
        .into_iter()
        .find(|line| !numeric_scalar_text(line))
        .map(ToString::to_string)
}

fn value_scalar_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => Some("null".to_string()),
        serde_json::Value::Bool(v) => Some(v.to_string()),
        serde_json::Value::Number(v) => Some(v.to_string()),
        serde_json::Value::String(v) => Some(v.trim().to_string()).filter(|v| !v.is_empty()),
        _ => None,
    }
}

fn value_structured_text(value: &serde_json::Value, value_text: Option<&str>) -> Option<String> {
    value_text
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
        .or_else(|| serde_json::to_string(value).ok())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArchiveListEntry {
    name: String,
    size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArchiveListSummary {
    archive: Option<String>,
    entries: Vec<ArchiveListEntry>,
}

#[derive(Debug, Clone)]
struct StructuredScalarObservation {
    text: String,
    source_key: String,
}

fn structured_scalar_observation_from_extract_item(
    value: &serde_json::Value,
    parent: Option<&serde_json::Value>,
) -> Option<StructuredScalarObservation> {
    if !value
        .get("exists")
        .and_then(|item| item.as_bool())
        .unwrap_or(false)
    {
        return None;
    }
    let raw_value = value.get("value").unwrap_or(&serde_json::Value::Null);
    if matches!(
        raw_value,
        serde_json::Value::Object(_) | serde_json::Value::Array(_)
    ) {
        return None;
    }
    value
        .get("value_text")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| StructuredScalarObservation {
            text: text.to_string(),
            source_key: structured_scalar_observation_source_key(value, parent),
        })
        .or_else(|| {
            value_scalar_text(raw_value).map(|text| StructuredScalarObservation {
                text,
                source_key: structured_scalar_observation_source_key(value, parent),
            })
        })
}

fn structured_scalar_observation_source_key(
    value: &serde_json::Value,
    parent: Option<&serde_json::Value>,
) -> String {
    let path = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .or_else(|| parent.and_then(|parent| parent.get("resolved_path")))
        .or_else(|| parent.and_then(|parent| parent.get("path")))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let field = value
        .get("resolved_field_path")
        .or_else(|| value.get("field_path"))
        .or_else(|| parent.and_then(|parent| parent.get("resolved_field_path")))
        .or_else(|| parent.and_then(|parent| parent.get("field_path")))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if path.is_empty() && field.is_empty() {
        String::new()
    } else {
        format!(
            "{}\n{}",
            path.to_ascii_lowercase(),
            field.to_ascii_lowercase()
        )
    }
}

fn structured_scalar_observation_from_step(
    step: &crate::executor::StepExecutionResult,
) -> Option<StructuredScalarObservation> {
    if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "config_basic") {
        return None;
    }
    let body = step.output.as_deref()?.trim();
    if body.is_empty() {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    match value.get("action").and_then(|item| item.as_str()) {
        Some("extract_field" | "read_field") => {
            structured_scalar_observation_from_extract_item(&value, None)
        }
        Some("extract_fields" | "read_fields") => {
            let results = value.get("results")?.as_array()?;
            if results.len() != 1 {
                return None;
            }
            structured_scalar_observation_from_extract_item(results.first()?, Some(&value))
        }
        _ => None,
    }
}

fn recent_structured_scalar_observations(
    loop_state: &LoopState,
    limit: usize,
) -> Vec<StructuredScalarObservation> {
    let mut recent = loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter_map(structured_scalar_observation_from_step)
        .take(limit.max(1))
        .collect::<Vec<_>>();
    recent.reverse();
    recent
}

pub(crate) fn recent_structured_scalar_observation_count(loop_state: &LoopState) -> usize {
    recent_structured_scalar_observations(loop_state, 2).len()
}

pub(crate) fn latest_structured_scalar_observation_text(loop_state: &LoopState) -> Option<String> {
    recent_structured_scalar_observations(loop_state, 1)
        .into_iter()
        .next()
        .map(|observation| observation.text)
}

pub(crate) fn structured_scalar_equality_direct_answer(
    _state: Option<&AppState>,
    route: &crate::RouteResult,
    loop_state: &LoopState,
    _agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::RecentScalarEqualityCheck
        || route.output_contract.delivery_required
    {
        return None;
    }
    let observations = recent_structured_scalar_observations(loop_state, 2);
    if observations.len() < 2 {
        return None;
    }
    let left = observations[0].text.trim();
    let right = observations[1].text.trim();
    if left.is_empty() || right.is_empty() {
        return None;
    }
    if !observations[0].source_key.is_empty()
        && observations[0].source_key == observations[1].source_key
    {
        return None;
    }
    let same = left == right;
    Some(format!("{left} {} {right}", if same { "==" } else { "!=" }))
}

fn route_needs_structured_scalar_pair_synthesis(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(|route| {
            recent_structured_scalar_observation_count(loop_state) > 1
                && matches!(
                    route.output_contract.semantic_kind,
                    crate::OutputSemanticKind::RecentScalarEqualityCheck
                        | crate::OutputSemanticKind::QuantityComparison
                )
        })
}

fn observed_request_language_hint(user_text: &str) -> &'static str {
    crate::language_policy::request_language_hint(user_text)
}

fn observed_language_supports_bilingual_template(language_hint: &str) -> bool {
    let hint = language_hint.trim().to_ascii_lowercase();
    hint == "config_default" || hint.starts_with("en") || hint.starts_with("zh")
}

fn route_should_synthesize_non_bilingual_existence_with_path(
    route: Option<&crate::RouteResult>,
    allow_localized_direct_template: bool,
) -> bool {
    if allow_localized_direct_template {
        return false;
    }
    let Some(route) = route else {
        return false;
    };
    route.output_contract.semantic_kind == crate::OutputSemanticKind::ExistenceWithPath
        && crate::contract_matrix::final_answer_shape_for_output_contract(&route.output_contract)
            .is_some_and(|shape| shape.allows_model_language())
}

fn observed_request_prefers_english_template(
    state: Option<&AppState>,
    language_hint: &str,
) -> bool {
    let hint = language_hint.trim().to_ascii_lowercase();
    if hint.starts_with("zh") {
        return false;
    }
    if hint.starts_with("en") {
        return true;
    }
    if hint == "mixed" {
        return false;
    }
    if hint == "config_default" {
        return state
            .map(|state| {
                state
                    .policy
                    .command_intent
                    .default_locale
                    .to_ascii_lowercase()
                    .starts_with("en")
            })
            .unwrap_or(false);
    }
    true
}

fn observed_response_style_hint(agent_run_context: Option<&AgentRunContext>) -> String {
    if let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) {
        if route_disallows_direct_observation_passthrough(route) {
            if let Some(count) = route.output_contract.exact_sentence_count {
                let sentence_label = if count == 1 { "sentence" } else { "sentences" };
                return format!(
                    "Use the observed output as evidence to produce exactly {count} {sentence_label}. Do not answer by copying only the raw observed output; that would be an incomplete passthrough for this contract."
                );
            }
            if route.output_contract.response_shape == crate::OutputResponseShape::OneSentence {
                return "Use the observed output as evidence to produce exactly one sentence. Do not answer by copying only the raw observed output; that would be an incomplete passthrough for this contract.".to_string();
            }
            return "Use the observed output as evidence to produce the requested final wording. Do not answer by copying only the raw observed output; that would be an incomplete passthrough for this contract.".to_string();
        }
    }
    let response_shape = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.output_contract.response_shape);
    let semantic_kind = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.output_contract.semantic_kind);
    if semantic_kind == Some(crate::OutputSemanticKind::RawCommandOutput)
        && response_shape == Some(crate::OutputResponseShape::Strict)
    {
        return "Use the observed command output as the value for the exact format requested by the user. If the user asked for a prefix, suffix, template, or key=value shape, apply that formatting instead of returning the raw command output unchanged.".to_string();
    }
    if let Some(count) = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .and_then(|route| route.output_contract.exact_sentence_count)
    {
        let sentence_label = if count == 1 { "sentence" } else { "sentences" };
        return format!(
            "Return exactly {count} {sentence_label}. Do not compress the answer into fewer sentences or expand beyond that count."
        );
    }
    if semantic_kind == Some(crate::OutputSemanticKind::ExistenceWithPathSummary) {
        return "Return whether the target exists, the resolved path when found, and the requested brief content-grounded purpose or summary. Do not answer from path evidence alone if content evidence is available.".to_string();
    }
    if semantic_kind == Some(crate::OutputSemanticKind::ExistenceWithPath) {
        return "Return a concise existence verdict and include the target path or observed path. This path requirement overrides response_shape=scalar unless the original user explicitly requested one bare boolean/scalar. Do not reduce the answer to only yes/no/exists/missing.".to_string();
    }
    if semantic_kind == Some(crate::OutputSemanticKind::ScalarCount)
        && response_shape != Some(crate::OutputResponseShape::Scalar)
    {
        return "Use observed numeric fields to answer the requested count dimensions. Do not collapse component counts into only an aggregate total unless the user explicitly asked for only the aggregate.".to_string();
    }
    match response_shape {
        Some(crate::OutputResponseShape::Scalar) => {
            "Return only the final scalar value with no label, prefix, suffix, or explanation."
        }
        Some(crate::OutputResponseShape::FileToken) => {
            "Return only the delivery token or delivery-marker output itself. Do not add explanation."
        }
        Some(crate::OutputResponseShape::OneSentence) => {
            "Return exactly one sentence unless the current user request explicitly asks for another exact sentence count."
        }
        Some(crate::OutputResponseShape::Strict) => {
            "Return exactly the format requested by the user. Do not add execution traces, headings, prefixes, suffixes, or extra explanation."
        }
        Some(crate::OutputResponseShape::Free) => {
            "Return a short direct answer: one short paragraph or compact listing plus one concise conclusion."
        }
        None => "Return the shortest grounded answer that directly satisfies the user request.",
    }
    .to_string()
}

pub(crate) fn route_requires_synthesized_delivery(route: &crate::RouteResult) -> bool {
    if route_allows_strict_plain_observation_passthrough(route) {
        return false;
    }
    let constrained_sentence_delivery = route.output_contract.response_shape
        == crate::OutputResponseShape::OneSentence
        || route.output_contract.exact_sentence_count.is_some();
    route.ask_mode.finalize_chat_wrapped()
        && route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::None
        && constrained_sentence_delivery
}

pub(crate) fn route_disallows_direct_observation_passthrough(route: &crate::RouteResult) -> bool {
    if route_requires_synthesized_delivery(route) {
        return true;
    }
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
    {
        return false;
    }
    if !matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
            | crate::OutputSemanticKind::ContentExcerptWithSummary
            | crate::OutputSemanticKind::ExcerptKindJudgment
    ) {
        return false;
    }
    matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
    ) || route.output_contract.exact_sentence_count.is_some()
}

fn db_basic_scalar_candidate(value: &serde_json::Value) -> Option<String> {
    let columns = value.get("columns")?.as_array()?;
    if columns.len() != 1 {
        return None;
    }
    let column = columns[0].as_str()?.trim();
    if column.is_empty() {
        return None;
    }
    let rows = value.get("rows")?.as_array()?;
    if rows.len() != 1 {
        return None;
    }
    let row = rows.first()?.as_object()?;
    value_scalar_text(row.get(column)?)
}

fn db_basic_count_candidate(value: &serde_json::Value) -> Option<String> {
    let columns = value.get("columns")?.as_array()?;
    let rows = value.get("rows")?.as_array()?;
    if rows.len() == 1 && columns.len() == 1 {
        return db_basic_scalar_candidate(value);
    }
    Some(rows.len().to_string())
}

fn db_basic_table_names(value: &serde_json::Value) -> Option<Vec<String>> {
    let columns = value.get("columns")?.as_array()?;
    if columns.len() != 1 {
        return None;
    }
    let column_name = columns[0].as_str()?.trim();
    if column_name != "name" {
        return None;
    }
    let rows = value.get("rows")?.as_array()?;
    Some(
        rows.iter()
            .filter_map(|row| row.as_object())
            .filter_map(|row| row.get(column_name))
            .filter_map(value_scalar_text)
            .collect(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SqliteTableObservedOutputKind {
    Listing,
    NamesOnly,
}

fn sqlite_table_observed_output_kind(
    route: &crate::RouteResult,
) -> Option<SqliteTableObservedOutputKind> {
    let locator_hint = route
        .output_contract
        .locator_hint
        .trim()
        .to_ascii_lowercase();
    if !(locator_hint.ends_with(".sqlite") || locator_hint.ends_with(".db")) {
        return None;
    }
    match route.output_contract.semantic_kind {
        crate::OutputSemanticKind::SqliteTableListing => {
            Some(SqliteTableObservedOutputKind::Listing)
        }
        crate::OutputSemanticKind::SqliteTableNamesOnly => {
            Some(SqliteTableObservedOutputKind::NamesOnly)
        }
        _ => None,
    }
}

fn db_basic_tables_summary_candidate(
    state: Option<&AppState>,
    route: &crate::RouteResult,
    body: &str,
    prefer_english: bool,
) -> Option<String> {
    let observed_kind = sqlite_table_observed_output_kind(route)?;
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let table_names = db_basic_table_names(&value)?;
    if table_names.is_empty() {
        return Some(observed_t(
            state,
            "clawd.msg.sqlite_no_tables",
            "这个 SQLite 文件里目前没有任何表。",
            "This SQLite file currently has no tables.",
            prefer_english,
        ));
    }
    if observed_kind == SqliteTableObservedOutputKind::NamesOnly {
        return Some(table_names.join("\n"));
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SqliteDatabaseKindClass {
    Business,
    Test,
}

impl SqliteDatabaseKindClass {
    fn label(self, prefer_english: bool) -> &'static str {
        match (self, prefer_english) {
            (Self::Business, true) => "more like a business database",
            (Self::Business, false) => "更像业务库",
            (Self::Test, true) => "more like a test database",
            (Self::Test, false) => "更像测试库",
        }
    }
}

fn sqlite_database_kind_from_contract_selector(
    request_text: Option<&str>,
) -> Option<SqliteDatabaseKindClass> {
    let value =
        crate::intent_router::contract_test_hint_value(request_text?, "selector_database_kind")
            .or_else(|| {
                crate::intent_router::contract_test_hint_value(
                    request_text?,
                    "selector_expected_kind",
                )
            })?;
    match value.trim().to_ascii_lowercase().as_str() {
        "business" | "business_database" | "prod" | "production" | "production_database" => {
            Some(SqliteDatabaseKindClass::Business)
        }
        "test" | "test_database" | "fixture" | "fixture_database" | "sample" | "demo" => {
            Some(SqliteDatabaseKindClass::Test)
        }
        _ => None,
    }
}

fn sqlite_database_kind_from_locator(
    route: &crate::RouteResult,
) -> Option<SqliteDatabaseKindClass> {
    let locator = route
        .output_contract
        .locator_hint
        .trim()
        .to_ascii_lowercase();
    if locator.is_empty() {
        return None;
    }
    if ["fixture", "fixtures", "test", "sample", "demo", "mock"]
        .iter()
        .any(|token| locator.contains(token))
    {
        return Some(SqliteDatabaseKindClass::Test);
    }
    if ["prod", "production", "business"]
        .iter()
        .any(|token| locator.contains(token))
    {
        return Some(SqliteDatabaseKindClass::Business);
    }
    None
}

fn db_basic_database_kind_judgment_candidate(
    route: &crate::RouteResult,
    body: &str,
    request_text: Option<&str>,
    prefer_english: bool,
) -> Option<String> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::SqliteDatabaseKindJudgment
    {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let table_names = db_basic_table_names(&value)?;
    if table_names.is_empty() {
        return None;
    }
    sqlite_database_kind_judgment_answer(route, &table_names, request_text, prefer_english)
}

fn sqlite_database_kind_judgment_answer(
    route: &crate::RouteResult,
    table_names: &[String],
    request_text: Option<&str>,
    prefer_english: bool,
) -> Option<String> {
    if table_names.is_empty() {
        return None;
    }
    let kind = sqlite_database_kind_from_contract_selector(request_text)
        .or_else(|| sqlite_database_kind_from_locator(route))?;
    let tables = if prefer_english {
        table_names.join(", ")
    } else {
        table_names.join("、")
    };
    let locator = route.output_contract.locator_hint.trim();
    if prefer_english {
        if locator.is_empty() {
            Some(format!(
                "{}; evidence: observed tables include {}.",
                kind.label(true),
                tables
            ))
        } else {
            Some(format!(
                "{}; evidence: observed tables include {}, and the database path is `{}`.",
                kind.label(true),
                tables,
                locator
            ))
        }
    } else if locator.is_empty() {
        Some(format!(
            "{}；依据：观测到的表包括 {}。",
            kind.label(false),
            tables
        ))
    } else {
        Some(format!(
            "{}；依据：观测到的表包括 {}，数据库路径为 `{}`。",
            kind.label(false),
            tables,
            locator
        ))
    }
}

fn run_cmd_sqlite_table_names(body: &str) -> Vec<String> {
    body.split_whitespace()
        .map(str::trim)
        .filter(|token| !token.is_empty() && !token.starts_with("exit="))
        .filter(|token| {
            token
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        })
        .take(64)
        .map(ToString::to_string)
        .collect()
}

fn run_cmd_sqlite_schema_version(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("exit="))
        .find_map(|line| {
            line.strip_prefix("schema_version=")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .or_else(|| {
                    line.chars()
                        .all(|ch| ch.is_ascii_digit())
                        .then(|| line.to_string())
                })
        })
}

fn sqlite_table_listing_markdown(table_names: &[String]) -> Option<String> {
    if table_names.is_empty() {
        return None;
    }
    let mut lines = vec!["| name |".to_string(), "| --- |".to_string()];
    lines.extend(table_names.iter().map(|name| format!("| {name} |")));
    Some(lines.join("\n"))
}

fn run_cmd_sqlite_direct_answer_candidate(
    route: &crate::RouteResult,
    body: &str,
    request_text: Option<&str>,
    prefer_english: bool,
) -> Option<String> {
    match route.output_contract.semantic_kind {
        crate::OutputSemanticKind::SqliteDatabaseKindJudgment => {
            let table_names = run_cmd_sqlite_table_names(body);
            sqlite_database_kind_judgment_answer(route, &table_names, request_text, prefer_english)
        }
        crate::OutputSemanticKind::SqliteSchemaVersion => {
            run_cmd_sqlite_schema_version(body).map(|value| format!("schema_version={value}"))
        }
        crate::OutputSemanticKind::SqliteTableNamesOnly => {
            let table_names = run_cmd_sqlite_table_names(body);
            (!table_names.is_empty()).then(|| table_names.join("\n"))
        }
        crate::OutputSemanticKind::SqliteTableListing => {
            let table_names = run_cmd_sqlite_table_names(body);
            sqlite_table_listing_markdown(&table_names)
        }
        _ => None,
    }
}

fn db_basic_database_kind_judgment_from_loop_state_candidate(
    route: &crate::RouteResult,
    loop_state: &LoopState,
    request_text: Option<&str>,
    prefer_english: bool,
) -> Option<String> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::SqliteDatabaseKindJudgment
    {
        return None;
    }
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.skill == "db_basic" && step.is_ok())
        .filter_map(|step| step.output.as_deref())
        .find_map(|body| {
            db_basic_database_kind_judgment_candidate(route, body, request_text, prefer_english)
        })
}

pub(crate) fn transform_skill_formatted_output_candidate(body: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let status_ok = value
        .get("status")
        .and_then(|value| value.as_str())
        .map(|status| status.eq_ignore_ascii_case("ok"))
        .unwrap_or(false);
    if !status_ok {
        return None;
    }
    value
        .get("formatted")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|formatted| !formatted.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            value
                .get("output")
                .filter(|output| !output.is_null())
                .and_then(|output| serde_json::to_string(output).ok())
        })
        .or_else(|| {
            value
                .get("result")
                .filter(|result| !result.is_null())
                .and_then(|result| serde_json::to_string(result).ok())
        })
}

fn service_control_summary_candidate(value: &serde_json::Value) -> Option<String> {
    value
        .get("summary")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

fn service_control_state_value(value: &serde_json::Value) -> Option<&str> {
    value
        .get("post_state")
        .or_else(|| value.get("pre_state"))
        .or_else(|| value.get("summary"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

fn service_control_state_is_running(state: &str) -> bool {
    state
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
        .map(|token| token.to_ascii_lowercase())
        .any(|token| matches!(token.as_str(), "active" | "running"))
}

fn service_control_status_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    response_shape: Option<crate::OutputResponseShape>,
    prefer_english: bool,
) -> Option<String> {
    let service_name = value
        .get("service_name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("service");
    let service_state = service_control_state_value(value)?;
    if matches!(response_shape, Some(crate::OutputResponseShape::Scalar)) {
        return Some(service_state.to_string());
    }
    let manager = value
        .get("manager_type")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("service manager");
    let verified = value
        .get("verified")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if service_control_state_is_running(service_state) {
        let key = if verified {
            "clawd.msg.service_status_running_verified"
        } else {
            "clawd.msg.service_status_running"
        };
        let zh = if verified {
            "{service} 正在运行：{manager} 返回 `{state}`，验证通过。"
        } else {
            "{service} 正在运行：{manager} 返回 `{state}`。"
        };
        let en = if verified {
            "{service} is running: {manager} reports `{state}` and verification passed."
        } else {
            "{service} is running: {manager} reports `{state}`."
        };
        return Some(observed_t_with_vars(
            state,
            key,
            zh,
            en,
            prefer_english,
            &[
                ("service", service_name),
                ("manager", manager),
                ("state", service_state),
            ],
        ));
    }
    Some(observed_t_with_vars(
        state,
        "clawd.msg.service_status_not_running",
        "{service} 当前状态是 `{state}`：{manager} 已完成检查，未显示为运行中。",
        "{service} is currently `{state}`: {manager} completed the check and it is not reported as running.",
        prefer_english,
        &[
            ("service", service_name),
            ("manager", manager),
            ("state", service_state),
        ],
    ))
}

fn health_check_service_status_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    response_shape: Option<crate::OutputResponseShape>,
    prefer_english: bool,
) -> Option<String> {
    let process_count = value.get("clawd_process_count").and_then(|v| v.as_i64());
    let port_open = value
        .get("clawd_health_port_open")
        .and_then(|v| v.as_bool());
    let status = match (process_count, port_open) {
        (Some(count), Some(true)) if count > 0 => "running",
        (Some(count), _) if count <= 0 => "not_running",
        (Some(_), Some(false)) => "degraded",
        (None, Some(true)) => "reachable",
        _ => "unknown",
    };
    if matches!(response_shape, Some(crate::OutputResponseShape::Scalar)) {
        return Some(status.to_string());
    }
    if health_check_output_needs_diagnostic_synthesis(value) {
        return None;
    }
    let process_count_text = process_count
        .map(|count| count.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let port_text = port_open
        .map(|open| open.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let key = match status {
        "running" => "clawd.msg.health_check_service_status_running",
        "not_running" => "clawd.msg.health_check_service_status_not_running",
        "degraded" => "clawd.msg.health_check_service_status_degraded",
        "reachable" => "clawd.msg.health_check_service_status_reachable",
        _ => "clawd.msg.health_check_service_status_unknown",
    };
    let (zh, en) = match status {
        "running" => (
            "clawd 正在运行：health_check 显示 clawd_process_count={process_count}，clawd_health_port_open={port_open}。",
            "clawd is running: health_check reports clawd_process_count={process_count} and clawd_health_port_open={port_open}.",
        ),
        "not_running" => (
            "clawd 未运行：health_check 显示 clawd_process_count={process_count}，clawd_health_port_open={port_open}。",
            "clawd is not running: health_check reports clawd_process_count={process_count} and clawd_health_port_open={port_open}.",
        ),
        "degraded" => (
            "clawd 状态异常：health_check 显示 clawd_process_count={process_count}，但 clawd_health_port_open={port_open}。",
            "clawd is degraded: health_check reports clawd_process_count={process_count}, but clawd_health_port_open={port_open}.",
        ),
        "reachable" => (
            "clawd 可访问性部分正常：health_check 显示 clawd_health_port_open={port_open}，但 clawd_process_count={process_count}。",
            "clawd appears reachable: health_check reports clawd_health_port_open={port_open}, but clawd_process_count={process_count}.",
        ),
        _ => (
            "clawd 状态不明确：health_check 未提供完整的 clawd_process_count 和 clawd_health_port_open。",
            "clawd status is unclear: health_check did not provide complete clawd_process_count and clawd_health_port_open fields.",
        ),
    };
    Some(observed_t_with_vars(
        state,
        key,
        zh,
        en,
        prefer_english,
        &[
            ("process_count", process_count_text.as_str()),
            ("port_open", port_text.as_str()),
        ],
    ))
}

fn health_check_output_needs_diagnostic_synthesis(value: &serde_json::Value) -> bool {
    if value
        .get("system_health")
        .and_then(|system_health| system_health.as_object())
        .is_some_and(|system_health| !system_health.is_empty())
    {
        return true;
    }
    ["clawd_log", "telegramd_log"].iter().any(|field| {
        value
            .get(*field)
            .and_then(|log| log.get("keyword_error_count"))
            .and_then(|count| count.as_i64())
            .is_some_and(|count| count > 0)
    })
}

fn process_basic_service_status_direct_answer_candidate(
    state: Option<&AppState>,
    body: &str,
    response_shape: Option<crate::OutputResponseShape>,
    prefer_english: bool,
) -> Option<String> {
    if let Some(answer) =
        process_basic_port_list_direct_answer_candidate(state, body, response_shape, prefer_english)
    {
        return Some(answer);
    }
    let rows = process_basic_table_rows(body);
    let status = if rows.is_empty() {
        "not_running"
    } else {
        "running"
    };
    if matches!(response_shape, Some(crate::OutputResponseShape::Scalar)) {
        return Some(status.to_string());
    }
    if let Some(answer) =
        process_basic_ps_inventory_direct_answer_candidate(state, &rows, prefer_english)
    {
        return Some(answer);
    }
    let no_match_filter = process_basic_no_match_filter(body);
    let row_count_text = rows.len().to_string();
    let comm = rows
        .first()
        .and_then(|row| row.split_whitespace().last())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("unknown");
    let subject = if rows.is_empty() {
        no_match_filter.as_deref().unwrap_or("process")
    } else if comm == "unknown" {
        "process"
    } else {
        comm
    };
    let key = if rows.is_empty() {
        "clawd.msg.process_basic_service_status_not_running"
    } else {
        "clawd.msg.process_basic_service_status_running"
    };
    let (zh, en) = if rows.is_empty() {
        (
            "{subject} 未运行：process_basic 没有返回匹配的进程记录。",
            "{subject} is not running: process_basic returned no matching process records.",
        )
    } else {
        (
            "{subject} 正在运行：process_basic 返回 {count} 条进程记录，COMM={comm}。",
            "{subject} is running: process_basic returned {count} process record(s), COMM={comm}.",
        )
    };
    Some(observed_t_with_vars(
        state,
        key,
        zh,
        en,
        prefer_english,
        &[
            ("count", row_count_text.as_str()),
            ("comm", comm),
            ("subject", subject),
        ],
    ))
}

#[derive(Debug, Clone, PartialEq)]
struct ProcessBasicPsRow {
    pid: String,
    cpu: String,
    mem: String,
    comm: String,
}

fn process_basic_ps_inventory_direct_answer_candidate(
    state: Option<&AppState>,
    rows: &[&str],
    prefer_english: bool,
) -> Option<String> {
    let rows = rows
        .iter()
        .filter_map(|row| process_basic_ps_row(row))
        .collect::<Vec<_>>();
    if rows.len() < 2 {
        return None;
    }
    let top = rows.first()?;
    let list = rows
        .iter()
        .enumerate()
        .map(|(idx, row)| {
            format!(
                "{}. {} CPU {}% MEM {}% PID {}",
                idx + 1,
                row.comm,
                row.cpu,
                row.mem,
                row.pid
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    Some(observed_t_with_vars(
        state,
        "clawd.msg.process_basic_ps_inventory_summary",
        "当前 CPU 占用最高的 {count} 个进程：{list}。最值得注意的是 {top_comm}（CPU {top_cpu}%，PID {top_pid}）。",
        "Top {count} processes by CPU: {list}. Most notable: {top_comm} (CPU {top_cpu}%, PID {top_pid}).",
        prefer_english,
        &[
            ("count", &rows.len().to_string()),
            ("list", &list),
            ("top_comm", top.comm.as_str()),
            ("top_cpu", top.cpu.as_str()),
            ("top_pid", top.pid.as_str()),
        ],
    ))
}

fn process_basic_ps_row(row: &str) -> Option<ProcessBasicPsRow> {
    let columns = row.split_whitespace().collect::<Vec<_>>();
    if columns.len() < 5 || !columns[0].chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(ProcessBasicPsRow {
        pid: columns[0].to_string(),
        cpu: columns[2].to_string(),
        mem: columns[3].to_string(),
        comm: columns[4..].join(" "),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessBasicPortRow {
    local: String,
    port: String,
    process: Option<String>,
}

fn process_basic_port_list_direct_answer_candidate(
    _state: Option<&AppState>,
    body: &str,
    response_shape: Option<crate::OutputResponseShape>,
    _prefer_english: bool,
) -> Option<String> {
    let rows = process_basic_port_rows(body);
    if rows.is_empty() {
        return None;
    }
    if matches!(response_shape, Some(crate::OutputResponseShape::Scalar)) {
        return Some(rows.len().to_string());
    }
    let mut lines = vec![format!("port.count={}", rows.len())];
    lines.extend(
        rows.iter()
            .enumerate()
            .map(|(idx, row)| process_basic_port_row_label(idx, row)),
    );
    Some(lines.join("\n"))
}

fn process_basic_port_row_label(idx: usize, row: &ProcessBasicPortRow) -> String {
    let exposure = if process_basic_local_addr_is_loopback(&row.local) {
        "loopback"
    } else {
        "public_bind"
    };
    match row.process.as_deref().filter(|value| !value.is_empty()) {
        Some(process) => format!(
            "port[{idx}].number={}\nport[{idx}].local={}\nport[{idx}].exposure={}\nport[{idx}].process={}",
            row.port, row.local, exposure, process
        ),
        None => format!(
            "port[{idx}].number={}\nport[{idx}].local={}\nport[{idx}].exposure={}",
            row.port, row.local, exposure
        ),
    }
}

fn process_basic_port_rows(body: &str) -> Vec<ProcessBasicPortRow> {
    let mut rows = Vec::new();
    for line in body.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line.starts_with("exit=")
            || line.contains("Local Address:Port")
            || line.starts_with("COMMAND ")
        {
            continue;
        }
        let Some(local) = process_basic_local_address_from_port_line(line) else {
            continue;
        };
        let Some(port) = process_basic_port_from_local_address(&local) else {
            continue;
        };
        if rows
            .iter()
            .any(|row: &ProcessBasicPortRow| row.port == port && row.local == local)
        {
            continue;
        }
        rows.push(ProcessBasicPortRow {
            local,
            port,
            process: process_basic_process_name_from_port_line(line),
        });
    }
    rows
}

fn process_basic_local_address_from_port_line(line: &str) -> Option<String> {
    let columns = line.split_whitespace().collect::<Vec<_>>();
    if columns.first().is_some_and(|column| *column == "LISTEN") && columns.len() >= 4 {
        return Some(columns[3].to_string());
    }
    if columns.iter().any(|column| *column == "(LISTEN)") {
        return columns
            .iter()
            .rev()
            .skip_while(|column| **column == "(LISTEN)")
            .find(|column| column.contains(':'))
            .map(|column| column.to_string());
    }
    None
}

fn process_basic_port_from_local_address(local: &str) -> Option<String> {
    let host_port = local.rsplit_once(':')?.1;
    let port = host_port
        .trim()
        .trim_end_matches(']')
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    (!port.is_empty()).then_some(port)
}

fn process_basic_process_name_from_port_line(line: &str) -> Option<String> {
    let marker = "users:((\"";
    let start = line.find(marker)? + marker.len();
    let rest = line.get(start..)?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string()).filter(|value| !value.trim().is_empty())
}

fn process_basic_local_addr_is_loopback(local: &str) -> bool {
    local.starts_with("127.") || local.starts_with("[::1]") || local.starts_with("::1")
}

fn process_basic_table_rows(body: &str) -> Vec<&str> {
    let mut saw_header = false;
    let mut rows = Vec::new();
    for line in body.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line.starts_with("exit=") {
            continue;
        }
        let columns = line.split_whitespace().collect::<Vec<_>>();
        if columns.iter().any(|column| *column == "PID")
            && columns.iter().any(|column| *column == "COMM")
        {
            saw_header = true;
            continue;
        }
        if saw_header
            && columns.len() >= 2
            && columns
                .first()
                .is_some_and(|column| column.chars().all(|ch| ch.is_ascii_digit()))
        {
            rows.push(line);
        }
    }
    rows
}

fn process_basic_no_match_filter(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("no matching processes for filter:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn system_basic_info_scalar_path_candidate(value: &serde_json::Value) -> Option<String> {
    value
        .get("cwd")
        .or_else(|| value.get("workspace_root"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

fn system_basic_value_looks_like_info(value: &serde_json::Value) -> bool {
    value
        .get("hostname")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .is_some()
        && value
            .get("os")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .is_some()
}

fn system_basic_info_value(skill: &str, body: &str) -> Option<serde_json::Value> {
    if skill != "system_basic" {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    system_basic_value_looks_like_info(&value).then_some(value)
}

fn system_basic_existence_with_path_value(skill: &str, body: &str) -> Option<serde_json::Value> {
    if !matches!(skill, "system_basic" | "fs_basic") {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    matches!(
        value.get("action").and_then(|v| v.as_str()),
        Some("find_path" | "path_batch_facts" | "find_name")
    )
    .then_some(value)
}

fn system_basic_structured_doc_value(skill: &str, body: &str) -> Option<serde_json::Value> {
    if !matches!(skill, "system_basic" | "config_basic") {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    matches!(
        value.get("action").and_then(|v| v.as_str()),
        Some("extract_field" | "extract_fields" | "read_field" | "read_fields" | "structured_keys")
    )
    .then_some(value)
}

fn system_basic_structured_doc_observed_body(skill: &str, body: &str) -> Option<String> {
    let value = system_basic_structured_doc_value(skill, body)?;
    match value.get("action").and_then(|v| v.as_str()) {
        Some("extract_field" | "read_field") => {
            let field_path = value
                .get("resolved_field_path")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .or_else(|| {
                    value
                        .get("field_path")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                })
                .unwrap_or("requested field");
            Some(structured_field_display_line(
                None,
                field_path,
                value.get("value").unwrap_or(&serde_json::Value::Null),
                value.get("value_text").and_then(|v| v.as_str()),
                value
                    .get("exists")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                true,
            ))
        }
        Some("extract_fields" | "read_fields") => extract_fields_direct_answer_candidate(
            None,
            &value,
            Some(crate::OutputResponseShape::Free),
            true,
            true,
        )
        .or_else(|| Some(body.to_string())),
        Some("structured_keys") => Some(body.to_string()),
        _ => None,
    }
}

fn inventory_dir_names(value: &serde_json::Value) -> Option<Vec<String>> {
    let action = value.get("action").and_then(|v| v.as_str())?;
    if action != "inventory_dir" {
        return None;
    }
    value
        .get("names")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
}

fn inventory_dir_names_by_kind(value: &serde_json::Value, kind: &str) -> Vec<String> {
    value
        .get("names_by_kind")
        .and_then(|v| v.get(kind))
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn inventory_dir_grouped_names_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    let files = inventory_dir_names_by_kind(value, "files");
    let dirs = inventory_dir_names_by_kind(value, "dirs");
    let other = inventory_dir_names_by_kind(value, "other");
    if files.is_empty() && dirs.is_empty() && other.is_empty() {
        return None;
    }
    let mut lines = Vec::new();
    let mut push_group = |title: &str, items: Vec<String>| {
        if items.is_empty() {
            return;
        }
        lines.push(format!("{title}:"));
        lines.extend(items.into_iter().map(|name| format!("- {name}")));
    };
    let dirs_title = observed_t(
        state,
        "clawd.msg.directory_group_dirs",
        "目录",
        "Directories",
        prefer_english,
    );
    let files_title = observed_t(
        state,
        "clawd.msg.directory_group_files",
        "文件",
        "Files",
        prefer_english,
    );
    let other_title = observed_t(
        state,
        "clawd.msg.directory_group_other",
        "其它",
        "Other",
        prefer_english,
    );
    push_group(&dirs_title, dirs);
    push_group(&files_title, files);
    push_group(&other_title, other);
    normalized_listing_text(&lines.join("\n"))
}

fn inventory_dir_direct_answer_candidate(
    state: Option<&AppState>,
    route: Option<&crate::RouteResult>,
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    if route.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::DirectoryEntryGroups
    }) {
        return inventory_dir_grouped_names_candidate(state, value, prefer_english);
    }
    if route.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::FileNames
    }) {
        let files = inventory_dir_names_by_kind(value, "files");
        if !files.is_empty() {
            return normalized_listing_text(&files.join("\n"));
        }
    }
    if route.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::DirectoryNames
    }) {
        let dirs = inventory_dir_names_by_kind(value, "dirs");
        if !dirs.is_empty() {
            return normalized_listing_text(&dirs.join("\n"));
        }
    }
    if value
        .get("names_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let names = inventory_dir_names(value)?;
        return normalized_listing_text(&names.join("\n"));
    }
    if let Some(entries) = value.get("entries").and_then(|v| v.as_array()) {
        let lines = entries
            .iter()
            .filter_map(|entry| {
                let name = entry
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())?;
                let size = entry.get("size_bytes").and_then(|v| v.as_u64())?;
                Some(format!("{name} {size}"))
            })
            .collect::<Vec<_>>();
        if !lines.is_empty() {
            return normalized_listing_text(&lines.join("\n"));
        }
    }
    let names = inventory_dir_names(value)?;
    normalized_listing_text(&names.join("\n"))
}

fn tree_summary_display_name(entry: &serde_json::Value) -> Option<String> {
    entry
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            entry
                .get("path")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .and_then(|path| Path::new(path).file_name().and_then(|name| name.to_str()))
                .map(ToOwned::to_owned)
        })
}

fn tree_summary_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    if value.get("action").and_then(|v| v.as_str()) != Some("tree_summary") {
        return None;
    }
    let tree = value.get("tree")?;
    let children = tree.get("children").and_then(|v| v.as_array())?;
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut other = Vec::new();
    for child in children {
        let mut name = tree_summary_display_name(child)?;
        let kind = child
            .get("kind")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("");
        match kind {
            "dir" => {
                if !name.ends_with('/') {
                    name.push('/');
                }
                dirs.push(name);
            }
            "file" => files.push(name),
            _ => other.push(name),
        }
    }
    if dirs.is_empty() && files.is_empty() && other.is_empty() {
        return Some(observed_t(
            state,
            "clawd.msg.tree_summary_empty",
            "顶层为空",
            "Top level is empty",
            prefer_english,
        ));
    }
    let mut parts = Vec::new();
    if !dirs.is_empty() {
        parts.push(format!(
            "{} {}",
            observed_t(
                state,
                "clawd.msg.tree_summary_dirs",
                "目录",
                "directories",
                prefer_english,
            ),
            dirs.join(", ")
        ));
    }
    if !files.is_empty() {
        parts.push(format!(
            "{} {}",
            observed_t(
                state,
                "clawd.msg.tree_summary_files",
                "文件",
                "files",
                prefer_english,
            ),
            files.join(", ")
        ));
    }
    if !other.is_empty() {
        parts.push(format!(
            "{} {}",
            observed_t(
                state,
                "clawd.msg.tree_summary_other",
                "其它",
                "other",
                prefer_english,
            ),
            other.join(", ")
        ));
    }
    let prefix = observed_t(
        state,
        "clawd.msg.tree_summary_top_level",
        "顶层结构：",
        "Top level: ",
        prefer_english,
    );
    let separator = if prefer_english { "; " } else { "；" };
    let mut answer = format!("{prefix}{}", parts.join(separator));
    let truncated_nodes = value
        .get("truncated_nodes")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let root_omitted = tree
        .get("omitted_children")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if truncated_nodes > 0 || root_omitted > 0 {
        let count = truncated_nodes.max(root_omitted).to_string();
        answer.push_str(&observed_t_with_vars(
            state,
            "clawd.msg.tree_summary_partial",
            "（另有 {count} 项未显示）",
            " ({count} more not shown)",
            prefer_english,
            &[("count", &count)],
        ));
    }
    Some(answer)
}

fn dir_compare_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    if value.get("action").and_then(|v| v.as_str()) != Some("dir_compare") {
        return None;
    }
    let counts = value.get("counts").and_then(|v| v.as_object())?;
    let left_only = counts
        .get("left_only")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let right_only = counts
        .get("right_only")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let kind_mismatches = counts
        .get("kind_mismatches")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if left_only == 0 && right_only == 0 && kind_mismatches == 0 {
        return Some(observed_t(
            state,
            "clawd.msg.dir_compare_no_diff",
            "未发现差异。",
            "No differences found.",
            prefer_english,
        ));
    }
    Some(if prefer_english {
        format!(
            "Differences found: left-only {left_only}, right-only {right_only}, kind mismatches {kind_mismatches}."
        )
    } else {
        format!("发现差异：左侧独有 {left_only} 项，右侧独有 {right_only} 项，类型不一致 {kind_mismatches} 项。")
    })
}

fn first_meaningful_excerpt_sentence(text: &str) -> Option<String> {
    let mut short_fallback = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with('<')
            || line.starts_with("```")
            || line.starts_with('|')
            || line.starts_with('-')
        {
            continue;
        }
        if short_fallback.is_none() {
            short_fallback = Some(line.to_string());
        }
        if line.chars().count() < 48 {
            continue;
        }
        let sentence = line
            .split_inclusive(['.', '。', '！', '!', '？', '?'])
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(line);
        return Some(sentence.to_string());
    }
    short_fallback
}

fn content_excerpt_summary_direct_answer_candidate(
    route: Option<&crate::RouteResult>,
    body: &str,
) -> Option<String> {
    if !route.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::ContentExcerptSummary
    }) {
        return None;
    }
    let text = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("text")
                .or_else(|| value.get("excerpt"))
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| body.to_string());
    first_meaningful_excerpt_sentence(&text)
}

fn doc_parse_text_from_body(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("text")
                .or_else(|| value.get("excerpt"))
                .or_else(|| value.get("content"))
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
        })
}

fn contract_hint_bool_from_request(request_text: Option<&str>, key: &str) -> Option<bool> {
    let value = crate::intent_router::contract_test_hint_value(request_text?, key)?;
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn content_presence_query_from_request(request_text: Option<&str>) -> Option<String> {
    crate::intent_router::contract_test_hint_value(request_text?, "selector_query")
        .map(|value| value.replace(['\r', '\n'], " "))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value.len() <= 160)
}

fn content_presence_case_insensitive_from_request(request_text: Option<&str>) -> bool {
    contract_hint_bool_from_request(request_text, "selector_case_insensitive")
        .or_else(|| contract_hint_bool_from_request(request_text, "selector_ignore_case"))
        .unwrap_or(true)
}

fn find_content_presence_match_line(
    text: &str,
    query: &str,
    case_insensitive: bool,
) -> Option<(u64, String)> {
    let needle = if case_insensitive {
        query.to_lowercase()
    } else {
        query.to_string()
    };
    for (idx, line) in text.lines().enumerate() {
        let haystack = if case_insensitive {
            line.to_lowercase()
        } else {
            line.to_string()
        };
        if haystack.contains(&needle) {
            return Some((idx as u64 + 1, line.trim().to_string()));
        }
    }
    None
}

fn doc_parse_content_presence_direct_answer_candidate(
    state: Option<&AppState>,
    route: &crate::RouteResult,
    body: &str,
    request_text: Option<&str>,
    path_hint: Option<&str>,
    prefer_english: bool,
) -> Option<String> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ContentPresenceCheck {
        return None;
    }
    let query = content_presence_query_from_request(request_text)?;
    let text = doc_parse_text_from_body(body)?;
    let case_insensitive = content_presence_case_insensitive_from_request(request_text);
    let path = path_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| route.output_contract.locator_hint.trim());
    let _ = state;
    if let Some((line, matched_text)) =
        find_content_presence_match_line(&text, &query, case_insensitive)
    {
        let location = if path.is_empty() {
            line.to_string()
        } else {
            format!("{path}:{line}")
        };
        return Some(if prefer_english {
            format!("Contains `{query}`; evidence: {location} `{matched_text}`.")
        } else {
            format!("包含 `{query}`，依据：{location} `{matched_text}`。")
        });
    }
    Some(if path.is_empty() {
        if prefer_english {
            format!("Does not contain `{query}`.")
        } else {
            format!("不包含 `{query}`。")
        }
    } else if prefer_english {
        format!("Does not contain `{query}`. Path: {path}")
    } else {
        format!("不包含 `{query}`。路径：{path}")
    })
}

fn direct_free_text_conflicts_with_request_language(
    candidate: &str,
    request_language_hint: &str,
) -> bool {
    crate::language_policy::text_language_conflicts_with_hint(candidate, request_language_hint)
}

fn read_range_candidate_looks_structured_artifact(candidate: &str) -> bool {
    let mut total = 0usize;
    let mut structural = 0usize;
    for line in candidate
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        total += 1;
        let starts_structural =
            line.starts_with(['{', '}', '[', ']', '<']) || line.ends_with(['{', '}', '[', ']']);
        let assignment_like = line
            .split_once('=')
            .is_some_and(|(left, right)| !left.trim().is_empty() && !right.trim().is_empty());
        let json_pair_like = line.trim_start().starts_with('"')
            && line
                .split_once(':')
                .is_some_and(|(left, right)| !left.trim().is_empty() && !right.trim().is_empty());
        if starts_structural || assignment_like || json_pair_like {
            structural += 1;
        }
    }
    total > 0 && structural.saturating_mul(2) >= total
}

fn read_range_direct_candidate_conflicts_with_request_language(
    candidate: &str,
    request_language_hint: &str,
) -> bool {
    !read_range_candidate_looks_structured_artifact(candidate)
        && direct_free_text_conflicts_with_request_language(candidate, request_language_hint)
}

fn inventory_dir_scalar_path_candidate(
    value: &serde_json::Value,
    prefer_full_path: bool,
) -> Option<String> {
    let names = inventory_dir_names(value)?;
    if !prefer_full_path {
        return Some(names.join("\n"));
    }
    let root = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let paths = names
        .into_iter()
        .map(|name| {
            let name_path = Path::new(&name);
            if name_path.is_absolute() {
                canonical_existing_path(name_path)
            } else if let Some(root) = root {
                let candidate = Path::new(root).join(&name);
                if candidate.exists() {
                    canonical_existing_path(&candidate)
                } else {
                    candidate.display().to_string()
                }
            } else {
                name
            }
        })
        .collect::<Vec<_>>();
    (!paths.is_empty()).then(|| paths.join("\n"))
}

fn compact_inventory_dir_kind_lines(entries: &[serde_json::Value]) -> Option<Vec<String>> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut others = Vec::new();

    for entry in entries {
        let entry = entry.as_object()?;
        let name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())?;
        let mut label = name.to_string();
        if let Some(size_bytes) = entry.get("size_bytes").and_then(|v| v.as_u64()) {
            label.push_str(&format!(":size_bytes={size_bytes}"));
        }
        match entry
            .get("kind")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("other")
        {
            "dir" => dirs.push(label),
            "file" => files.push(label),
            _ => others.push(label),
        }
    }

    let mut lines = Vec::new();
    if !dirs.is_empty() {
        lines.push(format!("dir_entries={}", dirs.join(",")));
    }
    if !files.is_empty() {
        lines.push(format!("file_entries={}", files.join(",")));
    }
    if !others.is_empty() {
        lines.push(format!("other_entries={}", others.join(",")));
    }
    (!lines.is_empty()).then_some(lines)
}

fn inventory_dir_observed_candidate(value: &serde_json::Value) -> Option<String> {
    let path = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    let mut header = format!("inventory_dir path={path}");
    if let Some(sort_by) = value
        .get("sort_by")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        header.push_str(&format!(" sort_by={sort_by}"));
    }
    if let Some(counts) = value.get("counts").and_then(|v| v.as_object()) {
        for key in ["total", "files", "dirs", "hidden"] {
            if let Some(count) = counts.get(key).and_then(value_scalar_text) {
                header.push_str(&format!(" {key}={count}"));
            }
        }
    }
    if let Some(entries) = value.get("entries").and_then(|v| v.as_array()) {
        if entries.len() > 16 {
            if let Some(lines) = compact_inventory_dir_kind_lines(entries) {
                return Some(format!("{header}\n{}", lines.join("\n")));
            }
        }
        let lines = entries
            .iter()
            .filter_map(|entry| {
                let entry = entry.as_object()?;
                let name = entry
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())?;
                let kind = entry
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .unwrap_or("-");
                let size = entry
                    .get("size_bytes")
                    .and_then(|v| v.as_u64())
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let modified = entry
                    .get("modified_ts")
                    .and_then(|v| v.as_i64())
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".to_string());
                Some(format!(
                    "entry name={name} kind={kind} size_bytes={size} modified_ts={modified}"
                ))
            })
            .collect::<Vec<_>>();
        if !lines.is_empty() {
            return Some(format!("{header}\n{}", lines.join("\n")));
        }
    }
    let names = inventory_dir_names(value)?;
    Some(format!(
        "{header}\n{}",
        names
            .into_iter()
            .map(|name| format!("entry name={name}"))
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

fn count_inventory_count_value(value: &serde_json::Value) -> Option<(String, &'static str)> {
    let counts = value.get("counts")?;
    let kind_filter = value
        .get("kind_filter")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_ascii_lowercase());
    let count_key = if value
        .get("files_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || matches!(
            kind_filter.as_deref(),
            Some("file" | "files" | "regular_file")
        ) {
        "files"
    } else if value
        .get("dirs_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || matches!(
            kind_filter.as_deref(),
            Some("dir" | "dirs" | "directory" | "directories")
        )
    {
        "dirs"
    } else {
        "total"
    };
    counts
        .get(count_key)
        .or_else(|| counts.get("total"))
        .and_then(value_scalar_text)
        .map(|count| (count, count_key))
}

fn count_inventory_breakdown_value(
    value: &serde_json::Value,
) -> Option<(String, String, Option<String>)> {
    let counts = value.get("counts")?;
    let files = counts.get("files").and_then(value_scalar_text)?;
    let dirs = counts.get("dirs").and_then(value_scalar_text)?;
    let total = counts.get("total").and_then(value_scalar_text);
    Some((files, dirs, total))
}

fn count_inventory_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    response_shape: Option<crate::OutputResponseShape>,
    prefer_english: bool,
) -> Option<String> {
    let (count, count_key) = count_inventory_count_value(value)?;
    let has_component_breakdown = count_inventory_breakdown_value(value).is_some();
    if matches!(response_shape, Some(crate::OutputResponseShape::Scalar)) {
        return Some(count);
    }
    if response_shape.is_none() && count_key == "total" && has_component_breakdown {
        return None;
    }
    let noun = match count_key {
        "files" => observed_t(
            state,
            "clawd.msg.count_inventory_noun_files",
            "普通文件",
            "regular files",
            prefer_english,
        ),
        "dirs" => observed_t(
            state,
            "clawd.msg.count_inventory_noun_dirs",
            "目录",
            "directories",
            prefer_english,
        ),
        _ => observed_t(
            state,
            "clawd.msg.count_inventory_noun_items",
            "项目",
            "items",
            prefer_english,
        ),
    };
    Some(observed_t_with_vars(
        state,
        "clawd.msg.count_inventory_direct_answer",
        "{count}，当前范围内共有 {count} 个{noun}。",
        "{count}: there are {count} {noun} in the requested scope.",
        prefer_english,
        &[("count", &count), ("noun", &noun)],
    ))
}

fn plan_requests_count_inventory_file_dir_breakdown(loop_state: &LoopState) -> bool {
    loop_state
        .round_traces
        .iter()
        .rev()
        .filter_map(|trace| trace.plan_result.as_ref())
        .any(|plan| {
            plan.steps.iter().any(|step| {
                step.action_type == "call_skill"
                    && step.skill == "system_basic"
                    && step.args.get("action").and_then(|v| v.as_str()) == Some("count_inventory")
                    && step
                        .args
                        .get("count_files")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    && step
                        .args
                        .get("count_dirs")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
            })
        })
}

fn latest_count_inventory_file_dir_breakdown(
    loop_state: &LoopState,
) -> Option<(String, String, Option<String>)> {
    let idx = latest_successful_step_index(loop_state, |step| {
        step.skill == "system_basic"
            && step
                .output
                .as_deref()
                .and_then(|body| serde_json::from_str::<serde_json::Value>(body).ok())
                .is_some_and(|value| {
                    value.get("action").and_then(|v| v.as_str()) == Some("count_inventory")
                })
    })?;
    let body = loop_state.executed_step_results[idx].output.as_deref()?;
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if value.get("action").and_then(|v| v.as_str()) != Some("count_inventory") {
        return None;
    }
    count_inventory_breakdown_value(&value)
}

fn count_inventory_planned_file_dir_breakdown_answer(
    state: Option<&AppState>,
    loop_state: &LoopState,
    prefer_english: bool,
) -> Option<String> {
    if !plan_requests_count_inventory_file_dir_breakdown(loop_state) {
        return None;
    }
    let (files, dirs, _total) = latest_count_inventory_file_dir_breakdown(loop_state)?;
    Some(observed_t_with_vars(
        state,
        "clawd.msg.count_inventory_file_dir_breakdown",
        "文件：{files} 个\n文件夹：{dirs} 个",
        "Files: {files}\nDirectories: {dirs}",
        prefer_english,
        &[("files", &files), ("dirs", &dirs)],
    ))
}

fn is_ignorable_shell_warning(line: &str) -> bool {
    let normalized = line.trim();
    normalized.starts_with("bash: warning: setlocale:")
        || normalized.starts_with("zsh: warning: setlocale:")
}

fn run_cmd_directory_entry_list_candidate(
    body: &str,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let auto_locator_path = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(Path::new)?;
    if !auto_locator_path.is_dir() {
        return None;
    }
    let lines = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| *line != "exit=0")
        .filter(|line| !is_ignorable_shell_warning(line))
        .collect::<Vec<_>>();
    if lines.is_empty()
        || lines.len() > 200
        || lines
            .iter()
            .any(|line| looks_like_shell_long_listing_line(line))
    {
        return None;
    }
    let all_direct_entries = lines.iter().all(|line| {
        let candidate = line.trim_end_matches('/');
        !candidate.is_empty()
            && !candidate.starts_with('/')
            && !candidate.starts_with('~')
            && !candidate.contains('/')
            && !candidate.contains('\\')
            && serde_json::from_str::<serde_json::Value>(candidate).is_err()
    });
    all_direct_entries
        .then(|| normalized_listing_text(&lines.join("\n")))
        .flatten()
}

fn run_cmd_semantic_listing_text_candidate(
    route: &crate::RouteResult,
    body: &str,
) -> Option<String> {
    if !matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::DirectoryNames
            | crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::DirectoryEntryGroups
            | crate::OutputSemanticKind::FilePaths
    ) {
        return None;
    }
    if matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Scalar
    ) {
        return None;
    }
    let lines = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| *line != "exit=0")
        .filter(|line| !is_ignorable_shell_warning(line))
        .collect::<Vec<_>>();
    if lines.is_empty()
        || lines.len() > 200
        || lines
            .iter()
            .any(|line| looks_like_shell_long_listing_line(line))
    {
        return None;
    }
    if lines
        .iter()
        .any(|line| serde_json::from_str::<serde_json::Value>(line).is_ok())
    {
        return None;
    }
    normalized_listing_text(&lines.join("\n"))
}

fn run_cmd_listing_text_candidate(body: &str, auto_locator_path: Option<&str>) -> Option<String> {
    run_cmd_shell_listing_entry_names(body)
        .map(|names| names.join("\n"))
        .or_else(|| run_cmd_directory_entry_list_candidate(body, auto_locator_path))
}

fn run_cmd_shell_listing_entry_names(body: &str) -> Option<Vec<String>> {
    let mut names = Vec::new();
    for line in body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| *line != "exit=0")
        .filter(|line| !is_ignorable_shell_warning(line))
    {
        if line.starts_with("total ") {
            continue;
        }
        let first = line.chars().next()?;
        if !matches!(first, '-' | 'd' | 'l' | 'b' | 'c' | 'p' | 's') {
            return None;
        }
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 9 {
            return None;
        }
        let raw_name = fields[8..].join(" ");
        let name = raw_name
            .split_once(" -> ")
            .map(|(name, _)| name)
            .unwrap_or(raw_name.as_str())
            .trim();
        if name.is_empty() {
            return None;
        }
        names.push(name.to_string());
    }
    if names.is_empty() {
        return None;
    }
    Some(names)
}

fn run_cmd_presence_with_path_candidate(
    state: Option<&AppState>,
    body: &str,
    locator_hint: Option<&str>,
    auto_locator_path: Option<&str>,
    english_answer: bool,
) -> Option<String> {
    let scalar = normalized_scalar_candidate(body)?;
    let normalized = scalar.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "exists" | "yes" | "true" => Some(candidate_exists_with_path_text(
            state,
            existence_with_path_target_hint(locator_hint, auto_locator_path).as_deref(),
            english_answer,
        )),
        "not_found" | "not found" | "no" | "false" => {
            Some(candidate_not_found_text(state, english_answer))
        }
        _ => None,
    }
}

fn existence_with_path_target_hint(
    locator_hint: Option<&str>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let locator_hint = locator_hint
        .map(str::trim)
        .filter(|hint| !hint.is_empty())?;
    let locator_path = Path::new(locator_hint);
    if locator_path.is_absolute() && locator_path.exists() {
        return Some(canonical_existing_path(locator_path));
    }
    resolve_listing_entry_full_path(locator_hint, auto_locator_path).or_else(|| {
        auto_locator_path
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .and_then(|root| {
                let root = Path::new(root);
                if !root.is_dir() {
                    return None;
                }
                let candidate = root.join(locator_hint);
                candidate
                    .exists()
                    .then(|| canonical_existing_path(&candidate))
            })
    })
}

fn candidate_exists_with_path_text(
    state: Option<&AppState>,
    path: Option<&str>,
    prefer_english: bool,
) -> String {
    match path.map(str::trim).filter(|path| !path.is_empty()) {
        Some(path) => observed_t_with_vars(
            state,
            "clawd.msg.exists_with_path",
            "有，路径：{path}",
            "yes, path: {path}",
            prefer_english,
            &[("path", path)],
        ),
        None => observed_t(state, "clawd.msg.exists_yes", "有", "yes", prefer_english),
    }
}

fn candidate_exists_scalar_text(state: Option<&AppState>, prefer_english: bool) -> String {
    observed_t(state, "clawd.msg.exists_yes", "有", "yes", prefer_english)
}

fn candidate_exists_with_path_and_size_text(
    state: Option<&AppState>,
    path: Option<&str>,
    size_bytes: u64,
    prefer_english: bool,
) -> String {
    match path.map(str::trim).filter(|path| !path.is_empty()) {
        Some(path) => observed_t_with_vars(
            state,
            "clawd.msg.exists_with_path_and_size",
            "有，路径：{path}，大小：{size_bytes} 字节",
            "yes, path: {path}, size: {size_bytes} bytes",
            prefer_english,
            &[("path", path), ("size_bytes", &size_bytes.to_string())],
        ),
        None => observed_t_with_vars(
            state,
            "clawd.msg.exists_with_size",
            "有，大小：{size_bytes} 字节",
            "yes, size: {size_bytes} bytes",
            prefer_english,
            &[("size_bytes", &size_bytes.to_string())],
        ),
    }
}

fn candidate_not_found_text(state: Option<&AppState>, prefer_english: bool) -> String {
    observed_t(state, "clawd.msg.exists_no", "没有", "no", prefer_english)
}

fn candidate_not_found_with_path_text(
    state: Option<&AppState>,
    path: Option<&str>,
    prefer_english: bool,
) -> String {
    match path.map(str::trim).filter(|path| !path.is_empty()) {
        Some(path) => observed_t_with_vars(
            state,
            "clawd.msg.exists_no_path_not_found",
            "没有，路径不存在：{path}",
            "no, path not found: {path}",
            prefer_english,
            &[("path", path)],
        ),
        None => candidate_not_found_text(state, prefer_english),
    }
}

fn normalize_system_basic_match_path(
    resolved_root: Option<&str>,
    candidate_path: Option<&str>,
) -> Option<String> {
    let candidate_path = candidate_path
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    let candidate = Path::new(candidate_path);
    if candidate.is_absolute() {
        return Some(candidate_path.to_string());
    }
    if candidate.exists() {
        return Some(canonical_existing_path(candidate));
    }
    let root = resolved_root
        .map(str::trim)
        .filter(|root| !root.is_empty())
        .map(Path::new)?;
    let rooted = root.join(candidate);
    if rooted.exists() {
        Some(canonical_existing_path(&rooted))
    } else {
        Some(rooted.to_string_lossy().to_string())
    }
}

fn path_batch_fact_preferred_path<'a>(
    entry: &'a serde_json::Map<String, serde_json::Value>,
) -> Option<&'a str> {
    let fact = entry.get("fact").and_then(|v| v.as_object());
    fact.and_then(|item| item.get("resolved_path"))
        .or_else(|| fact.and_then(|item| item.get("path")))
        .or_else(|| entry.get("resolved_path"))
        .or_else(|| entry.get("path"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())
}

fn system_basic_path_batch_scalar_path_candidate(value: &serde_json::Value) -> Option<String> {
    if value.get("action").and_then(|v| v.as_str()) != Some("path_batch_facts") {
        return None;
    }
    let facts = value.get("facts")?.as_array()?;
    if facts.len() != 1 {
        return None;
    }
    let entry = facts.first()?.as_object()?;
    let exists = entry
        .get("exists")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    exists
        .then(|| path_batch_fact_preferred_path(entry).map(ToString::to_string))
        .flatten()
}

fn route_requires_single_file_delivery(route: &crate::RouteResult) -> bool {
    matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    ) || matches!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
    ) || (route.wants_file_delivery
        && !matches!(
            route.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::DirectoryBatchFiles
        ))
}

fn path_batch_file_delivery_token_candidate(
    route: Option<&crate::RouteResult>,
    value: &serde_json::Value,
) -> Option<String> {
    let route = route?;
    if !route_requires_single_file_delivery(route)
        || value.get("action").and_then(|v| v.as_str()) != Some("path_batch_facts")
    {
        return None;
    }
    let facts = value.get("facts")?.as_array()?;
    if facts.len() != 1 {
        return None;
    }
    let entry = facts.first()?.as_object()?;
    if !entry
        .get("exists")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return None;
    }
    let path = path_batch_fact_preferred_path(entry)?;
    let fact_kind = entry
        .get("fact")
        .and_then(|value| value.get("kind"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if fact_kind.is_some_and(|kind| !kind.eq_ignore_ascii_case("file")) {
        return None;
    }
    if fact_kind.is_none() && !Path::new(path).is_file() {
        return None;
    }
    Some(format!("FILE:{path}"))
}

fn path_batch_facts_requests_size(value: &serde_json::Value) -> bool {
    value
        .get("fields")
        .and_then(|fields| fields.as_array())
        .map(|fields| {
            fields.iter().any(|field| {
                field.as_str().is_some_and(|field| {
                    let field = field.trim().to_ascii_lowercase();
                    field == "size" || field == "size_bytes" || field == "file_size"
                })
            })
        })
        .unwrap_or(false)
}

fn path_batch_fact_size_bytes(entry: &serde_json::Map<String, serde_json::Value>) -> Option<u64> {
    entry
        .get("fact")
        .and_then(|v| v.as_object())
        .and_then(|fact| fact.get("size_bytes"))
        .and_then(|v| v.as_u64())
        .or_else(|| entry.get("size_bytes").and_then(|v| v.as_u64()))
}

fn display_path_kind(kind: &str, prefer_english: bool) -> String {
    let normalized = kind.trim().to_ascii_lowercase();
    match (normalized.as_str(), prefer_english) {
        ("dir" | "directory", true) => "directory".to_string(),
        ("dir" | "directory", false) => "目录".to_string(),
        ("file", true) => "file".to_string(),
        ("file", false) => "文件".to_string(),
        ("symlink", true) => "symlink".to_string(),
        ("symlink", false) => "符号链接".to_string(),
        ("other", true) => "other".to_string(),
        ("other", false) => "其他".to_string(),
        _ => kind.trim().to_string(),
    }
}

fn route_prefers_path_kind_fact_answer(route: &crate::RouteResult) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::Strict
        && !route.output_contract.delivery_required
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::ExistenceWithPath
}

fn path_batch_fact_path_kind_candidate(
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    if value.get("action").and_then(|v| v.as_str()) != Some("path_batch_facts")
        || path_batch_facts_requests_size(value)
    {
        return None;
    }
    let facts = value.get("facts")?.as_array()?;
    if facts.len() != 1 {
        return None;
    }
    let entry = facts.first()?.as_object()?;
    if !entry
        .get("exists")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return None;
    }
    let fact = entry.get("fact").and_then(|v| v.as_object())?;
    let path = path_batch_fact_preferred_path(entry)?;
    let kind = fact
        .get("kind")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(format!(
        "{path} | {}",
        display_path_kind(kind, prefer_english)
    ))
}

fn system_basic_existence_with_path_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    locator_hint: Option<&str>,
    auto_locator_path: Option<&str>,
    prefer_english: bool,
) -> Option<String> {
    let action = value.get("action").and_then(|v| v.as_str())?;
    match action {
        "find_name" => {
            let (results, count, pattern) = fs_search_find_name_results(value)?;
            if count == 0 || results.is_empty() {
                return Some(candidate_not_found_text(state, prefer_english));
            }
            let preferred = if results.len() == 1 {
                Some(results[0].clone())
            } else {
                let pattern = normalized_find_name_pattern(pattern.as_deref())
                    .or_else(|| normalized_find_name_pattern(locator_hint))?;
                preferred_fs_search_exact_match(&results, &pattern)
            }?;
            let root = value
                .get("root")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|root| !root.is_empty());
            let resolved_path = Path::new(&preferred)
                .is_absolute()
                .then(|| canonical_existing_path(Path::new(&preferred)))
                .or_else(|| {
                    root.and_then(|root| {
                        let candidate = Path::new(root).join(&preferred);
                        candidate
                            .exists()
                            .then(|| canonical_existing_path(&candidate))
                    })
                })
                .or_else(|| resolve_listing_entry_full_path(&preferred, auto_locator_path))
                .unwrap_or(preferred);
            Some(candidate_exists_with_path_text(
                state,
                Some(resolved_path.as_str()),
                prefer_english,
            ))
        }
        "find_path" => {
            let count = value
                .get("count")
                .and_then(|v| v.as_u64())
                .unwrap_or_default() as usize;
            let matches = value.get("matches").and_then(|v| v.as_array())?;
            if count == 0 || matches.is_empty() {
                return Some(candidate_not_found_text(state, prefer_english));
            }
            if matches.len() != 1 {
                return None;
            }
            let matched = matches.first()?.as_object()?;
            let resolved_root = value.get("resolved_root").and_then(|v| v.as_str());
            let path = normalize_system_basic_match_path(
                resolved_root,
                matched
                    .get("resolved_path")
                    .and_then(|v| v.as_str())
                    .or_else(|| matched.get("path").and_then(|v| v.as_str())),
            );
            Some(candidate_exists_with_path_text(
                state,
                path.as_deref(),
                prefer_english,
            ))
        }
        "path_batch_facts" => {
            let facts = value.get("facts").and_then(|v| v.as_array())?;
            if facts.is_empty() {
                return Some(candidate_not_found_text(state, prefer_english));
            }
            if facts.len() != 1 {
                return multi_path_batch_facts_candidate(state, facts, prefer_english);
            }
            let entry = facts.first()?.as_object()?;
            let exists = entry
                .get("exists")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !exists {
                return Some(candidate_not_found_with_path_text(
                    state,
                    path_batch_fact_preferred_path(entry),
                    prefer_english,
                ));
            }
            let path = path_batch_fact_preferred_path(entry);
            if path_batch_facts_requests_size(value) {
                if let Some(size_bytes) = path_batch_fact_size_bytes(entry) {
                    return Some(candidate_exists_with_path_and_size_text(
                        state,
                        path,
                        size_bytes,
                        prefer_english,
                    ));
                }
            }
            Some(candidate_exists_with_path_text(state, path, prefer_english))
        }
        _ => None,
    }
}

fn multi_path_batch_facts_candidate(
    _state: Option<&AppState>,
    facts: &[serde_json::Value],
    prefer_english: bool,
) -> Option<String> {
    let lines = facts
        .iter()
        .filter_map(|entry| {
            let entry = entry.as_object()?;
            let path = path_batch_fact_preferred_path(entry).unwrap_or("-");
            let exists = entry
                .get("exists")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !exists {
                return Some(if prefer_english {
                    format!("{path}: not found")
                } else {
                    format!("{path}: 不存在")
                });
            }
            let kind = entry
                .get("fact")
                .and_then(|v| v.as_object())
                .and_then(|fact| fact.get("kind"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("unknown");
            Some(if prefer_english {
                format!("{path}: exists, type {}", display_path_kind(kind, true))
            } else {
                format!("{path}: 存在，类型：{}", display_path_kind(kind, false))
            })
        })
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn system_basic_scalar_existence_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    match value.get("action").and_then(|v| v.as_str())? {
        "path_batch_facts" => {
            let facts = value.get("facts").and_then(|v| v.as_array())?;
            if facts.len() != 1 {
                return None;
            }
            let exists = facts
                .first()?
                .as_object()?
                .get("exists")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Some(if exists {
                candidate_exists_scalar_text(state, prefer_english)
            } else {
                candidate_not_found_text(state, prefer_english)
            })
        }
        _ => None,
    }
}

fn fs_search_find_name_results(
    value: &serde_json::Value,
) -> Option<(Vec<String>, usize, Option<String>)> {
    let action = value.get("action").and_then(|v| v.as_str())?;
    if !action.eq_ignore_ascii_case("find_name") {
        return None;
    }
    let results = value
        .get("results")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let count = value
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(results.len() as u64) as usize;
    let pattern = value
        .get("pattern")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string);
    Some((results, count, pattern))
}

fn fs_search_find_ext_results(value: &serde_json::Value) -> Option<(Vec<String>, usize, String)> {
    let action = value.get("action").and_then(|v| v.as_str())?;
    if !action.eq_ignore_ascii_case("find_ext") {
        return None;
    }
    let results = value
        .get("results")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let count = value
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(results.len() as u64) as usize;
    let ext = value
        .get("ext")
        .or_else(|| value.get("extension"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?
        .to_string();
    Some((results, count, ext))
}

fn fs_search_grep_text_results(
    value: &serde_json::Value,
) -> Option<(Vec<(String, u64, String)>, usize, String)> {
    let action = value.get("action").and_then(|v| v.as_str())?;
    if !action.eq_ignore_ascii_case("grep_text") {
        return None;
    }
    let query = value
        .get("query")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?
        .to_string();
    let matches = value
        .get("matches")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let obj = item.as_object()?;
                    let path = obj
                        .get("path")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|v| !v.is_empty())?
                        .to_string();
                    let line = obj.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
                    let text = obj
                        .get("text")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|v| !v.is_empty())?
                        .to_string();
                    Some((path, line, text))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let match_count = value
        .get("match_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(matches.len() as u64) as usize;
    Some((matches, match_count, query))
}

fn fs_search_grep_text_name_results(value: &serde_json::Value) -> Option<(Vec<String>, usize)> {
    let action = value.get("action").and_then(|v| v.as_str())?;
    if !action.eq_ignore_ascii_case("grep_text") {
        return None;
    }
    let results = value
        .get("name_results")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let count = value
        .get("name_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(results.len() as u64) as usize;
    Some((results, count))
}

fn fs_search_grep_text_observed_candidate(value: &serde_json::Value) -> Option<String> {
    let (matches, match_count, query) = fs_search_grep_text_results(value)?;
    let file_count = value
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or_default();
    let patterns = value
        .get("patterns")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let (name_results, name_count) =
        fs_search_grep_text_name_results(value).unwrap_or((Vec::new(), 0));
    let mut lines = vec![format!(
        "grep_text query={query} file_count={file_count} match_count={match_count}"
    )];
    if !patterns.is_empty() {
        lines.push(format!("file_patterns={}", patterns.join(", ")));
    }
    if name_count > 0 && !name_results.is_empty() {
        lines.push(format!("name_count={name_count}"));
        lines.extend(
            name_results
                .into_iter()
                .take(16)
                .map(|path| format!("name_match path={path}")),
        );
    }
    if matches.is_empty() {
        lines.push("matches: none".to_string());
    } else {
        lines.extend(
            matches
                .into_iter()
                .take(16)
                .map(|(path, line, text)| format!("match path={path} line={line} text={text}")),
        );
    }
    Some(lines.join("\n"))
}

fn path_matches_find_name_pattern(path: &str, pattern: &str) -> bool {
    let path = Path::new(path);
    let Some(file_name) = path.file_name().and_then(|v| v.to_str()) else {
        return false;
    };
    if file_name.eq_ignore_ascii_case(pattern) {
        return true;
    }
    if pattern.contains('.') {
        return false;
    }
    path.file_stem()
        .and_then(|v| v.to_str())
        .map(|stem| stem.eq_ignore_ascii_case(pattern))
        .unwrap_or(false)
}

fn is_direct_child_relative_match(path: &str) -> bool {
    let path = Path::new(path);
    match path.parent().and_then(|parent| parent.to_str()) {
        None => true,
        Some("") | Some(".") => true,
        Some(_) => false,
    }
}

fn preferred_fs_search_exact_match(results: &[String], pattern: &str) -> Option<String> {
    let mut exact_matches = results
        .iter()
        .filter(|path| path_matches_find_name_pattern(path, pattern))
        .cloned()
        .collect::<Vec<_>>();
    exact_matches.sort();
    exact_matches.dedup();
    let mut direct_child_matches = exact_matches
        .iter()
        .filter(|path| is_direct_child_relative_match(path))
        .cloned()
        .collect::<Vec<_>>();
    direct_child_matches.sort();
    direct_child_matches.dedup();
    if direct_child_matches.len() == 1 {
        return direct_child_matches.into_iter().next();
    }
    (exact_matches.len() == 1).then(|| exact_matches.into_iter().next().unwrap_or_default())
}

fn rank_fs_search_candidates(results: &[String], pattern: &str) -> Vec<String> {
    let pattern_norm = pattern.trim().to_lowercase();
    let mut ranked = results
        .iter()
        .cloned()
        .map(|path| {
            let path_buf = Path::new(&path);
            let file_name = path_buf
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or_default()
                .to_string();
            let file_name_norm = file_name.to_lowercase();
            let stem_norm = path_buf
                .file_stem()
                .and_then(|v| v.to_str())
                .unwrap_or_default()
                .to_lowercase();
            let score = if stem_norm == pattern_norm {
                500
            } else if stem_norm.starts_with(&pattern_norm) {
                400
            } else if stem_norm.contains(&pattern_norm) {
                300
            } else if file_name_norm.starts_with(&pattern_norm) {
                200
            } else if file_name_norm.contains(&pattern_norm) {
                100
            } else {
                0
            };
            (score, file_name.len(), path)
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    ranked.dedup_by(|a, b| a.2 == b.2);
    ranked
        .into_iter()
        .take(3)
        .map(|(_, _, path)| path)
        .collect()
}

fn normalized_find_name_pattern(pattern: Option<&str>) -> Option<String> {
    let pattern = pattern?.trim();
    if pattern.is_empty() {
        return None;
    }
    let path = Path::new(pattern);
    path.file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| Some(pattern.to_string()))
}

fn fs_search_scalar_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    locator_hint: Option<&str>,
    auto_locator_path: Option<&str>,
    prefer_full_path: bool,
    prefer_english: bool,
) -> Option<String> {
    let (mut results, count, pattern) = fs_search_find_name_results(value)?;
    if count == 0 || results.is_empty() {
        return Some(observed_t(
            state,
            "clawd.msg.fs_search_no_match",
            "没有找到匹配项",
            "No matches found.",
            prefer_english,
        ));
    }
    if results.len() > 1 {
        if let Some(locator_ext) = locator_hint.and_then(path_extension_hint) {
            let filtered = results
                .iter()
                .filter(|path| path_has_extension(path, &locator_ext))
                .cloned()
                .collect::<Vec<_>>();
            if !filtered.is_empty() {
                results = filtered;
            }
        }
    }
    if results.len() == 1 {
        let root = value
            .get("root")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|root| !root.is_empty());
        if prefer_full_path {
            let resolved_path = Path::new(&results[0])
                .is_absolute()
                .then(|| canonical_existing_path(Path::new(&results[0])))
                .or_else(|| {
                    root.and_then(|root| {
                        let candidate = Path::new(root).join(&results[0]);
                        candidate
                            .exists()
                            .then(|| canonical_existing_path(&candidate))
                    })
                })
                .or_else(|| resolve_listing_entry_full_path(&results[0], auto_locator_path))
                .unwrap_or_else(|| results[0].clone());
            return Some(resolved_path);
        }
        return Some(results[0].clone());
    }
    let pattern = normalized_find_name_pattern(pattern.as_deref())
        .or_else(|| normalized_find_name_pattern(locator_hint))?;
    let preferred = preferred_fs_search_exact_match(&results, &pattern)?;
    if !prefer_full_path {
        return Some(preferred);
    }
    let root = value
        .get("root")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|root| !root.is_empty());
    Path::new(&preferred)
        .is_absolute()
        .then(|| canonical_existing_path(Path::new(&preferred)))
        .or_else(|| {
            root.and_then(|root| {
                let candidate = Path::new(root).join(&preferred);
                candidate
                    .exists()
                    .then(|| canonical_existing_path(&candidate))
            })
        })
        .or_else(|| resolve_listing_entry_full_path(&preferred, auto_locator_path))
        .or_else(|| Some(preferred))
}

fn path_extension_hint(path: &str) -> Option<String> {
    Path::new(path.trim())
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::trim)
        .filter(|ext| !ext.is_empty())
        .map(|ext| ext.trim_start_matches('.').to_ascii_lowercase())
}

fn path_has_extension(path: &str, expected_ext: &str) -> bool {
    Path::new(path.trim())
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .is_some_and(|ext| ext == expected_ext)
}

fn fs_search_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    locator_hint: Option<&str>,
    prefer_english: bool,
    allow_multi_result_list: bool,
    prefer_path_only: bool,
) -> Option<String> {
    if let Some(answer) = fs_search_grep_text_direct_answer_candidate(
        state,
        value,
        prefer_english,
        allow_multi_result_list,
        prefer_path_only,
    ) {
        return Some(answer);
    }
    if let Some((results, count, ext)) = fs_search_find_ext_results(value) {
        if count == 0 || results.is_empty() {
            return Some(observed_t_with_vars(
                state,
                "clawd.msg.fs_search_no_ext_match",
                "没有找到 .{ext} 文件",
                "No .{ext} files found.",
                prefer_english,
                &[("ext", &ext)],
            ));
        }
        return Some(results.join("\n"));
    }
    let (results, count, pattern) = fs_search_find_name_results(value)?;
    if count == 0 || results.is_empty() {
        return Some(observed_t(
            state,
            "clawd.msg.fs_search_no_match",
            "没有找到匹配项",
            "No matches found.",
            prefer_english,
        ));
    }
    if results.len() == 1 {
        if prefer_path_only {
            return Some(results[0].clone());
        }
        return Some(observed_t_with_vars(
            state,
            "clawd.msg.exists_with_path",
            "有，路径：{path}",
            "yes, path: {path}",
            prefer_english,
            &[("path", &results[0])],
        ));
    }
    if let Some(pattern) = normalized_find_name_pattern(pattern.as_deref())
        .or_else(|| normalized_find_name_pattern(locator_hint))
    {
        if let Some(preferred) = preferred_fs_search_exact_match(&results, &pattern) {
            if prefer_path_only {
                return Some(preferred);
            }
            return Some(observed_t_with_vars(
                state,
                "clawd.msg.exists_with_path",
                "有，路径：{path}",
                "yes, path: {path}",
                prefer_english,
                &[("path", &preferred)],
            ));
        }
        let ranked = rank_fs_search_candidates(&results, &pattern);
        if !ranked.is_empty() {
            return allow_multi_result_list.then(|| ranked.join("\n"));
        }
    }
    let matches = results.into_iter().take(3).collect::<Vec<_>>().join("\n");
    allow_multi_result_list.then_some(matches)
}

fn fs_search_grep_text_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    prefer_english: bool,
    allow_multi_result_list: bool,
    prefer_path_only: bool,
) -> Option<String> {
    let (matches, match_count, _query) = fs_search_grep_text_results(value)?;
    if match_count == 0 || matches.is_empty() {
        if let Some((name_results, name_count)) = fs_search_grep_text_name_results(value) {
            if name_count > 0 && !name_results.is_empty() {
                if name_results.len() == 1 {
                    return name_results.into_iter().next();
                }
                return allow_multi_result_list.then(|| {
                    name_results
                        .into_iter()
                        .take(3)
                        .collect::<Vec<_>>()
                        .join("\n")
                });
            }
        }
        return Some(observed_t(
            state,
            "clawd.msg.fs_search_no_match",
            "没有找到匹配项",
            "No matches found.",
            prefer_english,
        ));
    }
    if allow_multi_result_list && !prefer_path_only {
        return Some(
            matches
                .into_iter()
                .take(16)
                .map(|(_, line, text)| {
                    if line > 0 {
                        format!("{line}: {text}")
                    } else {
                        text
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    let mut paths = Vec::new();
    for (path, _, _) in matches {
        if !paths.iter().any(|seen| seen == &path) {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        return None;
    }
    if paths.len() == 1 {
        return paths.into_iter().next();
    }
    allow_multi_result_list.then(|| paths.into_iter().take(3).collect::<Vec<_>>().join("\n"))
}

fn fs_search_content_presence_direct_answer_candidate(
    state: Option<&AppState>,
    route: &crate::RouteResult,
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ContentPresenceCheck {
        return None;
    }
    let (matches, match_count, query) = fs_search_grep_text_results(value)?;
    if match_count == 0 || matches.is_empty() {
        if let Some((name_results, name_count)) = fs_search_grep_text_name_results(value) {
            if name_count > 0 && !name_results.is_empty() {
                let path_text = name_results
                    .into_iter()
                    .take(16)
                    .collect::<Vec<_>>()
                    .join("\n");
                return Some(path_text);
            }
        }
        let path = value
            .get("root")
            .or_else(|| value.get("path"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(route.output_contract.locator_hint.trim());
        if path.is_empty() {
            return Some(if prefer_english {
                format!("Does not contain `{query}`.")
            } else {
                format!("不包含 `{query}`。")
            });
        }
        return Some(if prefer_english {
            format!("Does not contain `{query}`. Path: {path}")
        } else {
            format!("不包含 `{query}`。路径：{path}")
        });
    }
    let mut paths = Vec::new();
    let mut first_match: Option<(String, u64, String)> = None;
    for (path, line, text) in matches {
        if first_match.is_none() {
            first_match = Some((path.clone(), line, text));
        }
        if !paths.iter().any(|seen| seen == &path) {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        return None;
    }
    let path_text = paths.into_iter().take(8).collect::<Vec<_>>().join("\n");
    let _ = state;
    if let Some((path, line, text)) = first_match {
        let location = if line > 0 {
            format!("{path}:{line}")
        } else {
            path
        };
        return Some(if prefer_english {
            format!("Contains `{query}`; evidence: {location} `{text}`.")
        } else {
            format!("包含 `{query}`，依据：{location} `{text}`。")
        });
    }
    Some(if prefer_english {
        format!("Contains `{query}`. Path:\n{path_text}")
    } else {
        format!("包含 `{query}`。路径：\n{path_text}")
    })
}

fn normalized_scope_text(value: &str) -> Option<String> {
    let normalized = value
        .trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_matches('/')
        .to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

fn locator_scope_candidates(locator_hint: &str) -> Vec<String> {
    let locator_hint = locator_hint.trim();
    if locator_hint.is_empty() {
        return Vec::new();
    }
    let path = Path::new(locator_hint);
    let scoped_path = if path.extension().is_some() {
        path.parent().unwrap_or(path)
    } else {
        path
    };
    let mut candidates = Vec::new();
    if let Some(scope) = normalized_scope_text(&scoped_path.to_string_lossy()) {
        candidates.push(scope);
    }
    if let Some(name) = scoped_path
        .file_name()
        .and_then(|value| value.to_str())
        .and_then(normalized_scope_text)
    {
        candidates.push(name);
    }
    candidates.sort();
    candidates.dedup();
    candidates
}

fn path_is_inside_locator_scope(path: &str, locator_hint: &str) -> bool {
    let Some(path) = normalized_scope_text(path) else {
        return false;
    };
    locator_scope_candidates(locator_hint)
        .into_iter()
        .any(|scope| {
            path == scope
                || path.starts_with(&format!("{scope}/"))
                || path.ends_with(&format!("/{scope}"))
                || path.contains(&format!("/{scope}/"))
        })
}

fn pathish_filter_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let push_token = |raw: &str, tokens: &mut Vec<String>| {
        let token = raw
            .trim_matches(|ch: char| ch == '.' || ch == '-' || ch == '_')
            .to_ascii_lowercase();
        if token.len() >= 2 && !tokens.iter().any(|seen| seen == &token) {
            tokens.push(token.clone());
        }
        for part in token.split(['.', '-', '_']) {
            let part = part.trim();
            if part.len() >= 3 && !tokens.iter().any(|seen| seen == part) {
                tokens.push(part.to_string());
            }
        }
    };
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
            current.push(ch);
        } else if !current.is_empty() {
            push_token(&current, &mut tokens);
            current.clear();
        }
    }
    if !current.is_empty() {
        push_token(&current, &mut tokens);
    }
    tokens
}

fn result_extensions(results: &[String]) -> Vec<String> {
    let mut exts = results
        .iter()
        .filter_map(|path| Path::new(path).extension().and_then(|ext| ext.to_str()))
        .map(|ext| ext.trim_start_matches('.').to_ascii_lowercase())
        .filter(|ext| !ext.is_empty())
        .collect::<Vec<_>>();
    exts.sort();
    exts.dedup();
    exts
}

fn structured_extension_hints(
    pattern: Option<&str>,
    locator_hint: &str,
    results: &[String],
) -> Vec<String> {
    let available_exts = result_extensions(results);
    if available_exts.is_empty() {
        return Vec::new();
    }
    let mut tokens = Vec::new();
    if let Some(pattern) = pattern {
        tokens.extend(pathish_filter_tokens(pattern));
    }
    tokens.extend(pathish_filter_tokens(locator_hint));
    tokens
        .into_iter()
        .filter(|token| available_exts.iter().any(|ext| ext == token))
        .collect::<Vec<_>>()
}

fn path_contains_filter_token(path: &str, token: &str) -> bool {
    let path = path.to_ascii_lowercase();
    let file_name = Path::new(&path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let stem = Path::new(&path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    path.contains(token) || file_name.contains(token) || stem.contains(token)
}

fn structured_fs_search_score(path: &str, tokens: &[String]) -> usize {
    tokens
        .iter()
        .filter(|token| {
            token.len() >= 3
                && !token.chars().all(|ch| ch.is_ascii_digit())
                && path_contains_filter_token(path, token)
        })
        .map(|token| token.len())
        .sum()
}

fn fs_search_route_filtered_listing_candidate(
    route: &crate::RouteResult,
    value: &serde_json::Value,
    allow_multi_result_list: bool,
) -> Option<String> {
    if !matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::FilePaths
            | crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::ScalarPathOnly
    ) {
        if route.output_contract.semantic_kind != crate::OutputSemanticKind::ExistenceWithPath
            || !route_prefers_plain_fs_search_paths(route)
        {
            return None;
        }
    }
    let (mut results, count, pattern) = fs_search_find_name_results(value)?;
    if count == 0 || results.is_empty() {
        return None;
    }
    if results.len() == 1 {
        return Some(results[0].clone());
    }

    let locator_hint = route.output_contract.locator_hint.trim();
    if !locator_hint.is_empty() {
        let scoped = results
            .iter()
            .filter(|path| path_is_inside_locator_scope(path, locator_hint))
            .cloned()
            .collect::<Vec<_>>();
        if !scoped.is_empty() && scoped.len() < results.len() {
            results = scoped;
        }
    }

    let normalized_pattern = normalized_find_name_pattern(pattern.as_deref());
    let ext_hints =
        structured_extension_hints(normalized_pattern.as_deref(), locator_hint, &results);
    if !ext_hints.is_empty() {
        let ext_filtered = results
            .iter()
            .filter(|path| {
                Path::new(path)
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.trim_start_matches('.').to_ascii_lowercase())
                    .is_some_and(|ext| ext_hints.iter().any(|hint| hint == &ext))
            })
            .cloned()
            .collect::<Vec<_>>();
        if !ext_filtered.is_empty() {
            results = ext_filtered;
        }
    }

    let mut tokens = Vec::new();
    tokens.extend(pathish_filter_tokens(locator_hint));
    if let Some(pattern) = normalized_pattern.as_deref() {
        tokens.extend(pathish_filter_tokens(pattern));
    }
    tokens.extend(ext_hints);
    tokens.sort();
    tokens.dedup();

    if tokens.is_empty() {
        return allow_multi_result_list.then(|| {
            results
                .into_iter()
                .take(fs_search_result_list_limit(route))
                .collect::<Vec<_>>()
                .join("\n")
        });
    }
    let mut scored = results
        .iter()
        .cloned()
        .map(|path| {
            let score = structured_fs_search_score(&path, &tokens);
            (score, path)
        })
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let top_score = scored.first().map(|(score, _)| *score).unwrap_or_default();
    if top_score == 0 {
        return allow_multi_result_list.then(|| {
            results
                .into_iter()
                .take(fs_search_result_list_limit(route))
                .collect::<Vec<_>>()
                .join("\n")
        });
    }
    let second_score = scored
        .iter()
        .find_map(|(score, _)| (*score < top_score).then_some(*score))
        .unwrap_or_default();
    let decisive_single_candidate =
        top_score >= 8 && (second_score == 0 || top_score >= second_score.saturating_mul(2));
    let mut filtered = scored
        .into_iter()
        .filter(|(score, _)| *score == top_score)
        .map(|(_, path)| path)
        .collect::<Vec<_>>();
    filtered.sort();
    filtered.dedup();
    if filtered.len() == 1 {
        if allow_multi_result_list && !decisive_single_candidate && results.len() > 1 {
            return Some(
                results
                    .into_iter()
                    .take(fs_search_result_list_limit(route))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        return filtered.into_iter().next();
    }
    allow_multi_result_list.then(|| filtered.join("\n"))
}

fn fs_search_result_list_limit(route: &crate::RouteResult) -> usize {
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::FilePaths {
        5
    } else {
        3
    }
}

fn absolutize_fs_search_answer_paths(
    state: Option<&AppState>,
    route: Option<&crate::RouteResult>,
    value: &serde_json::Value,
    answer: String,
    prefer_full_path: bool,
) -> String {
    if !prefer_full_path
        || !route.is_some_and(|route| {
            route.output_contract.semantic_kind == crate::OutputSemanticKind::FilePaths
        })
    {
        return answer;
    }
    let Some(state) = state else {
        return answer;
    };
    let lines = answer
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|path| absolutize_fs_search_result_path(state, value, path))
        .collect::<Vec<_>>();
    if lines.is_empty() {
        answer
    } else {
        lines.join("\n")
    }
}

fn absolutize_fs_search_result_path(
    state: &AppState,
    value: &serde_json::Value,
    path: &str,
) -> String {
    let path = path.trim();
    let path_obj = Path::new(path);
    if path_obj.is_absolute() {
        return canonical_existing_path(path_obj);
    }
    let workspace_root = &state.skill_rt.workspace_root;
    let workspace_candidate = workspace_root.join(path);
    if workspace_candidate.exists() {
        return canonical_existing_path(&workspace_candidate);
    }
    let root = value
        .get("root")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|root| !root.is_empty() && *root != ".");
    if let Some(root) = root {
        let root_path = Path::new(root);
        let base = if root_path.is_absolute() {
            root_path.to_path_buf()
        } else {
            workspace_root.join(root_path)
        };
        let rooted_candidate = base.join(path);
        if rooted_candidate.exists() {
            return canonical_existing_path(&rooted_candidate);
        }
        return rooted_candidate.display().to_string();
    }
    workspace_candidate.display().to_string()
}

fn parent_directory_listing_from_paths(paths: &[String]) -> Option<String> {
    let mut dirs = Vec::new();
    for path in paths {
        let path = path.trim();
        if path.is_empty() {
            continue;
        }
        let parent = Path::new(path)
            .parent()
            .map(|parent| {
                let display = parent.to_string_lossy().trim().to_string();
                if display.is_empty() {
                    ".".to_string()
                } else {
                    display
                }
            })
            .unwrap_or_else(|| ".".to_string());
        if !dirs.iter().any(|seen| seen == &parent) {
            dirs.push(parent);
        }
    }
    (!dirs.is_empty()).then(|| dirs.join("\n"))
}

fn fs_search_semantic_listing_candidate(
    route: &crate::RouteResult,
    value: &serde_json::Value,
) -> Option<String> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::DirectoryNames {
        return None;
    }
    let (results, count, _ext) = fs_search_find_ext_results(value)?;
    if count == 0 || results.is_empty() {
        return None;
    }
    parent_directory_listing_from_paths(&results)
}

fn structured_scalar_candidate(
    state: Option<&AppState>,
    route: Option<&crate::RouteResult>,
    skill: &str,
    body: &str,
    locator_hint: Option<&str>,
    auto_locator_path: Option<&str>,
    prefer_full_path: bool,
    allow_localized_direct_template: bool,
    prefer_english: bool,
) -> Option<String> {
    if skill == "package_manager" {
        let response_shape = route.map(|route| route.output_contract.response_shape);
        return package_manager_summary_candidate(
            state,
            body,
            response_shape,
            allow_localized_direct_template,
            prefer_english,
        );
    }
    if skill == "git_basic" {
        return git_basic_scalar_candidate(route, body);
    }
    if skill == "archive_basic" {
        if let Some(route) = route {
            match route.output_contract.semantic_kind {
                crate::OutputSemanticKind::ArchivePack => {
                    if let Some(path) = archive_basic_path_value_from_body(
                        body,
                        &["archive", "archive_path", "output_path", "path"],
                    ) {
                        return Some(path);
                    }
                }
                crate::OutputSemanticKind::ArchiveUnpack => {
                    if let Some(path) = archive_basic_path_value_from_body(
                        body,
                        &["dest", "dest_path", "destination", "path"],
                    ) {
                        return Some(path);
                    }
                }
                _ => {}
            }
        }
        let summary = archive_list_summary_from_body(body)?;
        if route.is_some_and(route_requests_scalar_count) {
            return Some(summary.entries.len().to_string());
        }
        return route
            .filter(|route| route_requests_scalar_existence(route))
            .and_then(|route| {
                archive_entry_existence_direct_answer(
                    state,
                    route,
                    Some(route.resolved_intent.as_str()),
                    &summary,
                    auto_locator_path.or(locator_hint),
                    prefer_english,
                )
            });
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if skill == "db_basic" {
        if let Some(route) = route {
            return match route.output_contract.semantic_kind {
                crate::OutputSemanticKind::ScalarCount => db_basic_count_candidate(&value),
                crate::OutputSemanticKind::SqliteTableNamesOnly => {
                    db_basic_table_names(&value).map(|names| names.join("\n"))
                }
                crate::OutputSemanticKind::SqliteTableListing
                | crate::OutputSemanticKind::SqliteDatabaseKindJudgment => None,
                _ => db_basic_scalar_candidate(&value),
            };
        }
        return db_basic_scalar_candidate(&value);
    }
    if skill == "service_control" {
        let response_shape = route.map(|route| route.output_contract.response_shape);
        return route
            .is_some_and(|route| {
                route.output_contract.semantic_kind == crate::OutputSemanticKind::ServiceStatus
                    || route.output_contract.response_shape == crate::OutputResponseShape::Scalar
            })
            .then(|| {
                service_control_status_direct_answer_candidate(
                    state,
                    &value,
                    response_shape,
                    prefer_english,
                )
            })
            .flatten()
            .or_else(|| service_control_summary_candidate(&value));
    }
    if skill == "fs_search" {
        let rooted_scalar = prefer_full_path
            .then(|| {
                fs_search_scalar_candidate(
                    state,
                    &value,
                    locator_hint,
                    auto_locator_path,
                    prefer_full_path,
                    prefer_english,
                )
            })
            .flatten();
        if let Some(answer) = rooted_scalar.or_else(|| {
            route
                .and_then(|route| {
                    fs_search_output_direct_answer_candidate(
                        state,
                        Some(route),
                        &value,
                        locator_hint,
                        prefer_english,
                        true,
                        prefer_full_path,
                    )
                })
                .or_else(|| {
                    fs_search_scalar_candidate(
                        state,
                        &value,
                        locator_hint,
                        auto_locator_path,
                        prefer_full_path,
                        prefer_english,
                    )
                })
        }) {
            return Some(answer);
        }
        return None;
    }
    if skill == "fs_basic"
        && value
            .get("action")
            .and_then(|v| v.as_str())
            .is_some_and(|action| {
                action.eq_ignore_ascii_case("find_ext") || action.eq_ignore_ascii_case("find_name")
            })
    {
        if let Some(answer) = route.and_then(|route| {
            fs_search_output_direct_answer_candidate(
                state,
                Some(route),
                &value,
                locator_hint,
                prefer_english,
                true,
                prefer_full_path,
            )
        }) {
            return Some(answer);
        }
    }
    if !matches!(skill, "system_basic" | "config_basic" | "fs_basic") {
        return None;
    }
    if skill == "system_basic"
        && route.is_some_and(route_requests_scalar_path_only)
        && system_basic_value_looks_like_info(&value)
    {
        return system_basic_info_scalar_path_candidate(&value);
    }
    let action = value.get("action").and_then(|v| v.as_str())?;
    match action {
        "validate_structured" => {
            validate_structured_direct_answer_candidate(state, &value, prefer_english)
        }
        "read_range" => route
            .filter(|route| route_allows_scalar_read_range_direct_answer(route))
            .and_then(|_| {
                value
                    .get("excerpt")
                    .and_then(|v| v.as_str())
                    .and_then(|excerpt| {
                        normalize_read_range_excerpt_for_direct_answer(
                            state,
                            excerpt,
                            prefer_english,
                            read_range_preserve_blank_lines(&value),
                        )
                    })
            }),
        "inventory_dir" => {
            let hidden_count_route = route.is_some_and(|route| {
                route.output_contract.response_shape == crate::OutputResponseShape::Scalar
                    && route_requests_hidden_entries_check(route)
            });
            if hidden_count_route {
                value
                    .get("counts")
                    .and_then(|v| v.get("hidden"))
                    .and_then(value_scalar_text)
                    .or_else(|| {
                        inventory_dir_names(&value).map(|names| {
                            names
                                .into_iter()
                                .filter(|name| is_user_hidden_entry(name))
                                .count()
                                .to_string()
                        })
                    })
            } else if route.is_some_and(route_requests_scalar_count) {
                value
                    .get("counts")
                    .and_then(|v| v.get("total"))
                    .and_then(value_scalar_text)
                    .or_else(|| inventory_dir_names(&value).map(|names| names.len().to_string()))
            } else if route.is_some_and(route_requests_scalar_path_only) {
                inventory_dir_scalar_path_candidate(&value, prefer_full_path)
            } else {
                None
            }
        }
        "tree_summary" => tree_summary_direct_answer_candidate(state, &value, prefer_english),
        "dir_compare" => dir_compare_direct_answer_candidate(state, &value, prefer_english),
        "extract_field" | "read_field" => {
            if route.is_some_and(|route| {
                route.output_contract.response_shape != crate::OutputResponseShape::Scalar
            }) {
                return None;
            }
            if value
                .get("exists")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                if route.is_some() && extract_field_has_non_exact_resolution(&value) {
                    if let Some(field_path) = json_trimmed_str(&value, "resolved_field_path") {
                        let field_value = value.get("value").unwrap_or(&serde_json::Value::Null);
                        if matches!(
                            field_value,
                            serde_json::Value::Object(_) | serde_json::Value::Array(_)
                        ) {
                            let scalar_contract = route.is_some_and(|route| {
                                route.output_contract.response_shape
                                    == crate::OutputResponseShape::Scalar
                            });
                            if !scalar_contract {
                                return None;
                            }
                            return Some(structured_field_display_line(
                                state,
                                field_path,
                                field_value,
                                value.get("value_text").and_then(|v| v.as_str()),
                                true,
                                prefer_english,
                            ));
                        }
                        if route.is_some_and(|route| {
                            route.output_contract.response_shape
                                == crate::OutputResponseShape::Scalar
                        }) && json_trimmed_str(&value, "match_strategy")
                            .is_some_and(|strategy| strategy == "array_item_key_path")
                        {
                            return value_structured_text(
                                field_value,
                                value.get("value_text").and_then(|v| v.as_str()),
                            );
                        }
                        return Some(structured_field_display_line(
                            state,
                            field_path,
                            field_value,
                            value.get("value_text").and_then(|v| v.as_str()),
                            true,
                            prefer_english,
                        ));
                    }
                }
                let field_value = value.get("value").unwrap_or(&serde_json::Value::Null);
                if matches!(
                    field_value,
                    serde_json::Value::Object(_) | serde_json::Value::Array(_)
                ) {
                    let scalar_contract = route.is_some_and(|route| {
                        route.output_contract.response_shape == crate::OutputResponseShape::Scalar
                    });
                    if !scalar_contract {
                        return None;
                    }
                    let value_text = value.get("value_text").and_then(|v| v.as_str());
                    if route.is_some() && extract_field_has_non_exact_resolution(&value) {
                        if let Some(field_path) = json_trimmed_str(&value, "resolved_field_path") {
                            return Some(structured_field_display_line(
                                state,
                                field_path,
                                field_value,
                                value_text,
                                true,
                                prefer_english,
                            ));
                        }
                    }
                    return value_structured_text(field_value, value_text);
                }
                let text = value
                    .get("value_text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if !text.is_empty() {
                    return Some(text.to_string());
                }
                return value.get("value").and_then(|v| match v {
                    serde_json::Value::Null => Some("null".to_string()),
                    serde_json::Value::Bool(b) => Some(b.to_string()),
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    serde_json::Value::String(s) => Some(s.clone()),
                    _ => None,
                });
            }
            let field_path = value
                .get("field_path")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .unwrap_or("requested field");
            Some(observed_t_with_vars(
                state,
                "clawd.msg.extract_field_missing",
                "未找到 {field_path} 字段",
                "field not found: {field_path}",
                prefer_english,
                &[("field_path", field_path)],
            ))
        }
        "path_batch_facts" => {
            if route.is_some_and(route_requests_scalar_existence) {
                system_basic_scalar_existence_candidate(state, &value, prefer_english)
            } else if route.is_some_and(route_requests_scalar_path_only) {
                system_basic_path_batch_scalar_path_candidate(&value)
            } else {
                None
            }
        }
        "runtime_status" => value
            .get("value")
            .or_else(|| value.get("field_value"))
            .or_else(|| value.get("command_output"))
            .and_then(value_scalar_text),
        "count_inventory" => count_inventory_direct_answer_candidate(
            state,
            &value,
            route.map(|route| route.output_contract.response_shape),
            prefer_english,
        ),
        "structured_keys" => structured_keys_direct_answer_candidate(
            state,
            &value,
            route.map(|route| route.resolved_intent.as_str()),
            route.map(|route| route.output_contract.response_shape),
            prefer_english,
        ),
        _ => None,
    }
}

fn archive_basic_path_value_from_body(body: &str, labels: &[&str]) -> Option<String> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body.trim()) {
        for label in labels {
            if let Some(path) = value
                .get(*label)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| archive_basic_observed_path_candidate(value))
            {
                return Some(path.to_string());
            }
        }
    }
    for token in body.split_whitespace() {
        let token = token.trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '。' | '，'
            )
        });
        let Some((key, rhs)) = token.split_once('=') else {
            continue;
        };
        if !labels
            .iter()
            .any(|label| key.trim().eq_ignore_ascii_case(label))
        {
            continue;
        }
        let rhs = rhs.trim();
        if archive_basic_observed_path_candidate(rhs) {
            return Some(rhs.to_string());
        }
    }
    None
}

fn archive_basic_observed_path_candidate(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 4096
        && !value.contains(|ch| matches!(ch, '\n' | '\r' | '\0'))
        && !value.contains("://")
        && (value.starts_with('/')
            || value.starts_with("./")
            || value.starts_with("../")
            || value.contains('/'))
}

fn package_manager_summary_candidate(
    state: Option<&AppState>,
    body: &str,
    response_shape: Option<crate::OutputResponseShape>,
    allow_localized_direct_template: bool,
    prefer_english: bool,
) -> Option<String> {
    let manager = body
        .lines()
        .find_map(|line| line.trim().strip_prefix("package_manager="))
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    match response_shape {
        Some(crate::OutputResponseShape::Scalar) => Some(manager.to_string()),
        Some(
            crate::OutputResponseShape::OneSentence
            | crate::OutputResponseShape::Free
            | crate::OutputResponseShape::Strict,
        ) if allow_localized_direct_template => {
            Some(observed_t_with_vars(
                state,
                "clawd.msg.package_manager_detected",
                "检测到的包管理器是 {manager}，依据是 package_manager 返回了 package_manager={manager}。",
                "Detected package manager: {manager}. Basis: package_manager returned package_manager={manager}.",
                prefer_english,
                &[("manager", manager)],
            ))
        }
        _ => None,
    }
}

fn git_basic_commit_subject_candidate(body: &str) -> Option<String> {
    static GIT_ONELINE_SUBJECT_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let regex = GIT_ONELINE_SUBJECT_RE.get_or_init(|| {
        regex::Regex::new(r"^[0-9a-fA-F]{7,40}\s+(.+)$").expect("valid git oneline regex")
    });
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| *line != "exit=0")
        .find_map(|line| regex.captures(line))
        .and_then(|captures| captures.get(1))
        .map(|subject| subject.as_str().trim().to_string())
        .filter(|subject| !subject.is_empty())
}

fn git_basic_scalar_candidate(route: Option<&crate::RouteResult>, body: &str) -> Option<String> {
    if route.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::GitCommitSubject
    }) {
        return git_basic_commit_subject_candidate(body);
    }
    let scalar = normalized_scalar_candidate(body)?;
    static GIT_ONELINE_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let regex = GIT_ONELINE_RE.get_or_init(|| {
        regex::Regex::new(r"^[0-9a-fA-F]{7,40}\s+.+$").expect("valid git oneline regex")
    });
    if regex.is_match(&scalar) {
        return None;
    }
    Some(scalar)
}

fn structured_field_display_line(
    state: Option<&AppState>,
    field_path: &str,
    value: &serde_json::Value,
    value_text: Option<&str>,
    exists: bool,
    prefer_english: bool,
) -> String {
    if !exists {
        return observed_t_with_vars(
            state,
            "clawd.msg.structured_field_missing_display",
            "{field_path}: 不存在",
            "{field_path}: not found",
            prefer_english,
            &[("field_path", field_path)],
        );
    }
    let rendered = value_text
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
        .or_else(|| value_scalar_text(value))
        .unwrap_or_else(|| {
            serde_json::to_string(value).unwrap_or_else(|_| "<unrenderable>".to_string())
        });
    format!("{field_path}: {rendered}")
}

fn json_trimmed_str<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn extract_field_has_non_exact_resolution(value: &serde_json::Value) -> bool {
    let Some(resolved) = json_trimmed_str(value, "resolved_field_path") else {
        return false;
    };
    let Some(requested) = json_trimmed_str(value, "field_path") else {
        return false;
    };
    if resolved.eq_ignore_ascii_case(requested) {
        return false;
    }
    !matches!(
        json_trimmed_str(value, "match_strategy"),
        Some("exact_path")
    )
}

fn field_path_has_array_identity_selector(field_path: &str) -> bool {
    let field_path = field_path.trim();
    field_path.contains('[') && field_path.contains(']') && field_path.contains('=')
}

fn extract_field_should_return_value_only(value: &serde_json::Value, field_path: &str) -> bool {
    field_path_has_array_identity_selector(field_path)
        || matches!(
            json_trimmed_str(value, "match_strategy"),
            Some("array_item_key_path")
        )
}

fn extract_fields_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    response_shape: Option<crate::OutputResponseShape>,
    prefer_english: bool,
    allow_localized_template: bool,
) -> Option<String> {
    if !matches!(
        value.get("action").and_then(|v| v.as_str()),
        Some("extract_fields" | "read_fields")
    ) {
        return None;
    }
    let results = value.get("results")?.as_array()?;
    if results.is_empty() {
        return None;
    }
    if !allow_localized_template && results.iter().any(|item| field_result_is_missing(item)) {
        return None;
    }
    let lines = results
        .iter()
        .filter_map(|item| {
            let field_path = item
                .get("resolved_field_path")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .or_else(|| item.get("field_path")?.as_str().map(str::trim))?;
            if field_path.is_empty() {
                return None;
            }
            Some(structured_field_display_line(
                state,
                field_path,
                item.get("value").unwrap_or(&serde_json::Value::Null),
                item.get("value_text").and_then(|v| v.as_str()),
                item.get("exists")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                prefer_english,
            ))
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }
    if matches!(
        response_shape,
        Some(crate::OutputResponseShape::OneSentence)
    ) {
        return Some(format!("{}.", lines.join("; ")));
    }
    Some(lines.join("\n"))
}

fn field_result_is_missing(value: &serde_json::Value) -> bool {
    !value
        .get("exists")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn extract_field_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    response_shape: Option<crate::OutputResponseShape>,
    prefer_english: bool,
    allow_localized_template: bool,
) -> Option<String> {
    if !matches!(
        value.get("action").and_then(|v| v.as_str()),
        Some("extract_field" | "read_field")
    ) {
        return None;
    }
    if matches!(response_shape, Some(crate::OutputResponseShape::Scalar)) {
        return None;
    }
    let field_path = value
        .get("resolved_field_path")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            value
                .get("field_path")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })?;
    let exists = value
        .get("exists")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !exists {
        if !allow_localized_template {
            return None;
        }
        return Some(observed_t_with_vars(
            state,
            "clawd.msg.extract_field_missing",
            "未找到 {field_path} 字段",
            "field not found: {field_path}",
            prefer_english,
            &[("field_path", field_path)],
        ));
    }
    let field_value = value.get("value").unwrap_or(&serde_json::Value::Null);
    if matches!(
        field_value,
        serde_json::Value::Object(_) | serde_json::Value::Array(_)
    ) {
        if let Some(answer) =
            enum_field_direct_answer_candidate(state, field_path, field_value, prefer_english)
        {
            return Some(answer);
        }
        return None;
    }
    if extract_field_should_return_value_only(value, field_path) {
        return value_structured_text(
            field_value,
            value.get("value_text").and_then(|v| v.as_str()),
        );
    }
    Some(structured_field_display_line(
        state,
        field_path,
        field_value,
        value.get("value_text").and_then(|v| v.as_str()),
        exists,
        prefer_english,
    ))
}

fn enum_field_direct_answer_candidate(
    state: Option<&AppState>,
    field_path: &str,
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    let enum_value = match value {
        serde_json::Value::Object(map) => map.get("enum")?,
        serde_json::Value::Array(_) => value,
        _ => return None,
    };
    let values = enum_value.as_array()?;
    if values.is_empty() {
        return None;
    }
    let rendered_values = values
        .iter()
        .map(value_scalar_text)
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .filter(|item| !item.trim().is_empty())
        .map(|item| format!("`{item}`"))
        .collect::<Vec<_>>();
    if rendered_values.is_empty() {
        return None;
    }
    let values_text = rendered_values.join(", ");
    Some(observed_t_with_vars(
        state,
        "clawd.msg.enum_field_values",
        "{field_path} 的枚举值是：{values}",
        "`{field_path}` enum values: {values}",
        prefer_english,
        &[("field_path", field_path), ("values", &values_text)],
    ))
}

fn structured_keys_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    current_request: Option<&str>,
    response_shape: Option<crate::OutputResponseShape>,
    prefer_english: bool,
) -> Option<String> {
    if value.get("action").and_then(|v| v.as_str()) != Some("structured_keys") {
        return None;
    }
    let field_path = value
        .get("field_path")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    let exists = value
        .get("exists")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !exists {
        return Some(if field_path.is_empty() {
            observed_t(
                state,
                "clawd.msg.structured_keys_root_missing",
                "没有可列出的顶层键。",
                "No top-level keys are available to list.",
                prefer_english,
            )
        } else {
            observed_t_with_vars(
                state,
                "clawd.msg.structured_keys_field_missing",
                "{field_path} 字段不存在。",
                "Field `{field_path}` does not exist.",
                prefer_english,
                &[("field_path", field_path)],
            )
        });
    }
    let container_type = value
        .get("container_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match container_type {
        "object" => {
            let keys = value
                .get("keys")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v.as_str())
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if keys.is_empty() {
                return None;
            }
            if let Some(target_key) = current_request
                .and_then(|request| structured_keys_presence_target_from_request(request, &keys))
            {
                let contains = keys
                    .iter()
                    .any(|key| key.eq_ignore_ascii_case(target_key.as_str()));
                return Some(structured_keys_presence_answer(
                    state,
                    &target_key,
                    contains,
                    prefer_english,
                ));
            }
            if matches!(
                response_shape,
                Some(crate::OutputResponseShape::OneSentence)
            ) {
                return None;
            }
            return Some(keys.join("\n"));
        }
        "array" => {
            let identity_values = value
                .get("identity_values")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v.as_str())
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if let Some(target_key) = current_request.and_then(|request| {
                structured_keys_presence_target_from_request(request, &identity_values)
            }) {
                let contains = identity_values
                    .iter()
                    .any(|key| key.eq_ignore_ascii_case(target_key.as_str()));
                return Some(structured_array_identity_presence_answer(
                    state,
                    &target_key,
                    contains,
                    prefer_english,
                ));
            }
            if !identity_values.is_empty()
                && !matches!(
                    response_shape,
                    Some(crate::OutputResponseShape::OneSentence)
                )
            {
                return Some(identity_values.join("\n"));
            }
            let indices = value
                .get("indices_preview")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.get("index").and_then(|v| v.as_u64()))
                        .map(|idx| idx.to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if indices.is_empty() {
                return None;
            }
            if matches!(
                response_shape,
                Some(crate::OutputResponseShape::OneSentence)
            ) {
                return None;
            }
            return Some(indices.join("\n"));
        }
        _ => {}
    }
    Some(if field_path.is_empty() {
        observed_t(
            state,
            "clawd.msg.structured_keys_non_container_root",
            "这个位置不是对象或数组，没有可列出的键。",
            "This value is not an object or array, so there are no keys to list.",
            prefer_english,
        )
    } else {
        observed_t_with_vars(
            state,
            "clawd.msg.structured_keys_non_container_field",
            "{field_path} 不是对象或数组，没有可列出的键。",
            "`{field_path}` is not an object or array, so there are no keys to list.",
            prefer_english,
            &[("field_path", field_path)],
        )
    })
}

fn structured_keys_presence_answer(
    state: Option<&AppState>,
    key: &str,
    contains: bool,
    prefer_english: bool,
) -> String {
    if contains {
        observed_t_with_vars(
            state,
            "clawd.msg.structured_keys_contains_key",
            "包含 {key} 字段",
            "Contains field `{key}`",
            prefer_english,
            &[("key", key)],
        )
    } else {
        observed_t_with_vars(
            state,
            "clawd.msg.structured_keys_missing_key",
            "不包含 {key} 字段",
            "Does not contain field `{key}`",
            prefer_english,
            &[("key", key)],
        )
    }
}

fn structured_array_identity_presence_answer(
    state: Option<&AppState>,
    value: &str,
    contains: bool,
    prefer_english: bool,
) -> String {
    if contains {
        observed_t_with_vars(
            state,
            "clawd.msg.structured_array_identity_contains_value",
            "包含 {value}",
            "Contains `{value}`",
            prefer_english,
            &[("value", value)],
        )
    } else {
        observed_t_with_vars(
            state,
            "clawd.msg.structured_array_identity_missing_value",
            "不包含 {value}",
            "Does not contain `{value}`",
            prefer_english,
            &[("value", value)],
        )
    }
}

fn structured_keys_presence_target_from_request(request: &str, keys: &[String]) -> Option<String> {
    let tokens = structured_key_candidate_tokens(request);
    if tokens.is_empty() {
        return None;
    }
    let mut observed_mentions = Vec::new();
    for key in keys {
        if tokens.iter().any(|token| token.eq_ignore_ascii_case(key)) {
            push_unique_case_insensitive_string(&mut observed_mentions, key.clone());
        }
    }
    if observed_mentions.len() == 1 {
        return observed_mentions.into_iter().next();
    }
    let mut candidate_mentions = Vec::new();
    for token in explicit_structured_key_candidate_tokens(request) {
        if keys.iter().any(|key| key.eq_ignore_ascii_case(&token)) {
            continue;
        }
        push_unique_case_insensitive_string(&mut candidate_mentions, token);
    }
    for token in tokens {
        if !token_looks_like_structured_key_identifier(&token) {
            continue;
        }
        if keys.iter().any(|key| key.eq_ignore_ascii_case(&token)) {
            continue;
        }
        push_unique_case_insensitive_string(&mut candidate_mentions, token);
    }
    (candidate_mentions.len() == 1).then(|| candidate_mentions.remove(0))
}

fn token_looks_like_structured_key_identifier(token: &str) -> bool {
    let token = token.trim();
    token.contains(['_', '.', '$'])
}

fn explicit_structured_key_candidate_tokens(request: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let chars = request.char_indices().collect::<Vec<_>>();
    let mut idx = 0usize;
    while idx < chars.len() {
        let (start, ch) = chars[idx];
        if !matches!(ch, '`' | '\'' | '"') {
            idx += 1;
            continue;
        }
        let quote = ch;
        let content_start = start + ch.len_utf8();
        let mut end_idx = idx + 1;
        while end_idx < chars.len() {
            let (end, end_ch) = chars[end_idx];
            if end_ch == quote {
                let raw = request[content_start..end].trim();
                let token = raw.trim_matches(|ch: char| matches!(ch, '.' | '-' | '_' | '$'));
                if token.len() >= 2
                    && !token.contains(['/', '\\'])
                    && !token.chars().all(|ch| ch.is_ascii_digit())
                {
                    push_unique_case_insensitive_string(&mut tokens, token.to_string());
                }
                break;
            }
            end_idx += 1;
        }
        idx = end_idx.saturating_add(1);
    }
    tokens
}

fn structured_key_candidate_tokens(request: &str) -> Vec<String> {
    let filename_candidates = crate::delivery_utils::extract_filename_candidates(request)
        .into_iter()
        .map(|candidate| candidate.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let mut tokens = Vec::new();
    for raw in request.split(|ch: char| {
        !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '$' | '/' | '\\'))
    }) {
        let token = raw.trim_matches(|ch: char| matches!(ch, '.' | '-' | '_' | '$'));
        if token.len() < 2
            || token.contains(['/', '\\'])
            || token.chars().all(|ch| ch.is_ascii_digit())
            || crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(token)
        {
            continue;
        }
        if filename_candidates
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(token))
        {
            continue;
        }
        push_unique_case_insensitive_string(&mut tokens, token.to_string());
    }
    tokens
}

fn push_unique_case_insensitive_string(values: &mut Vec<String>, value: String) {
    if !values
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&value))
    {
        values.push(value);
    }
}

fn validate_structured_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    if value.get("action").and_then(|v| v.as_str()) != Some("validate_structured") {
        return None;
    }
    let valid = value.get("valid")?.as_bool()?;
    let format = value
        .get("format")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("structured");
    if valid {
        return Some(observed_t_with_vars(
            state,
            "clawd.msg.validate_structured_pass",
            "通过：{format} 解析成功",
            "pass: {format} parsed successfully",
            prefer_english,
            &[("format", format)],
        ));
    }
    let reason = value
        .get("error_text")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("parse failed");
    Some(observed_t_with_vars(
        state,
        "clawd.msg.validate_structured_fail",
        "失败：{reason}",
        "fail: {reason}",
        prefer_english,
        &[("reason", reason)],
    ))
}

fn normalize_read_range_excerpt(excerpt: &str) -> Option<String> {
    let lines = excerpt
        .lines()
        .map(str::trim_end)
        .map(|line| {
            let content = line
                .split_once('|')
                .filter(|(prefix, _)| {
                    !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit())
                })
                .map(|(_, rest)| rest.trim_start().to_string())
                .unwrap_or_else(|| line.trim().to_string());
            crate::visible_text::sanitize_user_visible_text(&content)
        })
        .collect::<Vec<_>>();
    if lines.is_empty() || lines.iter().all(|line| line.is_empty()) {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn normalize_read_range_excerpt_for_direct_answer(
    state: Option<&AppState>,
    excerpt: &str,
    prefer_english: bool,
    preserve_blank_lines: bool,
) -> Option<String> {
    let lines = excerpt
        .lines()
        .map(str::trim_end)
        .map(|line| {
            let content = line
                .split_once('|')
                .filter(|(prefix, _)| {
                    !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit())
                })
                .map(|(_, rest)| rest.trim_start().to_string())
                .unwrap_or_else(|| line.trim().to_string());
            crate::visible_text::sanitize_user_visible_text(&content)
        })
        .collect::<Vec<_>>();
    if lines.is_empty() || lines.iter().all(|line| line.is_empty()) {
        return None;
    }
    if !preserve_blank_lines && lines.iter().any(|line| line.is_empty()) {
        let blank = observed_t(
            state,
            "clawd.msg.read_range_blank_line",
            "（空行）",
            "(blank line)",
            prefer_english,
        );
        return Some(
            lines
                .into_iter()
                .map(|line| if line.is_empty() { blank.clone() } else { line })
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    Some(lines.join("\n"))
}

fn read_range_preserve_blank_lines(value: &serde_json::Value) -> bool {
    value.get("start_line").and_then(|v| v.as_u64()).is_some()
        && value.get("end_line").and_then(|v| v.as_u64()).is_some()
}

pub(crate) fn tail_read_range_direct_answer_candidate(
    body: &str,
    prefer_english: bool,
) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if value.get("action").and_then(|v| v.as_str()) != Some("read_range") {
        return None;
    }
    if value.get("mode").and_then(|v| v.as_str()) != Some("tail") {
        return None;
    }
    let requested_n = value.get("requested_n").and_then(|v| v.as_u64())?;
    if requested_n == 0 || requested_n > 50 {
        return None;
    }
    value
        .get("excerpt")
        .and_then(|v| v.as_str())
        .and_then(|excerpt| {
            normalize_read_range_excerpt_for_direct_answer(None, excerpt, prefer_english, false)
        })
}

fn bounded_read_range_direct_answer_candidate(body: &str, prefer_english: bool) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if value.get("action").and_then(|v| v.as_str()) != Some("read_range") {
        return None;
    }
    let mode = value.get("mode").and_then(|v| v.as_str()).unwrap_or("");
    if !matches!(mode, "head" | "tail" | "range") {
        return None;
    }
    let bounded_lines = value
        .get("requested_n")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            let start = value.get("start_line")?.as_u64()?;
            let end = value.get("end_line")?.as_u64()?;
            (end >= start).then_some(end - start + 1)
        })?;
    if bounded_lines == 0 || bounded_lines > 100 {
        return None;
    }
    value
        .get("excerpt")
        .and_then(|v| v.as_str())
        .and_then(|excerpt| {
            normalize_read_range_excerpt_for_direct_answer(None, excerpt, prefer_english, false)
        })
}

fn latest_bounded_read_range_direct_answer(
    loop_state: &LoopState,
    prefer_english: bool,
) -> Option<String> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .find_map(|step| {
            if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
                return None;
            }
            let output = step.output.as_deref()?.trim();
            bounded_read_range_direct_answer_candidate(output, prefer_english)
        })
}

fn compact_delivery_match_text(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn answer_contains_observed_excerpt(answer: &str, excerpt: &str) -> bool {
    let answer = compact_delivery_match_text(answer);
    let excerpt = compact_delivery_match_text(excerpt);
    if excerpt.is_empty() {
        return true;
    }
    answer.contains(&excerpt) || excerpt.lines().all(|line| answer.contains(line))
}

fn compose_content_excerpt_with_summary_answer(
    answer: &str,
    loop_state: &LoopState,
    prefer_english: bool,
    route: Option<&crate::RouteResult>,
) -> String {
    if !route.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::ContentExcerptWithSummary
    }) {
        return answer.trim().to_string();
    }
    let answer = answer.trim();
    let Some(excerpt) = latest_bounded_read_range_direct_answer(loop_state, prefer_english) else {
        return answer.to_string();
    };
    if answer_contains_observed_excerpt(answer, &excerpt) {
        answer.to_string()
    } else if answer.is_empty() {
        excerpt
    } else {
        format!("{}\n\n{}", excerpt.trim(), answer)
    }
}

fn compare_paths_observed_candidate(body: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if value.get("action").and_then(|v| v.as_str()) != Some("compare_paths") {
        return None;
    }
    let left = value.get("left")?;
    let right = value.get("right")?;
    let left_path = left
        .get("resolved_path")
        .or_else(|| left.get("path"))
        .and_then(|v| v.as_str())
        .unwrap_or("left");
    let right_path = right
        .get("resolved_path")
        .or_else(|| right.get("path"))
        .and_then(|v| v.as_str())
        .unwrap_or("right");
    let left_size = left
        .get("size_bytes")
        .and_then(|v| v.as_u64())
        .map(|size| size.to_string())
        .unwrap_or_else(|| "-".to_string());
    let right_size = right
        .get("size_bytes")
        .and_then(|v| v.as_u64())
        .map(|size| size.to_string())
        .unwrap_or_else(|| "-".to_string());
    let comparison = value.get("comparison").and_then(|v| v.as_object());
    let same_size = comparison
        .and_then(|item| item.get("same_size"))
        .and_then(|v| v.as_bool())
        .map(|v| v.to_string())
        .unwrap_or_else(|| "-".to_string());
    let size_delta = comparison
        .and_then(|item| item.get("size_delta_bytes"))
        .and_then(|v| v.as_i64())
        .map(|v| v.to_string())
        .unwrap_or_else(|| "-".to_string());
    Some(format!(
        "compare_paths left={left_path} left_size_bytes={left_size} right={right_path} right_size_bytes={right_size} same_size={same_size} size_delta_bytes={size_delta}"
    ))
}

fn observed_path_label(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn path_batch_facts_observed_candidate(value: &serde_json::Value) -> Option<String> {
    if value.get("action").and_then(|v| v.as_str()) != Some("path_batch_facts") {
        return None;
    }
    let facts = value.get("facts")?.as_array()?;
    if facts.is_empty() {
        return None;
    }
    let lines = facts
        .iter()
        .filter_map(|entry| {
            let entry = entry.as_object()?;
            let exists = entry
                .get("exists")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            let fact = entry.get("fact").and_then(|value| value.as_object());
            let path = path_batch_fact_preferred_path(entry).unwrap_or("-");
            let label = observed_path_label(path);
            let kind = fact
                .and_then(|item| item.get("kind"))
                .and_then(|value| value.as_str())
                .unwrap_or("-");
            let size = fact
                .and_then(|item| item.get("size_bytes"))
                .and_then(|value| value.as_u64())
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string());
            let modified = fact
                .and_then(|item| item.get("modified_ts"))
                .and_then(|value| value.as_i64())
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string());
            Some(format!(
                "path_fact name={label} path={path} exists={exists} kind={kind} size_bytes={size} modified_ts={modified}"
            ))
        })
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| format!("path_batch_facts\n{}", lines.join("\n")))
}

fn read_range_observed_candidate(value: &serde_json::Value) -> Option<String> {
    let excerpt = value
        .get("excerpt")
        .and_then(|v| v.as_str())
        .and_then(normalize_read_range_excerpt)?;
    let path = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    Some(match path {
        Some(path) => format!("read_range path={path}\n{excerpt}"),
        None => excerpt,
    })
}

fn compact_log_analyze_excerpt(value: &serde_json::Value) -> Option<String> {
    let path = value
        .get("path")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    let keyword_counts = value
        .get("keyword_counts")
        .and_then(|v| v.as_object())
        .map(|map| {
            let mut pairs = map
                .iter()
                .filter_map(|(key, count)| count.as_u64().map(|count| (key.as_str(), count)))
                .collect::<Vec<_>>();
            pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
            pairs
                .into_iter()
                .map(|(key, count)| format!("{key}={count}"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let recent_matches = value
        .get("recent_matches")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .take(8)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let level_counts = value
        .get("level_counts")
        .and_then(|v| v.as_object())
        .map(|map| {
            let mut pairs = map
                .iter()
                .filter_map(|(key, count)| count.as_u64().map(|count| (key.as_str(), count)))
                .collect::<Vec<_>>();
            pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
            pairs
                .into_iter()
                .map(|(key, count)| format!("{key}={count}"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let recent_notable_lines = value
        .get("recent_notable_lines")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .take(8)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let total_lines = value
        .get("total_lines")
        .and_then(|v| v.as_u64())
        .unwrap_or_default();

    let mut sections = vec![format!("log_analyze path={path} total_lines={total_lines}")];
    if !level_counts.is_empty() {
        sections.push(format!("level_counts: {}", level_counts.join(", ")));
    }
    if !keyword_counts.is_empty() {
        sections.push(format!("keyword_counts: {}", keyword_counts.join(", ")));
    }
    if !recent_notable_lines.is_empty() {
        sections.push(format!(
            "recent_notable_lines:\n- {}",
            recent_notable_lines.join("\n- ")
        ));
    }
    if !recent_matches.is_empty() {
        sections.push(format!(
            "recent_matches:\n- {}",
            recent_matches.join("\n- ")
        ));
    }
    Some(sections.join("\n"))
}

fn archive_list_summary_from_body(body: &str) -> Option<ArchiveListSummary> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| archive_list_summary_from_value(&value))
        .or_else(|| archive_list_summary_from_raw_output(body, None))
}

fn archive_read_direct_answer_candidate(body: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if value.get("action").and_then(|value| value.as_str()) != Some("read") {
        return None;
    }
    value
        .get("content")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|content| !content.is_empty())
        .map(ToString::to_string)
}

fn archive_list_summary_from_value(value: &serde_json::Value) -> Option<ArchiveListSummary> {
    if value.get("action").and_then(|v| v.as_str())? != "list" {
        return None;
    }
    let archive = value
        .get("archive")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string);
    if let Some(entries) = archive_entries_from_value_array(value.get("entries")) {
        if !entries.is_empty() {
            return Some(ArchiveListSummary { archive, entries });
        }
    }
    value
        .get("output")
        .and_then(|v| v.as_str())
        .and_then(|output| archive_list_summary_from_raw_output(output, archive))
}

fn archive_entries_from_value_array(
    value: Option<&serde_json::Value>,
) -> Option<Vec<ArchiveListEntry>> {
    let entries = value?.as_array()?;
    Some(
        entries
            .iter()
            .filter_map(|entry| {
                let name = entry
                    .get("name")
                    .or_else(|| entry.get("path"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|name| !name.is_empty())?;
                let size_bytes = entry.get("size_bytes").and_then(|v| v.as_u64());
                Some(ArchiveListEntry {
                    name: name.to_string(),
                    size_bytes,
                })
            })
            .collect(),
    )
}

fn archive_list_summary_from_raw_output(
    output: &str,
    archive_hint: Option<String>,
) -> Option<ArchiveListSummary> {
    let archive = archive_hint.or_else(|| archive_path_from_listing_header(output));
    let mut entries = output
        .lines()
        .filter_map(parse_zip_listing_entry_line)
        .collect::<Vec<_>>();
    if entries.is_empty() {
        entries = parse_plain_archive_listing_entries(output);
    }
    if entries.is_empty() {
        return None;
    }
    Some(ArchiveListSummary { archive, entries })
}

fn archive_path_from_listing_header(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        line.trim()
            .strip_prefix("Archive:")
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(ToString::to_string)
    })
}

fn parse_zip_listing_entry_line(line: &str) -> Option<ArchiveListEntry> {
    static ZIP_ENTRY_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let regex = ZIP_ENTRY_RE.get_or_init(|| {
        regex::Regex::new(r"^\s*(\d+)\s+\d{4}-\d{2}-\d{2}\s+\d{2}:\d{2}\s+(.+?)\s*$")
            .expect("valid zip listing entry regex")
    });
    let captures = regex.captures(line)?;
    let size_bytes = captures.get(1)?.as_str().parse::<u64>().ok();
    let name = captures.get(2)?.as_str().trim();
    (!name.is_empty()).then(|| ArchiveListEntry {
        name: name.to_string(),
        size_bytes,
    })
}

fn parse_plain_archive_listing_entries(output: &str) -> Vec<ArchiveListEntry> {
    if output
        .lines()
        .any(|line| line.trim_start().starts_with("Archive:"))
    {
        return Vec::new();
    }
    if output.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("adding:")
            || line.starts_with("updating:")
            || line.starts_with("freshening:")
    }) {
        return Vec::new();
    }
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| *line != "exit=0")
        .filter(|line| !line.starts_with("tar:"))
        .filter(|line| !line.starts_with("zip warning:"))
        .filter(|line| !line.chars().all(|ch| ch == '-'))
        .map(|name| ArchiveListEntry {
            name: name.to_string(),
            size_bytes: None,
        })
        .collect()
}

fn archive_entry_display(entry: &ArchiveListEntry, prefer_english: bool) -> String {
    match entry.size_bytes {
        Some(size) if prefer_english => format!("{} ({size} bytes)", entry.name),
        Some(size) => format!("{}（{size} 字节）", entry.name),
        None => entry.name.clone(),
    }
}

fn archive_list_summary_direct_answer(
    state: Option<&AppState>,
    summary: &ArchiveListSummary,
    prefer_english: bool,
) -> Option<String> {
    if summary.entries.is_empty() {
        return None;
    }
    let shown = summary
        .entries
        .iter()
        .take(8)
        .map(|entry| archive_entry_display(entry, prefer_english))
        .collect::<Vec<_>>();
    if shown.is_empty() {
        return None;
    }
    let omitted = summary.entries.len().saturating_sub(shown.len());
    let separator = if prefer_english { ", " } else { "、" };
    let entries = shown.join(separator);
    let count = summary.entries.len().to_string();
    let count_label = if prefer_english {
        if summary.entries.len() == 1 {
            "1 entry".to_string()
        } else {
            format!("{} entries", summary.entries.len())
        }
    } else {
        format!("{} 个条目", summary.entries.len())
    };
    let more = if omitted == 0 {
        String::new()
    } else if prefer_english {
        format!(", plus {omitted} more")
    } else {
        format!("，另有 {omitted} 个未列出")
    };
    Some(observed_t_with_vars(
        state,
        "clawd.msg.archive_list_summary",
        "压缩包包含 {count_label}：{entries}{more}。",
        "The archive contains {count_label}: {entries}{more}.",
        prefer_english,
        &[
            ("count", &count),
            ("count_label", &count_label),
            ("entries", &entries),
            ("more", &more),
        ],
    ))
}

fn archive_entry_existence_direct_answer(
    state: Option<&AppState>,
    route: &crate::RouteResult,
    request_text: Option<&str>,
    summary: &ArchiveListSummary,
    archive_hint: Option<&str>,
    prefer_english: bool,
) -> Option<String> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ExistenceWithPath {
        return None;
    }
    let archive_path = archive_hint
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or(summary.archive.as_deref())
        .or_else(|| {
            let hint = route.output_contract.locator_hint.trim();
            (!hint.is_empty()).then_some(hint)
        });
    let target = archive_entry_target_for_observed_route(route, request_text, archive_path)?;
    let exists = archive_list_contains_requested_entry(summary, &target)?;
    let vars = [("entry", target.as_str())];
    Some(if exists {
        observed_t_with_vars(
            state,
            "clawd.msg.archive_entry_exists",
            "压缩包中存在 {entry}。",
            "Yes, {entry} exists in the archive.",
            prefer_english,
            &vars,
        )
    } else {
        observed_t_with_vars(
            state,
            "clawd.msg.archive_entry_missing",
            "压缩包中不存在 {entry}。",
            "No, {entry} does not exist in the archive.",
            prefer_english,
            &vars,
        )
    })
}

fn archive_entry_target_for_observed_route(
    route: &crate::RouteResult,
    request_text: Option<&str>,
    archive_path: Option<&str>,
) -> Option<String> {
    let mut path_candidates = Vec::new();
    let mut filename_candidates = Vec::new();
    for text in request_text
        .into_iter()
        .chain(std::iter::once(route.resolved_intent.as_str()))
    {
        for locator in
            crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(text)
        {
            push_archive_entry_observed_candidate(
                &mut path_candidates,
                &mut filename_candidates,
                &locator.locator_hint,
                archive_path,
            );
        }
        for filename in crate::delivery_utils::extract_filename_candidates(text) {
            push_archive_entry_observed_candidate(
                &mut path_candidates,
                &mut filename_candidates,
                &filename,
                archive_path,
            );
        }
    }
    path_candidates
        .into_iter()
        .next()
        .or_else(|| filename_candidates.into_iter().next())
}

fn push_archive_entry_observed_candidate(
    path_candidates: &mut Vec<String>,
    filename_candidates: &mut Vec<String>,
    candidate: &str,
    archive_path: Option<&str>,
) {
    let Some(candidate) = normalize_archive_entry_observed_candidate(candidate, archive_path)
    else {
        return;
    };
    let target = if candidate.contains('/') || candidate.contains('\\') {
        path_candidates
    } else {
        filename_candidates
    };
    if !target
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&candidate))
    {
        target.push(candidate);
    }
}

fn normalize_archive_entry_observed_candidate(
    candidate: &str,
    archive_path: Option<&str>,
) -> Option<String> {
    let trimmed = candidate.trim().trim_matches(|ch: char| {
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
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains("://")
        || Path::new(trimmed).is_absolute()
        || archive_candidate_has_supported_extension(trimmed)
        || archive_path.is_some_and(|path| archive_candidate_matches_archive(trimmed, path))
        || crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(trimmed)
    {
        return None;
    }
    if !trimmed.contains('/')
        && !trimmed.contains('\\')
        && !archive_entry_observed_candidate_has_extension(trimmed)
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn archive_candidate_has_supported_extension(path: &str) -> bool {
    let lower = path.trim().to_ascii_lowercase();
    lower.ends_with(".zip") || lower.ends_with(".tar.gz") || lower.ends_with(".tgz")
}

fn archive_candidate_matches_archive(candidate: &str, archive_path: &str) -> bool {
    let candidate_norm = candidate.replace('\\', "/");
    let archive_norm = archive_path.trim().replace('\\', "/");
    if candidate_norm.eq_ignore_ascii_case(&archive_norm) {
        return true;
    }
    let archive_name = archive_norm
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(archive_norm.as_str());
    candidate_norm.eq_ignore_ascii_case(archive_name)
}

fn archive_entry_observed_candidate_has_extension(candidate: &str) -> bool {
    let basename = candidate
        .rsplit(|ch| ch == '/' || ch == '\\')
        .next()
        .unwrap_or(candidate);
    let Some((stem, ext)) = basename.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && (1..=16).contains(&ext.len())
        && ext.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn normalize_archive_entry_name(value: &str) -> String {
    value
        .trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
}

fn archive_list_contains_requested_entry(
    summary: &ArchiveListSummary,
    target: &str,
) -> Option<bool> {
    let target_norm = normalize_archive_entry_name(target);
    if target_norm.is_empty() {
        return None;
    }
    if summary
        .entries
        .iter()
        .any(|entry| normalize_archive_entry_name(&entry.name).eq_ignore_ascii_case(&target_norm))
    {
        return Some(true);
    }
    if target_norm.contains('/') {
        return Some(false);
    }
    let basename_matches = summary
        .entries
        .iter()
        .filter(|entry| {
            normalize_archive_entry_name(&entry.name)
                .rsplit('/')
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case(&target_norm))
        })
        .take(2)
        .count();
    match basename_matches {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

fn archive_list_summary_observed_candidate(summary: &ArchiveListSummary) -> Option<String> {
    if summary.entries.is_empty() {
        return None;
    }
    let archive = summary.archive.as_deref().unwrap_or("-");
    let mut lines = vec![format!(
        "archive_basic action=list archive={archive} total_entries={}",
        summary.entries.len()
    )];
    for entry in summary.entries.iter().take(32) {
        match entry.size_bytes {
            Some(size_bytes) => {
                lines.push(format!("entry name={} size_bytes={size_bytes}", entry.name))
            }
            None => lines.push(format!("entry name={}", entry.name)),
        }
    }
    if summary.entries.len() > 32 {
        lines.push(format!("entries_omitted={}", summary.entries.len() - 32));
    }
    Some(lines.join("\n"))
}

fn answer_is_raw_archive_listing_passthrough(answer: &str) -> bool {
    let trimmed = answer.trim();
    if trimmed.is_empty() || archive_list_summary_from_raw_output(trimmed, None).is_none() {
        return false;
    }
    trimmed
        .lines()
        .map(str::trim_start)
        .any(|line| line.starts_with("Archive:") || line.starts_with("Length"))
}

fn latest_archive_list_summary(loop_state: &LoopState) -> Option<ArchiveListSummary> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && step.skill == "archive_basic")
        .filter_map(|step| step.output.as_deref())
        .find_map(archive_list_summary_from_body)
}

fn archive_list_raw_passthrough_replacement(
    answer: &str,
    state: &AppState,
    loop_state: &LoopState,
    request_language_hint: &str,
) -> Option<String> {
    if !answer_is_raw_archive_listing_passthrough(answer) {
        return None;
    }
    let summary = latest_archive_list_summary(loop_state)?;
    archive_list_summary_direct_answer(
        Some(state),
        &summary,
        observed_request_prefers_english_template(Some(state), request_language_hint),
    )
}

fn archive_basic_observed_candidate(value: &serde_json::Value) -> Option<String> {
    if let Some(summary) = archive_list_summary_from_value(value) {
        return archive_list_summary_observed_candidate(&summary);
    }
    let action = value.get("action").and_then(|v| v.as_str())?.trim();
    if action.is_empty() {
        return None;
    }
    let archive = value
        .get("archive")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("-");
    let output = value
        .get("output")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("ok");
    Some(format!(
        "archive_basic action={action} archive={archive}\n{output}"
    ))
}

fn db_basic_observed_candidate(value: &serde_json::Value) -> Option<String> {
    if let Some(table_names) = db_basic_table_names(value) {
        if table_names.is_empty() {
            return Some("db_tables=<empty>".to_string());
        }
        return Some(format!("db_tables={}", table_names.join(", ")));
    }
    db_basic_scalar_candidate(value).map(|text| format!("db_scalar={text}"))
}

fn count_inventory_observed_candidate(value: &serde_json::Value) -> Option<String> {
    if value.get("action").and_then(|v| v.as_str()) != Some("count_inventory") {
        return None;
    }
    let counts = value.get("counts")?;
    let mut lines = vec!["action=count_inventory".to_string()];
    for key in ["path", "resolved_path", "kind_filter"] {
        if let Some(text) = value
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            lines.push(format!("{key}={text}"));
        }
    }
    for key in ["files", "dirs", "total", "hidden", "total_size_bytes"] {
        if let Some(text) = counts.get(key).and_then(value_scalar_text) {
            lines.push(format!("count_{key}={text}"));
        }
    }
    if let Some(by_extension) = counts.get("by_extension").and_then(|v| v.as_object()) {
        let mut entries = by_extension
            .iter()
            .filter_map(|(ext, count)| {
                value_scalar_text(count).map(|count| format!("{ext}:{count}"))
            })
            .collect::<Vec<_>>();
        entries.sort();
        if !entries.is_empty() {
            lines.push(format!("count_by_extension={}", entries.join(", ")));
        }
    }
    (lines.len() > 1).then(|| lines.join("\n"))
}

fn validate_structured_observed_candidate(value: &serde_json::Value) -> Option<String> {
    if value.get("action").and_then(|v| v.as_str()) != Some("validate_structured") {
        return None;
    }
    let valid = value.get("valid")?.as_bool()?;
    let format = value
        .get("format")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("structured");
    Some(format!("validate_structured format={format} valid={valid}"))
}

fn structured_observed_body(skill: &str, body: &str) -> Option<String> {
    if skill == "archive_basic" {
        return archive_list_summary_from_body(body)
            .and_then(|summary| archive_list_summary_observed_candidate(&summary))
            .or_else(|| {
                serde_json::from_str::<serde_json::Value>(body)
                    .ok()
                    .and_then(|value| archive_basic_observed_candidate(&value))
            });
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    match skill {
        "system_basic" => {
            let action = value.get("action").and_then(|v| v.as_str())?;
            match action {
                "read_range" => read_range_observed_candidate(&value),
                "inventory_dir" => inventory_dir_observed_candidate(&value),
                "count_inventory" => count_inventory_observed_candidate(&value),
                "tree_summary" => tree_summary_direct_answer_candidate(None, &value, true),
                "dir_compare" => dir_compare_direct_answer_candidate(None, &value, true),
                "validate_structured" => validate_structured_observed_candidate(&value),
                "compare_paths" => compare_paths_observed_candidate(body),
                "path_batch_facts" => path_batch_facts_observed_candidate(&value),
                _ => None,
            }
        }
        "config_basic" => validate_structured_observed_candidate(&value),
        "db_basic" => db_basic_observed_candidate(&value),
        "service_control" => service_control_summary_candidate(&value),
        "fs_search" | "fs_basic" => {
            if skill == "fs_basic" {
                match value.get("action").and_then(|v| v.as_str()) {
                    Some("inventory_dir") => return inventory_dir_observed_candidate(&value),
                    Some("read_range") => return read_range_observed_candidate(&value),
                    Some("count_inventory") => return count_inventory_observed_candidate(&value),
                    Some("path_batch_facts") => return path_batch_facts_observed_candidate(&value),
                    _ => {}
                }
            }
            fs_search_grep_text_observed_candidate(&value).or_else(|| {
                fs_search_direct_answer_candidate(None, &value, None, false, true, false)
            })
        }
        "log_analyze" => compact_log_analyze_excerpt(&value),
        "package_manager" => package_manager_summary_candidate(
            None,
            body,
            Some(crate::OutputResponseShape::OneSentence),
            true,
            false,
        ),
        _ => None,
    }
}

fn extract_direct_scalar_from_generic_output_with_locator_hint_impl(
    state: Option<&AppState>,
    route: Option<&crate::RouteResult>,
    loop_state: &LoopState,
    locator_hint: Option<&str>,
    auto_locator_path: Option<&str>,
    prefer_full_path: bool,
    allow_localized_direct_template: bool,
    prefer_english: bool,
) -> Option<String> {
    if let Some(path) = recent_file_path_candidate_for_scalar_path(loop_state, route) {
        return matrix_checked_direct_candidate(route, loop_state, auto_locator_path, path);
    }
    if let Some(answer) = latest_successful_list_dir_answer_candidate(
        loop_state,
        Some(crate::OutputResponseShape::Scalar),
        auto_locator_path,
        prefer_full_path,
    ) {
        if !crate::finalize::looks_like_planner_artifact(&answer)
            && !crate::finalize::looks_like_internal_trace_artifact(&answer)
        {
            return matrix_checked_direct_candidate(route, loop_state, auto_locator_path, answer);
        }
    }
    let observed_output = extract_latest_generic_successful_output(loop_state)?;
    if route_should_synthesize_non_bilingual_existence_with_path(
        route,
        allow_localized_direct_template,
    ) {
        return None;
    }
    let answer = structured_scalar_candidate(
        state,
        route,
        &observed_output.skill,
        &observed_output.body,
        locator_hint.filter(|hint| !hint.trim().is_empty()),
        auto_locator_path,
        prefer_full_path,
        allow_localized_direct_template,
        prefer_english,
    )
    .or_else(|| {
        allows_normalized_scalar_direct_fallback(
            &observed_output.skill,
            route.map(|route| route.output_contract.response_shape),
        )
        .then(|| normalized_scalar_candidate(&observed_output.body))
        .flatten()
    })?;
    if crate::finalize::looks_like_planner_artifact(&answer)
        || crate::finalize::looks_like_internal_trace_artifact(&answer)
    {
        return None;
    }
    matrix_checked_direct_candidate(route, loop_state, auto_locator_path, answer)
}

fn matrix_checked_direct_candidate(
    route: Option<&crate::RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    answer: String,
) -> Option<String> {
    let Some(route) = route else {
        return Some(answer);
    };
    if latest_observation_is_explicitly_forbidden_by_contract(route, loop_state) {
        return None;
    }
    if route_requires_matrix_grounded_direct_candidate(route)
        && matrix_direct_candidate_satisfies_contract(route, loop_state, auto_locator_path, &answer)
    {
        return Some(answer);
    }
    if hidden_entries_empty_direct_candidate_satisfies_contract(route, loop_state, &answer) {
        return Some(answer);
    }
    if route_requires_matrix_grounded_direct_candidate(route) {
        return None;
    }
    Some(answer)
}

fn hidden_entries_empty_direct_candidate_satisfies_contract(
    route: &crate::RouteResult,
    loop_state: &LoopState,
    answer: &str,
) -> bool {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::HiddenEntriesCheck
        || route.output_contract.response_shape != crate::OutputResponseShape::Strict
        || answer.trim().is_empty()
        || crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
    {
        return false;
    }
    latest_hidden_entries(loop_state)
        .or_else(|| {
            latest_directory_listing_entries(loop_state, None)
                .map(|entries| hidden_entries_from_entries(&entries))
        })
        .is_some_and(|hidden_entries| hidden_entries.is_empty())
}

fn route_requires_matrix_grounded_direct_candidate(route: &crate::RouteResult) -> bool {
    crate::contract_matrix::final_answer_shape_for_output_contract(&route.output_contract)
        .is_some_and(|shape| !shape.allows_model_language())
}

fn matrix_direct_candidate_satisfies_contract(
    route: &crate::RouteResult,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    candidate: &str,
) -> bool {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "observed-output-direct-candidate",
        "ask",
        route.resolved_intent.as_str(),
    );
    journal.record_route_result(route);
    for step in &loop_state.executed_step_results {
        journal.push_step_result(step);
    }
    if let Some(path) = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "auto_locator_path".to_string(),
            skill: "auto_locator".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(
                serde_json::json!({
                    "action": "auto_locator",
                    "path": path,
                    "resolved_path": path,
                })
                .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
    }
    crate::answer_verifier::structurally_satisfies_answer_contract(route, &journal, candidate)
}

fn latest_observation_is_explicitly_forbidden_by_contract(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> bool {
    if !route_uses_enforced_generic_path_content_profile(route) {
        return false;
    }
    let Some(step) = loop_state.executed_step_results.iter().rev().find(|step| {
        step.is_ok()
            && !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think" | "answer_verifier"
            )
    }) else {
        return false;
    };
    let args = step
        .output
        .as_deref()
        .and_then(|body| serde_json::from_str::<serde_json::Value>(body.trim()).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    crate::contract_matrix::action_policy_for_output_contract(
        Some(&route.output_contract),
        &step.skill,
        &args,
    )
    .is_some_and(|policy| {
        policy.decision == crate::contract_matrix::ActionPolicyDecision::RejectedForbidden
    })
}

fn route_uses_enforced_generic_path_content_profile(route: &crate::RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::None
        && route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && route.output_contract.response_shape == crate::OutputResponseShape::Free
        && matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
        && !route_allows_strict_plain_observation_passthrough(route)
}

#[cfg(test)]
pub(crate) fn extract_direct_scalar_from_generic_output_with_locator_hint(
    loop_state: &LoopState,
    locator_hint: Option<&str>,
    auto_locator_path: Option<&str>,
    prefer_full_path: bool,
) -> Option<String> {
    extract_direct_scalar_from_generic_output_with_locator_hint_impl(
        None,
        None,
        loop_state,
        locator_hint,
        auto_locator_path,
        prefer_full_path,
        true,
        false,
    )
}

pub(crate) fn extract_direct_scalar_from_generic_output(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let auto_locator_path = agent_run_context
        .and_then(|ctx| ctx.auto_locator_path.as_deref())
        .filter(|path| !path.trim().is_empty());
    let prefer_full_path = route.is_some_and(route_requests_scalar_path_only);
    let request_language_hint = current_turn_request_text(route, agent_run_context)
        .map(observed_request_language_hint)
        .unwrap_or("config_default");
    let allow_localized_direct_template =
        observed_language_supports_bilingual_template(request_language_hint);
    if route_should_synthesize_non_bilingual_existence_with_path(
        route,
        allow_localized_direct_template,
    ) {
        return None;
    }
    if let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) {
        if let Some(answer) =
            structured_scalar_equality_direct_answer(None, route, loop_state, agent_run_context)
        {
            return matrix_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
        if route_needs_structured_scalar_pair_synthesis(loop_state, agent_run_context) {
            return None;
        }
        if let Some(answer) =
            count_inventory_planned_file_dir_breakdown_answer(None, loop_state, false)
        {
            return matrix_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
        if let Some(answer) = count_answer_from_latest_listing(route, loop_state) {
            return matrix_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
        if let Some(answer) = count_answer_from_latest_fs_search(route, loop_state) {
            return matrix_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
    }
    let locator_hint = route.map(|route| route.output_contract.locator_hint.as_str());
    extract_direct_scalar_from_generic_output_with_locator_hint_impl(
        None,
        route,
        loop_state,
        locator_hint,
        auto_locator_path,
        prefer_full_path,
        allow_localized_direct_template,
        false,
    )
}

pub(crate) fn extract_direct_scalar_from_generic_output_i18n(
    loop_state: &LoopState,
    state: &AppState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let auto_locator_path = agent_run_context
        .and_then(|ctx| ctx.auto_locator_path.as_deref())
        .filter(|path| !path.trim().is_empty());
    let prefer_full_path = route.is_some_and(route_requests_scalar_path_only);
    let request_language_hint = current_turn_request_text(route, agent_run_context)
        .map(crate::language_policy::request_language_hint)
        .unwrap_or("config_default");
    let prefer_english =
        observed_request_prefers_english_template(Some(state), request_language_hint);
    let allow_localized_direct_template =
        observed_language_supports_bilingual_template(request_language_hint);
    if let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) {
        if let Some(answer) = structured_scalar_equality_direct_answer(
            Some(state),
            route,
            loop_state,
            agent_run_context,
        ) {
            return matrix_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
        if route_needs_structured_scalar_pair_synthesis(loop_state, agent_run_context) {
            return None;
        }
        if let Some(answer) = count_inventory_planned_file_dir_breakdown_answer(
            Some(state),
            loop_state,
            prefer_english,
        ) {
            return matrix_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
        if let Some(answer) = count_answer_from_latest_listing(route, loop_state) {
            return matrix_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
        if let Some(answer) = count_answer_from_latest_fs_search(route, loop_state) {
            return matrix_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
    }
    let locator_hint = route.map(|route| route.output_contract.locator_hint.as_str());
    extract_direct_scalar_from_generic_output_with_locator_hint_impl(
        Some(state),
        route,
        loop_state,
        locator_hint,
        auto_locator_path,
        prefer_full_path,
        allow_localized_direct_template,
        prefer_english,
    )
}

fn extract_direct_answer_from_generic_output_impl(
    state: Option<&AppState>,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let response_shape = route.map(|route| route.output_contract.response_shape);
    let is_plain_act = route.is_some_and(|route| route.ask_mode.is_plain_act());
    let locator_hint = route
        .map(|route| route.output_contract.locator_hint.as_str())
        .filter(|hint| !hint.trim().is_empty());
    let auto_locator_path = agent_run_context
        .and_then(|ctx| ctx.auto_locator_path.as_deref())
        .filter(|path| !path.trim().is_empty());
    let request_language_hint = current_turn_request_text(route, agent_run_context)
        .map(observed_request_language_hint)
        .unwrap_or("config_default");
    let allow_localized_direct_template =
        observed_language_supports_bilingual_template(request_language_hint);
    let prefers_english_free_text =
        observed_request_prefers_english_template(state, request_language_hint);
    let prefers_english_presence_answer = route.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::ExistenceWithPath
            && prefers_english_free_text
    });
    let existence_with_path_should_use_llm_synthesis =
        route_should_synthesize_non_bilingual_existence_with_path(
            route,
            allow_localized_direct_template,
        );
    let hidden_entries_should_use_llm_synthesis = route.is_some_and(|route| {
        route_requests_hidden_entries_check(route)
            && route.output_contract.response_shape != crate::OutputResponseShape::Scalar
    });
    let allow_raw_listing_direct_answer = route_allows_raw_listing_direct_answer(route)
        && !existence_with_path_should_use_llm_synthesis
        && !hidden_entries_should_use_llm_synthesis;
    let health_check_prefers_raw_payload = is_plain_act
        && route.is_some_and(|route| {
            route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
        })
        && !matches!(
            response_shape,
            Some(crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Scalar)
        );
    let health_check_service_status_direct_allowed = route.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::ServiceStatus
    });
    if has_successful_step_for_skill(loop_state, "health_check")
        && !health_check_prefers_raw_payload
        && !health_check_service_status_direct_allowed
        && matches!(
            response_shape,
            Some(crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Scalar)
        )
    {
        return None;
    }
    let prefer_full_path = route.is_some_and(route_prefers_plain_fs_search_paths);

    if let Some(route) = route {
        if let Some(answer) =
            structured_scalar_equality_direct_answer(state, route, loop_state, agent_run_context)
        {
            return matrix_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
        if let Some(answer) = latest_git_repository_state_direct_answer(
            state,
            route,
            loop_state,
            response_shape,
            allow_localized_direct_template,
            prefers_english_free_text,
        ) {
            return matrix_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
        if let Some(answer) =
            hidden_entries_direct_answer(state, route, loop_state, prefers_english_free_text)
        {
            if latest_observation_is_explicitly_forbidden_by_contract(route, loop_state) {
                return None;
            }
            return matrix_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
        if let Some(answer) = db_basic_database_kind_judgment_from_loop_state_candidate(
            route,
            loop_state,
            current_turn_request_text(Some(route), agent_run_context),
            prefers_english_free_text,
        ) {
            return matrix_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
    }

    let answer = allow_raw_listing_direct_answer
        .then(|| {
            latest_successful_list_dir_answer_candidate(
                loop_state,
                response_shape,
                auto_locator_path,
                prefer_full_path,
            )
        })
        .flatten()
        .or_else(|| {
            let observed_output = extract_latest_generic_successful_output(loop_state)?;
            if observed_output.skill == "run_cmd" {
                route
                    .and_then(|route| {
                        run_cmd_sqlite_direct_answer_candidate(
                            route,
                            &observed_output.body,
                            current_turn_request_text(Some(route), agent_run_context),
                            prefers_english_free_text,
                        )
                    })
                    .or_else(|| {
                        (!existence_with_path_should_use_llm_synthesis)
                            .then(|| {
                                run_cmd_presence_with_path_candidate(
                                    state,
                                    &observed_output.body,
                                    locator_hint,
                                    auto_locator_path,
                                    prefers_english_presence_answer,
                                )
                            })
                            .flatten()
                    })
                    .or_else(|| {
                        (allow_raw_listing_direct_answer
                            && !existence_with_path_should_use_llm_synthesis)
                            .then(|| {
                                route
                                    .and_then(|route| {
                                        run_cmd_semantic_listing_text_candidate(
                                            route,
                                            &observed_output.body,
                                        )
                                    })
                                    .or_else(|| {
                                        run_cmd_listing_text_candidate(
                                            &observed_output.body,
                                            auto_locator_path,
                                        )
                                    })
                            })
                            .flatten()
                    })
                    .or_else(|| {
                        route
                            .filter(|route| {
                                route_allows_strict_plain_observation_passthrough(route)
                            })
                            .and_then(|_| {
                                strict_plain_observation_passthrough_candidate(
                                    &observed_output.body,
                                )
                            })
                    })
            } else {
                None
            }
            .or_else(|| match observed_output.skill.as_str() {
                "health_check" => serde_json::from_str::<serde_json::Value>(&observed_output.body)
                    .ok()
                    .and_then(|value| {
                        route
                            .is_some_and(|route| {
                                route.output_contract.semantic_kind
                                    == crate::OutputSemanticKind::ServiceStatus
                            })
                            .then(|| {
                                health_check_service_status_direct_answer_candidate(
                                    state,
                                    &value,
                                    response_shape,
                                    prefers_english_free_text,
                                )
                            })
                            .flatten()
                    })
                    .or_else(|| {
                        health_check_prefers_raw_payload.then_some(observed_output.body.clone())
                    }),
                "http_basic" => None,
                "process_basic" => route
                    .is_some_and(|route| {
                        route.output_contract.semantic_kind
                            == crate::OutputSemanticKind::ServiceStatus
                    })
                    .then(|| {
                        process_basic_service_status_direct_answer_candidate(
                            state,
                            &observed_output.body,
                            response_shape,
                            prefers_english_free_text,
                        )
                    })
                    .flatten(),
                "service_control" => {
                    serde_json::from_str::<serde_json::Value>(&observed_output.body)
                        .ok()
                        .and_then(|value| {
                            route
                                .is_some_and(|route| {
                                    route.output_contract.semantic_kind
                                        == crate::OutputSemanticKind::ServiceStatus
                                        || route.output_contract.response_shape
                                            == crate::OutputResponseShape::Scalar
                                })
                                .then(|| {
                                    service_control_status_direct_answer_candidate(
                                        state,
                                        &value,
                                        response_shape,
                                        prefers_english_free_text,
                                    )
                                })
                                .flatten()
                        })
                }
                "fs_search" => serde_json::from_str::<serde_json::Value>(&observed_output.body)
                    .ok()
                    .and_then(|value| {
                        fs_search_output_direct_answer_candidate(
                            state,
                            route,
                            &value,
                            locator_hint,
                            prefers_english_free_text,
                            allow_raw_listing_direct_answer,
                            prefer_full_path,
                        )
                    }),
                "git_basic" => {
                    let branch = latest_git_current_branch(loop_state);
                    git_basic_direct_answer_candidate(
                        state,
                        route,
                        &observed_output.body,
                        branch.as_deref(),
                        response_shape,
                        allow_localized_direct_template,
                        prefers_english_free_text,
                    )
                }
                "doc_parse" => route
                    .and_then(|route| {
                        doc_parse_content_presence_direct_answer_candidate(
                            state,
                            route,
                            &observed_output.body,
                            current_turn_request_text(Some(route), agent_run_context),
                            auto_locator_path.or(locator_hint),
                            prefers_english_free_text,
                        )
                    })
                    .or_else(|| {
                        content_excerpt_summary_direct_answer_candidate(
                            route,
                            &observed_output.body,
                        )
                        .filter(|candidate| {
                            !direct_free_text_conflicts_with_request_language(
                                candidate,
                                request_language_hint,
                            )
                        })
                    }),
                "db_basic" => route.and_then(|route| {
                    db_basic_database_kind_judgment_candidate(
                        route,
                        &observed_output.body,
                        current_turn_request_text(Some(route), agent_run_context),
                        prefers_english_free_text,
                    )
                    .or_else(|| {
                        db_basic_tables_summary_candidate(
                            state,
                            route,
                            &observed_output.body,
                            prefers_english_free_text,
                        )
                    })
                }),
                "transform" => transform_skill_formatted_output_candidate(&observed_output.body),
                "package_manager" => package_manager_summary_candidate(
                    state,
                    &observed_output.body,
                    response_shape,
                    allow_localized_direct_template,
                    prefers_english_free_text,
                ),
                "archive_basic" => {
                    if let Some(answer) = archive_unpack_direct_answer_candidate(
                        route,
                        &observed_output.body,
                        prefers_english_free_text,
                    ) {
                        Some(answer)
                    } else if let Some(answer) =
                        archive_read_direct_answer_candidate(&observed_output.body)
                    {
                        Some(answer)
                    } else {
                        archive_list_summary_from_body(&observed_output.body).and_then(|summary| {
                            route.and_then(|route| {
                                archive_entry_existence_direct_answer(
                                    state,
                                    route,
                                    current_turn_request_text(Some(route), agent_run_context),
                                    &summary,
                                    auto_locator_path.or(locator_hint),
                                    prefers_english_presence_answer,
                                )
                            })
                        })
                    }
                }
                "log_analyze" => None,
                "system_basic" | "config_basic" | "fs_basic" => {
                    let value = serde_json::from_str::<serde_json::Value>(&observed_output.body)
                        .ok()
                        .or_else(|| {
                            system_basic_info_value("system_basic", &observed_output.body)
                        })?;
                    let action = value.get("action").and_then(|v| v.as_str());
                    if observed_output.skill == "fs_basic" {
                        if let Some(answer) = fs_search_output_direct_answer_candidate(
                            state,
                            route,
                            &value,
                            locator_hint,
                            prefers_english_free_text,
                            allow_raw_listing_direct_answer,
                            prefer_full_path,
                        ) {
                            return Some(answer);
                        }
                    }
                    if action == Some("read_range")
                        && (route_allows_tail_read_range_direct_passthrough(
                            route,
                            response_shape,
                            &value,
                        ) || route_allows_read_range_direct_passthrough(route, response_shape))
                    {
                        value
                            .get("excerpt")
                            .and_then(|v| v.as_str())
                            .and_then(|excerpt| {
                                normalize_read_range_excerpt_for_direct_answer(
                                    state,
                                    excerpt,
                                    prefers_english_free_text,
                                    read_range_preserve_blank_lines(&value),
                                )
                            })
                            .filter(|candidate| {
                                !read_range_direct_candidate_conflicts_with_request_language(
                                    candidate,
                                    request_language_hint,
                                )
                            })
                    } else if action == Some("inventory_dir")
                        && (is_plain_act
                            || route.is_some_and(|route| {
                                route.output_contract.semantic_kind
                                    == crate::OutputSemanticKind::DirectoryEntryGroups
                            }))
                        && allow_raw_listing_direct_answer
                    {
                        inventory_dir_direct_answer_candidate(
                            state,
                            route,
                            &value,
                            prefers_english_free_text,
                        )
                    } else if action == Some("tree_summary") {
                        if route.is_some_and(|route| {
                            route.output_contract.semantic_kind
                                == crate::OutputSemanticKind::DirectoryPurposeSummary
                        }) {
                            None
                        } else {
                            tree_summary_direct_answer_candidate(
                                state,
                                &value,
                                prefers_english_free_text,
                            )
                        }
                    } else if action == Some("dir_compare") {
                        dir_compare_direct_answer_candidate(
                            state,
                            &value,
                            prefers_english_free_text,
                        )
                    } else if action == Some("count_inventory") {
                        count_inventory_direct_answer_candidate(
                            state,
                            &value,
                            response_shape,
                            prefers_english_free_text,
                        )
                    } else if matches!(action, Some("extract_field" | "read_field")) {
                        extract_field_direct_answer_candidate(
                            state,
                            &value,
                            response_shape,
                            prefers_english_free_text,
                            allow_localized_direct_template,
                        )
                    } else if matches!(action, Some("extract_fields" | "read_fields")) {
                        extract_fields_direct_answer_candidate(
                            state,
                            &value,
                            response_shape,
                            prefers_english_free_text,
                            allow_localized_direct_template,
                        )
                    } else if action == Some("structured_keys") {
                        structured_keys_direct_answer_candidate(
                            state,
                            &value,
                            current_turn_request_text(route, agent_run_context),
                            response_shape,
                            prefers_english_free_text,
                        )
                    } else if action == Some("validate_structured") {
                        validate_structured_direct_answer_candidate(
                            state,
                            &value,
                            prefers_english_free_text,
                        )
                    } else if action == Some("info")
                        || (action.is_none() && system_basic_value_looks_like_info(&value))
                    {
                        if route.is_some_and(route_requests_scalar_path_only) {
                            system_basic_info_scalar_path_candidate(&value)
                        } else {
                            None
                        }
                    } else if action == Some("path_batch_facts")
                        && route.is_some_and(route_requires_single_file_delivery)
                    {
                        path_batch_file_delivery_token_candidate(route, &value)
                    } else if action == Some("path_batch_facts")
                        && route.is_some_and(|route| {
                            route_allows_path_batch_scalar_path_observed_answer(route)
                                || route_scalar_has_plain_path_terminal_respond(route, loop_state)
                        })
                    {
                        system_basic_path_batch_scalar_path_candidate(&value)
                    } else if action == Some("path_batch_facts")
                        && route.is_some_and(route_requests_scalar_existence)
                    {
                        system_basic_scalar_existence_candidate(
                            state,
                            &value,
                            prefers_english_presence_answer,
                        )
                    } else if action == Some("path_batch_facts")
                        && !existence_with_path_should_use_llm_synthesis
                        && route.is_some_and(route_prefers_path_kind_fact_answer)
                    {
                        path_batch_fact_path_kind_candidate(&value, prefers_english_free_text)
                            .or_else(|| {
                                (!existence_with_path_should_use_llm_synthesis
                                    && route.is_some_and(|route| {
                                        route.output_contract.semantic_kind
                                            == crate::OutputSemanticKind::ExistenceWithPath
                                    }))
                                .then(|| {
                                    system_basic_existence_with_path_candidate(
                                        state,
                                        &value,
                                        locator_hint,
                                        auto_locator_path,
                                        prefers_english_presence_answer,
                                    )
                                })
                                .flatten()
                            })
                    } else if !existence_with_path_should_use_llm_synthesis
                        && route.is_some_and(|route| {
                            route.output_contract.semantic_kind
                                == crate::OutputSemanticKind::ExistenceWithPath
                        })
                    {
                        system_basic_existence_with_path_candidate(
                            state,
                            &value,
                            locator_hint,
                            auto_locator_path,
                            prefers_english_presence_answer,
                        )
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .or_else(|| {
                structured_scalar_candidate(
                    state,
                    route,
                    &observed_output.skill,
                    &observed_output.body,
                    locator_hint,
                    auto_locator_path,
                    prefer_full_path,
                    allow_localized_direct_template,
                    prefers_english_free_text,
                )
            })
            .or_else(|| {
                (!existence_with_path_should_use_llm_synthesis
                    && allows_normalized_scalar_direct_fallback(
                        &observed_output.skill,
                        response_shape,
                    ))
                .then(|| normalized_scalar_candidate(&observed_output.body))
                .flatten()
            })
        })?;
    if crate::finalize::looks_like_planner_artifact(&answer)
        || crate::finalize::looks_like_internal_trace_artifact(&answer)
    {
        return None;
    }
    matrix_checked_direct_candidate(route, loop_state, auto_locator_path, answer)
}

fn archive_unpack_direct_answer_candidate(
    route: Option<&crate::RouteResult>,
    body: &str,
    prefer_english: bool,
) -> Option<String> {
    let route = route?;
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ArchiveUnpack {
        return None;
    }
    let dest =
        archive_basic_path_value_from_body(body, &["dest", "dest_path", "destination", "path"])?;
    let members = archive_unpack_members_from_body(body, &dest);
    if !members.is_empty() {
        let joined = if prefer_english {
            members.join(", ")
        } else {
            members.join("、")
        };
        return if prefer_english {
            Some(format!("Unpacked to {dest}; extracted {joined}."))
        } else {
            Some(format!("已解压到 {dest}，包含 {joined}。"))
        };
    }
    if prefer_english {
        Some(format!("Unpacked to {dest}."))
    } else {
        Some(format!("已解压到 {dest}。"))
    }
}

fn archive_unpack_members_from_body(body: &str, dest: &str) -> Vec<String> {
    let dest_path = Path::new(dest);
    let mut members = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        let Some((prefix, raw_path)) = line.split_once(':') else {
            continue;
        };
        if !matches!(
            prefix.trim().to_ascii_lowercase().as_str(),
            "inflating" | "extracting" | "creating"
        ) {
            continue;
        }
        let raw_path = raw_path.trim();
        if raw_path.is_empty() {
            continue;
        }
        let path = Path::new(raw_path);
        let member = path
            .strip_prefix(dest_path)
            .ok()
            .and_then(|relative| relative.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(str::trim)
            })
            .filter(|value| !value.is_empty());
        let Some(member) = member else {
            continue;
        };
        let member = member.trim_matches('/').to_string();
        if member.is_empty() || members.iter().any(|existing| existing == &member) {
            continue;
        }
        members.push(member);
        if members.len() >= 5 {
            break;
        }
    }
    members
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitStatusEntry {
    pub(crate) status: String,
    pub(crate) path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitRepositoryStateObservation {
    pub(crate) branch: Option<String>,
    pub(crate) dirty: bool,
    pub(crate) changed_entries: Vec<GitStatusEntry>,
}

pub(crate) fn git_repository_state_observation_from_status_output(
    body: &str,
    branch_hint: Option<&str>,
) -> Option<GitRepositoryStateObservation> {
    let mut branch = branch_hint
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
        .map(ToOwned::to_owned);
    let mut saw_status_header = false;
    let mut changed_entries = Vec::new();
    for raw_line in body.lines() {
        let line = raw_line.trim_end();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("exit=") {
            continue;
        }
        if trimmed.starts_with("## ") {
            saw_status_header = true;
            if branch.is_none() {
                branch = git_current_branch_from_status_header(trimmed);
            }
            continue;
        }
        if let Some(entry) = git_short_status_entry_from_line(line) {
            changed_entries.push(entry);
        }
    }
    if !saw_status_header && changed_entries.is_empty() {
        return None;
    }
    Some(GitRepositoryStateObservation {
        branch,
        dirty: !changed_entries.is_empty(),
        changed_entries,
    })
}

fn git_repository_state_answer(
    observation: &GitRepositoryStateObservation,
    response_shape: Option<crate::OutputResponseShape>,
) -> String {
    let worktree = if observation.dirty { "dirty" } else { "clean" };
    let mut fields = Vec::new();
    if let Some(branch) = observation
        .branch
        .as_deref()
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
    {
        fields.push(format!("git.branch={branch}"));
    }
    fields.push(format!("git.worktree={worktree}"));
    if matches!(
        response_shape,
        Some(crate::OutputResponseShape::OneSentence)
    ) {
        return fields.join(" ");
    }
    fields.push(format!(
        "git.changed.count={}",
        observation.changed_entries.len()
    ));
    for (idx, entry) in observation.changed_entries.iter().enumerate() {
        fields.push(format!(
            "git.changed[{idx}]={} {}",
            entry.status, entry.path
        ));
    }
    fields.join("\n")
}

fn git_basic_direct_answer_candidate(
    _state: Option<&AppState>,
    route: Option<&crate::RouteResult>,
    body: &str,
    branch: Option<&str>,
    response_shape: Option<crate::OutputResponseShape>,
    _allow_localized_direct_template: bool,
    _prefer_english: bool,
) -> Option<String> {
    let route = route?;
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::GitRepositoryState {
        return None;
    }
    let observation = git_repository_state_observation_from_status_output(body, branch)?;
    if matches!(response_shape, Some(crate::OutputResponseShape::Scalar)) {
        return Some(if observation.dirty { "dirty" } else { "clean" }.to_string());
    }
    Some(git_repository_state_answer(&observation, response_shape))
}

fn latest_git_repository_state_direct_answer(
    state: Option<&AppState>,
    route: &crate::RouteResult,
    loop_state: &LoopState,
    response_shape: Option<crate::OutputResponseShape>,
    allow_localized_direct_template: bool,
    prefer_english: bool,
) -> Option<String> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::GitRepositoryState {
        return None;
    }
    let idx = latest_successful_step_index(loop_state, |step| step.skill == "git_basic")?;
    let body = loop_state.executed_step_results[idx]
        .output
        .as_deref()
        .map(str::trim)
        .filter(|body| !body.is_empty())?;
    let branch = latest_git_current_branch(loop_state);
    git_basic_direct_answer_candidate(
        state,
        Some(route),
        body,
        branch.as_deref(),
        response_shape,
        allow_localized_direct_template,
        prefer_english,
    )
}

fn latest_git_current_branch(loop_state: &LoopState) -> Option<String> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && step.skill == "git_basic")
        .filter_map(|step| step.output.as_deref())
        .find_map(git_current_branch_from_output)
}

fn git_current_branch_from_output(body: &str) -> Option<String> {
    for raw_line in body.lines() {
        let line = raw_line.trim_end();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("exit=") {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("* ") {
            return rest
                .split_whitespace()
                .next()
                .map(str::trim)
                .filter(|branch| !branch.is_empty())
                .map(ToOwned::to_owned);
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            if let Some(branch) = git_current_branch_from_status_header(rest) {
                return Some(branch);
            }
        }
    }
    None
}

fn git_current_branch_from_status_header(header: &str) -> Option<String> {
    let rest = header.strip_prefix("## ").unwrap_or(header);
    rest.split(['.', ' ', '\t'])
        .next()
        .map(str::trim)
        .filter(|branch| !branch.is_empty() && *branch != "HEAD")
        .map(ToOwned::to_owned)
}

fn git_short_status_entry_from_line(line: &str) -> Option<GitStatusEntry> {
    if !line_looks_like_git_short_status_entry(line) {
        return None;
    }
    let status = line.get(..2)?.trim().to_string();
    let path = line.get(3..)?.trim();
    if status.is_empty() || path.is_empty() {
        return None;
    }
    Some(GitStatusEntry {
        status,
        path: path.to_string(),
    })
}

fn line_looks_like_git_short_status_entry(line: &str) -> bool {
    let bytes = line.as_bytes();
    if bytes.len() < 3 {
        return false;
    }
    let status_a = bytes[0] as char;
    let status_b = bytes[1] as char;
    let sep = bytes[2] as char;
    (sep == ' ' || sep == '\t')
        && (is_git_short_status_code(status_a) || is_git_short_status_code(status_b))
}

fn is_git_short_status_code(ch: char) -> bool {
    matches!(ch, 'M' | 'A' | 'D' | 'R' | 'C' | 'U' | '?' | '!')
}

fn fs_search_output_direct_answer_candidate(
    state: Option<&AppState>,
    route: Option<&crate::RouteResult>,
    value: &serde_json::Value,
    locator_hint: Option<&str>,
    prefer_english: bool,
    allow_multi_result_list: bool,
    prefer_full_path: bool,
) -> Option<String> {
    route
        .and_then(|route| {
            fs_search_content_presence_direct_answer_candidate(state, route, value, prefer_english)
        })
        .or_else(|| {
            route.and_then(|route| {
                fs_search_route_filtered_listing_candidate(route, value, allow_multi_result_list)
            })
        })
        .or_else(|| route.and_then(|route| fs_search_semantic_listing_candidate(route, value)))
        .or_else(|| {
            fs_search_direct_answer_candidate(
                state,
                value,
                locator_hint,
                prefer_english,
                allow_multi_result_list,
                prefer_full_path,
            )
        })
        .map(|answer| {
            absolutize_fs_search_answer_paths(state, route, value, answer, prefer_full_path)
        })
}

fn route_allows_tail_read_range_direct_passthrough(
    route: Option<&crate::RouteResult>,
    response_shape: Option<crate::OutputResponseShape>,
    value: &serde_json::Value,
) -> bool {
    if matches!(
        response_shape,
        Some(crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Scalar)
    ) {
        return false;
    }
    let Some(route) = route else {
        return false;
    };
    if route.output_contract.delivery_required {
        return false;
    }
    if value.get("mode").and_then(|v| v.as_str()) != Some("tail") {
        return false;
    }
    let Some(requested_n) = value.get("requested_n").and_then(|v| v.as_u64()) else {
        return false;
    };
    if requested_n == 0 || requested_n > 50 {
        return false;
    }
    route.output_contract.requires_content_evidence
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::RawCommandOutput
        )
}

fn route_allows_read_range_direct_passthrough(
    route: Option<&crate::RouteResult>,
    response_shape: Option<crate::OutputResponseShape>,
) -> bool {
    if matches!(
        response_shape,
        Some(crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Scalar)
    ) {
        return false;
    }
    let Some(route) = route else {
        return false;
    };
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::None {
        return false;
    }
    if route.ask_mode.is_plain_act() {
        return true;
    }
    route.ask_mode.finalize_chat_wrapped()
        && route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
        )
}

fn allows_normalized_scalar_direct_fallback(
    skill: &str,
    response_shape: Option<crate::OutputResponseShape>,
) -> bool {
    match skill {
        "git_basic" => false,
        "package_manager" => false,
        "archive_basic" => false,
        "http_basic" => !matches!(
            response_shape,
            Some(crate::OutputResponseShape::OneSentence)
        ),
        _ => true,
    }
}

pub(crate) fn extract_direct_answer_from_generic_output(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    extract_direct_answer_from_generic_output_impl(None, loop_state, agent_run_context)
}

pub(crate) fn extract_direct_answer_from_generic_output_i18n(
    loop_state: &LoopState,
    state: &AppState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    extract_direct_answer_from_generic_output_impl(Some(state), loop_state, agent_run_context)
}

fn replace_internal_missing_sentinel_with_structured_observation(
    answer: &str,
    state: &AppState,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    if !is_internal_missing_scalar_sentinel(answer) {
        return None;
    }
    extract_direct_answer_from_generic_output_i18n(loop_state, state, agent_run_context)
        .or_else(|| {
            extract_direct_scalar_from_generic_output_i18n(loop_state, state, agent_run_context)
        })
        .map(|replacement| replacement.trim().to_string())
        .filter(|replacement| !replacement.is_empty())
        .filter(|replacement| !is_internal_missing_scalar_sentinel(replacement))
}

pub(crate) fn answer_is_direct_observation_passthrough(
    answer: &str,
    loop_state: &LoopState,
) -> bool {
    let answer = answer.trim();
    if answer.is_empty() {
        return false;
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| {
            step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "synthesize_answer" | "think"
                )
        })
        .filter_map(|step| step.output.as_deref().map(str::trim))
        .filter(|body| !body.is_empty())
        .any(|body| {
            answer == body
                || normalized_observed_listing(body).is_some_and(|listing| {
                    let listing = listing.trim();
                    answer == listing
                        || listing
                            .lines()
                            .map(str::trim)
                            .any(|line| !line.is_empty() && line == answer)
                })
        })
}

fn observed_error_step_body(
    step: &crate::executor::StepExecutionResult,
    body: &str,
) -> Option<String> {
    if !crate::skills::is_observable_run_cmd_error(&step.skill, body)
        && !crate::skills::is_recoverable_skill_error(&step.skill, body)
    {
        return None;
    }
    let normalized = crate::skills::normalize_skill_error_for_user(&step.skill, body);
    let sanitized = crate::visible_text::sanitize_user_visible_text(&normalized);
    (!sanitized.trim().is_empty()).then(|| {
        format!(
            "execution_status: error\nerror_summary: {}",
            sanitized.trim()
        )
    })
}

fn observed_step_body(step: &crate::executor::StepExecutionResult) -> Option<String> {
    let body = if step.is_ok() {
        step.output.as_deref()
    } else {
        step.error.as_deref().or(step.output.as_deref())
    }
    .map(str::trim)
    .filter(|text| !text.is_empty())?;
    if !step.is_ok() {
        return observed_error_step_body(step, body);
    }
    if let Some(normalized) = structured_observed_body(&step.skill, body) {
        let sanitized = crate::visible_text::sanitize_user_visible_text(&normalized);
        return (!sanitized.trim().is_empty()).then_some(sanitized);
    }
    if let Some(normalized) = system_basic_structured_doc_observed_body(&step.skill, body) {
        let sanitized = crate::visible_text::sanitize_user_visible_text(&normalized);
        return (!sanitized.trim().is_empty()).then_some(sanitized);
    }
    if crate::finalize::classify_observed_content_status(body)
        != crate::finalize::ObservedContentStatus::ContentAvailable
    {
        return None;
    }
    let sanitized = crate::visible_text::sanitize_user_visible_text(body);
    (!sanitized.trim().is_empty()).then_some(sanitized)
}

fn observed_step_entry(step: &crate::executor::StepExecutionResult) -> Option<String> {
    let output = observed_step_body(step)?;
    if !normalized_structured_observed_fact_allows_artifact_filter_bypass(&step.skill, &output)
        && crate::finalize::looks_like_planner_artifact(&output)
        || crate::finalize::looks_like_internal_trace_artifact(&output)
    {
        return None;
    }
    Some(format!(
        "### {} skill({})\n{}",
        step.step_id,
        step.skill,
        trim_for_observed_prompt(&output, 1800)
    ))
}

fn normalized_structured_observed_fact_allows_artifact_filter_bypass(
    skill: &str,
    output: &str,
) -> bool {
    skill == "archive_basic" && output.trim_start().starts_with("archive_basic action=")
}

fn observed_output_entries(loop_state: &LoopState) -> Vec<String> {
    let latest_listing_idx = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .rfind(|(_, step)| {
            is_observation_step_for_answer_synthesis(step)
                && step.skill == "list_dir"
                && observed_step_entry(step).is_some()
        })
        .map(|(idx, _)| idx);
    let mut selected_indices = latest_listing_idx.into_iter().collect::<Vec<_>>();
    let mut recent_non_listing = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .filter(|(_, step)| {
            is_observation_step_for_answer_synthesis(step)
                && step.skill != "list_dir"
                && observed_step_entry(step).is_some()
        })
        .map(|(idx, _)| idx)
        .collect::<Vec<_>>();
    recent_non_listing = retain_latest_observation_indices_by_supersede_key(
        &loop_state.executed_step_results,
        recent_non_listing,
    );
    if recent_non_listing.len() > 4 {
        recent_non_listing = recent_non_listing.split_off(recent_non_listing.len() - 4);
    }
    selected_indices.extend(recent_non_listing);
    selected_indices.sort_unstable();
    selected_indices.dedup();
    selected_indices
        .into_iter()
        .filter_map(|idx| observed_step_entry(&loop_state.executed_step_results[idx]))
        .collect()
}

fn retain_latest_observation_indices_by_supersede_key(
    steps: &[crate::executor::StepExecutionResult],
    indices: Vec<usize>,
) -> Vec<usize> {
    let mut seen = std::collections::HashSet::new();
    let mut kept = Vec::with_capacity(indices.len());
    for idx in indices.into_iter().rev() {
        let Some(step) = steps.get(idx) else {
            continue;
        };
        let Some(key) = observation_supersede_key(step) else {
            kept.push(idx);
            continue;
        };
        if seen.insert(key) {
            kept.push(idx);
        }
    }
    kept.reverse();
    kept
}

fn observation_supersede_key(step: &crate::executor::StepExecutionResult) -> Option<String> {
    if !step.is_ok() {
        return None;
    }
    let body = step.output.as_deref()?.trim();
    if body.is_empty() {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let action = value
        .get("action")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let is_range_read = matches!(
        action,
        "read_range" | "read_text_range" | "read_file" | "parse_doc"
    );
    if !is_range_read {
        return None;
    }
    let path = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    Some(format!(
        "file_content:{}:{path}",
        action_family_for_supersede(action)
    ))
}

fn action_family_for_supersede(action: &str) -> &'static str {
    match action {
        "read_range" | "read_text_range" | "read_file" | "parse_doc" => "read_content",
        _ => "other",
    }
}

fn is_observation_step_for_answer_synthesis(step: &crate::executor::StepExecutionResult) -> bool {
    !matches!(
        step.skill.as_str(),
        "respond" | "synthesize_answer" | "think" | "answer_verifier"
    )
}

fn route_observation_facts_entry(agent_run_context: Option<&AgentRunContext>) -> Option<String> {
    let ctx = agent_run_context?;
    let route = ctx.route_result.as_ref()?;
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ExistenceWithPathSummary {
        return None;
    }
    let resolved_path = ctx
        .auto_locator_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or_else(|| {
            let hint = route.output_contract.locator_hint.trim();
            (!hint.is_empty()).then_some(hint)
        })?;
    Some(format!(
        "### route_contract_facts\nresolved_target_path: {resolved_path}\npath_rule: use resolved_target_path as the target file path; do not infer the target path from file content fields such as WorkingDirectory."
    ))
}

fn cross_turn_synthesis_allowed(agent_run_context: Option<&AgentRunContext>) -> bool {
    matches!(
        agent_run_context
            .and_then(|ctx| ctx.turn_analysis.as_ref())
            .and_then(|analysis| analysis.target_task_policy),
        Some(crate::intent_router::TargetTaskPolicy::ReuseActive)
    )
}

fn recent_generated_output_from_user_request(user_request: &str) -> Option<String> {
    const MARKER: &str = "Most recent generated output:\n";
    let (_, tail) = user_request.split_once(MARKER)?;
    let stop_idx = [
        "\n\nContinuity rules:",
        "\n\nStructured task updates:",
        "\n\nNew user instruction:",
        "\n\n### SESSION_ALIAS_BINDINGS",
    ]
    .iter()
    .filter_map(|marker| tail.find(marker))
    .min()
    .unwrap_or(tail.len());
    let output = tail[..stop_idx].trim();
    if output.is_empty()
        || output == "<none>"
        || crate::finalize::looks_like_planner_artifact(output)
        || crate::finalize::looks_like_internal_trace_artifact(output)
    {
        return None;
    }
    let sanitized = crate::visible_text::sanitize_user_visible_text(output);
    (!sanitized.trim().is_empty()).then_some(sanitized)
}

fn cross_turn_observed_output_entries(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Vec<String> {
    if !cross_turn_synthesis_allowed(agent_run_context) {
        return Vec::new();
    }

    if let Some(recent_output) = agent_run_context
        .and_then(|ctx| ctx.user_request.as_deref())
        .and_then(recent_generated_output_from_user_request)
    {
        return vec![format!(
            "### prior_turn_observed_output\n{}",
            trim_for_observed_prompt(&recent_output, 1800)
        )];
    }

    if let Some(cross_turn_context) = loop_state
        .output_vars
        .get("cross_turn_recent_execution_context")
        .map(String::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty() && *text != "<none>")
    {
        return vec![format!(
            "### prior_turn_execution_context\n{}",
            trim_for_observed_prompt(
                &crate::visible_text::sanitize_user_visible_text(cross_turn_context),
                1800
            )
        )];
    }

    Vec::new()
}

pub(crate) fn has_observed_answer_candidates(loop_state: &LoopState) -> bool {
    !observed_output_entries(loop_state).is_empty()
}

fn observed_contract_json(agent_run_context: Option<&AgentRunContext>) -> String {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return "{}".to_string();
    };
    let direct_observation_passthrough_allowed =
        !route_disallows_direct_observation_passthrough(route);
    serde_json::json!({
        "ask_mode": route.ask_mode.as_str(),
        "derived_route_label": route.derived_route_label(),
        "response_shape": route.output_contract.response_shape.as_str(),
        "exact_sentence_count": route.output_contract.exact_sentence_count,
        "requires_content_evidence": route.output_contract.requires_content_evidence,
        "delivery_required": route.output_contract.delivery_required,
        "direct_observation_passthrough_allowed": direct_observation_passthrough_allowed,
        "locator_kind": route.output_contract.locator_kind.as_str(),
        "delivery_intent": route.output_contract.delivery_intent.as_str(),
        "semantic_kind": route.output_contract.semantic_kind.as_str(),
        "locator_hint": route.output_contract.locator_hint,
        "needs_clarify": route.needs_clarify,
    })
    .to_string()
}

fn resolved_user_intent(agent_run_context: Option<&AgentRunContext>, user_text: &str) -> String {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.resolved_intent.trim())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| user_text.trim())
        .to_string()
}

pub(crate) async fn try_synthesize_answer_from_observed_output(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<Option<(String, crate::task_journal::TaskJournalFinalizerSummary)>, String> {
    // §3.3 Stage 3.2 invariant：observed-tier LLM 兜底属于 finalize 子层，
    // 进入时 ask_state 必须是 Executing 或 Finalizing。Executing 是因为
    // 该兜底由 finalize_loop_reply 里调用、ask_state 还没 transition 到
    // Finalizing；Finalizing 兼容 §3.1 后续把 transition 提前的可能。
    debug_assert!(
        matches!(
            state.current_ask_state(&task.task_id),
            None | Some(crate::AskState::Executing) | Some(crate::AskState::Finalizing)
        ),
        "synthesize_answer_from_observed_output invariant: ask_state must be Executing|Finalizing, got {:?} (task_id={})",
        state.current_ask_state(&task.task_id),
        task.task_id,
    );

    let mut observed_entries = observed_output_entries(loop_state);
    if let Some(route_facts) = route_observation_facts_entry(agent_run_context) {
        observed_entries.insert(0, route_facts);
    }
    if observed_entries.is_empty() {
        observed_entries = cross_turn_observed_output_entries(loop_state, agent_run_context);
    }
    if observed_entries.is_empty() {
        return Ok(None);
    }
    let observed_block = observed_entries.join("\n\n");
    let resolved_intent = resolved_user_intent(agent_run_context, user_text);
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_text);
    let user_request_for_prompt = crate::language_policy::task_original_user_text(task)
        .unwrap_or_else(|| user_text.trim().to_string());
    let (prompt_template, prompt_source) =
        match crate::bootstrap::load_required_prompt_template_for_state(
            state,
            OBSERVED_ANSWER_FALLBACK_PROMPT_LOGICAL_PATH,
        ) {
            Ok(resolved) => resolved,
            Err(err) => {
                tracing::warn!(
                    "observed_answer_fallback prompt_missing task_id={} err={}",
                    task.task_id,
                    err
                );
                return Err(format!("observed answer fallback prompt missing: {err}"));
            }
        };
    let response_style_hint = observed_response_style_hint(agent_run_context);
    let prompt = crate::render_prompt_template(
        &prompt_template,
        &[
            ("__USER_REQUEST__", &user_request_for_prompt),
            ("__RESOLVED_USER_INTENT__", &resolved_intent),
            (
                "__OUTPUT_CONTRACT__",
                &observed_contract_json(agent_run_context),
            ),
            ("__OBSERVED_OUTPUTS__", &observed_block),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.policy.command_intent.default_locale,
            ),
            ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
            ("__RESPONSE_STYLE_HINT__", &response_style_hint),
        ],
    );
    crate::log_prompt_render(
        state,
        &task.task_id,
        "observed_answer_fallback_prompt",
        &prompt_source,
        None,
    );
    let llm_out =
        llm_gateway::run_with_fallback_with_prompt_source(state, task, &prompt, &prompt_source)
            .await
            .map_err(|err| format!("observed answer fallback LLM failed: {err}"))?;
    let llm_out_for_parse = strip_bare_json_language_prefix(&llm_out);
    let parsed = match crate::prompt_utils::validate_against_schema::<ObservedAnswerFallbackOut>(
        llm_out_for_parse,
        crate::prompt_utils::PromptSchemaId::FinalizerOut,
    ) {
        Ok(validated) => {
            if !validated.raw_parse_ok {
                tracing::info!(
                    "observed_answer_fallback schema_parse_recovery task_id={} schema_normalized={}",
                    task.task_id,
                    validated.schema_normalized
                );
            }
            Some(validated.value)
        }
        Err(err) => {
            tracing::info!(
                "observed_answer_fallback schema_validation_failed task_id={} err={}",
                task.task_id,
                err
            );
            None
        }
    }
    .or_else(|| {
            // F14: some providers occasionally violate the "Output JSON only" contract,
            // 直接吐 markdown 文本（常见表现：被多步 read 喂饱后给一段中文综述但没包成
            // JSON envelope）。原先 ObservedAnswerFallbackOut 解析失败 → 整个 fallback
            // 返回 None → finalize 落到 clarify_question_fallback，把已经合成好的真实
            // 答案丢掉，变成"假需要确认"。这里把 trim 后的整段文本视作 answer 兜底，
            // 同时 publishable=true、qualified=true、confidence=0.7（足以越过下游
            // OBSERVED_SELF_CLASSIFY_CONF_THRESHOLD=0.55，并保留下游 semantic_judge 的
            // meta-instruction 检查仍能拦截 "我会去检查/please confirm" 之类伪答案）。
            let trimmed_owned;
            let trimmed = if let Some(non_code_text) = non_code_markdown_text(llm_out_for_parse) {
                trimmed_owned = non_code_text;
                trimmed_owned.trim()
            } else {
                llm_out_for_parse.trim().trim_matches('`').trim()
            };
            if trimmed.is_empty() {
                return None;
            }
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                return None;
            }
        Some(ObservedAnswerFallbackOut {
            answer: trimmed.to_string(),
            qualified: true,
            needs_clarify: false,
            is_meta_instruction: false,
            publishable: true,
            confidence: 0.7,
            _reason: String::from("non_json_text_fallback"),
        })
    });
    let Some(parsed) = parsed else {
        return Ok(None);
    };
    let mut answer = parsed.answer.trim().to_string();
    if let Some(unwrapped) = extract_answer_from_finalizer_envelope_text(&answer) {
        answer = unwrapped;
    }
    if let Some(replacement) = replace_internal_missing_sentinel_with_structured_observation(
        &answer,
        state,
        loop_state,
        agent_run_context,
    ) {
        tracing::info!(
            "observed_answer_fallback_replace_internal_missing_sentinel task_id={} replacement={}",
            task.task_id,
            crate::truncate_for_log(&replacement)
        );
        answer = replacement;
    }
    if let Some(replacement) =
        archive_list_raw_passthrough_replacement(&answer, state, loop_state, &request_language_hint)
    {
        tracing::info!(
            "observed_answer_fallback_replace_archive_raw_passthrough task_id={} replacement={}",
            task.task_id,
            crate::truncate_for_log(&replacement)
        );
        answer = replacement;
    }
    if let Some(diagnostic) = scalar_count_diagnostic_line_for_answer(
        &answer,
        agent_run_context.and_then(|ctx| ctx.route_result.as_ref()),
        loop_state,
    ) {
        tracing::info!(
            "observed_answer_fallback_replace_scalar_count_with_diagnostic task_id={} diagnostic={}",
            task.task_id,
            crate::truncate_for_log(&diagnostic)
        );
        answer = observed_t_with_vars(
            Some(state),
            "clawd.msg.scalar_count_unreliable",
            "无法可靠统计：{diagnostic}",
            "Unable to produce a reliable count: {diagnostic}",
            request_language_hint.starts_with("en"),
            &[("diagnostic", &diagnostic)],
        );
    }
    let direct_passthrough_disallowed = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(route_disallows_direct_observation_passthrough)
        && answer_is_direct_observation_passthrough(&answer, loop_state);
    if direct_passthrough_disallowed {
        tracing::info!(
            "observed_answer_fallback_reject_direct_passthrough task_id={} answer={}",
            task.task_id,
            crate::truncate_for_log(&answer)
        );
        answer.clear();
    }
    if !answer.is_empty() {
        let prefer_english = crate::fallback::fallback_prefers_english_for_language_hint(
            state,
            &request_language_hint,
        );
        answer = compose_content_excerpt_with_summary_answer(
            &answer,
            loop_state,
            prefer_english,
            agent_run_context.and_then(|ctx| ctx.route_result.as_ref()),
        );
    }
    // §3.4 finalize-tier: 这里属于 observed_answer_fallback 兜底路径（finalize 层
    // 的 fallback 分支），是 semantic_judge LLM 入口的允许调用方之一。
    // Phase 0.2: 复用同一次 LLM 调用已经返回的 `publishable` + `is_meta_instruction`，
    // 高置信度时直接信任，避免再发一次 `semantic_judge::is_meta_respond_instruction`
    // 二次判定调用。低置信度（<0.55）时才回退到 semantic_judge 做安全兜底，
    // 保留"LLM 过保守错判为不可发"的救回链路。
    const OBSERVED_SELF_CLASSIFY_CONF_THRESHOLD: f64 = 0.55;
    let semantically_publishable = if !answer.is_empty() && !parsed.needs_clarify {
        if parsed.confidence >= OBSERVED_SELF_CLASSIFY_CONF_THRESHOLD {
            parsed.publishable && !parsed.is_meta_instruction
        } else if parsed.publishable {
            !parsed.is_meta_instruction
        } else {
            !crate::semantic_judge::is_meta_respond_instruction(state, task, &answer).await
        }
    } else {
        false
    };
    let qualified = !answer.is_empty()
        && !parsed.needs_clarify
        && !direct_passthrough_disallowed
        && (parsed.qualified || semantically_publishable);
    Ok(Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(if qualified {
                crate::finalize::FinalizerDisposition::QualifiedCompletion
            } else {
                crate::finalize::FinalizerDisposition::AllowFallback
            }),
            parsed: true,
            contract_ok: qualified,
            completion_ok: Some(qualified),
            grounded_ok: Some(qualified),
            format_ok: Some(qualified),
            needs_clarify: Some(parsed.needs_clarify),
            confidence: Some(parsed.confidence.clamp(0.0, 1.0)),
            used_evidence_ids_count: observed_entries.len(),
            evidence_quotes_count: 0,
            ..Default::default()
        },
    )))
}

pub(crate) async fn synthesize_answer_from_observed_output(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    match try_synthesize_answer_from_observed_output(
        state,
        task,
        user_text,
        loop_state,
        agent_run_context,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(err) => {
            tracing::warn!(
                "observed_answer_fallback unavailable task_id={} err={}",
                task.task_id,
                err
            );
            None
        }
    }
}

pub(crate) fn normalized_observed_listing(observed: &str) -> Option<String> {
    normalized_listing_text(observed)
}

#[cfg(test)]
#[path = "observed_output_tests.rs"]
mod tests;
