use super::*;

#[cfg(test)]
#[path = "ask_pipeline_ordered_entry_binding_tests.rs"]
mod tests;

pub(super) fn prebind_content_evidence_locator_from_active_ordered_resolved_prompt(
    state: &AppState,
    resolved_prompt: &str,
    route_result: &mut crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if !route_result.needs_clarify
        || route_clarify_reason_code(route_result) != Some("missing_read_target")
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    let Some(path) = resolve_active_ordered_entry_target_from_resolved_context(
        state,
        resolved_prompt,
        &route_result.resolved_intent,
        session_snapshot,
    ) else {
        return false;
    };
    promote_clarify_observation_to_execute_with_locator(
        route_result,
        crate::OutputLocatorKind::Path,
        path,
        "content_evidence_locator_prebound_from_active_ordered_resolved_prompt",
    )
}

pub(super) fn resolve_recent_ordered_entry_target_from_resolved_prompt(
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

fn resolve_active_ordered_entry_target_from_resolved_context(
    state: &AppState,
    resolved_prompt: &str,
    resolved_intent: &str,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> Option<String> {
    let frame = session_snapshot.active_followup_frame.as_ref()?;
    if !matches!(frame.op_kind, crate::followup_frame::FollowupOpKind::List)
        || frame.ordered_entries.is_empty()
    {
        return None;
    }
    let context = format!("{resolved_prompt}\n{resolved_intent}");
    if context.trim().is_empty() {
        return None;
    }
    let mut matches = Vec::new();
    for (index, entry) in frame.ordered_entries.iter().enumerate() {
        let Some(target) = crate::followup_frame::ordered_entry_target_at(frame, index) else {
            continue;
        };
        if !text_mentions_locator_identity(&context, entry)
            && !text_mentions_locator_identity(&context, &target)
        {
            continue;
        }
        if let Some(path) = resolve_existing_workspace_locator_hint(state, &target)
            .or_else(|| resolve_existing_workspace_locator_hint(state, entry))
        {
            matches.push(path);
        }
    }
    matches.sort();
    matches.dedup();
    if matches.len() == 1 {
        matches.pop()
    } else {
        None
    }
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

pub(super) fn promote_clarify_observation_to_execute_with_locator(
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
