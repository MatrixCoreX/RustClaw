use super::*;
use crate::model::OfficeFormat;
use crate::operations::normalize_operations;
use crate::package::OfficePackage;
use crate::package_write::publish_package;
use crate::test_support::temp_path;
use crate::xlsx::read_workbook;

#[test]
fn creates_typed_multisheet_workbook_with_formula_boundary() {
    let operations = normalize_operations(
        Some(&json!([
            {"op":"add_sheet","name":"摘要"},
            {"op":"set_range","sheet":"摘要","range":"A1:B3","values":[["项目","值"],["收入",42],["原样","=NOT_A_FORMULA"]]},
            {"op":"set_cell","sheet":"摘要","cell":"B4","value":"SUM(B2:B3)","value_type":"formula"},
            {"op":"freeze_panes","sheet":"摘要","cell":"A2"},
            {"op":"set_auto_filter","sheet":"摘要","range":"A1:B3"},
            {"op":"add_table","sheet":"摘要","range":"A1:B3","name":"SummaryTable"},
            {"op":"add_chart","sheet":"摘要","range":"B1:B3","title":"收入","chart_type":"column"},
            {"op":"add_sheet","name":"Details"},
            {"op":"set_cell","sheet":"Details","cell":"A1","value":"2026-07-24","value_type":"date"}
        ])),
        OfficeFormat::Xlsx,
        false,
    )
    .expect("operations");
    let result = create_xlsx(&operations).expect("create");
    let path = temp_path("xlsx");
    publish_package(
        &result.members,
        &path,
        OfficeFormat::Xlsx,
        false,
        None,
        None,
    )
    .expect("publish");
    let package = OfficePackage::open(&path, Some(OfficeFormat::Xlsx)).expect("package");
    let workbook = read_workbook(&package).expect("read");
    assert_eq!(workbook.sheets.len(), 2);
    let summary = &workbook.sheets[0];
    let plain_formula_like = summary
        .cells
        .iter()
        .find(|cell| cell.reference == "B3")
        .expect("B3");
    assert_eq!(plain_formula_like.cell_type, "string");
    assert_eq!(plain_formula_like.formula, None);
    let formula = summary
        .cells
        .iter()
        .find(|cell| cell.reference == "B4")
        .expect("B4");
    assert_eq!(formula.formula.as_deref(), Some("SUM(B2:B3)"));
    assert_eq!(summary.auto_filter.as_deref(), Some("A1:B3"));
    assert!(!summary.tables.is_empty());
    assert!(!summary.charts.is_empty());
    std::fs::remove_file(path).ok();
}

#[test]
fn edits_cell_without_removing_unknown_parts_or_formula_types() {
    let operations = normalize_operations(
        Some(&json!([
            {"op":"add_sheet","name":"Data"},
            {"op":"set_cell","sheet":"Data","cell":"A1","value":"before"}
        ])),
        OfficeFormat::Xlsx,
        false,
    )
    .expect("create operations");
    let mut members = create_xlsx(&operations).expect("create").members;
    members.insert("custom/preserve.bin".into(), b"keep".to_vec());
    let source = temp_path("xlsx");
    publish_package(&members, &source, OfficeFormat::Xlsx, false, None, None).expect("publish");
    let package = OfficePackage::open(&source, Some(OfficeFormat::Xlsx)).expect("package");
    let edit = normalize_operations(
        Some(&json!([
            {"op":"set_cell","sheet":"Data","cell":"A1","value":"after"},
            {"op":"set_cell","sheet":"Data","cell":"A2","value":"1+1","value_type":"formula"}
        ])),
        OfficeFormat::Xlsx,
        true,
    )
    .expect("edit operations");
    let result = edit_xlsx(&package, &edit).expect("edit");
    assert_eq!(
        result.members.get("custom/preserve.bin").map(Vec::as_slice),
        Some(b"keep".as_slice())
    );
    assert!(
        std::str::from_utf8(&result.members["xl/worksheets/sheet1.xml"])
            .expect("xml")
            .contains("<f>1+1</f>")
    );
    std::fs::remove_file(source).ok();
}
