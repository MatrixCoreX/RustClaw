use super::super::publication;
use super::{atomic_write, entries};
use std::fs;

#[test]
fn sdk_publication_partial_write_failure_preserves_original_and_cleans_staging() {
    use std::io::Write;
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("current.json");
    fs::write(&target, b"original").unwrap();
    let error = publication::write_with(&target, |file| {
        file.write_all(b"partial replacement")?;
        Err(std::io::Error::other("fixture_write_failure"))
    })
    .unwrap_err();
    assert_eq!(error.code, "io_failed");
    assert_eq!(fs::read(&target).unwrap(), b"original");
    assert_eq!(
        entries(root.path()),
        vec![target.file_name().unwrap().to_os_string()]
    );
}

#[test]
fn sdk_publication_panic_preserves_original_and_cleans_staging() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("current.json");
    fs::write(&target, b"original").unwrap();
    let failed = std::panic::catch_unwind(|| {
        publication::write_with(&target, |_| panic!("fixture_write_panic"))
    });
    assert!(failed.is_err());
    assert_eq!(fs::read(&target).unwrap(), b"original");
    assert_eq!(
        entries(root.path()),
        vec![target.file_name().unwrap().to_os_string()]
    );
}

#[test]
fn sdk_publication_rejects_independent_replacement_during_staging() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("current.json");
    let saved = root.path().join("independent.json");
    fs::write(&target, b"original").unwrap();
    let error = publication::write_with(&target, |_| {
        fs::rename(&target, &saved)?;
        fs::write(&target, b"independent replacement")
    })
    .unwrap_err();
    assert_eq!(error.code, "atomic_write_target_changed");
    assert_eq!(fs::read(&target).unwrap(), b"independent replacement");
    assert_eq!(fs::read(&saved).unwrap(), b"original");
    assert_eq!(
        entries(root.path()),
        vec![
            target.file_name().unwrap().to_os_string(),
            saved.file_name().unwrap().to_os_string()
        ]
    );
}

#[test]
fn sdk_publication_rejects_independent_in_place_edit_during_staging() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("current.json");
    fs::write(&target, b"original").unwrap();
    let error =
        publication::write_with(&target, |_| fs::write(&target, b"independent edit")).unwrap_err();
    assert_eq!(error.code, "atomic_write_target_changed");
    assert_eq!(fs::read(&target).unwrap(), b"independent edit");
    assert_eq!(
        entries(root.path()),
        vec![target.file_name().unwrap().to_os_string()]
    );
}

#[test]
fn sdk_publication_does_not_overwrite_a_new_target_created_during_staging() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("current.json");
    let error =
        publication::write_with(&target, |_| fs::write(&target, b"independent new pointer"))
            .unwrap_err();
    assert_eq!(error.code, "atomic_write_target_changed");
    assert_eq!(fs::read(&target).unwrap(), b"independent new pointer");
    assert_eq!(
        entries(root.path()),
        vec![target.file_name().unwrap().to_os_string()]
    );
}

#[cfg(unix)]
#[test]
fn sdk_publication_stages_bytes_privately_before_restoring_final_permissions() {
    use std::os::unix::fs::MetadataExt;
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("current.json");
    publication::write_with(&target, |file| {
        assert_eq!(file.metadata()?.mode() & 0o7777, 0o600);
        Ok(())
    })
    .unwrap();
}

#[cfg(unix)]
#[test]
fn sdk_publication_rejects_fifos_without_opening_or_replacing_them() {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::FileTypeExt;
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("current.json");
    let name = std::ffi::CString::new(target.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(atomic_write(&target, b"replacement").is_err());
    assert!(fs::symlink_metadata(&target).unwrap().file_type().is_fifo());
    assert_eq!(
        entries(root.path()),
        vec![target.file_name().unwrap().to_os_string()]
    );
}
