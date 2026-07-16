use super::*;
use crate::IntentOutputContract;

#[test]
fn file_path_search_contract_is_list_with_candidate_evidence() {
    let output_contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        semantic_kind: OutputSemanticKind::FilePaths,
        ..IntentOutputContract::default()
    };

    let contract = EvidencePolicyContract::from_output_contract(&output_contract);

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
fn evidence_policy_contract_includes_structured_workspace_target() {
    let output_contract = IntentOutputContract {
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };

    let contract = EvidencePolicyContract::from_output_contract(&output_contract);

    assert_eq!(contract.targets.len(), 1);
    assert_eq!(contract.targets[0].role, TaskTargetRole::Primary);
    assert_eq!(contract.targets[0].kind, TaskTargetObject::Directory);
    assert_eq!(contract.targets[0].locator, ".");
    assert!(
        evidence_policy_context_prompt_line_for_output_contract(&output_contract)
            .contains("\"locator\":\".\"")
    );
}

#[test]
fn directory_purpose_summary_uses_listing_candidates_as_required_evidence() {
    let output_contract = IntentOutputContract {
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        semantic_kind: OutputSemanticKind::DirectoryPurposeSummary,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };

    let contract = EvidencePolicyContract::from_output_contract(&output_contract);

    assert_eq!(contract.operation, TaskOperation::Summarize);
    assert_eq!(contract.required_evidence_fields, vec!["candidates"]);
    assert!(
        !evidence_policy_context_prompt_line_for_output_contract(&output_contract)
            .contains("required_evidence_fields=content_excerpt")
    );
}

#[test]
fn existence_contract_requires_structural_path_evidence() {
    let output_contract = IntentOutputContract {
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "README.md".to_string(),
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };

    let contract = EvidencePolicyContract::from_output_contract(&output_contract);

    assert_eq!(contract.targets.len(), 1);
    assert_eq!(contract.targets[0].kind, TaskTargetObject::Path);
    assert_eq!(contract.targets[0].locator, "README.md");
    assert_eq!(
        contract.required_evidence_fields,
        vec!["exists", "kind", "path"]
    );
}

#[test]
fn unclassified_evidence_contract_operation_does_not_depend_on_route_trace() {
    let output_contract = IntentOutputContract {
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "README.md".to_string(),
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };

    let contract = EvidencePolicyContract::from_output_contract(&output_contract);

    assert_eq!(contract.operation, TaskOperation::Inspect);
    assert_eq!(contract.target_object, TaskTargetObject::Path);
    assert_eq!(
        contract.failure_policy,
        TaskFailurePolicy::RetryWithAlternatives
    );
}

#[test]
fn task_contract_failure_policy_does_not_depend_on_execute_gate_trace() {
    let output_contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: false,
        locator_kind: OutputLocatorKind::None,
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };

    let contract = EvidencePolicyContract::from_output_contract(&output_contract);

    assert_eq!(contract.operation, TaskOperation::Unknown);
    assert!(!contract.evidence_required);
    assert!(contract.missing_parameters.is_empty());
    assert_eq!(contract.failure_policy, TaskFailurePolicy::NoRetry);
}

