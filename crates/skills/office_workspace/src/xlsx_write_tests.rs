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
fn creates_standard_comments_images_and_sheet_rules() {
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
    let operations = normalize_operations(
        Some(&json!([
            {"op":"add_sheet","name":"Review"},
            {"op":"set_range","sheet":"Review","range":"A1:A2","values":[["status"],["ready"]]},
            {"op":"add_comment","sheet":"Review","cell":"A2","text":"verified note"},
            {"op":"add_image","sheet":"Review","cell":"C2","path":image.display().to_string(),"alt":"status mark"},
            {"op":"add_data_validation","sheet":"Review","range":"A2:A20","validation_type":"list","formula1":"\"ready,blocked\""},
            {"op":"add_conditional_format","sheet":"Review","range":"A2:A20","formula":"A2=\"blocked\""}
        ])),
        OfficeFormat::Xlsx,
        false,
    )
    .expect("operations");
    let result = create_xlsx(&operations).expect("create");
    let worksheet =
        std::str::from_utf8(&result.members["xl/worksheets/sheet1.xml"]).expect("worksheet");
    assert!(worksheet.contains("<dataValidations count=\"1\">"));
    assert!(worksheet.contains("<conditionalFormatting sqref=\"A2:A20\">"));
    assert!(worksheet.contains("<legacyDrawing r:id=\"rIdVml1\"/>"));
    assert!(result.members.contains_key("xl/comments1.xml"));
    assert!(result.members.contains_key("xl/drawings/vmlDrawing1.vml"));
    assert!(result.members.contains_key("xl/media/image1.png"));

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
    let review = &workbook.sheets[0];
    let annotated = review
        .cells
        .iter()
        .find(|cell| cell.reference == "A2")
        .expect("A2");
    assert_eq!(annotated.comment.as_deref(), Some("verified note"));
    assert_eq!(review.images, vec!["xl/media/image1.png"]);
    std::fs::remove_file(image).ok();
    std::fs::remove_file(path).ok();
}
