use std::io::Read;
use std::path::{Path, PathBuf};

use super::remove_workspace_file;
use super::{atomic_create_workspace_file, atomic_write_workspace_file, open_workspace_file};

#[cfg(unix)]
#[test]
fn secure_workspace_atomic_replacement_preserves_mode_and_ownership() {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let root = TestWorkspace::new("replacement-metadata");
    let target = root.path().join("config.toml");
    std::fs::write(&target, b"fixture before").unwrap();
    // A dedicated Linux CI invocation runs only this fixture under sudo to
    // cover container-root replacement of files owned by a different UID.
    let privileged = unsafe { libc::geteuid() } == 0;
    if privileged {
        let file = std::fs::File::open(&target).unwrap();
        assert_eq!(unsafe { libc::fchown(file.as_raw_fd(), 65534, 65534) }, 0);
    }
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o660)).unwrap();
    let before = std::fs::metadata(&target).unwrap();
    atomic_write_workspace_file(root.path(), &target, b"fixture after").unwrap();
    let after = std::fs::metadata(&target).unwrap();
    assert_eq!(
        (after.uid(), after.gid(), after.mode() & 0o7777),
        (before.uid(), before.gid(), before.mode() & 0o7777)
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"fixture after");
    eprintln!("secure_workspace_metadata_preserved=true privileged_owner_check={privileged}");
}

#[cfg(unix)]
#[test]
fn secure_workspace_remove_unlinks_only_requested_leaf() {
    let root = TestWorkspace::new("remove-leaf");
    let target = root.path().join("remove.txt");
    let keep = root.path().join("keep.txt");
    std::fs::write(&target, b"fixture target").unwrap();
    std::fs::write(&keep, b"fixture keep").unwrap();
    remove_workspace_file(root.path(), &target).unwrap();
    assert!(!target.exists());
    assert_eq!(std::fs::read(&keep).unwrap(), b"fixture keep");
    assert_eq!(
        remove_workspace_file(root.path(), &target)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::NotFound
    );
    assert!(remove_workspace_file(root.path(), root.path()).is_err());
}

#[cfg(unix)]
#[test]
fn secure_workspace_remove_rejects_parent_links_and_outside_paths() {
    use std::os::unix::fs::symlink;
    let root = TestWorkspace::new("remove-root");
    let outside = TestWorkspace::new("remove-outside");
    let target = outside.path().join("keep.txt");
    std::fs::write(&target, b"fixture keep").unwrap();
    symlink(outside.path(), root.path().join("linked")).unwrap();
    for candidate in [
        target.clone(),
        root.path().join("linked/keep.txt"),
        root.path().join("../keep.txt"),
    ] {
        assert!(remove_workspace_file(root.path(), &candidate).is_err());
    }
    assert_eq!(std::fs::read(&target).unwrap(), b"fixture keep");
}

#[cfg(unix)]
#[test]
fn secure_workspace_remove_rejects_a_leaf_symlink_without_touching_its_target() {
    use std::os::unix::fs::symlink;
    let root = TestWorkspace::new("remove-link");
    let outside = TestWorkspace::new("remove-link-target");
    let target = outside.path().join("keep.txt");
    std::fs::write(&target, b"fixture keep").unwrap();
    let link = root.path().join("leaf");
    symlink(&target, &link).unwrap();
    assert!(remove_workspace_file(root.path(), &link).is_err());
    assert!(link.is_symlink());
    assert_eq!(std::fs::read(&target).unwrap(), b"fixture keep");
}

pub(crate) struct TestWorkspace(PathBuf);

