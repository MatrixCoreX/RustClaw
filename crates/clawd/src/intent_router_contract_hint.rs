use std::path::Path;

use super::{
    archive_list_contract_from_surface, archive_pair_contract_from_surface,
    archive_read_contract_from_surface, execution_finalize_style_for_contract,
    explicit_surface_path_fact_targets, output_semantic_kind_requires_fresh_evidence,
    parse_output_semantic_kind, ActFinalizeStyle, IntentOutputContract, OutputDeliveryIntent,
    OutputLocatorKind, OutputResponseShape, OutputSemanticKind, RouteDecision, ScheduleKind,
};

pub(crate) fn contract_test_hint_runtime_enabled() -> bool {
    cfg!(test)
}

pub(crate) fn contract_test_hint_value(req: &str, wanted_key: &str) -> Option<String> {
    if !contract_test_hint_runtime_enabled() {
        return None;
    }
    let hint_block = req
        .split_once("[CONTRACT_TEST_HINT]")?
        .1
        .split_once("[/CONTRACT_TEST_HINT]")?
        .0;
    for line in hint_block.lines().map(str::trim) {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != wanted_key {
            continue;
        }
        let value = value.trim();
        return (!value.is_empty()).then(|| value.to_string());
    }
    None
}

pub(crate) fn contract_test_hint_semantic_kind(req: &str) -> Option<OutputSemanticKind> {
    let semantic_kind =
        parse_output_semantic_kind(&contract_test_hint_value(req, "semantic_kind")?);
    (semantic_kind != OutputSemanticKind::None
        && !semantic_kind.is_normalizer_schema_capability_bridge())
    .then_some(semantic_kind)
}

pub(crate) fn request_without_contract_test_hint(req: &str) -> String {
    let mut remaining = req;
    let mut out = String::with_capacity(req.len());
    while let Some((before, after_start)) = remaining.split_once("[CONTRACT_TEST_HINT]") {
        out.push_str(before);
        let Some((_, after_end)) = after_start.split_once("[/CONTRACT_TEST_HINT]") else {
            remaining = "";
            break;
        };
        remaining = after_end;
    }
    out.push_str(remaining);
    out
}

fn contract_hint_requires_content_evidence(semantic_kind: OutputSemanticKind) -> bool {
    output_semantic_kind_requires_fresh_evidence(semantic_kind)
}

pub(super) fn apply_structured_contract_hint_repair(
    output_contract: &mut IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
    wants_file_delivery: &mut bool,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    let semantic_kind = contract_test_hint_semantic_kind(req)?;
    let surface_req = request_without_contract_test_hint(req);
    output_contract.semantic_kind = semantic_kind;
    output_contract.requires_content_evidence =
        contract_hint_requires_content_evidence(semantic_kind);
    output_contract.delivery_required = false;
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    output_contract.response_shape = response_shape_for_contract_hint_fallback(semantic_kind);
    apply_contract_hint_delivery_defaults(output_contract, wants_file_delivery);
    match semantic_kind {
        OutputSemanticKind::GitCommitSubject | OutputSemanticKind::GitRepositoryState => {
            if matches!(
                output_contract.locator_kind,
                OutputLocatorKind::None | OutputLocatorKind::Path
            ) && output_contract.locator_hint.trim().is_empty()
            {
                output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
                output_contract.locator_hint = workspace_root.display().to_string();
            }
        }
        _ => {}
    }
    apply_contract_hint_locator_defaults(
        output_contract,
        &surface_req,
        req_surface,
        workspace_root,
    );
    if output_contract.requires_content_evidence {
        *needs_clarify = false;
        clarify_question.clear();
        *execution_finalize_style =
            crate::post_route_policy::content_evidence_execution_finalize_style(
                output_contract,
                false,
            )
            .unwrap_or_else(|| execution_finalize_style_for_contract(output_contract));
    }
    Some("structured_contract_hint_repair")
}

