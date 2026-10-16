use super::*;
use crate::test_support::{temp_path, xlsx_fixture};

#[test]
fn typed_ingestion_adapter_preserves_pure_parser_boundary() {
    let path = temp_path("xlsx");
    xlsx_fixture(&path);

    let envelope = crate::inspect_for_ingestion(&path).expect("typed ingestion evidence");

    assert_eq!(envelope.schema_version, 1);
    assert_eq!(envelope.format.as_str(), "xlsx");
    assert_eq!(envelope.metadata["active_content_executed"], false);
    assert_eq!(envelope.metadata["pure_parser"], true);
    assert!(envelope.workbook.is_some());
    assert!(envelope.validation.valid);
}

#[test]
fn reads_a_bounded_spreadsheet_range() {
    let path = temp_path("xlsx");
    xlsx_fixture(&path);
    let value = execute(&json!({
        "action": "spreadsheet.read_range",
        "path": path,
        "sheet": "数据",
        "range": "B2:B2"
    }))
    .expect("execute");
    let cells = value["workbook"]["sheets"][0]["cells"]
        .as_array()
        .expect("cells");
    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0]["reference"], "B2");
    assert_eq!(cells[0]["untrusted"], true);
    assert!(value["warnings"]
        .as_array()
        .expect("warnings")
        .iter()
        .any(|warning| warning["code"] == "formula_content_present"));
    std::fs::remove_file(path).ok();
}

#[test]
fn cursors_are_bound_to_the_source_hash() {
    assert!(parse_cursor("office-v1:10:deadbeef", "cafebabe").is_err());
}
