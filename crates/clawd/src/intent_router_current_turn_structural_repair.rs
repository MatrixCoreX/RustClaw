use serde_json::Value;
use std::path::Path;

use crate::{ActFinalizeStyle, FirstLayerDecision};

use super::{
    active_primary_task_prompt, archive_list_contract_from_surface,
    archive_pair_contract_from_surface, archive_read_contract_from_surface,
    config_mutation_contract_from_surface,
    current_turn_extension_inventory_file_paths_repair_applies,
    current_workspace_generic_summary_needs_semantic_contract,
    existence_with_path_mixed_locator_summary_repair,
    explicit_surface_path_facts_fallback_decision,
    extension_inventory_locator_hint_should_use_workspace,
    file_paths_missing_file_locator_parent_dir,
    generated_file_delivery_existing_content_summary_repair,
    generated_file_delivery_filename_only_existing_target_repair, locator_hint_is_unset_or_broad,
    locator_hint_points_to_workspace_root, output_semantic_kind_requires_fresh_evidence,
    planner_execute_inline_structured_payload_context,
    planner_execute_inline_structured_transform_contract_context,
    quoted_literal_content_presence_contract_repair, scope_patch_hint_value,
    should_preserve_existing_observed_context_synthesis_contract,
    structural_config_value_after_field, structured_config_keys_contract_from_surface,
    structured_field_pair_contract_from_quantity_comparison,
    structured_field_value_contract_from_quantity_comparison,
    structured_identifier_presence_contract_from_surface,
    surface_has_directory_scoped_filename_lookup, workspace_direct_child_stem_locator_from_text,
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputScalarCountTargetKind, OutputSemanticKind, ScheduleKind, TargetTaskPolicy, TurnType,
};

pub(super) fn should_detach_bare_acknowledgement_from_active_task(
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    legacy_normalizer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    state_patch: Option<&Value>,
    should_refresh_long_term_memory: bool,
) -> bool {
    matches!(turn_type, Some(TurnType::PreferenceOrMemory))
        && matches!(target_task_policy, Some(TargetTaskPolicy::ReuseActive))
        && matches!(legacy_normalizer_decision, FirstLayerDecision::DirectAnswer)
        && !output_contract.requires_content_evidence
        && !output_contract.delivery_required
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && !should_refresh_long_term_memory
        && state_patch.is_none()
}

pub(super) fn should_downgrade_orphan_output_shape_clarify_to_direct_answer(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    legacy_normalizer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    state_patch: Option<&Value>,
    should_refresh_long_term_memory: bool,
    attachment_processing_required: bool,
) -> bool {
    matches!(legacy_normalizer_decision, FirstLayerDecision::Clarify)
        && active_primary_task_prompt(session_snapshot).is_none()
        && matches!(
            turn_type,
            Some(TurnType::TaskAppend | TurnType::TaskCorrect)
        )
        && matches!(target_task_policy, Some(TargetTaskPolicy::ReuseActive))
        && !should_refresh_long_term_memory
        && state_patch.is_none()
        && !attachment_processing_required
        && !output_contract.requires_content_evidence
        && !output_contract.delivery_required
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(output_contract.semantic_kind, OutputSemanticKind::None)
        && matches!(
            output_contract.response_shape,
            OutputResponseShape::Free
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Strict
        )
}

pub(super) fn should_downgrade_standalone_freeform_clarify_to_direct_answer(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    legacy_normalizer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    state_patch: Option<&Value>,
    should_refresh_long_term_memory: bool,
    attachment_processing_required: bool,
    wants_file_delivery: bool,
    schedule_kind: ScheduleKind,
) -> bool {
    matches!(legacy_normalizer_decision, FirstLayerDecision::Clarify)
        && active_primary_task_prompt(session_snapshot).is_none()
        && matches!(turn_type, None | Some(TurnType::TaskRequest))
        && matches!(
            target_task_policy,
            None | Some(TargetTaskPolicy::Standalone) | Some(TargetTaskPolicy::ReuseActive)
        )
        && !should_refresh_long_term_memory
        && state_patch.is_none()
        && !attachment_processing_required
        && !wants_file_delivery
        && matches!(schedule_kind, ScheduleKind::None)
        && !output_contract.requires_content_evidence
        && !output_contract.delivery_required
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(output_contract.semantic_kind, OutputSemanticKind::None)
        && matches!(output_contract.response_shape, OutputResponseShape::Free)
}

