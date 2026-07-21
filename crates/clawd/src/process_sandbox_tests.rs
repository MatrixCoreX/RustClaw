use std::path::{Path, PathBuf};
use std::process::Stdio;

use claw_core::config::{ToolSandboxBackend, ToolSandboxMode};

use super::{
    prepare_durable_process_command, prepare_process_command, sandbox_backend_diagnostics,
    ProcessNetworkPolicy, ProcessSandboxRequest,
};

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        Self::new_under(
            &std::env::current_dir().expect("current test directory"),
            label,
        )
    }

    fn new_in_system_temp(label: &str) -> Self {
        Self::new_under(&std::env::temp_dir(), label)
    }

    fn new_under(parent: &Path, label: &str) -> Self {
        let path = parent.join("target/process-sandbox-tests").join(format!(
            "{}_{}_{}",
            label,
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).expect("create test dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn danger_full_uses_direct_backend() {
    let root = TestDir::new("danger_full");
    let prepared = prepare_process_command(
        "bash",
        ProcessSandboxRequest {
            mode: ToolSandboxMode::DangerFull,
            backend: ToolSandboxBackend::Auto,
            workspace_root: root.path(),
            execution_root: root.path(),
            network: ProcessNetworkPolicy::Inherit,
            additional_writable_paths: &[],
        },
    )
    .expect("direct command");

    assert_eq!(prepared.backend, "direct");
    let diagnostics = sandbox_backend_diagnostics(
        ToolSandboxBackend::Auto,
        ToolSandboxMode::DangerFull,
        ProcessNetworkPolicy::Inherit,
    );
    assert_eq!(diagnostics.resolved_backend, "direct");
    assert!(diagnostics.available);
    assert!(!diagnostics.fail_closed);
}

#[test]
fn remote_container_contract_is_explicit_and_fail_closed() {
    let diagnostics = sandbox_backend_diagnostics(
        ToolSandboxBackend::RemoteContainer,
        ToolSandboxMode::WorkspaceWrite,
        ProcessNetworkPolicy::Deny,
    );
    assert_eq!(diagnostics.resolved_backend, "remote_container");
    assert!(!diagnostics.available);
    assert!(diagnostics.fail_closed);
    assert_eq!(
        diagnostics.reason_code,
        Some("sandbox_remote_backend_not_configured")
    );

    let root = TestDir::new("remote_contract");
    let result = prepare_process_command(
        "bash",
        ProcessSandboxRequest {
            mode: ToolSandboxMode::WorkspaceWrite,
            backend: ToolSandboxBackend::RemoteContainer,
            workspace_root: root.path(),
            execution_root: root.path(),
            network: ProcessNetworkPolicy::Deny,
            additional_writable_paths: &[],
        },
    );
    assert!(matches!(
        result,
        Err("sandbox_remote_backend_not_configured")
    ));
}

#[test]
fn platform_mismatched_explicit_backend_is_rejected() {
    let backend = if cfg!(target_os = "macos") {
        ToolSandboxBackend::Bubblewrap
    } else {
        ToolSandboxBackend::MacosSeatbelt
    };
    let diagnostics = sandbox_backend_diagnostics(
        backend,
        ToolSandboxMode::WorkspaceWrite,
        ProcessNetworkPolicy::Deny,
    );
    assert!(!diagnostics.available);
    assert!(diagnostics.fail_closed);
    assert_eq!(
        diagnostics.reason_code,
        Some("sandbox_backend_unsupported_platform")
    );
}

#[test]
fn auto_backend_reports_the_current_platform_without_direct_fallback() {
    let diagnostics = sandbox_backend_diagnostics(
        ToolSandboxBackend::Auto,
        ToolSandboxMode::WorkspaceWrite,
        ProcessNetworkPolicy::Deny,
    );
    #[cfg(target_os = "linux")]
    assert_eq!(diagnostics.resolved_backend, "bubblewrap");
    #[cfg(target_os = "macos")]
    assert_eq!(diagnostics.resolved_backend, "macos_seatbelt");
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    assert_eq!(diagnostics.resolved_backend, "unsupported");
    assert_ne!(diagnostics.resolved_backend, "direct");
    assert!(diagnostics.fail_closed);
}

#[cfg(target_os = "linux")]
#[test]
fn durable_sandbox_omits_parent_death_while_foreground_keeps_it() {
    if !std::path::Path::new("/usr/bin/bwrap").is_file()
        && !std::path::Path::new("/bin/bwrap").is_file()
    {
        return;
    }
    let root = TestDir::new("durable_lifetime");
    let request = || ProcessSandboxRequest {
        mode: ToolSandboxMode::WorkspaceWrite,
        backend: ToolSandboxBackend::Auto,
        workspace_root: root.path(),
        execution_root: root.path(),
        network: ProcessNetworkPolicy::Deny,
        additional_writable_paths: &[],
    };
    let foreground = prepare_process_command("bash", request()).expect("foreground command");
    let durable =
        prepare_durable_process_command("bash", request()).expect("durable async command");
    let foreground_args = foreground
        .command
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let durable_args = durable
        .command
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    assert!(foreground_args.iter().any(|arg| arg == "--die-with-parent"));
    assert!(!durable_args.iter().any(|arg| arg == "--die-with-parent"));
    assert!(durable_args.iter().any(|arg| arg == "--new-session"));
    assert!(durable_args.iter().any(|arg| arg == "--unshare-pid"));
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn read_only_backend_rejects_workspace_mutation() {
    if !std::path::Path::new("/usr/bin/bwrap").is_file()
        && !std::path::Path::new("/bin/bwrap").is_file()
    {
        return;
    }
    let root = TestDir::new("read_only");
    let mut prepared = prepare_process_command(
        "bash",
        ProcessSandboxRequest {
            mode: ToolSandboxMode::ReadOnly,
            backend: ToolSandboxBackend::Auto,
            workspace_root: root.path(),
            execution_root: root.path(),
            network: ProcessNetworkPolicy::Deny,
            additional_writable_paths: &[],
        },
    )
    .expect("sandbox command");
    prepared
        .command
        .arg("-lc")
        .arg("printf blocked > should-not-exist")
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let status = prepared.command.status().await.expect("sandbox status");
    assert!(!status.success());
    assert!(!root.path().join("should-not-exist").exists());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn workspace_backend_writes_only_inside_bound_workspace() {
    if !std::path::Path::new("/usr/bin/bwrap").is_file()
        && !std::path::Path::new("/bin/bwrap").is_file()
    {
        return;
    }
    let root = TestDir::new("workspace_write");
    let mut prepared = prepare_process_command(
        "bash",
        ProcessSandboxRequest {
            mode: ToolSandboxMode::WorkspaceWrite,
            backend: ToolSandboxBackend::Auto,
            workspace_root: root.path(),
            execution_root: root.path(),
            network: ProcessNetworkPolicy::Deny,
            additional_writable_paths: &[],
        },
    )
    .expect("sandbox command");
    prepared
        .command
        .arg("-lc")
        .arg("printf allowed > result.txt")
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let status = prepared.command.status().await.expect("sandbox status");
    assert!(status.success());
    assert_eq!(
        std::fs::read_to_string(root.path().join("result.txt")).expect("result"),
        "allowed"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn read_only_backend_can_execute_inside_system_temp_workspace() {
    if !std::path::Path::new("/usr/bin/bwrap").is_file()
        && !std::path::Path::new("/bin/bwrap").is_file()
    {
        return;
    }
    let root = TestDir::new_in_system_temp("read_only_system_temp");
    std::fs::write(root.path().join("input.txt"), "visible").expect("write input");
    let mut prepared = prepare_process_command(
        "bash",
        ProcessSandboxRequest {
            mode: ToolSandboxMode::ReadOnly,
            backend: ToolSandboxBackend::Auto,
            workspace_root: root.path(),
            execution_root: root.path(),
            network: ProcessNetworkPolicy::Deny,
            additional_writable_paths: &[],
        },
    )
    .expect("sandbox command");
    prepared
        .command
        .arg("-lc")
        .arg("test \"$(cat input.txt)\" = visible")
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let status = prepared.command.status().await.expect("sandbox status");
    assert!(status.success());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn workspace_backend_can_write_inside_system_temp_workspace() {
    if !std::path::Path::new("/usr/bin/bwrap").is_file()
        && !std::path::Path::new("/bin/bwrap").is_file()
    {
        return;
    }
    let root = TestDir::new_in_system_temp("workspace_write_system_temp");
    let mut prepared = prepare_process_command(
        "bash",
        ProcessSandboxRequest {
            mode: ToolSandboxMode::WorkspaceWrite,
            backend: ToolSandboxBackend::Auto,
            workspace_root: root.path(),
            execution_root: root.path(),
            network: ProcessNetworkPolicy::Deny,
            additional_writable_paths: &[],
        },
    )
    .expect("sandbox command");
    prepared
        .command
        .arg("-lc")
        .arg("printf visible > result.txt")
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let status = prepared.command.status().await.expect("sandbox status");
    assert!(status.success());
    assert_eq!(
        std::fs::read_to_string(root.path().join("result.txt")).expect("result"),
        "visible"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn read_only_backend_limits_writes_to_explicit_internal_path() {
    if !std::path::Path::new("/usr/bin/bwrap").is_file()
        && !std::path::Path::new("/bin/bwrap").is_file()
    {
        return;
    }
    let root = TestDir::new_in_system_temp("read_only_explicit_write");
    let internal = TestDir::new_in_system_temp("internal_write");
    let writable_paths = vec![internal.path().to_path_buf()];
    let mut prepared = prepare_process_command(
        "bash",
        ProcessSandboxRequest {
            mode: ToolSandboxMode::ReadOnly,
            backend: ToolSandboxBackend::Auto,
            workspace_root: root.path(),
            execution_root: root.path(),
            network: ProcessNetworkPolicy::Deny,
            additional_writable_paths: &writable_paths,
        },
    )
    .expect("sandbox command");
    let internal_target = prepared
        .additional_writable_targets
        .first()
        .expect("internal target")
        .clone();
    prepared
        .command
        .arg("-lc")
        .arg(format!(
            "printf internal > {}/result.txt; printf blocked > workspace.txt",
            internal_target.display()
        ))
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let status = prepared.command.status().await.expect("sandbox status");
    assert!(!status.success());
    assert_eq!(
        std::fs::read_to_string(internal.path().join("result.txt")).expect("internal result"),
        "internal"
    );
    assert!(!root.path().join("workspace.txt").exists());
}
