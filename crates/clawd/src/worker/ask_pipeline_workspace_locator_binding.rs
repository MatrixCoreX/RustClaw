use super::*;

#[cfg(test)]
#[path = "ask_pipeline_workspace_locator_binding_tests.rs"]
mod tests;

pub(super) fn current_request_has_concrete_locator_surface(prompt: &str) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    let filename_candidates = surface.filename_candidates_excluding_field_selectors();
    surface.has_explicit_path_or_url()
        || !filename_candidates.is_empty()
        || surface.locator_target_pair.is_some()
        || surface.has_delivery_token_reference()
        || surface.is_structural_locator_only_reply()
}

pub(super) fn current_request_resolves_workspace_child_locator(
    state: &AppState,
    prompt: &str,
) -> Option<String> {
    let explicit_path_locators =
        crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(prompt)
            .into_iter()
            .filter(|locator| matches!(locator.locator_kind, crate::OutputLocatorKind::Path))
            .collect::<Vec<_>>();
    if !explicit_path_locators.is_empty() {
        if let Some(path) = explicit_path_locators.into_iter().find_map(|locator| {
            resolve_existing_workspace_locator_hint(state, &locator.locator_hint)
        }) {
            return Some(path);
        }
    }
    super::try_resolve_workspace_child_locator_from_text(
        &state.skill_rt.workspace_root,
        &state.skill_rt.default_locator_search_dir,
        prompt,
    )
}

pub(super) fn current_request_has_structural_locator_surface_for_route(
    state: &AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    let has_concrete_locator_surface = current_request_has_concrete_locator_surface(prompt);
    if surface.has_deictic_reference() && !has_concrete_locator_surface {
        return false;
    }
    if surface.has_structured_target_refinement() && !has_concrete_locator_surface {
        return false;
    }
    if auto_locator_binding::route_reason_has_structured_field_selector_marker(route_result)
        && !structured_field_route_has_current_locator_surface(state, &surface)
    {
        return false;
    }
    if has_concrete_locator_surface {
        return true;
    }
    if route_result.output_contract.requires_content_evidence
        && !command_observation_route_has_runtime_evidence(state, prompt, route_result)
        && workspace_child_locator_surface_can_bind_route(route_result)
    {
        let Some(path) = current_request_resolves_workspace_child_locator(state, prompt) else {
            return false;
        };
        return implicit_workspace_child_locator_counts_as_structural(&path)
            || route_locator_hint_matches_resolved_workspace_child(state, route_result, &path);
    }
    false
}

pub(super) fn recent_artifacts_judgment_can_use_recent_execution_context(
    route_result: &crate::RouteResult,
    recent_execution_context: &str,
) -> bool {
    route_result.output_contract.semantic_kind == crate::OutputSemanticKind::RecentArtifactsJudgment
        && route_result.output_contract.requires_content_evidence
        && route_result.output_contract.locator_kind == crate::OutputLocatorKind::None
        && route_result.output_contract.locator_hint.trim().is_empty()
        && recent_execution_result_segments(recent_execution_context).len() >= 2
}

fn implicit_workspace_child_locator_counts_as_structural(path: &str) -> bool {
    !std::path::Path::new(path).is_file()
}

fn route_locator_hint_matches_resolved_workspace_child(
    state: &AppState,
    route_result: &crate::RouteResult,
    resolved_path: &str,
) -> bool {
    let locator_hint = route_result.output_contract.locator_hint.trim();
    if locator_hint.is_empty() {
        return false;
    }
    let Some(hint_path) = resolve_existing_workspace_locator_hint(state, locator_hint) else {
        return false;
    };
    normalize_workspace_locator_path(std::path::Path::new(&hint_path))
        == normalize_workspace_locator_path(std::path::Path::new(resolved_path))
}

pub(super) fn structured_field_route_has_current_locator_surface(
    state: &AppState,
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    if surface.has_explicit_path_or_url()
        || surface.locator_target_pair.is_some()
        || surface.has_delivery_token_reference()
        || surface.is_structural_locator_only_reply()
    {
        return true;
    }
    surface
        .filename_candidates_excluding_field_selectors()
        .into_iter()
        .any(|candidate| resolve_existing_workspace_locator_hint(state, &candidate).is_some())
}

