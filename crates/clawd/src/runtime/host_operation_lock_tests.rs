use super::*;
use std::fs;
use std::os::unix::fs::{symlink, DirBuilderExt, PermissionsExt};

#[test]
fn workspace_operation_lock_excludes_fresh_descriptions_and_releases_only_on_drop() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let filesystem = HostFilesystemPolicy::without_private_authority();
    let path = root.join("operation.lock");
    let lock = filesystem
        .try_lock_operation_file(Some(&root), &path)
        .unwrap();
    assert_ne!(
        unsafe { libc::fcntl(lock.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
        0
    );
    assert_eq!(lock.metadata().unwrap().mode() & 0o7777, 0o600);
    assert_eq!(
        filesystem
            .try_lock_operation_file(Some(&root), &path)
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    drop(lock);
    filesystem
        .try_lock_operation_file(Some(&root), &path)
        .unwrap();
}

#[test]
fn workspace_operation_lock_rejects_private_symlink_hardlink_and_public_mode_files() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let private = root.join("private");
    fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
    let filesystem = HostFilesystemPolicy::pin_directories(&[private.clone()], &root).unwrap();
    assert!(filesystem
        .try_lock_operation_file(Some(&root), &private.join("x.lock"))
        .is_err());
    assert!(!private.join("x.lock").exists());
    let target = root.join("target.lock");
    filesystem
        .try_lock_operation_file(Some(&root), &target)
        .unwrap();
    symlink(&target, root.join("symlink.lock")).unwrap();
    assert!(filesystem
        .try_lock_operation_file(Some(&root), &root.join("symlink.lock"))
        .is_err());
    fs::hard_link(&target, root.join("hardlink.lock")).unwrap();
    assert!(filesystem
        .try_lock_operation_file(Some(&root), &target)
        .is_err());
    let public = root.join("public.lock");
    fs::write(&public, []).unwrap();
    fs::set_permissions(&public, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(filesystem
        .try_lock_operation_file(Some(&root), &public)
        .is_err());
}
