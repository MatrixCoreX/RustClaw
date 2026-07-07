use super::*;

pub(super) fn observed_request_language_hint(user_text: &str) -> &'static str {
    crate::language_policy::request_language_hint(user_text)
}

pub(super) fn observed_language_supports_bilingual_template(language_hint: &str) -> bool {
    let hint = language_hint.trim().to_ascii_lowercase();
    hint == "config_default" || hint.starts_with("en") || hint.starts_with("zh")
}

pub(super) fn route_should_synthesize_non_bilingual_existence_with_path(
    route: Option<&crate::RouteResult>,
    allow_localized_direct_template: bool,
) -> bool {
    if allow_localized_direct_template {
        return false;
    }
    let Some(route) = route else {
        return false;
    };
    route_contract_marker_is(route, crate::OutputSemanticKind::ExistenceWithPath)
        && crate::evidence_policy::final_answer_shape_for_route(route)
            .is_some_and(|shape| shape.allows_model_language())
}

pub(super) fn route_contract_marker_is(
    route: &crate::RouteResult,
    semantic_kind: crate::OutputSemanticKind,
) -> bool {
    route.output_contract_marker_is(semantic_kind)
}

pub(super) fn route_contract_marker_is_any(
    route: &crate::RouteResult,
    semantic_kinds: &[crate::OutputSemanticKind],
) -> bool {
    route.output_contract_marker_is_any(semantic_kinds)
}

pub(super) fn route_is_unclassified_contract(route: &crate::RouteResult) -> bool {
    route.output_contract_is_unclassified()
}

pub(super) fn observed_request_prefers_english_template(
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

pub(crate) const QUANTITY_COMPARISON_MODEL_LANGUAGE_SYNTHESIS_MARKER: &str =
    "quantity_comparison_requires_model_language_synthesis";

fn route_reason_has_marker(route: &crate::RouteResult, marker: &str) -> bool {
    route.route_reason.split(';').map(str::trim).any(|part| {
        part == marker
            || part.starts_with(&format!("{marker}:"))
            || machine_marker_token_present(part, marker)
    })
}

fn machine_marker_token_present(text: &str, marker: &str) -> bool {
    let mut start = 0;
    while let Some(offset) = text[start..].find(marker) {
        let marker_start = start + offset;
        let marker_end = marker_start + marker.len();
        let before_ok = text[..marker_start]
            .chars()
            .next_back()
            .is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != '_');
        let after_ok = text[marker_end..]
            .chars()
            .next()
            .is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != '_');
        if before_ok && after_ok {
            return true;
        }
        start = marker_end;
    }
    false
}

pub(crate) fn route_quantity_comparison_requires_model_language_synthesis(
    route: &crate::RouteResult,
) -> bool {
    !route.needs_clarify
        && route_contract_marker_is(route, crate::OutputSemanticKind::QuantityComparison)
        && route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && (route.output_contract.response_shape == crate::OutputResponseShape::Free
            || route_reason_has_marker(route, QUANTITY_COMPARISON_MODEL_LANGUAGE_SYNTHESIS_MARKER))
}

