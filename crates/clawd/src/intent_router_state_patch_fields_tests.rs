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
