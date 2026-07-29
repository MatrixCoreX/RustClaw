use std::fs::File;
use std::io::Write;
use std::time::Duration;

use super::{
    extract_safe_archive, inspect_safe_archive, read_safe_archive_member, SafeArchiveLimits,
};

fn limits() -> SafeArchiveLimits {
    SafeArchiveLimits {
        max_entries: 20,
        max_expanded_bytes: 1024 * 1024,
        max_depth: 8,
        max_elapsed: Duration::from_secs(10),
    }
}

#[test]
fn zip_is_preflighted_then_extracted() {
    let root = tempfile::tempdir().expect("root");
    let archive_path = root.path().join("safe.zip");
    let mut writer = zip::ZipWriter::new(File::create(&archive_path).expect("archive"));
    writer
        .start_file("nested/file.txt", zip::write::SimpleFileOptions::default())
        .expect("entry");
    writer.write_all(b"safe").expect("content");
    writer.finish().expect("finish");

    let inspection = inspect_safe_archive(&archive_path, limits()).expect("inspect");
    assert_eq!(inspection.entry_count, 1);
    assert_eq!(inspection.expanded_bytes, 4);

    let destination = root.path().join("out");
    let extracted = extract_safe_archive(&archive_path, &destination, limits()).expect("extract");
    assert_eq!(extracted, inspection);
    assert_eq!(
        std::fs::read_to_string(destination.join("nested/file.txt")).unwrap(),
        "safe"
    );
}

#[test]
fn zip_traversal_is_rejected_before_destination_creation() {
    let root = tempfile::tempdir().expect("root");
    let archive_path = root.path().join("unsafe.zip");
    let mut writer = zip::ZipWriter::new(File::create(&archive_path).expect("archive"));
    writer
        .start_file("../escape.txt", zip::write::SimpleFileOptions::default())
        .expect("entry");
    writer.write_all(b"unsafe").expect("content");
    writer.finish().expect("finish");
    let destination = root.path().join("out");

    let error = extract_safe_archive(&archive_path, &destination, limits())
        .expect_err("traversal must fail");
    assert_eq!(error.code, "archive_path_unsafe");
    assert!(!destination.exists());
    assert!(!root.path().join("escape.txt").exists());
}

#[test]
fn tar_links_are_rejected_during_preflight() {
    let root = tempfile::tempdir().expect("root");
    let archive_path = root.path().join("unsafe.tar.gz");
    let encoder = flate2::write::GzEncoder::new(
        File::create(&archive_path).expect("archive"),
        flate2::Compression::default(),
    );
    let mut builder = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_mode(0o777);
    header.set_cksum();
    builder
        .append_link(&mut header, "link", "../outside")
        .expect("link");
    let encoder = builder.into_inner().expect("builder");
    encoder.finish().expect("finish");

    let error = inspect_safe_archive(&archive_path, limits()).expect_err("link must fail");
    assert_eq!(error.code, "archive_entry_type_forbidden");
}

#[test]
fn budgets_are_enforced_during_preflight() {
    let root = tempfile::tempdir().expect("root");
    let archive_path = root.path().join("large.zip");
    let mut writer = zip::ZipWriter::new(File::create(&archive_path).expect("archive"));
    writer
        .start_file("large.txt", zip::write::SimpleFileOptions::default())
        .expect("entry");
    writer.write_all(&[0_u8; 128]).expect("content");
    writer.finish().expect("finish");
    let mut strict = limits();
    strict.max_expanded_bytes = 64;

    let error = inspect_safe_archive(&archive_path, strict).expect_err("budget must fail");
    assert_eq!(error.code, "archive_budget_exceeded");
}

#[test]
fn large_member_streams_to_recoverable_artifact() {
    let root = tempfile::tempdir().expect("root");
    let archive_path = root.path().join("large-member.zip");
    let source = "safe archive body\n".repeat(20_000);
    let mut writer = zip::ZipWriter::new(File::create(&archive_path).expect("archive"));
    writer
        .start_file("large.txt", zip::write::SimpleFileOptions::default())
        .expect("entry");
    writer.write_all(source.as_bytes()).expect("content");
    writer.finish().expect("finish");
    let spill = crate::ArtifactSpill::new(root.path().join("artifacts"), "archive")
        .expect("artifact spill");

    let result = read_safe_archive_member(&archive_path, "large.txt", limits(), 1024, Some(&spill))
        .expect("bounded member");

    assert!(!result.complete);
    assert_eq!(result.artifacts.len(), 1);
    assert_eq!(
        std::fs::read(&result.artifacts[0].path).unwrap(),
        source.as_bytes()
    );
}
