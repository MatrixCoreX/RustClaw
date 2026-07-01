use serde_json::Value;
use std::path::Path;

use super::{
    execution_finalize_style_for_contract, is_bare_path_only_input_for_clarify,
    machine_context_has_capability_ref, state_patch_deictic_reference_requires_clarify,
    surface_has_unbound_scope_plus_single_filename_target, ActFinalizeStyle, IntentOutputContract,
    OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape, TargetTaskPolicy, TurnType,
};

const WORKSPACE_DEFAULT_OBSERVATION_MARKERS: &[&str] = &[
    "hidden_entries_check",
    "file_names",
    "directory_names",
    "directory_entry_groups",
    "file_paths",
    "directory_purpose_summary",
    "workspace_project_summary",
    "existence_with_path",
    "existence_with_path_summary",
    "git_commit_subject",
    "git_repository_state",
];

const LOCATORLESS_DEFAULT_OBSERVATION_MARKERS: &[&str] = &["service_status", "tool_discovery"];

const EXISTING_OBSERVED_CONTEXT_MARKERS: &[&str] = &[
    "content_excerpt_summary",
    "content_presence_check",
    "excerpt_kind_judgment",
    "recent_artifacts_judgment",
    "execution_failed_step",
];

fn route_reason_has_machine_marker(route_reason: &str, marker: &str) -> bool {
    route_reason.split(';').map(str::trim).any(|part| {
        part == marker
            || part
                .rsplit_once(':')
                .is_some_and(|(_, suffix)| suffix.trim() == marker)
    })
}

fn route_reason_has_any_machine_marker(route_reason: &str, markers: &[&str]) -> bool {
    markers
        .iter()
        .any(|marker| route_reason_has_machine_marker(route_reason, marker))
}

pub(super) fn apply_deictic_missing_locator_state_patch_clarify_repair(
    output_contract: &mut IntentOutputContract,
    state_patch: Option<&Value>,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if !state_patch_deictic_reference_requires_clarify(state_patch) {
        return None;
    }
    output_contract.requires_content_evidence = true;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.locator_hint.clear();
    *needs_clarify = true;
    if clarify_question.trim().is_empty() {
        clarify_question.clear();
    }
    *execution_finalize_style = ActFinalizeStyle::Plain;
    Some("state_patch_deictic_missing_locator_clarify")
}

pub(super) fn should_preserve_existing_observed_context_synthesis_contract(
    route_reason: &str,
    output_contract: &IntentOutputContract,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
) -> bool {
    matches!(turn_type, Some(TurnType::TaskAppend))
        && matches!(target_task_policy, Some(TargetTaskPolicy::ReuseActive))
        && route_reason_has_any_machine_marker(route_reason, EXISTING_OBSERVED_CONTEXT_MARKERS)
        && !output_contract.delivery_required
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(
            output_contract.response_shape,
            OutputResponseShape::Free
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Strict
        )
        && !req_surface.has_concrete_locator_hint()
        && !req_surface.has_structured_target_refinement()
        && !req_surface.has_delivery_token_reference()
}

