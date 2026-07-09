use std::path::Path;

use crate::pipeline_types::OutputContractRef;
use crate::AppState;

use super::{
    active_clarify_locator_task_prompt, active_observable_task_prompt,
    compare_path_targets_current_anchor, first_compare_path_from_text, locator_hint_compare_path,
    locator_hint_points_to_workspace_root, route_has_structured_execution_signal,
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, ScheduleKind, TargetTaskPolicy, TurnType,
};

const CURRENT_ANCHOR_CONTEXT_ONLY_MARKERS: &[&str] = &["tool_discovery"];
const CURRENT_ANCHOR_COMMAND_OBSERVATION_MARKERS: &[&str] =
    &["raw_command_output", "execution_failed_step"];
const CURRENT_ANCHOR_CONFIG_MARKERS: &[&str] = &[
    "config_risk_assessment",
    "config_validation",
    "config_mutation",
];

fn route_reason_has_machine_marker(route_reason: &str, marker: &str) -> bool {
    crate::RouteReasonMarkers::new(route_reason).has_machine_marker(marker)
}

fn route_reason_has_any_machine_marker(route_reason: &str, markers: &[&str]) -> bool {
    crate::RouteReasonMarkers::new(route_reason).has_any_machine_marker(markers)
}

fn output_contract_has_file_delivery_signal(
    route_reason: &str,
    output_contract: &IntentOutputContract,
) -> bool {
    route_reason_has_machine_marker(route_reason, "generated_file_delivery")
        || output_contract.delivery_required
        || matches!(
            output_contract.response_shape,
            OutputResponseShape::FileToken
        )
        || matches!(
            output_contract.delivery_intent,
            OutputDeliveryIntent::FileSingle
        )
}

pub(super) fn bare_path_only_input_can_fill_active_observable_task(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    output_contract: &IntentOutputContract,
) -> bool {
    let active_delivery_frame = session_snapshot
        .and_then(|snapshot| snapshot.active_followup_frame.as_ref())
        .is_some_and(|frame| {
            matches!(
                frame.op_kind,
                crate::followup_frame::FollowupOpKind::Delivery
            )
        });
    let file_delivery_contract = output_contract.delivery_required
        || matches!(
            output_contract.response_shape,
            OutputResponseShape::FileToken
        )
        || matches!(
            output_contract.delivery_intent,
            OutputDeliveryIntent::FileSingle
        );
    if active_delivery_frame && file_delivery_contract {
        return true;
    }

    let active_followup_policy = matches!(
        turn_type,
        Some(TurnType::TaskAppend | TurnType::TaskCorrect | TurnType::TaskReplace)
    ) && matches!(
        target_task_policy,
        Some(TargetTaskPolicy::ReuseActive | TargetTaskPolicy::ReplaceActive)
    );
    let executable_observation_contract = output_contract.requires_content_evidence
        && (matches!(
            output_contract.response_shape,
            OutputResponseShape::Scalar
                | OutputResponseShape::Strict
                | OutputResponseShape::FileToken
        ) || matches!(
            output_contract.locator_kind,
            OutputLocatorKind::Path
                | OutputLocatorKind::Filename
                | OutputLocatorKind::Url
                | OutputLocatorKind::CurrentWorkspace
        ) || !output_contract.locator_hint.trim().is_empty());
    let active_replacement_locator_policy = matches!(turn_type, Some(TurnType::TaskRequest))
        && matches!(target_task_policy, Some(TargetTaskPolicy::ReplaceActive))
        && executable_observation_contract;
    let active_clarify_locator_policy = active_clarify_locator_task_prompt(session_snapshot)
        .is_some()
        && executable_observation_contract;
    let active_implicit_locator_policy =
        turn_type.is_none() && target_task_policy.is_none() && executable_observation_contract;

    if active_observable_task_prompt(session_snapshot).is_none()
        || !executable_observation_contract
        || !(active_followup_policy
            || active_replacement_locator_policy
            || active_clarify_locator_policy
            || active_implicit_locator_policy)
    {
        return false;
    }

    output_contract.requires_content_evidence
        || matches!(
            output_contract.response_shape,
            OutputResponseShape::Scalar
                | OutputResponseShape::Strict
                | OutputResponseShape::FileToken
        )
}

