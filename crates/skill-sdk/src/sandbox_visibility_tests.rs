use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

use super::*;

fn private_directory(root: &Path) -> PathBuf {
    let path = root.join("private");
    fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
    fs::write(path.join("marker"), b"non-secret-private-marker").unwrap();
    fs::set_permissions(path.join("marker"), fs::Permissions::from_mode(0o600)).unwrap();
    path.canonicalize().unwrap()
}

#[test]
fn sdk_sandbox_enforces_private_reads_writes_and_symlink_aliases_with_public_controls() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let workspace = root.join("workspace");
    fs::create_dir(&workspace).unwrap();
    let private = private_directory(&root);
    fs::write(workspace.join("public"), b"public-marker").unwrap();
    std::os::unix::fs::symlink(&private, workspace.join("alias")).unwrap();
    let prepared = prepare_sandboxed_invocation(
        Path::new("/bin/sh"),
        &workspace,
        &[workspace.clone()],
        SandboxNetwork::Deny,
        &[private.clone()],
    )
    .unwrap();
    let mut command = prepared.into_command();
    #[cfg(target_os = "linux")]
    {
        assert!(command.get_args().any(|arg| arg == "--unshare-user"));
        assert!(!command.get_args().any(|arg| arg == "--unshare-user-try"));
    }
    let output = command.args(["-c", "set -eu; test \"$(cat \"$1/public\")\" = public-marker; printf result > \"$1/result\"; shift; for path do if cat \"$path\"; then exit 42; fi; if printf changed > \"$path\"; then exit 43; fi; done; printf '%s\\n' sdk-visibility-control", "fixture"])
        .arg(&workspace).arg(private.join("marker")).arg(workspace.join("alias/marker"))
        .arg(private.join("future-record")).output().unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"sdk-visibility-control\n");
    assert_eq!(fs::read(workspace.join("result")).unwrap(), b"result");
    assert_eq!(
        fs::read(private.join("marker")).unwrap(),
        b"non-secret-private-marker"
    );
    assert!(!private.join("future-record").exists());
}

#[test]
fn sdk_sandbox_refuses_private_writable_sources_and_missing_roots() {
    let root = tempfile::tempdir().unwrap();
    let private = private_directory(root.path());
    let error = prepare_sandboxed_invocation(
        Path::new("/bin/sh"),
        root.path(),
        &[private.clone()],
        SandboxNetwork::Deny,
        &[private.clone()],
    )
    .err()
    .expect("private writable source refused");
    assert_eq!(error.code, "sandbox_private_directory_writable");
    let missing = root.path().join("missing");
    let error = prepare_sandboxed_invocation(
        Path::new("/bin/sh"),
        root.path(),
        &[],
        SandboxNetwork::Deny,
        &[missing.clone()],
    )
    .err()
    .expect("missing private root refused");
    assert_eq!(error.code, "sandbox_private_directory_unavailable");
    assert!(!missing.exists());
}

#[test]
fn sdk_empty_exclusions_preserve_explicit_ordinary_workspace_read_access() {
    let root = tempfile::tempdir().unwrap();
    let private = private_directory(root.path());
    let prepared = prepare_sandboxed_invocation(
        Path::new("/bin/cat"),
        root.path(),
        &[],
        SandboxNetwork::Deny,
        &[],
    )
    .unwrap();
    let mut command = prepared.into_command();
    #[cfg(target_os = "linux")]
    assert!(!command.get_args().any(|arg| arg == "--unshare-user"));
    let output = command.arg(private.join("marker")).output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"non-secret-private-marker");
}

#[test]
fn sdk_private_policy_digest_is_bounded_and_delimits_every_path() {
    let first = [PathBuf::from("/ab"), PathBuf::from("/c")];
    let second = [PathBuf::from("/a"), PathBuf::from("/bc")];
    let digest = private_directories_digest(&first).unwrap();
    assert_eq!(digest.len(), 64);
    assert!(digest.bytes().all(|ch| ch.is_ascii_hexdigit()));
    assert_eq!(digest, private_directories_digest(&first).unwrap());
    assert_ne!(digest, private_directories_digest(&second).unwrap());
    assert_ne!(digest, private_directories_digest(&[]).unwrap());
    assert!(private_directories_digest(&vec![PathBuf::from("/a"); 33]).is_err());
}

