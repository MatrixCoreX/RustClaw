use std::path::Path;

use super::{
    current_request_mentions_workspace_identity, parse_first_layer_decision_text,
    parse_output_delivery_intent, parse_output_locator_kind, parse_output_response_shape,
    parse_output_semantic_kind, FirstLayerDecision, IntentExecutionRecipeOut, IntentNormalizerOut,
    IntentOutputContractOut, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind,
};

#[cfg(test)]
pub(super) fn semantic_suspect_detail_for_normalizer_output(
    out: &IntentNormalizerOut,
    req_surface: Option<&crate::intent::surface_signals::PromptSurfaceSignals>,
    req: &str,
    workspace_root: &Path,
) -> Option<&'static str> {
    semantic_suspect_detail_for_normalizer_output_with_command_runtime(
        out,
        req_surface,
        req,
        workspace_root,
        None,
    )
}

pub(super) fn semantic_suspect_detail_for_normalizer_output_with_command_runtime(
    out: &IntentNormalizerOut,
    req_surface: Option<&crate::intent::surface_signals::PromptSurfaceSignals>,
    req: &str,
    workspace_root: &Path,
    command_runtime: Option<&crate::CommandIntentRuntime>,
) -> Option<&'static str> {
    if out.needs_clarify {
        return None;
    }
    let Some(contract) = out.output_contract.as_ref() else {
        return None;
    };
    if parse_first_layer_decision_text(&out.decision) == Some(FirstLayerDecision::PlannerExecute)
        && contract.requires_content_evidence
        && !out.wants_file_delivery
        && !contract.delivery_required
        && matches!(
            parse_output_delivery_intent(&contract.delivery_intent),
            OutputDeliveryIntent::None
        )
        && matches!(
            parse_output_semantic_kind(&contract.semantic_kind),
            OutputSemanticKind::CommandOutputSummary
        )
    {
        return Some("command_output_summary_needs_failure_contract_review");
    }
    if parse_first_layer_decision_text(&out.decision) == Some(FirstLayerDecision::PlannerExecute)
        && contract.requires_content_evidence
        && matches!(
            parse_output_semantic_kind(&contract.semantic_kind),
            OutputSemanticKind::FileNames
        )
    {
        return Some("file_names_contract_needs_semantic_shape_review");
    }
    if parse_first_layer_decision_text(&out.decision) == Some(FirstLayerDecision::PlannerExecute)
        && contract.requires_content_evidence
        && matches!(
            parse_output_semantic_kind(&contract.semantic_kind),
            OutputSemanticKind::FilePaths
        )
    {
        return Some("file_paths_contract_needs_semantic_shape_review");
    }
    if parse_first_layer_decision_text(&out.decision) == Some(FirstLayerDecision::PlannerExecute)
        && contract.requires_content_evidence
        && matches!(
            parse_output_semantic_kind(&contract.semantic_kind),
            OutputSemanticKind::DirectoryEntryGroups
        )
    {
        return Some("directory_entry_groups_contract_needs_semantic_shape_review");
    }
    if parse_first_layer_decision_text(&out.decision) == Some(FirstLayerDecision::PlannerExecute)
        && contract.requires_content_evidence
        && matches!(
            parse_output_semantic_kind(&contract.semantic_kind),
            OutputSemanticKind::ExistenceWithPathSummary
        )
    {
        return Some("existence_summary_contract_needs_semantic_shape_review");
    }
    if parse_first_layer_decision_text(&out.decision) == Some(FirstLayerDecision::PlannerExecute)
        && !out.wants_file_delivery
        && !contract.delivery_required
        && contract.requires_content_evidence
        && out.answer_candidate.trim().is_empty()
        && matches!(
            parse_output_delivery_intent(&contract.delivery_intent),
            OutputDeliveryIntent::None
        )
        && matches!(
            parse_output_semantic_kind(&contract.semantic_kind),
            OutputSemanticKind::RawCommandOutput
        )
        && matches!(
            parse_output_response_shape(&contract.response_shape),
            OutputResponseShape::Scalar
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Free
                | OutputResponseShape::Strict
        )
        && raw_command_locator_contract_has_observable_target(contract, req_surface)
        && !normalizer_execution_recipe_declares_active_profile(out.execution_recipe.as_ref())
        && command_runtime.is_none_or(|runtime| {
            crate::agent_engine::explicit_command_segment_for_policy(runtime, req).is_none()
        })
    {
        return Some("raw_command_output_locator_needs_semantic_review");
    }
    if parse_first_layer_decision_text(&out.decision) == Some(FirstLayerDecision::PlannerExecute)
        && !out.wants_file_delivery
        && !contract.delivery_required
        && matches!(
            parse_output_delivery_intent(&contract.delivery_intent),
            OutputDeliveryIntent::None
        )
        && matches!(
            parse_output_semantic_kind(&contract.semantic_kind),
            OutputSemanticKind::None
        )
        && req_surface.is_some_and(|surface| surface.locator_target_pair.is_some())
    {
        return Some("multi_path_generic_contract_needs_semantic_shape_review");
    }
    if parse_first_layer_decision_text(&out.decision) == Some(FirstLayerDecision::PlannerExecute)
        && !out.wants_file_delivery
        && !contract.delivery_required
        && contract.requires_content_evidence
        && matches!(
            parse_output_delivery_intent(&contract.delivery_intent),
            OutputDeliveryIntent::None
        )
        && matches!(
            parse_output_semantic_kind(&contract.semantic_kind),
            OutputSemanticKind::None
        )
        && matches!(
            parse_output_response_shape(&contract.response_shape),
            OutputResponseShape::Scalar
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Free
                | OutputResponseShape::Strict
        )
        && contract_has_single_path_locator_target(contract, req_surface)
    {
        return Some("single_path_generic_contract_needs_semantic_shape_review");
    }
    if parse_first_layer_decision_text(&out.decision) == Some(FirstLayerDecision::PlannerExecute)
        && !out.wants_file_delivery
        && !contract.delivery_required
        && contract.requires_content_evidence
        && matches!(
            parse_output_delivery_intent(&contract.delivery_intent),
            OutputDeliveryIntent::None
        )
        && matches!(
            parse_output_semantic_kind(&contract.semantic_kind),
            OutputSemanticKind::ScalarCount
        )
        && matches!(
            parse_output_response_shape(&contract.response_shape),
            OutputResponseShape::Scalar
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Strict
        )
        && contract_has_single_path_locator_target(contract, req_surface)
    {
        return Some("single_path_scalar_count_contract_needs_semantic_shape_review");
    }
    if parse_first_layer_decision_text(&out.decision) == Some(FirstLayerDecision::PlannerExecute)
        && !out.wants_file_delivery
        && !contract.delivery_required
        && contract.requires_content_evidence
        && out.answer_candidate.trim().is_empty()
        && matches!(
            parse_output_delivery_intent(&contract.delivery_intent),
            OutputDeliveryIntent::None
        )
        && matches!(
            parse_output_locator_kind(&contract.locator_kind),
            OutputLocatorKind::None
        )
        && contract.locator_hint.trim().is_empty()
        && matches!(
            parse_output_semantic_kind(&contract.semantic_kind),
            OutputSemanticKind::None
        )
        && matches!(
            parse_output_response_shape(&contract.response_shape),
            OutputResponseShape::Scalar
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Free
                | OutputResponseShape::Strict
        )
    {
        return Some("locatorless_generic_evidence_contract_needs_semantic_shape_review");
    }
    if parse_first_layer_decision_text(&out.decision) != Some(FirstLayerDecision::DirectAnswer) {
        return None;
    }
    if out.wants_file_delivery {
        return Some("chat_route_with_file_delivery_request");
    }
    if contract.requires_content_evidence {
        return Some("chat_route_requires_content_evidence");
    }
    if contract.delivery_required {
        return Some("chat_route_requires_delivery");
    }
    if !matches!(
        parse_output_semantic_kind(&contract.semantic_kind),
        OutputSemanticKind::None | OutputSemanticKind::FileBasename
    ) {
        return Some("chat_route_has_observable_semantic_kind");
    }
    if !matches!(
        parse_output_locator_kind(&contract.locator_kind),
        OutputLocatorKind::None
    ) && !contract.locator_hint.trim().is_empty()
    {
        return Some("chat_route_has_observable_locator");
    }
    if out.answer_candidate.trim().is_empty()
        && contract.exact_sentence_count.is_none()
        && matches!(
            parse_output_response_shape(&contract.response_shape),
            OutputResponseShape::Free
        )
        && req_surface.is_some_and(|surface| {
            surface.token_count >= 3
                && surface.inline_json_shape.is_none()
                && !surface.has_delivery_token_reference()
                && !surface.has_deictic_reference()
                && !surface.is_structural_locator_only_reply()
        })
        && current_request_mentions_workspace_identity(req, workspace_root)
    {
        return Some("workspace_identity_chat_route_needs_semantic_review");
    }
    None
}