pub(super) fn path_scoped_locator_guard_can_defer_to_prompt_targets(
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    if route_result.needs_clarify
        || !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
        )
        || !route_result.output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    surface.has_filename_candidates()
}

pub(super) fn promote_clarify_path_scoped_filename_targets_to_execute(
    prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.needs_clarify
        || route_result.output_contract.delivery_required
        || !route_result.output_contract.requires_content_evidence
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if !surface.has_filename_candidates()
        || !clarify_filename_target_contract_can_promote(route_result, &surface)
    {
        return false;
    }
    promote_clarify_observation_to_execute_with_locator(
        route_result,
        crate::OutputLocatorKind::CurrentWorkspace,
        String::new(),
        "clarify_path_scoped_filename_targets_promoted_to_execute",
    )
}

pub(super) fn promote_clarify_resolved_multifile_targets_to_execute(
    state: &AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.needs_clarify
        || route_result.output_contract.delivery_required
        || !route_has_observable_evidence_requirement(route_result)
        || !super::has_multiple_distinct_explicit_local_path_locators(state, prompt, None)
    {
        return false;
    }
    promote_clarify_observation_to_execute_with_locator(
        route_result,
        crate::OutputLocatorKind::CurrentWorkspace,
        String::new(),
        "clarify_resolved_multifile_targets_promoted_to_execute",
    )
}

fn route_has_observable_evidence_requirement(route_result: &crate::RouteResult) -> bool {
    route_result.output_contract.requires_content_evidence
        || route_reason_has_marker(route_result, "semantic_contract_requires_evidence")
}

fn clarify_filename_target_contract_can_promote(
    route_result: &crate::RouteResult,
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    match route_result.output_contract.locator_kind {
        crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename => {
            let hint = route_result.output_contract.locator_hint.trim();
            if hint.is_empty() {
                return true;
            }
            let prompt_targets = surface.filename_candidates_excluding_field_selectors();
            !prompt_targets.is_empty()
                && prompt_targets.iter().any(|target| {
                    crate::delivery_utils::extract_filename_candidates(hint)
                        .iter()
                        .any(|hint_target| hint_target.eq_ignore_ascii_case(target))
                })
        }
        crate::OutputLocatorKind::CurrentWorkspace => true,
        _ => false,
    }
}

fn workspace_child_locator_surface_can_bind_route(route_result: &crate::RouteResult) -> bool {
    !semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind)
        || scalar_equality_route_requests_workspace_child_locator(route_result)
}

pub(super) fn implicit_workspace_file_locator_route_should_force_clarify(
    state: &AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if route_result.needs_clarify
        || !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || generated_file_delivery_uses_runtime_target(route_result)
        || !locator_kind_accepts_workspace_child_prebind(route_result.output_contract.locator_kind)
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || current_request_has_concrete_locator_surface(prompt)
        || current_request_has_self_contained_structured_payload(prompt)
        || state_patch_allows_deictic_locator_guard_bypass(turn_analysis)
        || session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot)
        || active_session_has_structured_observation_anchor(session_snapshot)
        || route_should_skip_workspace_child_prebind(route_result)
        || command_observation_route_has_runtime_evidence(state, prompt, route_result)
        || archive_unpack_requires_structural_locator_pair(route_result)
        || super::has_multiple_distinct_explicit_local_path_locators(state, prompt, None)
    {
        return false;
    }
    let Some(path) = current_request_resolves_workspace_child_locator(state, prompt) else {
        return false;
    };
    std::path::Path::new(&path).is_file()
}

fn scalar_equality_route_requests_workspace_child_locator(
    route_result: &crate::RouteResult,
) -> bool {
    route_result.output_contract.requires_content_evidence
        && route_result.output_contract.semantic_kind
            == crate::OutputSemanticKind::RecentScalarEqualityCheck
        && locator_kind_accepts_workspace_child_prebind(route_result.output_contract.locator_kind)
}

