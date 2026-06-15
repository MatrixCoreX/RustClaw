pub(super) fn active_observed_facts_have_bound_target(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    session_snapshot
        .active_observed_facts
        .as_ref()
        .and_then(|facts| facts.bound_target.as_deref())
        .map(str::trim)
        .is_some_and(|target| !target.is_empty())
}

fn active_bound_targets(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> Vec<&str> {
    let mut targets = Vec::new();
    if let Some(facts) = session_snapshot.active_observed_facts.as_ref() {
        if let Some(target) = facts
            .bound_target
            .as_deref()
            .map(str::trim)
            .filter(|target| !target.is_empty())
        {
            targets.push(target);
        }
    }
    if let Some(frame) = session_snapshot.active_followup_frame.as_ref() {
        if matches!(
            frame.op_kind,
            crate::followup_frame::FollowupOpKind::Read
                | crate::followup_frame::FollowupOpKind::List
        ) {
            if let Some(target) = frame
                .bound_target
                .as_deref()
                .map(str::trim)
                .filter(|target| !target.is_empty())
            {
                targets.push(target);
            }
        }
    }
    targets
}

pub(super) fn single_component_locator_hint(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_matches(|ch: char| {
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
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || std::path::Path::new(trimmed).is_absolute()
        || std::path::Path::new(trimmed).components().count() != 1
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn path_basename_eq(path: &str, basename: &str) -> bool {
    let Some(candidate) = single_component_locator_hint(basename) else {
        return false;
    };
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case(&candidate))
        .unwrap_or(false)
}

fn active_bound_target_for_locator_hint<'a>(
    session_snapshot: &'a crate::conversation_state::ActiveSessionSnapshot,
    locator_hint: &str,
) -> Option<&'a str> {
    let hint = single_component_locator_hint(locator_hint)?;
    active_bound_targets(session_snapshot)
        .into_iter()
        .find(|target| path_basename_eq(target, &hint))
}

pub(super) fn prebind_active_bound_target_from_matching_locator_hint(
    route_result: &mut crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if route_result.needs_clarify
        || !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || super::semantic_kind_can_execute_without_locator(
            route_result.output_contract.semantic_kind,
        )
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        return false;
    }
    let locator_hint = route_result.output_contract.locator_hint.trim();
    let Some(target) = active_bound_target_for_locator_hint(session_snapshot, locator_hint) else {
        return false;
    };
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = target.to_string();
    super::append_route_reason(
        route_result,
        "active_bound_target_prebound_from_matching_locator_hint",
    );
    true
}

fn active_bound_target_semantic_kind_can_prebind(kind: crate::OutputSemanticKind) -> bool {
    matches!(
        kind,
        crate::OutputSemanticKind::ContentExcerptSummary
            | crate::OutputSemanticKind::ContentExcerptWithSummary
            | crate::OutputSemanticKind::ContentPresenceCheck
            | crate::OutputSemanticKind::DirectoryPurposeSummary
            | crate::OutputSemanticKind::ExcerptKindJudgment
    )
}

fn locator_kind_for_bound_target(target: &str) -> crate::OutputLocatorKind {
    if target.starts_with("http://") || target.starts_with("https://") {
        crate::OutputLocatorKind::Url
    } else {
        crate::OutputLocatorKind::Path
    }
}

pub(super) fn prebind_active_bound_target_for_locatorless_content_evidence(
    route_result: &mut crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if route_result.needs_clarify
        || !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        || !matches!(
            route_result.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        )
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || !active_bound_target_semantic_kind_can_prebind(
            route_result.output_contract.semantic_kind,
        )
    {
        return false;
    }
    let Some(target) = active_bound_targets(session_snapshot)
        .into_iter()
        .next()
        .map(ToString::to_string)
    else {
        return false;
    };
    route_result.output_contract.locator_kind = locator_kind_for_bound_target(&target);
    route_result.output_contract.locator_hint = target;
    super::append_route_reason(
        route_result,
        "active_bound_target_prebound_for_locatorless_content_evidence",
    );
    true
}

pub(super) fn repair_service_status_file_locator_to_content_excerpt(
    state: &crate::AppState,
    route_result: &mut crate::RouteResult,
) -> bool {
    if route_result.output_contract.semantic_kind != crate::OutputSemanticKind::ServiceStatus
        || route_result.wants_file_delivery
        || route_result.output_contract.delivery_required
        || !route_result.output_contract.requires_content_evidence
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
        )
    {
        return false;
    }
    let Some(path) = super::resolve_existing_workspace_locator_hint(
        state,
        &route_result.output_contract.locator_hint,
    ) else {
        return false;
    };
    if !std::path::Path::new(&path).is_file() {
        return false;
    }
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = path;
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    super::append_route_reason(
        route_result,
        "service_status_file_locator_repaired_to_content_excerpt",
    );
    true
}

fn active_listing_bound_target_for_scalar_count<'a>(
    session_snapshot: &'a crate::conversation_state::ActiveSessionSnapshot,
) -> Option<&'a str> {
    if let Some(frame) = session_snapshot.active_followup_frame.as_ref() {
        if matches!(frame.op_kind, crate::followup_frame::FollowupOpKind::List)
            && !frame.ordered_entries.is_empty()
        {
            if let Some(target) = frame
                .bound_target
                .as_deref()
                .map(str::trim)
                .filter(|target| !target.is_empty())
            {
                return Some(target);
            }
        }
    }
    if let Some(facts) = session_snapshot.active_observed_facts.as_ref() {
        if facts.observed_entry_count.is_some() || !facts.ordered_entries.is_empty() {
            if let Some(target) = facts
                .bound_target
                .as_deref()
                .map(str::trim)
                .filter(|target| !target.is_empty())
            {
                return Some(target);
            }
        }
    }
    None
}

