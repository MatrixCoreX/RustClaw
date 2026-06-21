use crate::{
    OutputLocatorKind, OutputResponseShape, OutputSemanticKind, RiskCeiling, RouteResult,
    ScheduleKind,
};

use super::planning_route_markers::route_has_unresolved_clarify_or_locator_marker;

const LOW_RISK_READ_BOUNDARY_REQUIREMENTS: &[&str] = &[
    "planner_execute",
    "non_high_risk",
    "no_schedule",
    "no_clarify",
    "no_delivery",
    "evidence_required",
];

const LOW_RISK_CONTEXT_BOUNDARY_REQUIREMENTS: &[&str] = &[
    "planner_execute",
    "non_high_risk",
    "no_schedule",
    "no_clarify",
    "no_delivery",
    "planner_context_available",
];

const LOW_RISK_DIRECT_RESPONSE_BOUNDARY_REQUIREMENTS: &[&str] = &[
    "planner_execute",
    "non_high_risk",
    "no_schedule",
    "no_clarify",
    "no_delivery",
    "no_external_evidence_required",
];

const LOW_RISK_SINGLE_FILE_DELIVERY_BOUNDARY_REQUIREMENTS: &[&str] = &[
    "planner_execute",
    "non_high_risk",
    "no_schedule",
    "no_clarify",
    "delivery_required",
    "file_token_delivery",
    "single_file_delivery",
    "evidence_required",
    "bound_locator_or_selector",
    "delivery_consistency_gate",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentLoopEligibilityBucket {
    LowRiskStructuredRead,
    LowRiskListing,
    LowRiskGroundedSummary,
    LowRiskMetadataJudgment,
    LowRiskScalarObservation,
    LowRiskStatusObservation,
    LowRiskConfigRead,
    LowRiskLogObservation,
    LowRiskWorkspaceQuestion,
    LowRiskToolDiscovery,
    LowRiskDirectResponse,
    LowRiskSingleFileDelivery,
}

impl AgentLoopEligibilityBucket {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::LowRiskStructuredRead => "low_risk_structured_read",
            Self::LowRiskListing => "low_risk_listing",
            Self::LowRiskGroundedSummary => "low_risk_grounded_summary",
            Self::LowRiskMetadataJudgment => "low_risk_metadata_judgment",
            Self::LowRiskScalarObservation => "low_risk_scalar_observation",
            Self::LowRiskStatusObservation => "low_risk_status_observation",
            Self::LowRiskConfigRead => "low_risk_config_read",
            Self::LowRiskLogObservation => "low_risk_log_observation",
            Self::LowRiskWorkspaceQuestion => "low_risk_workspace_question",
            Self::LowRiskToolDiscovery => "low_risk_tool_discovery",
            Self::LowRiskDirectResponse => "low_risk_direct_response",
            Self::LowRiskSingleFileDelivery => "low_risk_single_file_delivery",
        }
    }

    fn compatibility_migration_class(self) -> &'static str {
        match self {
            Self::LowRiskStructuredRead => "structured_field_read",
            Self::LowRiskListing => "exact_path_list",
            Self::LowRiskGroundedSummary => "bound_path_summary",
            Self::LowRiskMetadataJudgment => "recent_artifacts_judgment",
            Self::LowRiskScalarObservation => "scalar_count",
            Self::LowRiskStatusObservation
            | Self::LowRiskConfigRead
            | Self::LowRiskLogObservation
            | Self::LowRiskWorkspaceQuestion
            | Self::LowRiskToolDiscovery
            | Self::LowRiskDirectResponse
            | Self::LowRiskSingleFileDelivery => self.as_str(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AgentLoopEligibility {
    pub(crate) eligible: bool,
    pub(crate) bucket: Option<AgentLoopEligibilityBucket>,
    pub(crate) blocked_reason: &'static str,
    pub(crate) boundary_requirements: &'static [&'static str],
}

impl AgentLoopEligibility {
    fn eligible(bucket: AgentLoopEligibilityBucket) -> Self {
        Self::eligible_with_requirements(bucket, LOW_RISK_READ_BOUNDARY_REQUIREMENTS)
    }

    fn eligible_with_requirements(
        bucket: AgentLoopEligibilityBucket,
        boundary_requirements: &'static [&'static str],
    ) -> Self {
        Self {
            eligible: true,
            bucket: Some(bucket),
            blocked_reason: "none",
            boundary_requirements,
        }
    }

    fn blocked(blocked_reason: &'static str) -> Self {
        Self {
            eligible: false,
            bucket: None,
            blocked_reason,
            boundary_requirements: &[],
        }
    }

    pub(crate) fn bucket_token(self) -> &'static str {
        self.bucket
            .map(AgentLoopEligibilityBucket::as_str)
            .unwrap_or("none")
    }

    pub(crate) fn compatibility_migration_class(self) -> &'static str {
        self.bucket
            .map(AgentLoopEligibilityBucket::compatibility_migration_class)
            .unwrap_or("none")
    }
}

