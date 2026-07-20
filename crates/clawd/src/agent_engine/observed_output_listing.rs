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
    _route: Option<&'a crate::IntentOutputContract>,
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
}

pub(super) fn route_requests_scalar_count(route: &crate::IntentOutputContract) -> bool {
    super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::ScalarCount,
    )
}

pub(super) fn route_requests_scalar_existence(route: &crate::IntentOutputContract) -> bool {
    route.response_shape == crate::OutputResponseShape::Scalar
        && super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::ExistenceWithPath,
        )
        && !route.delivery_required
}

pub(crate) fn route_prefers_direct_observed_answer_for_scalar(
    route: &crate::IntentOutputContract,
) -> bool {
    route_requests_scalar_existence(route)
}

pub(crate) fn scalar_route_prefers_structured_observed_answer(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
) -> bool {
    route.response_shape == crate::OutputResponseShape::Scalar
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

pub(super) fn route_allows_scalar_read_range_direct_answer(
    route: &crate::IntentOutputContract,
) -> bool {
    matches!(
        route.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
    ) && !route.delivery_required
        && (super::output_route_policy::route_is_unclassified_contract(route)
            || super::output_route_policy::route_contract_marker_is_any(
                route,
                &[
                    crate::OutputSemanticKind::ContentExcerptSummary,
                    crate::OutputSemanticKind::RawCommandOutput,
                ],
            ))
}

pub(super) fn route_requests_exact_scalar_path(route: &crate::IntentOutputContract) -> bool {
    crate::machine_kv_projection::output_contract_requests_exact_scalar_path(route)
}

pub(super) fn exact_scalar_path_selector(route: &crate::IntentOutputContract) -> Option<String> {
    crate::machine_kv_projection::output_contract_exact_scalar_field(
        route,
        &["path", "resolved_path"],
    )
}

pub(super) fn route_allows_path_batch_scalar_path_observed_answer(
    route: &crate::IntentOutputContract,
) -> bool {
    route_requests_exact_scalar_path(route) && !route.requires_content_evidence
}

pub(super) fn recent_file_path_candidate_for_scalar_path(
    loop_state: &LoopState,
    route: Option<&crate::IntentOutputContract>,
) -> Option<String> {
    if !route.is_some_and(route_requests_exact_scalar_path) {
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

pub(super) fn route_prefers_plain_fs_search_paths(route: &crate::IntentOutputContract) -> bool {
    route_requests_exact_scalar_path(route)
        || (route.response_shape == crate::OutputResponseShape::Strict
            && route.locator_kind == crate::OutputLocatorKind::Path
            && super::output_route_policy::route_contract_marker_is(
                route,
                crate::OutputSemanticKind::ExistenceWithPath,
            )
            && !route.delivery_required)
        || (matches!(
            route.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        ) && super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::FileNames,
        ))
        || (matches!(
            route.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        ) && super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::DirectoryNames,
        ))
        || (matches!(
            route.response_shape,
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
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
) -> bool {
    route.response_shape == crate::OutputResponseShape::Scalar
        && super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::ExistenceWithPath,
        )
        && route.locator_kind == crate::OutputLocatorKind::Path
        && !route.delivery_required
        && loop_state
            .last_user_visible_respond
            .as_deref()
            .is_some_and(looks_like_plain_path_literal)
}

pub(super) fn route_allows_raw_listing_direct_answer(
    route: Option<&crate::IntentOutputContract>,
) -> bool {
    route.is_none_or(|route| {
        if !route.requires_content_evidence {
            return true;
        }
        if !route.delivery_required
            && route.locator_kind == crate::OutputLocatorKind::Path
            && (super::output_route_policy::route_is_unclassified_contract(route)
                || super::output_route_policy::route_contract_marker_is(
                    route,
                    crate::OutputSemanticKind::ExistenceWithPath,
                ))
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

fn latest_list_dir_listing(loop_state: &LoopState) -> Option<String> {
    let idx = latest_successful_step_index(loop_state, |step| step.skill == "list_dir")?;
    let step = &loop_state.executed_step_results[idx];
    if !step.is_ok() || step.skill != "list_dir" {
        return None;
    }
    normalized_observed_listing(step.output.as_deref().unwrap_or_default())
}

pub(super) fn count_answer_from_latest_listing(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
) -> Option<String> {
    if !route_requests_scalar_count(route) {
        return None;
    }
    let listing = latest_list_dir_listing(loop_state)?;
    Some(normalized_listing_entry_count(&listing).to_string())
}

pub(super) fn count_answer_from_latest_fs_search(
    route: &crate::IntentOutputContract,
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
