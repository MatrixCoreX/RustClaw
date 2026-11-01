use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Child;

use super::tests::{
    prepare_durable_host_process_command, prepare_durable_process_command,
    prepare_host_process_command,
};
use super::*;

const MARKER: &[u8] = b"non-secret-owned-private-marker";

struct Fixture {
    root: PathBuf,
    private: PathBuf,
    peer: Child,
}

impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("host-boundary-{}", uuid::Uuid::new_v4()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let private = root.join("private");
        fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
        fs::write(private.join("marker"), MARKER).unwrap();
        fs::set_permissions(private.join("marker"), fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(root.join("public"), b"public-marker").unwrap();
        let mut peer = Command::new("/bin/sh");
        peer.args([
            "-c",
            "exec 3< \"$1\"; printf ready; IFS= read -r value || :; exit 0",
            "fixture",
        ])
        .arg(private.join("marker"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
        let mut peer = peer.spawn().unwrap();
        let mut ready = [0; 5];
        tokio::time::timeout(
            Duration::from_secs(5),
            peer.stdout.as_mut().unwrap().read_exact(&mut ready),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(&ready, b"ready");
        Self {
            root,
            private,
            peer,
        }
    }

    fn pid(&self) -> u32 {
        self.peer.id().expect("owned live peer")
    }

    fn proc_alias(&self) -> PathBuf {
        PathBuf::from(format!("/proc/{}/root", self.pid()))
            .join(self.private.strip_prefix("/").unwrap())
            .join("marker")
    }

    fn request<'a>(&'a self, private: &'a [PathBuf]) -> ProcessSandboxRequest<'a> {
        ProcessSandboxRequest {
            mode: ToolSandboxMode::WorkspaceWrite,
            backend: ToolSandboxBackend::Auto,
            workspace_root: &self.root,
            execution_root: &self.root,
            network: ProcessNetworkPolicy::Deny,
            additional_writable_paths: &[],
            host_private_directories: private,
            browser_subprocess: false,
        }
    }

    async fn finish(&mut self) {
        drop(self.peer.stdin.take());
        assert!(
            tokio::time::timeout(Duration::from_secs(5), self.peer.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
    }

    fn nest_private(&mut self) -> PathBuf {
        let parent = self.root.join("parent");
        fs::DirBuilder::new().mode(0o700).create(&parent).unwrap();
        fs::rename(&self.private, parent.join("private")).unwrap();
        self.private = parent.join("private");
        fs::write(parent.join("public"), b"public-before").unwrap();
        parent
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

async fn output(mut command: Command) -> std::process::Output {
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    tokio::time::timeout(Duration::from_secs(5), command.output())
        .await
        .unwrap()
        .unwrap()
}

async fn broad_workspace_keeps_owned_parent_hidden(workspace: &str) {
    let mut fixture = Fixture::new().await;
    let parent = std::process::id();
    let parent_comm = fs::read_to_string("/proc/self/comm").unwrap();
    let mut results = Vec::new();
    for protected in [false, true] {
        for durable in [false, true] {
            let private = if protected {
                vec![fixture.private.clone()]
            } else {
                vec![]
            };
            let mut request = fixture.request(&private);
            request.mode = ToolSandboxMode::ReadOnly;
            request.workspace_root = std::path::Path::new(workspace);
            let prepare = if durable {
                prepare_durable_process_command
            } else {
                prepare_process_command
            };
            let mut command = prepare("/bin/sh", request).unwrap().command;
            command.args(["-c", "set -eu; if [ \"$(cat \"/proc/$1/comm\" 2>/dev/null || :)\" = \"$2\" ]; then printf parent_visible; else printf parent_hidden; fi; printf '\\n'; cat \"$3\"", "fixture"])
                .arg(parent.to_string()).arg(parent_comm.trim_end()).arg(fixture.root.join("public"));
            let result = output(command).await;
            eprintln!("SPECIAL_WORKSPACE actual_backend=true workspace={workspace} private={protected} durable={durable} output={:?} llm_calls=0", String::from_utf8_lossy(&result.stdout));
            results.push(result);
        }
    }
    fixture.finish().await;
    for result in results {
        assert!(result.status.success(), "{result:?}");
        assert_eq!(
            result.stdout, b"parent_hidden\npublic-marker",
            "workspace binding must not restore the owned parent's proc view"
        );
    }
}

#[tokio::test]
async fn root_workspace_keeps_isolated_proc_private() {
    broad_workspace_keeps_owned_parent_hidden("/").await;
}

#[tokio::test]
async fn proc_workspace_keeps_isolated_proc_private() {
    broad_workspace_keeps_owned_parent_hidden("/proc").await;
}

#[tokio::test]
async fn device_workspace_keeps_owned_shm_marker_outside_private_dev() {
    let mut fixture = Fixture::new().await;
    let device_directory = tempfile::Builder::new()
        .prefix("owned-dev-")
        .tempdir_in("/dev/shm")
        .unwrap();
    let marker = device_directory.path().join("marker");
    fs::write(&marker, b"non-secret-owned-device-marker").unwrap();
    let mut results = Vec::new();
    for protected in [false, true] {
        let private = if protected {
            vec![fixture.private.clone()]
        } else {
            vec![]
        };
        let mut request = fixture.request(&private);
        request.mode = ToolSandboxMode::ReadOnly;
        request.workspace_root = std::path::Path::new("/dev");
        let mut command = prepare_process_command("/bin/sh", request).unwrap().command;
        command.args(["-c", "set -eu; if [ -e \"$1\" ]; then printf device_marker_visible; else printf device_marker_hidden; fi; printf '\\n'; cat \"$2\"", "fixture"])
            .arg(&marker).arg(fixture.root.join("public"));
        let result = output(command).await;
        eprintln!("SPECIAL_WORKSPACE actual_backend=true workspace=/dev private={protected} output={:?} llm_calls=0", String::from_utf8_lossy(&result.stdout));
        results.push(result);
    }
    fixture.finish().await;
    assert_eq!(fs::read(marker).unwrap(), b"non-secret-owned-device-marker");
    for result in results {
        assert!(result.status.success(), "{result:?}");
        assert_eq!(result.stdout, b"device_marker_hidden\npublic-marker");
    }
}

#[tokio::test]
async fn temporary_workspace_preserves_explicit_public_reads_and_private_exclusions() {
    let mut fixture = Fixture::new().await;
    let temporary = tempfile::Builder::new()
        .prefix("owned-tmp-")
        .tempdir_in("/tmp")
        .unwrap();
    let public = temporary.path().join("public");
    fs::write(&public, b"public-temporary-marker").unwrap();
    let private = [fixture.private.clone()];
    let mut request = fixture.request(&private);
    request.mode = ToolSandboxMode::ReadOnly;
    request.workspace_root = std::path::Path::new("/tmp");
    let mut command = prepare_process_command("/bin/sh", request).unwrap().command;
    command
        .args([
            "-c",
            "set -eu; cat \"$1\"; if cat \"$2\" 2>/dev/null; then exit 42; fi",
            "fixture",
        ])
        .arg(&public)
        .arg(fixture.private.join("marker"));
    let result = output(command).await;
    fixture.finish().await;
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"public-temporary-marker");
}

#[tokio::test]
async fn proc_workspace_preserves_explicit_host_visible_policy() {
    let mut fixture = Fixture::new().await;
    let parent = std::process::id();
    let parent_comm = fs::read_to_string("/proc/self/comm").unwrap();
    let private = [fixture.private.clone()];
    let mut results = Vec::new();
    for durable in [false, true] {
        let mut request = fixture.request(&private);
        request.mode = ToolSandboxMode::ReadOnly;
        request.workspace_root = std::path::Path::new("/proc");
        let prepare = if durable {
            prepare_durable_host_process_command
        } else {
            prepare_host_process_command
        };
        let mut command = prepare("/bin/sh", request).unwrap().command;
        command
            .args([
                "-c",
                "set -eu; test \"$(cat \"/proc/$1/comm\")\" = \"$2\"; cat \"$3\"",
                "fixture",
            ])
            .arg(parent.to_string())
            .arg(parent_comm.trim_end())
            .arg(fixture.root.join("public"));
        results.push(output(command).await);
    }
    fixture.finish().await;
    for result in results {
        assert!(result.status.success(), "{result:?}");
        assert_eq!(result.stdout, b"public-marker");
    }
}

#[tokio::test]
async fn shared_memory_data_workspace_preserves_scoped_read_and_write() {
    let directory = tempfile::Builder::new()
        .prefix("owned-shm-workspace-")
        .tempdir_in("/dev/shm")
        .unwrap();
    fs::write(directory.path().join("public"), b"public-marker").unwrap();
    let request = ProcessSandboxRequest {
        mode: ToolSandboxMode::WorkspaceWrite,
        backend: ToolSandboxBackend::Auto,
        workspace_root: directory.path(),
        execution_root: directory.path(),
        network: ProcessNetworkPolicy::Deny,
        additional_writable_paths: &[],
        host_private_directories: &[],
        browser_subprocess: false,
    };
    let mut command = prepare_process_command("/bin/sh", request).unwrap().command;
    command.args(["-c", "set -eu; cat public; printf scoped-write > created"]);
    let result = output(command).await;
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"public-marker");
    assert_eq!(
        fs::read(directory.path().join("created")).unwrap(),
        b"scoped-write"
    );
}

#[tokio::test]
async fn private_host_scope_requires_explicit_user_namespace_and_denies_peer_access() {
    let mut fixture = Fixture::new().await;
    let host_namespace = fs::read_link("/proc/self/ns/user").unwrap();
    let mut explicit_namespaces = Vec::new();
    let mut outputs = Vec::new();
    let mut controls = Vec::new();
    for durable in [false, true] {
        let prepare = if durable {
            prepare_durable_host_process_command
        } else {
            prepare_host_process_command
        };
        let mut control = prepare("/bin/sh", fixture.request(&[])).unwrap().command;
        control
            .args(["-c", "test -r \"/proc/$1/status\" && cat \"$2\"", "fixture"])
            .arg(fixture.pid().to_string())
            .arg(fixture.root.join("public"));
        controls.push(output(control).await);
        let mut command = prepare("/bin/sh", fixture.request(&[fixture.private.clone()]))
            .unwrap()
            .command;
        explicit_namespaces.push(
            command
                .as_std()
                .get_args()
                .any(|arg| arg == "--unshare-user"),
        );
        command.args(["-c", "set -eu; readlink /proc/self/ns/user; cat \"$1\"; for path in \"$2\" \"$3\"; do if cat \"$path\" 2>/dev/null; then exit 42; fi; done; if (: < \"$4\") 2>/dev/null; then exit 43; fi", "fixture"])
            .arg(fixture.root.join("public")).arg(fixture.proc_alias())
            .arg(format!("/proc/{}/fd/3", fixture.pid()))
            .arg(format!("/proc/{}/mem", fixture.pid()));
        outputs.push(output(command).await);
    }
    fixture.finish().await;
    for control in controls {
        assert!(control.status.success());
        assert_eq!(control.stdout, b"public-marker");
    }
    for result in outputs {
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let boundary = result
            .stdout
            .iter()
            .position(|byte| *byte == b'\n')
            .unwrap();
        assert_ne!(
            &result.stdout[..boundary],
            host_namespace.as_os_str().as_encoded_bytes()
        );
        assert_eq!(&result.stdout[boundary + 1..], b"public-marker");
    }
    eprintln!("HOST_PRIVATE_USERNS_WITNESS foreground_explicit={} durable_explicit={} peer_root_fd_mem_denied=true peer_reaped=true", explicit_namespaces[0], explicit_namespaces[1]);
    assert_eq!(
        explicit_namespaces,
        [true, true],
        "protected host scope must not depend on implicit backend user-namespace defaults"
    );
}

#[tokio::test]
async fn isolated_private_scope_blocks_peer_proc_root_and_preserves_public_access() {
    let mut fixture = Fixture::new().await;
    let mut outputs = Vec::new();
    for durable in [false, true] {
        let prepare = if durable {
            prepare_durable_process_command
        } else {
            prepare_process_command
        };
        let mut command = prepare("/bin/sh", fixture.request(&[fixture.private.clone()]))
            .unwrap()
            .command;
        assert!(command
            .as_std()
            .get_args()
            .any(|arg| arg == "--unshare-user"));
        assert!(!command
            .as_std()
            .get_args()
            .any(|arg| arg == "--unshare-user-try"));
        command
            .args(["-c", "cat \"$1\"; cat \"$2\" 2>/dev/null || :", "fixture"])
            .arg(fixture.root.join("public"))
            .arg(fixture.proc_alias());
        outputs.push(output(command).await);
    }
    fixture.finish().await;
    for result in outputs {
        assert!(result.status.success());
        assert_eq!(result.stdout, b"public-marker");
    }
    assert_eq!(fs::read(fixture.private.join("marker")).unwrap(), MARKER);
}

#[tokio::test]
async fn private_ancestor_cannot_redirect_the_next_runtime_scopes_exclusion() {
    let mut observations = Vec::new();
    for host_visible in [false, true] {
        for durable in [false, true] {
            for alias in [false, true] {
                let mut fixture = Fixture::new().await;
                let parent = fixture.nest_private();
                let prepare = match (host_visible, durable) {
                    (false, false) => prepare_process_command,
                    (false, true) => prepare_durable_process_command,
                    (true, false) => prepare_host_process_command,
                    (true, true) => prepare_durable_host_process_command,
                };
                let private = [fixture.private.clone()];
                let writable = if alias {
                    vec![fixture.root.clone()]
                } else {
                    Vec::new()
                };
                let mut request = fixture.request(&private);
                request.additional_writable_paths = &writable;
                let prepared = prepare("/bin/sh", request).unwrap();
                let active_root = if alias {
                    prepared.additional_writable_targets[0].clone()
                } else {
                    fixture.root.clone()
                };
                let mut command = prepared.command;
                command.args(["-c", "set -eu; printf public-after > \"$1/parent/public\"; if mv \"$1/parent\" \"$1/moved\"; then mkdir -m 700 \"$1/parent\" \"$1/parent/private\"; printf moved; else printf anchored; fi", "fixture"])
                .arg(&active_root);
                let moved = output(command).await;
                let mut followup = prepare("/bin/sh", request).unwrap().command;
                followup.args(["-c", "if cat \"$1/moved/private/marker\" 2>/dev/null; then exit 42; fi; printf protected", "fixture"])
                .arg(&fixture.root);
                let next = output(followup).await;
                let public_path = if moved.stdout == b"moved" {
                    fixture.root.join("moved/public")
                } else {
                    parent.join("public")
                };
                let public = fs::read(public_path).unwrap();
                fixture.finish().await;
                eprintln!("PRIVATE_ANCESTOR_WITNESS producer=runtime host_visible={host_visible} durable={durable} alias={alias} moved={} next_scope_private_read={} llm_calls=0",
                moved.stdout == b"moved", next.stdout.starts_with(MARKER));
                observations.push((moved, next, public));
            }
        }
    }
    for (moved, next, public) in observations {
        assert!(
            moved.status.success(),
            "{}",
            String::from_utf8_lossy(&moved.stderr)
        );
        assert_eq!(moved.stdout, b"anchored");
        assert!(next.status.success());
        assert_eq!(next.stdout, b"protected");
        assert_eq!(public, b"public-after");
    }
}

#[tokio::test]
async fn private_ancestor_anchors_never_add_workspace_write_to_readonly_runtime_scopes() {
    let mut observations = Vec::new();
    for host_visible in [false, true] {
        for durable in [false, true] {
            let mut fixture = Fixture::new().await;
            let parent = fixture.nest_private();
            let private = [fixture.private.clone()];
            let mut request = fixture.request(&private);
            request.mode = ToolSandboxMode::ReadOnly;
            let prepare = match (host_visible, durable) {
                (false, false) => prepare_process_command,
                (false, true) => prepare_durable_process_command,
                (true, false) => prepare_host_process_command,
                (true, true) => prepare_durable_host_process_command,
            };
            let mut command = prepare("/bin/sh", request).unwrap().command;
            command.args(["-c", "set -eu; cat \"$1/parent/public\"; if printf changed > \"$1/parent/public\" 2>/dev/null; then exit 42; fi; if mkdir \"$1/parent/new\" 2>/dev/null; then exit 43; fi", "fixture"])
                .arg(&fixture.root);
            let result = output(command).await;
            let public = fs::read(parent.join("public")).unwrap();
            let created = parent.join("new").exists();
            fixture.finish().await;
            observations.push((result, public, created));
        }
    }
    for (result, public, created) in observations {
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, b"public-before");
        assert_eq!(public, b"public-before");
        assert!(!created);
    }
}
