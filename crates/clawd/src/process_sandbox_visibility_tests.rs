use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

use super::tests::{
    prepare_durable_host_process_command, prepare_durable_process_command,
    prepare_host_process_command,
};
use super::*;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("pv-{}", uuid::Uuid::new_v4().simple()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        Self(root.canonicalize().unwrap())
    }

    fn private_directory(&self) -> PathBuf {
        let private = self.0.join("private");
        fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
        for name in ["marker", "journal.db", "journal.db-wal", "journal.db-shm"] {
            let path = private.join(name);
            fs::write(&path, b"non-secret-host-private-marker").unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        private
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn readonly_mapping_retains_private_masks_and_cannot_write_its_source() {
    let workspace = Fixture::new();
    let metadata = Fixture::new();
    let private = metadata.private_directory();
    let public = metadata.0.join("public");
    fs::write(&public, b"public-marker").unwrap();
    let private_paths = [private.clone()];
    let mut request = request(&workspace.0, &private_paths, &[]);
    request.mode = ToolSandboxMode::ReadOnly;
    let mut prepared =
        prepare_readonly_process_command("/bin/sh", request, &[metadata.0.clone()]).unwrap();
    let output = prepared.command.args(["-c", "set -eu; test \"$(cat \"$1\")\" = public-marker; if printf changed > \"$1\"; then exit 41; fi; if cat \"$2\"; then exit 42; fi; printf readonly-complete", "fixture"])
        .arg(&public).arg(private.join("marker")).output().await.unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"readonly-complete");
    assert_eq!(fs::read(public).unwrap(), b"public-marker");
    assert_eq!(
        fs::read(private.join("marker")).unwrap(),
        b"non-secret-host-private-marker"
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn readonly_mappings_reject_infrastructure_and_non_readonly_modes() {
    let workspace = Fixture::new();
    for path in ["/", "/tmp", "/dev", "/proc", "/dev/null"] {
        let mut request = request(&workspace.0, &[], &[]);
        request.mode = ToolSandboxMode::ReadOnly;
        assert!(
            prepare_readonly_process_command("/bin/sh", request, &[PathBuf::from(path)]).is_err()
        );
    }
    for mode in [
        ToolSandboxMode::DangerFull,
        ToolSandboxMode::WorkspaceWrite,
        ToolSandboxMode::IsolatedWorktree,
    ] {
        let mut request = request(&workspace.0, &[], &[]);
        request.mode = mode;
        assert!(matches!(
            prepare_readonly_process_command("/bin/sh", request, &[]),
            Err("sandbox_readonly_mapping_requires_readonly_mode")
        ));
    }
}

fn request<'a>(
    root: &'a Path,
    private: &'a [PathBuf],
    writable: &'a [PathBuf],
) -> ProcessSandboxRequest<'a> {
    ProcessSandboxRequest {
        mode: ToolSandboxMode::WorkspaceWrite,
        backend: ToolSandboxBackend::Auto,
        workspace_root: root,
        execution_root: root,
        network: ProcessNetworkPolicy::Deny,
        additional_writable_paths: writable,
        host_private_directories: private,
        browser_subprocess: false,
    }
}

fn prepare(
    request: ProcessSandboxRequest<'_>,
    durable: bool,
    host_visible: bool,
) -> Result<PreparedProcessCommand, &'static str> {
    match (durable, host_visible) {
        (false, false) => prepare_process_command("/bin/sh", request),
        (false, true) => prepare_host_process_command("/bin/sh", request),
        (true, false) => prepare_durable_process_command("/bin/sh", request),
        (true, true) => prepare_durable_host_process_command("/bin/sh", request),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn host_private_directory_is_hidden_from_a_workspace_payload() {
    let fixture = Fixture::new();
    let private = fixture.private_directory();
    let public = fixture.0.join("public");
    fs::write(&public, b"public-marker").unwrap();
    for durable in [false, true] {
        for host_visible in [false, true] {
            let mut prepared = prepare(
                request(&fixture.0, &[private.clone()], &[]),
                durable,
                host_visible,
            )
            .unwrap();
            let output = prepared.command.args(["-c", "set -eu; test \"$(cat \"$1\")\" = public-marker; printf '%s\\n' visibility-control; shift; for path do if cat \"$path\"; then exit 42; fi; if printf changed > \"$path\"; then exit 43; fi; done; printf '%s\\n' visibility-complete", "fixture"])
                .arg(&public)
                .args(["marker", "journal.db", "journal.db-wal", "journal.db-shm", "future-record"].map(|name| private.join(name)))
                .output().await.unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(output.stdout, b"visibility-control\nvisibility-complete\n");
        }
    }
    for name in ["marker", "journal.db", "journal.db-wal", "journal.db-shm"] {
        assert_eq!(
            fs::read(private.join(name)).unwrap(),
            b"non-secret-host-private-marker"
        );
    }
    assert!(!private.join("future-record").exists());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn private_directory_masks_every_ancestor_alias_without_blocking_public_artifacts() {
    let fixture = Fixture::new();
    let private = fixture.private_directory();
    let artifact = fixture.0.join("artifacts");
    fs::create_dir(&artifact).unwrap();
    let paths = [fixture.0.clone(), fixture.0.clone(), artifact.clone()];
    let mut prepared =
        prepare(request(&fixture.0, &[private.clone()], &paths), true, true).unwrap();
    let aliases = prepared.additional_writable_targets;
    let output = prepared.command.args(["-c", "set -eu; printf '%s' public-artifact > \"$1/result\"; shift; for path do if cat \"$path\"; then exit 42; fi; if printf changed > \"$path\"; then exit 43; fi; done; printf '%s\\n' visibility-complete", "fixture"])
        .arg(&aliases[2])
        .args([private.join("marker"), aliases[0].join("private/marker"), aliases[1].join("private/marker")])
        .output().await.unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"visibility-complete\n");
    assert_eq!(
        fs::read(artifact.join("result")).unwrap(),
        b"public-artifact"
    );
    assert_eq!(
        fs::read(private.join("marker")).unwrap(),
        b"non-secret-host-private-marker"
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn empty_exclusions_preserve_the_existing_workspace_access_policy() {
    let fixture = Fixture::new();
    let private = fixture.private_directory();
    let mut prepared = prepare(request(&fixture.0, &[], &[]), false, false).unwrap();
    let output = prepared
        .command
        .args(["-c", "cat \"$1\"", "fixture"])
        .arg(private.join("marker"))
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"non-secret-host-private-marker");
}

#[test]
fn direct_launch_cannot_silently_discard_requested_private_directory_exclusions() {
    let fixture = Fixture::new();
    let private = [fixture.private_directory()];
    let mut request = request(&fixture.0, &private, &[]);
    request.mode = ToolSandboxMode::DangerFull;
    for durable in [false, true] {
        for host_visible in [false, true] {
            assert!(matches!(
                prepare(request, durable, host_visible),
                Err("sandbox_private_directories_require_isolation")
            ));
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn private_paths_cannot_be_additional_writable_directories_or_the_execution_root() {
    let fixture = Fixture::new();
    let private = [fixture.private_directory()];
    assert!(matches!(
        prepare(request(&fixture.0, &private, &private), false, false),
        Err("sandbox_private_directory_writable")
    ));
    let request = request(&private[0], &private, &[]);
    assert!(matches!(
        prepare(request, false, false),
        Err("sandbox_execution_root_private")
    ));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn missing_private_directory_is_not_created_or_ignored_during_sandbox_preparation() {
    let fixture = Fixture::new();
    let private = [fixture.0.join("missing")];
    assert!(matches!(
        prepare(request(&fixture.0, &private, &[]), false, false),
        Err("sandbox_private_directory_unavailable")
    ));
    assert!(!private[0].exists());
}
