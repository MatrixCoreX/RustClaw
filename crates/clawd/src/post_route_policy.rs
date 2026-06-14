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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PostRoutePolicyOutcome {
    #[default]
    NoChange,
    Clarify,
    Execute,
    RefineContract,
}

impl PostRoutePolicyOutcome {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NoChange => "no_change",
            Self::Clarify => "clarify",
            Self::Execute => "execute",
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
            owner_layer: "post_route_policy",
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
    pub(crate) gate_record: PostRouteGateRecord,
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
            OutputLocatorKind::Path
                | OutputLocatorKind::Filename
                | OutputLocatorKind::CurrentWorkspace
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

fn semantic_requires_database_file_locator(kind: OutputSemanticKind) -> bool {
    matches!(
        kind,
        OutputSemanticKind::SqliteTableListing
            | OutputSemanticKind::SqliteTableNamesOnly
            | OutputSemanticKind::SqliteDatabaseKindJudgment
            | OutputSemanticKind::SqliteSchemaVersion
    )
}

fn direct_locator_path_is_unsuitable_for_contract(route_result: &RouteResult, path: &str) -> bool {
    semantic_requires_database_file_locator(route_result.output_contract.semantic_kind)
        && path_is_existing_directory(path)
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
        && !scalar_count_contract_allows_count_shape(&route_result.output_contract)
}

fn scalar_count_contract_allows_count_shape(contract: &IntentOutputContract) -> bool {
    matches!(
        contract.response_shape,
        OutputResponseShape::Scalar | OutputResponseShape::OneSentence
    ) || (contract.response_shape == OutputResponseShape::Strict
        && contract.exact_sentence_count == Some(1))
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

fn path_without_parent_components(path: &Path) -> bool {
    !path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
}

fn relative_locator_hint_is_specific_path(path: &Path) -> bool {
    path_without_parent_components(path)
        && path
            .components()
            .filter(|component| {
                matches!(
                    component,
                    std::path::Component::Normal(_) | std::path::Component::Prefix(_)
                )
            })
            .count()
            >= 2
}

fn normalize_path_for_identity(path: &Path) -> std::path::PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn locator_hint_matches_direct_locator(
    route_result: &RouteResult,
    direct_locator_path: Option<&str>,
) -> bool {
    let hint = route_result.output_contract.locator_hint.trim();
    let Some(direct_locator_path) = direct_locator_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    if hint.is_empty() || hint.contains('\n') {
        return false;
    }
    let hint_path = Path::new(hint);
    let direct_path = Path::new(direct_locator_path);
    if !path_without_parent_components(hint_path) {
        return false;
    }
    if hint_path.is_absolute() {
        return normalize_path_for_identity(hint_path) == normalize_path_for_identity(direct_path);
    }
    relative_locator_hint_is_specific_path(hint_path)
        && (direct_path.ends_with(hint_path)
            || hint_path
                .canonicalize()
                .is_ok_and(|hint| hint == normalize_path_for_identity(direct_path)))
}

fn direct_auto_locator_can_satisfy_background_clarify(
    route_result: &RouteResult,
    direct_locator_path: Option<&str>,
) -> bool {
    if !route_reason_has_marker(route_result, "clarify_reason_code:missing_read_target") {
        return true;
    }
    if route_result.output_contract.semantic_kind == OutputSemanticKind::FilesystemMutationResult {
        return filesystem_mutation_locator_can_satisfy_missing_read_target(
            route_result,
            direct_locator_path,
        );
    }
    if route_result.output_contract.semantic_kind == OutputSemanticKind::ArchiveUnpack {
        return archive_locator_can_satisfy_missing_read_target(route_result, direct_locator_path);
    }
    if matches!(
        route_result.output_contract.semantic_kind,
        OutputSemanticKind::ContentExcerptSummary
            | OutputSemanticKind::ContentExcerptWithSummary
            | OutputSemanticKind::ContentPresenceCheck
            | OutputSemanticKind::DocumentHeading
            | OutputSemanticKind::ExcerptKindJudgment
    ) {
        return locator_hint_matches_direct_locator(route_result, direct_locator_path);
    }
    true
}

fn filesystem_mutation_locator_can_satisfy_missing_read_target(
    route_result: &RouteResult,
    direct_locator_path: Option<&str>,
) -> bool {
    let Some(path) = direct_locator_path else {
        return false;
    };
    if !locator_hint_matches_direct_locator(route_result, Some(path)) {
        return false;
    }
    if route_result
        .output_contract
        .self_extension
        .structured_field_selector
        .is_some()
    {
        return true;
    }
    !path_is_existing_directory(path)
}

fn archive_locator_can_satisfy_missing_read_target(
    route_result: &RouteResult,
    direct_locator_path: Option<&str>,
) -> bool {
    let Some(path) = direct_locator_path else {
        return false;
    };
    locator_hint_matches_direct_locator(route_result, Some(path)) && path_looks_like_archive(path)
}

fn path_looks_like_archive(path: &str) -> bool {
    let lower = path.trim().to_ascii_lowercase();
    lower.ends_with(".zip")
        || lower.ends_with(".tar")
        || lower.ends_with(".tar.gz")
        || lower.ends_with(".tgz")
        || lower.ends_with(".tar.bz2")
        || lower.ends_with(".tbz2")
        || lower.ends_with(".tar.xz")
        || lower.ends_with(".txz")
        || lower.ends_with(".gz")
        || lower.ends_with(".bz2")
        || lower.ends_with(".xz")
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

    let cleared_scalar_count_semantic =
        should_clear_scalar_count_for_non_scalar_contract(&execution_route_result);
    if cleared_scalar_count_semantic {
        execution_route_result.output_contract.semantic_kind = OutputSemanticKind::None;
    }
    let cleared_scalar_path_only_semantic =
        should_clear_scalar_path_only_without_locator_binding(&execution_route_result);
    if cleared_scalar_path_only_semantic {
        execution_route_result.output_contract.semantic_kind = OutputSemanticKind::None;
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

    let background_locator_clarify = route_reason_has_marker(
        &execution_route_result,
        "background_locator_requires_clarify",
    );
    let deictic_bare_locator_clarify = route_reason_has_marker(
        &execution_route_result,
        "deictic_bare_locator_requires_clarify",
    );
    let direct_locator_matches_hint =
        locator_hint_matches_direct_locator(&execution_route_result, auto_locator_path.as_deref());
    let direct_auto_locator_can_satisfy_background_clarify =
        direct_auto_locator_can_satisfy_background_clarify(
            &execution_route_result,
            auto_locator_path.as_deref(),
        );
    let direct_auto_locator_satisfies_background_clarify = auto_locator_resolved_direct
        && path_scoped_content_request
        && (!deictic_bare_locator_clarify || direct_locator_matches_hint)
        && direct_auto_locator_can_satisfy_background_clarify;
    if direct_auto_locator_satisfies_background_clarify {
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
        || fuzzy_locator_requires_clarify
        || (background_locator_clarify && !direct_auto_locator_satisfies_background_clarify);
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
    let gate_record = if missing_locator_for_path_scoped_content {
        PostRouteGateRecord::new(
            "post_route_missing_path_scoped_locator",
            PostRoutePolicyOutcome::Clarify,
        )
    } else if !fuzzy_locator_suggestions.is_empty() {
        PostRouteGateRecord::new(
            "post_route_fuzzy_locator_candidates",
            PostRoutePolicyOutcome::Clarify,
        )
    } else if background_locator_clarify && direct_auto_locator_satisfies_background_clarify {
        PostRouteGateRecord::new(
            "post_route_auto_locator_satisfied_background_clarify",
            PostRoutePolicyOutcome::Execute,
        )
    } else if direct_auto_locator_satisfies_background_clarify {
        PostRouteGateRecord::new(
            "post_route_auto_locator_satisfied_path_scoped_content",
            PostRoutePolicyOutcome::Execute,
        )
    } else if force_clarify {
        PostRouteGateRecord::new(
            "post_route_upstream_clarify_required",
            PostRoutePolicyOutcome::Clarify,
        )
    } else if forced_content_evidence
        || cleared_scalar_count_semantic
        || cleared_scalar_path_only_semantic
    {
        PostRouteGateRecord::new(
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
        clarify_reason,
        clarify_reason_kind,
        gate_record,
    }
}

#[cfg(test)]
#[path = "post_route_policy_tests.rs"]
mod tests;
