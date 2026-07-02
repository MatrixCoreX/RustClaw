use crate::{
    ActFinalizeStyle, OutputLocatorKind, OutputResponseShape, OutputSemanticKind, RouteResult,
};

mod boundary_contract;
mod boundary_delivery;
mod boundary_locator;

pub(crate) use boundary_contract::content_evidence_execution_finalize_style;

use boundary_contract::{
    should_clear_scalar_count_marker_for_non_scalar_contract,
    should_clear_scalar_path_marker_without_locator_binding,
    should_force_content_evidence_for_path_bound_chat_wrapped_execution,
};
use boundary_delivery::{
    existing_file_delivery_can_try_locator_hint,
    file_delivery_can_materialize_target_without_existing_locator,
    scalar_path_output_can_be_observed_without_input_locator,
};
use boundary_locator::{
    current_workspace_content_summary_requires_concrete_locator,
    direct_auto_locator_can_satisfy_background_clarify,
    direct_locator_path_is_unsuitable_for_contract, locator_kind_is_current_workspace,
    locator_kind_requires_path_binding, semantic_locator_hint_satisfies_non_path_binding,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ClarifyReasonKind {
    #[default]
    RouteReasonText,
    MissingPathScopedLocator,
    FuzzyLocatorCandidates,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PostRoutePolicyOutcome {
    #[default]
    NoChange,
    BoundaryClarify,
    BoundaryReady,
    RefineContract,
}

impl PostRoutePolicyOutcome {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NoChange => "no_change",
            Self::BoundaryClarify => "boundary_clarify",
            Self::BoundaryReady => "boundary_ready",
            Self::RefineContract => "refine_contract",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PostRouteGateRecord {
    pub(crate) owner_layer: &'static str,
    pub(crate) reason_code: &'static str,
    pub(crate) outcome: PostRoutePolicyOutcome,
}

impl Default for PostRouteGateRecord {
    fn default() -> Self {
        Self {
            owner_layer: "post_route_boundary",
            reason_code: "post_route_no_change",
            outcome: PostRoutePolicyOutcome::NoChange,
        }
    }
}

impl PostRouteGateRecord {
    pub(crate) fn new(reason_code: &'static str, outcome: PostRoutePolicyOutcome) -> Self {
        Self {
            reason_code,
            outcome,
            ..Self::default()
        }
    }

    pub(crate) fn with_owner(
        owner_layer: &'static str,
        reason_code: &'static str,
        outcome: PostRoutePolicyOutcome,
    ) -> Self {
        Self {
            owner_layer,
            reason_code,
            outcome,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum LocatorResolution {
    None,
    Direct(String),
    Fuzzy(Vec<String>),
}

#[derive(Debug, Clone)]
pub(crate) struct PostRoutePolicyResult {
    pub(crate) execution_route_result: RouteResult,
    pub(crate) auto_locator_path: Option<String>,
    pub(crate) auto_locator_hint: Option<String>,
    pub(crate) auto_locator_resolved_direct: bool,
    pub(crate) fuzzy_locator_suggestions: Vec<String>,
    pub(crate) missing_locator_for_path_scoped_content: bool,
    pub(crate) clarify_reason_kind: ClarifyReasonKind,
    pub(crate) gate_record: PostRouteGateRecord,
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
            if direct_locator_path_is_unsuitable_for_contract(&execution_route_result, &path) {
                missing_locator_for_path_scoped_content = true;
            } else {
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
    let existing_file_delivery_can_try_locator_hint =
        existing_file_delivery_can_try_locator_hint(&execution_route_result)
            && fuzzy_locator_suggestions.is_empty()
            && !missing_locator_for_path_scoped_content;
    if existing_file_delivery_can_try_locator_hint {
        execution_route_result.needs_clarify = false;
        execution_route_result.set_planner_execute_finalize(ActFinalizeStyle::Plain);
    }

    let cleared_scalar_count_marker =
        should_clear_scalar_count_marker_for_non_scalar_contract(&execution_route_result);
    if cleared_scalar_count_marker {
        execution_route_result.output_contract.semantic_kind = OutputSemanticKind::None;
        remove_route_reason_machine_marker(&mut execution_route_result, "scalar_count");
    }
    let cleared_scalar_path_only_marker =
        should_clear_scalar_path_marker_without_locator_binding(&execution_route_result);
    if cleared_scalar_path_only_marker {
        execution_route_result.output_contract.semantic_kind = OutputSemanticKind::None;
        remove_route_reason_machine_marker(&mut execution_route_result, "scalar_path_only");
    }

    let forced_content_evidence =
        should_force_content_evidence_for_path_bound_chat_wrapped_execution(
            &execution_route_result,
            auto_locator_path.as_deref(),
        );
    if forced_content_evidence {
        execution_route_result
            .output_contract
            .requires_content_evidence = true;
    }

    let direct_auto_locator_can_satisfy_background_clarify =
        direct_auto_locator_can_satisfy_background_clarify(
            &execution_route_result,
            auto_locator_path.as_deref(),
        );
    let direct_auto_locator_satisfies_background_clarify = auto_locator_resolved_direct
        && path_scoped_content_request
        && direct_auto_locator_can_satisfy_background_clarify;
    if direct_auto_locator_satisfies_background_clarify {
        execution_route_result.needs_clarify = false;
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

    let fuzzy_locator_requires_clarify = !fuzzy_locator_suggestions.is_empty()
        && (matches!(
            execution_route_result.output_contract.locator_kind,
            OutputLocatorKind::Path | OutputLocatorKind::Filename
        ) || current_workspace_content_summary_needs_concrete_locator);
    let force_clarify = execution_route_result.needs_clarify
        || missing_locator_for_path_scoped_content
        || fuzzy_locator_requires_clarify;
    let force_clarify = force_clarify && !existing_file_delivery_can_try_locator_hint;
    let content_evidence_has_boundary_scope = execution_route_result
        .output_contract
        .requires_content_evidence
        && (locator_kind_requires_path_binding(
            execution_route_result.output_contract.locator_kind,
        ) || execution_route_result.output_contract.delivery_required
            || !matches!(
                execution_route_result.output_contract.delivery_intent,
                crate::OutputDeliveryIntent::None
            )
            || current_workspace_content_summary_needs_concrete_locator);
    let clarify_has_boundary_contract = missing_locator_for_path_scoped_content
        || fuzzy_locator_requires_clarify
        || content_evidence_has_boundary_scope
        || locator_kind_requires_path_binding(execution_route_result.output_contract.locator_kind)
        || execution_route_result.output_contract.delivery_required
        || !matches!(
            execution_route_result.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        )
        || current_workspace_content_summary_needs_concrete_locator;
    let non_boundary_clarify_requested = force_clarify && !clarify_has_boundary_contract;
    let apply_force_clarify = force_clarify && !non_boundary_clarify_requested;
    if non_boundary_clarify_requested {
        execution_route_result.needs_clarify = false;
        if !execution_route_result.is_execute_gate()
            || execution_route_result.is_resume_discussion_mode()
        {
            execution_route_result.set_planner_execute_finalize(ActFinalizeStyle::ChatWrapped);
        }
    }
    if apply_force_clarify {
        execution_route_result.needs_clarify = true;
    }

    let clarify_reason_kind = if missing_locator_for_path_scoped_content {
        ClarifyReasonKind::MissingPathScopedLocator
    } else if !fuzzy_locator_suggestions.is_empty() {
        ClarifyReasonKind::FuzzyLocatorCandidates
    } else {
        ClarifyReasonKind::RouteReasonText
    };
    let gate_record = if missing_locator_for_path_scoped_content {
        PostRouteGateRecord::with_owner(
            "boundary_locator_gate",
            "post_route_missing_path_scoped_locator",
            PostRoutePolicyOutcome::BoundaryClarify,
        )
    } else if !fuzzy_locator_suggestions.is_empty() {
        PostRouteGateRecord::with_owner(
            "boundary_locator_gate",
            "post_route_fuzzy_locator_candidates",
            PostRoutePolicyOutcome::BoundaryClarify,
        )
    } else if direct_auto_locator_satisfies_background_clarify {
        PostRouteGateRecord::with_owner(
            "boundary_locator_gate",
            "post_route_auto_locator_satisfied_path_scoped_content",
            PostRoutePolicyOutcome::BoundaryReady,
        )
    } else if existing_file_delivery_can_try_locator_hint {
        PostRouteGateRecord::with_owner(
            "boundary_delivery_gate",
            "post_route_file_delivery_locator_hint_deferred_to_execution",
            PostRoutePolicyOutcome::BoundaryReady,
        )
    } else if non_boundary_clarify_requested {
        PostRouteGateRecord::with_owner(
            "agent_loop_boundary_defer",
            "post_route_non_boundary_clarify_deferred_to_agent_loop",
            PostRoutePolicyOutcome::NoChange,
        )
    } else if force_clarify {
        PostRouteGateRecord::with_owner(
            "boundary_clarify_gate",
            "post_route_boundary_clarify_required",
            PostRoutePolicyOutcome::BoundaryClarify,
        )
    } else if forced_content_evidence
        || cleared_scalar_count_marker
        || cleared_scalar_path_only_marker
    {
        PostRouteGateRecord::with_owner(
            "boundary_contract_gate",
            "post_route_contract_refined",
            PostRoutePolicyOutcome::RefineContract,
        )
    } else {
        PostRouteGateRecord::default()
    };

    PostRoutePolicyResult {
        execution_route_result,
        auto_locator_path,
        auto_locator_hint,
        auto_locator_resolved_direct,
        fuzzy_locator_suggestions,
        missing_locator_for_path_scoped_content,
        clarify_reason_kind,
        gate_record,
    }
}

fn remove_route_reason_machine_marker(route_result: &mut RouteResult, marker: &str) {
    route_result.route_reason = route_result
        .route_reason
        .split(';')
        .map(str::trim)
        .filter(|part| {
            !part.is_empty()
                && *part != marker
                && !part
                    .rsplit_once(':')
                    .is_some_and(|(_, suffix)| suffix.trim() == marker)
        })
        .collect::<Vec<_>>()
        .join("; ");
}

#[cfg(test)]
#[path = "post_route_policy_tests.rs"]
mod tests;
