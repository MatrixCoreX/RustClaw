use std::io::Read;
use std::path::{Path, PathBuf};

use super::{atomic_write_workspace_file, open_workspace_file};

struct TestWorkspace(PathBuf);

impl TestWorkspace {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "agent-runtime-secure-workspace-{label}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&path).expect("create test workspace");
        Self(path)
    }

    fn path(&self) -> &Path {
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
    let workspace = TestWorkspace::new("special-file");
    let socket_path = workspace.path().join("agent.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&socket_path).expect("socket");
    assert!(open_workspace_file(workspace.path(), &socket_path).is_err());
}
