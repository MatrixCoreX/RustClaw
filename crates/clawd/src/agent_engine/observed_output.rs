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
        if matches!(step.skill.as_str(), "read_file" | "list_dir") {
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
    max_entries: Option<usize>,
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
    let listing =
        normalized_observed_listing(step.output.as_deref().unwrap_or_default(), max_entries)?;
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

fn trim_listing_to_max_entries(listing: &str, max_entries: Option<usize>) -> Option<String> {
    let mut lines = listing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if let Some(limit) = max_entries.filter(|limit| *limit > 0) {
        lines.truncate(limit);
    }
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
        .and_then(|ctx| ctx.user_request.as_deref())
        .filter(|text| !text.trim().is_empty())
        .or_else(|| {
            route
                .map(|route| route.resolved_intent.as_str())
                .filter(|text| !text.trim().is_empty())
        })
}

fn route_requests_scalar_count(route: &crate::RouteResult) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::ScalarCount
}

fn route_requests_hidden_entries_check(route: &crate::RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::HiddenEntriesCheck
}

pub(crate) fn route_prefers_direct_observed_answer_for_scalar(route: &crate::RouteResult) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::ExistenceWithPath
                | crate::OutputSemanticKind::HiddenEntriesCheck
        )
}

pub(crate) fn scalar_route_prefers_structured_observed_answer(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && (route_prefers_direct_observed_answer_for_scalar(route)
            || extract_latest_generic_successful_output(loop_state)
                .is_some_and(|observed| observed.skill == "health_check"))
}

fn route_requests_scalar_path_only(route: &crate::RouteResult) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::ScalarPathOnly
}

fn route_allows_raw_listing_direct_answer(route: Option<&crate::RouteResult>) -> bool {
    route.is_none_or(|route| {
        if !route.output_contract.requires_content_evidence {
            return true;
        }
        route.output_contract.semantic_kind == crate::OutputSemanticKind::FileNames
    })
}

fn latest_list_dir_listing(loop_state: &LoopState) -> Option<String> {
    let idx = latest_successful_step_index(loop_state, |step| step.skill == "list_dir")?;
    let step = &loop_state.executed_step_results[idx];
    if !step.is_ok() || step.skill != "list_dir" {
        return None;
    }
    normalized_observed_listing(step.output.as_deref().unwrap_or_default(), None)
}

fn hidden_entries_from_listing(listing: &str) -> Vec<String> {
    listing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| line.starts_with('.'))
        .map(ToString::to_string)
        .collect()
}

fn hidden_entries_from_entries(entries: &[String]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| entry.trim())
        .filter(|entry| !entry.is_empty())
        .filter(|entry| entry.starts_with('.'))
        .map(ToString::to_string)
        .collect()
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
    let hidden_entries = latest_directory_listing_entries(loop_state, None, None)
        .map(|entries| hidden_entries_from_entries(&entries))
        .or_else(|| {
            latest_list_dir_listing(loop_state).map(|listing| hidden_entries_from_listing(&listing))
        })?;
    let examples = hidden_entries
        .iter()
        .take(3)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if route.ask_mode.is_plain_act()
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
        )
    {
        if hidden_entries.is_empty() {
            return Some(observed_t(
                state,
                "clawd.msg.hidden_entries_none_scalar",
                "没有",
                "No",
                prefer_english,
            ));
        }
        return Some(observed_t_with_vars(
            state,
            "clawd.msg.hidden_entries_found_scalar_examples",
            "有。示例：{examples}",
            "Yes: {examples}",
            prefer_english,
            &[("examples", examples.as_str())],
        ));
    }
    None
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
    max_entries: Option<usize>,
) -> Option<Vec<String>> {
    let idx = latest_successful_step_index(loop_state, |_| true)?;
    let step = &loop_state.executed_step_results[idx];
    directory_listing_entries_from_step(step, auto_locator_path, max_entries)
}

