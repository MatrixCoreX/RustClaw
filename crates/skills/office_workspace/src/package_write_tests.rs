use super::*;
use crate::test_support::{docx_fixture, temp_path};

#[test]
fn publishes_only_after_reopen_validation() {
    let source = temp_path("docx");
    let output = temp_path("docx");
    docx_fixture(&source);
    let package = OfficePackage::open(&source, Some(OfficeFormat::Docx)).expect("source");
    let result = publish_package(
        &package.members,
        &output,
        OfficeFormat::Docx,
        false,
        None,
        None,
    )
    .expect("publish");
    assert!(output.exists());
    assert_eq!(
        OfficePackage::open(&output, Some(OfficeFormat::Docx))
            .expect("output")
            .source
            .sha256,
        result.output_sha256
    );
    std::fs::remove_file(source).ok();
    std::fs::remove_file(output).ok();
}

#[test]
fn failed_validation_leaves_no_output() {
    let output = temp_path("docx");
    let members = BTreeMap::from([("[Content_Types].xml".to_string(), b"<Types/>".to_vec())]);
    let error = publish_package(&members, &output, OfficeFormat::Docx, false, None, None)
        .expect_err("invalid package");
    assert_eq!(error.code, "validation_failed");
    assert!(!output.exists());
}
