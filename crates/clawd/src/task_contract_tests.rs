use super::*;
use crate::IntentOutputContract;

#[test]
fn file_path_search_maps_to_directory_list_evidence() {
    let output_contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        semantic_kind: OutputSemanticKind::FilePaths,
        ..IntentOutputContract::default()
    };

    assert_eq!(
        target_object_for_output_contract(&output_contract),
        TaskTargetObject::Directory
    );
    assert_eq!(
        operation_for_output_contract(&output_contract),
        TaskOperation::List
    );
    assert_eq!(
        delivery_shape_for_output_contract(&output_contract),
        TaskDeliveryShape::List
    );
    assert_eq!(
        required_evidence_fields_for_output_contract(&output_contract),
        vec!["candidates"]
    );
}

#[test]
fn directory_summary_uses_listing_candidates() {
    let output_contract = IntentOutputContract {
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        semantic_kind: OutputSemanticKind::DirectoryPurposeSummary,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };

    assert_eq!(
        operation_for_output_contract(&output_contract),
        TaskOperation::Summarize
    );
    assert_eq!(
        required_evidence_fields_for_output_contract(&output_contract),
        vec!["candidates"]
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

    assert_eq!(
        target_object_for_output_contract(&output_contract),
        TaskTargetObject::Path
    );
    assert_eq!(
        required_evidence_fields_for_output_contract(&output_contract),
        vec!["exists", "kind", "path"]
    );
}

#[test]
fn unclassified_contract_uses_machine_output_fields_only() {
    let output_contract = IntentOutputContract {
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "README.md".to_string(),
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };

    assert_eq!(
        operation_for_output_contract(&output_contract),
        TaskOperation::Inspect
    );
    assert_eq!(
        target_object_for_output_contract(&output_contract),
        TaskTargetObject::Path
    );
}

#[test]
fn planner_semantic_matrix_drives_evidence_contract() {
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
            OutputSemanticKind::GitCommitSubject,
            TaskTargetObject::System,
            TaskOperation::Inspect,
            TaskDeliveryShape::Raw,
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
            OutputSemanticKind::ArchivePack,
            TaskTargetObject::Path,
            TaskOperation::Write,
            TaskDeliveryShape::File,
            vec!["path"],
        ),
    ] {
        let output_contract = IntentOutputContract {
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            semantic_kind,
            ..IntentOutputContract::default()
        };

        assert_eq!(
            target_object_for_output_contract(&output_contract),
            target,
            "{semantic_kind:?}"
        );
        assert_eq!(
            operation_for_output_contract(&output_contract),
            operation,
            "{semantic_kind:?}"
        );
        assert_eq!(
            delivery_shape_for_output_contract(&output_contract),
            delivery_shape,
            "{semantic_kind:?}"
        );
        assert_eq!(
            required_evidence_fields_for_output_contract(&output_contract),
            evidence,
            "{semantic_kind:?}"
        );
    }
}
