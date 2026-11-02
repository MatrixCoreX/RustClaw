use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use tempfile::tempdir;

use super::*;
use crate::manifest::{BuildAdapter, LauncherKind, PackageManifest, SandboxProfile};
use crate::receipt::{
    ArtifactReceipt, LaunchProgramScope, ProtocolSmokeReceipt, ReceiptLaunch,
    INSTALL_RECEIPT_SCHEMA_VERSION,
};

const SKILL: &str = "sample_weather";
const PROGRAM: &str = "runtime/bin/sample-weather-skill";

fn staged(store: &InstallReceiptStore, bytes: &[u8]) -> (PathBuf, InstallReceipt) {
    let manifest = PackageManifest::from_toml_str(crate::tests::manifest_source())
        .unwrap()
        .into_current()
        .unwrap();
    let staging = store.create_staging_dir(SKILL).unwrap();
    let binary = staging.join(PROGRAM);
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::write(&binary, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(
        staging.join("skill.toml"),
        manifest.to_toml_string().unwrap(),
    )
    .unwrap();
    let receipt = InstallReceipt {
        schema_version: INSTALL_RECEIPT_SCHEMA_VERSION,
        skill_name: SKILL.into(),
        version: manifest.package.version.clone(),
        manifest_digest: manifest.digest().unwrap(),
        semantic_contract_digest: Some(manifest.capability_request_digest().unwrap()),
        source_digest: hex::encode(Sha256::digest(bytes)),
        lockfile_digests: BTreeMap::new(),
        adapter: BuildAdapter::Cargo,
        adapter_version: "fixture".into(),
        platform: crate::HostPlatform::current(),
        artifacts: vec![ArtifactReceipt {
            path: PROGRAM.into(),
            sha256: crate::receipt::digest_file(&binary).unwrap(),
            size_bytes: bytes.len() as u64,
            executable: true,
        }],
        launch: ReceiptLaunch {
            launcher: LauncherKind::Native,
            program: PROGRAM.into(),
            program_scope: LaunchProgramScope::Package,
            args: vec![],
            working_directory: "runtime".into(),
            environment: BTreeMap::new(),
            environment_allowlist: vec![],
            trusted_runtime_sha256: None,
            trusted_runtime_version: None,
            remote_endpoint: None,
        },
        sandbox_profile: SandboxProfile::Required,
        runtime_network: false,
        protocol_smoke: ProtocolSmokeReceipt {
            protocol: "agent-jsonl-v1".into(),
            passed: true,
            request_id: "fixture-smoke".into(),
            checked_at_unix: 1,
        },
        installed_at_unix: 1,
    };
    (staging, receipt)
}

fn commit(store: &InstallReceiptStore, bytes: &[u8]) -> InstallOutcome {
    let (staging, receipt) = staged(store, bytes);
    finish_install(
        store,
        &staging,
        &mut StagingGuard::new(staging.clone()),
        receipt,
        vec!["activate".into()],
        InstallOrigin::BuiltArtifact,
        None,
    )
    .unwrap()
}

#[test]
fn healthy_reinstall_reuses_the_verified_package_and_keeps_rollback() {
    let temp = tempdir().unwrap();
    let store = InstallReceiptStore::new(temp.path());
    let first = commit(&store, b"first");
    let second = commit(&store, b"second");
    let previous = store.skill_root(SKILL).unwrap().join("previous.json");
    let rollback = fs::read(&previous).unwrap();
    let repeated = commit(&store, b"second");
    assert!(repeated.reused);
    assert_eq!(repeated.install_root, second.install_root);
    assert_eq!(fs::read(&previous).unwrap(), rollback);
    assert_eq!(
        store.rollback(SKILL).unwrap().receipt_digest,
        first.receipt_digest
    );
}

#[test]
fn repair_never_reuses_missing_or_same_size_tampered_artifacts_or_metadata() {
    for damage in [
        "missing_binary",
        "tampered_binary",
        "missing_receipt",
        "invalid_receipt",
        "invalid_manifest",
    ] {
        let temp = tempdir().unwrap();
        let store = InstallReceiptStore::new(temp.path());
        let first = commit(&store, b"first");
        let broken = commit(&store, b"second");
        let root = store.skill_root(SKILL).unwrap();
        let rollback = fs::read(root.join("previous.json")).unwrap();
        let lease = store
            .acquire_version_lease(SKILL, &broken.install_root)
            .unwrap();
        store
            .retain_background_version_lease(
                SKILL,
                &broken.install_root,
                "pending-job",
                u64::MAX,
                1,
            )
            .unwrap();
        let background = root.join("background-leases").join(lease.install_dir());
        let lease_files: Vec<_> = fs::read_dir(&background)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        let lease_bytes = fs::read(&lease_files[0]).unwrap();
        match damage {
            "missing_binary" => fs::remove_file(broken.install_root.join(PROGRAM)).unwrap(),
            "tampered_binary" => fs::write(broken.install_root.join(PROGRAM), b"broken").unwrap(),
            "missing_receipt" => {
                fs::remove_file(broken.install_root.join("install-receipt.json")).unwrap()
            }
            "invalid_receipt" => {
                fs::write(broken.install_root.join("install-receipt.json"), b"{").unwrap()
            }
            _ => fs::write(broken.install_root.join("skill.toml"), b"not = [toml").unwrap(),
        }
        let repaired = commit(&store, b"second");
        assert!(!repaired.reused, "{damage}");
        assert_ne!(repaired.install_root, broken.install_root);
        assert!(broken.install_root.is_dir());
        assert_eq!(fs::read(root.join("previous.json")).unwrap(), rollback);
        assert_eq!(fs::read(&lease_files[0]).unwrap(), lease_bytes);
        let resolver = SkillRuntimeResolver::new(store.root());
        assert_eq!(
            resolver.resolve(SKILL).unwrap().install_root,
            repaired.install_root
        );
        // Identical precompiled receipts must resolve to the verified repair;
        // a corrupt unrelated receipt must not poison the scan.
        assert_eq!(
            resolver
                .resolve_pinned(
                    SKILL,
                    &repaired.version,
                    &resolver.resolve(SKILL).unwrap().manifest_digest,
                    &repaired.receipt_digest
                )
                .unwrap()
                .install_root,
            repaired.install_root
        );
        assert_eq!(
            resolver
                .pin_exact(
                    SKILL,
                    &repaired.version,
                    &resolver.resolve(SKILL).unwrap().manifest_digest,
                    &repaired.receipt_digest
                )
                .unwrap()
                .install_root,
            repaired.install_root
        );
        assert_eq!(
            commit(&store, b"second").install_root,
            repaired.install_root
        );
        assert_eq!(fs::read(root.join("previous.json")).unwrap(), rollback);
        drop(lease);
        assert!(broken.install_root.is_dir());
        assert_eq!(
            store.rollback(SKILL).unwrap().receipt_digest,
            first.receipt_digest
        );
    }
}

#[test]
fn broken_current_pointer_does_not_destroy_existing_rollback() {
    for invalid in [b"{".as_slice(), b"{\"schema_version\":999}".as_slice()] {
        let temp = tempdir().unwrap();
        let store = InstallReceiptStore::new(temp.path());
        let first = commit(&store, b"first");
        commit(&store, b"second");
        let root = store.skill_root(SKILL).unwrap();
        let rollback = fs::read(root.join("previous.json")).unwrap();
        fs::write(root.join("current.json"), invalid).unwrap();
        let repair = commit(&store, b"second");
        assert!(repair.reused);
        assert_eq!(fs::read(root.join("previous.json")).unwrap(), rollback);
        assert_eq!(
            store.rollback(SKILL).unwrap().receipt_digest,
            first.receipt_digest
        );
    }
}

#[test]
fn multiple_healthy_identical_receipts_stay_ambiguous() {
    let temp = tempdir().unwrap();
    let store = InstallReceiptStore::new(temp.path());
    let installed = commit(&store, b"first");
    let (duplicate, receipt) = staged(&store, b"first");
    store.write_receipt(&duplicate, &receipt).unwrap();
    fs::rename(
        &duplicate,
        installed.install_root.parent().unwrap().join("duplicate"),
    )
    .unwrap();
    let resolver = SkillRuntimeResolver::new(store.root());
    assert_eq!(
        resolver
            .resolve_pinned(
                SKILL,
                &receipt.version,
                &receipt.manifest_digest,
                &receipt.digest().unwrap()
            )
            .unwrap_err()
            .code,
        "launch_pinned_version_ambiguous"
    );
}

#[test]
fn failed_new_package_verification_keeps_current_and_removes_own_output() {
    let temp = tempdir().unwrap();
    let store = InstallReceiptStore::new(temp.path());
    commit(&store, b"first");
    let current = store.current_pointer(SKILL).unwrap();
    let (staging, receipt) = staged(&store, b"second");
    fs::remove_file(staging.join(PROGRAM)).unwrap();
    let destination = store
        .version_dir(
            SKILL,
            &receipt.version,
            &receipt_install_identity(&receipt).unwrap(),
        )
        .unwrap();
    let result = finish_install(
        &store,
        &staging,
        &mut StagingGuard::new(staging.clone()),
        receipt,
        vec![],
        InstallOrigin::BuiltArtifact,
        None,
    );
    assert!(result.is_err());
    assert_eq!(store.current_pointer(SKILL).unwrap(), current);
    assert!(!destination.exists());
}

#[test]
fn cancellation_before_activation_keeps_current_and_cleans_published_output() {
    let temp = tempdir().unwrap();
    let store = InstallReceiptStore::new(temp.path());
    commit(&store, b"first");
    let current = store.current_pointer(SKILL).unwrap();
    let (staging, receipt) = staged(&store, b"second");
    let destination = store
        .version_dir(
            SKILL,
            &receipt.version,
            &receipt_install_identity(&receipt).unwrap(),
        )
        .unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&cancelled);
    let control = InstallControl::with_progress(
        cancelled,
        Arc::new(move |_| {
            signal.store(true, Ordering::Release);
        }),
    );
    let error = finish_install(
        &store,
        &staging,
        &mut StagingGuard::new(staging.clone()),
        receipt,
        vec![],
        InstallOrigin::BuiltArtifact,
        Some(&control),
    )
    .unwrap_err();
    assert_eq!(error.code, "process_cancelled");
    assert_eq!(store.current_pointer(SKILL).unwrap(), current);
    assert!(!destination.exists());
}

