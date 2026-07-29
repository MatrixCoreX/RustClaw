use super::*;
use crate::test_support::{docx_fixture, temp_path, xlsx_fixture};

#[test]
fn skill_protocol_returns_structured_docx_evidence() {
    let path = temp_path("docx");
    docx_fixture(&path);
    let response = process_line(
        &json!({
            "request_id": "office-1",
            "args": {"action": "word.read", "path": path}
        })
        .to_string(),
    );
    assert_eq!(response.status, "ok");
    assert_eq!(response.request_id, "office-1");
    assert_eq!(response.extra["format"], "docx");
    std::fs::remove_file(path).ok();
}

#[test]
fn skill_protocol_exposes_a_bounded_model_observation() {
    let path = temp_path("xlsx");
    xlsx_fixture(&path);
    let response = process_line(
        &json!({
            "request_id": "office-model-observation-1",
            "args": {
                "action": "spreadsheet.read_range",
                "path": path,
                "sheet": "数据",
                "range": "A1:B2"
            }
        })
        .to_string(),
    );

    assert_eq!(response.status, "ok");
    assert_eq!(
        response.extra["model_observation"]["action"],
        "spreadsheet.read_range"
    );
    assert_eq!(
        response.extra["model_observation"]["workbook"]["sheets"][0]["cells"][0]["reference"],
        "A1"
    );
    assert!(
        response.extra["model_observation"].get("package").is_none(),
        "bulk package metadata must not displace task content"
    );
    std::fs::remove_file(path).ok();
}

#[test]
fn preview_protocol_omits_absent_optional_contract_fields() {
    let output_path = temp_path("docx");
    std::fs::remove_file(&output_path).ok();
    let response = process_line(
        &json!({
            "request_id": "office-preview-1",
            "args": {
                "action": "word.preview_create",
                "output_path": output_path,
                "operations": [
                    {"op": "add_heading", "level": 1, "text": "Preview"},
                    {"op": "add_paragraph", "text": "No write"}
                ]
            }
        })
        .to_string(),
    );

    assert_eq!(response.status, "ok");
    assert_eq!(response.extra["preview"], true);
    assert_eq!(response.extra["writes_performed"], false);
    let text: Value = serde_json::from_str(&response.text).expect("compact JSON response");
    assert_eq!(text["preview"], true);
    assert_eq!(text["writes_performed"], false);
    assert!(text.get("cursor").is_none());
    assert!(text.get("source").is_none());
    assert!(!output_path.exists());
}
