use super::*;
use crate::model::OfficeFormat;
use crate::package::OfficePackage;
use crate::test_support::{pptx_fixture, temp_path};

#[test]
fn reads_multilingual_slide_structure() {
    let path = temp_path("pptx");
    pptx_fixture(&path);
    let package = OfficePackage::open(&path, Some(OfficeFormat::Pptx)).expect("package");
    let presentation = read_presentation(&package).expect("presentation");
    assert_eq!(presentation.slides.len(), 1);
    assert_eq!(presentation.slides[0].title.as_deref(), Some("产品路线图"));
    assert!(presentation.slides[0]
        .text
        .iter()
        .any(|text| text == "Next milestone"));
    std::fs::remove_file(path).ok();
}
