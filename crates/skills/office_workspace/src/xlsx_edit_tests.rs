use super::*;
use crate::model::OfficeFormat;
use crate::operations::normalize_operations;
use crate::package::OfficePackage;
use crate::package_write::publish_package;
use crate::test_support::temp_path;
use crate::xlsx::read_workbook;
use crate::xlsx_write::create_xlsx;

fn source_workbook() -> (std::path::PathBuf, OfficePackage) {
    let operations = normalize_operations(
        Some(&json!([
            {"op":"add_sheet","name":"Data"},
            {"op":"set_range","sheet":"Data","range":"A1:A2","values":[["before"],["source"]]},
            {"op":"add_sheet","name":"Target"},
            {"op":"set_cell","sheet":"Target","cell":"A1","value":"keep"}
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
    (source, package)
}

#[test]
fn edits_cells_without_removing_unknown_parts_or_formula_types() {
    let (source, package) = source_workbook();
    let edit = normalize_operations(
        Some(&json!([
            {"op":"set_cell","sheet":"Data","cell":"A1","value":"after"},
            {"op":"set_cell","sheet":"Data","cell":"A3","value":"1+1","value_type":"formula"}
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
    let worksheet =
        std::str::from_utf8(&result.members["xl/worksheets/sheet1.xml"]).expect("worksheet");
    assert!(worksheet.contains("<row r=\"3\"><c r=\"A3\""));
    assert!(worksheet.contains("<f>1+1</f>"));
    std::fs::remove_file(source).ok();
}

#[test]
fn applies_consecutive_sheet_range_and_layout_edits() {
    let (source, package) = source_workbook();
    let edit = normalize_operations(
        Some(&json!([
            {"op":"copy_sheet","sheet":"Data","new_name":"Working"},
            {"op":"reorder_sheet","sheet":"Working","index":0},
            {"op":"rename_sheet","sheet":"Working","new_name":"Review"},
            {"op":"fill_range","sheet":"Review","range":"B1:B2","value":"filled"},
            {"op":"move_range","sheet":"Review","range":"B1:B2","target_sheet":"Target","target_cell":"C1"},
            {"op":"set_column_width","sheet":"Target","column":3,"width":24.5},
            {"op":"set_row_height","sheet":"Target","row":2,"height":31.0},
            {"op":"set_auto_filter","sheet":"Target","range":"A1:C10"},
            {"op":"merge_cells","sheet":"Target","range":"A10:B10"},
            {"op":"add_data_validation","sheet":"Target","range":"C1:C10","validation_type":"list","formula1":"\"filled,empty\""},
            {"op":"add_conditional_format","sheet":"Target","range":"C1:C10","formula":"C1=\"empty\""},
            {"op":"add_named_range","name":"ReviewCells","reference":"Target!$C$1:$C$2"},
            {"op":"delete_sheet","sheet":"Data"}
        ])),
        OfficeFormat::Xlsx,
        true,
    )
    .expect("edit operations");
    let result = edit_xlsx(&package, &edit).expect("edit");
    let output = temp_path("xlsx");
    publish_package(
        &result.members,
        &output,
        OfficeFormat::Xlsx,
        false,
        None,
        None,
    )
    .expect("publish");
    let reopened = OfficePackage::open(&output, Some(OfficeFormat::Xlsx)).expect("reopen");
    let workbook = read_workbook(&reopened).expect("read");
    assert_eq!(
        workbook
            .sheets
            .iter()
            .map(|sheet| sheet.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Review", "Target"]
    );
    let target = workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name == "Target")
        .expect("target");
    assert_eq!(
        target
            .cells
            .iter()
            .find(|cell| cell.reference == "C1")
            .and_then(|cell| cell.displayed_value.as_deref()),
        Some("filled")
    );
    let target_xml =
        std::str::from_utf8(&reopened.members["xl/worksheets/sheet2.xml"]).expect("target XML");
    assert!(target_xml.contains("<dataValidations count=\"1\">"));
    assert!(target_xml.contains("<conditionalFormatting sqref=\"C1:C10\">"));
    assert!(target_xml.contains("width=\"24.5\""));
    assert!(target_xml.contains("ht=\"31\""));
    assert!(
        target_xml.find("<autoFilter").expect("filter")
            < target_xml.find("<mergeCells").expect("merges")
    );
    assert!(
        target_xml.find("<mergeCells").expect("merges")
            < target_xml
                .find("<conditionalFormatting")
                .expect("conditional")
    );
    assert!(
        target_xml
            .find("<conditionalFormatting")
            .expect("conditional")
            < target_xml.find("<dataValidations").expect("validations")
    );
    assert!(reopened.members.contains_key("custom/preserve.bin"));
    std::fs::remove_file(source).ok();
    std::fs::remove_file(output).ok();
}

#[test]
fn adds_relationship_backed_objects_without_rebuilding_the_sheet() {
    let (source, package) = source_workbook();
    let image = temp_path("png");
    std::fs::write(
        &image,
        [
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H',
            b'D', b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, b'I', b'D', b'A', b'T', 0x08,
            0xd7, 0x63, 0xf8, 0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99,
            0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82,
        ],
    )
    .expect("image");
    let edit = normalize_operations(
        Some(&json!([
            {"op":"set_range","sheet":"Target","range":"A2:B3","values":[["label","value"],["x",42]]},
            {"op":"add_table","sheet":"Target","range":"A2:B3","name":"TargetTable"},
            {"op":"add_chart","sheet":"Target","range":"B2:B3","title":"Values","chart_type":"column"},
            {"op":"add_comment","sheet":"Target","cell":"A2","text":"reviewed"},
            {"op":"add_hyperlink","sheet":"Target","cell":"A3","url":"https://example.invalid/reference"},
            {"op":"add_image","sheet":"Target","cell":"D2","path":image.display().to_string(),"alt":"review status"}
        ])),
        OfficeFormat::Xlsx,
        true,
    )
    .expect("edit operations");
    let result = edit_xlsx(&package, &edit).expect("edit");
    let output = temp_path("xlsx");
    publish_package(
        &result.members,
        &output,
        OfficeFormat::Xlsx,
        false,
        None,
        None,
    )
    .expect("publish");
    let reopened = OfficePackage::open(&output, Some(OfficeFormat::Xlsx)).expect("reopen");
    let workbook = read_workbook(&reopened).expect("read");
    let target = workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name == "Target")
        .expect("target");
    assert_eq!(target.tables.len(), 1);
    assert_eq!(target.charts.len(), 1);
    assert_eq!(target.images.len(), 1);
    let comment = target
        .cells
        .iter()
        .find(|cell| cell.reference == "A2")
        .and_then(|cell| cell.comment.as_deref());
    assert_eq!(comment, Some("reviewed"));
    let hyperlink = target
        .cells
        .iter()
        .find(|cell| cell.reference == "A3")
        .and_then(|cell| cell.hyperlink.as_deref());
    assert_eq!(hyperlink, Some("https://example.invalid/reference"));
    let target_xml =
        std::str::from_utf8(&reopened.members["xl/worksheets/sheet2.xml"]).expect("target XML");
    assert!(
        target_xml.find("<hyperlinks").expect("hyperlinks")
            < target_xml.find("<drawing").expect("drawing")
    );
    assert!(
        target_xml.find("<drawing").expect("drawing")
            < target_xml.find("<legacyDrawing").expect("legacy drawing")
    );
    assert!(
        target_xml.find("<legacyDrawing").expect("legacy drawing")
            < target_xml.find("<tableParts").expect("table parts")
    );
    assert!(reopened.members.contains_key("custom/preserve.bin"));
    std::fs::remove_file(source).ok();
    std::fs::remove_file(output).ok();
    std::fs::remove_file(image).ok();
}
