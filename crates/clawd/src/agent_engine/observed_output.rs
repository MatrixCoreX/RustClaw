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

fn hidden_entries_direct_answer(
    _state: Option<&AppState>,
    route: &crate::RouteResult,
    loop_state: &LoopState,
    _prefer_english: bool,
) -> Option<String> {
    if !route_requests_hidden_entries_check(route) {
        return None;
    }
    if route.output_contract.response_shape != crate::OutputResponseShape::Scalar {
        return None;
    }
    let hidden_entries = latest_directory_listing_entries(loop_state, None)
        .map(|entries| hidden_entries_from_entries(&entries))
        .or_else(|| {
            latest_list_dir_listing(loop_state).map(|listing| hidden_entries_from_listing(&listing))
        })?;
    Some(hidden_entries.len().to_string())
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

#[derive(Debug, Clone)]
struct StructuredScalarObservation;

fn structured_scalar_observation_from_extract_item(
    value: &serde_json::Value,
) -> Option<StructuredScalarObservation> {
    if !value
        .get("exists")
        .and_then(|item| item.as_bool())
        .unwrap_or(false)
    {
        return None;
    }
    let raw_value = value.get("value").unwrap_or(&serde_json::Value::Null);
    value
        .get("value_text")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|_| StructuredScalarObservation)
        .or_else(|| value_scalar_text(raw_value).map(|_| StructuredScalarObservation))
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
        Some("extract_field") => structured_scalar_observation_from_extract_item(&value),
        Some("extract_fields") => {
            let results = value.get("results")?.as_array()?;
            if results.len() != 1 {
                return None;
            }
            structured_scalar_observation_from_extract_item(results.first()?)
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

fn observed_response_style_hint(agent_run_context: Option<&AgentRunContext>) -> String {
    if agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(route_requires_synthesized_delivery)
    {
        return "Use the observed output as evidence to produce the requested final wording. Do not answer by copying only the raw observed output; that would be an incomplete passthrough for this contract.".to_string();
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

fn db_basic_scalar_candidate(value: &serde_json::Value) -> Option<String> {
    let columns = value.get("columns")?.as_array()?;
    if columns.len() != 1 {
        return None;
    }
    let column = columns[0].as_str()?.trim();
    if column.is_empty() {
        return None;
    }
    let row = value.get("rows")?.as_array()?.first()?.as_object()?;
    value_scalar_text(row.get(column)?)
}

fn db_basic_table_names(value: &serde_json::Value) -> Option<Vec<String>> {
    let columns = value.get("columns")?.as_array()?;
    let column_name = if columns.len() == 1 {
        columns[0].as_str()?.trim()
    } else if columns
        .iter()
        .any(|column| column.as_str().is_some_and(|name| name == "name"))
    {
        "name"
    } else {
        return None;
    };
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

fn transform_skill_formatted_output_candidate(body: &str) -> Option<String> {
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
    if skill != "system_basic" {
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
        Some("extract_field" | "extract_fields" | "structured_keys")
    )
    .then_some(value)
}

fn system_basic_structured_doc_observed_body(skill: &str, body: &str) -> Option<String> {
    let value = system_basic_structured_doc_value(skill, body)?;
    match value.get("action").and_then(|v| v.as_str()) {
        Some("extract_field") => {
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
        Some("extract_fields") => extract_fields_direct_answer_candidate(
            None,
            &value,
            Some(crate::OutputResponseShape::Free),
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
        match entry
            .get("kind")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("other")
        {
            "dir" => dirs.push(name.to_string()),
            "file" => files.push(name.to_string()),
            _ => others.push(name.to_string()),
        }
    }

    let mut lines = Vec::new();
    if !dirs.is_empty() {
        lines.push(format!("dir_names={}", dirs.join(",")));
    }
    if !files.is_empty() {
        lines.push(format!("file_names={}", files.join(",")));
    }
    if !others.is_empty() {
        lines.push(format!("other_names={}", others.join(",")));
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
                return None;
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

fn route_intent_extension_hints(route: &crate::RouteResult, results: &[String]) -> Vec<String> {
    let available_exts = result_extensions(results);
    if available_exts.is_empty() {
        return Vec::new();
    }
    pathish_filter_tokens(&route.resolved_intent)
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

fn semantic_fs_search_score(path: &str, tokens: &[String], ignored_tokens: &[String]) -> usize {
    tokens
        .iter()
        .filter(|token| {
            token.len() >= 3
                && !token.chars().all(|ch| ch.is_ascii_digit())
                && !ignored_tokens.iter().any(|ignored| ignored == *token)
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

    let ext_hints = route_intent_extension_hints(route, &results);
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

    let mut ignored_tokens = ext_hints;
    ignored_tokens.extend(pathish_filter_tokens(locator_hint));
    if let Some(pattern) = normalized_find_name_pattern(pattern.as_deref()) {
        if locator_hint
            .is_empty()
            .then_some(false)
            .unwrap_or_else(|| path_is_inside_locator_scope(&pattern, locator_hint))
        {
            ignored_tokens.extend(pathish_filter_tokens(&pattern));
        }
    }
    ignored_tokens.sort();
    ignored_tokens.dedup();

    let tokens = pathish_filter_tokens(&route.resolved_intent);
    let mut scored = results
        .iter()
        .cloned()
        .map(|path| {
            let score = semantic_fs_search_score(&path, &tokens, &ignored_tokens);
            (score, path)
        })
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let top_score = scored.first().map(|(score, _)| *score).unwrap_or_default();
    if top_score == 0 {
        return None;
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
            return Some(results.into_iter().take(3).collect::<Vec<_>>().join("\n"));
        }
        return filtered.into_iter().next();
    }
    allow_multi_result_list.then(|| filtered.join("\n"))
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
    prefer_english: bool,
) -> Option<String> {
    if skill == "package_manager" {
        let response_shape = route.map(|route| route.output_contract.response_shape);
        return package_manager_summary_candidate(state, body, response_shape, prefer_english);
    }
    if skill == "git_basic" {
        return git_basic_scalar_candidate(route, body);
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if skill == "db_basic" {
        if let Some(route) = route {
            return match route.output_contract.semantic_kind {
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
        if let Some(answer) = route
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
        {
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
    let action = value.get("action").and_then(|v| v.as_str())?;
    match action {
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
        "extract_field" => {
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
                        return Some(structured_field_display_line(
                            state,
                            field_path,
                            value.get("value").unwrap_or(&serde_json::Value::Null),
                            value.get("value_text").and_then(|v| v.as_str()),
                            true,
                            prefer_english,
                        ));
                    }
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
        "count_inventory" => count_inventory_direct_answer_candidate(
            state,
            &value,
            route.map(|route| route.output_contract.response_shape),
            prefer_english,
        ),
        _ => None,
    }
}

fn package_manager_summary_candidate(
    state: Option<&AppState>,
    body: &str,
    response_shape: Option<crate::OutputResponseShape>,
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
        ) => Some(observed_t_with_vars(
            state,
            "clawd.msg.package_manager_detected",
            "当前识别到的包管理器是 {manager}。",
            "Detected package manager: {manager}.",
            prefer_english,
            &[("manager", manager)],
        )),
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

fn extract_fields_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    response_shape: Option<crate::OutputResponseShape>,
    prefer_english: bool,
) -> Option<String> {
    if value.get("action").and_then(|v| v.as_str()) != Some("extract_fields") {
        return None;
    }
    let results = value.get("results")?.as_array()?;
    if results.is_empty() {
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

fn extract_field_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    response_shape: Option<crate::OutputResponseShape>,
    prefer_english: bool,
) -> Option<String> {
    if value.get("action").and_then(|v| v.as_str()) != Some("extract_field") {
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
        return None;
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

fn structured_keys_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
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
            if matches!(
                response_shape,
                Some(crate::OutputResponseShape::OneSentence)
            ) {
                return None;
            }
            return Some(keys.join("\n"));
        }
        "array" => {
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
    let total_lines = value
        .get("total_lines")
        .and_then(|v| v.as_u64())
        .unwrap_or_default();

    let mut sections = vec![format!("log_analyze path={path} total_lines={total_lines}")];
    if !keyword_counts.is_empty() {
        sections.push(format!("keyword_counts: {}", keyword_counts.join(", ")));
    }
    if !recent_matches.is_empty() {
        sections.push(format!(
            "recent_matches:\n- {}",
            recent_matches.join("\n- ")
        ));
    }
    Some(sections.join("\n"))
}

fn archive_basic_observed_candidate(value: &serde_json::Value) -> Option<String> {
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
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    match skill {
        "system_basic" => {
            let action = value.get("action").and_then(|v| v.as_str())?;
            match action {
                "read_range" => read_range_observed_candidate(&value),
                "inventory_dir" => inventory_dir_observed_candidate(&value),
                "count_inventory" => count_inventory_observed_candidate(&value),
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
                    _ => {}
                }
            }
            fs_search_grep_text_observed_candidate(&value).or_else(|| {
                fs_search_direct_answer_candidate(None, &value, None, false, true, false)
            })
        }
        "archive_basic" => archive_basic_observed_candidate(&value),
        "log_analyze" => compact_log_analyze_excerpt(&value),
        "package_manager" => package_manager_summary_candidate(
            None,
            body,
            Some(crate::OutputResponseShape::OneSentence),
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
    prefer_english: bool,
) -> Option<String> {
    if let Some(path) = recent_file_path_candidate_for_scalar_path(loop_state, route) {
        return Some(path);
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
            return Some(answer);
        }
    }
    let observed_output = extract_latest_generic_successful_output(loop_state)?;
    let answer = structured_scalar_candidate(
        state,
        route,
        &observed_output.skill,
        &observed_output.body,
        locator_hint.filter(|hint| !hint.trim().is_empty()),
        auto_locator_path,
        prefer_full_path,
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
    Some(answer)
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
    if let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) {
        if route_needs_structured_scalar_pair_synthesis(loop_state, agent_run_context) {
            return None;
        }
        if let Some(answer) =
            count_inventory_planned_file_dir_breakdown_answer(None, loop_state, false)
        {
            return Some(answer);
        }
        if let Some(answer) = count_answer_from_latest_listing(route, loop_state) {
            return Some(answer);
        }
        if let Some(answer) = count_answer_from_latest_fs_search(route, loop_state) {
            return Some(answer);
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
    let prefer_english = current_turn_request_text(route, agent_run_context)
        .map(
            |intent| match crate::language_policy::request_language_hint(intent) {
                "en" => true,
                "zh-CN" => false,
                _ => state
                    .policy
                    .command_intent
                    .default_locale
                    .to_ascii_lowercase()
                    .starts_with("en"),
            },
        )
        .unwrap_or_else(|| {
            state
                .policy
                .command_intent
                .default_locale
                .to_ascii_lowercase()
                .starts_with("en")
        });
    if let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) {
        if route_needs_structured_scalar_pair_synthesis(loop_state, agent_run_context) {
            return None;
        }
        if let Some(answer) = count_inventory_planned_file_dir_breakdown_answer(
            Some(state),
            loop_state,
            prefer_english,
        ) {
            return Some(answer);
        }
        if let Some(answer) = count_answer_from_latest_listing(route, loop_state) {
            return Some(answer);
        }
        if let Some(answer) = count_answer_from_latest_fs_search(route, loop_state) {
            return Some(answer);
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
    let prefers_english_free_text = request_language_hint == "en";
    let prefers_english_presence_answer = route.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::ExistenceWithPath
            && prefers_english_free_text
    });
    let existence_with_path_should_use_llm_synthesis = false;
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
    if has_successful_step_for_skill(loop_state, "health_check")
        && !health_check_prefers_raw_payload
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
            hidden_entries_direct_answer(state, route, loop_state, prefers_english_free_text)
        {
            return Some(answer);
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
                "health_check" => {
                    health_check_prefers_raw_payload.then_some(observed_output.body.clone())
                }
                "http_basic" => None,
                "process_basic" => None,
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
                "git_basic" => None,
                "doc_parse" => {
                    content_excerpt_summary_direct_answer_candidate(route, &observed_output.body)
                }
                "db_basic" => route.and_then(|route| {
                    db_basic_tables_summary_candidate(
                        state,
                        route,
                        &observed_output.body,
                        prefers_english_free_text,
                    )
                }),
                "transform" => transform_skill_formatted_output_candidate(&observed_output.body),
                "package_manager" => package_manager_summary_candidate(
                    state,
                    &observed_output.body,
                    response_shape,
                    prefers_english_free_text,
                ),
                "archive_basic" => None,
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
                    } else if action == Some("inventory_dir")
                        && is_plain_act
                        && allow_raw_listing_direct_answer
                    {
                        inventory_dir_direct_answer_candidate(
                            state,
                            route,
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
                    } else if action == Some("extract_field") {
                        extract_field_direct_answer_candidate(
                            state,
                            &value,
                            response_shape,
                            prefers_english_free_text,
                        )
                    } else if action == Some("extract_fields") {
                        extract_fields_direct_answer_candidate(
                            state,
                            &value,
                            response_shape,
                            prefers_english_free_text,
                        )
                    } else if action == Some("structured_keys") {
                        structured_keys_direct_answer_candidate(
                            state,
                            &value,
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
                        && route.is_some_and(|route| {
                            route_requests_scalar_path_only(route)
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
    Some(answer)
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
            fs_search_route_filtered_listing_candidate(route, value, allow_multi_result_list)
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

fn answer_is_direct_observation_passthrough(answer: &str, loop_state: &LoopState) -> bool {
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
    if crate::finalize::looks_like_planner_artifact(&output)
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

fn observed_output_entries(loop_state: &LoopState) -> Vec<String> {
    let latest_listing_idx = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .rfind(|(_, step)| step.skill == "list_dir" && observed_step_entry(step).is_some())
        .map(|(idx, _)| idx);
    let mut selected_indices = latest_listing_idx.into_iter().collect::<Vec<_>>();
    let mut recent_non_listing = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .filter(|(_, step)| step.skill != "list_dir" && observed_step_entry(step).is_some())
        .map(|(idx, _)| idx)
        .collect::<Vec<_>>();
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
    let direct_observation_passthrough_allowed = !route_requires_synthesized_delivery(route);
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
        .is_some_and(route_requires_synthesized_delivery)
        && answer_is_direct_observation_passthrough(&answer, loop_state);
    if direct_passthrough_disallowed {
        tracing::info!(
            "observed_answer_fallback_reject_direct_passthrough task_id={} answer={}",
            task.task_id,
            crate::truncate_for_log(&answer)
        );
        answer.clear();
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
mod tests {
    use std::path::Path;

    use super::super::LoopState;
    use super::{
        answer_is_direct_observation_passthrough, cross_turn_observed_output_entries,
        extract_direct_answer_from_generic_output, extract_direct_scalar_from_generic_output,
        extract_direct_scalar_from_generic_output_i18n,
        extract_direct_scalar_from_generic_output_with_locator_hint,
        has_observed_answer_candidates, inventory_dir_direct_answer_candidate,
        normalize_system_basic_match_path, normalized_observed_listing, observed_contract_json,
        observed_output_entries, observed_request_language_hint, observed_response_style_hint,
        recent_generated_output_from_user_request,
        replace_internal_missing_sentinel_with_structured_observation,
        route_observation_facts_entry, route_requires_synthesized_delivery,
        scalar_count_diagnostic_line_for_answer, scalar_route_prefers_structured_observed_answer,
        structured_observed_body, AgentRunContext, OBSERVED_ANSWER_FALLBACK_PROMPT_TEMPLATE,
    };
    use crate::executor::{StepExecutionResult, StepExecutionStatus};
    use crate::{
        AppState, IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind,
        OutputResponseShape, OutputSemanticKind, ResumeBehavior, RiskCeiling, RouteResult,
        ScheduleKind,
    };

    fn ok_step(step_id: &str, skill: &str, output: &str) -> StepExecutionResult {
        StepExecutionResult {
            step_id: step_id.to_string(),
            skill: skill.to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(output.to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        }
    }

    fn error_step(step_id: &str, skill: &str, error: &str) -> StepExecutionResult {
        StepExecutionResult {
            step_id: step_id.to_string(),
            skill: skill.to_string(),
            status: StepExecutionStatus::Error,
            output: None,
            error: Some(error.to_string()),
            started_at: 0,
            finished_at: 0,
        }
    }

    #[test]
    fn observed_outputs_include_structured_run_cmd_error() {
        let err = format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "run_cmd",
                "error_kind": "nonzero_exit",
                "error_text": "Command failed with exit code 128",
                "platform": "linux",
                "extra": {
                    "command": "git -C /tmp status",
                    "exit_code": 128,
                    "exit_category": "terminated_by_signal_or_shell_status",
                    "stderr": "fatal: not a git repository",
                    "output_truncated": false
                }
            })
        );
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(error_step("step_1", "run_cmd", &err));

        let entries = observed_output_entries(&loop_state);
        let joined = entries.join("\n");

        assert!(has_observed_answer_candidates(&loop_state));
        assert!(joined.contains("skill(run_cmd)"), "entries: {joined}");
        assert!(
            joined.contains("execution_status: error"),
            "entries: {joined}"
        );
        assert!(
            joined.contains("fatal: not a git repository"),
            "entries: {joined}"
        );
    }

    fn chat_wrapped_unclassified_route(response_shape: OutputResponseShape) -> RouteResult {
        RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "Run an observation, then produce the requested final wording."
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::None,
                locator_hint: "/workspace/project".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        }
    }

    #[test]
    fn scalar_count_answer_detects_non_numeric_diagnostic_line() {
        let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
        route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "configs/config_copy".to_string();
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "0\n\nfind: /workspace/configs/config_copy: No such file or directory\n",
        ));

        let diagnostic = scalar_count_diagnostic_line_for_answer("0", Some(&route), &loop_state);

        assert_eq!(
            diagnostic.as_deref(),
            Some("find: /workspace/configs/config_copy: No such file or directory")
        );
    }

    fn reuse_active_context(user_request: &str) -> AgentRunContext {
        AgentRunContext {
            turn_analysis: Some(crate::intent_router::TurnAnalysis {
                turn_type: Some(crate::intent_router::TurnType::TaskAppend),
                target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
                should_interrupt_active_run: false,
                state_patch: None,
                attachment_processing_required: false,
            }),
            user_request: Some(user_request.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn recent_generated_output_extracts_internal_merge_block() {
        let merged = "Current task:\nlook at that docs dir\n\nMost recent generated output:\narchive\nrelease_checklist.md\nservice_notes.md\n\nContinuity rules:\n- keep scope\n\nNew user instruction:\ncount only";

        assert_eq!(
            recent_generated_output_from_user_request(merged).as_deref(),
            Some("archive\nrelease_checklist.md\nservice_notes.md")
        );
    }

    #[test]
    fn cross_turn_observed_entries_require_reuse_active_context() {
        let merged = "Current task:\nlook at that docs dir\n\nMost recent generated output:\narchive\nrelease_checklist.md\nservice_notes.md\n\nContinuity rules:\n- keep scope";
        let loop_state = LoopState::new(1);
        let allowed = reuse_active_context(merged);

        let entries = cross_turn_observed_output_entries(&loop_state, Some(&allowed));
        assert_eq!(entries.len(), 1);
        assert!(entries[0].contains("prior_turn_observed_output"));
        assert!(entries[0].contains("archive"));
        assert!(!entries[0].contains("Continuity rules"));

        let standalone = AgentRunContext {
            turn_analysis: Some(crate::intent_router::TurnAnalysis {
                turn_type: Some(crate::intent_router::TurnType::TaskRequest),
                target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
                should_interrupt_active_run: false,
                state_patch: None,
                attachment_processing_required: false,
            }),
            user_request: Some(merged.to_string()),
            ..Default::default()
        };
        assert!(cross_turn_observed_output_entries(&loop_state, Some(&standalone)).is_empty());
    }

    #[test]
    fn direct_scalar_ignores_exit_zero_prefix() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "git_basic", "exit=0\nmain\n"));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
            Some("main")
        );
    }

    #[test]
    fn direct_scalar_defers_git_oneline_log_record_to_synthesis() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "git_basic",
            "exit=0\n09342a6a fix: expose nl execution and locator flows\n",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "查看当前工作区最近一次 git 提交的标题，并简短告诉我。".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "Self-contained workspace inspection request for git commit title."
                .to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: ".".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };

        assert!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context))
                .is_none()
        );
    }

    #[test]
    fn observed_entries_include_structured_extract_field_outputs() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"react-example","value":"react-example","value_type":"string"}"#,
        ));
        loop_state.executed_step_results.push(ok_step(
            "step_2",
            "system_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"package.name","value_text":"clawd","value":"clawd","value_type":"string"}"#,
        ));

        let entries = observed_output_entries(&loop_state);
        assert_eq!(entries.len(), 2);
        assert!(entries[0].contains("name: react-example"));
        assert!(entries[1].contains("package.name: clawd"));
    }

    #[test]
    fn direct_scalar_ignores_shell_locale_warning_noise() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "/tmp/rustclaw-workspace\n\nbash: warning: setlocale: LC_ALL: cannot change locale (C.UTF-8): No such file or directory\n",
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
            Some("/tmp/rustclaw-workspace")
        );
    }

    #[test]
    fn direct_scalar_reads_extract_field_value_from_structured_output() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"rustclaw","value":"rustclaw","value_type":"string"}"#,
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
            Some("rustclaw")
        );
    }

    #[test]
    fn direct_scalar_preserves_resolved_extract_field_label_for_non_exact_match() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"model.vendor","resolved_field_path":"llm.selected_vendor","match_strategy":"missing_parent_leaf_key_suffix","value_text":"minimax","value":"minimax","value_type":"string"}"#,
        ));
        let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
        route_result.output_contract.locator_kind = OutputLocatorKind::Path;
        route_result.output_contract.locator_hint = "configs/config.toml".to_string();
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };

        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("llm.selected_vendor: minimax")
        );
    }

    #[test]
    fn direct_answer_reads_config_basic_extract_field_value() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"run_cmd.planner_kind","value_text":"tool","value":"tool","value_type":"string"}"#,
        ));
        let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
        route_result.output_contract.locator_kind = OutputLocatorKind::Path;
        route_result.output_contract.locator_hint = "configs/skills_registry.toml".to_string();
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };

        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("run_cmd.planner_kind: tool")
        );
        assert!(has_observed_answer_candidates(&loop_state));
    }

    #[test]
    fn direct_answer_defers_container_extract_field_to_synthesis() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"scripts","value":{"build":"echo build","dev":"echo dev","lint":"echo lint"},"value_text":"{\"build\":\"echo build\",\"dev\":\"echo dev\",\"lint\":\"echo lint\"}","value_type":"object"}"#,
        ));
        let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
        route_result.output_contract.locator_kind = OutputLocatorKind::Path;
        route_result.output_contract.locator_hint = "package.json".to_string();
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };

        assert!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .is_none()
        );
    }

    #[test]
    fn direct_answer_formats_config_basic_validate_result_as_pass_fail() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"validate_structured","path":"configs/config.toml","resolved_path":"/tmp/configs/config.toml","format":"toml","valid":true,"root_type":"object"}"#,
        ));
        let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::OneSentence);
        route_result.output_contract.locator_kind = OutputLocatorKind::Path;
        route_result.output_contract.locator_hint = "configs/config.toml".to_string();
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            original_user_request: Some(
                "Validate configs/config.toml and answer pass or fail.".to_string(),
            ),
            ..AgentRunContext::default()
        };

        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("pass: toml parsed successfully")
        );
    }

    #[test]
    fn direct_scalar_defers_recent_structured_scalar_comparison_to_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_fields","path":"UI/package.json","resolved_path":"/tmp/UI/package.json","count":1,"results":[{"field_path":"name","exists":true,"value_type":"string","value_text":"react-example","value":"react-example"}]}"#,
        ));
        loop_state.executed_step_results.push(ok_step(
            "step_2",
            "system_basic",
            r#"{"action":"extract_field","path":"crates/clawd/Cargo.toml","resolved_path":"/tmp/crates/clawd/Cargo.toml","field_path":"package.name","exists":true,"value_type":"string","value_text":"clawd","value":"clawd"}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent:
                "UI/package.json 里的 name 和 crates/clawd/Cargo.toml 里的 package.name 一样吗？只回答一样或不一样"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:compare_targets".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::QuantityComparison,
                locator_hint: "UI/package.json|crates/clawd/Cargo.toml".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context))
                .is_none()
        );
    }

    #[test]
    fn direct_scalar_defers_recent_structured_scalar_equality_to_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","field_path":"name","exists":true,"value_text":"RustClaw","value":"RustClaw","value_type":"string"}"#,
        ));
        loop_state.executed_step_results.push(ok_step(
            "step_2",
            "system_basic",
            r#"{"action":"extract_field","field_path":"crate_name","exists":true,"value_text":"rustclaw","value":"rustclaw","value_type":"string"}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "Are those two names the same? Answer same or different".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:same_or_different".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::RecentScalarEqualityCheck,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert!(extract_direct_scalar_from_generic_output_i18n(
            &loop_state,
            &AppState::test_default_with_fixture_provider(),
            Some(&agent_run_context)
        )
        .is_none());
    }

    #[test]
    fn structured_pair_answer_does_not_infer_fields_from_read_file_outputs() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "read_file",
            r#"{"name":"react-example","version":"0.0.0"}"#,
        ));
        loop_state.executed_step_results.push(ok_step(
            "step_2",
            "read_file",
            r#"[package]
name = "clawd"
version.workspace = true
"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent:
                "读取 UI/package.json 里的 name 字段，再读取 crates/clawd/Cargo.toml 里的 package.name 字段，最后用一行输出：前者、后者、一样或不一样"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:same_or_different".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::RecentScalarEqualityCheck,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let _agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            super::recent_structured_scalar_observation_count(&loop_state),
            0
        );
    }

    #[test]
    fn direct_scalar_reports_missing_extract_field_as_readable_message() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":false,"field_path":"name","value_text":"","value":null,"value_type":"null"}"#,
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
            Some("未找到 name 字段")
        );
    }

    #[test]
    fn internal_missing_sentinel_uses_structured_extract_field_evidence() {
        let state = AppState::test_default_with_fixture_provider();
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":false,"field_path":"package.name","value_text":"","value":null,"value_type":"null"}"#,
        ));

        assert_eq!(
            replace_internal_missing_sentinel_with_structured_observation(
                "<missing>",
                &state,
                &loop_state,
                None
            )
            .as_deref(),
            Some("未找到 package.name 字段")
        );
        assert_eq!(
            replace_internal_missing_sentinel_with_structured_observation(
                "package.name: <missing>",
                &state,
                &loop_state,
                None
            )
            .as_deref(),
            Some("未找到 package.name 字段")
        );
    }

    #[test]
    fn direct_scalar_missing_field_language_uses_original_request_before_resolved_prompt() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":false,"field_path":"name","value_text":"","value":null,"value_type":"null"}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "Read the name field from package.json and output only its value."
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:field_extract".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: "package.json".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            original_user_request: Some("读取 package.json 里的 name 字段，只输出值".to_string()),
            user_request: Some(
                "Read the name field from package.json and output only its value.".to_string(),
            ),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_scalar_from_generic_output_i18n(
                &loop_state,
                &AppState::test_default_with_fixture_provider(),
                Some(&agent_run_context),
            )
            .as_deref(),
            Some("未找到 name 字段")
        );
    }

    #[test]
    fn direct_scalar_defers_count_inventory_total_with_component_breakdown_to_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"count_inventory","counts":{"total":12,"files":9,"dirs":3}}"#,
        ));
        assert!(extract_direct_scalar_from_generic_output(&loop_state, None).is_none());
    }

    #[test]
    fn direct_scalar_reads_count_inventory_single_dimension_from_structured_output() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"count_inventory","kind_filter":"file","counts":{"total":12,"files":9,"dirs":3}}"#,
        ));
        let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
        route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
        let agent_run_context = AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("9")
        );
    }

    #[test]
    fn direct_count_inventory_uses_total_when_response_contract_is_known() {
        let value = serde_json::json!({
            "action": "count_inventory",
            "counts": {"total": 66, "files": 40, "dirs": 26},
            "path": ".",
            "recursive": false
        });

        assert!(
            super::count_inventory_direct_answer_candidate(None, &value, None, false,).is_none()
        );

        assert_eq!(
            super::count_inventory_direct_answer_candidate(
                None,
                &value,
                Some(OutputResponseShape::Scalar),
                false,
            )
            .as_deref(),
            Some("66")
        );

        let one_sentence = super::count_inventory_direct_answer_candidate(
            None,
            &value,
            Some(OutputResponseShape::OneSentence),
            false,
        )
        .expect("one-sentence count answer");
        assert!(one_sentence.contains("66"));
    }

    #[test]
    fn inventory_dir_grouped_contract_uses_names_by_kind() {
        let value = serde_json::json!({
            "action": "inventory_dir",
            "names_only": true,
            "names": ["Cargo.toml", "src", "README.md"],
            "names_by_kind": {
                "files": ["Cargo.toml", "README.md"],
                "dirs": ["src"],
                "other": []
            },
            "counts": {"files": 2, "dirs": 1, "total": 3}
        });
        let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
        route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;

        let answer = inventory_dir_direct_answer_candidate(None, Some(&route), &value, false)
            .expect("grouped inventory answer");

        assert!(answer.contains("目录:"));
        assert!(answer.contains("- src"));
        assert!(answer.contains("文件:"));
        assert!(answer.contains("- Cargo.toml"));
        assert!(answer.contains("- README.md"));
    }

    #[test]
    fn direct_count_inventory_answer_uses_file_count_and_explanation_for_one_sentence() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"count_inventory","counts":{"total":53,"files":53,"dirs":0},"kind_filter":"file","path":".","recursive":false}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "数一下当前目录一级有多少个普通文件，只告诉我数字和一句解释"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "scalar_count".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Low,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ScalarCount,
                locator_hint: ".".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            original_user_request: Some(
                "数一下当前目录一级有多少个普通文件，只告诉我数字和一句解释".to_string(),
            ),
            ..AgentRunContext::default()
        };

        let answer =
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .expect("count_inventory should produce a direct count answer");

        assert!(answer.contains("53"));
        assert!(answer.contains("普通文件"));
        assert!(!answer.contains("无法计数"));
    }

    #[test]
    fn direct_scalar_prefers_unique_exact_fs_search_match_path() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","pattern":"README.md","count":5,"results":["RUSTCLAW_SERVICE_README.md","UI/README.md","README.md","pi_app/README.md","skill_develop/README.md"],"root":""}"#,
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
            Some("README.md")
        );
    }

    #[test]
    fn direct_scalar_uses_locator_hint_when_fs_search_output_omits_pattern() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","count":5,"results":["RUSTCLAW_SERVICE_README.md","UI/README.md","README.md","pi_app/README.md","skill_develop/README.md"],"root":""}"#,
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output_with_locator_hint(
                &loop_state,
                Some("README.md"),
                None,
                false,
            )
            .as_deref(),
            Some("README.md")
        );
    }

    #[test]
    fn direct_scalar_does_not_collapse_ambiguous_fs_search_to_count() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","pattern":"README","count":2,"results":["README.md","README.txt"],"root":""}"#,
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, None),
            None
        );
    }

    #[test]
    fn direct_scalar_prefers_locator_extension_when_fs_search_pattern_is_broad() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","pattern":"execution_intent","count":2,"results":["plan/execution_intent_route_trace_cases.txt","plan/execution_intent_routing_repair_plan_20260509.md"],"root":"plan"}"#,
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output_with_locator_hint(
                &loop_state,
                Some("plan/extra_missing_repair_probe.md"),
                None,
                false,
            )
            .as_deref(),
            Some("plan/execution_intent_routing_repair_plan_20260509.md")
        );
    }

    #[test]
    fn fs_search_file_paths_contract_filters_broad_pattern_with_route_semantics() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","pattern":"plan","count":8,"results":["crates/clawd/src/agent_engine/planning.rs","docs/planning_deterministic_guardrails_audit.md","plan/agent_intelligence_architecture_plan_20260511_已完成.md","plan/builtin_skill_capability_governance_plan_20260510.md","plan/codex_style_agent_architecture_refactor_plan_20260511.md","plan/execution_intent_routing_repair_plan_20260509_已完成.md","plan/llm_first_agent_convergence_plan_20260511.md","prompts/layers/overlays/plan_repair_prompt.md"],"root":""}"#,
        ));
        let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
        route.ask_mode = crate::AskMode::planner_execute_plain();
        route.resolved_intent = "Read plan/definitely_missing_20260511.md; if missing, search the plan directory for md files related to execution_intent and return only found paths.".to_string();
        route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "/home/guagua/rustclaw/plan".to_string();
        let agent_run_context = AgentRunContext {
            route_result: Some(route),
            ..AgentRunContext::default()
        };

        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("plan/execution_intent_routing_repair_plan_20260509_已完成.md")
        );
    }

    #[test]
    fn fs_search_file_paths_contract_preserves_multi_candidates_when_not_decisive() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"find_name","pattern":"README","count":5,"results":["README.md","README.zh-CN.md","UI/README.md","data/vendor/whisper.cpp/examples/whisper.android.java/README_files","data/vendor/whisper.cpp/README.md"],"root":""}"#,
        ));
        let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
        route.ask_mode = crate::AskMode::planner_execute_plain();
        route.resolved_intent =
            "Find files named README under the current repo. If there are multiple candidates, list candidates instead of choosing one."
                .to_string();
        route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint = "/home/guagua/rustclaw".to_string();
        let agent_run_context = AgentRunContext {
            route_result: Some(route),
            ..AgentRunContext::default()
        };

        let answer =
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .expect("multi-candidate search should produce a direct candidate list");

        assert!(answer.contains("README.md"));
        assert!(answer.contains("README.zh-CN.md"));
        assert!(
            answer.contains('\n'),
            "answer should not collapse to one path: {answer}"
        );
        assert_ne!(
            answer.trim(),
            "data/vendor/whisper.cpp/examples/whisper.android.java/README_files"
        );
    }

    #[test]
    fn direct_scalar_count_uses_latest_fs_search_count() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"count_inventory","counts":{"total":107,"files":101,"dirs":6},"path":"scripts/nl_tests/cases"}"#,
        ));
        loop_state.executed_step_results.push(ok_step(
            "step_2",
            "fs_search",
            r#"{"action":"find_name","count":10,"patterns":["clarify"],"results":["a.txt","b.txt"],"root":"scripts/nl_tests/cases"}"#,
        ));
        let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
        route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;

        let agent_run_context = AgentRunContext {
            route_result: Some(route),
            ..AgentRunContext::default()
        };

        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("10")
        );
    }

    #[test]
    fn fs_search_find_ext_direct_answer_returns_paths_list() {
        let value = serde_json::json!({
            "action": "find_ext",
            "ext": "toml",
            "count": 3,
            "results": ["Cargo.toml", "configs/config.toml", "configs/git_basic.toml"]
        });
        assert_eq!(
            super::fs_search_direct_answer_candidate(None, &value, None, false, true, false)
                .as_deref(),
            Some("Cargo.toml\nconfigs/config.toml\nconfigs/git_basic.toml")
        );
    }

    #[test]
    fn fs_search_grep_text_direct_answer_returns_unique_matching_paths() {
        let value = serde_json::json!({
            "action": "grep_text",
            "query": "FirstLayerDecision",
            "count": 1,
            "match_count": 2,
            "matches": [
                {"path": "README.md", "line": 45, "text": "FirstLayerDecision"},
                {"path": "README.md", "line": 95, "text": "FirstLayerDecision"}
            ]
        });

        assert_eq!(
            super::fs_search_direct_answer_candidate(None, &value, None, false, false, false)
                .as_deref(),
            Some("README.md")
        );
    }

    #[test]
    fn fs_search_grep_text_direct_answer_preserves_path_answer_when_requested() {
        let value = serde_json::json!({
            "action": "grep_text",
            "query": "FirstLayerDecision",
            "count": 4,
            "match_count": 5,
            "matches": [
                {"path": "README.md", "line": 45, "text": "FirstLayerDecision"},
                {"path": "README.md", "line": 95, "text": "FirstLayerDecision"},
                {"path": "crates/clawd/src/ask_flow.rs", "line": 10, "text": "FirstLayerDecision"},
                {"path": "crates/clawd/src/intent_router.rs", "line": 20, "text": "FirstLayerDecision"},
                {"path": "crates/clawd/src/main.rs", "line": 30, "text": "FirstLayerDecision"}
            ]
        });

        assert_eq!(
            super::fs_search_direct_answer_candidate(None, &value, None, false, true, true)
                .as_deref(),
            Some("README.md\ncrates/clawd/src/ask_flow.rs\ncrates/clawd/src/intent_router.rs")
        );
    }

    #[test]
    fn fs_search_grep_text_direct_answer_returns_matching_lines_when_listing_allowed() {
        let value = serde_json::json!({
            "action": "grep_text",
            "query": "ERROR",
            "count": 1,
            "match_count": 1,
            "matches": [
                {
                    "path": "logs/app.log",
                    "line": 16,
                    "text": "2026-04-01 10:08:44 ERROR provider timeout while fetching external metadata"
                }
            ]
        });

        assert_eq!(
            super::fs_search_direct_answer_candidate(None, &value, None, false, true, false)
                .as_deref(),
            Some("16: 2026-04-01 10:08:44 ERROR provider timeout while fetching external metadata")
        );
    }

    #[test]
    fn fs_search_grep_text_direct_answer_uses_name_matches_when_content_empty() {
        let value = serde_json::json!({
            "action": "grep_text",
            "query": "abcd",
            "count": 0,
            "match_count": 0,
            "matches": [],
            "name_count": 4,
            "name_results": [
                "abcd_report.md",
                "my_abcd.txt",
                "x_abcd_log.txt",
                "zz_abcd_backup.log"
            ]
        });

        assert_eq!(
            super::fs_search_direct_answer_candidate(None, &value, None, false, true, false)
                .as_deref(),
            Some("abcd_report.md\nmy_abcd.txt\nx_abcd_log.txt")
        );
    }

    #[test]
    fn virtual_fs_basic_grep_text_output_can_direct_answer_file_paths() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"grep_text","query":"FirstLayerDecision","count":4,"match_count":5,"matches":[{"path":"README.md","line":45,"text":"FirstLayerDecision"},{"path":"README.md","line":95,"text":"FirstLayerDecision"},{"path":"crates/clawd/src/ask_flow.rs","line":10,"text":"FirstLayerDecision"},{"path":"crates/clawd/src/intent_router.rs","line":20,"text":"FirstLayerDecision"},{"path":"crates/clawd/src/main.rs","line":30,"text":"FirstLayerDecision"}]}"#,
        ));
        let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
        route_result.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
        route_result.output_contract.requires_content_evidence = true;
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };

        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("README.md\ncrates/clawd/src/ask_flow.rs\ncrates/clawd/src/intent_router.rs")
        );
    }

    #[test]
    fn fs_search_grep_text_observed_body_keeps_line_evidence() {
        let body = r#"{"action":"grep_text","query":"run_cmd","patterns":["prompt_utils.rs"],"count":1,"match_count":2,"matches":[{"path":"crates/clawd/src/prompt_utils.rs","line":1275,"text":"if step_type == \"run_cmd\" {"},{"path":"crates/clawd/src/prompt_utils.rs","line":1276,"text":"return normalize_run_cmd_call(obj, obj.get(\"args\").and_then(|v| v.as_object()));"}]}"#;
        let observed = super::structured_observed_body("fs_search", body)
            .expect("grep_text should compact observed evidence");

        assert!(observed.contains("grep_text query=run_cmd"));
        assert!(observed.contains("file_patterns=prompt_utils.rs"));
        assert!(observed.contains("match path=crates/clawd/src/prompt_utils.rs line=1275"));
        assert!(observed.contains("step_type == \"run_cmd\""));
    }

    #[test]
    fn fs_search_grep_text_observed_body_keeps_name_match_fallback() {
        let body = r#"{"action":"grep_text","query":"abcd","count":0,"match_count":0,"matches":[],"name_count":1,"name_results":["my_abcd.txt"]}"#;
        let observed = super::structured_observed_body("fs_search", body)
            .expect("grep_text should compact name fallback evidence");

        assert!(observed.contains("grep_text query=abcd"));
        assert!(observed.contains("name_count=1"));
        assert!(observed.contains("name_match path=my_abcd.txt"));
        assert!(observed.contains("matches: none"));
    }

    #[test]
    fn fs_search_find_ext_directory_contract_returns_parent_dirs() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_ext","ext":"sh","count":4,"results":["system_report.sh","scripts/run.sh","scripts/dev/check.sh","component_start/start-clawd.sh"],"root":""}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "list directories containing sh files".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                semantic_kind: OutputSemanticKind::DirectoryNames,
                ..IntentOutputContract::default()
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some("/home/guagua/rustclaw".to_string()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(".\nscripts\nscripts/dev\ncomponent_start")
        );
    }

    #[test]
    fn virtual_fs_basic_find_ext_directory_contract_returns_parent_dirs() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"find_ext","ext":"sh","count":4,"results":["system_report.sh","scripts/run.sh","scripts/dev/check.sh","component_start/start-clawd.sh"],"root":""}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "list unique directories containing sh scripts".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                semantic_kind: OutputSemanticKind::DirectoryNames,
                ..IntentOutputContract::default()
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some("/home/guagua/rustclaw".to_string()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(".\nscripts\nscripts/dev\ncomponent_start")
        );
    }

    #[test]
    fn fs_search_direct_answer_does_not_confirm_ambiguous_matches_when_direct_list_disallowed() {
        let value = serde_json::from_str::<serde_json::Value>(
            r#"{"action":"find_name","pattern":"abcd","count":4,"results":["abcd_report.md","my_abcd.txt","x_abcd_log.txt","zz_abcd_backup.log"],"root":""}"#,
        )
        .expect("json");
        assert_eq!(
            super::fs_search_direct_answer_candidate(None, &value, None, false, false, false)
                .as_deref(),
            None
        );
        assert_eq!(
            super::fs_search_direct_answer_candidate(None, &value, None, false, true, false)
                .as_deref(),
            Some("abcd_report.md\nmy_abcd.txt\nx_abcd_log.txt")
        );
    }

    #[test]
    fn fs_search_direct_answer_prefers_exact_match_before_confirmation() {
        let value = serde_json::from_str::<serde_json::Value>(
            r#"{"action":"find_name","pattern":"README.md","count":5,"results":["RUSTCLAW_SERVICE_README.md","UI/README.md","README.md","pi_app/README.md","skill_develop/README.md"],"root":""}"#,
        )
        .expect("json");
        assert_eq!(
            super::fs_search_direct_answer_candidate(None, &value, None, false, false, false)
                .as_deref(),
            Some("有，路径：README.md")
        );
        assert_eq!(
            super::fs_search_direct_answer_candidate(None, &value, None, false, false, true)
                .as_deref(),
            Some("README.md")
        );
    }

    #[test]
    fn direct_answer_for_strict_file_names_fs_search_uses_plain_path() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","count":1,"results":["scripts/nl_tests/fixtures/locator_smart/stem_unique/ABCD.txt"],"root":"scripts/nl_tests/fixtures/locator_smart/stem_unique"}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "在目标目录里找 abcd，只输出路径".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Strict,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::FileNames,
                locator_hint: "scripts/nl_tests/fixtures/locator_smart/stem_unique".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("scripts/nl_tests/fixtures/locator_smart/stem_unique/ABCD.txt")
        );
    }

    #[test]
    fn fs_search_direct_answer_uses_locator_hint_for_ambiguous_list_when_allowed() {
        let value = serde_json::from_str::<serde_json::Value>(
            r#"{"action":"find_name","count":4,"results":["scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt"],"root":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3"}"#,
        )
        .expect("json");
        assert_eq!(
            super::fs_search_direct_answer_candidate(
                None,
                &value,
                Some("abcd"),
                false,
                false,
                false
            )
            .as_deref(),
            None
        );
        assert_eq!(
            super::fs_search_direct_answer_candidate(
                None,
                &value,
                Some("abcd"),
                false,
                true,
                false
            )
            .as_deref(),
            Some(
                "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt"
            )
        );
    }

    #[test]
    fn observed_entries_keep_latest_listing_plus_recent_non_listing_steps() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "list_dir", "a.md\nb.md\nc.md\n"));
        loop_state
            .executed_step_results
            .push(ok_step("step_2", "read_file", "# A\nalpha\n"));
        loop_state
            .executed_step_results
            .push(ok_step("step_3", "read_file", "# B\nbeta\n"));
        loop_state
            .executed_step_results
            .push(ok_step("step_4", "read_file", "# C\ngamma\n"));
        loop_state
            .executed_step_results
            .push(ok_step("step_5", "read_file", "# D\ndelta\n"));
        loop_state
            .executed_step_results
            .push(ok_step("step_6", "read_file", "# E\nepsilon\n"));

        let entries = observed_output_entries(&loop_state);
        assert_eq!(entries.len(), 5);
        assert!(entries
            .iter()
            .any(|entry| entry.contains("step_1 skill(list_dir)")));
        assert!(entries
            .iter()
            .any(|entry| entry.contains("step_6 skill(read_file)")));
        assert!(!entries
            .iter()
            .any(|entry| entry.contains("step_2 skill(read_file)")));
    }

    #[test]
    fn normalized_listing_trims_blank_lines() {
        assert_eq!(
            normalized_observed_listing("\nfoo\n\nbar\n").as_deref(),
            Some("foo\nbar")
        );
    }

    #[test]
    fn observed_entries_use_read_range_excerpt_body_instead_of_raw_json() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/README.md","excerpt":"1|# RustClaw\n2|\n3|Hello"}"#,
        ));
        let entries = observed_output_entries(&loop_state);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].contains("read_range path=/tmp/README.md"));
        assert!(entries[0].contains("# RustClaw"));
        assert!(entries[0].contains("# RustClaw\n\nHello"));
        assert!(entries[0].contains("Hello"));
        assert!(!entries[0].contains(r#""action":"read_range""#));
    }

    #[test]
    fn observed_contract_json_includes_semantic_kind_and_locator_hint() {
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "读一下 README.md 开头，然后用一句话总结".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Filename,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ContentExcerptSummary,
                locator_hint: "README.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        let contract = observed_contract_json(Some(&agent_run_context));
        assert!(contract.contains(r#""semantic_kind":"content_excerpt_summary""#));
        assert!(contract.contains(r#""locator_hint":"README.md""#));
    }

    #[test]
    fn observed_request_language_hint_follows_current_user_text() {
        assert_eq!(
            observed_request_language_hint("读一下 README 开头，三句话总结"),
            "zh-CN"
        );
        assert_eq!(
            observed_request_language_hint("Summarize the README in one sentence."),
            "en"
        );
        assert_eq!(observed_request_language_hint("只输出路径"), "zh-CN");
        assert_eq!(observed_request_language_hint("12345"), "config_default");
    }

    #[test]
    fn observed_response_style_hint_reflects_output_contract_shape() {
        let mut route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "读一下 README.md 开头，然后用一句话总结".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Filename,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ContentExcerptSummary,
                locator_hint: "README.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let mut agent_run_context = AgentRunContext {
            route_result: Some(route_result.clone()),
            ..AgentRunContext::default()
        };
        assert!(
            observed_response_style_hint(Some(&agent_run_context)).contains("exactly one sentence")
        );

        route_result.output_contract.exact_sentence_count = Some(3);
        agent_run_context.route_result = Some(route_result.clone());
        assert!(
            observed_response_style_hint(Some(&agent_run_context)).contains("exactly 3 sentences")
        );
        route_result.output_contract.exact_sentence_count = None;

        route_result.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
        route_result.output_contract.response_shape = OutputResponseShape::Strict;
        route_result.output_contract.exact_sentence_count = Some(1);
        agent_run_context.route_result = Some(route_result.clone());
        assert!(observed_response_style_hint(Some(&agent_run_context)).contains("key=value"));
        route_result.output_contract.exact_sentence_count = None;
        route_result.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;

        route_result.output_contract.response_shape = OutputResponseShape::Scalar;
        agent_run_context.route_result = Some(route_result.clone());
        assert!(observed_response_style_hint(Some(&agent_run_context))
            .contains("only the final scalar value"));

        route_result.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
        route_result.output_contract.response_shape = OutputResponseShape::OneSentence;
        agent_run_context.route_result = Some(route_result.clone());
        assert!(observed_response_style_hint(Some(&agent_run_context))
            .contains("Do not collapse component counts"));

        route_result.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
        route_result.output_contract.response_shape = OutputResponseShape::Free;
        agent_run_context.route_result = Some(route_result.clone());
        assert!(
            observed_response_style_hint(Some(&agent_run_context)).contains("short direct answer")
        );

        route_result.output_contract.response_shape = OutputResponseShape::FileToken;
        agent_run_context.route_result = Some(route_result);
        assert!(observed_response_style_hint(Some(&agent_run_context)).contains("delivery token"));
    }

    #[test]
    fn chat_wrapped_free_unclassified_contract_allows_finalizer_passthrough() {
        let route = chat_wrapped_unclassified_route(OutputResponseShape::Free);
        assert!(!route_requires_synthesized_delivery(&route));

        let agent_run_context = AgentRunContext {
            route_result: Some(route),
            ..AgentRunContext::default()
        };
        let contract = observed_contract_json(Some(&agent_run_context));
        assert!(contract.contains(r#""direct_observation_passthrough_allowed":true"#));
        assert!(
            observed_response_style_hint(Some(&agent_run_context)).contains("short direct answer")
        );
    }

    #[test]
    fn chat_wrapped_one_sentence_unclassified_contract_requires_synthesized_delivery() {
        let route = chat_wrapped_unclassified_route(OutputResponseShape::OneSentence);
        assert!(route_requires_synthesized_delivery(&route));

        let agent_run_context = AgentRunContext {
            route_result: Some(route),
            ..AgentRunContext::default()
        };
        let contract = observed_contract_json(Some(&agent_run_context));
        assert!(contract.contains(r#""direct_observation_passthrough_allowed":false"#));
        assert!(observed_response_style_hint(Some(&agent_run_context))
            .contains("Do not answer by copying only the raw observed output"));
    }

    #[test]
    fn chat_wrapped_strict_exact_sentence_contract_requires_synthesized_delivery() {
        let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
        route.output_contract.exact_sentence_count = Some(1);
        assert!(route_requires_synthesized_delivery(&route));

        let agent_run_context = AgentRunContext {
            route_result: Some(route),
            ..AgentRunContext::default()
        };
        let contract = observed_contract_json(Some(&agent_run_context));
        assert!(contract.contains(r#""direct_observation_passthrough_allowed":false"#));
        assert!(observed_response_style_hint(Some(&agent_run_context))
            .contains("Do not answer by copying only the raw observed output"));
    }

    #[test]
    fn strict_plain_observation_contract_allows_passthrough() {
        let route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
        assert!(!route_requires_synthesized_delivery(&route));

        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "model_io.log.2026-05-14 215M\nmodel_io.log.2026-05-11 149M\n",
        ));
        let agent_run_context = AgentRunContext {
            route_result: Some(route),
            ..AgentRunContext::default()
        };

        let answer =
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .expect("strict plain observation passthrough");

        assert_eq!(
            answer,
            "model_io.log.2026-05-14 215M\nmodel_io.log.2026-05-11 149M"
        );
    }

    #[test]
    fn raw_command_contract_allows_observation_passthrough() {
        let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
        route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
        assert!(!route_requires_synthesized_delivery(&route));

        let agent_run_context = AgentRunContext {
            route_result: Some(route),
            ..AgentRunContext::default()
        };
        let contract = observed_contract_json(Some(&agent_run_context));
        assert!(contract.contains(r#""direct_observation_passthrough_allowed":true"#));
    }

    #[test]
    fn direct_observation_passthrough_detector_matches_raw_output() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "/home/guagua/rustclaw\n",
        ));

        assert!(answer_is_direct_observation_passthrough(
            "/home/guagua/rustclaw",
            &loop_state
        ));
        assert!(!answer_is_direct_observation_passthrough(
            "Working directory: /home/guagua/rustclaw",
            &loop_state
        ));
    }

    #[test]
    fn route_observation_facts_pin_resolved_path_for_existence_summary() {
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "check service file and explain purpose".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Strict,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Filename,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ExistenceWithPathSummary,
                locator_hint: "rustclaw.service".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let ctx = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some("/home/guagua/rustclaw/rustclaw.service".to_string()),
            ..AgentRunContext::default()
        };

        let facts = route_observation_facts_entry(Some(&ctx)).expect("route facts");

        assert!(facts.contains("resolved_target_path: /home/guagua/rustclaw/rustclaw.service"));
        assert!(facts.contains("do not infer the target path from file content fields"));
    }

    #[test]
    fn observed_fallback_prompt_renders_language_and_response_style_hints() {
        let prompt = crate::render_prompt_template(
            OBSERVED_ANSWER_FALLBACK_PROMPT_TEMPLATE,
            &[
                ("__USER_REQUEST__", "读一下 README 开头，然后用一句话总结"),
                (
                    "__RESOLVED_USER_INTENT__",
                    "读一下 README 开头，然后用一句话总结",
                ),
                (
                    "__OUTPUT_CONTRACT__",
                    r#"{"response_shape":"one_sentence","semantic_kind":"content_excerpt_summary"}"#,
                ),
                (
                    "__OBSERVED_OUTPUTS__",
                    "### step_1 skill(read_file)\n# RustClaw",
                ),
                ("__CONFIG_RESPONSE_LANGUAGE__", "zh-CN"),
                ("__REQUEST_LANGUAGE_HINT__", "mixed"),
                (
                    "__RESPONSE_STYLE_HINT__",
                    "Return exactly one sentence unless the current user request explicitly asks for another exact sentence count.",
                ),
            ],
        );
        assert!(prompt.contains("Request language hint:\nmixed"));
        assert!(prompt.contains("Response style hint:"));
        assert!(prompt.contains("Return exactly one sentence"));
        assert!(prompt.contains("Do not collapse multi-dimensional structured evidence"));
    }

    #[test]
    fn markdown_non_json_fallback_prefers_text_outside_code_fences() {
        let answer = super::non_code_markdown_text(
            "```bash\n#!/usr/bin/env bash\nset -euo pipefail\n```\n\n这个脚本用于重启 clawd 服务。",
        );
        assert_eq!(answer.as_deref(), Some("这个脚本用于重启 clawd 服务。"));
    }

    #[test]
    fn content_excerpt_summary_is_not_hard_summarized_by_observed_output() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/config.toml","resolved_path":"/tmp/config.toml","excerpt":"12|# timeout note\n13|task_timeout_seconds = 3600\n14|# end"}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "读取 /tmp/config.toml 最后 3 行，然后用一句话总结".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ContentExcerptSummary,
                locator_hint: "/tmp/config.toml".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_keeps_fallback_for_unstructured_content_excerpt_summary() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "read_file",
            "RustClaw is deployed locally and keeps task state in sqlite.",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "看一下 /tmp/README.txt，然后用一句话总结".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ContentExcerptSummary,
                locator_hint: "/tmp/README.txt".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some("/tmp/README.txt".to_string()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_summarizes_doc_parse_content_excerpt_without_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "doc_parse",
            r##"{"text":"# RustClaw\n\n<img src=\"./RustClaw.png\" width=\"420\" />\n\nRustClaw is a local Rust agent runtime centered on clawd and designed for multi-channel task execution.\n\n## Overview\nMore text."}"##,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "读取 README.md 并总结一行".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ContentExcerptSummary,
                locator_hint: "README.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(
                "RustClaw is a local Rust agent runtime centered on clawd and designed for multi-channel task execution."
            )
        );
    }

    #[test]
    fn direct_answer_passthroughs_contract_filename_read_range_excerpt_without_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/README.md","resolved_path":"/tmp/README.md","excerpt":"1|# RustClaw\n2|\n3|<img src=\"./RustClaw.png\" width=\"420\" />\n4|"}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "先读一下 README.md 前 4 行".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Filename,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::None,
                locator_hint: "README.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some("/tmp/README.md".to_string()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("# RustClaw\n（空行）\n<img src=\"./RustClaw.png\" width=\"420\" />\n（空行）")
        );
    }

    #[test]
    fn direct_answer_preserves_blank_lines_for_explicit_read_range() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","mode":"range","start_line":1,"end_line":4,"path":"/tmp/README.md","resolved_path":"/tmp/README.md","excerpt":"1|# RustClaw\n2|\n3|<img src=\"./RustClaw.png\" width=\"420\" />\n4|"}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "Show exactly the first 4 raw lines of README.md.".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Strict,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Filename,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ContentExcerptSummary,
                locator_hint: "README.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some("/tmp/README.md".to_string()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("# RustClaw\n\n<img src=\"./RustClaw.png\" width=\"420\" />\n")
        );
    }

    #[test]
    fn direct_answer_sanitizes_read_range_log_excerpt_without_llm() {
        let mut loop_state = LoopState::new(2);
        let skill_output = serde_json::json!({
            "action": "read_range",
            "path": "/tmp/feishud.log",
            "resolved_path": "/tmp/feishud.log",
            "excerpt": "1|\u{1b}[32mconnected\u{1b}[0m to wss://host/ws?device_id=123&access_key=abc123&service_id=7&ticket=deadbeef"
        })
        .to_string();
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "system_basic", &skill_output));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "看日志最后 1 行".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Filename,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::None,
                locator_hint: "feishud.log".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some("/tmp/feishud.log".to_string()),
            ..AgentRunContext::default()
        };

        let answer =
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .expect("read_range direct answer");

        assert!(answer.contains("access_key=[REDACTED]"));
        assert!(answer.contains("ticket=[REDACTED]"));
        assert!(!answer.contains('\u{1b}'));
        assert!(!answer.contains("abc123"));
        assert!(!answer.contains("deadbeef"));
    }

    #[test]
    fn scalar_route_fs_basic_tail_read_range_prefers_structured_excerpt() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "older output mentioning scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
        ));
        let skill_output = serde_json::json!({
            "action": "read_range",
            "path": "/home/guagua/rustclaw/logs/clawd.log",
            "resolved_path": "/home/guagua/rustclaw/logs/clawd.log",
            "mode": "tail",
            "requested_n": 2,
            "excerpt": "1858|2026-05-13T18:29:58Z finalize_ok\n1859|2026-05-13T18:29:59Z prior task mentioned release_checklist.md"
        })
        .to_string();
        loop_state
            .executed_step_results
            .push(ok_step("step_2", "fs_basic", &skill_output));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "查看 logs 目录下第二个文件（clawd.log）的最后2行内容".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::None,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        assert!(scalar_route_prefers_structured_observed_answer(
            &route_result,
            &loop_state
        ));
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };

        let answer =
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .expect("fs_basic read_range direct answer");

        assert!(answer.contains("finalize_ok"));
        assert!(answer.contains("release_checklist.md"));
        assert!(!answer.contains(r#""action":"read_range""#));
        assert!(!answer.contains("older output mentioning"));
    }

    #[test]
    fn direct_answer_passthroughs_chat_wrapped_execution_path_read_range_when_no_transform_is_requested(
    ) {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/config.toml","resolved_path":"/tmp/config.toml","excerpt":"1|[app]\n2|name = \"fixture\"\n3|mode = \"test\""}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "用户提供了文件路径 /tmp/config.toml，但未说明要对该文件执行什么操作"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::None,
                locator_hint: "/tmp/config.toml".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some("/tmp/config.toml".to_string()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("[app]\nname = \"fixture\"\nmode = \"test\"")
        );
    }

    #[test]
    fn direct_answer_does_not_passthrough_read_range_when_summary_is_requested() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/README.md","resolved_path":"/tmp/README.md","excerpt":"1|# RustClaw\n2|\n3|A tool runtime\n4|"}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "先读一下 README.md 前 4 行，再用三句话总结".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:generic_filename_read_range".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Filename,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ContentExcerptSummary,
                locator_hint: "README.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some("/tmp/README.md".to_string()),
            ..AgentRunContext::default()
        };
        assert!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .is_none(),
            "summary-style read_range requests should fall back to synthesis instead of raw passthrough"
        );
    }

    #[test]
    fn direct_answer_does_not_passthrough_read_range_for_existence_with_path_contract() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/rustclaw.service","resolved_path":"/tmp/rustclaw.service","excerpt":"1|[Unit]\n2|Description=RustClaw Service\n3|[Service]\n4|ExecStart=/bin/bash start-all-bin.sh"}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "检查 rustclaw.service 是否存在，若存在返回路径并解释用途".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Strict,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Filename,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ExistenceWithPath,
                locator_hint: "rustclaw.service".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some("/tmp/rustclaw.service".to_string()),
            ..AgentRunContext::default()
        };

        assert!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .is_none(),
            "existence/path contracts with read_range evidence need synthesis, not raw file passthrough"
        );
    }

    #[test]
    fn direct_answer_prefers_current_turn_excerpt_summary_request_over_resolved_intent_drift() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/README.md","resolved_path":"/tmp/README.md","excerpt":"1|# RustClaw\n2|\n3|A tool runtime\n4|"}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "先读一下 README.md 前 4 行".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:generic_filename_read_range".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Filename,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ContentExcerptSummary,
                locator_hint: "README.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            user_request: Some("先读一下 README.md 前 4 行，再用三句话总结".to_string()),
            route_result: Some(route_result),
            auto_locator_path: Some("/tmp/README.md".to_string()),
            ..AgentRunContext::default()
        };
        assert!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .is_none(),
            "current-turn summary/read-range request should still block raw passthrough even if resolved_intent drifted"
        );
    }

    #[test]
    fn direct_answer_formats_structured_keys_result_without_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"structured_keys","path":"/tmp/package.json","resolved_path":"/tmp/package.json","field_path":"scripts","exists":true,"container_type":"object","count":3,"keys":["build","dev","lint"]}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "读 /tmp/package.json，告诉我 scripts 字段下都有哪些子键".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:generic_explicit_path_structured_keys".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::None,
                locator_hint: "/tmp/package.json".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("build\ndev\nlint")
        );
    }

    #[test]
    fn structured_keys_one_sentence_defers_to_synthesis() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"structured_keys","path":"/tmp/package.json","resolved_path":"/tmp/package.json","field_path":"scripts","exists":true,"container_type":"object","count":3,"keys":["build","dev","lint"]}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "读 /tmp/package.json，用一句话告诉我 scripts 字段下有哪些子键"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:generic_explicit_path_structured_keys".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::None,
                locator_hint: "/tmp/package.json".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_formats_extract_fields_result_without_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_fields","path":"/tmp/config.toml","resolved_path":"/tmp/config.toml","count":2,"results":[{"field_path":"database.sqlite_path","exists":true,"value_type":"string","value_text":"data/rustclaw.db","value":"data/rustclaw.db"},{"field_path":"tools.allow_sudo","exists":true,"value_type":"bool","value_text":"true","value":true}]}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent:
                "读取 /tmp/config.toml 里的 database.sqlite_path 和 tools.allow_sudo，告诉我两个字段的值"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:generic_explicit_path_extract_fields"
                .to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::None,
                locator_hint: "/tmp/config.toml".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("database.sqlite_path: data/rustclaw.db\ntools.allow_sudo: true")
        );
    }

    #[test]
    fn direct_answer_uses_inventory_dir_names_for_system_basic() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/logs","resolved_path":"/tmp/logs","names_only":true,"names":["act_plan.log","clawd.log","feishud.log"]}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "列出 logs 目录下前 5 个文件名".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::FileNames,
                locator_hint: "logs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("act_plan.log\nclawd.log\nfeishud.log")
        );
    }

    #[test]
    fn direct_answer_uses_inventory_dir_names_for_fs_basic() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"inventory_dir","path":"/tmp/document","resolved_path":"/tmp/document","files_only":true,"names_only":true,"names":["a.txt","b.md","c.png"]}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "List file names from a known directory.".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::FileNames,
                locator_hint: "document".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("a.txt\nb.md\nc.png")
        );
    }

    #[test]
    fn direct_answer_uses_inventory_dir_entry_sizes_when_names_only_is_false() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/logs","resolved_path":"/tmp/logs","names_only":false,"entries":[{"name":"act_plan.log","kind":"file","size_bytes":2467002},{"name":"clawd.run.log","kind":"file","size_bytes":397321},{"name":"clawd.log","kind":"file","size_bytes":2035}],"names":["act_plan.log","clawd.run.log","clawd.log"]}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "列出 logs 目录下最大的 3 个文件，输出文件名和大小".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Strict,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::FileNames,
                locator_hint: "logs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("act_plan.log 2467002\nclawd.run.log 397321\nclawd.log 2035")
        );
    }

    #[test]
    fn direct_answer_does_not_apply_listing_limit_from_resolved_intent_text() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/logs","resolved_path":"/tmp/logs","names_only":true,"names":["a","b","c","d"]}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "列出 logs 目录下前 2 个文件名".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "logs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("a\nb\nc\nd")
        );
    }

    #[test]
    fn direct_answer_does_not_apply_listing_limit_from_current_turn_request_text() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/logs","resolved_path":"/tmp/logs","names_only":true,"names":["a","b","c","d"]}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "列出 logs 目录下的文件名".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "normalizer:planner_execute_chat_wrapped".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "logs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            user_request: Some("列出 logs 目录下前 2 个文件名".to_string()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("a\nb\nc\nd")
        );
    }

    #[test]
    fn scalar_listing_gate_does_not_repair_count_from_request_text_limit() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "list_dir", "a\nb\nc\n"));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "列出 logs 目录下的文件名".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ScalarCount,
                locator_hint: "logs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            user_request: Some("列出 logs 目录下前 2 个文件名，只输出文件名".to_string()),
            ..AgentRunContext::default()
        };
        let route = agent_run_context.route_result.as_ref().unwrap();
        assert!(
            !super::scalar_route_prefers_structured_observed_answer(route, &loop_state,),
            "scalar/listing gate must not infer bounded listing from current-turn request text"
        );
    }

    #[test]
    fn direct_answer_uses_latest_list_dir_entries_for_act_free_shape() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            "README.txt\nnotes.md\n",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "列出 archive 目录下有什么".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "archive".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("README.txt\nnotes.md")
        );
    }

    #[test]
    fn direct_answer_uses_latest_list_dir_even_after_synthesis_step() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "list_dir", "alpha.md\nbeta.md\n"));
        loop_state.executed_step_results.push(ok_step(
            "step_2",
            "synthesize_answer",
            "document 目录下有 alpha.md 和 beta.md。",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "列出 document 目录下有哪些文件，只输出文件名列表".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::FileNames,
                locator_hint: "document".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            user_request: Some("列出 document 目录下有哪些文件，只输出文件名列表".to_string()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("alpha.md\nbeta.md")
        );
    }

    #[test]
    fn direct_answer_preserves_list_dir_entries_without_request_text_limit() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "list_dir", "a\nb\nc\nd\n"));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "列出 logs 目录下前 2 个文件名".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "logs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("a\nb\nc\nd")
        );
    }

    #[test]
    fn direct_answer_defers_hidden_entries_explanation_shape_to_synthesis() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            ".git\nREADME.md\n.env\nsrc\n",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "检查当前目录是否存在隐藏文件，然后用一句话解释隐藏文件的常见用途"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .is_none()
        );
    }

    #[test]
    fn direct_answer_formats_hidden_entries_check_scalar_from_listing() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            ".git\nREADME.md\n.env\nsrc\n",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
                locator_hint: ".".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("2")
        );
    }

    #[test]
    fn direct_answer_defers_hidden_entries_check_strict_shape_to_synthesis() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            ".\n..\n.codex\n.git/\n.gitignore\nREADME.md\n",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Strict,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
                locator_hint: ".".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .is_none()
        );
    }

    #[test]
    fn direct_answer_defers_hidden_entries_check_free_shape_to_synthesis() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            ".cargo/\nREADME.md\n.dockerignore\n.env.example\nsrc\n",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
                locator_hint: ".".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .is_none()
        );
    }

    #[test]
    fn direct_answer_defers_hidden_entries_check_one_sentence_from_system_basic_inventory_dir() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/workspace","resolved_path":"/tmp/workspace","names_only":true,"include_hidden":true,"names":[".cargo",".dockerignore",".env.example","README.md","src"]}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
                locator_hint: ".".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .is_none()
        );
    }

    #[test]
    fn direct_answer_formats_existence_with_path_from_system_basic_path_batch_facts() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"rustclaw.service","resolved_path":"/tmp/rustclaw-workspace/rustclaw.service","size_bytes":1190},"path":"/tmp/rustclaw-workspace/rustclaw.service"}],"include_missing":true}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ExistenceWithPath,
                locator_hint: "rustclaw.service".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("有，路径：/tmp/rustclaw-workspace/rustclaw.service")
        );
    }

    #[test]
    fn direct_answer_formats_scalar_existence_without_path_from_system_basic_path_batch_facts() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"configs/config.toml","resolved_path":"/tmp/repo/configs/config.toml","size_bytes":1190},"path":"/tmp/repo/configs/config.toml"}],"include_missing":true}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "检查 configs/config.toml 是否存在，只回答有或没有".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ExistenceWithPath,
                locator_hint: "configs/config.toml".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("有")
        );
    }

    #[test]
    fn direct_answer_formats_path_batch_facts_requested_size() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"fields":["exists","size"],"facts":[{"exists":true,"fact":{"kind":"file","path":"data/rustclaw.db","resolved_path":"/tmp/repo/data/rustclaw.db","size_bytes":55226368},"path":"/tmp/repo/data/rustclaw.db"}],"include_missing":true}"#,
        ));
        let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
        route_result.ask_mode = crate::AskMode::planner_execute_plain();
        route_result.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
        route_result.output_contract.locator_kind = OutputLocatorKind::Path;
        route_result.output_contract.locator_hint = "data/rustclaw.db".to_string();
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };

        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("yes, path: /tmp/repo/data/rustclaw.db, size: 55226368 bytes")
        );
    }

    #[test]
    fn direct_answer_formats_missing_path_batch_facts_with_reason() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":false,"path":"/tmp/missing.txt","error":"not found"}],"include_missing":true}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "检查文件 /tmp/missing.txt 是否存在，如果不存在，简短说明原因。"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Strict,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ExistenceWithPath,
                locator_hint: "/tmp/missing.txt".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };

        let answer =
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .expect("missing path answer");

        assert!(answer.contains("路径不存在"));
        assert!(answer.contains("/tmp/missing.txt"));
    }

    #[test]
    fn direct_answer_formats_existence_with_path_from_run_cmd_yes_output() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd_observed_exists_yes_{}_{}",
            std::process::id(),
            crate::now_ts_u64()
        ));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        let target = temp_dir.join("rustclaw.service");
        std::fs::write(&target, "ok").expect("write target");
        let expected = format!(
            "有，路径：{}",
            target
                .canonicalize()
                .unwrap_or(target.clone())
                .to_string_lossy()
        );

        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "run_cmd", "yes\n"));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ExistenceWithPath,
                locator_hint: "rustclaw.service".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(expected.as_str())
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn direct_answer_formats_existence_with_path_from_run_cmd_exists_output() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd_observed_exists_lower_{}_{}",
            std::process::id(),
            crate::now_ts_u64()
        ));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        let target = temp_dir.join("rustclaw.service");
        std::fs::write(&target, "ok").expect("write target");
        let expected = format!(
            "有，路径：{}",
            target
                .canonicalize()
                .unwrap_or(target.clone())
                .to_string_lossy()
        );

        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "run_cmd", "exists\n"));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ExistenceWithPath,
                locator_hint: "rustclaw.service".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(expected.as_str())
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn direct_answer_formats_existence_with_path_from_system_basic_find_name_output() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd_observed_exists_find_name_{}_{}",
            std::process::id(),
            crate::now_ts_u64()
        ));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        let target = temp_dir.join("rustclaw.service");
        std::fs::write(&target, "ok").expect("write target");
        let resolved = target
            .canonicalize()
            .unwrap_or(target.clone())
            .to_string_lossy()
            .to_string();

        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"find_name","count":1,"results":["rustclaw.service"],"root":""}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ExistenceWithPath,
                locator_hint: "rustclaw.service".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
            ..AgentRunContext::default()
        };
        let expected = format!("有，路径：{resolved}");
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(expected.as_str())
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn direct_answer_does_not_passthrough_listing_when_content_evidence_is_required() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            "base_skill_response_contract.md\nskill_integration_guide.md\n",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "docs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_does_not_passthrough_inventory_dir_when_content_evidence_is_required() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/docs","resolved_path":"/tmp/docs","names_only":true,"names":["base_skill_response_contract.md","skill_integration_guide.md"]}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "docs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_does_not_passthrough_run_cmd_listing_when_content_evidence_is_required() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd-observed-output-listing-only-{}-{}",
            std::process::id(),
            crate::now_ts_u64()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "run_cmd", "a.md\nb.md\n"));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "docs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn directory_purpose_summary_is_not_hard_classified_by_observed_output() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/docs","resolved_path":"/tmp/docs","names_only":true,"names":["release_checklist.md","operator-guide.md","rollout-summary.pdf"]}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::DirectoryPurposeSummary,
                locator_hint: "docs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn recent_artifacts_judgment_is_not_hard_classified_by_observed_output() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "total 151792\n-rw-r--r--@ 1 testuser staff 76509771 Apr 12 16:30 model_io.log\n-rw-r--r--@ 1 testuser staff 906739 Apr 12 16:30 act_plan.log\n-rw-r--r--@ 1 testuser staff 191187 Apr 12 15:48 service_ops.log\n",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent:
                "列出 logs 目录最近修改的 3 个文件，再告诉我这更像是测试日志还是正式产物"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::RecentArtifactsJudgment,
                locator_hint: "logs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_defers_system_basic_info_summary_to_llm_for_brief_request() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"info","hostname":"rustclaw-test-host.local","os":"macos","arch":"x86_64","cwd":"/tmp/rustclaw-workspace"}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent:
                "show me the basic machine info here like hostname and system, keep it brief"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::RawCommandOutput,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_defers_archive_creation_success_to_synthesis() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "archive_basic",
            "exit=0\nupdating: tmp/rustclaw-workspace/scripts/skill_calls/\n",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent:
                "把 scripts/skill_calls 打成一个 zip 到 tmp/nl_archive_case.zip，然后告诉我是否成功"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ExistenceWithPath,
                locator_hint: "scripts/skill_calls -> tmp/nl_archive_case.zip".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
        assert!(
            has_observed_answer_candidates(&loop_state),
            "archive output should remain available as observed facts for synthesis"
        );
    }

    #[test]
    fn direct_answer_defers_archive_basic_output_destination_to_synthesis() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "archive_basic",
            r#"{"action":"pack","format":"zip","source":"/tmp/rustclaw-workspace/scripts/skill_calls","archive":"/tmp/rustclaw-workspace/tmp/nl_archive_case.zip","output":"exit=0\nupdating: skill_calls/\n"}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent:
                "把 scripts/skill_calls 打成一个 zip 到 tmp/nl_archive_case.zip，然后告诉我是否成功"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ExistenceWithPath,
                locator_hint: "scripts/skill_calls".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
        assert!(
            has_observed_answer_candidates(&loop_state),
            "archive json should remain available as observed facts for synthesis"
        );
    }

    #[test]
    fn direct_answer_defers_system_basic_info_summary_without_action_field() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"hostname":"rustclaw-test-host.local","os":"macos","arch":"x86_64","cwd":"/tmp/rustclaw-workspace"}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent:
                "show me the basic machine info here like hostname and system, keep it brief"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::RawCommandOutput,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_defers_system_basic_info_for_free_shape_request() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"info","hostname":"ThinkPad-X1","os":"linux","arch":"x86_64","cwd":"/home/guagua/rustclaw"}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent:
                "show me the basic machine info here like hostname and system, keep it brief"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::RawCommandOutput,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_extracts_cwd_from_system_basic_info_for_scalar_path_contract() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"info","hostname":"ThinkPad-X1","os":"linux","arch":"x86_64","cwd":"/home/guagua/rustclaw","workspace_root":"/home/guagua/rustclaw"}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "获取当前工作目录的绝对路径".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:scalar_path_only".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ScalarPathOnly,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("/home/guagua/rustclaw")
        );
    }

    #[test]
    fn direct_scalar_path_contract_prefers_recorded_write_file_path() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "/home/guagua/rustclaw",
        ));
        loop_state.executed_step_results.push(ok_step(
            "step_2",
            "write_file",
            "written 40 bytes to /home/guagua/rustclaw/document/pwd_line.txt",
        ));
        loop_state.output_vars.insert(
            "last_file_path".to_string(),
            "/home/guagua/rustclaw/document/pwd_line.txt".to_string(),
        );
        loop_state.last_written_file_path =
            Some("/home/guagua/rustclaw/document/pwd_line.txt".to_string());
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "create the file and send me the file path only".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:scalar_path_only".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ScalarPathOnly,
                locator_hint: "pwd_line.txt".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };

        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("/home/guagua/rustclaw/document/pwd_line.txt")
        );
    }

    #[test]
    fn workspace_project_summary_is_not_hard_summarized_by_observed_output() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            "Cargo.toml\ncrates/\nUI/\nconfigs/\nREADME.md\nREADME.zh-CN.md\nprompts/\nrustclaw.service\ncomponent_start/start-telegramd.sh\ncomponent_start/start-wechatd.sh\ncomponent_start/start-whatsappd.sh\n",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "用非技术用户能听懂的话，简短解释这个仓库主要是干什么的".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_scalar_uses_latest_list_dir_entries_when_listing_is_latest_step() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "list_dir", "README.txt\n"));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
            Some("README.txt")
        );
    }

    #[test]
    fn direct_scalar_path_only_uses_auto_locator_full_path_for_unique_list_dir_match() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd-observed-output-{}-{}",
            std::process::id(),
            crate::now_ts_u64()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("Report.MD");
        std::fs::write(&file_path, "hello").unwrap();

        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "list_dir", "Report.MD\n"));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "去 case_only 找 report.md，只输出路径".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
                locator_hint: "report.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(file_path.to_string_lossy().to_string()),
            ..AgentRunContext::default()
        };
        let resolved = file_path
            .canonicalize()
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(resolved.as_str())
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn direct_scalar_path_only_uses_rooted_full_path_for_unique_find_name_match() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd-observed-output-find-name-{}-{}",
            std::process::id(),
            crate::now_ts_u64()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("Report.MD");
        std::fs::write(&file_path, "hello").unwrap();

        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            &format!(
                r#"{{"action":"find_name","pattern":"report.md","count":1,"results":["Report.MD"],"root":"{}"}}"#,
                temp_dir.display()
            ),
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "去 case_only 找 report.md，只输出路径".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
                locator_hint: "report.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
            ..AgentRunContext::default()
        };
        let resolved = file_path
            .canonicalize()
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(resolved.as_str())
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn system_basic_find_path_normalization_prefers_existing_relative_path() {
        let rel_dir = Path::new("target").join(format!(
            "clawd-observed-output-find-path-{}-{}",
            std::process::id(),
            crate::now_ts_u64()
        ));
        std::fs::create_dir_all(&rel_dir).unwrap();
        let file_path = rel_dir.join("Report.MD");
        std::fs::write(&file_path, "hello").unwrap();
        let cwd = std::env::current_dir().unwrap();
        let resolved_root = cwd.join(&rel_dir).to_string_lossy().to_string();
        let expected = file_path
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string();

        assert_eq!(
            normalize_system_basic_match_path(
                Some(&resolved_root),
                Some(file_path.to_string_lossy().as_ref())
            )
            .as_deref(),
            Some(expected.as_str())
        );
        let _ = std::fs::remove_dir_all(rel_dir);
    }

    #[test]
    fn direct_scalar_path_only_prefers_resolved_path_from_path_batch_facts() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"scripts/nl_tests/fixtures/locator_smart/case_only/Report.MD","resolved_path":"/tmp/case_only/Report.MD","size_bytes":33},"path":"/tmp/case_only/report.md","resolved_from_case_insensitive":true}],"include_missing":true}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "去 case_only 目录里找 report.md，只输出路径".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
                locator_hint: "report.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("/tmp/case_only/Report.MD")
        );
    }

    #[test]
    fn direct_answer_keeps_plain_path_terminal_format_for_observed_path_fact() {
        let mut loop_state = LoopState::new(2);
        loop_state.last_user_visible_respond = Some("/tmp/case_only/Report.MD".to_string());
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"scripts/nl_tests/fixtures/locator_smart/case_only/Report.MD","resolved_path":"/tmp/case_only/Report.MD","size_bytes":33},"path":"/tmp/case_only/Report.MD"}],"include_missing":true}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "去 case_only 目录里找 report.md，只输出路径".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ExistenceWithPath,
                locator_hint: "report.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("/tmp/case_only/Report.MD")
        );
    }

    #[test]
    fn direct_scalar_does_not_passthrough_multiline_list_dir_listing() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            "README.txt\nnotes.md\n",
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, None),
            None
        );
    }

    #[test]
    fn direct_scalar_counts_multiline_list_dir_when_route_requests_count() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "list_dir", "a\nb\nc\n"));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "数一下 scripts 目录直接有多少个子项".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ScalarCount,
                locator_hint: "scripts".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("3")
        );
    }

    #[test]
    fn direct_scalar_uses_inventory_dir_count_for_scalar_count() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"scripts","resolved_path":"/tmp/scripts","names_only":true,"names":["a","b","c"],"counts":{"total":3}}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "数一下 scripts 目录直接子项有多少个，只输出数字".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:current_workspace_scalar_count".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ScalarCount,
                locator_hint: "scripts".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("3")
        );
    }

    #[test]
    fn direct_count_uses_inventory_dir_total_for_non_scalar_shape() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"inventory_dir","path":"document","resolved_path":"/tmp/document","names_only":true,"names":["a","b","c","d"],"counts":{"total":4,"files":4,"dirs":0},"recursive":false}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "再数一下 document 目录直接有多少个子项".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "scalar count with free-form response shape".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ScalarCount,
                locator_hint: "document".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("4")
        );
    }

    #[test]
    fn direct_scalar_path_lists_inventory_dir_candidates_without_choosing_first() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/stem_multi","resolved_path":"/tmp/stem_multi","names_only":true,"names":["abcd.cpp","abcd.txt"],"counts":{"total":2}}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "find matching paths".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "structured scalar path request".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
                locator_hint: "/tmp/stem_multi".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };

        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("/tmp/stem_multi/abcd.cpp\n/tmp/stem_multi/abcd.txt")
        );
    }

    #[test]
    fn direct_scalar_uses_inventory_dir_hidden_count_for_hidden_entries_contract() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":".","resolved_path":"/tmp/workspace","include_hidden":true,"names_only":true,"names":[".git",".env","README.md"],"counts":{"total":3,"hidden":2}}"#,
        ));
        let route_result = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "数一下当前目录里以点开头的隐藏文件有几个，只输出数字".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:hidden_entries_check".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::HiddenEntriesCheck,
                locator_hint: ".".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("2")
        );
    }

    #[test]
    fn direct_answer_formats_package_manager_detect_summary() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "package_manager",
            "package_manager=brew",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "看看当前机器识别到的包管理器，再一句话说最可能日常会用哪个"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:package_manager_detect_summary".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("当前识别到的包管理器是 brew。")
        );
    }

    #[test]
    fn direct_scalar_extracts_package_manager_detect_value() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "package_manager",
            "package_manager=brew",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "只输出当前机器识别到的包管理器名称".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:package_manager_detect_scalar".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("brew")
        );
    }

    #[test]
    fn sqlite_database_kind_judgment_is_not_hard_classified_by_observed_output() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "db_basic",
            r#"{"columns":["name"],"rows":[]}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent:
                "看看 data/db-basic-contract.sqlite 里有哪些表，再一句话说这更像业务库还是测试库"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "normalizer:planner_execute_chat_wrapped".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
                locator_hint: "data/db-basic-contract.sqlite".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_lists_sqlite_table_names_without_llm_when_names_only_is_requested() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "db_basic",
            r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"users"}]}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent:
                "看一下 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 里有哪些表，只输出表名"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "normalizer:planner_execute_chat_wrapped".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::SqliteTableNamesOnly,
                locator_hint:
                    "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("orders\nusers")
        );
    }

    #[test]
    fn direct_scalar_lists_sqlite_table_names_when_names_only_contract_is_scalar() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "db_basic",
            r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"users"}]}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent:
                "看一下 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 里有哪些表，只输出表名"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "normalizer:act".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::SqliteTableNamesOnly,
                locator_hint:
                    "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("orders\nusers")
        );
    }

    #[test]
    fn structured_observed_body_preserves_db_table_inventory_instead_of_first_scalar_only() {
        let body = r#"{"columns":["name"],"rows":[{"name":"users"},{"name":"orders"},{"name":"service_logs"}]}"#;
        assert_eq!(
            structured_observed_body("db_basic", body).as_deref(),
            Some("db_tables=users, orders, service_logs")
        );
    }

    #[test]
    fn structured_observed_body_includes_path_batch_metadata_for_synthesis() {
        let body = r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","modified_ts":1777345844,"path":"Cargo.lock","resolved_path":"/tmp/repo/Cargo.lock","size_bytes":121657},"path":"/tmp/repo/Cargo.lock"},{"exists":true,"fact":{"kind":"file","modified_ts":1777357772,"path":"Cargo.toml","resolved_path":"/tmp/repo/Cargo.toml","size_bytes":2606},"path":"/tmp/repo/Cargo.toml"}],"include_missing":true}"#;
        assert_eq!(
            structured_observed_body("system_basic", body).as_deref(),
            Some(
                "path_batch_facts\npath_fact name=Cargo.lock path=/tmp/repo/Cargo.lock exists=true kind=file size_bytes=121657 modified_ts=1777345844\npath_fact name=Cargo.toml path=/tmp/repo/Cargo.toml exists=true kind=file size_bytes=2606 modified_ts=1777357772"
            )
        );
    }

    #[test]
    fn structured_observed_body_includes_inventory_dir_entry_metadata_for_synthesis() {
        let body = r#"{"action":"inventory_dir","counts":{"dirs":0,"files":2,"hidden":0,"total":2},"entries":[{"hidden":false,"kind":"file","modified_ts":1777513843,"name":"intent_normalizer.schema.json","path":"prompts/schemas/intent_normalizer.schema.json","size_bytes":9402},{"hidden":false,"kind":"file","modified_ts":1777526917,"name":"plan_result.schema.json","path":"prompts/schemas/plan_result.schema.json","size_bytes":4187}],"names":["intent_normalizer.schema.json","plan_result.schema.json"],"path":"prompts/schemas","resolved_path":"/tmp/repo/prompts/schemas","sort_by":"size_desc"}"#;
        assert_eq!(
            structured_observed_body("system_basic", body).as_deref(),
            Some(
                "inventory_dir path=/tmp/repo/prompts/schemas sort_by=size_desc total=2 files=2 dirs=0 hidden=0\nentry name=intent_normalizer.schema.json kind=file size_bytes=9402 modified_ts=1777513843\nentry name=plan_result.schema.json kind=file size_bytes=4187 modified_ts=1777526917"
            )
        );
    }

    #[test]
    fn structured_observed_body_compacts_large_inventory_dir_by_kind() {
        let entries = (0..9)
            .map(|idx| {
                serde_json::json!({
                    "hidden": false,
                    "kind": "dir",
                    "modified_ts": 1777513843,
                    "name": format!("dir_{idx}"),
                    "path": format!("dir_{idx}"),
                    "size_bytes": 0
                })
            })
            .chain((0..9).map(|idx| {
                serde_json::json!({
                    "hidden": false,
                    "kind": "file",
                    "modified_ts": 1777513843,
                    "name": format!("file_{idx}.md"),
                    "path": format!("file_{idx}.md"),
                    "size_bytes": 42
                })
            }))
            .collect::<Vec<_>>();
        let body = serde_json::json!({
            "action": "inventory_dir",
            "counts": {"dirs": 9, "files": 9, "hidden": 0, "total": 18},
            "entries": entries,
            "path": ".",
            "resolved_path": "/tmp/repo",
            "sort_by": "name"
        })
        .to_string();

        let observed = structured_observed_body("system_basic", &body).expect("observed body");
        assert!(observed.contains("dir_names=dir_0,dir_1,dir_2"));
        assert!(observed.contains("file_names=file_0.md,file_1.md,file_2.md"));
        assert!(!observed.contains("modified_ts=1777513843"));
        assert!(!observed.contains("size_bytes=42"));
    }

    #[test]
    fn structured_observed_body_includes_count_inventory_breakdown_for_synthesis() {
        let body = r#"{"action":"count_inventory","counts":{"dirs":26,"files":40,"hidden":0,"total":66},"kind_filter":"any","path":".","resolved_path":"/tmp/repo"}"#;
        assert_eq!(
            structured_observed_body("system_basic", body).as_deref(),
            Some(
                "action=count_inventory\npath=.\nresolved_path=/tmp/repo\nkind_filter=any\ncount_files=40\ncount_dirs=26\ncount_total=66\ncount_hidden=0"
            )
        );
    }

    #[test]
    fn sqlite_table_listing_summary_defers_to_synthesis() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "db_basic",
            r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"users"}]}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "列一下 data/app.sqlite 里有哪些表".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "normalizer:planner_execute_chat_wrapped".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::SqliteTableListing,
                locator_hint: "data/app.sqlite".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .is_none()
        );
    }

    #[test]
    fn direct_scalar_defers_route_locator_hint_quantity_comparison_to_synthesis() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "list_dir", "a\nb\n"));
        loop_state
            .executed_step_results
            .push(ok_step("step_2", "list_dir", "a\nb\nc\n"));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "上一个和上上个哪个更多，只回答目录名".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "'上一个'=assistant[-1](document,2), '上上个'=assistant[-2](scripts,3); scripts 更多"
                .to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::QuantityComparison,
                locator_hint: "scripts".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_scalar_defers_compare_paths_result_to_synthesis() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"compare_paths","left":{"path":"Cargo.toml","resolved_path":"/tmp/Cargo.toml","kind":"file","size_bytes":123},"right":{"path":"Cargo.lock","resolved_path":"/tmp/Cargo.lock","kind":"file","size_bytes":456},"comparison":{"same_kind":true,"same_name":false,"same_size":false,"size_delta_bytes":-333,"left_newer":null,"same_content":false}}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "比较 Cargo.toml 和 Cargo.lock 哪个更大，顺手用一句通俗话解释原因"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:compare_targets".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::QuantityComparison,
                locator_hint: "Cargo.lock|Cargo.toml".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
        assert!(
            has_observed_answer_candidates(&loop_state),
            "compare_paths should remain available as observed facts for synthesis"
        );
    }

    #[test]
    fn quantity_comparison_does_not_force_direct_scalar_observed_answer() {
        let route = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "比较 Cargo.toml 和 Cargo.lock 哪个更大".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:compare_targets".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::QuantityComparison,
                locator_hint: "Cargo.lock|Cargo.toml".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        assert!(!super::route_prefers_direct_observed_answer_for_scalar(
            &route
        ));
    }

    #[test]
    fn direct_answer_defers_git_status_dirty_worktree_to_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "git_basic",
            "exit=0\n## main...origin/main\n M Cargo.toml\n?? new_file.txt\n",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "检查当前仓库是否存在未提交的改动，用一句话返回结果".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_defers_git_log_release_note_to_synthesis() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "read_file",
            "RustClaw is a local Rust agent runtime centered on clawd.",
        ));
        loop_state.executed_step_results.push(ok_step(
            "step_2",
            "system_basic",
            r#"{"action":"extract_field","field_path":"workspace.package.version","value_text":"0.1.7"}"#,
        ));
        loop_state.executed_step_results.push(ok_step(
            "step_3",
            "git_basic",
            "exit=0\n09342a6a fix: expose nl execution and locator flows\n336e8d92 docs: update planner-first architecture diagrams\n",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "Write a short release note for RustClaw.".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
                locator_hint: "RustClaw".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };

        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_scalar_extracts_git_commit_subject_from_oneline_log() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "git_basic",
            "exit=0\n09342a6a fix: expose nl execution and locator flows\n",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "return the latest git commit subject only".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::GitCommitSubject,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };

        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)),
            Some("fix: expose nl execution and locator flows".to_string())
        );
    }

    #[test]
    fn direct_answer_defers_git_status_clean_when_exit_only_to_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "git_basic", "exit=0\n"));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "看看这个仓库现在有没有未提交改动，用一句话告诉我".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_defers_git_status_dirty_without_branch_header_to_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "git_basic",
            " M Cargo.toml\n?? new_file.txt\n",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "看看这个仓库现在有没有未提交改动，用一句话告诉我".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_preserves_run_cmd_directory_entry_names() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd_observed_output_test_{}_run_cmd_names",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "act_plan.log\nclawd.log\nfeishud.log\n",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "列出 logs 目录下前 5 个文件名".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "logs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("act_plan.log\nclawd.log\nfeishud.log")
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn direct_answer_preserves_run_cmd_semantic_directory_path_list() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            ".\n./scripts\n./scripts/nl_tests\n./crates/skills/browser_web/node_modules/playwright-core/bin\n",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent:
                "查找当前工作目录中哪些文件夹存放了 .sh 脚本文件，列出这些文件夹的名称".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::DirectoryNames,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some("/home/guagua/rustclaw".to_string()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(
                ".\n./scripts\n./scripts/nl_tests\n./crates/skills/browser_web/node_modules/playwright-core/bin"
            )
        );
    }

    #[test]
    fn direct_answer_preserves_run_cmd_directory_entry_names_without_request_text_limit() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd_observed_output_test_{}_run_cmd_limit",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "run_cmd", "a\nb\nc\nd\n"));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "列出 logs 目录下前 2 个文件名".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "logs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("a\nb\nc\nd")
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn direct_answer_formats_run_cmd_exists_probe_with_resolved_path() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd_observed_output_test_{}_run_cmd_exists",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("rustclaw.service");
        std::fs::write(&file_path, "unit").expect("write fixture file");
        let resolved = file_path
            .canonicalize()
            .unwrap_or_else(|_| file_path.clone())
            .to_string_lossy()
            .to_string();
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "run_cmd", "EXISTS\n"));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "rustclaw.service".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(resolved.clone()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(format!("有，路径：{resolved}").as_str())
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn direct_answer_formats_run_cmd_not_found_probe_as_no() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "run_cmd", "NOT_FOUND\n"));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "rustclaw.service".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("没有")
        );
    }

    #[test]
    fn direct_answer_defers_health_check_json_for_act_free_shape() {
        let mut loop_state = LoopState::new(2);
        let body = r#"{"clawd_health_port_open":true,"telegramd_process_count":0}"#;
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "health_check", body));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "做一次 health check".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_defers_health_check_summary_for_act_free_shape() {
        let mut loop_state = LoopState::new(2);
        let body = r#"{"clawd_process_count":7,"telegramd_process_count":0,"clawd_health_port_open":false,"clawd_log":{"exists":false},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#;
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "health_check", body));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent:
                "对系统做一次基础健康检查，只总结操作系统信息，RustClaw 自身不展开总结，仅返回其关键字段"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_passes_health_check_json_only_for_raw_output_contract() {
        let mut loop_state = LoopState::new(2);
        let body = r#"{"clawd_health_port_open":true,"telegramd_process_count":0}"#;
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "health_check", body));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "run health_check and return the raw output".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::RawCommandOutput,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(body)
        );
    }

    #[test]
    fn direct_answer_defers_health_check_summary_over_later_steps_to_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":12,"telegramd_process_count":0,"clawd_health_port_open":false,"clawd_log":{"exists":false},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#,
        ));
        loop_state.executed_step_results.push(ok_step(
            "step_2",
            "system_basic",
            r#"{"action":"info","os":"macos","hostname":"example"}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "Run a basic health check. Summarize only the host operating system, and for RustClaw itself just list the key fields.".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_defers_health_check_one_sentence_summary_to_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false}}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "帮我做一次基础健康检查，只列最重要的结论".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_defers_health_check_unhealthy_summary_to_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":0,"telegramd_process_count":1,"clawd_health_port_open":false,"clawd_log":{"exists":true,"keyword_error_count":3},"telegramd_log":{"exists":true,"keyword_error_count":0}}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent:
                "run a basic health check here and summarize only the most important findings"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_defers_health_check_telegramd_stopped_summary_to_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false}}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "帮我做一次基础健康检查，只列最重要的结论".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_defers_health_check_language_sensitive_summary_to_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false}}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "帮我做一次基础健康检查，只列最重要的结论".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            user_request: Some(
                "run a basic health check here and summarize only the most important findings"
                    .to_string(),
            ),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_defers_health_check_os_summary_to_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":12,"telegramd_process_count":0,"clawd_health_port_open":false,"clawd_log":{"exists":false},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent:
                "做一次基础健康检查，只返回操作系统层面的关键字段，不要包含 RustClaw 自身的状态摘要"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_failed_safe_clarify".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            user_request: Some(
                "做一次基础健康检查，只总结操作系统；RustClaw 自身不要总结，直接给我关键字段。"
                    .to_string(),
            ),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_defers_health_check_os_warning_summary_to_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":1,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":true,"keyword_error_count":0},"system_health":{"os_family":"linux","warnings":["disk_root_low"]}}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent:
                "run a basic health check here and summarize only the most important findings"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            user_request: Some(
                "run a basic health check here and summarize only the most important findings"
                    .to_string(),
            ),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_defers_process_basic_port_summary_to_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "process_basic",
            "exit=0\nCOMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME\nclawd 4498 testuser 12u IPv4 0x0 0t0 TCP *:8787 (LISTEN)\nnginx 51129 testuser 6u IPv4 0x0 0t0 TCP *:80 (LISTEN)\nss-local 424 testuser 6u IPv4 0x0 0t0 TCP 127.0.0.1:1086 (LISTEN)\n",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "看看这台机器现在有哪些端口在监听，然后挑最值得注意的几个简单说一下"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_defers_http_basic_one_sentence_summary_to_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "http_basic",
            "status=200\n{\"ok\":true}\n",
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "请求一下 http://127.0.0.1:8787/v1/health ，如果能通就简短总结结果"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Url,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "http://127.0.0.1:8787/v1/health".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_preserves_http_basic_raw_scalar_for_free_shape() {
        let mut loop_state = LoopState::new(2);
        let body = "status=200\n{\"ok\":true}\n";
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "http_basic", body));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "请求接口并返回原始结果".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Url,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "http://127.0.0.1:8787/v1/health".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("status=200")
        );
    }

    #[test]
    fn direct_answer_formats_service_control_status_summary_for_chinese_request() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "service_control",
            r#"{"status":"ok","service_name":"telegramd","manager_type":"rustclaw","requested_action":"status","executed_actions":["status"],"pre_state":"telegramd=stopped","post_state":"telegramd=stopped","verified":true,"key_evidence":["telegramd process_count=0 memory_rss_bytes=Some(0)"],"failure_reason":"","next_step":"","summary":"Status: telegramd=stopped"}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "帮我检查 telegramd 现在是不是在运行，顺手简短解释状态".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ServiceStatus,
                locator_hint: "telegramd".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("telegramd 当前状态是 `telegramd=stopped`：rustclaw 已完成检查，未显示为运行中。")
        );
    }

    #[test]
    fn direct_answer_formats_service_control_status_summary_for_english_request() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "service_control",
            r#"{"status":"ok","service_name":"telegramd","manager_type":"rustclaw","requested_action":"status","executed_actions":["status"],"pre_state":"telegramd=running","post_state":"telegramd=running","verified":true,"key_evidence":["telegramd process_count=1 memory_rss_bytes=Some(1024)"],"failure_reason":"","next_step":"","summary":"Status: telegramd=running"}"#,
        ));
        let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent:
                "check whether telegramd is running right now and briefly explain the status"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ServiceStatus,
                locator_hint: "telegramd".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(
                "telegramd is running: rustclaw reports `telegramd=running` and verification passed."
            )
        );
    }

    #[test]
    fn observed_entries_compact_log_analyze_json_into_summary() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "log_analyze",
            r#"{"path":"/tmp/test.log","total_lines":120,"keyword_counts":{"error":9,"panic":1},"recent_matches":["10: error one","20: panic two"]}"#,
        ));
        let entries = observed_output_entries(&loop_state);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].contains("log_analyze path=/tmp/test.log total_lines=120"));
        assert!(entries[0].contains("keyword_counts: error=9, panic=1"));
        assert!(entries[0].contains("recent_matches:\n- 10: error one\n- 20: panic two"));
        assert!(!entries[0].contains(r#""keyword_counts""#));
    }

    #[test]
    fn observed_answer_parser_strips_bare_json_language_prefix() {
        let raw = "json\n{\"answer\":\"ok\",\"qualified\":true}";
        assert_eq!(
            super::strip_bare_json_language_prefix(raw),
            "{\"answer\":\"ok\",\"qualified\":true}"
        );
        assert_eq!(
            super::strip_bare_json_language_prefix("json response follows"),
            "json response follows"
        );
    }

    #[test]
    fn observed_answer_parser_unwraps_nested_finalizer_envelope() {
        let raw = "json\n{\"answer\":\"# RustClaw\\n正文\",\"qualified\":true,\"needs_clarify\":false,\"is_meta_instruction\":false,\"publishable\":true,\"confidence\":0.85,\"reason\":\"grounded\"}";
        assert_eq!(
            super::extract_answer_from_finalizer_envelope_text(raw).as_deref(),
            Some("# RustClaw\n正文")
        );
    }

    /// §D2.b：finalizer_out schema 与 `ObservedAnswerFallbackOut` 漂移检查。
    ///
    /// 校验内容：
    /// 1. `prompts/schemas/finalizer_out.schema.json` 是合法 JSON 且为 object schema；
    /// 2. `properties` ⊇ `ObservedAnswerFallbackOut` 全部字段（含 serde rename 后的 `reason`）；
    /// 3. `required` 列表精确包含 5 个核心硬要求字段（answer + 4 个布尔 + confidence）；
    /// 4. 完整性闭环：把一份 schema-conformant 的最小负载 round-trip
    ///    `serde_json::from_str::<ObservedAnswerFallbackOut>` 必须成功，且 confidence 0/1
    ///    边界都被接受。
    ///
    /// 任意不满足说明 prompt / schema / parser 三者已漂移，build 红灯。
    #[test]
    fn finalizer_out_schema_drift() {
        const SCHEMA_RAW: &str =
            include_str!("../../../../prompts/schemas/finalizer_out.schema.json");
        let schema: serde_json::Value =
            serde_json::from_str(SCHEMA_RAW).expect("finalizer_out.schema.json must be valid JSON");
        assert_eq!(
            schema.get("type").and_then(|v| v.as_str()),
            Some("object"),
            "schema root must be object"
        );

        const STRUCT_FIELDS: &[&str] = &[
            "answer",
            "qualified",
            "needs_clarify",
            "is_meta_instruction",
            "publishable",
            "confidence",
            "reason",
        ];
        let properties = schema
            .get("properties")
            .and_then(|v| v.as_object())
            .expect("schema must have `properties` object");
        for field in STRUCT_FIELDS {
            assert!(
                properties.contains_key(*field),
                "schema missing parser field `{}` under properties — sync prompts/schemas/finalizer_out.schema.json with ObservedAnswerFallbackOut",
                field
            );
        }

        let required: std::collections::HashSet<&str> = schema
            .get("required")
            .and_then(|v| v.as_array())
            .expect("schema must have `required`")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        let expected_required: std::collections::HashSet<&str> = [
            "answer",
            "qualified",
            "needs_clarify",
            "is_meta_instruction",
            "publishable",
            "confidence",
        ]
        .into_iter()
        .collect();
        assert_eq!(
            required, expected_required,
            "finalizer_out required set drifted from canonical 5+1"
        );

        // 步骤 4：最小 schema-conformant 负载必须能解码到 parser struct。
        let probes: &[(&str, &str)] = &[
            (
                "minimum",
                r#"{"answer":"ok","qualified":true,"needs_clarify":false,"is_meta_instruction":false,"publishable":true,"confidence":0.0}"#,
            ),
            (
                "boundary_high",
                r#"{"answer":"ok","qualified":true,"needs_clarify":false,"is_meta_instruction":false,"publishable":true,"confidence":1.0,"reason":"r"}"#,
            ),
            (
                "needs_clarify_with_empty_answer",
                r#"{"answer":"","qualified":false,"needs_clarify":true,"is_meta_instruction":false,"publishable":false,"confidence":0.5}"#,
            ),
        ];
        for (label, raw) in probes {
            serde_json::from_str::<super::ObservedAnswerFallbackOut>(raw).unwrap_or_else(|err| {
                panic!(
                    "ObservedAnswerFallbackOut probe `{}` failed: {} (raw: {})",
                    label, err, raw
                )
            });
        }
    }
}