pub(super) fn infer_missing_target_policy_from_contract(
    target_task_policy: Option<TargetTaskPolicy>,
    turn_type: Option<TurnType>,
    legacy_normalizer_decision: FirstLayerDecision,
    needs_clarify: bool,
    schedule_kind: ScheduleKind,
    should_refresh_long_term_memory: bool,
    output_contract: &IntentOutputContract,
) -> Option<TargetTaskPolicy> {
    if target_task_policy.is_some()
        || turn_type.is_some()
        || needs_clarify
        || should_refresh_long_term_memory
        || !matches!(schedule_kind, ScheduleKind::None)
        || !matches!(legacy_normalizer_decision, FirstLayerDecision::DirectAnswer)
    {
        return target_task_policy;
    }

    let strict_chat_deliverable =
        matches!(output_contract.response_shape, OutputResponseShape::Strict)
            && !output_contract.requires_content_evidence
            && !output_contract.delivery_required
            && matches!(output_contract.locator_kind, OutputLocatorKind::None)
            && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
            && matches!(output_contract.semantic_kind, OutputSemanticKind::None);

    if strict_chat_deliverable {
        Some(TargetTaskPolicy::Standalone)
    } else {
        target_task_policy
    }
}

pub(super) fn is_meaningful_state_patch(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Object(map) => map.values().any(is_meaningful_state_patch),
        Value::Array(items) => items.iter().any(is_meaningful_state_patch),
        Value::String(text) => !text.trim().is_empty(),
        _ => true,
    }
}

pub(super) fn apply_workspace_scope_patch_to_contract(
    output_contract: &mut IntentOutputContract,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    state_patch: Option<&Value>,
) -> Option<String> {
    if !matches!(turn_type, Some(TurnType::TaskScopeUpdate))
        || !matches!(target_task_policy, Some(TargetTaskPolicy::ReuseActive))
        || output_contract.semantic_kind != OutputSemanticKind::WorkspaceProjectSummary
    {
        return None;
    }
    let scope_hint = scope_patch_hint_value(state_patch?)?;
    if !locator_hint_is_unset_or_broad(&output_contract.locator_hint) {
        return None;
    }
    output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    output_contract.locator_hint = scope_hint.clone();
    Some(scope_hint)
}