pub(crate) fn agent_decides_eligible_migration_class(route: &RouteResult) -> &'static str {
    agent_loop_eligibility(route).compatibility_migration_class()
}

pub(crate) fn agent_loop_eligibility(route: &RouteResult) -> AgentLoopEligibility {
    let contract = crate::TaskContract::from_route_result(route);
    if !matches!(
        contract.intent_kind,
        crate::task_contract::TaskIntentKind::PlannerExecute
    ) {
        return AgentLoopEligibility::blocked("not_planner_execute");
    }
    if route.risk_ceiling == RiskCeiling::High {
        return AgentLoopEligibility::blocked("risk_ceiling_high");
    }
    if route.schedule_kind != ScheduleKind::None {
        return AgentLoopEligibility::blocked("schedule_active");
    }
    if route_has_unresolved_clarify_or_locator_marker(route) {
        return AgentLoopEligibility::blocked("unresolved_clarify_or_locator");
    }
    if route_is_low_risk_single_file_delivery(route, &contract) {
        return AgentLoopEligibility::eligible_with_requirements(
            AgentLoopEligibilityBucket::LowRiskSingleFileDelivery,
            LOW_RISK_SINGLE_FILE_DELIVERY_BOUNDARY_REQUIREMENTS,
        );
    }
    if route.wants_file_delivery || route.output_contract.delivery_required {
        return AgentLoopEligibility::blocked("delivery_required");
    }
    if route.output_contract.semantic_kind == OutputSemanticKind::ToolDiscovery {
        return AgentLoopEligibility::eligible_with_requirements(
            AgentLoopEligibilityBucket::LowRiskToolDiscovery,
            LOW_RISK_CONTEXT_BOUNDARY_REQUIREMENTS,
        );
    }
    if route_is_low_risk_direct_response(route) {
        return AgentLoopEligibility::eligible_with_requirements(
            AgentLoopEligibilityBucket::LowRiskDirectResponse,
            LOW_RISK_DIRECT_RESPONSE_BOUNDARY_REQUIREMENTS,
        );
    }
    if !contract.evidence_required {
        return AgentLoopEligibility::blocked("evidence_not_required");
    }
    if !contract.missing_parameters.is_empty() {
        return AgentLoopEligibility::blocked("missing_parameters");
    }
    if matches!(
        contract.operation,
        crate::task_contract::TaskOperation::Write | crate::task_contract::TaskOperation::Modify
    ) {
        return AgentLoopEligibility::blocked("side_effect_operation");
    }

    let has_bound_locator = route_has_bound_locator(route);
    if route.output_contract.semantic_kind == OutputSemanticKind::ServiceStatus {
        AgentLoopEligibility::eligible(AgentLoopEligibilityBucket::LowRiskStatusObservation)
    } else if matches!(
        route.output_contract.semantic_kind,
        OutputSemanticKind::PackageManagerDetection
    ) {
        AgentLoopEligibility::eligible(AgentLoopEligibilityBucket::LowRiskStatusObservation)
    } else if matches!(
        route.output_contract.semantic_kind,
        OutputSemanticKind::StructuredKeys
            | OutputSemanticKind::ConfigValidation
            | OutputSemanticKind::ConfigRiskAssessment
    ) {
        AgentLoopEligibility::eligible(AgentLoopEligibilityBucket::LowRiskConfigRead)
    } else if route.output_contract.semantic_kind == OutputSemanticKind::DockerLogs {
        AgentLoopEligibility::eligible(AgentLoopEligibilityBucket::LowRiskLogObservation)
    } else if route.output_contract.response_shape == OutputResponseShape::Scalar
        && has_bound_locator
        && !matches!(
            route.output_contract.semantic_kind,
            OutputSemanticKind::ScalarCount
        )
    {
        AgentLoopEligibility::eligible(AgentLoopEligibilityBucket::LowRiskStructuredRead)
    } else if matches!(
        route.output_contract.semantic_kind,
        OutputSemanticKind::FilePaths
            | OutputSemanticKind::FileNames
            | OutputSemanticKind::DirectoryNames
            | OutputSemanticKind::DirectoryEntryGroups
            | OutputSemanticKind::HiddenEntriesCheck
    ) && has_bound_locator
    {
        AgentLoopEligibility::eligible(AgentLoopEligibilityBucket::LowRiskListing)
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
        AgentLoopEligibility::eligible(AgentLoopEligibilityBucket::LowRiskGroundedSummary)
    } else if route.output_contract.semantic_kind == OutputSemanticKind::RecentArtifactsJudgment {
        AgentLoopEligibility::eligible(AgentLoopEligibilityBucket::LowRiskMetadataJudgment)
    } else if route.output_contract.semantic_kind == OutputSemanticKind::ScalarCount
        && route.output_contract.response_shape == OutputResponseShape::Scalar
        && has_bound_locator
    {
        AgentLoopEligibility::eligible(AgentLoopEligibilityBucket::LowRiskScalarObservation)
    } else if route.output_contract.semantic_kind == OutputSemanticKind::None
        && route.output_contract.locator_kind == OutputLocatorKind::CurrentWorkspace
        && matches!(
            route.output_contract.response_shape,
            OutputResponseShape::Free
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Strict
        )
    {
        AgentLoopEligibility::eligible(AgentLoopEligibilityBucket::LowRiskWorkspaceQuestion)
    } else {
        AgentLoopEligibility::blocked("unsupported_contract")
    }
}

