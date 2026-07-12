use super::*;
use serde_json::json;

fn output_contract_for_selector_test() -> IntentOutputContract {
    IntentOutputContract {
        response_shape: crate::OutputResponseShape::Scalar,
        exact_sentence_count: None,
        requires_content_evidence: false,
        delivery_required: true,
        locator_kind: crate::OutputLocatorKind::Path,
        delivery_intent: crate::OutputDeliveryIntent::FileSingle,
        semantic_kind: crate::OutputSemanticKind::RawCommandOutput,
        locator_hint: "logs/clawd.log".to_string(),
        self_extension: Default::default(),
    }
}

#[test]
fn task_lifecycle_structured_field_selector_normalizes_to_service_status_contract() {
    let mut contract = output_contract_for_selector_test();

    let selector = apply_state_patch_structured_field_selector(
        &mut contract,
        Some(&json!({"structured_field_selector": "task_lifecycle.*"})),
    );

    assert_eq!(selector.as_deref(), Some("task_lifecycle.*"));
    assert_eq!(
        contract.self_extension.structured_field_selector.as_deref(),
        Some("task_lifecycle.*")
    );
    assert_eq!(
        contract.semantic_kind,
        crate::OutputSemanticKind::ServiceStatus
    );
    assert_eq!(contract.response_shape, crate::OutputResponseShape::Free);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.locator_kind, crate::OutputLocatorKind::None);
    assert_eq!(contract.delivery_intent, crate::OutputDeliveryIntent::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn task_lifecycle_required_machine_fields_normalize_to_service_status_contract() {
    let mut contract = output_contract_for_selector_test();

    let selector = apply_state_patch_structured_field_selector(
        &mut contract,
        Some(&json!({
            "required_machine_fields": [
                "task_lifecycle.state",
                "task_lifecycle.can_poll",
                "task_lifecycle.can_cancel",
                "task_lifecycle.checkpoint_id"
            ]
        })),
    );

    assert_eq!(selector.as_deref(), Some("task_lifecycle.*"));
    assert!(state_patch_targets_task_lifecycle_fields(Some(&json!({
        "required_machine_fields": ["can_poll", "can_cancel", "checkpoint_id"]
    }))));
    assert_eq!(
        contract.self_extension.structured_field_selector.as_deref(),
        Some("task_lifecycle.*")
    );
    assert_eq!(
        contract.semantic_kind,
        crate::OutputSemanticKind::ServiceStatus
    );
    assert_eq!(contract.response_shape, crate::OutputResponseShape::Free);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.locator_kind, crate::OutputLocatorKind::None);
    assert_eq!(contract.delivery_intent, crate::OutputDeliveryIntent::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn required_machine_fields_clear_non_file_delivery_contract() {
    let mut contract = output_contract_for_selector_test();
    contract.response_shape = crate::OutputResponseShape::Strict;

    let repair = apply_state_patch_required_machine_fields_contract(
        &mut contract,
        Some(&json!({
            "required_machine_fields": [
                "created_files",
                "test_command",
                "test_status"
            ]
        })),
    );

    assert_eq!(
        repair,
        Some("required_machine_fields_clear_delivery_contract")
    );
    assert!(!contract.delivery_required);
    assert_eq!(contract.delivery_intent, crate::OutputDeliveryIntent::None);
    assert_eq!(contract.response_shape, crate::OutputResponseShape::Strict);
}

#[test]
fn required_machine_fields_keep_true_file_token_delivery_contract() {
    let mut contract = output_contract_for_selector_test();
    contract.response_shape = crate::OutputResponseShape::FileToken;
    contract.delivery_required = true;
    contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;

    let repair = apply_state_patch_required_machine_fields_contract(
        &mut contract,
        Some(&json!({"required_machine_fields": ["created_files"]})),
    );

    assert_eq!(repair, None);
    assert!(contract.delivery_required);
    assert_eq!(
        contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
    );
}

#[test]
fn required_machine_fields_clear_contradictory_file_token_contract() {
    let mut contract = output_contract_for_selector_test();
    contract.response_shape = crate::OutputResponseShape::FileToken;
    contract.delivery_required = false;
    contract.delivery_intent = crate::OutputDeliveryIntent::None;

    let repair = apply_state_patch_required_machine_fields_contract(
        &mut contract,
        Some(&json!({
            "required_machine_fields": [
                "changed_files",
                "test_command",
                "test_status",
                "functions",
                "error_codes"
            ]
        })),
    );

    assert_eq!(
        repair,
        Some("required_machine_fields_clear_delivery_contract")
    );
    assert!(!contract.delivery_required);
    assert_eq!(contract.delivery_intent, crate::OutputDeliveryIntent::None);
    assert_eq!(contract.response_shape, crate::OutputResponseShape::Strict);
}
