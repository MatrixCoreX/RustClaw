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

#[test]
fn style_header_footer_and_section_edits_keep_valid_structure() {
    let create = normalize_operations(
        Some(&json!([
            {"op":"add_paragraph","text":"Body"}
        ])),
        OfficeFormat::Docx,
        false,
    )
    .expect("create operations");
    let members = create_docx(&create).expect("create").members;
    let edit = normalize_operations(
        Some(&json!([
            {"op":"set_block_style","block_id":"word_document_paragraph_1","style":"Heading2"},
            {"op":"set_header","text":"Header"},
            {"op":"set_footer","text":"Footer"},
            {"op":"set_section","orientation":"landscape"}
        ])),
        OfficeFormat::Docx,
        true,
    )
    .expect("edit operations");
    let result = edit_docx(&members, &edit).expect("edit");
    let document = std::str::from_utf8(&result.members["word/document.xml"]).expect("document");
    assert!(document.contains("<w:p><w:pPr><w:pStyle w:val=\"Heading2\"/></w:pPr>"));
    assert!(!document.contains("</w:pPr>>"));
    assert!(document.contains("<w:headerReference"));
    assert!(document.contains("<w:footerReference"));
    assert!(document.contains("w:orient=\"landscape\""));
    let content_types = std::str::from_utf8(&result.members["[Content_Types].xml"]).expect("types");
    assert!(content_types.contains("/word/header1.xml"));
    assert!(content_types.contains("/word/footer1.xml"));
}

#[test]
fn appends_relationship_and_note_parts_without_collisions() {
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
    let create = normalize_operations(
        Some(&json!([
            {"op":"add_paragraph","text":"Base"},
            {"op":"add_image","path":image.display().to_string(),"alt":"first"},
            {"op":"add_footnote","text":"first footnote"},
            {"op":"add_comment","text":"first target","comment":"first comment"}
        ])),
        OfficeFormat::Docx,
        false,
    )
    .expect("create operations");
    let members = create_docx(&create).expect("create").members;
    let edit = normalize_operations(
        Some(&json!([
            {"op":"add_image","path":image.display().to_string(),"alt":"second"},
            {"op":"add_hyperlink","text":"reference","url":"https://example.invalid/doc"},
            {"op":"add_footnote","text":"second footnote"},
            {"op":"add_endnote","text":"second endnote"},
            {"op":"add_comment","text":"second target","comment":"second comment"}
        ])),
        OfficeFormat::Docx,
        true,
    )
    .expect("edit operations");
    let result = edit_docx(&members, &edit).expect("edit");
    assert!(result.members.contains_key("word/media/image1.png"));
    assert!(result.members.contains_key("word/media/image2.png"));
    let relationships =
        std::str::from_utf8(&result.members["word/_rels/document.xml.rels"]).expect("rels");
    assert!(relationships.contains("TargetMode=\"External\""));
    let footnotes = std::str::from_utf8(&result.members["word/footnotes.xml"]).expect("footnotes");
    assert!(footnotes.contains("first footnote"));
    assert!(footnotes.contains("second footnote"));
    let comments = std::str::from_utf8(&result.members["word/comments.xml"]).expect("comments");
    assert!(comments.contains("first comment"));
    assert!(comments.contains("second comment"));
    let output = temp_path("docx");
    publish_package(
        &result.members,
        &output,
        OfficeFormat::Docx,
        false,
        None,
        None,
    )
    .expect("publish");
    let reopened = OfficePackage::open(&output, Some(OfficeFormat::Docx)).expect("reopen");
    let evidence = read_docx(&reopened).expect("read");
    assert!(evidence
        .blocks
        .iter()
        .any(|block| block.text.contains("second footnote")));
    assert!(evidence
        .blocks
        .iter()
        .any(|block| block.text.contains("second comment")));
    std::fs::remove_file(image).ok();
    std::fs::remove_file(output).ok();
}