#[test]
fn task_contract_uses_planner_semantic_matrix_without_capability_ref() {
    for (semantic_kind, target, operation, delivery_shape, evidence) in [
        (
            OutputSemanticKind::WeatherQuery,
            TaskTargetObject::Web,
            TaskOperation::Summarize,
            TaskDeliveryShape::Summary,
            vec!["content_excerpt"],
        ),
        (
            OutputSemanticKind::PackageManagerDetection,
            TaskTargetObject::System,
            TaskOperation::Inspect,
            TaskDeliveryShape::Summary,
            vec!["field_value"],
        ),
        (
            OutputSemanticKind::DockerImages,
            TaskTargetObject::Process,
            TaskOperation::List,
            TaskDeliveryShape::List,
            vec!["candidates"],
        ),
        (
            OutputSemanticKind::ConfigValidation,
            TaskTargetObject::ConfigKey,
            TaskOperation::Validate,
            TaskDeliveryShape::Summary,
            vec!["valid"],
        ),
        (
            OutputSemanticKind::ConfigMutation,
            TaskTargetObject::ConfigKey,
            TaskOperation::Modify,
            TaskDeliveryShape::Summary,
            vec!["field_value", "path", "valid"],
        ),
        (
            OutputSemanticKind::ConfigRiskAssessment,
            TaskTargetObject::ConfigKey,
            TaskOperation::Validate,
            TaskDeliveryShape::Summary,
            vec!["candidates", "count"],
        ),
        (
            OutputSemanticKind::GitCommitSubject,
            TaskTargetObject::System,
            TaskOperation::Inspect,
            TaskDeliveryShape::Raw,
            vec!["field_value"],
        ),
        (
            OutputSemanticKind::GitRepositoryState,
            TaskTargetObject::System,
            TaskOperation::Inspect,
            TaskDeliveryShape::Summary,
            vec!["field_value"],
        ),
        (
            OutputSemanticKind::SqliteTableListing,
            TaskTargetObject::Db,
            TaskOperation::List,
            TaskDeliveryShape::List,
            vec!["candidates"],
        ),
        (
            OutputSemanticKind::SqliteTableNamesOnly,
            TaskTargetObject::Db,
            TaskOperation::List,
            TaskDeliveryShape::List,
            vec!["candidates"],
        ),
        (
            OutputSemanticKind::SqliteDatabaseKindJudgment,
            TaskTargetObject::Db,
            TaskOperation::Inspect,
            TaskDeliveryShape::Summary,
            vec!["field_value"],
        ),
        (
            OutputSemanticKind::SqliteSchemaVersion,
            TaskTargetObject::Db,
            TaskOperation::Inspect,
            TaskDeliveryShape::Raw,
            vec!["field_value"],
        ),
        (
            OutputSemanticKind::ArchiveList,
            TaskTargetObject::Path,
            TaskOperation::List,
            TaskDeliveryShape::List,
            vec!["candidates"],
        ),
        (
            OutputSemanticKind::ArchiveRead,
            TaskTargetObject::Path,
            TaskOperation::Inspect,
            TaskDeliveryShape::Summary,
            vec!["content_excerpt"],
        ),
        (
            OutputSemanticKind::ArchivePack,
            TaskTargetObject::Path,
            TaskOperation::Write,
            TaskDeliveryShape::File,
            vec!["path"],
        ),
        (
            OutputSemanticKind::ArchiveUnpack,
            TaskTargetObject::Path,
            TaskOperation::Modify,
            TaskDeliveryShape::Summary,
            vec!["path"],
        ),
    ] {
        let output_contract = IntentOutputContract {
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::None,
            semantic_kind,
            ..IntentOutputContract::default()
        };

        let contract = EvidencePolicyContract::from_output_contract(&output_contract);

        assert_eq!(contract.target_object, target, "{semantic_kind:?}");
        assert_eq!(contract.operation, operation, "{semantic_kind:?}");
        assert_eq!(contract.delivery_shape, delivery_shape, "{semantic_kind:?}");
        assert_eq!(
            contract.required_evidence_fields, evidence,
            "{semantic_kind:?}"
        );
    }
}

#[test]
fn task_contract_splits_structured_multi_target_locator() {
    let output_contract = IntentOutputContract {
        locator_kind: OutputLocatorKind::Filename,
        locator_hint: "README.md | AGENTS.md".to_string(),
        semantic_kind: OutputSemanticKind::QuantityComparison,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };

    let contract = EvidencePolicyContract::from_output_contract(&output_contract);

    assert_eq!(contract.targets.len(), 2);
    assert_eq!(contract.targets[0].locator, "README.md");
    assert_eq!(contract.targets[1].locator, "AGENTS.md");
    assert_eq!(
        contract.required_evidence_fields,
        vec!["exists", "field_value", "kind", "size_bytes"]
    );
    assert!(
        evidence_policy_context_prompt_line_for_output_contract(&output_contract)
            .contains("\"locator\":\"AGENTS.md\"")
    );
}

#[test]
fn task_contract_splits_comma_multi_target_locator() {
    let output_contract = IntentOutputContract {
        locator_kind: OutputLocatorKind::Filename,
        locator_hint: "README.md, README.zh-CN.md, Cargo.toml".to_string(),
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };

    let contract = EvidencePolicyContract::from_output_contract(&output_contract);

    assert_eq!(contract.targets.len(), 3);
    assert_eq!(contract.targets[0].locator, "README.md");
    assert_eq!(contract.targets[1].locator, "README.zh-CN.md");
    assert_eq!(contract.targets[2].locator, "Cargo.toml");
}
