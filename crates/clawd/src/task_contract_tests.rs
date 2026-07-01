use super::*;
use crate::{AskMode, IntentOutputContract};

fn route_with_contract(ask_mode: AskMode, output_contract: IntentOutputContract) -> RouteResult {
    RouteResult {
        ask_mode,
        resolved_intent: "test intent".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "test".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract,
    }
}

#[test]
fn file_path_search_contract_is_list_with_candidate_evidence() {
    let route = route_with_contract(
        AskMode::planner_execute_plain(),
        IntentOutputContract {
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            semantic_kind: OutputSemanticKind::FilePaths,
            ..IntentOutputContract::default()
        },
    );

    let contract = TaskContract::from_route_result(&route);

    assert_eq!(contract.intent_kind, TaskIntentKind::PlannerExecute);
    assert_eq!(contract.target_object, TaskTargetObject::Directory);
    assert_eq!(contract.operation, TaskOperation::List);
    assert_eq!(contract.delivery_shape, TaskDeliveryShape::List);
    assert_eq!(contract.required_evidence_fields, vec!["candidates"]);
    assert_eq!(
        contract.failure_policy,
        TaskFailurePolicy::RetryWithAlternatives
    );
}

#[test]
fn missing_locator_contract_prefers_clarify_policy() {
    let mut route = route_with_contract(
        AskMode::clarify(),
        IntentOutputContract {
            locator_kind: OutputLocatorKind::Path,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        },
    );
    route.needs_clarify = true;

    let contract = TaskContract::from_route_result(&route);

    assert_eq!(contract.intent_kind, TaskIntentKind::Clarify);
    assert_eq!(contract.missing_parameters, vec!["locator"]);
    assert_eq!(contract.failure_policy, TaskFailurePolicy::Clarify);
}

#[test]
fn task_contract_includes_structured_workspace_target() {
    let route = route_with_contract(
        AskMode::planner_execute_plain(),
        IntentOutputContract {
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        },
    );

    let contract = TaskContract::from_route_result(&route);

    assert_eq!(contract.targets.len(), 1);
    assert_eq!(contract.targets[0].role, TaskTargetRole::Primary);
    assert_eq!(contract.targets[0].kind, TaskTargetObject::Directory);
    assert_eq!(contract.targets[0].locator, ".");
    assert!(contract.compact_prompt_line().contains("\"locator\":\".\""));
}

#[test]
fn directory_purpose_summary_uses_listing_candidates_as_required_evidence() {
    let route = route_with_contract(
        AskMode::planner_execute_chat_wrapped(),
        IntentOutputContract {
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            semantic_kind: OutputSemanticKind::DirectoryPurposeSummary,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        },
    );

    let contract = TaskContract::from_route_result(&route);

    assert_eq!(contract.operation, TaskOperation::Summarize);
    assert_eq!(contract.required_evidence_fields, vec!["candidates"]);
    assert!(!contract
        .compact_prompt_line()
        .contains("required_evidence_fields=content_excerpt"));
}

#[test]
fn existence_contract_requires_structural_path_evidence() {
    let route = route_with_contract(
        AskMode::planner_execute_plain(),
        IntentOutputContract {
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "README.md".to_string(),
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        },
    );

    let contract = TaskContract::from_route_result(&route);

    assert_eq!(contract.targets.len(), 1);
    assert_eq!(contract.targets[0].kind, TaskTargetObject::Path);
    assert_eq!(contract.targets[0].locator, "README.md");
    assert_eq!(
        contract.required_evidence_fields,
        vec!["exists", "kind", "path"]
    );
}

#[test]
fn task_contract_uses_specific_config_archive_capability_ref_evidence() {
    for (marker, target, operation, evidence) in [
        (
            "capability_ref=config.validate",
            TaskTargetObject::ConfigKey,
            TaskOperation::Validate,
            vec!["valid"],
        ),
        (
            "capability_ref=config.apply_change",
            TaskTargetObject::ConfigKey,
            TaskOperation::Modify,
            vec!["field_value", "path", "valid"],
        ),
        (
            "capability_ref=config.guard_after_change",
            TaskTargetObject::ConfigKey,
            TaskOperation::Validate,
            vec!["candidates", "count"],
        ),
        (
            "capability_ref=archive.pack",
            TaskTargetObject::Path,
            TaskOperation::Write,
            vec!["path"],
        ),
        (
            "capability_ref=archive.unpack",
            TaskTargetObject::Path,
            TaskOperation::Modify,
            vec!["path"],
        ),
    ] {
        let mut route = route_with_contract(
            AskMode::planner_execute_plain(),
            IntentOutputContract {
                response_shape: OutputResponseShape::Strict,
                requires_content_evidence: true,
                locator_kind: OutputLocatorKind::Path,
                semantic_kind: OutputSemanticKind::None,
                ..IntentOutputContract::default()
            },
        );
        route.resolved_intent = marker.to_string();

        let contract = TaskContract::from_route_result(&route);

        assert_eq!(contract.target_object, target, "{marker}");
        assert_eq!(contract.operation, operation, "{marker}");
        assert_eq!(contract.required_evidence_fields, evidence, "{marker}");
    }
}

#[test]
fn task_contract_splits_structured_multi_target_locator() {
    let route = route_with_contract(
        AskMode::planner_execute_plain(),
        IntentOutputContract {
            locator_kind: OutputLocatorKind::Filename,
            locator_hint: "README.md | AGENTS.md".to_string(),
            semantic_kind: OutputSemanticKind::QuantityComparison,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        },
    );

    let contract = TaskContract::from_route_result(&route);

    assert_eq!(contract.targets.len(), 2);
    assert_eq!(contract.targets[0].locator, "README.md");
    assert_eq!(contract.targets[1].locator, "AGENTS.md");
    assert_eq!(
        contract.required_evidence_fields,
        vec!["exists", "field_value", "kind", "size_bytes"]
    );
    assert!(contract
        .compact_prompt_line()
        .contains("\"locator\":\"AGENTS.md\""));
}

#[test]
fn task_contract_splits_comma_multi_target_locator() {
    let route = route_with_contract(
        AskMode::planner_execute_plain(),
        IntentOutputContract {
            locator_kind: OutputLocatorKind::Filename,
            locator_hint: "README.md, README.zh-CN.md, Cargo.toml".to_string(),
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        },
    );

    let contract = TaskContract::from_route_result(&route);

    assert_eq!(contract.targets.len(), 3);
    assert_eq!(contract.targets[0].locator, "README.md");
    assert_eq!(contract.targets[1].locator, "README.zh-CN.md");
    assert_eq!(contract.targets[2].locator, "Cargo.toml");
}