pub(super) fn apply_current_turn_structural_contract_repair(
    output_contract: &mut IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
    legacy_normalizer_decision: FirstLayerDecision,
    answer_candidate: &str,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
) -> Option<&'static str> {
    let mut reason = None;
    if should_preserve_existing_observed_context_synthesis_contract(
        output_contract,
        req_surface,
        turn_type,
        target_task_policy,
    ) {
        output_contract.requires_content_evidence = false;
        reason = Some("existing_observed_context_synthesis");
    } else if output_semantic_kind_requires_fresh_evidence(output_contract.semantic_kind) {
        output_contract.requires_content_evidence = true;
        reason = Some("semantic_contract_requires_evidence");
    }

    if let Some((semantic_kind, locator_hint)) =
        archive_pair_contract_from_surface(output_contract, req_surface)
    {
        output_contract.semantic_kind = semantic_kind;
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.delivery_intent = OutputDeliveryIntent::None;
        output_contract.response_shape = match semantic_kind {
            OutputSemanticKind::ArchivePack => OutputResponseShape::Scalar,
            OutputSemanticKind::ArchiveUnpack => OutputResponseShape::OneSentence,
            _ => output_contract.response_shape,
        };
        output_contract.locator_kind = OutputLocatorKind::Path;
        output_contract.locator_hint = locator_hint;
        reason = Some(match semantic_kind {
            OutputSemanticKind::ArchivePack => "archive_pack_pair_contract_repair",
            OutputSemanticKind::ArchiveUnpack => "archive_unpack_pair_contract_repair",
            _ => "archive_pair_contract_repair",
        });
    }

    if planner_execute_inline_structured_payload_context(
        req,
        req_surface,
        legacy_normalizer_decision,
        output_contract,
    ) {
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.delivery_intent = OutputDeliveryIntent::None;
        output_contract.locator_kind = OutputLocatorKind::None;
        output_contract.locator_hint.clear();
        output_contract.semantic_kind = OutputSemanticKind::None;
        if matches!(
            output_contract.response_shape,
            OutputResponseShape::Free | OutputResponseShape::OneSentence
        ) {
            output_contract.response_shape = OutputResponseShape::Strict;
        }
        reason = Some("inline_structured_payload_context_execute");
    }

    if planner_execute_inline_structured_transform_contract_context(
        req_surface,
        legacy_normalizer_decision,
        output_contract,
        answer_candidate,
    ) {
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.delivery_intent = OutputDeliveryIntent::None;
        output_contract.locator_kind = OutputLocatorKind::None;
        output_contract.locator_hint.clear();
        output_contract.semantic_kind = OutputSemanticKind::None;
        if matches!(
            output_contract.response_shape,
            OutputResponseShape::Free | OutputResponseShape::OneSentence
        ) {
            output_contract.response_shape = OutputResponseShape::Strict;
        }
        reason = Some("inline_structured_transform_contract_repair");
    }

    if matches!(
        legacy_normalizer_decision,
        FirstLayerDecision::PlannerExecute
    ) && output_contract.delivery_required
        && matches!(
            output_contract.response_shape,
            OutputResponseShape::FileToken
        )
        && matches!(
            output_contract.delivery_intent,
            OutputDeliveryIntent::FileSingle
        )
        && matches!(output_contract.semantic_kind, OutputSemanticKind::None)
        && output_contract.locator_hint.trim().is_empty()
        && answer_candidate.trim().is_empty()
        && !req_surface.has_delivery_token_reference()
    {
        output_contract.semantic_kind = OutputSemanticKind::GeneratedFileDelivery;
        output_contract.requires_content_evidence = true;
        reason = Some("file_token_delivery_contract_repair");
    }

    if let Some(filename) = generated_file_delivery_filename_only_existing_target_repair(
        output_contract,
        req_surface,
        workspace_root,
    ) {
        output_contract.semantic_kind = OutputSemanticKind::None;
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = true;
        output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
        output_contract.response_shape = OutputResponseShape::FileToken;
        output_contract.locator_kind = OutputLocatorKind::Filename;
        output_contract.locator_hint = filename;
        reason = Some("generated_file_delivery_filename_only_existing_target_repair");
    }

    if let Some(locator_hint) =
        generated_file_delivery_existing_content_summary_repair(output_contract, workspace_root)
    {
        output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = true;
        output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
        output_contract.response_shape = OutputResponseShape::Strict;
        output_contract.locator_kind = OutputLocatorKind::Path;
        output_contract.locator_hint = locator_hint;
        reason = Some("generated_file_delivery_existing_content_summary_repair");
    }

    if let Some(locator_hint) = archive_read_contract_from_surface(output_contract, req_surface) {
        output_contract.semantic_kind = OutputSemanticKind::ArchiveRead;
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.delivery_intent = OutputDeliveryIntent::None;
        output_contract.response_shape = OutputResponseShape::Free;
        output_contract.locator_kind = OutputLocatorKind::Path;
        output_contract.locator_hint = locator_hint;
        reason = Some("archive_read_member_contract_repair");
    }

    if let Some(locator_hint) = archive_list_contract_from_surface(output_contract, req_surface) {
        let repaired_from_semantic_kind = output_contract.semantic_kind;
        output_contract.semantic_kind = OutputSemanticKind::ArchiveList;
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.delivery_intent = OutputDeliveryIntent::None;
        output_contract.response_shape = OutputResponseShape::Strict;
        output_contract.locator_kind = OutputLocatorKind::Path;
        output_contract.locator_hint = locator_hint;
        if !output_contract
            .self_extension
            .list_selector
            .target_kind_specified
            && (matches!(
                repaired_from_semantic_kind,
                OutputSemanticKind::ArchiveList
                    | OutputSemanticKind::ArchiveUnpack
                    | OutputSemanticKind::FileNames
                    | OutputSemanticKind::FilePaths
            ) || output_contract.self_extension.list_selector.target_kind
                == OutputScalarCountTargetKind::Any)
        {
            output_contract.self_extension.list_selector.target_kind =
                OutputScalarCountTargetKind::File;
            output_contract
                .self_extension
                .list_selector
                .target_kind_specified = true;
        }
        reason = Some("archive_list_single_archive_contract_repair");
    }

    if let Some(locator_hint) = config_mutation_contract_from_surface(
        output_contract,
        req,
        req_surface,
        legacy_normalizer_decision,
    ) {
        output_contract.semantic_kind = OutputSemanticKind::ConfigMutation;
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.delivery_intent = OutputDeliveryIntent::None;
        output_contract.response_shape = OutputResponseShape::Free;
        output_contract.locator_kind = OutputLocatorKind::Path;
        output_contract.locator_hint = locator_hint;
        reason = Some("config_mutation_structural_contract_repair");
    }

    if let Some(repair_reason) =
        apply_fs_basic_lifecycle_machine_contract_repair(output_contract, req)
    {
        reason = Some(repair_reason);
    }

    if let Some(locator_hint) = structured_config_keys_contract_from_surface(output_contract, req) {
        output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.delivery_intent = OutputDeliveryIntent::None;
        output_contract.response_shape = OutputResponseShape::Strict;
        output_contract.locator_kind = OutputLocatorKind::Path;
        output_contract.locator_hint = locator_hint;
        reason = Some("structured_config_keys_overrides_file_names");
    }

    if output_contract.semantic_kind == OutputSemanticKind::ScalarPathOnly
        && req_surface.has_structured_target_refinement()
        && !surface_has_directory_scoped_filename_lookup(req, req_surface, workspace_root)
    {
        output_contract.semantic_kind = OutputSemanticKind::None;
        output_contract.requires_content_evidence = true;
        reason = Some("structured_file_scalar_repair");
    }

    if output_contract.semantic_kind == OutputSemanticKind::StructuredKeys
        && req_surface.dotted_field_selector.is_some()
    {
        output_contract.semantic_kind = OutputSemanticKind::None;
        output_contract.response_shape = OutputResponseShape::Scalar;
        output_contract.requires_content_evidence = true;
        reason = Some("structured_field_selector_requires_scalar_value");
    }

    if let Some(locator_hint) =
        structured_field_pair_contract_from_quantity_comparison(output_contract, req, req_surface)
    {
        output_contract.semantic_kind = OutputSemanticKind::RecentScalarEqualityCheck;
        output_contract.response_shape = OutputResponseShape::Strict;
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.delivery_intent = OutputDeliveryIntent::None;
        output_contract.locator_kind = OutputLocatorKind::Path;
        output_contract.locator_hint = locator_hint;
        reason = Some("structured_field_pair_requires_scalar_equality_check");
    }

    if let Some(locator_hint) =
        structured_field_value_contract_from_quantity_comparison(output_contract, req, req_surface)
    {
        output_contract.semantic_kind = OutputSemanticKind::None;
        output_contract.response_shape = OutputResponseShape::Scalar;
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.delivery_intent = OutputDeliveryIntent::None;
        output_contract.locator_kind = OutputLocatorKind::Path;
        output_contract.locator_hint = locator_hint;
        reason = Some("structured_field_selector_requires_scalar_value");
    }

    if output_contract.semantic_kind == OutputSemanticKind::ConfigValidation
        && req_surface
            .dotted_field_selector
            .as_deref()
            .is_some_and(|field_path| !structural_config_value_after_field(req, field_path))
    {
        output_contract.semantic_kind = OutputSemanticKind::None;
        output_contract.response_shape = OutputResponseShape::Scalar;
        output_contract.requires_content_evidence = true;
        reason = Some("config_validation_field_selector_requires_scalar_value");
    }

    if let Some(locator_hint) =
        structured_identifier_presence_contract_from_surface(output_contract, req, workspace_root)
    {
        output_contract.semantic_kind = OutputSemanticKind::None;
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.delivery_intent = OutputDeliveryIntent::None;
        output_contract.response_shape = OutputResponseShape::Scalar;
        output_contract.locator_kind = OutputLocatorKind::Path;
        output_contract.locator_hint = locator_hint;
        reason = Some("structured_identifier_presence_requires_content_evidence");
    }

    if output_contract.semantic_kind == OutputSemanticKind::StructuredKeys
        && matches!(output_contract.response_shape, OutputResponseShape::Scalar)
        && !output_contract.delivery_required
        && matches!(
            output_contract.locator_kind,
            OutputLocatorKind::Path
                | OutputLocatorKind::Filename
                | OutputLocatorKind::CurrentWorkspace
        )
    {
        output_contract.semantic_kind = OutputSemanticKind::None;
        output_contract.requires_content_evidence = true;
        reason = Some("structured_keys_scalar_response_requires_field_value");
    }

    if current_workspace_generic_summary_needs_semantic_contract(
        output_contract,
        legacy_normalizer_decision,
    ) {
        if current_turn_extension_inventory_file_paths_repair_applies(
            output_contract,
            req,
            req_surface,
        ) {
            output_contract.semantic_kind = OutputSemanticKind::FilePaths;
            output_contract.response_shape = OutputResponseShape::Strict;
            output_contract.requires_content_evidence = true;
            output_contract.delivery_required = false;
            output_contract.delivery_intent = OutputDeliveryIntent::None;
            if extension_inventory_locator_hint_should_use_workspace(
                &output_contract.locator_hint,
                workspace_root,
            ) {
                output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
                output_contract.locator_hint = workspace_root.display().to_string();
            }
            reason = Some("current_workspace_extension_file_paths_contract_repair");
        } else {
            output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
            if output_contract.locator_hint.trim().is_empty() {
                output_contract.locator_hint = workspace_root.display().to_string();
            }
            reason = Some("current_workspace_summary_semantic_contract_repair");
        }
    }

    if output_contract.semantic_kind == OutputSemanticKind::WorkspaceProjectSummary
        && !matches!(
            output_contract.locator_kind,
            OutputLocatorKind::None | OutputLocatorKind::CurrentWorkspace
        )
        && locator_hint_points_to_workspace_root(&output_contract.locator_hint, workspace_root)
    {
        output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        reason = reason.or(Some("workspace_summary_root_locator_repair"));
    }

    if let Some(locator_hint) =
        file_paths_missing_file_locator_parent_dir(output_contract, workspace_root)
    {
        output_contract.locator_kind = OutputLocatorKind::Path;
        output_contract.locator_hint = locator_hint;
        reason = Some("file_paths_missing_file_locator_parent_dir_repair");
    }

    if existence_with_path_mixed_locator_summary_repair(output_contract, req_surface) {
        output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPathSummary;
        output_contract.requires_content_evidence = true;
        reason = Some("existence_with_path_mixed_locator_summary_repair");
    }

    if quoted_literal_content_presence_contract_repair(output_contract, req_surface) {
        output_contract.semantic_kind = OutputSemanticKind::ContentPresenceCheck;
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.delivery_intent = OutputDeliveryIntent::None;
        if matches!(
            output_contract.response_shape,
            OutputResponseShape::Free | OutputResponseShape::Scalar
        ) {
            output_contract.response_shape = OutputResponseShape::OneSentence;
        }
        reason = Some("quoted_literal_content_presence_contract_repair");
    }

    let scalar_direct_answer =
        matches!(legacy_normalizer_decision, FirstLayerDecision::DirectAnswer)
            && !answer_candidate.trim().is_empty()
            && !req_surface.has_structured_target_refinement();

    if matches!(output_contract.response_shape, OutputResponseShape::Scalar)
        && !output_contract.delivery_required
        && !scalar_direct_answer
        && (req_surface.has_explicit_path_or_url() || req_surface.has_filename_candidates())
    {
        output_contract.requires_content_evidence = true;
        reason = reason.or(Some("scalar_locator_requires_evidence"));
    }

    if matches!(
        legacy_normalizer_decision,
        FirstLayerDecision::PlannerExecute
    ) && !output_contract.requires_content_evidence
        && !output_contract.delivery_required
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(output_contract.semantic_kind, OutputSemanticKind::None)
        && req_surface.inline_json_shape.is_none()
        && (req_surface.has_explicit_path_or_url() || req_surface.has_filename_candidates())
        && !req_surface.is_structural_locator_only_reply()
    {
        output_contract.requires_content_evidence = true;
        reason = reason.or(Some("planner_locator_requires_evidence"));
    }

    if output_contract.requires_content_evidence
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && !semantic_kind_uses_locatorless_system_observation(output_contract.semantic_kind)
        && !planner_execute_inline_structured_payload_context(
            req,
            req_surface,
            legacy_normalizer_decision,
            output_contract,
        )
    {
        let filename_candidates = req_surface.filename_candidates_excluding_field_selectors();
        if let Some(locator) =
            crate::intent::locator_extractor::extract_explicit_locator_for_fallback(req)
        {
            output_contract.locator_kind = locator.locator_kind;
            output_contract.locator_hint = locator.locator_hint;
            reason = reason.or(Some("structured_locator_contract_repair"));
        } else if let Some(locator_hint) =
            workspace_direct_child_stem_locator_from_text(req, workspace_root)
        {
            output_contract.locator_kind = OutputLocatorKind::Path;
            output_contract.locator_hint = locator_hint;
            reason = reason.or(Some("workspace_direct_child_stem_contract_repair"));
        } else if filename_candidates.len() == 1 {
            output_contract.locator_kind = OutputLocatorKind::Filename;
            output_contract.locator_hint = filename_candidates[0].clone();
            reason = reason.or(Some("filename_target_contract_repair"));
        } else if !filename_candidates.is_empty() {
            output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
            output_contract.locator_hint = workspace_root.display().to_string();
            reason = reason.or(Some("workspace_filename_targets_contract_repair"));
        }
    }

    if output_contract.requires_content_evidence
        && matches!(
            output_contract.locator_kind,
            OutputLocatorKind::Path
                | OutputLocatorKind::Filename
                | OutputLocatorKind::CurrentWorkspace
        )
        && matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::ExistenceWithPath | OutputSemanticKind::ExistenceWithPathSummary
        )
        && explicit_surface_path_facts_fallback_decision(req, req_surface, workspace_root).is_some()
    {
        output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        output_contract.locator_hint = workspace_root.display().to_string();
        reason = reason.or(Some("explicit_multi_path_facts_workspace_contract_repair"));
    }

    reason
}

