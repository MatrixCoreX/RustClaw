use super::*;
#[cfg(target_os = "linux")]
use std::ffi::OsString;

#[cfg(target_os = "linux")]
#[test]
fn linux_recipe_preserves_backend_flags_network_mount_order_and_payload_boundary() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    for network in [SandboxNetwork::Deny, SandboxNetwork::Allow] {
        let prepared = prepare_sandboxed_invocation(
            Path::new("/bin/sh"),
            &workspace,
            &[workspace.clone()],
            network,
            &[],
        )
        .unwrap();
        assert_eq!(prepared.backend, "bubblewrap");
        assert!(prepared.working_directory.is_none());
        let (program, args) = prepared.invocation.into_parts();
        assert!(["/usr/bin/bwrap", "/bin/bwrap"]
            .iter()
            .any(|value| program == *value));
        let mut expected: Vec<OsString> = [
            "--die-with-parent",
            "--new-session",
            "--unshare-pid",
            "--unshare-ipc",
            "--unshare-uts",
            "--ro-bind",
            "/",
            "/",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
        ]
        .map(OsString::from)
        .into();
        if !workspace.starts_with("/tmp") {
            expected.extend(["--tmpfs", "/tmp"].map(OsString::from));
        }
        if network == SandboxNetwork::Deny {
            expected.push("--unshare-net".into());
        }
        expected.extend([
            OsString::from("--bind"),
            workspace.clone().into_os_string(),
            workspace.clone().into_os_string(),
            OsString::from("--chdir"),
            workspace.clone().into_os_string(),
            OsString::from("--"),
            OsString::from("/bin/sh"),
        ]);
        assert_eq!(args, expected);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn typed_sandbox_arguments_execute_once_without_interpolation() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let mut prepared =
        prepare_sandboxed_invocation(Path::new("/bin/sh"), &root, &[], SandboxNetwork::Deny, &[])
            .unwrap();
    prepared.invocation.args([
        "-c",
        "printf '%s\\n' \"$@\"",
        "fixture",
        "",
        "space value",
        "$(not-executed)",
        "'quoted'",
    ]);
    let output = prepared.into_command().output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"\nspace value\n$(not-executed)\n'quoted'\n");
}

#[cfg(target_os = "macos")]
#[test]
fn macos_recipe_keeps_the_canonical_working_directory_before_materialization() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let prepared = prepare_sandboxed_invocation(
        Path::new("/usr/bin/true"),
        &root,
        &[],
        SandboxNetwork::Deny,
        &[],
    )
    .unwrap();
    assert_eq!(prepared.backend, "macos_seatbelt");
    assert_eq!(prepared.working_directory.as_ref(), Some(&root));
    assert_eq!(
        prepared.into_command().get_current_dir(),
        Some(root.as_path())
    );
}