fn route_should_skip_workspace_child_prebind(route_result: &crate::RouteResult) -> bool {
    semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind)
        && !scalar_equality_route_requests_workspace_child_locator(route_result)
}

pub(super) fn prebind_workspace_child_locator_from_current_request(
    state: &AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if route_result.needs_clarify
        || !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || is_bare_topic_only_prompt(prompt)
        || !locator_kind_accepts_workspace_child_prebind(route_result.output_contract.locator_kind)
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || route_should_skip_workspace_child_prebind(route_result)
        || command_observation_route_has_runtime_evidence(state, prompt, route_result)
        || archive_unpack_requires_structural_locator_pair(route_result)
        || super::has_multiple_distinct_explicit_local_path_locators(state, prompt, None)
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

fn workspace_root_name_token_present(workspace_root: &std::path::Path, text: &str) -> bool {
    let Some(root_name) = workspace_root.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let normalized_root = normalize_locator_identity_token(root_name);
    if normalized_root.is_empty() {
        return false;
    }
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')))
        .map(normalize_locator_identity_token)
        .any(|token| token == normalized_root)
}

fn workspace_root_current_request_binding_contract(route_result: &crate::RouteResult) -> bool {
    route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && !route_result.wants_file_delivery
        && route_result.output_contract.locator_hint.trim().is_empty()
        && locator_kind_accepts_workspace_child_prebind(route_result.output_contract.locator_kind)
        && matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Free
                | crate::OutputResponseShape::OneSentence
                | crate::OutputResponseShape::Strict
        )
        && matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
                | crate::OutputSemanticKind::WorkspaceProjectSummary
                | crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::ContentExcerptWithSummary
                | crate::OutputSemanticKind::DirectoryPurposeSummary
        )
}

pub(super) fn prebind_workspace_root_locator_from_current_request(
    state: &AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if is_bare_topic_only_prompt(prompt)
        || !workspace_root_current_request_binding_contract(route_result)
        || command_observation_route_has_runtime_evidence(state, prompt, route_result)
        || archive_unpack_requires_structural_locator_pair(route_result)
    {
        return false;
    }
    if !workspace_root_name_token_present(&state.skill_rt.workspace_root, prompt)
        && !workspace_root_name_token_present(
            &state.skill_rt.workspace_root,
            &route_result.resolved_intent,
        )
    {
        return false;
    }
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
    promote_clarify_observation_to_execute_with_locator(
        route_result,
        crate::OutputLocatorKind::CurrentWorkspace,
        String::new(),
        "workspace_root_locator_prebound_from_current_request",
    )
}

fn locator_kind_accepts_workspace_child_prebind(kind: crate::OutputLocatorKind) -> bool {
    matches!(
        kind,
        crate::OutputLocatorKind::None
            | crate::OutputLocatorKind::Path
            | crate::OutputLocatorKind::Filename
            | crate::OutputLocatorKind::CurrentWorkspace
    )
}

pub(super) const WORKSPACE_LOCATOR_HINT_PREBOUND_FROM_CURRENT_REQUEST: &str =
    "workspace_locator_hint_prebound_from_current_request";

