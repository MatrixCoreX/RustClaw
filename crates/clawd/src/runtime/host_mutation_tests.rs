use super::HostFilesystemPolicy;
use rustix::fs::FileType;
use std::fs;
use std::os::unix::fs::{symlink, DirBuilderExt, PermissionsExt};

#[test]
fn workspace_bounded_reads_preserve_empty_exact_binary_and_encoded_byte_boundaries() {
    use super::BoundedFileReadError;
    let (root, policy) = fixture();
    let path = root.path().join("public");
    let cases: &[&[u8]] = &[b"", b"ascii", &[0xc3, 0xa9], &[0xff, 0, 1]];
    for bytes in cases {
        fs::write(&path, *bytes).unwrap();
        assert_eq!(
            policy
                .read_file_bounded(Some(root.path()), &path, bytes.len())
                .unwrap(),
            *bytes
        );
    }
    fs::write(&path, b"more-than-the-budget").unwrap();
    match policy
        .read_file_bounded(Some(root.path()), &path, 3)
        .unwrap_err()
    {
        BoundedFileReadError::LimitExceeded {
            limit_bytes,
            observed_bytes_at_least,
        } => {
            assert_eq!(limit_bytes, 3);
            assert_eq!(observed_bytes_at_least, 4);
        }
        error => panic!("expected a typed read limit: {error:?}"),
    }
}

#[test]
fn workspace_bounded_reads_keep_missing_private_symlink_and_special_file_errors() {
    use super::BoundedFileReadError;
    use std::os::unix::ffi::OsStrExt;
    let (root, policy) = fixture();
    match policy
        .read_file_bounded(Some(root.path()), &root.path().join("missing"), 8)
        .unwrap_err()
    {
        BoundedFileReadError::Io(error) => assert_eq!(error.kind(), std::io::ErrorKind::NotFound),
        error => panic!("missing file is not size evidence: {error:?}"),
    }
    let private = root.path().join("private/marker");
    fs::write(&private, b"private").unwrap();
    let alias = root.path().join("private-alias");
    symlink(&private, &alias).unwrap();
    for path in [&private, &alias] {
        match policy
            .read_file_bounded(Some(root.path()), path, 0)
            .unwrap_err()
        {
            BoundedFileReadError::Io(error) => {
                assert!(HostFilesystemPolicy::is_private_access_denied(&error))
            }
            error => panic!("private denial must remain typed IO: {error:?}"),
        }
    }
    let fifo = root.path().join("fifo");
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    for path in [fifo.as_path(), root.path()] {
        assert!(matches!(
            policy.read_file_bounded(Some(root.path()), path, 0),
            Err(BoundedFileReadError::Io(_))
        ));
    }
}

#[test]
fn workspace_bounded_read_limit_overflow_is_an_argument_error_before_open() {
    use super::BoundedFileReadError;
    let (root, policy) = fixture();
    match policy
        .read_file_bounded(Some(root.path()), &root.path().join("missing"), usize::MAX)
        .unwrap_err()
    {
        BoundedFileReadError::Io(error) => {
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput)
        }
        error => panic!("overflow is not observed file size: {error:?}"),
    }
}

fn fixture() -> (tempfile::TempDir, HostFilesystemPolicy) {
    let root = tempfile::tempdir().unwrap();
    let private = root.path().join("private");
    fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
    let policy = HostFilesystemPolicy::pin_directories(&[private], root.path()).unwrap();
    (root, policy)
}

#[test]
fn mutation_options_preserve_public_modes_and_missing_parent_semantics() {
    let (root, policy) = fixture();
    let path = root.path().join("public");
    fs::write(&path, b"before").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
    policy
        .atomic_write_file_with_options(Some(root.path()), &path, b"after", false)
        .unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"after");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o750
    );
    let nested = root.path().join("new/child");
    assert!(policy
        .atomic_write_file_with_options(Some(root.path()), &nested, b"new", false)
        .is_err());
    assert!(!root.path().join("new").exists());
    policy
        .atomic_write_file_with_options(Some(root.path()), &nested, b"new", true)
        .unwrap();
    assert_eq!(fs::read(nested).unwrap(), b"new");
    assert_eq!(
        policy
            .metadata(Some(root.path()), root.path())
            .unwrap()
            .file_type,
        FileType::Directory
    );
}

