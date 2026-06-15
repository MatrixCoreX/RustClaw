use std::path::Path;

use super::{AgentRunContext, LoopState};
use crate::{llm_gateway, AppState, ClaimedTask};

#[path = "observed_output_text.rs"]
mod output_text;
use output_text::{
    extract_answer_from_finalizer_envelope_text, non_code_markdown_text,
    strip_bare_json_language_prefix, ObservedAnswerFallbackOut,
};
#[path = "observed_output_transform.rs"]
mod output_transform;
pub(crate) use output_transform::{
    direct_answer_from_referenced_observation_i18n, transform_skill_formatted_output_candidate,
};
#[path = "observed_output_success.rs"]
mod output_success;
pub(crate) use output_success::{
    extract_latest_generic_successful_output, normalized_success_body_for_direct_answer,
    GenericObservedOutput,
};
use output_success::{
    extract_latest_generic_successful_output_with_state, has_successful_step_for_skill,
    latest_successful_step_index,
};

#[path = "observed_output_listing.rs"]
mod output_listing;
#[cfg(test)]
use output_listing::route_prefers_direct_observed_answer_for_scalar;
pub(crate) use output_listing::scalar_route_prefers_structured_observed_answer;
use output_listing::{
    canonical_existing_path, count_answer_from_latest_fs_search, count_answer_from_latest_listing,
    current_turn_request_text, directory_purpose_summary_find_ext_answer_candidate,
    hidden_entries_direct_answer, is_user_hidden_entry, latest_hidden_entries,
    latest_successful_list_dir_answer_candidate, looks_like_shell_long_listing_line,
    normalized_listing_text, recent_file_path_candidate_for_scalar_path,
    resolve_listing_entry_full_path, route_allows_path_batch_scalar_path_observed_answer,
    route_allows_raw_listing_direct_answer, route_allows_scalar_read_range_direct_answer,
    route_allows_strict_plain_observation_passthrough, route_prefers_plain_fs_search_paths,
    route_requests_hidden_entries_check, route_requests_scalar_count,
    route_requests_scalar_existence, route_requests_scalar_path_only,
    route_scalar_has_plain_path_terminal_respond, strict_plain_observation_passthrough_candidate,
};

#[path = "observed_output_system_inventory.rs"]
mod output_system_inventory;
use output_system_inventory::{
    count_inventory_direct_answer_candidate, count_inventory_planned_file_dir_breakdown_answer,
    dir_compare_direct_answer_candidate, inventory_dir_direct_answer_candidate,
    inventory_dir_names, inventory_dir_observed_candidate, inventory_dir_scalar_path_candidate,
    system_basic_existence_with_path_value, system_basic_info_scalar_path_candidate,
    system_basic_info_value, system_basic_structured_doc_observed_body,
    system_basic_structured_doc_value, system_basic_value_looks_like_info,
    tree_summary_direct_answer_candidate,
};

#[path = "observed_output_fs_search.rs"]
mod output_fs_search;
use output_fs_search::{
    absolutize_fs_search_answer_paths, fs_search_content_presence_direct_answer_candidate,
    fs_search_direct_answer_candidate, fs_search_find_ext_results, fs_search_find_name_results,
    fs_search_grep_text_observed_candidate, fs_search_route_filtered_listing_candidate,
    fs_search_scalar_candidate, fs_search_semantic_listing_candidate, normalized_find_name_pattern,
    preferred_fs_search_exact_match,
};

#[path = "observed_output_path_facts.rs"]
mod output_path_facts;
use output_path_facts::*;

#[path = "observed_output_archive.rs"]
mod output_archive;
use output_archive::*;

#[path = "observed_output_git.rs"]
mod output_git;
pub(crate) use output_git::{
    answer_is_git_repository_state_machine_summary,
    git_repository_state_observation_from_status_output,
};
use output_git::{
    git_basic_direct_answer_candidate, git_basic_json_action,
    git_basic_observation_text_candidates, git_current_branch_from_json_value,
    latest_git_current_branch, latest_git_repository_state_direct_answer,
};

#[path = "observed_output_entries.rs"]
mod output_entries;
pub(crate) use output_entries::has_observed_answer_candidates;
#[cfg(test)]
use output_entries::recent_generated_output_from_user_request;
use output_entries::{
    compound_listing_content_delivery_guard_entry, cross_turn_observed_output_entries,
    execution_failed_step_guard_entry, git_repository_state_facts_entry, observed_output_entries,
    route_observation_facts_entry,
};

#[path = "observed_output_direct_scalar.rs"]
mod output_direct_scalar;
use output_direct_scalar::{
    market_quote_output_has_scalar_price, package_manager_summary_candidate,
    structured_scalar_candidate,
};

#[path = "observed_output_direct_answer.rs"]
mod output_direct_answer;
use output_direct_answer::{
    allows_normalized_scalar_direct_fallback, fs_search_output_direct_answer_candidate,
};
pub(crate) use output_direct_answer::{
    answer_is_direct_observation_passthrough, extract_direct_answer_from_generic_output,
    extract_direct_answer_from_generic_output_i18n,
};

