use super::tests::{journal, model};
use super::*;
use serde_json::json;

fn strict(fields: &str) -> crate::IntentOutputContract {
    let mut contract = crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::Strict,
        ..Default::default()
    };
    contract.selection.structured_field_selector = Some(fields.to_string());
    contract
}

#[test]
fn semantic_presentation_labels_validate_aliases_without_creating_source_field_requirements() {
    let mut contract = strict("unused_source_field");
    contract.selection.structured_field_selector = None;
    contract.selection.output_field_labels =
        vec!["new_count".into(), "read_count".into(), "play_count".into()];
    contract.requires_content_evidence = true;
    let candidate = r#"{"new_count":0,"read_count":null,"play_count":null}"#;
    let checks = host_output_field_checks(&contract, candidate);
    assert_eq!(checks.len(), 3);
    assert!(checks.iter().all(|check| check.exact_label_present));
    assert!(!contract.requests_exact_structured_fields());
    assert!(
        !crate::evidence_policy::required_evidence_fields_for_output_contract(&contract)
            .iter()
            .any(|field| ["new_count", "read_count", "play_count"].contains(&field.as_str()))
    );
    let missing = host_output_field_checks(&contract, r#"{"new_count":0,"read_count":null}"#);
    assert!(
        !validate_output_field_audit(
            validate_operation_audit(model(json!([])), &journal()),
            &missing,
            r#"{"new_count":0,"read_count":null}"#
        )
        .pass
    );
    let no_operations = TaskJournal::for_task("missing", "ask", "fixture");
    let verdict = validate_operation_audit(
        model(json!([{
            "requested_operation":"fixture read", "evidence_step_ids":[],
            "method_observed":false, "result_observed":false
        }])),
        &no_operations,
    );
    assert!(!validate_output_field_audit(verdict, &checks, candidate).pass);
}

#[test]
fn json_values_and_similar_keys_are_not_requested_field_labels() {
    for candidate in [
        r#"{"note":"path size_bytes"}"#,
        r#"{"path_hint":"sample","not_size_bytes":12}"#,
        r#"{"text":{"path":"sample","size_bytes":12}}"#,
        r#"["path","size_bytes"]"#,
    ] {
        let checks = host_output_field_checks(&strict("path,size_bytes"), candidate);
        assert_eq!(checks.len(), 2);
        assert!(
            checks.iter().all(|check| !check.exact_label_present),
            "{candidate}"
        );
        let verdict = validate_output_field_audit(
            validate_operation_audit(model(json!([])), &journal()),
            &checks,
            candidate,
        );
        assert!(!verdict.pass, "{candidate}");
        assert_eq!(verdict.missing_evidence_fields, vec!["output_format"]);
    }
}

#[test]
fn semantic_labels_preserve_literal_localized_text_and_dotted_json_keys() {
    let mut contract = strict("unused");
    contract.selection.structured_field_selector = None;
    contract.selection.output_field_labels =
        vec!["文字".into(), "text".into(), "entry.path".into()];
    let candidate = r#"{"文字":"sample","text":"sample","entry.path":null}"#;
    let checks = host_output_field_checks(&contract, candidate);
    assert_eq!(checks.len(), 3);
    assert!(checks.iter().all(|check| check.exact_label_present));
    assert!(
        validate_output_field_audit(
            validate_operation_audit(model(json!([])), &journal()),
            &checks,
            candidate
        )
        .pass
    );
}

#[test]
fn nested_json_paths_are_present_without_a_dotted_key_literal() {
    let candidate = r#"{"entry":{"path":"sample","size_bytes":0}}"#;
    let checks = host_output_field_checks(&strict("entry.path,entry.size_bytes"), candidate);
    assert_eq!(checks.len(), 2);
    assert!(checks.iter().all(|check| check.exact_label_present));
    assert!(
        validate_output_field_audit(
            validate_operation_audit(model(json!([])), &journal()),
            &checks,
            candidate,
        )
        .pass
    );
}

#[test]
fn actual_json_keys_preserve_zero_null_and_escaped_spelling() {
    for candidate in [
        r#"{"path":null,"size_bytes":0}"#,
        r#"{"\u0070ath":"sample","size_bytes":0}"#,
        r#"{"result":{"path":"sample","size_bytes":0}}"#,
    ] {
        let checks = host_output_field_checks(&strict("path,size_bytes"), candidate);
        assert!(
            checks.iter().all(|check| check.exact_label_present),
            "{candidate}"
        );
    }
}

#[test]
fn second_field_audit_rejects_true_flags_when_labels_are_only_json_values() {
    let checks = vec![OutputFieldCheck {
        requested_field: "path".to_string(),
        exact_label_present: true,
    }];
    let verdict = validate_output_field_audit(
        validate_operation_audit(model(json!([])), &journal()),
        &checks,
        r#"{"note":"path","resolved_path":"sample"}"#,
    );
    assert!(!verdict.pass);
    assert!(verdict.should_retry);
}

#[test]
fn value_only_and_file_delivery_do_not_require_source_field_labels() {
    for (shape, field, candidate) in [
        (crate::OutputResponseShape::Scalar, "count", "0"),
        (
            crate::OutputResponseShape::FileToken,
            "path",
            "FILE:/workspace/report.txt",
        ),
    ] {
        let mut contract = strict(field);
        contract.response_shape = shape;
        assert!(host_output_field_checks(&contract, candidate).is_empty());
    }
}

#[test]
fn removing_scalar_labels_does_not_waive_required_execution_evidence() {
    let mut contract = strict("count");
    contract.response_shape = crate::OutputResponseShape::Scalar;
    let checks = host_output_field_checks(&contract, "0");
    assert!(checks.is_empty());
    let missing = crate::task_journal::TaskJournal::for_task("missing", "ask", "fixture");
    let verdict = validate_output_field_audit(
        validate_operation_audit(
            model(json!([{
                "requested_operation":"fixture read", "evidence_step_ids":[],
                "method_observed":false, "result_observed":false
            }])),
            &missing,
        ),
        &checks,
        "0",
    );
    assert!(!verdict.pass);
    assert!(verdict
        .missing_evidence_fields
        .iter()
        .any(|field| field == "requested_result"));
}
