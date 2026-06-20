use std::path::Path;

use super::{
    ActFinalizeStyle, FirstLayerDecision, IntentOutputContract, OutputDeliveryIntent,
    OutputLocatorKind, OutputResponseShape, OutputSemanticKind,
};

pub(super) fn archive_pair_contract_from_surface(
    output_contract: &IntentOutputContract,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> Option<(OutputSemanticKind, String)> {
    let generated_delivery_contract = output_contract.semantic_kind
        == OutputSemanticKind::GeneratedFileDelivery
        || (output_contract.semantic_kind == OutputSemanticKind::None
            && (output_contract.delivery_required
                || matches!(
                    output_contract.response_shape,
                    OutputResponseShape::FileToken
                )
                || matches!(
                    output_contract.delivery_intent,
                    OutputDeliveryIntent::FileSingle
                )));
    let (left, right) = req_surface.locator_target_pair.as_ref()?;
    let left_is_archive = contract_repair_supported_archive_path(left);
    let right_is_archive = contract_repair_supported_archive_path(right);
    let inferred_kind = match (left_is_archive, right_is_archive) {
        (false, true) => Some((
            OutputSemanticKind::ArchivePack,
            format!("{} | {}", left.trim(), right.trim()),
        )),
        (true, false) => Some((
            OutputSemanticKind::ArchiveUnpack,
            format!("{} | {}", left.trim(), right.trim()),
        )),
        _ => None,
    }?;
    let structural_operation_pair =
        archive_pair_has_structural_operation_shape(inferred_kind.0, left, right);
    let already_archive_contract = output_contract.semantic_kind == inferred_kind.0;
    let scalar_or_drift_contract = structural_operation_pair
        && !matches!(output_contract.response_shape, OutputResponseShape::Strict)
        && matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::None
                | OutputSemanticKind::ScalarPathOnly
                | OutputSemanticKind::ContentExcerptSummary
                | OutputSemanticKind::FilesystemMutationResult
        );
    if structural_operation_pair
        && (already_archive_contract || generated_delivery_contract || scalar_or_drift_contract)
    {
        return Some(inferred_kind);
    }
    None
}

fn archive_pair_has_structural_operation_shape(
    semantic_kind: OutputSemanticKind,
    left: &str,
    right: &str,
) -> bool {
    match semantic_kind {
        OutputSemanticKind::ArchivePack => {
            !contract_repair_supported_archive_path(left)
                && contract_repair_path_operand_is_structural(left)
                && contract_repair_supported_archive_path(right)
        }
        OutputSemanticKind::ArchiveUnpack => {
            contract_repair_supported_archive_path(left)
                && !contract_repair_supported_archive_path(right)
                && contract_repair_archive_unpack_dest_is_structural(right)
        }
        _ => false,
    }
}

fn contract_repair_archive_unpack_dest_is_structural(path: &str) -> bool {
    let path = path.trim();
    let structurally_path_like = path.starts_with("./")
        || path.starts_with("../")
        || path.starts_with('/')
        || path.starts_with("~/")
        || path.contains('/')
        || path.contains('\\');
    structurally_path_like && !path_basename_looks_like_file(path)
}

