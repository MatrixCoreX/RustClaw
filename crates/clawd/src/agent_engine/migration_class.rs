use crate::{OutputLocatorKind, OutputResponseShape, OutputSemanticKind, RouteResult};

pub(crate) fn agent_decides_eligible_migration_class(route: &RouteResult) -> &'static str {
    let contract = crate::TaskContract::from_route_result(route);
    if !matches!(
        contract.intent_kind,
        crate::task_contract::TaskIntentKind::PlannerExecute
    ) || route.needs_clarify
        || route.wants_file_delivery
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
    {
        return "none";
    }

    let has_bound_locator = route_has_bound_locator(route);
    if route.output_contract.response_shape == OutputResponseShape::Scalar
        && has_bound_locator
        && !matches!(
            route.output_contract.semantic_kind,
            OutputSemanticKind::ScalarCount
        )
    {
        "structured_field_read"
    } else if matches!(
        route.output_contract.semantic_kind,
        OutputSemanticKind::FilePaths
            | OutputSemanticKind::FileNames
            | OutputSemanticKind::DirectoryNames
            | OutputSemanticKind::DirectoryEntryGroups
            | OutputSemanticKind::HiddenEntriesCheck
    ) && has_bound_locator
    {
        "exact_path_list"
    } else if matches!(
        route.output_contract.semantic_kind,
        OutputSemanticKind::ContentExcerptSummary
            | OutputSemanticKind::ContentExcerptWithSummary
            | OutputSemanticKind::DirectoryPurposeSummary
            | OutputSemanticKind::WorkspaceProjectSummary
    ) && has_bound_locator
        && matches!(
            route.output_contract.response_shape,
            OutputResponseShape::Free
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Strict
        )
    {
        "bound_path_summary"
    } else if route.output_contract.semantic_kind == OutputSemanticKind::RecentArtifactsJudgment {
        "recent_artifacts_judgment"
    } else if route.output_contract.semantic_kind == OutputSemanticKind::ScalarCount
        && route.output_contract.response_shape == OutputResponseShape::Scalar
        && has_bound_locator
    {
        "scalar_count"
    } else {
        "none"
    }
}

fn route_has_bound_locator(route: &RouteResult) -> bool {
    match route.output_contract.locator_kind {
        OutputLocatorKind::CurrentWorkspace => true,
        OutputLocatorKind::Path | OutputLocatorKind::Filename | OutputLocatorKind::Url => {
            !route.output_contract.locator_hint.trim().is_empty()
        }
        OutputLocatorKind::None => false,
    }
}

#[cfg(test)]
#[path = "migration_class_tests.rs"]
mod tests;
