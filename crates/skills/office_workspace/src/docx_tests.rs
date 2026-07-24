use super::*;
use crate::model::OfficeFormat;
use crate::package::OfficePackage;
use crate::test_support::{docx_fixture, temp_path};

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