pub(super) fn apply_fs_basic_lifecycle_machine_contract_repair(
    output_contract: &mut IntentOutputContract,
    machine_context: &str,
) -> Option<&'static str> {
    if !command_summary_declares_fs_basic_lifecycle(output_contract, machine_context) {
        return None;
    }
    output_contract.semantic_kind = OutputSemanticKind::FilesystemMutationResult;
    output_contract.requires_content_evidence = true;
    output_contract.delivery_required = false;
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    if matches!(
        output_contract.response_shape,
        OutputResponseShape::Free | OutputResponseShape::Scalar
    ) {
        output_contract.response_shape = OutputResponseShape::Strict;
    }
    Some("fs_basic_lifecycle_contract_repair")
}

fn command_summary_declares_fs_basic_lifecycle(
    output_contract: &IntentOutputContract,
    machine_context: &str,
) -> bool {
    output_contract.semantic_kind == OutputSemanticKind::CommandOutputSummary
        && output_contract.requires_content_evidence
        && matches!(output_contract.locator_kind, OutputLocatorKind::Path)
        && !output_contract.locator_hint.trim().is_empty()
        && machine_context.contains("fs_basic.make_dir")
        && machine_context.contains("write_text")
        && machine_context.contains("append_text")
        && machine_context.contains("read_text_range")
        && machine_context.contains("remove_path")
}

