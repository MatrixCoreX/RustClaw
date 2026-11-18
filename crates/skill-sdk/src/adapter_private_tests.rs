use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;
use std::sync::{atomic::AtomicBool, Arc};

use super::*;

#[test]
fn build_adapter_receives_host_exclusions_without_changing_public_build_outputs() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let private = root.path().join("private");
    fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
    fs::write(private.join("marker"), b"non-secret-private-marker").unwrap();
    let control = InstallControl::new(Arc::new(AtomicBool::new(false)))
        .with_host_private_directories(vec![private.clone()]);
    let manifest = PackageManifest::from_toml_str(crate::tests::manifest_source()).unwrap();
    let platform = HostPlatform::current();
    let context = AdapterContext {
        manifest: &manifest,
        workspace_root: &workspace,
        manifest_dir: &workspace,
        staging_root: &workspace,
        cache_root: &workspace,
        platform: &platform,
        target: None,
        allow_network: false,
        control: Some(&control),
    };
    let mut command = sandbox_command(
        &context,
        Path::new("/bin/sh"),
        &workspace,
        &[workspace.clone()],
    )
    .unwrap();
    let output = command.args(["-c", "set -eu; printf build-output > \"$1/output\"; if cat \"$2\"; then exit 42; fi; printf '%s\\n' adapter-visibility-control", "fixture"])
        .arg(&workspace).arg(private.join("marker")).output().unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"adapter-visibility-control\n");
    assert_eq!(fs::read(workspace.join("output")).unwrap(), b"build-output");
    assert_eq!(
        fs::read(private.join("marker")).unwrap(),
        b"non-secret-private-marker"
    );
}