pub(super) fn observed_response_style_hint(agent_run_context: Option<&AgentRunContext>) -> String {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let response_shape = route.map(|route| route.output_contract.response_shape);
    let route_has_marker =
        |semantic_kind| route.is_some_and(|route| route_contract_marker_is(route, semantic_kind));
    if route_has_marker(crate::OutputSemanticKind::RawCommandOutput)
        && response_shape == Some(crate::OutputResponseShape::Strict)
    {
        return "style_policy=exact_observed_value observed_source=command_output requested_format=preserve raw_passthrough=conditional".to_string();
    }
    if route_has_marker(crate::OutputSemanticKind::DirectoryPurposeSummary) {
        return "style_policy=directory_purpose_summary evidence=listing_metadata include=selected_entries,purpose_summary content_read_required=only_when_requested".to_string();
    }
    if route_has_marker(crate::OutputSemanticKind::ExcerptKindJudgment) {
        return "style_policy=excerpt_kind_judgment evidence=observed_excerpt include=candidates,judgment one_sentence=combine_deliverables".to_string();
    }
    if let Some(route) = route {
        if route_disallows_direct_observation_passthrough(route) {
            if route_quantity_comparison_requires_model_language_synthesis(route) {
                return "style_policy=evidence_synthesis passthrough=disallowed synthesis=quantity_comparison include=requested_model_language_synthesis".to_string();
            }
            if let Some(count) = route.output_contract.exact_sentence_count {
                return format!(
                    "style_policy=evidence_synthesis passthrough=disallowed sentence_count={count}"
                );
            }
            if route.output_contract.response_shape == crate::OutputResponseShape::OneSentence {
                return "style_policy=evidence_synthesis passthrough=disallowed response_shape=one_sentence include_all_deliverables=true".to_string();
            }
            return "style_policy=evidence_synthesis passthrough=disallowed response_shape=requested_final_wording".to_string();
        }
    }
    if let Some(count) = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .and_then(|route| route.output_contract.exact_sentence_count)
    {
        return format!("style_policy=exact_sentence_count sentence_count={count}");
    }
    if route_has_marker(crate::OutputSemanticKind::ExistenceWithPathSummary) {
        return "style_policy=existence_with_path_summary include=verdict,path,summary evidence_preference=content_when_available".to_string();
    }
    if route_has_marker(crate::OutputSemanticKind::ExistenceWithPath) {
        return "style_policy=existence_with_path include=verdict,path scalar_override=path_required".to_string();
    }
    if route_has_marker(crate::OutputSemanticKind::ScalarCount)
        && response_shape != Some(crate::OutputResponseShape::Scalar)
    {
        return "style_policy=scalar_count include=count_dimensions aggregate_only=explicit_request_only".to_string();
    }
    match response_shape {
        Some(crate::OutputResponseShape::Scalar) => "style_policy=scalar bare_value=true",
        Some(crate::OutputResponseShape::FileToken) => {
            "style_policy=file_token bare_delivery_token=true"
        }
        Some(crate::OutputResponseShape::OneSentence) => {
            "style_policy=one_sentence include_all_deliverables=true"
        }
        Some(crate::OutputResponseShape::Strict) => "style_policy=strict_user_format no_extra=true",
        Some(crate::OutputResponseShape::Free) => "style_policy=compact_direct answer_shape=short",
        None => "style_policy=shortest_grounded_direct",
    }
    .to_string()
}

pub(crate) fn route_requires_synthesized_delivery(route: &crate::RouteResult) -> bool {
    if route_allows_strict_plain_observation_passthrough(route) {
        return false;
    }
    if route_git_repository_state_requires_language_synthesis(route) {
        return true;
    }
    if route_quantity_comparison_requires_model_language_synthesis(route) {
        return true;
    }
    let constrained_sentence_delivery = route.output_contract.response_shape
        == crate::OutputResponseShape::OneSentence
        || route.output_contract.exact_sentence_count.is_some();
    route.ask_mode.finalize_chat_wrapped()
        && route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && route_is_unclassified_contract(route)
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
    if route_contract_marker_is(route, crate::OutputSemanticKind::CommandOutputSummary) {
        return true;
    }
    if route_contract_marker_is(route, crate::OutputSemanticKind::ExecutionFailedStep) {
        return true;
    }
    if route_contract_marker_is(route, crate::OutputSemanticKind::RawCommandOutput)
        && route.output_contract.response_shape == crate::OutputResponseShape::Strict
        && route.output_contract.locator_kind == crate::OutputLocatorKind::None
        && crate::evidence_policy::final_answer_shape_for_route(route)
            .is_some_and(|shape| shape.allows_model_language())
    {
        return true;
    }
    if !route_contract_marker_is_any(
        route,
        &[
            crate::OutputSemanticKind::ContentExcerptSummary,
            crate::OutputSemanticKind::ContentExcerptWithSummary,
            crate::OutputSemanticKind::ExcerptKindJudgment,
        ],
    ) {
        return false;
    }
    matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
    ) || route.output_contract.exact_sentence_count.is_some()
}

pub(super) fn route_git_repository_state_requires_language_synthesis(
    route: &crate::RouteResult,
) -> bool {
    route_contract_marker_is(route, crate::OutputSemanticKind::GitRepositoryState)
        && route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && (matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
        ) || route.output_contract.exact_sentence_count.is_some())
}
