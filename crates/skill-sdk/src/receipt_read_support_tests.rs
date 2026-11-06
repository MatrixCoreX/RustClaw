//! Shared fixture for the public metadata-read regressions; never production code.
use super::super::*;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::sync::mpsc::{self, TryRecvError};
use std::time::{Duration, Instant};

pub(super) const SKILL: &str = "sample_weather";

pub(super) struct Fixture {
    _root: tempfile::TempDir,
    pub store: InstallReceiptStore,
    pub install: PathBuf,
    pub receipt: InstallReceipt,
    pub pointer: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = InstallReceiptStore::new(root.path());
        let manifest = PackageManifest::from_toml_str(crate::tests::manifest_source())
            .unwrap()
            .into_current()
            .unwrap();
        let digest = manifest.digest().unwrap();
        let install = store
            .version_dir(SKILL, &manifest.package.version, &digest)
            .unwrap();
        let program = install.join(&manifest.run.entrypoint);
        fs::create_dir_all(program.parent().unwrap()).unwrap();
        fs::write(&program, b"fixture, never executed").unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(
            install.join("skill.toml"),
            manifest.to_toml_string().unwrap(),
        )
        .unwrap();
        let receipt = InstallReceipt {
            schema_version: INSTALL_RECEIPT_SCHEMA_VERSION,
            skill_name: SKILL.into(),
            version: manifest.package.version.clone(),
            manifest_digest: digest,
            semantic_contract_digest: Some(manifest.capability_request_digest().unwrap()),
            source_digest: "1".repeat(64),
            lockfile_digests: BTreeMap::new(),
            adapter: manifest.build.adapter,
            adapter_version: "fixture".into(),
            platform: HostPlatform::current(),
            artifacts: vec![ArtifactReceipt {
                path: manifest.run.entrypoint.clone(),
                sha256: digest_file(&program).unwrap(),
                size_bytes: fs::metadata(&program).unwrap().len(),
                executable: true,
            }],
            launch: ReceiptLaunch {
                launcher: manifest.run.launcher,
                program: manifest.run.entrypoint,
                program_scope: LaunchProgramScope::Package,
                args: vec![],
                working_directory: manifest.run.working_directory,
                environment: BTreeMap::new(),
                environment_allowlist: manifest.run.environment_allowlist,
                trusted_runtime_sha256: None,
                trusted_runtime_version: None,
                remote_endpoint: None,
            },
            sandbox_profile: manifest.security.sandbox,
            runtime_network: false,
            protocol_smoke: ProtocolSmokeReceipt {
                protocol: "agent-jsonl-v1".into(),
                passed: true,
                request_id: "metadata-read-fixture".into(),
                checked_at_unix: 1,
            },
            installed_at_unix: 1,
        };
        store.write_receipt(&install, &receipt).unwrap();
        store.activate(&install, &receipt).unwrap();
        let pointer = store.active_install_path(SKILL).unwrap();
        Self {
            _root: root,
            store,
            install,
            receipt,
            pointer,
        }
    }

    pub fn replace_with_link(&self, path: &Path, dangling: bool) -> PathBuf {
        let outside = self
            ._root
            .path()
            .join(format!("outside-{}", Uuid::new_v4()));
        if !dangling {
            fs::copy(path, &outside).unwrap();
        }
        fs::remove_file(path).unwrap();
        std::os::unix::fs::symlink(&outside, path).unwrap();
        outside
    }
}

/// If the old reader opens the FIFO, feed its original bytes and let it finish.
/// A watchdog panic is a failed fixture, never proof that a bad target was rejected.
pub(super) fn read_fifo<T: Send + 'static>(
    path: &Path,
    operation: impl FnOnce() -> SkillSdkResult<T> + Send + 'static,
) -> (SkillSdkResult<T>, bool) {
    let raw = fs::read(path).unwrap();
    fs::remove_file(path).unwrap();
    let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
    let (send, receive) = mpsc::channel();
    let child = std::thread::spawn(move || send.send(operation()).is_ok());
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut reader_observed = false;
    loop {
        match receive.try_recv() {
            Ok(result) => {
                assert!(child.join().unwrap());
                return (result, reader_observed);
            }
            Err(TryRecvError::Disconnected) => panic!("metadata reader panicked"),
            Err(TryRecvError::Empty) => {}
        }
        if !reader_observed {
            match OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(path)
            {
                Ok(mut writer) => {
                    reader_observed = true;
                    writer.write_all(&raw).unwrap();
                }
                Err(error) if error.raw_os_error() == Some(libc::ENXIO) => {}
                Err(error) => panic!("FIFO observation failed: {error}"),
            }
        }
        assert!(
            Instant::now() < deadline,
            "fixture stalled, not a successful target rejection"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

pub(super) fn assert_rejected<T: std::fmt::Debug>(result: SkillSdkResult<T>) {
    let error = result.unwrap_err();
    assert_eq!(error.code, "metadata_read_target_invalid");
    assert_eq!(error.message_key, "skill_sdk.metadata_read_target_invalid");
}