fn directory_listing_entries_from_step(
    step: &crate::executor::StepExecutionResult,
    auto_locator_path: Option<&str>,
    max_entries: Option<usize>,
) -> Option<Vec<String>> {
    if !step.is_ok() {
        return None;
    }
    let body = step.output.as_deref().unwrap_or_default();
    match step.skill.as_str() {
        "list_dir" => {
            normalized_observed_listing(body, max_entries).map(|listing| listing_entries(&listing))
        }
        "run_cmd" => run_cmd_listing_text_candidate(body, auto_locator_path, max_entries)
            .map(|listing| listing_entries(&listing)),
        "system_basic" => {
            let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
            let mut entries = inventory_dir_names(&value)?;
            if let Some(limit) = max_entries.filter(|limit| *limit > 0) {
                entries.truncate(limit);
            }
            Some(entries)
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
    if !step.is_ok() || step.skill != "system_basic" {
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
    let response_shape = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.output_contract.response_shape);
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
            "Return a short direct answer, usually one short paragraph or compact listing plus one concise conclusion."
        }
        None => "Return the shortest grounded answer that directly satisfies the user request.",
    }
    .to_string()
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
    route: &crate::RouteResult,
    body: &str,
    prefer_english: bool,
) -> Option<String> {
    let observed_kind = sqlite_table_observed_output_kind(route)?;
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let table_names = db_basic_table_names(&value)?;
    if table_names.is_empty() {
        return Some(
            if observed_kind == SqliteTableObservedOutputKind::NamesOnly {
                if prefer_english {
                    "This SQLite file currently has no tables.".to_string()
                } else {
                    "这个 sqlite 文件里目前没有任何表。".to_string()
                }
            } else if prefer_english {
                "This SQLite file currently has no tables.".to_string()
            } else {
                "这个 sqlite 文件里目前没有任何表。".to_string()
            },
        );
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
    if skill != "system_basic" {
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
                .get("field_path")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .unwrap_or("requested field");
            Some(structured_field_display_line(
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

fn inventory_dir_direct_answer_candidate(
    value: &serde_json::Value,
    max_entries: Option<usize>,
) -> Option<String> {
    let names = inventory_dir_names(value)?;
    trim_listing_to_max_entries(&names.join("\n"), max_entries)
}

fn inventory_dir_observed_candidate(value: &serde_json::Value) -> Option<String> {
    let names = inventory_dir_names(value)?;
    let path = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    Some(format!(
        "inventory_dir path={path}\n- {}",
        names.join("\n- ")
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
    max_entries: Option<usize>,
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
        .then(|| trim_listing_to_max_entries(&lines.join("\n"), max_entries))
        .flatten()
}

fn run_cmd_listing_text_candidate(
    body: &str,
    auto_locator_path: Option<&str>,
    max_entries: Option<usize>,
) -> Option<String> {
    run_cmd_shell_listing_entry_names(body, max_entries)
        .map(|names| names.join("\n"))
        .or_else(|| run_cmd_directory_entry_list_candidate(body, auto_locator_path, max_entries))
}

fn run_cmd_shell_listing_entry_names(
    body: &str,
    max_entries: Option<usize>,
) -> Option<Vec<String>> {
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
    if let Some(limit) = max_entries.filter(|limit| *limit > 0) {
        names.truncate(limit);
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

fn candidate_not_found_text(state: Option<&AppState>, prefer_english: bool) -> String {
    observed_t(state, "clawd.msg.exists_no", "没有", "no", prefer_english)
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
    let root = resolved_root
        .map(str::trim)
        .filter(|root| !root.is_empty())
        .map(Path::new)?;
    Some(root.join(candidate).to_string_lossy().to_string())
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
                return Some(candidate_not_found_text(state, prefer_english));
            }
            let path = entry
                .get("fact")
                .and_then(|v| v.as_object())
                .and_then(|fact| {
                    fact.get("resolved_path")
                        .and_then(|v| v.as_str())
                        .or_else(|| fact.get("path").and_then(|v| v.as_str()))
                })
                .or_else(|| entry.get("path").and_then(|v| v.as_str()));
            Some(candidate_exists_with_path_text(state, path, prefer_english))
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

fn fs_search_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    locator_hint: Option<&str>,
    prefer_english: bool,
    allow_multi_result_list: bool,
) -> Option<String> {
    if let Some((results, count, ext)) = fs_search_find_ext_results(value) {
        if count == 0 || results.is_empty() {
            return Some(if prefer_english {
                format!("No .{ext} files found.")
            } else {
                format!("没有找到 .{ext} 文件")
            });
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
        return package_manager_summary_candidate(body, response_shape);
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
        return route
            .is_some_and(|route| {
                route.output_contract.response_shape == crate::OutputResponseShape::Scalar
            })
            .then(|| service_control_summary_candidate(&value))
            .flatten();
    }
    if skill == "fs_search" {
        return fs_search_scalar_candidate(
            state,
            &value,
            locator_hint,
            auto_locator_path,
            prefer_full_path,
            prefer_english,
        );
    }
    if skill != "system_basic" {
        return None;
    }
    let action = value.get("action").and_then(|v| v.as_str())?;
    match action {
        "inventory_dir" => {
            let hidden_count_route = route.is_some_and(|route| {
                route.output_contract.response_shape == crate::OutputResponseShape::Scalar
                    && route.output_contract.semantic_kind
                        == crate::OutputSemanticKind::HiddenEntriesCheck
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
                                .filter(|name| name.trim_start().starts_with('.'))
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
            } else {
                None
            }
        }
        "extract_field" => {
            let field_path = value
                .get("field_path")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .unwrap_or("requested field");
            if value
                .get("exists")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
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
            Some(observed_t_with_vars(
                state,
                "clawd.msg.field_not_found",
                "{field_path} 字段不存在",
                "Field `{field_path}` does not exist.",
                prefer_english,
                &[("field_path", field_path)],
            ))
        }
        "count_inventory" => value
            .get("counts")
            .and_then(|v| v.get("total"))
            .and_then(value_scalar_text),
        _ => None,
    }
}

fn package_manager_summary_candidate(
    body: &str,
    response_shape: Option<crate::OutputResponseShape>,
) -> Option<String> {
    let manager = body
        .lines()
        .find_map(|line| line.trim().strip_prefix("package_manager="))
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    match response_shape {
        Some(crate::OutputResponseShape::Scalar) => Some(manager.to_string()),
        _ => None,
    }
}

fn structured_field_display_line(
    field_path: &str,
    value: &serde_json::Value,
    value_text: Option<&str>,
    exists: bool,
    prefer_english: bool,
) -> String {
    if !exists {
        return if prefer_english {
            format!("{field_path}: <missing>")
        } else {
            format!("{field_path}: 不存在")
        };
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

fn extract_fields_direct_answer_candidate(
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
            let field_path = item.get("field_path")?.as_str()?.trim();
            if field_path.is_empty() {
                return None;
            }
            Some(structured_field_display_line(
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

fn normalize_read_range_excerpt(excerpt: &str) -> Option<String> {
    let lines = excerpt
        .lines()
        .map(str::trim_end)
        .map(|line| {
            line.split_once('|')
                .filter(|(prefix, _)| {
                    !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit())
                })
                .map(|(_, rest)| rest.trim_start().to_string())
                .unwrap_or_else(|| line.trim().to_string())
        })
        .collect::<Vec<_>>();
    if lines.is_empty() || lines.iter().all(|line| line.is_empty()) {
        None
    } else {
        Some(lines.join("\n"))
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

fn structured_observed_body(skill: &str, body: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    match skill {
        "system_basic" => {
            let action = value.get("action").and_then(|v| v.as_str())?;
            match action {
                "read_range" => read_range_observed_candidate(&value),
                "inventory_dir" => inventory_dir_observed_candidate(&value),
                "compare_paths" => compare_paths_observed_candidate(body),
                _ => None,
            }
        }
        "db_basic" => db_basic_observed_candidate(&value),
        "service_control" => service_control_summary_candidate(&value),
        "fs_search" => fs_search_direct_answer_candidate(None, &value, None, false, true),
        "archive_basic" => archive_basic_observed_candidate(&value),
        "log_analyze" => compact_log_analyze_excerpt(&value),
        "package_manager" => {
            package_manager_summary_candidate(body, Some(crate::OutputResponseShape::OneSentence))
        }
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
    if let Some(answer) = latest_successful_list_dir_answer_candidate(
        loop_state,
        Some(crate::OutputResponseShape::Scalar),
        None,
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
        if let Some(answer) = count_answer_from_latest_listing(route, loop_state) {
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
        if let Some(answer) = count_answer_from_latest_listing(route, loop_state) {
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
    let allow_raw_listing_direct_answer = route_allows_raw_listing_direct_answer(route);
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
    let health_check_prefers_raw_payload = is_plain_act
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
    let prefer_full_path = route.is_some_and(route_requests_scalar_path_only);

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
                None,
                auto_locator_path,
                prefer_full_path,
            )
        })
        .flatten()
        .or_else(|| {
            let observed_output = extract_latest_generic_successful_output(loop_state)?;
            if observed_output.skill == "run_cmd" {
                run_cmd_presence_with_path_candidate(
                    state,
                    &observed_output.body,
                    locator_hint,
                    auto_locator_path,
                    prefers_english_presence_answer,
                )
                .or_else(|| {
                    allow_raw_listing_direct_answer
                        .then(|| {
                            run_cmd_listing_text_candidate(
                                &observed_output.body,
                                auto_locator_path,
                                None,
                            )
                        })
                        .flatten()
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
                "service_control" => None,
                "fs_search" => serde_json::from_str::<serde_json::Value>(&observed_output.body)
                    .ok()
                    .and_then(|value| {
                        fs_search_direct_answer_candidate(
                            state,
                            &value,
                            locator_hint,
                            prefers_english_free_text,
                            allow_raw_listing_direct_answer,
                        )
                    }),
                "git_basic" => None,
                "db_basic" => route.and_then(|route| {
                    db_basic_tables_summary_candidate(
                        route,
                        &observed_output.body,
                        prefers_english_free_text,
                    )
                }),
                "transform" => transform_skill_formatted_output_candidate(&observed_output.body),
                "package_manager" => {
                    package_manager_summary_candidate(&observed_output.body, response_shape)
                }
                "archive_basic" => None,
                "log_analyze" => None,
                "system_basic" => {
                    let value = serde_json::from_str::<serde_json::Value>(&observed_output.body)
                        .ok()
                        .or_else(|| {
                            system_basic_info_value("system_basic", &observed_output.body)
                        })?;
                    let action = value.get("action").and_then(|v| v.as_str());
                    if action == Some("read_range")
                        && (is_plain_act
                            && !matches!(
                                response_shape,
                                Some(crate::OutputResponseShape::OneSentence)
                            ))
                    {
                        value
                            .get("excerpt")
                            .and_then(|v| v.as_str())
                            .and_then(normalize_read_range_excerpt)
                            .map(|text| text.trim_end().to_string())
                    } else if action == Some("inventory_dir")
                        && is_plain_act
                        && allow_raw_listing_direct_answer
                    {
                        inventory_dir_direct_answer_candidate(&value, None)
                    } else if action == Some("extract_fields") {
                        extract_fields_direct_answer_candidate(
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
                    } else if action == Some("info")
                        || (action.is_none() && system_basic_value_looks_like_info(&value))
                    {
                        if route.is_some_and(route_requests_scalar_path_only) {
                            system_basic_info_scalar_path_candidate(&value)
                        } else {
                            None
                        }
                    } else if route.is_some_and(|route| {
                        route.output_contract.semantic_kind
                            == crate::OutputSemanticKind::ExistenceWithPath
                    }) {
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
                allows_normalized_scalar_direct_fallback(&observed_output.skill, response_shape)
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

fn allows_normalized_scalar_direct_fallback(
    skill: &str,
    response_shape: Option<crate::OutputResponseShape>,
) -> bool {
    match skill {
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

fn observed_step_body(step: &crate::executor::StepExecutionResult) -> Option<String> {
    let body = step
        .output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())?;
    if let Some(normalized) = structured_observed_body(&step.skill, body) {
        return Some(normalized);
    }
    if let Some(normalized) = system_basic_structured_doc_observed_body(&step.skill, body) {
        return Some(normalized);
    }
    (crate::finalize::classify_observed_content_status(body)
        == crate::finalize::ObservedContentStatus::ContentAvailable)
        .then(|| body.to_string())
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
        .rfind(|(_, step)| {
            step.is_ok() && step.skill == "list_dir" && observed_step_entry(step).is_some()
        })
        .map(|(idx, _)| idx);
    let mut selected_indices = latest_listing_idx.into_iter().collect::<Vec<_>>();
    let mut recent_non_listing = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .filter(|(_, step)| {
            step.is_ok() && step.skill != "list_dir" && observed_step_entry(step).is_some()
        })
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

pub(crate) fn has_observed_answer_candidates(loop_state: &LoopState) -> bool {
    !observed_output_entries(loop_state).is_empty()
}

fn observed_contract_json(agent_run_context: Option<&AgentRunContext>) -> String {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return "{}".to_string();
    };
    serde_json::json!({
        "routed_mode": route.routed_mode.as_str(),
        "response_shape": route.output_contract.response_shape.as_str(),
        "requires_content_evidence": route.output_contract.requires_content_evidence,
        "delivery_required": route.output_contract.delivery_required,
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

pub(crate) async fn synthesize_answer_from_observed_output(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
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

    let observed_entries = observed_output_entries(loop_state);
    if observed_entries.is_empty() {
        return None;
    }
    let observed_block = observed_entries.join("\n\n");
    let resolved_intent = resolved_user_intent(agent_run_context, user_text);
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_text);
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
                return None;
            }
        };
    let response_style_hint = observed_response_style_hint(agent_run_context);
    let prompt = crate::render_prompt_template(
        &prompt_template,
        &[
            ("__USER_REQUEST__", user_text.trim()),
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
            .ok()?;
    let parsed = match crate::prompt_utils::validate_against_schema::<ObservedAnswerFallbackOut>(
        &llm_out,
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
            // F14: minimax 等 vendor 偶发不遵守 prompt 的 "Output JSON only" 契约，
            // 直接吐 markdown 文本（典型例：被多步 read 喂饱后给一段中文综述但没包成
            // JSON envelope）。原先 ObservedAnswerFallbackOut 解析失败 → 整个 fallback
            // 返回 None → finalize 落到 clarify_question_fallback，把已经合成好的真实
            // 答案丢掉，变成"假需要确认"。这里把 trim 后的整段文本视作 answer 兜底，
            // 同时 publishable=true、qualified=true、confidence=0.7（足以越过下游
            // OBSERVED_SELF_CLASSIFY_CONF_THRESHOLD=0.55，并保留下游 semantic_judge 的
            // meta-instruction 检查仍能拦截 "我会去检查/please confirm" 之类伪答案）。
            let trimmed = llm_out.trim().trim_matches('`').trim();
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
        })?;
    let answer = parsed.answer.trim().to_string();
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
        && (parsed.qualified || semantically_publishable);
    Some((
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
    ))
}

pub(crate) fn normalized_observed_listing(
    observed: &str,
    max_entries: Option<usize>,
) -> Option<String> {
    trim_listing_to_max_entries(observed, max_entries)
}

#[cfg(test)]
mod tests {
    use super::super::LoopState;
    use super::{
        extract_direct_answer_from_generic_output, extract_direct_scalar_from_generic_output,
        extract_direct_scalar_from_generic_output_i18n,
        extract_direct_scalar_from_generic_output_with_locator_hint,
        has_observed_answer_candidates, normalized_observed_listing, observed_contract_json,
        observed_output_entries, observed_request_language_hint, observed_response_style_hint,
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent:
                "UI/package.json 里的 name 和 crates/clawd/Cargo.toml 里的 package.name 一样吗？只回答一样或不一样"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "route_contract:compare_targets".to_string(),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "Are those two names the same? Answer same or different".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "route_contract:same_or_different".to_string(),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent:
                "读取 UI/package.json 里的 name 字段，再读取 crates/clawd/Cargo.toml 里的 package.name 字段，最后用一行输出：前者、后者、一样或不一样"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "route_contract:same_or_different".to_string(),
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
    fn direct_scalar_reports_missing_extract_field_as_field_absent() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":false,"field_path":"name","value_text":"","value":null,"value_type":"null"}"#,
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
            Some("name 字段不存在")
        );
    }

    #[test]
    fn direct_scalar_reads_count_inventory_total_from_structured_output() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"count_inventory","counts":{"total":12,"files":9,"dirs":3}}"#,
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
            Some("12")
        );
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
    fn fs_search_find_ext_direct_answer_returns_paths_list() {
        let value = serde_json::json!({
            "action": "find_ext",
            "ext": "toml",
            "count": 3,
            "results": ["Cargo.toml", "configs/config.toml", "configs/git_basic.toml"]
        });
        assert_eq!(
            super::fs_search_direct_answer_candidate(None, &value, None, false, true).as_deref(),
            Some("Cargo.toml\nconfigs/config.toml\nconfigs/git_basic.toml")
        );
    }

    #[test]
    fn fs_search_direct_answer_does_not_confirm_ambiguous_matches_when_direct_list_disallowed() {
        let value = serde_json::from_str::<serde_json::Value>(
            r#"{"action":"find_name","pattern":"abcd","count":4,"results":["abcd_report.md","my_abcd.txt","x_abcd_log.txt","zz_abcd_backup.log"],"root":""}"#,
        )
        .expect("json");
        assert_eq!(
            super::fs_search_direct_answer_candidate(None, &value, None, false, false).as_deref(),
            None
        );
        assert_eq!(
            super::fs_search_direct_answer_candidate(None, &value, None, false, true).as_deref(),
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
            super::fs_search_direct_answer_candidate(None, &value, None, false, false).as_deref(),
            Some("有，路径：README.md")
        );
    }

    #[test]
    fn fs_search_direct_answer_uses_locator_hint_for_ambiguous_list_when_allowed() {
        let value = serde_json::from_str::<serde_json::Value>(
            r#"{"action":"find_name","count":4,"results":["scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt"],"root":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3"}"#,
        )
        .expect("json");
        assert_eq!(
            super::fs_search_direct_answer_candidate(None, &value, Some("abcd"), false, false)
                .as_deref(),
            None
        );
        assert_eq!(
            super::fs_search_direct_answer_candidate(None, &value, Some("abcd"), false, true)
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
            normalized_observed_listing("\nfoo\n\nbar\n", None).as_deref(),
            Some("foo\nbar")
        );
    }

    #[test]
    fn normalized_listing_honors_requested_entry_limit() {
        assert_eq!(
            normalized_observed_listing("a\nb\nc\n", Some(2)).as_deref(),
            Some("a\nb")
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            "mixed"
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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

        route_result.output_contract.response_shape = OutputResponseShape::Scalar;
        agent_run_context.route_result = Some(route_result.clone());
        assert!(observed_response_style_hint(Some(&agent_run_context))
            .contains("only the final scalar value"));

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
    fn observed_fallback_prompt_renders_language_and_response_style_hints() {
        let prompt = crate::render_prompt_template(
            OBSERVED_ANSWER_FALLBACK_PROMPT_TEMPLATE,
            &[
                ("__USER_REQUEST__", "读一下 README 开头，然后用一句话总结"),
                ("__RESOLVED_USER_INTENT__", "读一下 README 开头，然后用一句话总结"),
                (
                    "__OUTPUT_CONTRACT__",
                    r#"{"response_shape":"one_sentence","semantic_kind":"content_excerpt_summary"}"#,
                ),
                ("__OBSERVED_OUTPUTS__", "### step_1 skill(read_file)\n# RustClaw"),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
    fn direct_answer_passthroughs_contract_filename_read_range_excerpt_without_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/README.md","resolved_path":"/tmp/README.md","excerpt":"1|# RustClaw\n2|\n3|<img src=\"./RustClaw.png\" width=\"420\" />\n4|"}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            Some("# RustClaw\n\n<img src=\"./RustClaw.png\" width=\"420\" />")
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "先读一下 README.md 前 4 行，再用三句话总结".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "route_contract:generic_filename_read_range".to_string(),
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
    fn direct_answer_prefers_current_turn_excerpt_summary_request_over_resolved_intent_drift() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/README.md","resolved_path":"/tmp/README.md","excerpt":"1|# RustClaw\n2|\n3|A tool runtime\n4|"}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "先读一下 README.md 前 4 行".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "route_contract:generic_filename_read_range".to_string(),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "读 /tmp/package.json，告诉我 scripts 字段下都有哪些子键".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "route_contract:generic_explicit_path_structured_keys".to_string(),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "读 /tmp/package.json，用一句话告诉我 scripts 字段下有哪些子键"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "route_contract:generic_explicit_path_structured_keys".to_string(),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent:
                "读取 /tmp/config.toml 里的 database.sqlite_path 和 tools.allow_sudo，告诉我两个字段的值"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "route_contract:generic_explicit_path_extract_fields"
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
    fn direct_answer_preserves_inventory_dir_names_without_request_text_limit() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/logs","resolved_path":"/tmp/logs","names_only":true,"names":["a","b","c","d"]}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
    fn direct_answer_does_not_use_current_turn_request_text_to_truncate_inventory_dir() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/logs","resolved_path":"/tmp/logs","names_only":true,"names":["a","b","c","d"]}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "列出 logs 目录下的文件名".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "normalizer:chat_act".to_string(),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
    fn hidden_entries_explanation_requests_defer_to_synthesis() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            ".git\nREADME.md\n.env\nsrc\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            Some("有。示例：.git, .env")
        );
    }

    #[test]
    fn direct_answer_formats_hidden_entries_check_act_free_from_listing() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            ".cargo/\nREADME.md\n.dockerignore\n.env.example\nsrc\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("有。示例：.cargo/, .dockerignore, .env.example")
        );
    }

    #[test]
    fn direct_answer_formats_hidden_entries_check_from_system_basic_inventory_dir() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/workspace","resolved_path":"/tmp/workspace","names_only":true,"include_hidden":true,"names":[".cargo",".dockerignore",".env.example","README.md","src"]}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("有。示例：.cargo, .dockerignore, .env.example")
        );
    }

    #[test]
    fn direct_answer_formats_existence_with_path_from_system_basic_path_batch_facts_even_when_free()
    {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"rustclaw.service","resolved_path":"/tmp/rustclaw-workspace/rustclaw.service","size_bytes":1190},"path":"/tmp/rustclaw-workspace/rustclaw.service"}],"include_missing":true}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
    fn direct_answer_formats_existence_with_path_from_run_cmd_yes_output() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd_observed_exists_yes_{}_{}",
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
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "run_cmd", "yes\n"));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
        let expected = format!("有，路径：{resolved}");
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
        let resolved = target
            .canonicalize()
            .unwrap_or(target.clone())
            .to_string_lossy()
            .to_string();

        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "run_cmd", "exists\n"));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
        let expected = format!("有，路径：{resolved}");
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
    fn workspace_project_summary_is_not_hard_summarized_by_observed_output() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            "Cargo.toml\ncrates/\nUI/\nconfigs/\nREADME.md\nREADME.zh-CN.md\nprompts/\nrustclaw.service\nstart-telegramd.sh\nstart-wechatd.sh\nstart-whatsappd.sh\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "数一下 scripts 目录直接子项有多少个，只输出数字".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "route_contract:current_workspace_scalar_count".to_string(),
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
    fn direct_scalar_uses_inventory_dir_hidden_count_for_hidden_entries_count_route() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":".","resolved_path":"/tmp/workspace","include_hidden":true,"names_only":true,"names":[".git",".env","README.md"],"counts":{"total":3,"hidden":2}}"#,
        ));
        let route_result = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "数一下当前目录里以点开头的隐藏文件有几个，只输出数字".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "route_contract:current_workspace_hidden_entries_count".to_string(),
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
    fn direct_answer_defers_package_manager_detect_summary_to_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "package_manager",
            "package_manager=brew",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "看看当前机器识别到的包管理器，再一句话说最可能日常会用哪个"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "route_contract:package_manager_detect_summary".to_string(),
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
            None
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "只输出当前机器识别到的包管理器名称".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "route_contract:package_manager_detect_scalar".to_string(),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent:
                "看看 data/db-basic-contract.sqlite 里有哪些表，再一句话说这更像业务库还是测试库"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "normalizer:chat_act".to_string(),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent:
                "看一下 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 里有哪些表，只输出表名"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "normalizer:chat_act".to_string(),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
    fn sqlite_table_listing_summary_defers_to_synthesis() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "db_basic",
            r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"users"}]}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "列一下 data/app.sqlite 里有哪些表".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "normalizer:chat_act".to_string(),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "比较 Cargo.toml 和 Cargo.lock 哪个更大，顺手用一句通俗话解释原因"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "route_contract:compare_targets".to_string(),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "比较 Cargo.toml 和 Cargo.lock 哪个更大".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "route_contract:compare_targets".to_string(),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
    fn direct_answer_defers_git_status_clean_when_exit_only_to_llm() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "git_basic", "exit=0\n"));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
    fn direct_answer_passes_health_check_json_through_for_act_free_shape() {
        let mut loop_state = LoopState::new(2);
        let body = r#"{"clawd_health_port_open":true,"telegramd_process_count":0}"#;
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "health_check", body));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(body)
        );
    }

    #[test]
    fn direct_answer_does_not_use_health_check_request_text_to_force_summary() {
        let mut loop_state = LoopState::new(2);
        let body = r#"{"clawd_process_count":7,"telegramd_process_count":0,"clawd_health_port_open":false,"clawd_log":{"exists":false},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#;
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "health_check", body));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
    fn direct_answer_defers_service_control_status_summary_to_llm_for_chinese_request() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "service_control",
            r#"{"status":"ok","service_name":"telegramd","manager_type":"rustclaw","requested_action":"status","executed_actions":["status"],"pre_state":"telegramd=stopped","post_state":"telegramd=stopped","verified":true,"key_evidence":["telegramd process_count=0 memory_rss_bytes=Some(0)"],"failure_reason":"","next_step":"","summary":"Status: telegramd=stopped"}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "telegramd".to_string(),
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
    fn direct_answer_defers_service_control_status_summary_to_llm_for_english_request() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "service_control",
            r#"{"status":"ok","service_name":"telegramd","manager_type":"rustclaw","requested_action":"status","executed_actions":["status"],"pre_state":"telegramd=running","post_state":"telegramd=running","verified":true,"key_evidence":["telegramd process_count=1 memory_rss_bytes=Some(1024)"],"failure_reason":"","next_step":"","summary":"Status: telegramd=running"}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
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
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "telegramd".to_string(),
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