pub(super) fn prebind_existing_workspace_locator_hint_from_current_request(
    state: &AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if route_result.needs_clarify
        || !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
        )
        || route_should_skip_workspace_child_prebind(route_result)
        || (!super::semantic_kind_can_bind_workspace_child_locator(
            route_result.output_contract.semantic_kind,
        ) && !scalar_equality_route_requests_workspace_child_locator(route_result))
    {
        return false;
    }
    let locator_hint = route_result.output_contract.locator_hint.trim();
    if locator_hint.is_empty() || !locator_hint_token_present_in_prompt(prompt, locator_hint) {
        return false;
    }
    let direct_child_stem_path =
        resolve_direct_child_stem_workspace_locator_hint(state, locator_hint);
    let resolved_hint_path = resolve_existing_workspace_locator_hint(state, locator_hint);
    let current_request_matching_hint_path = resolved_hint_path.as_deref().and_then(|path| {
        current_request_resolved_workspace_child_matches_path(state, prompt, path)
    });
    if locator_hint_file_name_has_extension(locator_hint)
        && !locator_hint_full_file_name_token_present_in_prompt(prompt, locator_hint)
        && current_request_matching_hint_path.is_none()
    {
        return false;
    }
    if !crate::worker::has_explicit_path_or_url_locator_hint(prompt)
        && locator_hint_token_ambiguous_in_workspace(state, locator_hint)
        && direct_child_stem_path.is_none()
        && current_request_matching_hint_path.is_none()
    {
        return false;
    }
    let Some(path) = current_request_matching_hint_path
        .or(resolved_hint_path)
        .or(direct_child_stem_path)
    else {
        return false;
    };
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = path;
    append_route_reason(
        route_result,
        WORKSPACE_LOCATOR_HINT_PREBOUND_FROM_CURRENT_REQUEST,
    );
    true
}

fn locator_hint_file_name_has_extension(locator_hint: &str) -> bool {
    std::path::Path::new(locator_hint.trim())
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|file_name| {
            file_name
                .rsplit_once('.')
                .is_some_and(|(_, ext)| !ext.is_empty())
        })
}

pub(super) fn model_completed_workspace_file_locator_hint_should_force_clarify(
    state: &AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if route_result.needs_clarify
        || !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || generated_file_delivery_uses_runtime_target(route_result)
        || is_bare_topic_only_prompt(prompt)
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
        )
        || route_result.output_contract.locator_hint.trim().is_empty()
        || route_reason_has_marker(route_result, "current_workspace_scope_from_current_request")
        || crate::worker::has_explicit_path_or_url_locator_hint(prompt)
        || current_request_has_self_contained_structured_payload(prompt)
        || state_patch_allows_deictic_locator_guard_bypass(turn_analysis)
        || session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot)
        || active_session_has_structured_observation_anchor(session_snapshot)
    {
        return false;
    }
    let locator_hint = route_result.output_contract.locator_hint.trim();
    if locator_hint_full_file_name_token_present_in_prompt(prompt, locator_hint) {
        return false;
    }
    let Some(path) = resolve_existing_workspace_locator_hint(state, locator_hint) else {
        return false;
    };
    if !std::path::Path::new(&path).is_file() {
        return false;
    }
    if current_request_resolved_workspace_child_matches_path(state, prompt, &path).is_some() {
        return false;
    }
    if locator_hint_file_name_has_extension(locator_hint)
        && !locator_hint_full_file_name_token_present_in_prompt(prompt, locator_hint)
    {
        return true;
    }
    current_request_resolved_workspace_child_matches_path(state, prompt, &path).is_none()
}

pub(super) fn inferred_missing_workspace_locator_hint_should_force_clarify(
    state: &AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if route_result.needs_clarify
        || !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || generated_file_delivery_uses_runtime_target(route_result)
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
        )
        || route_result.output_contract.locator_hint.trim().is_empty()
        || crate::worker::has_explicit_path_or_url_locator_hint(prompt)
        || current_request_has_self_contained_structured_payload(prompt)
        || state_patch_allows_deictic_locator_guard_bypass(turn_analysis)
        || session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot)
        || active_session_has_structured_observation_anchor(session_snapshot)
        || current_request_resolves_workspace_child_locator(state, prompt).is_some()
    {
        return false;
    }
    if current_request_file_delivery_locator_hint_can_defer_to_execution(prompt, route_result) {
        return false;
    }
    let Some(path) = direct_workspace_child_locator_hint_path(
        &state.skill_rt.workspace_root,
        route_result.output_contract.locator_hint.trim(),
    ) else {
        return false;
    };
    !path.exists()
}

fn current_request_file_delivery_locator_hint_can_defer_to_execution(
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    (route_result.wants_file_delivery
        || route_result.output_contract.delivery_required
        || route_result.output_contract.response_shape == crate::OutputResponseShape::FileToken
        || route_result.output_contract.delivery_intent == crate::OutputDeliveryIntent::FileSingle)
        && route_result.output_contract.locator_kind == crate::OutputLocatorKind::Filename
        && locator_hint_full_file_name_token_present_in_prompt(
            prompt,
            route_result.output_contract.locator_hint.trim(),
        )
}

