fn structured_fuzzy_locator_clarify_context(
    fuzzy_locator_suggestions: &[String],
) -> Option<String> {
    if fuzzy_locator_suggestions.is_empty() {
        return None;
    }
    let candidate_block = fuzzy_locator_suggestions
        .iter()
        .enumerate()
        .map(|(idx, value)| format!("candidate_{}: {}", idx + 1, value))
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!(
        "clarify_case: fuzzy_locator_candidates\nexact_target_found: false\ncandidate_count: {}\n{}",
        fuzzy_locator_suggestions.len(),
        candidate_block
    ))
}

pub(super) fn build_locator_fuzzy_clarify_context(
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

pub(super) fn should_suppress_recent_execution_in_clarify_context(
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
        && route_result.output_contract.locator_hint.trim().is_empty()
}

pub(super) fn should_reuse_route_clarify_question(
    route_result: &crate::RouteResult,
    _clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind,
    fuzzy_locator_suggestions: &[String],
) -> bool {
    fuzzy_locator_suggestions.is_empty() && !route_result.clarify_question.trim().is_empty()
}

pub(super) fn structured_missing_locator_clarify_context(
    route_result: &crate::RouteResult,
    fuzzy_locator_suggestions: &[String],
) -> Option<String> {
    if !route_result.needs_clarify {
        return None;
    }
    if let Some(context) = structured_fuzzy_locator_clarify_context(fuzzy_locator_suggestions) {
        return Some(context);
    }
    let route_reason_clarify_case = route_clarify_reason_code(route_result);
    if route_reason_clarify_case.is_none()
        && !route_result.output_contract.locator_hint.trim().is_empty()
    {
        return None;
    }
    let clarify_case = route_reason_clarify_case.or_else(|| {
        if matches!(
            route_result.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::DirectoryLookup
        ) {
            Some("missing_directory_locator")
        } else if matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::ScalarCount
        ) {
            Some("missing_count_target")
        } else if matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::ServiceStatus
        ) {
            Some("missing_service_target")
        } else if route_result.output_contract.delivery_required {
            Some("missing_file_locator")
        } else if route_result.output_contract.requires_content_evidence
            && matches!(
                route_result.output_contract.response_shape,
                crate::OutputResponseShape::Scalar
                    | crate::OutputResponseShape::Free
                    | crate::OutputResponseShape::OneSentence
            )
        {
            Some("missing_read_target")
        } else {
            None
        }
    })?;
    let mut lines = vec![
        format!("clarify_case: {clarify_case}"),
        format!(
            "resolved_user_intent: {}",
            crate::truncate_for_agent_trace(route_result.resolved_intent.trim())
        ),
        format!(
            "locator_kind: {}",
            route_result.output_contract.locator_kind.as_str()
        ),
        format!(
            "response_shape: {}",
            route_result.output_contract.response_shape.as_str()
        ),
        format!(
            "semantic_kind: {}",
            route_result.output_contract.semantic_kind.as_str()
        ),
        format!(
            "delivery_required: {}",
            route_result.output_contract.delivery_required
        ),
        format!(
            "requires_content_evidence: {}",
            route_result.output_contract.requires_content_evidence
        ),
    ];
    let route_question = route_result.clarify_question.trim();
    if !route_question.is_empty() {
        lines.push(format!(
            "normalizer_clarify_question_candidate: {}",
            crate::truncate_for_agent_trace(route_question)
        ));
    }
    Some(lines.join("\n"))
}

pub(super) fn route_clarify_reason_code(route_result: &crate::RouteResult) -> Option<&'static str> {
    if super::route_reason_has_marker(route_result, "clarify_reason_code:missing_count_target") {
        Some("missing_count_target")
    } else if super::route_reason_has_marker(
        route_result,
        "clarify_reason_code:missing_delivery_locator",
    ) {
        Some("missing_delivery_locator")
    } else if super::route_reason_has_marker(
        route_result,
        "clarify_reason_code:missing_service_target",
    ) {
        Some("missing_service_target")
    } else if super::route_reason_has_marker(
        route_result,
        "clarify_reason_code:missing_search_locator",
    ) {
        Some("missing_search_locator")
    } else if super::route_reason_has_marker(
        route_result,
        "clarify_reason_code:missing_read_target",
    ) {
        Some("missing_read_target")
    } else if super::route_reason_has_marker(
        route_result,
        "unresolved_file_delivery_requires_clarify",
    ) {
        Some("missing_file_locator")
    } else if super::route_reason_has_marker(route_result, "clarify_reason_code:missing_target") {
        Some("missing_target")
    } else {
        None
    }
}

#[cfg(test)]
#[path = "ask_pipeline_clarify_context_tests.rs"]
mod tests;