fn raw_command_locator_contract_has_observable_target(
    contract: &IntentOutputContractOut,
    req_surface: Option<&crate::intent::surface_signals::PromptSurfaceSignals>,
) -> bool {
    let locator_hint = contract.locator_hint.trim();
    if !locator_hint.is_empty()
        && !locator_hint.contains('|')
        && matches!(
            parse_output_locator_kind(&contract.locator_kind),
            OutputLocatorKind::Path
                | OutputLocatorKind::Filename
                | OutputLocatorKind::CurrentWorkspace
                | OutputLocatorKind::Url
        )
    {
        return true;
    }
    req_surface.is_some_and(|surface| {
        surface.has_explicit_path_or_url()
            || surface.has_single_filename_candidate()
            || surface.has_concrete_locator_hint()
    })
}

fn normalizer_execution_recipe_declares_active_profile(
    recipe: Option<&IntentExecutionRecipeOut>,
) -> bool {
    let Some(recipe) = recipe else {
        return false;
    };
    !matches!(
        crate::execution_recipe::parse_execution_recipe_kind_text(&recipe.kind),
        crate::execution_recipe::ExecutionRecipeKind::None
    ) || !matches!(
        crate::execution_recipe::parse_execution_recipe_profile_text(&recipe.profile),
        crate::execution_recipe::ExecutionRecipeProfile::None
    )
}

fn contract_has_single_path_locator_target(
    contract: &IntentOutputContractOut,
    req_surface: Option<&crate::intent::surface_signals::PromptSurfaceSignals>,
) -> bool {
    if req_surface.is_some_and(|surface| surface.locator_target_pair.is_some()) {
        return false;
    }
    if req_surface.is_some_and(|surface| {
        surface.has_explicit_path_or_url() || surface.has_single_filename_candidate()
    }) {
        return true;
    }
    let locator_hint = contract.locator_hint.trim();
    !locator_hint.is_empty()
        && !locator_hint.contains('|')
        && matches!(
            parse_output_locator_kind(&contract.locator_kind),
            OutputLocatorKind::Path
                | OutputLocatorKind::Filename
                | OutputLocatorKind::CurrentWorkspace
        )
}
