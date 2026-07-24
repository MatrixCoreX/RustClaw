use super::*;
use crate::test_support::{docx_fixture, temp_path};
use std::time::Duration;

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

#[test]
fn cleanup_respects_age_and_exact_temp_package_names() {
    let output = temp_path("docx");
    let parent = output.parent().expect("parent");
    let file_name = output
        .file_name()
        .and_then(|name| name.to_str())
        .expect("name");
    let stale = parent.join(format!(".{file_name}.rustclaw-{}.tmp", Uuid::new_v4()));
    let recent = parent.join(format!(".{file_name}.rustclaw-{}.tmp", Uuid::new_v4()));
    let unrelated = parent.join(format!(".{file_name}.rustclaw-not-a-uuid.tmp"));
    std::fs::write(&stale, b"stale").expect("stale");
    std::fs::write(&recent, b"recent").expect("recent");
    std::fs::write(&unrelated, b"unrelated").expect("unrelated");
    let modified = std::fs::metadata(&stale)
        .and_then(|metadata| metadata.modified())
        .expect("modified");
    let recent_evidence =
        cleanup_abandoned_temp_packages(parent, file_name, modified, Duration::from_secs(3_600));
    assert_eq!(recent_evidence.removed, 0);
    assert!(stale.exists());
    assert!(recent.exists());
    let evidence = cleanup_abandoned_temp_packages(
        parent,
        file_name,
        modified + Duration::from_secs(7_200),
        Duration::from_secs(3_600),
    );
    assert_eq!(evidence.removed, 2);
    assert!(evidence.errors.is_empty());
    assert!(!stale.exists());
    assert!(!recent.exists());
    assert!(unrelated.exists());
    std::fs::remove_file(unrelated).ok();
}
