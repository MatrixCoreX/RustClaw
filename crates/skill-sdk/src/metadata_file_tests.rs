use super::*;

#[cfg(any(unix, windows))]
fn link(source: &Path, target: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, target).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(source, target).unwrap();
}

#[cfg(any(unix, windows))]
#[test]
fn metadata_regular_json_keeps_complete_content_without_new_length_limit() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("record.json");
    let expected = vec![123456789_u64; 500_000];
    let bytes = serde_json::to_vec(&expected).unwrap();
    assert!(bytes.len() > 4 * 1024 * 1024);
    fs::write(&target, &bytes).unwrap();
    assert_eq!(read(&target).unwrap(), bytes);
    assert_eq!(json::<Vec<u64>>(&target).unwrap(), expected);
    assert_eq!(read_optional(&target).unwrap().unwrap(), bytes);
    assert_eq!(fs::read(target).unwrap(), bytes);
}

#[cfg(any(unix, windows))]
#[test]
fn metadata_missing_optional_and_malformed_json_keep_machine_errors() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("record.json");
    assert!(read_optional(&target).unwrap().is_none());
    assert_eq!(read(&target).unwrap_err().code, "io_failed");
    fs::write(&target, b"not JSON").unwrap();
    assert_eq!(
        json::<serde_json::Value>(&target).unwrap_err().code,
        "json_invalid"
    );
}

#[cfg(any(unix, windows))]
#[test]
fn metadata_leaf_link_is_not_missing_and_never_changes_its_target() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("record.json");
    let outside = root.path().join("outside.json");
    fs::write(&outside, b"original").unwrap();
    link(&outside, &target);
    assert_eq!(
        read_optional(&target).unwrap_err().code,
        "metadata_read_target_invalid"
    );
    assert_eq!(fs::read(&outside).unwrap(), b"original");
    fs::remove_file(&outside).unwrap();
    assert_eq!(
        read_optional(&target).unwrap_err().code,
        "metadata_read_target_invalid"
    );
}

#[cfg(any(unix, windows))]
#[test]
fn metadata_descriptor_rejects_leaf_link_even_after_an_earlier_regular_type_check() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("record.json");
    let outside = root.path().join("outside.json");
    fs::write(&target, b"previous").unwrap();
    fs::write(&outside, b"protected").unwrap();
    assert!(fs::symlink_metadata(&target).unwrap().is_file());
    fs::remove_file(&target).unwrap();
    link(&outside, &target);
    assert_eq!(
        open_descriptor(&target, LeafLinks::Reject)
            .unwrap_err()
            .sdk(&target)
            .code,
        "metadata_read_target_invalid"
    );
    assert_eq!(fs::read(outside).unwrap(), b"protected");
}

#[cfg(any(unix, windows))]
#[test]
fn metadata_source_manifest_retains_regular_link_and_utf8_error_contract() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("selected.toml");
    let actual = root.path().join("actual.toml");
    fs::write(&actual, b"selected = true").unwrap();
    link(&actual, &target);
    assert_eq!(
        manifest_text(&target, LeafLinks::Follow).unwrap(),
        "selected = true"
    );
    assert_eq!(
        manifest_text(&target, LeafLinks::Reject).unwrap_err().code,
        "metadata_read_target_invalid"
    );
    fs::write(&actual, [0xff]).unwrap();
    assert_eq!(
        manifest_text(&target, LeafLinks::Follow).unwrap_err().code,
        "manifest_read_failed"
    );
}

#[cfg(unix)]
#[test]
fn metadata_descriptor_rejects_fifo_after_type_check_for_both_link_policies() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("record.json");
    fs::write(&target, b"previous").unwrap();
    assert!(fs::symlink_metadata(&target).unwrap().is_file());
    fs::remove_file(&target).unwrap();
    let name = CString::new(target.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    for links in [LeafLinks::Reject, LeafLinks::Follow] {
        assert_eq!(
            open_descriptor(&target, links)
                .unwrap_err()
                .sdk(&target)
                .code,
            "metadata_read_target_invalid"
        );
    }
}