impl TestWorkspace {
    pub(crate) fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "agent-runtime-secure-workspace-{label}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&path).expect("exclusively create test workspace");
        Self(path)
    }

    #[cfg(unix)]
    fn new_for_unix_socket() -> Self {
        // AF_UNIX paths are limited to roughly 104-108 bytes. Test runners may
        // provide a deeply nested TMPDIR, so keep this fixture path bounded.
        let path = PathBuf::from("/tmp").join(format!(
            "ar-secure-socket-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&path).expect("exclusively create socket test workspace");
        Self(path)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn descriptor_relative_read_and_write_preserve_regular_files() {
    let workspace = TestWorkspace::new("read-write");
    let nested = workspace.path().join("nested");
    std::fs::create_dir(&nested).expect("nested");
    let target = nested.join("value.txt");
    std::fs::write(&target, b"before").expect("fixture");

    let mut file = open_workspace_file(workspace.path(), &target).expect("secure read");
    let mut content = String::new();
    file.read_to_string(&mut content).expect("read");
    assert_eq!(content, "before");

    atomic_write_workspace_file(workspace.path(), &target, b"after").expect("secure write");
    assert_eq!(std::fs::read(&target).unwrap(), b"after");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn atomic_create_never_replaces_an_existing_inode_or_content() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let workspace = TestWorkspace::new("create-new");
    let target = workspace.path().join("nested/immutable.json");
    atomic_create_workspace_file(workspace.path(), &target, b"first").unwrap();
    let mut pinned = open_workspace_file(workspace.path(), &target).unwrap();
    let before = pinned.metadata().unwrap();
    assert_eq!(before.permissions().mode() & 0o7777, 0o600);
    assert_eq!(
        atomic_create_workspace_file(workspace.path(), &target, b"replacement")
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::AlreadyExists
    );
    let mut retained = Vec::new();
    pinned.read_to_end(&mut retained).unwrap();
    assert_eq!(retained, b"first");
    assert_eq!(pinned.metadata().unwrap().nlink(), 1);
    assert_eq!(std::fs::metadata(&target).unwrap().ino(), before.ino());
    assert_eq!(std::fs::read(&target).unwrap(), b"first");
    assert_eq!(
        std::fs::read_dir(target.parent().unwrap()).unwrap().count(),
        1
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn atomic_create_race_has_one_winner_and_keeps_that_winners_inode() {
    use std::os::unix::fs::MetadataExt;
    let workspace = TestWorkspace::new("create-race");
    let target = workspace.path().join("value");
    let barrier = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..8u8)
            .map(|value| {
                let (barrier, target, workspace) = (&barrier, &target, &workspace);
                scope.spawn(move || {
                    barrier.wait();
                    match atomic_create_workspace_file(workspace.path(), target, &[value]) {
                        Ok(()) => Some((
                            value,
                            open_workspace_file(workspace.path(), target).unwrap(),
                        )),
                        Err(error) => {
                            assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
                            None
                        }
                    }
                })
            })
            .collect();
        let mut winners: Vec<_> = workers
            .into_iter()
            .filter_map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(winners.len(), 1);
        let (value, mut file) = winners.pop().unwrap();
        assert_eq!(file.metadata().unwrap().nlink(), 1);
        assert_eq!(
            file.metadata().unwrap().ino(),
            std::fs::metadata(&target).unwrap().ino()
        );
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, [value]);
        assert_eq!(std::fs::read(&target).unwrap(), [value]);
    });
    assert_eq!(std::fs::read_dir(workspace.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn descriptor_relative_access_rejects_parent_and_target_symlinks() {
    use std::os::unix::fs::symlink;

    let workspace = TestWorkspace::new("symlink-workspace");
    let outside = TestWorkspace::new("symlink-outside");
    let outside_file = outside.path().join("outside.txt");
    std::fs::write(&outside_file, b"outside").expect("outside fixture");
    symlink(outside.path(), workspace.path().join("escape")).expect("parent symlink");
    symlink(&outside_file, workspace.path().join("target.txt")).expect("target symlink");

    assert!(open_workspace_file(
        workspace.path(),
        &workspace.path().join("escape/outside.txt")
    )
    .is_err());
    assert!(open_workspace_file(workspace.path(), &workspace.path().join("target.txt")).is_err());
    atomic_write_workspace_file(
        workspace.path(),
        &workspace.path().join("target.txt"),
        b"local",
    )
    .expect_err("target symlink must be rejected");
    assert_eq!(std::fs::read(&outside_file).unwrap(), b"outside");
    assert!(workspace.path().join("target.txt").is_symlink());
}

#[cfg(unix)]
#[test]
fn descriptor_relative_read_rejects_special_files() {
    let workspace = TestWorkspace::new_for_unix_socket();
    let socket_path = workspace.path().join("agent.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&socket_path).expect("socket");
    assert!(open_workspace_file(workspace.path(), &socket_path).is_err());
}
