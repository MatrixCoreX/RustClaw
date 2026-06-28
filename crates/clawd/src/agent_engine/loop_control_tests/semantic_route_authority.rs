use super::super::LoopBudgetProfile;
use super::*;

#[test]
fn boundary_context_marks_structured_field_read_migration_eligibility() {
    let mut policy = test_policy();
    policy.agent_loop_canary_bucket = "structured_field_read".to_string();
    let mut route = route_result(OutputResponseShape::Scalar);
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/package.json".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.wants_file_delivery = false;

    let boundary = boundary_context_snapshot_json(
        &test_task(),
        &policy,
        Some(&AgentRunContext::default()),
        Some(&route),
        LoopBudgetProfile::FastRead,
    );

    assert_eq!(
        boundary
            .pointer("/budget/eligible_migration_class")
            .and_then(serde_json::Value::as_str),
        Some("structured_field_read")
    );
    assert_eq!(
        boundary
            .pointer("/budget/selected_migration_class")
            .and_then(serde_json::Value::as_str),
        Some("structured_field_read")
    );
    assert_eq!(
        boundary
            .pointer("/budget/agent_loop_eligibility_bucket")
            .and_then(serde_json::Value::as_str),
        Some("low_risk_structured_read")
    );
    assert_eq!(
        boundary
            .pointer("/budget/agent_loop_eligibility_blocked_reason")
            .and_then(serde_json::Value::as_str),
        Some("none")
    );
}

#[test]
fn boundary_context_exposes_normalizer_hints_as_machine_fields() {
    let policy = test_policy();
    let mut route = route_result(OutputResponseShape::Scalar);
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "README.md".to_string();
    route.route_confidence = Some(0.82);

    let boundary = boundary_context_snapshot_json(
        &test_task(),
        &policy,
        Some(&AgentRunContext::default()),
        Some(&route),
        LoopBudgetProfile::FastRead,
    );

    assert_eq!(
        boundary
            .pointer("/normalizer_hints/source")
            .and_then(serde_json::Value::as_str),
        Some("route_result_machine_fields")
    );
    assert_eq!(
        boundary
            .pointer("/normalizer_hints/gate_kind_hint")
            .and_then(serde_json::Value::as_str),
        Some("execute")
    );
    assert!(boundary
        .pointer("/normalizer_hints/decision_hint")
        .is_none());
    assert_eq!(
        boundary
            .pointer("/normalizer_hints/output_contract/semantic_kind")
            .and_then(serde_json::Value::as_str),
        Some("scalar_path_only")
    );
    assert_eq!(
        boundary
            .pointer("/normalizer_hints/candidate_locators/0/hint")
            .and_then(serde_json::Value::as_str),
        Some("README.md")
    );
    assert!(boundary
        .pointer("/normalizer_hints/candidate_contracts/0")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| value.starts_with("- task_contract ")));
}

#[test]
fn boundary_context_marks_agent_loop_canary_authority_for_selected_class() {
    let mut policy = test_policy();
    policy.semantic_route_authority = SemanticRouteAuthority::AgentLoopCanary;
    policy.agent_loop_canary_bucket = "structured_field_read".to_string();
    let mut route = route_result(OutputResponseShape::Scalar);
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_hint = "README.md".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.wants_file_delivery = false;

    let boundary = boundary_context_snapshot_json(
        &test_task(),
        &policy,
        Some(&AgentRunContext::default()),
        Some(&route),
        LoopBudgetProfile::FastRead,
    );

    assert_eq!(
        boundary
            .pointer("/semantic_routing/activation_state")
            .and_then(serde_json::Value::as_str),
        Some("agent_loop_canary")
    );
    assert_eq!(
        boundary
            .pointer("/semantic_routing/ordinary_semantic_authority")
            .and_then(serde_json::Value::as_str),
        Some("planner_loop_selected_class")
    );
    assert_eq!(
        boundary
            .pointer("/semantic_routing/runtime_default_authority")
            .and_then(serde_json::Value::as_str),
        Some("agent_loop_for_selected_migration_class")
    );
    assert_eq!(
        boundary
            .pointer("/semantic_routing/agent_loop_authority_enabled")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        boundary
            .pointer("/semantic_routing/chosen_authority")
            .and_then(serde_json::Value::as_str),
        Some("agent_loop_canary")
    );
    assert_eq!(
        boundary
            .pointer("/semantic_routing/rollback_reason")
            .and_then(serde_json::Value::as_str),
        Some("none")
    );
    assert_eq!(
        boundary
            .pointer("/budget/selected_migration_class")
            .and_then(serde_json::Value::as_str),
        Some("structured_field_read")
    );
}

