use serde_json::Value;
use std::path::{Component, Path};

use super::{
    execution_finalize_style_for_contract, is_bare_path_only_input_for_clarify,
    output_semantic_kind_requires_fresh_evidence, state_patch_deictic_reference_requires_clarify,
    state_patch_requests_filename_only_output,
    surface_has_unbound_scope_plus_single_filename_target, ActFinalizeStyle, FirstLayerDecision,
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, TargetTaskPolicy, TurnType,
};

fn clean_answer_candidate_path_token(answer_candidate: &str) -> Option<String> {
    let token = answer_candidate
        .trim()
        .trim_matches('`')
        .trim_matches('"')
        .trim_matches('\'')
        .trim();
    if token.is_empty() || token.contains('\n') {
        None
    } else {
        Some(token.to_string())
    }
}

fn existing_answer_candidate_path(answer_candidate: &str, workspace_root: &Path) -> Option<String> {
    let token = clean_answer_candidate_path_token(answer_candidate)?;
    let path = Path::new(&token);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };
    candidate.exists().then_some(token)
}

fn answer_candidate_has_path_context(path: &str) -> bool {
    let path = Path::new(path);
    if path.is_absolute() {
        return true;
    }
    let mut normal_components = 0usize;
    for component in path.components() {
        match component {
            Component::Normal(_) => normal_components += 1,
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return true;
            }
        }
    }
    normal_components > 1
}

pub(super) fn apply_answer_candidate_path_evidence_repair(
    output_contract: &mut IntentOutputContract,
    answer_candidate: &str,
    state_patch: Option<&Value>,
    workspace_root: &Path,
    needs_clarify: bool,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if needs_clarify
        || !matches!(first_layer_decision, FirstLayerDecision::DirectAnswer)
        || output_contract.requires_content_evidence
        || output_contract.delivery_required
        || output_contract.response_shape != OutputResponseShape::Scalar
        || output_contract.locator_kind != OutputLocatorKind::None
        || output_contract.delivery_intent != OutputDeliveryIntent::None
    {
        return None;
    }
    if state_patch_requests_filename_only_output(state_patch)
        || state_patch_deictic_reference_requires_clarify(state_patch)
    {
        return None;
    }
    let path = existing_answer_candidate_path(answer_candidate, workspace_root)?;
    if !answer_candidate_has_path_context(&path) {
        return None;
    }
    output_contract.requires_content_evidence = true;
    output_contract.locator_kind = OutputLocatorKind::Path;
    output_contract.locator_hint = path;
    output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    *first_layer_decision = FirstLayerDecision::PlannerExecute;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    Some("answer_candidate_path_requires_evidence")
}

pub(super) fn semantic_kind_can_use_existing_observed_context(kind: OutputSemanticKind) -> bool {
    matches!(
        kind,
        OutputSemanticKind::ContentExcerptSummary
            | OutputSemanticKind::ContentPresenceCheck
            | OutputSemanticKind::ExcerptKindJudgment
            | OutputSemanticKind::RecentArtifactsJudgment
            | OutputSemanticKind::ExecutionFailedStep
            | OutputSemanticKind::PublishingPreview
    )
}