fn route_is_low_risk_single_file_delivery(
    route: &RouteResult,
    contract: &crate::TaskContract,
) -> bool {
    if !(route.wants_file_delivery || route.output_contract.delivery_required) {
        return false;
    }
    if matches!(
        contract.operation,
        crate::task_contract::TaskOperation::Write | crate::task_contract::TaskOperation::Modify
    ) {
        return false;
    }
    if route.output_contract.semantic_kind == OutputSemanticKind::GeneratedFileDelivery {
        return false;
    }
    if route.output_contract.response_shape != OutputResponseShape::FileToken
        || !route.output_contract.delivery_required
        || route.output_contract.delivery_intent != crate::OutputDeliveryIntent::FileSingle
        || !route.output_contract.requires_content_evidence
    {
        return false;
    }
    if !route_has_delivery_locator_scope(route) {
        return false;
    }
    let selector = &route.output_contract.self_extension.list_selector;
    let selector_is_bounded_single_file = selector.target_kind_specified
        && selector.target_kind == crate::OutputScalarCountTargetKind::File
        && selector.limit == Some(1);
    selector_is_bounded_single_file || !route.output_contract.locator_hint.trim().is_empty()
}

fn route_has_delivery_locator_scope(route: &RouteResult) -> bool {
    match route.output_contract.locator_kind {
        OutputLocatorKind::Path | OutputLocatorKind::Filename => {
            !route.output_contract.locator_hint.trim().is_empty()
        }
        OutputLocatorKind::CurrentWorkspace => {
            let selector = &route.output_contract.self_extension.list_selector;
            selector.target_kind_specified
                && selector.target_kind == crate::OutputScalarCountTargetKind::File
                && selector.limit == Some(1)
        }
        OutputLocatorKind::None | OutputLocatorKind::Url => false,
    }
}

fn route_is_low_risk_direct_response(route: &RouteResult) -> bool {
    route.output_contract.semantic_kind == OutputSemanticKind::None
        && !route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && !route.wants_file_delivery
        && route.output_contract.locator_kind == OutputLocatorKind::None
        && route.output_contract.locator_hint.trim().is_empty()
        && matches!(
            route.output_contract.response_shape,
            OutputResponseShape::Free
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Strict
        )
        && matches!(
            route.output_contract.self_extension.mode,
            crate::SelfExtensionMode::None
        )
        && matches!(
            route.output_contract.self_extension.trigger,
            crate::SelfExtensionTrigger::None
        )
        && !route.output_contract.self_extension.execute_now
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
