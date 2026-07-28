use std::io::Write;

use sha2::{Digest, Sha256};
use tempfile::tempdir;

use super::*;

fn artifact(bytes: &[u8]) -> PlatformArtifact {
    PlatformArtifact {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        sha256: hex::encode(Sha256::digest(bytes)),
        source_path: None,
        url: Some("https://packages.example.invalid/demo".to_string()),
        executable: true,
        size_bytes: Some(bytes.len() as u64),
        archive: None,
    }
}

#[test]
fn verified_download_cache_works_without_network_or_credentials() {
    let root = tempdir().expect("tempdir");
    let bytes = b"verified fixture";
    let artifact = artifact(bytes);
    let cached = root.path().join("cache/downloads").join(&artifact.sha256);
    std::fs::create_dir_all(cached.parent().expect("cache parent")).expect("cache directory");
    std::fs::write(&cached, bytes).expect("cache fixture");

    let resolved = resolve_source(&artifact, root.path(), &root.path().join("cache"), false)
        .expect("verified cache");
    assert_eq!(resolved, cached);
}

#[test]
fn zip_extraction_is_confined_to_runtime_and_installs_declared_entrypoint() {
    let root = tempdir().expect("tempdir");
    let archive_path = root.path().join("package.zip");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&archive_path).expect("zip file"));
    writer
        .start_file(
            "runtime/bin/demo",
            zip::write::SimpleFileOptions::default().unix_permissions(0o755),
        )
        .expect("zip entry");
    writer
        .write_all(b"#!/bin/sh\necho ok\n")
        .expect("zip content");
    writer.finish().expect("finish zip");
    let bytes = std::fs::read(&archive_path).expect("archive bytes");
    let mut artifact = artifact(&bytes);
    artifact.source_path = Some("package.zip".to_string());
    artifact.url = None;
    artifact.archive = Some(ArchiveFormat::Zip);
    let staging = root.path().join("staging");
    std::fs::create_dir_all(&staging).expect("staging");

    let installed =
        install(&archive_path, &artifact, &staging, "runtime/bin/demo").expect("extract archive");
    assert!(installed.starts_with(std::fs::canonicalize(&staging).expect("canonical staging")));
    assert_eq!(
        std::fs::read(installed).expect("installed bytes"),
        b"#!/bin/sh\necho ok\n"
    );
}

#[test]
fn archive_traversal_and_links_are_rejected() {
    let root = tempdir().expect("tempdir");
    let archive_path = root.path().join("malicious.zip");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&archive_path).expect("zip file"));
    writer
        .start_file(
            "../escape",
            zip::write::SimpleFileOptions::default().unix_permissions(0o755),
        )
        .expect("zip entry");
    writer.write_all(b"escape").expect("zip content");
    writer.finish().expect("finish zip");
    let bytes = std::fs::read(&archive_path).expect("archive bytes");
    let mut artifact = artifact(&bytes);
    artifact.archive = Some(ArchiveFormat::Zip);
    let staging = root.path().join("staging");
    std::fs::create_dir_all(&staging).expect("staging");

    let error = install(&archive_path, &artifact, &staging, "runtime/bin/demo")
        .expect_err("archive traversal must fail");
    assert_eq!(error.code, "prebuilt_archive_path_unsafe");
    assert!(!root.path().join("escape").exists());
}

#[test]
fn tar_gz_links_are_rejected_before_extraction() {
    let root = tempdir().expect("tempdir");
    let archive_path = root.path().join("malicious.tar.gz");
    let archive_file = std::fs::File::create(&archive_path).expect("tar.gz file");
    let encoder = flate2::write::GzEncoder::new(archive_file, flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_mode(0o777);
    header.set_cksum();
    header
        .set_link_name("../../escape")
        .expect("symlink target");
    builder
        .append_data(&mut header, "runtime/bin/demo", std::io::empty())
        .expect("append symlink");
    let encoder = builder.into_inner().expect("finish tar");
    encoder.finish().expect("finish gzip");

    let bytes = std::fs::read(&archive_path).expect("archive bytes");
    let mut artifact = artifact(&bytes);
    artifact.source_path = Some("malicious.tar.gz".to_string());
    artifact.url = None;
    artifact.archive = Some(ArchiveFormat::TarGz);
    let staging = root.path().join("staging");
    std::fs::create_dir_all(&staging).expect("staging");
    let error = install(&archive_path, &artifact, &staging, "runtime/bin/demo")
        .expect_err("tar symlink must fail");
    assert_eq!(error.code, "prebuilt_archive_link_forbidden");
    assert!(!root.path().join("escape").exists());
}