pub(super) fn should_preserve_existing_observed_context_synthesis_contract(
    output_contract: &IntentOutputContract,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
) -> bool {
    matches!(turn_type, Some(TurnType::TaskAppend))
        && matches!(target_task_policy, Some(TargetTaskPolicy::ReuseActive))
        && semantic_kind_can_use_existing_observed_context(output_contract.semantic_kind)
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
    output_contract: &mut IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
    state_patch: Option<&Value>,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if !*needs_clarify || is_bare_path_only_input_for_clarify(req, req_surface) {
        return None;
    }
    if state_patch_deictic_reference_requires_clarify(state_patch) {
        return None;
    }
    if surface_has_unbound_scope_plus_single_filename_target(output_contract, req, req_surface) {
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
    ) || output_semantic_kind_requires_fresh_evidence(
        output_contract.semantic_kind,
    ) || req_surface.has_structured_target_refinement()
        || req_surface.locator_target_pair.is_some();
    if surface_locator_is_insufficient_for_clarify_repair(
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
    *first_layer_decision = FirstLayerDecision::PlannerExecute;
    *execution_finalize_style =
        crate::post_route_policy::content_evidence_execution_finalize_style(output_contract, false)
            .unwrap_or_else(|| execution_finalize_style_for_contract(output_contract));
    Some("structured_observation_clarify_repair")
}

fn surface_locator_is_insufficient_for_clarify_repair(
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
    if matches!(
        output_contract.semantic_kind,
        OutputSemanticKind::ArchivePack | OutputSemanticKind::ArchiveUnpack
    ) {
        return true;
    }
    !output_contract.requires_content_evidence && !has_observable_answer_shape
}

fn semantic_kind_can_use_workspace_default_for_observation(kind: OutputSemanticKind) -> bool {
    matches!(
        kind,
        OutputSemanticKind::HiddenEntriesCheck
            | OutputSemanticKind::FileNames
            | OutputSemanticKind::DirectoryNames
            | OutputSemanticKind::DirectoryEntryGroups
            | OutputSemanticKind::FilePaths
            | OutputSemanticKind::DirectoryPurposeSummary
            | OutputSemanticKind::WorkspaceProjectSummary
            | OutputSemanticKind::ExistenceWithPath
            | OutputSemanticKind::ExistenceWithPathSummary
            | OutputSemanticKind::GitCommitSubject
            | OutputSemanticKind::GitRepositoryState
            | OutputSemanticKind::DockerPs
            | OutputSemanticKind::DockerImages
            | OutputSemanticKind::DockerLogs
            | OutputSemanticKind::DockerContainerLifecycle
    )
}

pub(super) fn apply_workspace_default_observation_clarify_repair(
    output_contract: &mut IntentOutputContract,
    workspace_root: &Path,
    state_patch: Option<&Value>,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if !*needs_clarify
        || !output_contract.requires_content_evidence
        || !semantic_kind_can_use_workspace_default_for_observation(output_contract.semantic_kind)
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
    *first_layer_decision = FirstLayerDecision::PlannerExecute;
    *execution_finalize_style =
        crate::post_route_policy::content_evidence_execution_finalize_style(output_contract, false)
            .unwrap_or_else(|| execution_finalize_style_for_contract(output_contract));
    Some("workspace_default_observation_clarify_repair")
}

fn semantic_kind_can_use_locatorless_default_for_observation(kind: OutputSemanticKind) -> bool {
    matches!(
        kind,
        OutputSemanticKind::ServiceStatus
            | OutputSemanticKind::PackageManagerDetection
            | OutputSemanticKind::WeatherQuery
            | OutputSemanticKind::MarketQuote
            | OutputSemanticKind::WebSearchSummary
            | OutputSemanticKind::DockerPs
            | OutputSemanticKind::DockerImages
    )
}

pub(super) fn apply_locatorless_observation_clarify_repair(
    output_contract: &mut IntentOutputContract,
    state_patch: Option<&Value>,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if !*needs_clarify
        || !output_contract.requires_content_evidence
        || output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(output_contract.locator_kind, OutputLocatorKind::None)
        || !output_contract.locator_hint.trim().is_empty()
        || !semantic_kind_can_use_locatorless_default_for_observation(output_contract.semantic_kind)
    {
        return None;
    }
    if state_patch_deictic_reference_requires_clarify(state_patch) {
        return None;
    }
    *needs_clarify = false;
    clarify_question.clear();
    *first_layer_decision = FirstLayerDecision::PlannerExecute;
    *execution_finalize_style =
        crate::post_route_policy::content_evidence_execution_finalize_style(output_contract, false)
            .unwrap_or_else(|| execution_finalize_style_for_contract(output_contract));
    Some("locatorless_observation_clarify_repair")
}
