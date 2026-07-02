use super::{
    agent_decides_eligible_migration_class, agent_loop_eligibility, AgentLoopEligibilityBucket,
};
use crate::{
    AskMode, IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputScalarCountTargetKind, OutputSemanticKind, ResumeBehavior, RiskCeiling, RouteResult,
    ScheduleKind,
};

fn route_result(shape: OutputResponseShape, kind: OutputSemanticKind) -> RouteResult {
    RouteResult {
        ask_mode: AskMode::planner_execute_with_chat_finalizer(),
        resolved_intent: "test".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Low,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: shape,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: kind,
            locator_hint: "docs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    }
}

#[test]
fn exact_path_list_accepts_path_and_inventory_contracts() {
    for kind in [
        OutputSemanticKind::FilePaths,
        OutputSemanticKind::FileNames,
        OutputSemanticKind::DirectoryNames,
        OutputSemanticKind::DirectoryEntryGroups,
        OutputSemanticKind::HiddenEntriesCheck,
    ] {
        let route = route_result(OutputResponseShape::Strict, kind);

        assert_eq!(
            agent_decides_eligible_migration_class(&route),
            "exact_path_list"
        );
    }
}

#[test]
fn exact_path_list_requires_bound_locator_and_task_contract_evidence() {
    let mut route = route_result(OutputResponseShape::Strict, OutputSemanticKind::FileNames);
    route.output_contract.locator_hint.clear();
    assert_eq!(agent_decides_eligible_migration_class(&route), "none");

    route.output_contract.locator_hint = "docs".to_string();
    route.output_contract.requires_content_evidence = false;
    assert_eq!(
        agent_decides_eligible_migration_class(&route),
        "exact_path_list"
    );

    route.output_contract.semantic_kind = OutputSemanticKind::None;
    assert_eq!(agent_decides_eligible_migration_class(&route), "none");
}

#[test]
fn bound_path_summary_accepts_grounded_summary_contracts() {
    for (shape, kind) in [
        (
            OutputResponseShape::Free,
            OutputSemanticKind::ContentExcerptSummary,
        ),
        (
            OutputResponseShape::OneSentence,
            OutputSemanticKind::ContentExcerptWithSummary,
        ),
        (
            OutputResponseShape::Strict,
            OutputSemanticKind::DirectoryPurposeSummary,
        ),
        (
            OutputResponseShape::Free,
            OutputSemanticKind::WorkspaceProjectSummary,
        ),
    ] {
        let route = route_result(shape, kind);

        assert_eq!(
            agent_decides_eligible_migration_class(&route),
            "bound_path_summary"
        );
    }
}

#[test]
fn bound_path_summary_requires_bound_locator_task_contract_evidence_and_summary_shape() {
    let mut route = route_result(
        OutputResponseShape::Free,
        OutputSemanticKind::ContentExcerptSummary,
    );
    route.output_contract.locator_hint.clear();
    assert_eq!(agent_decides_eligible_migration_class(&route), "none");

    route.output_contract.locator_hint = "docs".to_string();
    route.output_contract.requires_content_evidence = false;
    assert_eq!(
        agent_decides_eligible_migration_class(&route),
        "bound_path_summary"
    );

    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    assert_ne!(
        agent_decides_eligible_migration_class(&route),
        "bound_path_summary"
    );
}

#[test]
fn eligibility_maps_legacy_classes_to_general_buckets() {
    for (shape, kind, expected_class, expected_bucket) in [
        (
            OutputResponseShape::Scalar,
            OutputSemanticKind::ScalarPathOnly,
            "structured_field_read",
            AgentLoopEligibilityBucket::LowRiskStructuredRead,
        ),
        (
            OutputResponseShape::Strict,
            OutputSemanticKind::FileNames,
            "exact_path_list",
            AgentLoopEligibilityBucket::LowRiskListing,
        ),
        (
            OutputResponseShape::Free,
            OutputSemanticKind::ContentExcerptSummary,
            "bound_path_summary",
            AgentLoopEligibilityBucket::LowRiskGroundedSummary,
        ),
        (
            OutputResponseShape::OneSentence,
            OutputSemanticKind::RecentArtifactsJudgment,
            "recent_artifacts_judgment",
            AgentLoopEligibilityBucket::LowRiskMetadataJudgment,
        ),
        (
            OutputResponseShape::Scalar,
            OutputSemanticKind::ScalarCount,
            "scalar_count",
            AgentLoopEligibilityBucket::LowRiskScalarObservation,
        ),
    ] {
        let route = route_result(shape, kind);
        let eligibility = agent_loop_eligibility(&route);

        assert!(eligibility.eligible);
        assert_eq!(eligibility.bucket, Some(expected_bucket));
        assert_eq!(eligibility.compatibility_migration_class(), expected_class);
    }
}

#[test]
fn eligibility_uses_contract_not_legacy_route_trace() {
    let mut listing = route_result(OutputResponseShape::Strict, OutputSemanticKind::FileNames);
    listing.ask_mode = AskMode::direct_answer();

    let eligibility = agent_loop_eligibility(&listing);

    assert!(eligibility.eligible);
    assert_eq!(
        eligibility.bucket,
        Some(AgentLoopEligibilityBucket::LowRiskListing)
    );
    assert_eq!(
        eligibility.compatibility_migration_class(),
        "exact_path_list"
    );
    assert!(eligibility
        .boundary_requirements
        .contains(&"agent_loop_entry"));
    assert!(eligibility
        .boundary_requirements
        .contains(&"loop_owned_clarify"));
    assert!(!eligibility
        .boundary_requirements
        .contains(&"planner_execute"));
    assert!(!eligibility.boundary_requirements.contains(&"no_clarify"));
}

#[test]
fn unresolved_locator_marker_stays_loop_owned_for_selected_evidence_guard() {
    let mut route = route_result(
        OutputResponseShape::Scalar,
        OutputSemanticKind::FileBasename,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "package.json".to_string();
    route.route_reason =
        "state_patch.deictic_reference=missing_locator; clarify_reason_code:missing_read_target"
            .to_string();

    let eligibility = agent_loop_eligibility(&route);

    assert!(eligibility.eligible);
    assert_eq!(
        eligibility.bucket,
        Some(AgentLoopEligibilityBucket::LowRiskStructuredRead)
    );
    assert_eq!(
        eligibility.compatibility_migration_class(),
        "structured_field_read"
    );
    assert!(eligibility
        .boundary_requirements
        .contains(&"loop_owned_clarify"));
}

#[test]
fn eligibility_adds_generic_low_risk_buckets() {
    for (shape, kind, expected_class, expected_bucket) in [
        (
            OutputResponseShape::Strict,
            OutputSemanticKind::ServiceStatus,
            "low_risk_status_observation",
            AgentLoopEligibilityBucket::LowRiskStatusObservation,
        ),
        (
            OutputResponseShape::Strict,
            OutputSemanticKind::StructuredKeys,
            "low_risk_config_read",
            AgentLoopEligibilityBucket::LowRiskConfigRead,
        ),
    ] {
        let route = route_result(shape, kind);
        let eligibility = agent_loop_eligibility(&route);

        assert!(eligibility.eligible);
        assert_eq!(eligibility.bucket, Some(expected_bucket));
        assert_eq!(eligibility.compatibility_migration_class(), expected_class);
    }

    let legacy_docker_logs =
        route_result(OutputResponseShape::Strict, OutputSemanticKind::DockerLogs);
    let eligibility = agent_loop_eligibility(&legacy_docker_logs);
    assert!(!eligibility.eligible);
    assert_eq!(eligibility.compatibility_migration_class(), "none");

    for marker in [
        "capability_ref=package.detect_manager",
        "capability_ref=docker.list_containers",
        "capability_ref=docker.list_images",
        "capability_ref=docker.version",
        "capability_ref=docker.version_extra",
    ] {
        let mut route = route_result(OutputResponseShape::Strict, OutputSemanticKind::None);
        route.resolved_intent = marker.to_string();
        route.output_contract.locator_kind = OutputLocatorKind::None;
        route.output_contract.locator_hint.clear();
        let eligibility = agent_loop_eligibility(&route);

        assert!(eligibility.eligible, "{marker} should be eligible");
        assert_eq!(
            eligibility.bucket,
            Some(AgentLoopEligibilityBucket::LowRiskStatusObservation)
        );
        assert_eq!(
            eligibility.compatibility_migration_class(),
            "low_risk_status_observation"
        );
    }

    let mut docker_logs = route_result(OutputResponseShape::Strict, OutputSemanticKind::None);
    docker_logs.resolved_intent = "capability_ref=docker.read_logs".to_string();
    docker_logs.output_contract.locator_kind = OutputLocatorKind::None;
    docker_logs.output_contract.locator_hint.clear();
    let eligibility = agent_loop_eligibility(&docker_logs);
    assert!(eligibility.eligible);
    assert_eq!(
        eligibility.bucket,
        Some(AgentLoopEligibilityBucket::LowRiskLogObservation)
    );
    assert_eq!(
        eligibility.compatibility_migration_class(),
        "low_risk_log_observation"
    );

    let mut suffix = route_result(OutputResponseShape::Strict, OutputSemanticKind::None);
    suffix.resolved_intent = "capability_ref=dockerversion".to_string();
    suffix.output_contract.locator_kind = OutputLocatorKind::None;
    suffix.output_contract.locator_hint.clear();
    let eligibility = agent_loop_eligibility(&suffix);
    assert!(!eligibility.eligible);
    assert_eq!(eligibility.compatibility_migration_class(), "none");

    let mut workspace = route_result(OutputResponseShape::Free, OutputSemanticKind::None);
    workspace.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    workspace.output_contract.locator_hint.clear();
    let eligibility = agent_loop_eligibility(&workspace);
    assert!(eligibility.eligible);
    assert_eq!(
        eligibility.bucket,
        Some(AgentLoopEligibilityBucket::LowRiskWorkspaceQuestion)
    );
    assert_eq!(
        eligibility.compatibility_migration_class(),
        "low_risk_workspace_question"
    );

    let mut tool_discovery =
        route_result(OutputResponseShape::Free, OutputSemanticKind::ToolDiscovery);
    tool_discovery.output_contract.requires_content_evidence = false;
    tool_discovery.output_contract.locator_kind = OutputLocatorKind::None;
    tool_discovery.output_contract.locator_hint.clear();
    let eligibility = agent_loop_eligibility(&tool_discovery);
    assert!(eligibility.eligible);
    assert_eq!(
        eligibility.bucket,
        Some(AgentLoopEligibilityBucket::LowRiskToolDiscovery)
    );
    assert_eq!(
        eligibility.compatibility_migration_class(),
        "low_risk_tool_discovery"
    );
    assert!(eligibility
        .boundary_requirements
        .contains(&"planner_context_available"));

    let mut direct_response =
        route_result(OutputResponseShape::OneSentence, OutputSemanticKind::None);
    direct_response.output_contract.requires_content_evidence = false;
    direct_response.output_contract.locator_kind = OutputLocatorKind::None;
    direct_response.output_contract.locator_hint.clear();
    let eligibility = agent_loop_eligibility(&direct_response);
    assert!(eligibility.eligible);
    assert_eq!(
        eligibility.bucket,
        Some(AgentLoopEligibilityBucket::LowRiskDirectResponse)
    );
    assert_eq!(
        eligibility.compatibility_migration_class(),
        "low_risk_direct_response"
    );
    assert!(eligibility
        .boundary_requirements
        .contains(&"no_external_evidence_required"));
}

#[test]
fn eligibility_accepts_machine_markers_without_semantic_kind() {
    let mut listing = route_result(OutputResponseShape::Strict, OutputSemanticKind::None);
    listing.route_reason = "file_names".to_string();
    let eligibility = agent_loop_eligibility(&listing);
    assert!(eligibility.eligible);
    assert_eq!(
        eligibility.bucket,
        Some(AgentLoopEligibilityBucket::LowRiskListing)
    );

    let mut summary = route_result(OutputResponseShape::Free, OutputSemanticKind::None);
    summary.route_reason = "content_excerpt_summary".to_string();
    let eligibility = agent_loop_eligibility(&summary);
    assert!(eligibility.eligible);
    assert_eq!(
        eligibility.bucket,
        Some(AgentLoopEligibilityBucket::LowRiskGroundedSummary)
    );

    let mut scalar = route_result(OutputResponseShape::Scalar, OutputSemanticKind::None);
    scalar.route_reason = "scalar_count".to_string();
    let eligibility = agent_loop_eligibility(&scalar);
    assert!(eligibility.eligible);
    assert_eq!(
        eligibility.bucket,
        Some(AgentLoopEligibilityBucket::LowRiskScalarObservation)
    );

    let mut config = route_result(OutputResponseShape::Strict, OutputSemanticKind::None);
    config.route_reason = "config_validation".to_string();
    let eligibility = agent_loop_eligibility(&config);
    assert!(eligibility.eligible);
    assert_eq!(
        eligibility.bucket,
        Some(AgentLoopEligibilityBucket::LowRiskConfigRead)
    );

    let mut tool_discovery = route_result(OutputResponseShape::Free, OutputSemanticKind::None);
    tool_discovery.route_reason = "tool_discovery".to_string();
    tool_discovery.output_contract.requires_content_evidence = false;
    tool_discovery.output_contract.locator_kind = OutputLocatorKind::None;
    tool_discovery.output_contract.locator_hint.clear();
    let eligibility = agent_loop_eligibility(&tool_discovery);
    assert!(eligibility.eligible);
    assert_eq!(
        eligibility.bucket,
        Some(AgentLoopEligibilityBucket::LowRiskToolDiscovery)
    );
}

#[test]
fn low_risk_single_file_delivery_accepts_bound_file_token_contracts() {
    let mut route = route_result(OutputResponseShape::FileToken, OutputSemanticKind::None);
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "README.md".to_string();
    route.output_contract.requires_content_evidence = true;

    let eligibility = agent_loop_eligibility(&route);

    assert!(eligibility.eligible);
    assert_eq!(
        eligibility.bucket,
        Some(AgentLoopEligibilityBucket::LowRiskSingleFileDelivery)
    );
    assert_eq!(
        eligibility.compatibility_migration_class(),
        "low_risk_single_file_delivery"
    );
    assert!(eligibility
        .boundary_requirements
        .contains(&"delivery_consistency_gate"));
}

#[test]
fn low_risk_single_file_delivery_accepts_bounded_directory_selector() {
    let mut route = route_result(OutputResponseShape::FileToken, OutputSemanticKind::None);
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "document".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.self_extension.list_selector = crate::OutputListSelector {
        target_kind: OutputScalarCountTargetKind::File,
        target_kind_specified: true,
        limit: Some(1),
        sort_by: Some("name_desc".to_string()),
        include_metadata: Some(false),
        include_hidden: Some(false),
    };

    let eligibility = agent_loop_eligibility(&route);

    assert!(eligibility.eligible);
    assert_eq!(
        eligibility.bucket,
        Some(AgentLoopEligibilityBucket::LowRiskSingleFileDelivery)
    );
}

#[test]
fn low_risk_single_file_delivery_rejects_unsafe_or_unbounded_delivery_contracts() {
    let mut generated = route_result(
        OutputResponseShape::FileToken,
        OutputSemanticKind::GeneratedFileDelivery,
    );
    generated.wants_file_delivery = true;
    generated.output_contract.delivery_required = true;
    generated.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    generated.output_contract.locator_hint = "document/generated.md".to_string();
    assert_eq!(agent_decides_eligible_migration_class(&generated), "none");

    let mut batch = route_result(OutputResponseShape::FileToken, OutputSemanticKind::None);
    batch.wants_file_delivery = true;
    batch.output_contract.delivery_required = true;
    batch.output_contract.delivery_intent = OutputDeliveryIntent::DirectoryBatchFiles;
    batch.output_contract.locator_hint = "document".to_string();
    assert_eq!(agent_decides_eligible_migration_class(&batch), "none");

    let mut missing_locator =
        route_result(OutputResponseShape::FileToken, OutputSemanticKind::None);
    missing_locator.wants_file_delivery = true;
    missing_locator.output_contract.delivery_required = true;
    missing_locator.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    missing_locator.output_contract.locator_kind = OutputLocatorKind::None;
    missing_locator.output_contract.locator_hint.clear();
    assert_eq!(
        agent_decides_eligible_migration_class(&missing_locator),
        "none"
    );

    let mut non_file_token = route_result(OutputResponseShape::Strict, OutputSemanticKind::None);
    non_file_token.wants_file_delivery = true;
    non_file_token.output_contract.delivery_required = true;
    non_file_token.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    non_file_token.output_contract.locator_hint = "README.md".to_string();
    assert_eq!(
        agent_decides_eligible_migration_class(&non_file_token),
        "none"
    );
}

#[test]
fn low_risk_direct_response_rejects_locator_delivery_and_evidence_contracts() {
    let mut with_locator = route_result(OutputResponseShape::OneSentence, OutputSemanticKind::None);
    with_locator.output_contract.requires_content_evidence = false;
    assert_eq!(
        agent_decides_eligible_migration_class(&with_locator),
        "none"
    );

    let mut with_delivery =
        route_result(OutputResponseShape::OneSentence, OutputSemanticKind::None);
    with_delivery.output_contract.requires_content_evidence = false;
    with_delivery.output_contract.locator_kind = OutputLocatorKind::None;
    with_delivery.output_contract.locator_hint.clear();
    with_delivery.output_contract.delivery_required = true;
    assert_eq!(
        agent_decides_eligible_migration_class(&with_delivery),
        "none"
    );

    let mut with_evidence =
        route_result(OutputResponseShape::OneSentence, OutputSemanticKind::None);
    with_evidence.output_contract.locator_kind = OutputLocatorKind::None;
    with_evidence.output_contract.locator_hint.clear();
    assert_eq!(
        agent_decides_eligible_migration_class(&with_evidence),
        "none"
    );
}

#[test]
fn eligibility_blocks_non_boundary_safe_routes_with_machine_reasons() {
    let mut route = route_result(OutputResponseShape::Strict, OutputSemanticKind::FileNames);
    route.risk_ceiling = RiskCeiling::High;
    let eligibility = agent_loop_eligibility(&route);
    assert!(!eligibility.eligible);
    assert_eq!(eligibility.blocked_reason, "risk_ceiling_high");

    route.risk_ceiling = RiskCeiling::Low;
    route.schedule_kind = ScheduleKind::Create;
    let eligibility = agent_loop_eligibility(&route);
    assert!(!eligibility.eligible);
    assert_eq!(eligibility.blocked_reason, "schedule_active");

    route.schedule_kind = ScheduleKind::None;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.route_reason = "capability_ref=config.apply_change".to_string();
    let eligibility = agent_loop_eligibility(&route);
    assert!(!eligibility.eligible);
    assert_eq!(eligibility.blocked_reason, "side_effect_operation");
}