fn direct_workspace_child_locator_hint_path(
    workspace_root: &std::path::Path,
    locator_hint: &str,
) -> Option<std::path::PathBuf> {
    let hint = locator_hint.trim();
    if hint.is_empty() || hint.contains('\n') {
        return None;
    }
    let hint_path = std::path::Path::new(hint);
    if hint_path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return None;
    }
    let root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let candidate = if hint_path.is_absolute() {
        hint_path.to_path_buf()
    } else {
        root.join(hint_path)
    };
    if !candidate.starts_with(&root) {
        return None;
    }
    let relative = candidate.strip_prefix(&root).ok()?;
    (relative.components().count() == 1).then_some(candidate)
}

pub(super) fn locator_hint_full_file_name_token_present_in_prompt(
    prompt: &str,
    locator_hint: &str,
) -> bool {
    let Some(file_name) = std::path::Path::new(locator_hint.trim())
        .file_name()
        .and_then(|value| value.to_str())
        .and_then(locator_component_token)
    else {
        return false;
    };
    structural_locator_token_candidates(prompt)
        .into_iter()
        .any(|token| token.eq_ignore_ascii_case(&file_name))
}

fn current_request_resolved_workspace_child_matches_path(
    state: &AppState,
    prompt: &str,
    resolved_path: &str,
) -> Option<String> {
    let current_path = current_request_resolves_workspace_child_locator(state, prompt)?;
    let current_path = normalize_workspace_locator_path(std::path::Path::new(&current_path));
    let resolved_path_buf = normalize_workspace_locator_path(std::path::Path::new(resolved_path));
    (current_path == resolved_path_buf).then(|| resolved_path.to_string())
}

pub(super) fn prebind_clarify_workspace_child_locator_from_current_request(
    state: &AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.needs_clarify
        || route_result.output_contract.delivery_required
        || !route_result.output_contract.requires_content_evidence
        || is_bare_topic_only_prompt(prompt)
        || !locator_kind_accepts_workspace_child_prebind(route_result.output_contract.locator_kind)
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || route_should_skip_workspace_child_prebind(route_result)
        || command_observation_route_has_runtime_evidence(state, prompt, route_result)
        || archive_unpack_requires_structural_locator_pair(route_result)
    {
        return false;
    }
    let Some(path) = current_request_resolves_workspace_child_locator(state, prompt) else {
        return false;
    };
    if super::has_multiple_distinct_explicit_local_path_locators(state, prompt, None)
        && !clarify_workspace_child_prebind_allows_multiple_structural_locators(
            state,
            prompt,
            route_result,
            &path,
        )
    {
        return false;
    }
    promote_clarify_observation_to_execute_with_locator(
        route_result,
        crate::OutputLocatorKind::Path,
        path,
        "workspace_child_locator_prebound_from_clarify_current_request",
    )
}

fn clarify_workspace_child_prebind_allows_multiple_structural_locators(
    state: &AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
    resolved_path: &str,
) -> bool {
    (route_result.output_contract.semantic_kind
        == crate::OutputSemanticKind::RecentScalarEqualityCheck
        && route_result.output_contract.response_shape == crate::OutputResponseShape::OneSentence)
        || super::workspace_child_resolution_is_directory_scope_with_child_filename(
            &state.skill_rt.workspace_root,
            prompt,
            resolved_path,
        )
}

pub(super) fn prebind_workspace_child_locator_from_resolved_prompt(
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
        || command_observation_route_has_runtime_evidence(state, resolved_prompt, route_result)
        || archive_unpack_requires_structural_locator_pair(route_result)
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

pub(super) fn archive_unpack_requires_structural_locator_pair(
    route_result: &crate::RouteResult,
) -> bool {
    route_result.output_contract.semantic_kind == crate::OutputSemanticKind::ArchiveUnpack
}