pub(super) fn sanitize_resolved_intent_for_current_turn_locator(
    resolved_user_intent: &str,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> Option<String> {
    if !req_surface.has_concrete_locator_hint()
        || req_surface.has_explicit_path_or_url()
        || crate::worker::has_explicit_path_or_url_locator_hint(req)
    {
        return None;
    }
    let req_lower = req.to_ascii_lowercase();
    let resolved_introduced_path =
        crate::worker::has_explicit_path_or_url_locator_hint(resolved_user_intent)
            || crate::delivery_utils::extract_filename_candidates(resolved_user_intent)
                .into_iter()
                .any(|candidate| !req_lower.contains(&candidate.to_ascii_lowercase()));
    if !resolved_introduced_path {
        return None;
    }
    let trimmed_req = req.trim();
    if trimmed_req.is_empty() {
        return None;
    }
    Some(trimmed_req.to_string())
}

fn normalizer_target_drifted_from_current_anchor(
    output_contract: &IntentOutputContract,
    resolved_user_intent: &str,
    current_anchor_path: &str,
    workspace_root: &Path,
) -> bool {
    let Some(current_anchor) = locator_hint_compare_path(current_anchor_path, workspace_root)
    else {
        return false;
    };

    let mut saw_model_target = false;
    if let Some(contract_target) =
        locator_hint_compare_path(&output_contract.locator_hint, workspace_root)
    {
        saw_model_target = true;
        if compare_path_targets_current_anchor(&contract_target, &current_anchor) {
            return false;
        }
    }
    if let Some(resolved_target) =
        first_compare_path_from_text(resolved_user_intent, workspace_root)
    {
        saw_model_target = true;
        if compare_path_targets_current_anchor(&resolved_target, &current_anchor) {
            return false;
        }
    }

    saw_model_target
}

pub(super) fn apply_current_turn_anchor_drift_repair(
    output_contract: &mut IntentOutputContract,
    route_reason: &str,
    resolved_user_intent: &str,
    current_anchor_path: &str,
    workspace_root: &Path,
) -> Option<&'static str> {
    if route_reason_has_any_machine_marker(route_reason, CURRENT_ANCHOR_CONTEXT_ONLY_MARKERS) {
        return None;
    }
    if route_reason_has_machine_marker(route_reason, "generated_file_delivery") {
        return None;
    }
    if matches!(
        output_contract.semantic_kind,
        OutputSemanticKind::ArchivePack | OutputSemanticKind::ArchiveUnpack
    ) && output_contract.locator_hint.contains('|')
    {
        return None;
    }
    if !normalizer_target_drifted_from_current_anchor(
        output_contract,
        resolved_user_intent,
        current_anchor_path,
        workspace_root,
    ) {
        return None;
    }

    let preserve_file_delivery =
        output_contract_has_file_delivery_signal(route_reason, output_contract);
    let preserve_command_observation = route_reason_has_any_machine_marker(
        route_reason,
        CURRENT_ANCHOR_COMMAND_OBSERVATION_MARKERS,
    );
    let preserve_quantity_comparison =
        route_reason_has_machine_marker(route_reason, "quantity_comparison");

    output_contract.response_shape = if preserve_file_delivery {
        OutputResponseShape::FileToken
    } else if preserve_command_observation {
        output_contract.response_shape
    } else if preserve_quantity_comparison {
        OutputResponseShape::Strict
    } else {
        OutputResponseShape::Free
    };
    output_contract.exact_sentence_count = None;
    output_contract.requires_content_evidence = !preserve_file_delivery;
    output_contract.delivery_required = preserve_file_delivery;
    output_contract.locator_kind = if preserve_command_observation {
        OutputLocatorKind::None
    } else if preserve_quantity_comparison {
        OutputLocatorKind::CurrentWorkspace
    } else {
        OutputLocatorKind::Path
    };
    output_contract.delivery_intent = if preserve_file_delivery {
        OutputDeliveryIntent::FileSingle
    } else {
        OutputDeliveryIntent::None
    };
    let output_contract_ref = if preserve_command_observation {
        if route_reason_has_machine_marker(route_reason, "execution_failed_step") {
            OutputSemanticKind::ExecutionFailedStep
        } else {
            OutputSemanticKind::RawCommandOutput
        }
    } else if preserve_quantity_comparison {
        OutputSemanticKind::QuantityComparison
    } else {
        OutputSemanticKind::None
    };
    output_contract.apply_output_contract_ref(OutputContractRef::new(output_contract_ref));
    output_contract.locator_hint = if preserve_command_observation {
        String::new()
    } else if preserve_quantity_comparison {
        workspace_root.display().to_string()
    } else {
        current_anchor_path.trim().to_string()
    };
    output_contract.self_extension = Default::default();
    Some("current_turn_anchor_overrides_contextual_target")
}

pub(super) fn resolve_current_turn_anchor_path(state: &AppState, req: &str) -> Option<String> {
    match crate::worker::try_resolve_implicit_locator_path(
        state,
        req,
        "",
        OutputLocatorKind::Path,
        None,
    ) {
        Some(crate::worker::LocatorAutoResolution::Direct(path)) => Some(path),
        Some(crate::worker::LocatorAutoResolution::Fuzzy(_)) | None => None,
    }
}

pub(super) fn current_request_mentions_session_alias(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    req: &str,
) -> bool {
    session_snapshot
        .and_then(|snapshot| snapshot.conversation_state.as_ref())
        .is_some_and(|conversation_state| {
            crate::conversation_state::single_alias_binding_mentioned_in_prompt(
                &conversation_state.alias_bindings,
                req,
            )
            .is_some()
        })
}

pub(super) fn current_turn_anchor_drift_repair_allowed(
    needs_clarify: bool,
    route_reason: &str,
    output_contract: &IntentOutputContract,
    wants_file_delivery: bool,
    schedule_kind: ScheduleKind,
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    workspace_root: &Path,
) -> bool {
    if needs_clarify {
        return false;
    }
    if wants_file_delivery
        || output_contract_has_file_delivery_signal(route_reason, output_contract)
    {
        return false;
    }
    if route_reason_has_any_machine_marker(route_reason, CURRENT_ANCHOR_CONTEXT_ONLY_MARKERS) {
        return false;
    }
    if route_reason_has_any_machine_marker(route_reason, CURRENT_ANCHOR_CONFIG_MARKERS)
        && !output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    if output_contract.locator_kind == OutputLocatorKind::CurrentWorkspace {
        let hint = output_contract.locator_hint.trim();
        if hint.is_empty() || locator_hint_points_to_workspace_root(hint, workspace_root) {
            return false;
        }
    }
    route_has_structured_execution_signal(
        output_contract,
        wants_file_delivery,
        schedule_kind,
        execution_recipe_hint,
    )
}
