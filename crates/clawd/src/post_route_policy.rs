use crate::{
    ActFinalizeStyle, IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind,
    OutputResponseShape, OutputSemanticKind, RouteResult,
};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ClarifyReasonKind {
    #[default]
    RouteReasonText,
    MissingPathScopedLocator,
    FuzzyLocatorCandidates,
}

#[derive(Debug, Clone)]
pub(crate) enum LocatorResolution {
    None,
    Direct(String),
    Fuzzy(Vec<String>),
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct PostRoutePolicyResult {
    pub(crate) execution_route_result: RouteResult,
    pub(crate) auto_locator_path: Option<String>,
    pub(crate) auto_locator_hint: Option<String>,
    pub(crate) auto_locator_resolved_direct: bool,
    pub(crate) fuzzy_locator_suggestions: Vec<String>,
    pub(crate) missing_locator_for_path_scoped_content: bool,
    pub(crate) clarify_reason: String,
    pub(crate) clarify_reason_kind: ClarifyReasonKind,
}

pub(crate) fn content_evidence_execution_finalize_style(
    contract: &IntentOutputContract,
    needs_clarify: bool,
) -> Option<ActFinalizeStyle> {
    if needs_clarify || !contract.requires_content_evidence {
        return None;
    }
    if matches!(contract.locator_kind, OutputLocatorKind::None)
        && !contract.delivery_required
        && !matches!(
            contract.response_shape,
            OutputResponseShape::Scalar | OutputResponseShape::FileToken
        )
    {
        return None;
    }
    if let Some(style) = contract_matrix_finalize_style(contract) {
        return Some(style);
    }
    if matches!(
        contract.response_shape,
        OutputResponseShape::Scalar | OutputResponseShape::FileToken
    ) {
        Some(ActFinalizeStyle::Plain)
    } else {
        Some(ActFinalizeStyle::ChatWrapped)
    }
}

fn contract_matrix_finalize_style(contract: &IntentOutputContract) -> Option<ActFinalizeStyle> {
    let shape = crate::contract_matrix::final_answer_shape_for_output_contract(contract)?;
    match shape.class() {
        crate::contract_matrix::FinalAnswerShapeClass::DeliveryArtifact
        | crate::contract_matrix::FinalAnswerShapeClass::ScalarValue
        | crate::contract_matrix::FinalAnswerShapeClass::SinglePath
        | crate::contract_matrix::FinalAnswerShapeClass::StrictList
        | crate::contract_matrix::FinalAnswerShapeClass::Table => Some(ActFinalizeStyle::Plain),
        crate::contract_matrix::FinalAnswerShapeClass::Freeform
        | crate::contract_matrix::FinalAnswerShapeClass::GroundedSummary
        | crate::contract_matrix::FinalAnswerShapeClass::Verdict => {
            Some(ActFinalizeStyle::ChatWrapped)
        }
    }
}

fn locator_kind_is_current_workspace(kind: OutputLocatorKind) -> bool {
    matches!(kind, OutputLocatorKind::CurrentWorkspace)
}

fn locator_kind_requires_path_binding(kind: OutputLocatorKind) -> bool {
    matches!(
        kind,
        OutputLocatorKind::Path | OutputLocatorKind::CurrentWorkspace | OutputLocatorKind::Filename
    )
}

fn semantic_locator_hint_satisfies_non_path_binding(route_result: &RouteResult) -> bool {
    route_result.output_contract.semantic_kind == OutputSemanticKind::ServiceStatus
        && !route_result.output_contract.locator_hint.trim().is_empty()
}

fn file_delivery_can_materialize_target_without_existing_locator(
    route_result: &RouteResult,
) -> bool {
    // New-file delivery may choose a filename during planning; an empty locator
    // hint is not necessarily a missing existing-file target.
    route_result.is_execute_gate()
        && !route_result.needs_clarify
        && route_result.wants_file_delivery
        && route_result.output_contract.delivery_required
        && route_result.output_contract.response_shape == OutputResponseShape::FileToken
        && route_result.output_contract.delivery_intent == OutputDeliveryIntent::FileSingle
        && route_result.output_contract.requires_content_evidence
        && route_result.output_contract.semantic_kind == OutputSemanticKind::GeneratedFileDelivery
        && matches!(
            route_result.output_contract.locator_kind,
            OutputLocatorKind::Path | OutputLocatorKind::Filename
        )
        && route_result.output_contract.locator_hint.trim().is_empty()
}

fn scalar_path_output_can_be_observed_without_input_locator(route_result: &RouteResult) -> bool {
    route_result.is_execute_gate()
        && !route_result.needs_clarify
        && route_result.output_contract.response_shape == OutputResponseShape::Scalar
        && route_result.output_contract.semantic_kind == OutputSemanticKind::ScalarPathOnly
        && !route_result.output_contract.delivery_required
        && route_result.output_contract.locator_kind == OutputLocatorKind::Path
        && route_result.output_contract.locator_hint.trim().is_empty()
}

fn path_is_existing_directory(path: &str) -> bool {
    let trimmed = path.trim();
    !trimmed.is_empty() && Path::new(trimmed).is_dir()
}

fn should_force_content_evidence_for_path_bound_chat_wrapped_execution(
    route_result: &RouteResult,
    direct_locator_path: Option<&str>,
) -> bool {
    if route_result.output_contract.semantic_kind != OutputSemanticKind::None
        || route_result.output_contract.delivery_required
        || !route_result.ask_mode.finalize_chat_wrapped()
        || !matches!(
            route_result.output_contract.response_shape,
            OutputResponseShape::Free | OutputResponseShape::OneSentence
        )
    {
        return false;
    }

    match route_result.output_contract.locator_kind {
        OutputLocatorKind::Path | OutputLocatorKind::CurrentWorkspace => {
            direct_locator_path.is_some_and(path_is_existing_directory)
        }
        _ => false,
    }
}

fn should_clear_scalar_count_for_non_scalar_contract(route_result: &RouteResult) -> bool {
    route_result.output_contract.semantic_kind == OutputSemanticKind::ScalarCount
        && route_result.output_contract.response_shape != OutputResponseShape::Scalar
}

fn should_clear_scalar_path_only_without_locator_binding(route_result: &RouteResult) -> bool {
    if route_result.output_contract.semantic_kind != OutputSemanticKind::ScalarPathOnly
        || route_result.output_contract.response_shape != OutputResponseShape::Scalar
        || route_result.output_contract.delivery_required
    {
        return false;
    }
    route_result.output_contract.locator_kind == OutputLocatorKind::None
        && route_result.output_contract.locator_hint.trim().is_empty()
}

fn current_workspace_content_summary_requires_concrete_locator(route_result: &RouteResult) -> bool {
    route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && route_result.output_contract.locator_kind == OutputLocatorKind::CurrentWorkspace
        && matches!(
            route_result.output_contract.semantic_kind,
            OutputSemanticKind::ContentExcerptSummary
                | OutputSemanticKind::ContentExcerptWithSummary
                | OutputSemanticKind::ExcerptKindJudgment
        )
}

fn route_reason_has_marker(route_result: &RouteResult, marker: &str) -> bool {
    route_result
        .route_reason
        .split(';')
        .any(|part| part.trim() == marker)
}

fn direct_auto_locator_can_rescue_background_content_clarify(route_result: &RouteResult) -> bool {
    route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && route_result
            .output_contract
            .semantic_kind
            .is_content_excerpt_summary()
        && !matches!(
            route_result.output_contract.response_shape,
            OutputResponseShape::Scalar | OutputResponseShape::FileToken
        )
        && matches!(
            route_result.output_contract.locator_kind,
            OutputLocatorKind::Path | OutputLocatorKind::Filename
        )
}

pub(crate) fn apply_post_route_policy(
    route_result: RouteResult,
    locator_resolution: LocatorResolution,
) -> PostRoutePolicyResult {
    let mut execution_route_result = route_result.clone();
    let path_scoped_content_request = route_result.output_contract.requires_content_evidence
        && locator_kind_requires_path_binding(route_result.output_contract.locator_kind)
        && !semantic_locator_hint_satisfies_non_path_binding(&route_result);
    let mut auto_locator_path = None;
    let mut auto_locator_hint = None;
    let mut auto_locator_resolved_direct = false;
    let mut fuzzy_locator_suggestions = Vec::new();
    let normalizer_locator_hint_present =
        !route_result.output_contract.locator_hint.trim().is_empty();
    let file_delivery_can_materialize_target =
        file_delivery_can_materialize_target_without_existing_locator(&route_result);
    let scalar_path_output_can_be_observed =
        scalar_path_output_can_be_observed_without_input_locator(&route_result);
    let current_workspace_content_summary_needs_concrete_locator =
        current_workspace_content_summary_requires_concrete_locator(&route_result);
    let mut missing_locator_for_path_scoped_content = path_scoped_content_request
        && !locator_kind_is_current_workspace(route_result.output_contract.locator_kind)
        && !normalizer_locator_hint_present
        && !file_delivery_can_materialize_target
        && !scalar_path_output_can_be_observed;

    match locator_resolution {
        LocatorResolution::Direct(path) => {
            let locator_notice = if locator_kind_is_current_workspace(
                execution_route_result.output_contract.locator_kind,
            ) {
                format!(
                    "\n\n[AUTO_LOCATOR]\nResolved present workspace scope to: {path}\nUse this path as the target unless user explicitly overrides it.\n"
                )
            } else {
                format!(
                    "\n\n[AUTO_LOCATOR]\nResolved concrete path from default locator directory: {path}\nUse this path as the target unless user explicitly overrides it.\n"
                )
            };
            auto_locator_hint = Some(locator_notice);
            auto_locator_path = Some(path);
            auto_locator_resolved_direct = true;
            if missing_locator_for_path_scoped_content {
                missing_locator_for_path_scoped_content = false;
            }
        }
        LocatorResolution::Fuzzy(candidates) => {
            fuzzy_locator_suggestions = candidates;
        }
        LocatorResolution::None => {}
    }
    if !fuzzy_locator_suggestions.is_empty() {
        missing_locator_for_path_scoped_content = false;
    }
    if current_workspace_content_summary_needs_concrete_locator
        && !auto_locator_resolved_direct
        && fuzzy_locator_suggestions.is_empty()
    {
        missing_locator_for_path_scoped_content = true;
    }

    if should_clear_scalar_count_for_non_scalar_contract(&execution_route_result) {
        execution_route_result.output_contract.semantic_kind = OutputSemanticKind::None;
    }
    if should_clear_scalar_path_only_without_locator_binding(&execution_route_result) {
        execution_route_result.output_contract.semantic_kind = OutputSemanticKind::None;
    }

    if should_force_content_evidence_for_path_bound_chat_wrapped_execution(
        &execution_route_result,
        auto_locator_path.as_deref(),
    ) {
        execution_route_result
            .output_contract
            .requires_content_evidence = true;
    }

    let background_locator_clarify = route_reason_has_marker(
        &execution_route_result,
        "background_locator_requires_clarify",
    );
    if auto_locator_resolved_direct
        && path_scoped_content_request
        && (!background_locator_clarify
            || direct_auto_locator_can_rescue_background_content_clarify(&execution_route_result))
    {
        execution_route_result.needs_clarify = false;
        if execution_route_result.is_clarify_gate() || execution_route_result.is_chat_gate() {
            let finalize = if matches!(
                execution_route_result.output_contract.response_shape,
                OutputResponseShape::Scalar | OutputResponseShape::FileToken
            ) {
                ActFinalizeStyle::Plain
            } else {
                ActFinalizeStyle::ChatWrapped
            };
            execution_route_result.set_planner_execute_finalize(finalize);
        }
    }

    let fuzzy_locator_requires_clarify = !fuzzy_locator_suggestions.is_empty()
        && (matches!(
            execution_route_result.output_contract.locator_kind,
            OutputLocatorKind::Path | OutputLocatorKind::Filename
        ) || current_workspace_content_summary_needs_concrete_locator);
    let force_clarify = execution_route_result.is_clarify_gate()
        || execution_route_result.needs_clarify
        || missing_locator_for_path_scoped_content
        || fuzzy_locator_requires_clarify;
    if force_clarify {
        execution_route_result.needs_clarify = true;
        execution_route_result.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
    }

    let (clarify_reason, clarify_reason_kind) = if missing_locator_for_path_scoped_content {
        if execution_route_result.route_reason.trim().is_empty() {
            (
                "locator_required_for_path_scoped_content".to_string(),
                ClarifyReasonKind::MissingPathScopedLocator,
            )
        } else {
            (
                format!(
                    "{}; locator_required_for_path_scoped_content",
                    execution_route_result.route_reason
                ),
                ClarifyReasonKind::MissingPathScopedLocator,
            )
        }
    } else if !fuzzy_locator_suggestions.is_empty() {
        let reason = if execution_route_result.route_reason.trim().is_empty() {
            "fuzzy_locator_candidates".to_string()
        } else {
            execution_route_result.route_reason.clone()
        };
        (reason, ClarifyReasonKind::FuzzyLocatorCandidates)
    } else {
        (
            execution_route_result.route_reason.clone(),
            ClarifyReasonKind::RouteReasonText,
        )
    };

    PostRoutePolicyResult {
        execution_route_result,
        auto_locator_path,
        auto_locator_hint,
        auto_locator_resolved_direct,
        fuzzy_locator_suggestions,
        missing_locator_for_path_scoped_content,
        clarify_reason,
        clarify_reason_kind,
    }
}

#[cfg(test)]
#[path = "post_route_policy_tests.rs"]
mod tests;