#[test]
fn moved_private_inode_refuses_metadata_creation_removal_and_publication() {
    let (root, policy) = fixture();
    let private = root.path().join("private");
    fs::write(private.join("marker"), b"owned").unwrap();
    let moved = root.path().join("moved-private");
    fs::rename(&private, &moved).unwrap();
    fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
    for path in [&private, &moved] {
        for outcome in [
            policy.metadata(Some(root.path()), path).map(|_| ()),
            policy.create_directory(Some(root.path()), &path.join("child"), true),
            policy.remove_path(Some(root.path()), path, true),
            policy.atomic_write_file_with_options(
                Some(root.path()),
                &path.join("child/marker"),
                b"no",
                true,
            ),
        ] {
            let error = outcome.unwrap_err();
            assert!(HostFilesystemPolicy::is_private_access_denied(&error));
        }
        assert!(!path.join("child").exists());
    }
    assert_eq!(fs::read(moved.join("marker")).unwrap(), b"owned");
}

#[test]
fn recursive_removal_refuses_private_descendants_before_public_side_effects() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("parent");
    fs::create_dir(&parent).unwrap();
    let private = parent.join("private");
    fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
    fs::write(parent.join("a-public"), b"public").unwrap();
    fs::write(private.join("marker"), b"owned").unwrap();
    let policy = HostFilesystemPolicy::pin_directories(&[private.clone()], root.path()).unwrap();
    let error = policy
        .remove_path(Some(root.path()), &parent, true)
        .unwrap_err();
    assert!(HostFilesystemPolicy::is_private_access_denied(&error));
    assert_eq!(fs::read(parent.join("a-public")).unwrap(), b"public");
    assert_eq!(fs::read(private.join("marker")).unwrap(), b"owned");
}

#[test]
fn authorized_outside_public_parent_aliases_work_without_following_leaf_symlinks() {
    let (root, policy) = fixture();
    let outside = tempfile::tempdir().unwrap();
    let parent = outside.path().join("public");
    fs::create_dir(&parent).unwrap();
    let alias = outside.path().join("alias");
    symlink(&parent, &alias).unwrap();
    let target = alias.join("new/file");
    policy
        .atomic_write_file_with_options(None, &target, b"outside", true)
        .unwrap();
    assert_eq!(fs::read(parent.join("new/file")).unwrap(), b"outside");
    let leaf = parent.join("link");
    let private = root.path().join("private/marker");
    fs::write(&private, b"owned").unwrap();
    symlink(&private, &leaf).unwrap();
    assert!(policy
        .atomic_write_file_with_options(None, &leaf, b"no", true)
        .is_err());
    assert_eq!(
        policy.metadata(None, &leaf).unwrap().file_type,
        FileType::Symlink
    );
    policy.remove_path(None, &leaf, false).unwrap();
    assert_eq!(fs::read(private).unwrap(), b"owned");
    policy.remove_path(None, &alias.join("new"), true).unwrap();
    assert!(!parent.join("new").exists());
}

#[test]
fn native_directory_creation_retains_idempotent_and_exclusive_modes() {
    let (root, policy) = fixture();
    let path = root.path().join("public/deep");
    assert!(policy
        .create_directory(Some(root.path()), &path, false)
        .is_err());
    assert!(!root.path().join("public").exists());
    policy
        .create_directory(Some(root.path()), &path, true)
        .unwrap();
    policy
        .create_directory(Some(root.path()), &path, true)
        .unwrap();
    assert_eq!(
        policy
            .create_directory(Some(root.path()), &path, false)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::AlreadyExists
    );
    policy.remove_path(Some(root.path()), &path, false).unwrap();
    assert!(!path.exists());
}

#[test]
fn workspace_guarded_create_is_immutable_and_keeps_private_path_admission() {
    let (root, policy) = fixture();
    let path = root.path().join("intent.json");
    policy
        .atomic_create_file(Some(root.path()), &path, b"first")
        .unwrap();
    assert_eq!(
        policy
            .atomic_create_file(Some(root.path()), &path, b"second")
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read(&path).unwrap(), b"first");
    let private = root.path().join("private/intent.json");
    assert!(HostFilesystemPolicy::is_private_access_denied(
        &policy
            .atomic_create_file(Some(root.path()), &private, b"denied")
            .unwrap_err()
    ));
    assert!(!private.exists());
    symlink(&path, root.path().join("alias.json")).unwrap();
    assert!(policy
        .atomic_create_file(
            Some(root.path()),
            &root.path().join("alias.json"),
            b"denied"
        )
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), b"first");
}