pub(super) fn contract_hint_fallback_decision(
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
    reason: &'static str,
) -> Option<RouteDecision> {
    let semantic_kind = contract_test_hint_semantic_kind(req)?;
    let surface_req = request_without_contract_test_hint(req);
    let mut wants_file_delivery = false;
    let mut output_contract = IntentOutputContract {
        response_shape: response_shape_for_contract_hint_fallback(semantic_kind),
        requires_content_evidence: contract_hint_requires_content_evidence(semantic_kind),
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind,
        locator_hint: String::new(),
        ..Default::default()
    };
    apply_contract_hint_delivery_defaults(&mut output_contract, &mut wants_file_delivery);
    apply_contract_hint_locator_defaults(
        &mut output_contract,
        &surface_req,
        req_surface,
        workspace_root,
    );

    let resolved_user_intent = if surface_req.trim().is_empty() {
        req.trim().to_string()
    } else {
        surface_req.trim().to_string()
    };
    Some(RouteDecision {
        resolved_user_intent,
        needs_clarify: false,
        clarify_question: String::new(),
        reason: reason.to_string(),
        confidence: Some(0.70),
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract,
    })
}

fn response_shape_for_contract_hint_fallback(kind: OutputSemanticKind) -> OutputResponseShape {
    match kind {
        OutputSemanticKind::RawCommandOutput
        | OutputSemanticKind::CommandOutputSummary
        | OutputSemanticKind::ServiceStatus
        | OutputSemanticKind::DirectoryPurposeSummary
        | OutputSemanticKind::ContentExcerptSummary
        | OutputSemanticKind::ContentPresenceCheck
        | OutputSemanticKind::ExcerptKindJudgment
        | OutputSemanticKind::RecentArtifactsJudgment
        | OutputSemanticKind::WorkspaceProjectSummary
        | OutputSemanticKind::ExecutionFailedStep
        | OutputSemanticKind::ExistenceWithPathSummary
        | OutputSemanticKind::FilesystemMutationResult
        | OutputSemanticKind::GitRepositoryState
        | OutputSemanticKind::ConfigValidation
        | OutputSemanticKind::ConfigMutation
        | OutputSemanticKind::ConfigRiskAssessment
        | OutputSemanticKind::SqliteDatabaseKindJudgment
        | OutputSemanticKind::ArchiveUnpack => OutputResponseShape::OneSentence,
        OutputSemanticKind::ScalarCount
        | OutputSemanticKind::ScalarPathOnly
        | OutputSemanticKind::GeneratedFilePathReport
        | OutputSemanticKind::FileBasename
        | OutputSemanticKind::DocumentHeading
        | OutputSemanticKind::RecentScalarEqualityCheck
        | OutputSemanticKind::GitCommitSubject
        | OutputSemanticKind::SqliteSchemaVersion
        | OutputSemanticKind::ArchivePack => OutputResponseShape::Scalar,
        OutputSemanticKind::GeneratedFileDelivery => OutputResponseShape::FileToken,
        OutputSemanticKind::None
        | OutputSemanticKind::HiddenEntriesCheck
        | OutputSemanticKind::FileNames
        | OutputSemanticKind::DirectoryNames
        | OutputSemanticKind::DirectoryEntryGroups
        | OutputSemanticKind::FilePaths
        | OutputSemanticKind::ContentExcerptWithSummary
        | OutputSemanticKind::QuantityComparison
        | OutputSemanticKind::ExistenceWithPath
        | OutputSemanticKind::StructuredKeys
        | OutputSemanticKind::SqliteTableListing
        | OutputSemanticKind::SqliteTableNamesOnly
        | OutputSemanticKind::ArchiveList
        | OutputSemanticKind::ArchiveRead => OutputResponseShape::Strict,
        _ => OutputResponseShape::Strict,
    }
}

fn apply_contract_hint_delivery_defaults(
    output_contract: &mut IntentOutputContract,
    wants_file_delivery: &mut bool,
) {
    if !output_contract.semantic_kind_is(OutputSemanticKind::GeneratedFileDelivery) {
        return;
    }
    output_contract.delivery_required = true;
    output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    output_contract.response_shape = OutputResponseShape::FileToken;
    *wants_file_delivery = true;
}