#[path = "observed_output_read_range.rs"]
mod output_read_range;
pub(crate) use output_read_range::tail_read_range_direct_answer_candidate;
use output_read_range::{
    compose_content_excerpt_with_summary_answer, content_excerpt_summary_direct_answer_candidate,
    doc_parse_content_presence_direct_answer_candidate, normalize_read_range_excerpt,
    normalize_read_range_excerpt_for_direct_answer, read_range_observed_candidate,
    read_range_preserve_blank_lines,
};

#[path = "observed_output_structured_scalar.rs"]
mod output_structured_scalar;
#[cfg(test)]
use output_structured_scalar::structured_scalar_observation_from_extract_item;
pub(crate) use output_structured_scalar::{
    latest_structured_scalar_observation_text, recent_structured_scalar_observation_count,
    structured_scalar_equality_direct_answer,
};
use output_structured_scalar::{
    multiple_structured_scalar_observations_need_synthesis,
    route_needs_structured_scalar_pair_synthesis, structured_scalar_observation_from_value,
};

#[path = "observed_output_structured_fields.rs"]
mod output_structured_fields;
use output_structured_fields::*;

#[path = "observed_output_route_policy.rs"]
mod output_route_policy;
use output_route_policy::{
    observed_language_supports_bilingual_template, observed_request_language_hint,
    observed_request_prefers_english_template, observed_response_style_hint,
    route_git_repository_state_requires_language_synthesis,
    route_should_synthesize_non_bilingual_existence_with_path,
};
pub(crate) use output_route_policy::{
    route_disallows_direct_observation_passthrough,
    route_quantity_comparison_requires_model_language_synthesis,
    route_requires_synthesized_delivery,
};

#[path = "observed_output_sqlite.rs"]
mod output_sqlite;
use output_sqlite::{
    db_basic_count_candidate, db_basic_database_kind_judgment_candidate,
    db_basic_database_kind_judgment_from_loop_state_candidate, db_basic_scalar_candidate,
    db_basic_table_names, db_basic_tables_summary_candidate,
    run_cmd_sqlite_direct_answer_candidate,
};

#[path = "observed_output_process_service.rs"]
mod output_process_service;
use output_process_service::{
    latest_process_basic_service_status_direct_answer_candidate,
    process_basic_port_list_should_use_llm_synthesis,
    process_basic_service_status_direct_answer_candidate,
    service_control_status_direct_answer_candidate, service_control_summary_candidate,
};

#[path = "observed_output_scalar_text.rs"]
mod output_scalar_text;
use output_scalar_text::{
    normalized_scalar_candidate, scalar_count_diagnostic_line_for_answer, trim_for_observed_prompt,
    value_scalar_text, value_structured_text,
};

#[path = "observed_output_status_json.rs"]
mod output_status_json;
use output_status_json::{
    find_ext_representative_lines, latest_find_ext_results, multi_status_json_summary_candidate,
};

#[cfg(test)]
const OBSERVED_ANSWER_FALLBACK_PROMPT_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/observed_answer_fallback_prompt.md");
const OBSERVED_ANSWER_FALLBACK_PROMPT_LOGICAL_PATH: &str =
    "prompts/observed_answer_fallback_prompt.md";
const MARKET_QUOTE_SCALAR_SEMANTIC_TAG: &str = "market_quote_scalar";

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

fn count_inventory_observation_row(
    step: &crate::executor::StepExecutionResult,
) -> Option<(String, u64)> {
    if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
        return None;
    }
    let body = step.output.as_deref()?;
    let body = normalized_success_body_for_direct_answer(body);
    let value = serde_json::from_str::<serde_json::Value>(body.trim()).ok()?;
    if value.get("action").and_then(|v| v.as_str()) != Some("count_inventory") {
        return None;
    }
    let total = value
        .get("counts")
        .and_then(|counts| counts.get("total"))
        .and_then(|v| v.as_u64())?;
    let path = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(step.step_id.as_str())
        .to_string();
    Some((path, total))
}

