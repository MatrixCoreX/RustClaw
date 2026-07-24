use super::{
    exact_machine_field_selector, output_contract_exact_scalar_field,
    output_contract_requests_exact_list_path, structured_json_satisfies_field_selector,
};

#[test]
fn selector_accepts_unique_machine_paths_only() {
    assert_eq!(
        exact_machine_field_selector("count, result.path count"),
        Some(vec!["count".to_string(), "result.path".to_string()])
    );
    assert!(exact_machine_field_selector("result.*").is_none());
    assert!(exact_machine_field_selector("text").is_none());
    assert!(exact_machine_field_selector("结果").is_none());
}

#[test]
fn selector_finds_nested_machine_fields_without_reading_visible_text() {
    assert!(structured_json_satisfies_field_selector(
        "exists,path",
        r#"{"extra":{"exists":true,"path":"/workspace/a"}}"#
    ));
    assert!(!structured_json_satisfies_field_selector(
        "path",
        r#"{"text":"path=/workspace/a"}"#
    ));
}

#[test]
fn scalar_and_list_contract_helpers_require_exact_shapes() {
    let mut scalar = crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::Scalar,
        ..Default::default()
    };
    scalar.selection.structured_field_selector = Some("resolved_path".to_string());
    assert_eq!(
        output_contract_exact_scalar_field(&scalar, &["path", "resolved_path"]).as_deref(),
        Some("resolved_path")
    );

    let mut list = crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::Strict,
        ..Default::default()
    };
    list.selection.structured_field_selector = Some("path".to_string());
    assert!(output_contract_requests_exact_list_path(&list));
    list.selection.structured_field_selector = Some("path,count".to_string());
    assert!(!output_contract_requests_exact_list_path(&list));
}