fn apply_contract_hint_locator_defaults(
    output_contract: &mut IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
) {
    match output_contract.semantic_kind {
        OutputSemanticKind::RawCommandOutput
        | OutputSemanticKind::CommandOutputSummary
        | OutputSemanticKind::ServiceStatus => {
            output_contract.locator_kind = OutputLocatorKind::None;
            output_contract.locator_hint.clear();
        }
        OutputSemanticKind::GitCommitSubject
        | OutputSemanticKind::GitRepositoryState
        | OutputSemanticKind::HiddenEntriesCheck
        | OutputSemanticKind::RecentScalarEqualityCheck => {
            output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
            output_contract.locator_hint = workspace_root.display().to_string();
        }
        OutputSemanticKind::WorkspaceProjectSummary => {
            apply_path_locator_defaults_for_contract_hint(
                output_contract,
                req,
                req_surface,
                workspace_root,
            );
        }
        _ => apply_path_locator_defaults_for_contract_hint(
            output_contract,
            req,
            req_surface,
            workspace_root,
        ),
    }
}

fn apply_path_locator_defaults_for_contract_hint(
    output_contract: &mut IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
) {
    if matches!(
        output_contract.semantic_kind,
        OutputSemanticKind::ArchivePack | OutputSemanticKind::ArchiveUnpack
    ) {
        if let Some((semantic_kind, locator_hint)) =
            archive_pair_contract_from_surface(output_contract, req_surface)
        {
            output_contract.semantic_kind = semantic_kind;
            output_contract.locator_kind = OutputLocatorKind::Path;
            output_contract.locator_hint = locator_hint;
            return;
        }
    }
    if output_contract.semantic_kind_is(OutputSemanticKind::ArchiveRead) {
        if let Some(locator_hint) = archive_read_contract_from_surface(output_contract, req_surface)
        {
            output_contract.locator_kind = OutputLocatorKind::Path;
            output_contract.locator_hint = locator_hint;
            return;
        }
    }
    if output_contract.semantic_kind_is_any(&[
        OutputSemanticKind::ArchiveList,
        OutputSemanticKind::ArchiveUnpack,
    ]) {
        if let Some(locator_hint) = archive_list_contract_from_surface(output_contract, req_surface)
        {
            output_contract.semantic_kind = OutputSemanticKind::ArchiveList;
            output_contract.locator_kind = OutputLocatorKind::Path;
            output_contract.locator_hint = locator_hint;
            return;
        }
    }
    if output_contract.semantic_kind_is(OutputSemanticKind::QuantityComparison) {
        if let Some((left, right)) = req_surface.locator_target_pair.as_ref() {
            output_contract.locator_kind = OutputLocatorKind::Path;
            output_contract.locator_hint = format!("{} | {}", left.trim(), right.trim());
            return;
        }
        let targets = explicit_surface_path_fact_targets(req_surface);
        if targets.len() >= 2 {
            output_contract.locator_kind = OutputLocatorKind::Path;
            output_contract.locator_hint = format!("{} | {}", targets[0].trim(), targets[1].trim());
            return;
        }
    }
    if let Some(locator) =
        crate::intent::locator_extractor::extract_explicit_locator_for_fallback(req)
    {
        output_contract.locator_kind = locator.locator_kind;
        output_contract.locator_hint = locator.locator_hint;
        return;
    }
    let filename_candidates = req_surface.filename_candidates_excluding_field_selectors();
    if filename_candidates.len() == 1 {
        output_contract.locator_kind = OutputLocatorKind::Filename;
        output_contract.locator_hint = filename_candidates[0].clone();
        return;
    }
    if !filename_candidates.is_empty() {
        output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        output_contract.locator_hint = workspace_root.display().to_string();
        return;
    }
    if output_contract.requires_content_evidence {
        output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        output_contract.locator_hint = workspace_root.display().to_string();
    }
}
