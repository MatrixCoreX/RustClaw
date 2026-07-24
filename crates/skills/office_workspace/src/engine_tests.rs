use super::*;
use crate::test_support::{temp_path, xlsx_fixture};

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
    std::fs::remove_file(path).ok();
}

#[test]
fn cursors_are_bound_to_the_source_hash() {
    assert!(parse_cursor("office-v1:10:deadbeef", "cafebabe").is_err());
}
