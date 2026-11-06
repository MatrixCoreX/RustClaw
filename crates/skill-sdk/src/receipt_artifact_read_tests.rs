use super::*;

#[cfg(unix)]
#[path = "receipt_artifact_read_unix_tests.rs"]
mod unix_tests;

fn link(source: &Path, target: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, target).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(source, target).unwrap();
}

fn assert_artifact_invalid<T: std::fmt::Debug>(result: SkillSdkResult<T>) {
    let error = result.unwrap_err();
    assert_eq!(error.code, "artifact_read_target_invalid");
    assert_eq!(error.message_key, "skill_sdk.artifact_read_target_invalid");
}

#[test]
fn artifact_digest_regular_file_keeps_all_bytes_and_file_identity() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact");
    let bytes = (0..3 * 1024 * 1024 + 37)
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    fs::write(&path, &bytes).unwrap();
    let original = fs::metadata(&path).unwrap();
    let expected = hex::encode(Sha256::digest(&bytes));
    assert_eq!(digest_file(&path).unwrap(), expected);
    assert_eq!(digest_verified_file(&path).unwrap(), expected);
    let after = fs::metadata(&path).unwrap();
    assert_eq!(after.len(), original.len());
    assert_eq!(after.modified().unwrap(), original.modified().unwrap());
    assert_eq!(
        after.permissions().readonly(),
        original.permissions().readonly()
    );
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn artifact_digest_empty_file_keeps_standard_sha256() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("empty");
    fs::write(&path, []).unwrap();
    let expected = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    assert_eq!(digest_file(&path).unwrap(), expected);
    assert_eq!(digest_verified_file(&path).unwrap(), expected);
}

#[test]
fn artifact_digest_missing_path_keeps_existing_io_error() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("missing");
    for result in [digest_file(&path), digest_verified_file(&path)] {
        let error = result.unwrap_err();
        assert_eq!(error.code, "artifact_read_failed");
        assert_eq!(error.message_key, "skill_sdk.artifact_read_failed");
    }
}

#[test]
fn artifact_digest_directory_is_rejected_before_reading() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("protected"), b"unchanged").unwrap();
    assert_artifact_invalid(digest_file(root.path()));
    assert_artifact_invalid(digest_verified_file(root.path()));
    assert_eq!(
        fs::read(root.path().join("protected")).unwrap(),
        b"unchanged"
    );
}

#[test]
fn artifact_digest_public_regular_source_link_remains_supported() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let selected = root.path().join("selected");
    fs::write(&source, b"explicit source link").unwrap();
    link(&source, &selected);
    assert_eq!(
        digest_file(&selected).unwrap(),
        digest_file(&source).unwrap()
    );
    assert_eq!(fs::read_link(selected).unwrap(), source);
}

#[test]
fn artifact_digest_public_dangling_source_link_keeps_io_error() {
    let root = tempfile::tempdir().unwrap();
    let selected = root.path().join("selected");
    link(&root.path().join("absent"), &selected);
    assert_eq!(
        digest_file(&selected).unwrap_err().code,
        "artifact_read_failed"
    );
    assert!(fs::symlink_metadata(selected)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn artifact_digest_verified_leaf_link_rejected_after_an_earlier_type_check() {
    let root = tempfile::tempdir().unwrap();
    let selected = root.path().join("verified");
    let outside = root.path().join("outside");
    fs::write(&selected, b"earlier checked file").unwrap();
    fs::write(&outside, b"protected bytes").unwrap();
    assert!(fs::symlink_metadata(&selected).unwrap().is_file());
    fs::remove_file(&selected).unwrap();
    link(&outside, &selected);
    assert_artifact_invalid(digest_verified_file(&selected));
    assert_eq!(fs::read(outside).unwrap(), b"protected bytes");
}

#[test]
fn artifact_digest_verified_dangling_leaf_link_is_invalid_not_missing() {
    let root = tempfile::tempdir().unwrap();
    let selected = root.path().join("verified");
    link(&root.path().join("absent"), &selected);
    assert_artifact_invalid(digest_verified_file(&selected));
    assert!(fs::symlink_metadata(selected)
        .unwrap()
        .file_type()
        .is_symlink());
}
