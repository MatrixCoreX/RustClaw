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
