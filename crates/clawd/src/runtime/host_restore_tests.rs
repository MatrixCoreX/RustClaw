use super::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;

#[test]
fn restored_files_and_directory_modes_follow_the_held_inode_after_rename() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    fs::create_dir(root.join("original")).unwrap();
    let policy = HostFilesystemPolicy::without_private_authority();
    let parent = policy
        .open_directory(Some(&root), &root.join("original"))
        .unwrap();
    let child = policy
        .create_restore_directory_at(&parent, OsStr::new("child"))
        .unwrap();
    fs::rename(root.join("original"), root.join("moved")).unwrap();
    fs::create_dir_all(root.join("original/child")).unwrap();
    fs::set_permissions(
        root.join("original/child"),
        fs::Permissions::from_mode(0o750),
    )
    .unwrap();
    policy
        .restore_file_at(
            &child,
            OsStr::new("script"),
            b"held-inode",
            Some(HostAccessMode::try_from(0o751).unwrap()),
        )
        .unwrap();
    policy
        .finish_restore_directory(&child, Some(HostAccessMode::try_from(0o711).unwrap()))
        .unwrap();
    assert_eq!(
        fs::read(root.join("moved/child/script")).unwrap(),
        b"held-inode"
    );
    assert_eq!(
        fs::metadata(root.join("moved/child/script"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o751
    );
    assert_eq!(
        fs::metadata(root.join("moved/child"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o711
    );
    assert_eq!(
        fs::metadata(root.join("original/child"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o750
    );
    assert!(!root.join("original/child/script").exists());
}

#[test]
fn restore_methods_reject_non_child_names_before_any_mutation() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let policy = HostFilesystemPolicy::without_private_authority();
    let parent = policy.open_directory(Some(&root), &root).unwrap();
    for name in ["", ".", "..", "../escape", "one/two", "bad\0name"] {
        assert!(policy
            .create_restore_directory_at(&parent, OsStr::new(name))
            .is_err());
        assert!(policy
            .restore_file_at(&parent, OsStr::new(name), b"denied", None)
            .is_err());
    }
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
}

#[test]
fn restore_methods_readmit_private_directory_handles_before_mutation() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let private = root.join("private");
    fs::create_dir(&private).unwrap();
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
    let unprotected = HostFilesystemPolicy::without_private_authority();
    let handle = unprotected.open_directory(Some(&root), &private).unwrap();
    let policy = HostFilesystemPolicy::pin_directories(&[private.clone()], &root).unwrap();
    for error in [
        policy
            .create_restore_directory_at(&handle, OsStr::new("child"))
            .err()
            .unwrap(),
        policy
            .restore_file_at(&handle, OsStr::new("file"), b"denied", None)
            .unwrap_err(),
        policy
            .finish_restore_directory(&handle, Some(HostAccessMode::try_from(0o777).unwrap()))
            .unwrap_err(),
    ] {
        assert!(HostFilesystemPolicy::is_private_access_denied(&error));
    }
    assert_eq!(fs::read_dir(&private).unwrap().count(), 0);
    assert_eq!(
        fs::metadata(&private).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[test]
fn unrecorded_file_modes_use_private_creation_without_inheriting_a_replacement() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let path = root.join("file");
    fs::write(&path, b"replacement").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    let policy = HostFilesystemPolicy::without_private_authority();
    let parent = policy.open_directory(Some(&root), &root).unwrap();
    policy
        .restore_file_at(&parent, OsStr::new("file"), b"restored", None)
        .unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"restored");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
        0o600
    );
}
