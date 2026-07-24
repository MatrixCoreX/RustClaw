use super::*;
use crate::docx::read_docx;
use crate::model::OfficeFormat;
use crate::operations::normalize_operations;
use crate::package::OfficePackage;
use crate::package_write::publish_package;
use crate::test_support::temp_path;

#[test]
fn creates_and_reopens_structured_docx() {
    let operations = normalize_operations(
        Some(&json!([
            {"op":"set_properties","title":"季度报告","creator":"RustClaw"},
            {"op":"set_header","text":"内部资料"},
            {"op":"set_footer","text":"第 1 页"},
            {"op":"add_heading","text":"Résumé","level":1},
            {"op":"add_paragraph","text":"跨语言正文"},
            {"op":"add_table","rows":[["项目","数值"],["收入",42]]},
            {"op":"add_page_break"}
        ])),
        OfficeFormat::Docx,
        false,
    )
    .expect("operations");
    let result = create_docx(&operations).expect("create");
    let path = temp_path("docx");
    publish_package(
        &result.members,
        &path,
        OfficeFormat::Docx,
        false,
        None,
        None,
    )
    .expect("publish");
    let package = OfficePackage::open(&path, Some(OfficeFormat::Docx)).expect("package");
    let evidence = read_docx(&package).expect("read");
    assert!(evidence.blocks.iter().any(|block| block.text == "Résumé"));
    assert!(evidence.blocks.iter().any(|block| block.text == "内部资料"));
    assert!(evidence.tables.iter().any(|table| table.rows[1][1] == "42"));
    std::fs::remove_file(path).ok();
}

#[test]
fn edits_selected_block_and_preserves_unknown_part() {
    let create = normalize_operations(
        Some(&json!([
            {"op":"add_heading","text":"Original","level":1},
            {"op":"add_paragraph","text":"Keep structure"}
        ])),
        OfficeFormat::Docx,
        false,
    )
    .expect("create operations");
    let mut members = create_docx(&create).expect("create").members;
    members.insert("custom/data.bin".into(), b"preserve-me".to_vec());
    let edit = normalize_operations(
        Some(&json!([
            {"op":"replace_block","block_id":"word_document_paragraph_1","text":"Revised"}
        ])),
        OfficeFormat::Docx,
        true,
    )
    .expect("edit operations");
    let result = edit_docx(&members, &edit).expect("edit");
    assert_eq!(
        result.members.get("custom/data.bin").map(Vec::as_slice),
        Some(b"preserve-me".as_slice())
    );
    let xml = std::str::from_utf8(&result.members["word/document.xml"]).expect("xml");
    assert!(xml.contains("Revised"));
    assert!(!xml.contains("Original"));
}