pub(super) fn prebind_active_listing_target_for_locatorless_scalar_count(
    route_result: &mut crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.semantic_kind != crate::OutputSemanticKind::ScalarCount
        || route_result.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || !matches!(
            route_result.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        )
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    let Some(target) =
        active_listing_bound_target_for_scalar_count(session_snapshot).map(ToString::to_string)
    else {
        return false;
    };
    if route_result.needs_clarify || route_result.is_chat_gate() {
        return super::promote_clarify_observation_to_execute_with_locator(
            route_result,
            locator_kind_for_bound_target(&target),
            target,
            "active_listing_target_prebound_for_locatorless_scalar_count",
        );
    }
    if !route_result.is_execute_gate() {
        return false;
    }
    route_result.output_contract.locator_kind = locator_kind_for_bound_target(&target);
    route_result.output_contract.locator_hint = target;
    super::append_route_reason(
        route_result,
        "active_listing_target_prebound_for_locatorless_scalar_count",
    );
    true
}

pub(super) fn prebind_current_workspace_root_hint_for_scalar_count(
    state: &crate::AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    let has_current_workspace_scope_marker = super::route_reason_has_marker(
        route_result,
        "current_workspace_scope_from_current_request",
    );
    if unresolved_deictic_prompt_blocks_workspace_root_prebind(prompt)
        && !has_current_workspace_scope_marker
    {
        return false;
    }
    let has_workspace_scope_authority =
        scalar_count_workspace_root_prebind_has_semantic_authority(state, prompt, route_result);
    if !has_workspace_scope_authority {
        return false;
    }
    let can_repair_locator_kind = route_result.output_contract.locator_kind
        == crate::OutputLocatorKind::None
        && has_current_workspace_scope_marker;
    let can_promote_clarify = route_result.needs_clarify
        && route_result.is_clarify_gate()
        && has_workspace_scope_authority;
    if (!route_result.is_execute_gate() && !can_promote_clarify)
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || route_result.output_contract.semantic_kind != crate::OutputSemanticKind::ScalarCount
        || !scalar_count_route_allows_workspace_root_prebind(route_result)
        || (route_result.output_contract.locator_kind != crate::OutputLocatorKind::CurrentWorkspace
            && !can_repair_locator_kind)
        || !route_result.output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    let locator_hint = state.skill_rt.workspace_root.display().to_string();
    if can_promote_clarify {
        super::promote_clarify_observation_to_execute_with_locator(
            route_result,
            crate::OutputLocatorKind::CurrentWorkspace,
            locator_hint,
            "current_workspace_root_hint_prebound_for_scalar_count",
        )
    } else {
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route_result.output_contract.locator_hint = locator_hint;
        super::append_route_reason(
            route_result,
            "current_workspace_root_hint_prebound_for_scalar_count",
        );
        true
    }
}

fn unresolved_deictic_prompt_blocks_workspace_root_prebind(prompt: &str) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    surface.has_deictic_reference() && !super::current_request_has_concrete_locator_surface(prompt)
}

fn scalar_count_workspace_root_prebind_has_semantic_authority(
    state: &crate::AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    super::route_reason_has_marker(route_result, "current_workspace_scope_from_current_request")
        || super::text_contains_workspace_root_locator(prompt, &state.skill_rt.workspace_root)
        || super::text_contains_workspace_root_locator(
            &route_result.resolved_intent,
            &state.skill_rt.workspace_root,
        )
}

fn scalar_count_route_allows_workspace_root_prebind(route_result: &crate::RouteResult) -> bool {
    matches!(
        route_result.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::OneSentence
    ) || (route_result.output_contract.response_shape == crate::OutputResponseShape::Strict
        && route_result.output_contract.exact_sentence_count == Some(1))
}

pub(super) const SESSION_ALIAS_LOCATOR_PREBOUND_FROM_CURRENT_REQUEST: &str =
    "session_alias_locator_prebound_from_current_request";

pub(super) fn prebind_session_alias_locator_from_current_request(
    prompt: &str,
    route_result: &mut crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    let should_bind_locator = route_result.needs_clarify
        || route_result.is_execute_gate()
        || route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        );
    if !should_bind_locator {
        return false;
    }
    let Some(conversation_state) = session_snapshot.conversation_state.as_ref() else {
        return false;
    };
    let Some(binding) = crate::conversation_state::single_alias_binding_mentioned_in_prompt(
        &conversation_state.alias_bindings,
        prompt,
    ) else {
        return false;
    };
    let target = binding.target.trim();
    if target.is_empty() {
        return false;
    }
    if super::semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind)
        && !route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && !route_result.wants_file_delivery
        && !route_result.needs_clarify
        && !matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
    {
        return false;
    }
    let locator_kind = if target.starts_with("http://") || target.starts_with("https://") {
        crate::OutputLocatorKind::Url
    } else {
        crate::OutputLocatorKind::Path
    };
    if route_result.needs_clarify || route_result.is_chat_gate() {
        return super::promote_clarify_observation_to_execute_with_locator(
            route_result,
            locator_kind,
            target.to_string(),
            SESSION_ALIAS_LOCATOR_PREBOUND_FROM_CURRENT_REQUEST,
        );
    }
    route_result.output_contract.locator_kind = locator_kind;
    route_result.output_contract.locator_hint = target.to_string();
    super::append_route_reason(
        route_result,
        SESSION_ALIAS_LOCATOR_PREBOUND_FROM_CURRENT_REQUEST,
    );
    true
}

#[cfg(test)]
#[path = "ask_pipeline_active_binding_tests.rs"]
mod tests;
