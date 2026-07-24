use super::*;

#[test]
fn rejects_operations_from_the_wrong_format() {
    let error = normalize_operations(
        Some(&json!([{"op": "add_slide"}])),
        OfficeFormat::Docx,
        false,
    )
    .expect_err("wrong operation");
    assert_eq!(error.code, "unsupported_operation");
}

#[test]
fn gives_operations_stable_batch_ids() {
    let operations = normalize_operations(
        Some(&json!([
            {"op": "add_heading", "text": "Report"},
            {"op": "add_paragraph", "text": "Body"}
        ])),
        OfficeFormat::Docx,
        false,
    )
    .expect("operations");
    assert_eq!(operations[0].id, "op_1");
    assert_eq!(operations[1].id, "op_2");
}