fn path_basename_looks_like_file(path: &str) -> bool {
    let basename = path.trim().rsplit(['/', '\\']).next().unwrap_or("").trim();
    let Some((stem, ext)) = basename.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && (1..=16).contains(&ext.len())
        && ext.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn contract_repair_path_operand_is_structural(path: &str) -> bool {
    let path = path.trim();
    path.starts_with("./")
        || path.starts_with("../")
        || path.starts_with('/')
        || path.starts_with("~/")
        || path.contains('/')
        || path.contains('\\')
}

fn contract_repair_supported_archive_path(path: &str) -> bool {
    let path = path.trim().to_ascii_lowercase();
    path.ends_with(".zip") || path.ends_with(".tar.gz") || path.ends_with(".tgz")
}

pub(super) fn archive_read_contract_from_surface(
    output_contract: &IntentOutputContract,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> Option<String> {
    if output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::ArchiveRead
                | OutputSemanticKind::ArchiveUnpack
                | OutputSemanticKind::ContentExcerptSummary
                | OutputSemanticKind::None
        )
    {
        return None;
    }

    if req_surface
        .locator_target_pair
        .as_ref()
        .is_some_and(|(left, right)| {
            archive_pair_has_structural_operation_shape(
                OutputSemanticKind::ArchiveUnpack,
                left,
                right,
            )
        })
    {
        return None;
    }

    let archive = if contract_repair_supported_archive_path(&output_contract.locator_hint) {
        output_contract.locator_hint.trim().to_string()
    } else {
        req_surface
            .filename_candidates
            .iter()
            .find(|candidate| contract_repair_supported_archive_path(candidate))
            .cloned()?
    };
    if let Some(member) = archive_member_from_locator_target_pair(req_surface, &archive) {
        return Some(format!("{} | {}", archive.trim(), member));
    }
    let candidates = req_surface.filename_candidates.clone();
    let archive_key = archive.trim().to_ascii_lowercase();
    let member = candidates
        .iter()
        .find(|candidate| {
            let candidate = candidate.trim();
            !candidate.is_empty()
                && candidate.to_ascii_lowercase() != archive_key
                && archive_member_candidate_is_structural(candidate)
        })?
        .trim()
        .to_string();

    Some(format!("{} | {}", archive.trim(), member))
}

pub(super) fn archive_list_contract_from_surface(
    output_contract: &IntentOutputContract,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> Option<String> {
    if output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::ArchiveList
                | OutputSemanticKind::ArchiveUnpack
                | OutputSemanticKind::FileNames
                | OutputSemanticKind::FilePaths
                | OutputSemanticKind::DirectoryEntryGroups
        )
    {
        return None;
    }
    if matches!(
        output_contract.semantic_kind,
        OutputSemanticKind::FileNames
            | OutputSemanticKind::FilePaths
            | OutputSemanticKind::DirectoryEntryGroups
    ) && !contract_repair_supported_archive_path(&output_contract.locator_hint)
        && !req_surface
            .filename_candidates
            .iter()
            .any(|candidate| contract_repair_supported_archive_path(candidate))
    {
        return None;
    }
    if req_surface
        .locator_target_pair
        .as_ref()
        .is_some_and(|(left, right)| {
            archive_pair_has_structural_operation_shape(
                OutputSemanticKind::ArchivePack,
                left,
                right,
            ) || archive_pair_has_structural_operation_shape(
                OutputSemanticKind::ArchiveUnpack,
                left,
                right,
            ) || archive_member_from_locator_target_pair_for_archive(left, right).is_some()
                || archive_member_from_locator_target_pair_for_archive(right, left).is_some()
        })
    {
        return None;
    }
    if output_contract.locator_hint.contains('|') {
        return None;
    }
    let mut candidates = Vec::new();
    push_unique_archive_candidate(&mut candidates, output_contract.locator_hint.trim());
    for candidate in &req_surface.filename_candidates {
        push_unique_archive_candidate(&mut candidates, candidate);
    }
    if candidates.len() == 1 {
        Some(candidates.remove(0))
    } else {
        None
    }
}

fn push_unique_archive_candidate(candidates: &mut Vec<String>, candidate: &str) {
    let candidate = candidate.trim();
    if !contract_repair_supported_archive_path(candidate) {
        return;
    }
    if let Some(existing) = candidates
        .iter_mut()
        .find(|existing| archive_candidate_refers_same_archive(existing, candidate))
    {
        if contract_repair_path_operand_is_structural(candidate)
            && !contract_repair_path_operand_is_structural(existing)
        {
            *existing = candidate.to_string();
        }
        return;
    }
    candidates.push(candidate.to_string());
}

fn archive_candidate_refers_same_archive(left: &str, right: &str) -> bool {
    let left = left.trim();
    let right = right.trim();
    if left.eq_ignore_ascii_case(right) {
        return true;
    }
    let left_name = Path::new(left)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(left);
    let right_name = Path::new(right)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(right);
    left_name.eq_ignore_ascii_case(right_name)
        && (contract_repair_path_operand_is_structural(left)
            || contract_repair_path_operand_is_structural(right))
}

fn archive_member_from_locator_target_pair_for_archive(
    archive_side: &str,
    member_side: &str,
) -> Option<String> {
    if !contract_repair_supported_archive_path(archive_side) {
        return None;
    }
    let member = member_side.trim();
    if archive_member_candidate_is_structural(member) {
        Some(member.to_string())
    } else {
        None
    }
}

fn archive_member_from_locator_target_pair(
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    archive: &str,
) -> Option<String> {
    let (left, right) = req_surface.locator_target_pair.as_ref()?;
    let archive_key = archive.trim().to_ascii_lowercase();
    for (archive_side, member_side) in [(left, right), (right, left)] {
        if archive_side.trim().to_ascii_lowercase() != archive_key {
            continue;
        }
        let member = member_side.trim();
        if archive_member_candidate_is_structural(member) {
            return Some(member.to_string());
        }
    }
    None
}

fn archive_member_candidate_is_structural(candidate: &str) -> bool {
    let candidate = candidate.trim();
    !candidate.is_empty()
        && !contract_repair_supported_archive_path(candidate)
        && !candidate.ends_with('/')
        && (candidate.contains('.') || candidate.contains('/') || candidate.contains('\\'))
}

fn archive_unpack_has_supported_archive_locator(
    output_contract: &IntentOutputContract,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> bool {
    output_contract
        .locator_hint
        .split('|')
        .any(contract_repair_supported_archive_path)
        || req_surface
            .locator_target_pair
            .as_ref()
            .is_some_and(|(left, right)| {
                contract_repair_supported_archive_path(left)
                    || contract_repair_supported_archive_path(right)
            })
        || req_surface
            .filename_candidates_excluding_field_selectors()
            .iter()
            .any(|candidate| contract_repair_supported_archive_path(candidate))
        || active_session_has_supported_archive_locator(session_snapshot)
}

fn active_session_has_supported_archive_locator(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> bool {
    let Some(snapshot) = session_snapshot else {
        return false;
    };
    snapshot
        .active_followup_frame
        .as_ref()
        .and_then(|frame| frame.bound_target.as_deref())
        .is_some_and(contract_repair_supported_archive_path)
        || snapshot
            .active_observed_facts
            .as_ref()
            .and_then(|facts| facts.bound_target.as_deref())
            .is_some_and(contract_repair_supported_archive_path)
        || snapshot
            .active_observed_facts
            .as_ref()
            .is_some_and(|facts| {
                facts
                    .delivery_targets
                    .iter()
                    .any(|target| contract_repair_supported_archive_path(target))
            })
}

pub(super) fn apply_archive_unpack_missing_archive_locator_clarify(
    output_contract: &mut IntentOutputContract,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    legacy_normalizer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if !matches!(
        output_contract.semantic_kind,
        OutputSemanticKind::ArchiveUnpack
    ) || !output_contract.requires_content_evidence
        || archive_unpack_has_supported_archive_locator(
            output_contract,
            req_surface,
            session_snapshot,
        )
    {
        return None;
    }
    *needs_clarify = true;
    clarify_question.clear();
    *legacy_normalizer_decision = FirstLayerDecision::Clarify;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.locator_hint.clear();
    output_contract.delivery_required = false;
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    Some("archive_unpack_missing_archive_locator_clarify")
}
