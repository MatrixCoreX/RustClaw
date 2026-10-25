use super::*;
use crate::model::OfficeFormat;
use crate::package::OfficePackage;
use crate::test_support::{docx_fixture, temp_path, write_package};

#[test]
fn docx_references_keep_text_runs_and_table_cells_intact() {
    let mut blocks = Vec::new();
    let mut tables = Vec::new();
    parse_part(
        "<document><tbl><tr><tc><p><r><t>A &amp; B &#x1F642; &amp;lt;</t></r></p></tc></tr></tbl></document>",
        "word/document.xml",
        &mut blocks,
        &mut tables,
    )
    .expect("DOCX reference text");
    assert_eq!(blocks[0].text, "A & B 🙂 &lt;");
    assert_eq!(blocks[0].runs[0].text, "A & B 🙂 &lt;");
    assert_eq!(tables[0].rows[0][0], "A & B 🙂 &lt;");

    let error = parse_part(
        "<document><p><r><t>A &unknown; B</t></r></p></document>",
        "word/document.xml",
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .expect_err("unknown entities remain errors");
    assert_eq!(error.code, "malformed_xml");
}

#[test]
fn reads_multilingual_docx_blocks_and_table() {
    let path = temp_path("docx");
    docx_fixture(&path);
    let package = OfficePackage::open(&path, Some(OfficeFormat::Docx)).expect("package");
    let evidence = read_docx(&package).expect("docx");
    assert!(evidence.blocks.iter().any(|block| block.text == "季度报告"));
    assert!(evidence
        .blocks
        .iter()
        .any(|block| block.text == "Hello résumé"));
    assert_eq!(evidence.tables[0].rows[0], ["项目", "42"]);
    std::fs::remove_file(path).ok();
}

#[test]
fn object_ids_are_local_to_each_word_package_part() {
    let path = temp_path("docx");
    write_package(
        &path,
        &[
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
            ),
            (
                "word/document.xml",
                r#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>one</w:t></w:r></w:p><w:p><w:r><w:t>two</w:t></w:r></w:p><w:sectPr/></w:body></w:document>"#,
            ),
            (
                "word/header1.xml",
                r#"<?xml version="1.0"?><w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>header</w:t></w:r></w:p></w:hdr>"#,
            ),
        ],
    );
    let package = OfficePackage::open(&path, Some(OfficeFormat::Docx)).expect("package");
    let evidence = read_docx(&package).expect("docx");
    assert!(evidence
        .blocks
        .iter()
        .any(|block| block.id == "word_document_paragraph_2"));
    assert!(evidence
        .blocks
        .iter()
        .any(|block| block.id == "word_header1_paragraph_1"));
    std::fs::remove_file(path).ok();
}
