use super::*;

pub(super) fn latest_successful_list_dir_answer_candidate(
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

pub(super) fn canonical_existing_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

pub(super) fn resolve_listing_entry_full_path(
    entry: &str,
    auto_locator_path: Option<&str>,
) -> Option<String> {
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

pub(super) fn normalized_listing_text(listing: &str) -> Option<String> {
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

pub(super) fn looks_like_shell_long_listing_line(line: &str) -> bool {
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

pub(super) fn current_turn_request_text<'a>(
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

pub(super) fn route_requests_scalar_count(route: &crate::RouteResult) -> bool {
    super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::ScalarCount,
    )
}

pub(super) fn route_requests_scalar_existence(route: &crate::RouteResult) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::ExistenceWithPath,
        )
        && !route.output_contract.delivery_required
}

pub(super) fn route_requests_hidden_entries_check(route: &crate::RouteResult) -> bool {
    super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::HiddenEntriesCheck,
    )
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

pub(super) fn route_allows_scalar_read_range_direct_answer(route: &crate::RouteResult) -> bool {
    matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
    ) && !route.output_contract.delivery_required
        && (super::output_route_policy::route_is_unclassified_contract(route)
            || super::output_route_policy::route_contract_marker_is_any(
                route,
                &[
                    crate::OutputSemanticKind::ContentExcerptSummary,
                    crate::OutputSemanticKind::RawCommandOutput,
                ],
            ))
}

pub(super) fn route_requests_scalar_path_only(route: &crate::RouteResult) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::ScalarPathOnly,
        )
}

pub(super) fn route_requests_file_basename(route: &crate::RouteResult) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::FileBasename,
        )
}

pub(super) fn route_allows_path_batch_scalar_path_observed_answer(
    route: &crate::RouteResult,
) -> bool {
    route_requests_scalar_path_only(route)
        && !route.output_contract.requires_content_evidence
        && !route.has_route_reason_machine_marker("execution_required_read_file_extract_scalar")
        && !route.has_route_reason_machine_marker(
            "request_requires_fresh_file_observation_to_extract_title",
        )
}

pub(super) fn route_allows_path_batch_file_basename_observed_answer(
    route: &crate::RouteResult,
) -> bool {
    route_requests_file_basename(route)
        && !route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
}

pub(super) fn recent_file_path_candidate_for_scalar_path(
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

pub(super) fn route_prefers_plain_fs_search_paths(route: &crate::RouteResult) -> bool {
    route_requests_scalar_path_only(route)
        || (route.output_contract.response_shape == crate::OutputResponseShape::Strict
            && route.output_contract.locator_kind == crate::OutputLocatorKind::Path
            && super::output_route_policy::route_contract_marker_is(
                route,
                crate::OutputSemanticKind::ExistenceWithPath,
            )
            && !route.output_contract.delivery_required)
        || (matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        ) && super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::FileNames,
        ))
        || (matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        ) && super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::DirectoryNames,
        ))
        || (matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        ) && super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::FilePaths,
        ))
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

pub(super) fn route_scalar_has_plain_path_terminal_respond(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::ExistenceWithPath,
        )
        && route.output_contract.locator_kind == crate::OutputLocatorKind::Path
        && !route.output_contract.delivery_required
        && loop_state
            .last_user_visible_respond
            .as_deref()
            .is_some_and(looks_like_plain_path_literal)
}

pub(super) fn route_allows_raw_listing_direct_answer(route: Option<&crate::RouteResult>) -> bool {
    route.is_none_or(|route| {
        if !route.output_contract.requires_content_evidence {
            return true;
        }
        if !route.output_contract.delivery_required
            && route.output_contract.locator_kind == crate::OutputLocatorKind::Path
            && (super::output_route_policy::route_is_unclassified_contract(route)
                || super::output_route_policy::route_contract_marker_is(
                    route,
                    crate::OutputSemanticKind::ExistenceWithPath,
                ))
            && route.ask_mode.is_plain_act()
        {
            return true;
        }
        super::output_route_policy::route_contract_marker_is_any(
            route,
            &[
                crate::OutputSemanticKind::FileNames,
                crate::OutputSemanticKind::DirectoryNames,
                crate::OutputSemanticKind::DirectoryEntryGroups,
                crate::OutputSemanticKind::FilePaths,
            ],
        )
    })
}

pub(super) fn route_allows_strict_plain_observation_passthrough(
    route: &crate::RouteResult,
) -> bool {
    route.ask_mode.finalize_chat_wrapped()
        && route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && super::output_route_policy::route_is_unclassified_contract(route)
        && route.output_contract.response_shape == crate::OutputResponseShape::Strict
        && route.output_contract.exact_sentence_count.is_none()
}

pub(super) fn strict_plain_observation_passthrough_candidate(body: &str) -> Option<String> {
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

pub(super) fn is_user_hidden_entry(entry: &str) -> bool {
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

pub(super) fn latest_hidden_entries(loop_state: &LoopState) -> Option<Vec<String>> {
    let idx = latest_successful_step_index(loop_state, |_| true)?;
    let step = &loop_state.executed_step_results[idx];
    let body = step.output.as_deref().unwrap_or_default();
    match step.skill.as_str() {
        "system_basic" | "fs_basic" => {
            let body = normalized_success_body_for_direct_answer(body);
            serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|value| inventory_dir_hidden_entries(&value))
        }
        "list_dir" => {
            normalized_observed_listing(body).map(|listing| hidden_entries_from_listing(&listing))
        }
        "run_cmd" => run_cmd_listing_text_candidate(body, None)
            .map(|listing| hidden_entries_from_listing(&listing)),
        _ => None,
    }
}

fn hidden_entries_answer_limit(route: &crate::RouteResult) -> usize {
    route
        .output_contract
        .self_extension
        .list_selector
        .limit
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or(8)
        .min(8)
}

pub(super) fn hidden_entries_direct_answer(
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
    let hidden_entries = latest_hidden_entries(loop_state)?;
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
            .take(hidden_entries_answer_limit(route))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

pub(super) fn count_answer_from_latest_listing(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<String> {
    if !route_requests_scalar_count(route) {
        return None;
    }
    let listing = latest_list_dir_listing(loop_state)?;
    Some(normalized_listing_entry_count(&listing).to_string())
}

pub(super) fn count_answer_from_latest_fs_search(
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

pub(super) fn directory_purpose_summary_find_ext_answer_candidate(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<String> {
    if !super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::DirectoryPurposeSummary,
    ) || route.output_contract.delivery_required
        || route.output_contract.requires_content_evidence
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        )
    {
        return None;
    }
    let (results, count, ext) = latest_find_ext_results(loop_state)?;
    if count == 0 || results.is_empty() {
        return None;
    }

    let mut lines = vec![
        format!("find_ext.ext={ext}"),
        format!("find_ext.count={count}"),
    ];
    lines.extend(results.iter().map(|path| format!("find_ext.result={path}")));
    lines.extend(find_ext_representative_lines(&results));
    Some(lines.join("\n"))
}
