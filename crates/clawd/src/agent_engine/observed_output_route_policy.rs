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
    route.output_contract.semantic_kind == crate::OutputSemanticKind::ExistenceWithPath
        && crate::contract_matrix::final_answer_shape_for_output_contract(&route.output_contract)
            .is_some_and(|shape| shape.allows_model_language())
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
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison
        && route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && (route.output_contract.response_shape == crate::OutputResponseShape::Free
            || route_reason_has_marker(route, QUANTITY_COMPARISON_MODEL_LANGUAGE_SYNTHESIS_MARKER))
}

pub(super) fn observed_response_style_hint(agent_run_context: Option<&AgentRunContext>) -> String {
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
    if semantic_kind == Some(crate::OutputSemanticKind::DirectoryPurposeSummary) {
        return "For a listing-grounded directory purpose summary, use observed entry names, paths, counts, metadata, and file extensions as sufficient evidence for a cautious directory-level purpose or role. Include the requested selected entries plus the purpose/role summary; do not refuse only because file contents were not read unless the user asked for exact per-file contents or concrete setup steps.".to_string();
    }
    if semantic_kind == Some(crate::OutputSemanticKind::ExcerptKindJudgment) {
        return "For an excerpt-kind judgment, base the category judgment on the observed excerpt. If the same current task also observed listing names or candidates, include those observed names/candidates and the excerpt-based judgment in the final answer; for one_sentence shape, combine both deliverables into one compact sentence.".to_string();
    }
    if let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) {
        if route_disallows_direct_observation_passthrough(route) {
            if route_quantity_comparison_requires_model_language_synthesis(route) {
                return "Use the observed comparison values as evidence, and include the requested concise model-language synthesis. Do not answer with only the raw comparison verdict; that would be incomplete for this contract.".to_string();
            }
            if let Some(count) = route.output_contract.exact_sentence_count {
                let sentence_label = if count == 1 { "sentence" } else { "sentences" };
                return format!(
                    "Use the observed output as evidence to produce exactly {count} {sentence_label}. Do not answer by copying only the raw observed output; that would be an incomplete passthrough for this contract."
                );
            }
            if route.output_contract.response_shape == crate::OutputResponseShape::OneSentence {
                return "Use the observed output as evidence to produce exactly one sentence. If the request has multiple deliverables, include all of them in that one sentence. Do not answer by copying only the raw observed output; that would be an incomplete passthrough for this contract.".to_string();
            }
            return "Use the observed output as evidence to produce the requested final wording. Do not answer by copying only the raw observed output; that would be an incomplete passthrough for this contract.".to_string();
        }
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
    if semantic_kind == Some(crate::OutputSemanticKind::ExistenceWithPath) {
        return "Return a concise existence verdict and include the target path or observed path. This path requirement overrides response_shape=scalar unless the original user explicitly requested one bare boolean/scalar. Do not reduce the answer to only yes/no/exists/missing.".to_string();
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
            "Return exactly one sentence unless the current user request explicitly asks for another exact sentence count. If the request has multiple deliverables, include all of them in that one sentence."
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
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::None
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
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::CommandOutputSummary {
        return true;
    }
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::ExecutionFailedStep {
        return true;
    }
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
        && route.output_contract.response_shape == crate::OutputResponseShape::Strict
        && route.output_contract.locator_kind == crate::OutputLocatorKind::None
        && crate::contract_matrix::final_answer_shape_for_output_contract(&route.output_contract)
            .is_some_and(|shape| shape.allows_model_language())
    {
        return true;
    }
    if !matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
            | crate::OutputSemanticKind::ContentExcerptWithSummary
            | crate::OutputSemanticKind::ExcerptKindJudgment
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
    route.output_contract.semantic_kind == crate::OutputSemanticKind::GitRepositoryState
        && route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && (matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
        ) || route.output_contract.exact_sentence_count.is_some())
}
