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
fn structured_selector_owns_required_evidence_without_domain_semantics() {
    let mut output_contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    output_contract.selection.structured_field_selector =
        Some("datetime,timezone,title".to_string());

    assert_eq!(
        operation_for_output_contract(&output_contract),
        TaskOperation::Inspect
    );
    assert_eq!(
        required_evidence_fields_for_output_contract(&output_contract),
        vec!["datetime", "timezone", "title"]
    );
    let expression =
        crate::evidence_policy::evidence_expression_for_output_contract(&output_contract)
            .expect("selector evidence expression");
    assert_eq!(expression.all_of, vec!["datetime", "timezone", "title"]);
    assert!(expression.one_of.is_empty());
    assert!(expression.any_of.is_empty());
}

#[test]
fn planner_semantic_matrix_drives_evidence_contract() {
    let output_contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ConfigValidation,
        ..IntentOutputContract::default()
    };

    assert_eq!(
        target_object_for_output_contract(&output_contract),
        TaskTargetObject::ConfigKey
    );
    assert_eq!(
        operation_for_output_contract(&output_contract),
        TaskOperation::Validate
    );
    assert_eq!(
        delivery_shape_for_output_contract(&output_contract),
        TaskDeliveryShape::Summary
    );
    assert_eq!(
        required_evidence_fields_for_output_contract(&output_contract),
        vec!["valid"]
    );
}