#[test]
fn separate_commits_fail_retryably_instead_of_racing_activation() {
    let temp = tempdir().unwrap();
    let store = InstallReceiptStore::new(temp.path());
    fs::create_dir_all(store.skill_root(SKILL).unwrap()).unwrap();
    let held = commit_lock(&store, SKILL).unwrap();
    let error = commit_lock(&store, SKILL).unwrap_err();
    assert_eq!(error.code, "install_commit_busy");
    assert!(error.retryable);
    drop(held);
    assert!(commit_lock(&store, SKILL).is_ok());
}

#[cfg(unix)]
#[test]
fn symlinked_destination_is_never_overwritten_or_reused() {
    let temp = tempdir().unwrap();
    let store = InstallReceiptStore::new(temp.path().join("packages"));
    let (external, receipt) = staged(&store, b"first");
    store.write_receipt(&external, &receipt).unwrap();
    let external_path = temp.path().join("external");
    fs::rename(&external, &external_path).unwrap();
    let base = store
        .version_dir(
            SKILL,
            &receipt.version,
            &receipt_install_identity(&receipt).unwrap(),
        )
        .unwrap();
    fs::create_dir_all(base.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&external_path, &base).unwrap();
    let repaired = commit(&store, b"first");
    assert!(!repaired.reused);
    assert!(fs::symlink_metadata(&base)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(external_path.join(PROGRAM)).unwrap(), b"first");
    assert_ne!(repaired.install_root, external_path);
}

#[test]
fn healthy_identity_collision_is_not_reused_or_replaced() {
    let temp = tempdir().unwrap();
    let store = InstallReceiptStore::new(temp.path());
    let installed = commit(&store, b"first");
    let (staging, mut receipt) = staged(&store, b"second");
    // Source + manifest alone are insufficient: the complete artifact/launch
    // identity must match even if both claim to derive from the same source.
    receipt.source_digest = hex::encode(Sha256::digest(b"first"));
    let desired = store
        .version_dir(
            SKILL,
            &receipt.version,
            &receipt_install_identity(&receipt).unwrap(),
        )
        .unwrap();
    fs::rename(&installed.install_root, &desired).unwrap();
    let error = finish_install(
        &store,
        &staging,
        &mut StagingGuard::new(staging.clone()),
        receipt,
        vec![],
        InstallOrigin::BuiltArtifact,
        None,
    )
    .unwrap_err();
    assert_eq!(error.code, "install_identity_collision");
    assert_eq!(fs::read(desired.join(PROGRAM)).unwrap(), b"first");
}