fn semantic_kind_uses_locatorless_system_observation(kind: OutputSemanticKind) -> bool {
    matches!(
        kind,
        OutputSemanticKind::RawCommandOutput
            | OutputSemanticKind::CommandOutputSummary
            | OutputSemanticKind::ServiceStatus
            | OutputSemanticKind::PackageManagerDetection
            | OutputSemanticKind::ToolDiscovery
            | OutputSemanticKind::DockerPs
            | OutputSemanticKind::DockerImages
            | OutputSemanticKind::DockerLogs
            | OutputSemanticKind::DockerContainerLifecycle
            | OutputSemanticKind::WeatherQuery
            | OutputSemanticKind::MarketQuote
            | OutputSemanticKind::ImageUnderstanding
            | OutputSemanticKind::PublishingPreview
    )
}

pub(super) fn apply_unbound_workspace_generic_content_clarify_repair(
    output_contract: &mut IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    legacy_normalizer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if *needs_clarify
        || !matches!(
            *legacy_normalizer_decision,
            FirstLayerDecision::PlannerExecute
        )
        || !output_contract.requires_content_evidence
        || output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(output_contract.semantic_kind, OutputSemanticKind::None)
        || matches!(
            output_contract.response_shape,
            OutputResponseShape::FileToken
        )
        || !matches!(
            output_contract.locator_kind,
            OutputLocatorKind::None | OutputLocatorKind::CurrentWorkspace
        )
        || !short_unbound_topic_surface(req, req_surface)
    {
        return None;
    }

    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.locator_hint.clear();
    *needs_clarify = true;
    clarify_question.clear();
    *legacy_normalizer_decision = FirstLayerDecision::Clarify;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    Some("unbound_workspace_generic_content_requires_clarify")
}

fn short_unbound_topic_surface(
    req: &str,
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    let trimmed = req.trim();
    if trimmed.is_empty()
        || surface.token_count != 1
        || surface.inline_json_shape.is_some()
        || surface.has_concrete_locator_hint()
        || surface.has_filename_candidates()
        || surface.locator_target_pair.is_some()
        || surface.has_structured_target_refinement()
        || surface.has_delivery_token_reference()
        || surface.has_deictic_reference()
        || surface.is_structural_locator_only_reply()
        || trimmed.contains(['/', '\\', '.', ':'])
    {
        return false;
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return false;
    }
    let signal_chars = trimmed.chars().filter(|ch| ch.is_alphanumeric()).count();
    if signal_chars == 0 {
        return false;
    }
    if trimmed.is_ascii() {
        signal_chars <= 32
    } else {
        signal_chars <= 8
    }
}