#[test]
fn inherited_private_digest_cannot_reuse_a_path_only_pre_anchor_claim() {
    use sha2::{Digest, Sha256};
    let directories = [PathBuf::from("/owned-policy-fixture/private")];
    let mut old = Sha256::new();
    old.update(b"sandbox-private-directories-v1\0");
    old.update(serde_json::to_vec(&directories).unwrap());
    assert_ne!(
        private_directories_digest(&directories).unwrap(),
        hex::encode(old.finalize())
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_private_mask_follows_a_writable_ancestor_rename_without_revealing_records() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let parent = workspace.join("parent");
    fs::create_dir_all(&parent).unwrap();
    let private = private_directory(&parent);
    let prepared = prepare_sandboxed_invocation(
        Path::new("/bin/sh"),
        &workspace,
        &[workspace.clone()],
        SandboxNetwork::Deny,
        &[private.clone()],
    )
    .unwrap();
    let output = prepared.into_command().args(["-c", "set -eu; if mv \"$1/parent\" \"$1/moved\"; then active=\"$1/moved\"; else active=\"$1/parent\"; fi; if cat \"$active/private/marker\"; then exit 42; fi; if printf changed > \"$active/private/marker\"; then exit 43; fi; printf public-control > \"$1/result\"; printf rename-control", "fixture"])
        .arg(&workspace).output().unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"rename-control");
    assert_eq!(
        fs::read(workspace.join("result")).unwrap(),
        b"public-control"
    );
    let renamed = workspace.join("moved").exists();
    eprintln!(
        "private_ancestor_renamed={renamed} private_read_blocked=true private_write_blocked=true"
    );
    let parent = if renamed {
        workspace.join("moved")
    } else {
        parent
    };
    assert_eq!(
        fs::read(parent.join("private/marker")).unwrap(),
        b"non-secret-private-marker"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_private_ancestor_cannot_redirect_the_next_invocations_exclusion() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let parent = workspace.join("parent");
    fs::create_dir_all(&parent).unwrap();
    let private = private_directory(&parent);
    fs::write(parent.join("public"), b"public-before").unwrap();
    let prepare = || {
        prepare_sandboxed_invocation(
            Path::new("/bin/sh"),
            &workspace,
            &[workspace.clone()],
            SandboxNetwork::Deny,
            &[private.clone()],
        )
        .unwrap()
        .into_command()
    };
    let moved = prepare().args(["-c", "set -eu; printf public-after > \"$1/parent/public\"; if mv \"$1/parent\" \"$1/moved\"; then mkdir -m 700 \"$1/parent\" \"$1/parent/private\"; printf moved; else printf anchored; fi", "fixture"])
        .arg(&workspace).env_clear().env("PATH", "/usr/bin:/bin").output().unwrap();
    let followup = prepare()
        .args([
            "-c",
            "if cat \"$1/moved/private/marker\" 2>/dev/null; then exit 42; fi; printf protected",
            "fixture",
        ])
        .arg(&workspace)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    eprintln!(
        "PRIVATE_ANCESTOR_WITNESS producer=sdk moved={} next_scope_private_read={} llm_calls=0",
        moved.stdout == b"moved",
        followup.stdout.starts_with(b"non-secret-private-marker")
    );
    assert!(
        moved.status.success(),
        "{}",
        String::from_utf8_lossy(&moved.stderr)
    );
    assert_eq!(moved.stdout, b"anchored");
    assert!(followup.status.success());
    assert_eq!(followup.stdout, b"protected");
    assert_eq!(fs::read(parent.join("public")).unwrap(), b"public-after");
    assert_eq!(
        fs::read(private.join("marker")).unwrap(),
        b"non-secret-private-marker"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_private_ancestor_protection_does_not_upgrade_readonly_access() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let parent = workspace.join("parent");
    fs::create_dir_all(&parent).unwrap();
    let private = private_directory(&parent);
    fs::write(parent.join("public"), b"public-before").unwrap();
    let output = prepare_sandboxed_invocation(
        Path::new("/bin/sh"), &workspace, &[], SandboxNetwork::Deny, &[private],
    ).unwrap().into_command().args(["-c", "set -eu; cat \"$1/parent/public\"; if printf changed > \"$1/parent/public\" 2>/dev/null; then exit 42; fi; if mkdir \"$1/parent/new\" 2>/dev/null; then exit 43; fi", "fixture"])
        .arg(&workspace).env_clear().env("PATH", "/usr/bin:/bin").output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"public-before");
    assert_eq!(fs::read(parent.join("public")).unwrap(), b"public-before");
    assert!(!parent.join("new").exists());
}
