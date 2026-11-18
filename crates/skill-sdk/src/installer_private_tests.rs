use std::os::unix::fs::DirBuilderExt;
use std::sync::{atomic::AtomicBool, Arc};

use super::*;

#[test]
fn install_control_clone_and_source_build_policy_preserve_host_private_directories() {
    let root = tempfile::tempdir().unwrap();
    let directories = vec![root.path().to_path_buf()];
    let control = InstallControl::new(Arc::new(AtomicBool::new(false)))
        .with_host_private_directories(directories.clone())
        .without_source_build();
    assert_eq!(control.clone().host_private_directories(), directories);
    assert!(!control.source_build_allowed());
    assert!(!format!("{control:?}").contains(&root.path().display().to_string()));
    assert!(InstallControl::new(Arc::new(AtomicBool::new(false)))
        .host_private_directories()
        .is_empty());
}

#[test]
fn protocol_smoke_uses_host_exclusions_and_never_embeds_them_in_receipts() {
    let root = tempfile::tempdir().unwrap();
    let staging = root.path().join("staging");
    fs::create_dir(&staging).unwrap();
    let private = root.path().join("private");
    fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
    fs::write(private.join("marker"), b"non-secret-private-marker").unwrap();
    fs::write(staging.join("public"), b"public-marker").unwrap();
    let python = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("python3"))
        .find(|path| path.is_file())
        .unwrap()
        .canonicalize()
        .unwrap();
    let manifest = PackageManifest::from_toml_str(crate::tests::manifest_source()).unwrap();
    let platform = HostPlatform::current();
    let prepared = PreparedPackage {
        adapter_version: "fixture-v1".to_string(), artifacts: Vec::new(), phases: Vec::new(),
        launch: ReceiptLaunch { launcher: LauncherKind::Process, program: python.display().to_string(),
            program_scope: LaunchProgramScope::TrustedRuntime,
            args: vec!["-c".to_string(), r#"
import errno, json, pathlib, sys
request = json.loads(sys.stdin.readline())
assert pathlib.Path('public').read_bytes() == b'public-marker'
marker = pathlib.Path(sys.argv[1])
try:
    marker.read_bytes()
except OSError as error:
    assert error.errno in (errno.ENOENT, errno.EACCES, errno.EPERM)
else:
    raise AssertionError('private read unexpectedly allowed')
for target in (marker, marker.with_name('future-record')):
    try:
        target.write_bytes(b'changed')
    except OSError as error:
        assert error.errno in (errno.ENOENT, errno.EACCES, errno.EPERM, errno.EROFS)
    else:
        raise AssertionError('private write unexpectedly allowed')
print(json.dumps({'request_id':request['request_id'], 'status':'ok', 'text':'fixture', 'error_text':None, 'extra':None}))
"#.to_string(), private.join("marker").display().to_string()],
            working_directory: ".".to_string(), environment: BTreeMap::new(), environment_allowlist: Vec::new(),
            trusted_runtime_sha256: None, trusted_runtime_version: None, remote_endpoint: None },
    };
    assert!(protocol_smoke(
        &manifest,
        &prepared,
        &staging,
        root.path(),
        &platform,
        false,
        None
    )
    .is_err());
    let control = InstallControl::new(Arc::new(AtomicBool::new(false)))
        .with_host_private_directories(vec![private.clone()]);
    let receipt = protocol_smoke(
        &manifest,
        &prepared,
        &staging,
        root.path(),
        &platform,
        false,
        Some(&control),
    )
    .unwrap();
    assert!(receipt.passed);
    let serialized = serde_json::to_string(&receipt).unwrap();
    assert!(!serialized.contains(&private.display().to_string()));
    assert!(!serialized.contains("host_private_directories"));
    assert_eq!(
        fs::read(private.join("marker")).unwrap(),
        b"non-secret-private-marker"
    );
    assert!(!private.join("future-record").exists());
}
