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
fn stored_and_deflated_zip_entries_are_still_readable() {
    let root = tempfile::tempdir().expect("root");
    let archive_path = root.path().join("formats.zip");
    let mut writer = zip::ZipWriter::new(File::create(&archive_path).expect("archive"));
    for (name, method) in [
        ("stored.txt", zip::CompressionMethod::Stored),
        ("deflated.txt", zip::CompressionMethod::Deflated),
    ] {
        writer
            .start_file(
                name,
                zip::write::SimpleFileOptions::default().compression_method(method),
            )
            .expect("entry");
        writer.write_all(b"same content").expect("content");
    }
    writer.finish().expect("finish");
    let destination = root.path().join("out");
    let inspection = extract_safe_archive(&archive_path, &destination, limits()).expect("extract");
    assert_eq!(inspection.entry_count, 2);
    for name in ["stored.txt", "deflated.txt"] {
        assert_eq!(
            std::fs::read(destination.join(name)).unwrap(),
            b"same content"
        );
    }
}

#[test]
fn zip_symlinks_are_rejected_before_destination_creation() {
    let root = tempfile::tempdir().expect("root");
    let archive_path = root.path().join("link.zip");
    let outside = root.path().join("outside");
    std::fs::write(&outside, b"unchanged").unwrap();
    let mut writer = zip::ZipWriter::new(File::create(&archive_path).expect("archive"));
    writer
        .add_symlink(
            "link",
            "../outside",
            zip::write::SimpleFileOptions::default(),
        )
        .expect("symlink entry");
    writer.finish().expect("finish");
    let destination = root.path().join("out");
    let error = extract_safe_archive(&archive_path, &destination, limits()).expect_err("link");
    assert_eq!(error.code, "archive_entry_type_forbidden");
    assert!(!destination.exists());
    assert_eq!(std::fs::read(outside).unwrap(), b"unchanged");
}

#[cfg(unix)]
#[test]
fn zip_extraction_does_not_restore_privileged_archive_modes() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().expect("root");
    let archive_path = root.path().join("mode.zip");
    let mut writer = zip::ZipWriter::new(File::create(&archive_path).expect("archive"));
    writer
        .start_file("file", zip::write::SimpleFileOptions::default())
        .expect("entry");
    writer.write_all(b"safe").expect("content");
    writer.finish().expect("finish");
    // Set the untrusted central-directory mode directly: writers may mask it.
    let mut bytes = std::fs::read(&archive_path).unwrap();
    let central = bytes.windows(4).position(|b| b == b"PK\x01\x02").unwrap();
    bytes[central + 5] = 3; // Unix origin
    bytes[central + 38..central + 42].copy_from_slice(&(0o106755_u32 << 16).to_le_bytes());
    std::fs::write(&archive_path, bytes).unwrap();
    let mut archive = zip::ZipArchive::new(File::open(&archive_path).unwrap()).unwrap();
    let entry = archive.by_index(0).unwrap();
    assert_eq!(entry.system(), zip::System::Unix);
    // Check the raw fixture, since the patched reader already strips these bits.
    assert_eq!((entry.external_attributes() >> 16) & 0o7000, 0o6000);
    assert_eq!(entry.unix_mode().unwrap() & 0o7000, 0);

    let destination = root.path().join("out");
    extract_safe_archive(&archive_path, &destination, limits()).expect("extract");
    let mode = std::fs::metadata(destination.join("file"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o7000, 0);
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