fn multi_count_quantity_comparison_guard_entry(
    loop_state: &LoopState,
    route: Option<&crate::RouteResult>,
) -> Option<String> {
    let route = route?;
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::QuantityComparison {
        return None;
    }
    let rows = loop_state
        .executed_step_results
        .iter()
        .filter_map(count_inventory_observation_row)
        .collect::<Vec<_>>();
    if rows.len() < 2 {
        return None;
    }
    let mut lines = vec![
        "### multi_count_quantity_comparison_guard".to_string(),
        "delivery_constraint=cover_all_observed_count_rows".to_string(),
        format!("observed_count_rows={}", rows.len()),
    ];
    for (idx, (path, total)) in rows.iter().enumerate() {
        let row_no = idx + 1;
        lines.push(format!("observed_count.{row_no}.path={path}"));
        lines.push(format!("observed_count.{row_no}.count_total={total}"));
    }
    Some(lines.join("\n"))
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
    let observed_output = extract_latest_generic_successful_output_with_state(state, loop_state)?;
    if route_should_synthesize_non_bilingual_existence_with_path(
        route,
        allow_localized_direct_template,
    ) {
        return None;
    }
    if multiple_structured_scalar_observations_need_synthesis(route, loop_state) {
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
            route,
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
    latest_hidden_entries(loop_state).is_some_and(|hidden_entries| hidden_entries.is_empty())
}

fn route_requires_matrix_grounded_direct_candidate(route: &crate::RouteResult) -> bool {
    crate::contract_matrix::final_answer_shape_for_output_contract(&route.output_contract)
        .is_some_and(|shape| !shape.allows_model_language())
}

fn route_allows_model_language_direct_candidate(route: &crate::RouteResult) -> bool {
    crate::contract_matrix::final_answer_shape_for_output_contract(&route.output_contract)
        .is_some_and(|shape| shape.allows_model_language())
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

fn observed_contract_json(agent_run_context: Option<&AgentRunContext>) -> String {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return "{}".to_string();
    };
    let direct_observation_passthrough_allowed =
        !route_disallows_direct_observation_passthrough(route);
    serde_json::json!({
        "route_gate_kind": route.gate_kind().as_str(),
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

    if let Some(answer) = strict_raw_tail_read_observed_answer(loop_state, agent_run_context) {
        return Ok(Some((
            answer,
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                parsed: true,
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 1,
                evidence_quotes_count: 0,
                ..Default::default()
            },
        )));
    }

    let mut observed_entries = observed_output_entries(loop_state);
    if let Some(guard) = execution_failed_step_guard_entry(
        loop_state,
        agent_run_context.and_then(|ctx| ctx.route_result.as_ref()),
    ) {
        observed_entries = vec![guard];
    } else {
        if let Some(guard) = multi_count_quantity_comparison_guard_entry(
            loop_state,
            agent_run_context.and_then(|ctx| ctx.route_result.as_ref()),
        ) {
            observed_entries.insert(0, guard);
        }
        if let Some(route_facts) = route_observation_facts_entry(agent_run_context) {
            observed_entries.insert(0, route_facts);
        }
        if let Some(git_facts) = git_repository_state_facts_entry(
            loop_state,
            agent_run_context.and_then(|ctx| ctx.route_result.as_ref()),
        ) {
            observed_entries.insert(0, git_facts);
        }
        if let Some(guard) = compound_listing_content_delivery_guard_entry(
            loop_state,
            agent_run_context.and_then(|ctx| ctx.route_result.as_ref()),
        ) {
            observed_entries.insert(0, guard);
        }
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
        && (answer_is_direct_observation_passthrough(&answer, loop_state)
            || answer_is_git_repository_state_machine_summary(&answer));
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
            agent_run_context,
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

fn strict_raw_tail_read_observed_answer(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|context| context.route_result.as_ref())?;
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::RawCommandOutput
        || route.output_contract.response_shape != crate::OutputResponseShape::Strict
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
    {
        return None;
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "fs_basic" | "system_basic"))
        .filter_map(|step| step.output.as_deref())
        .find_map(strict_raw_tail_read_answer_from_output)
        .map(|answer| answer.trim().to_string())
        .filter(|answer| !answer.is_empty())
}

fn strict_raw_tail_read_answer_from_output(output: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    strict_raw_tail_read_answer_from_value(&value)
}

fn strict_raw_tail_read_answer_from_value(value: &serde_json::Value) -> Option<String> {
    if let Some(answer) = strict_raw_tail_read_answer_from_flat_value(value) {
        return Some(answer);
    }
    value
        .get("extra")
        .and_then(strict_raw_tail_read_answer_from_value)
        .or_else(|| {
            value
                .get("text")
                .and_then(serde_json::Value::as_str)
                .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
                .and_then(|inner| strict_raw_tail_read_answer_from_value(&inner))
        })
}

fn strict_raw_tail_read_answer_from_flat_value(value: &serde_json::Value) -> Option<String> {
    if !matches!(
        value.get("action").and_then(serde_json::Value::as_str),
        Some("read_range" | "read_text_range")
    ) || value.get("mode").and_then(serde_json::Value::as_str) != Some("tail")
    {
        return None;
    }
    let requested_n = value
        .get("requested_n")
        .or_else(|| value.get("n"))
        .or_else(|| value.get("count"))
        .and_then(serde_json::Value::as_u64)?;
    if requested_n == 0 || requested_n > 50 {
        return None;
    }
    value
        .get("excerpt")
        .and_then(serde_json::Value::as_str)
        .filter(|excerpt| !excerpt.trim().is_empty())?;
    let mut candidate = value.clone();
    let obj = candidate.as_object_mut()?;
    obj.insert(
        "action".to_string(),
        serde_json::Value::String("read_range".to_string()),
    );
    if !obj.contains_key("requested_n") {
        obj.insert(
            "requested_n".to_string(),
            serde_json::Value::Number(requested_n.into()),
        );
    }
    tail_read_range_direct_answer_candidate(&candidate.to_string(), false)
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
#[path = "observed_output_empty_values_tests.rs"]
mod empty_values_tests;
#[cfg(test)]
#[path = "observed_output_tests.rs"]
mod tests;