#[test]
fn agent_loop_canary_uses_agent_decision_for_selected_class() {
    let mut policy = test_policy();
    policy.semantic_route_authority = SemanticRouteAuthority::AgentLoopCanary;
    policy.agent_loop_canary_bucket = "structured_field_read".to_string();
    let mut route = route_result(OutputResponseShape::Scalar);
    route.risk_ceiling = RiskCeiling::Low;
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_hint = "README.md".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.wants_file_delivery = false;

    assert_eq!(
        crate::agent_engine::agent_loop_authority_selected_migration_class_for_policy(
            &policy, &route
        ),
        Some("structured_field_read")
    );
}

#[test]
fn agent_loop_canary_falls_back_to_legacy_for_unselected_class() {
    let mut policy = test_policy();
    policy.semantic_route_authority = SemanticRouteAuthority::AgentLoopCanary;
    policy.agent_loop_canary_bucket = "exact_path_list".to_string();
    let mut route = route_result(OutputResponseShape::Scalar);
    route.risk_ceiling = RiskCeiling::Low;
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_hint = "README.md".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.wants_file_delivery = false;

    assert_eq!(
        crate::agent_engine::agent_loop_authority_selected_migration_class_for_policy(
            &policy, &route
        ),
        None
    );
    let boundary = boundary_context_snapshot_json(
        &test_task(),
        &policy,
        Some(&AgentRunContext::default()),
        Some(&route),
        LoopBudgetProfile::FastRead,
    );
    assert_eq!(
        boundary
            .pointer("/semantic_routing/chosen_authority")
            .and_then(serde_json::Value::as_str),
        Some("legacy_pre_agent")
    );
    assert_eq!(
        boundary
            .pointer("/semantic_routing/rollback_reason")
            .and_then(serde_json::Value::as_str),
        Some("migration_class_not_selected")
    );
}

fn selected_migration_class_for(route: RouteResult, selected: &str) -> Option<String> {
    let mut policy = test_policy();
    policy.agent_loop_canary_bucket = selected.to_string();
    let boundary = boundary_context_snapshot_json(
        &test_task(),
        &policy,
        Some(&AgentRunContext::default()),
        Some(&route),
        LoopBudgetProfile::FastRead,
    );
    boundary
        .pointer("/budget/selected_migration_class")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

#[test]
fn boundary_context_classifies_all_low_risk_migration_tokens() {
    let mut structured = route_result(OutputResponseShape::Scalar);
    structured.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    structured.output_contract.locator_hint = "README.md".to_string();
    assert_eq!(
        selected_migration_class_for(structured, "structured_field_read").as_deref(),
        Some("structured_field_read")
    );

    let mut exact_paths = route_result(OutputResponseShape::Strict);
    exact_paths.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    exact_paths.output_contract.locator_hint = "docs".to_string();
    assert_eq!(
        selected_migration_class_for(exact_paths, "exact_path_list").as_deref(),
        Some("exact_path_list")
    );

    let mut summary = route_result(OutputResponseShape::Free);
    summary.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    summary.output_contract.locator_hint = "README.md".to_string();
    assert_eq!(
        selected_migration_class_for(summary, "bound_path_summary").as_deref(),
        Some("bound_path_summary")
    );

    let mut recent = route_result(OutputResponseShape::OneSentence);
    recent.output_contract.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
    recent.output_contract.locator_kind = OutputLocatorKind::None;
    assert_eq!(
        selected_migration_class_for(recent, "recent_artifacts_judgment").as_deref(),
        Some("recent_artifacts_judgment")
    );

    let mut count = route_result(OutputResponseShape::Scalar);
    count.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    count.output_contract.locator_hint = "docs".to_string();
    assert_eq!(
        selected_migration_class_for(count, "scalar_count").as_deref(),
        Some("scalar_count")
    );
}

#[test]
fn boundary_context_keeps_migration_class_unselected_by_default() {
    let policy = test_policy();
    let mut route = route_result(OutputResponseShape::Scalar);
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/package.json".to_string();
    route.output_contract.requires_content_evidence = true;

    let boundary = boundary_context_snapshot_json(
        &test_task(),
        &policy,
        Some(&AgentRunContext::default()),
        Some(&route),
        LoopBudgetProfile::FastRead,
    );

    assert_eq!(
        boundary
            .pointer("/budget/eligible_migration_class")
            .and_then(serde_json::Value::as_str),
        Some("structured_field_read")
    );
    assert_eq!(
        boundary
            .pointer("/budget/selected_migration_class")
            .and_then(serde_json::Value::as_str),
        Some("none")
    );
}