pub(super) fn apply_spurious_structured_observation_clarify_repair(
    route_reason: &str,
    output_contract: &mut IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
    state_patch: Option<&Value>,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if !*needs_clarify || is_bare_path_only_input_for_clarify(req, req_surface) {
        return None;
    }
    if state_patch_deictic_reference_requires_clarify(state_patch) {
        return None;
    }
    if surface_has_unbound_scope_plus_single_filename_target(
        route_reason,
        output_contract,
        req,
        req_surface,
    ) {
        return None;
    }
    let has_current_turn_locator = req_surface.has_explicit_path_or_url()
        || req_surface.has_single_filename_candidate()
        || req_surface.has_filename_candidates()
        || req_surface.locator_target_pair.is_some()
        || req_surface.has_concrete_locator_hint();
    let has_observable_answer_shape = matches!(
        output_contract.response_shape,
        OutputResponseShape::Scalar | OutputResponseShape::Strict | OutputResponseShape::FileToken
    ) || req_surface.has_structured_target_refinement()
        || req_surface.locator_target_pair.is_some();
    if surface_locator_is_insufficient_for_clarify_repair(
        route_reason,
        output_contract,
        req_surface,
        has_observable_answer_shape,
    ) {
        return None;
    }
    if !has_current_turn_locator
        || (!has_observable_answer_shape && !req_surface.has_concrete_locator_hint())
    {
        return None;
    }
    let fallback_locator = if matches!(output_contract.locator_kind, OutputLocatorKind::None) {
        crate::intent::locator_extractor::extract_explicit_locator_for_fallback(req)
    } else {
        None
    };
    if matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && fallback_locator.is_none()
        && !req_surface.has_filename_candidates()
        && req_surface.locator_target_pair.is_none()
    {
        return None;
    }

    output_contract.requires_content_evidence = true;
    if output_contract.locator_hint.trim().is_empty() && req_surface.locator_target_pair.is_some() {
        if let Some((left, right)) = req_surface.locator_target_pair.as_ref() {
            output_contract.locator_kind = OutputLocatorKind::Path;
            output_contract.locator_hint = format!("{left}, {right}");
        }
    }
    if output_contract.locator_hint.trim().is_empty() {
        if let Some(filename) = req_surface.single_filename_candidate() {
            output_contract.locator_kind = OutputLocatorKind::Filename;
            output_contract.locator_hint = filename.to_string();
        }
    }
    if let Some(locator) =
        fallback_locator.filter(|_| output_contract.locator_hint.trim().is_empty())
    {
        output_contract.locator_kind = locator.locator_kind;
        output_contract.locator_hint = locator.locator_hint;
    } else if matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && req_surface.has_filename_candidates()
    {
        output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        output_contract.locator_hint = workspace_root.display().to_string();
    }
    *needs_clarify = false;
    clarify_question.clear();
    *execution_finalize_style =
        crate::post_route_policy::content_evidence_execution_finalize_style(output_contract, false)
            .unwrap_or_else(|| execution_finalize_style_for_contract(output_contract));
    Some("structured_observation_clarify_repair")
}

fn surface_locator_is_insufficient_for_clarify_repair(
    route_reason: &str,
    output_contract: &IntentOutputContract,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    has_observable_answer_shape: bool,
) -> bool {
    if !req_surface.has_concrete_locator_hint()
        || req_surface.locator_target_pair.is_some()
        || req_surface.has_filename_candidates()
    {
        return false;
    }
    if route_reason_has_machine_marker(route_reason, "archive_pack")
        || route_reason_has_machine_marker(route_reason, "archive_unpack")
    {
        return true;
    }
    !output_contract.requires_content_evidence && !has_observable_answer_shape
}

pub(super) fn apply_workspace_default_observation_clarify_repair(
    route_reason: &str,
    output_contract: &mut IntentOutputContract,
    workspace_root: &Path,
    state_patch: Option<&Value>,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if !*needs_clarify
        || !output_contract.requires_content_evidence
        || !route_reason_has_any_machine_marker(route_reason, WORKSPACE_DEFAULT_OBSERVATION_MARKERS)
    {
        return None;
    }
    if state_patch_deictic_reference_requires_clarify(state_patch) {
        return None;
    }
    if matches!(output_contract.locator_kind, OutputLocatorKind::None) {
        output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        output_contract.locator_hint = workspace_root.display().to_string();
    }
    *needs_clarify = false;
    clarify_question.clear();
    *execution_finalize_style =
        crate::post_route_policy::content_evidence_execution_finalize_style(output_contract, false)
            .unwrap_or_else(|| execution_finalize_style_for_contract(output_contract));
    Some("workspace_default_observation_clarify_repair")
}

pub(super) fn apply_locatorless_observation_clarify_repair(
    route_reason: &str,
    output_contract: &mut IntentOutputContract,
    machine_context: &str,
    state_patch: Option<&Value>,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if !*needs_clarify
        || !output_contract.requires_content_evidence
        || output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(output_contract.locator_kind, OutputLocatorKind::None)
        || !output_contract.locator_hint.trim().is_empty()
        || (!route_reason_has_any_machine_marker(
            route_reason,
            LOCATORLESS_DEFAULT_OBSERVATION_MARKERS,
        ) && !machine_context_has_capability_ref(machine_context))
    {
        return None;
    }
    if state_patch_deictic_reference_requires_clarify(state_patch) {
        return None;
    }
    *needs_clarify = false;
    clarify_question.clear();
    *execution_finalize_style =
        crate::post_route_policy::content_evidence_execution_finalize_style(output_contract, false)
            .unwrap_or_else(|| execution_finalize_style_for_contract(output_contract));
    Some("locatorless_observation_clarify_repair")
}
