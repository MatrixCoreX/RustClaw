use super::*;
use std::os::unix::fs::{FileTypeExt, MetadataExt};

#[path = "receipt_read_test_support.rs"]
mod support;
use support::{assert_rejected, read_fifo, Fixture, SKILL};

#[test]
fn sdk_read_current_rejects_leaf_link() {
    let f = Fixture::new();
    let original = fs::read(&f.pointer).unwrap();
    let outside = f.replace_with_link(&f.pointer, false);
    assert_rejected(f.store.current_pointer(SKILL));
    assert_eq!(fs::read_link(&f.pointer).unwrap(), outside);
    assert_eq!(fs::read(outside).unwrap(), original);
}

#[test]
fn sdk_read_current_rejects_dangling_leaf_link() {
    let f = Fixture::new();
    let outside = f.replace_with_link(&f.pointer, true);
    assert_rejected(f.store.current_pointer(SKILL));
    assert!(fs::symlink_metadata(&f.pointer)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(!outside.exists());
}

#[test]
fn sdk_read_current_rejects_FIFO_without_opening_a_reader() {
    let f = Fixture::new();
    let store = f.store.clone();
    let (result, observed) = read_fifo(&f.pointer, move || store.current_pointer(SKILL));
    assert!(
        !observed,
        "an invalid pointer must not wait for a FIFO writer"
    );
    assert_rejected(result);
    assert!(fs::symlink_metadata(f.pointer)
        .unwrap()
        .file_type()
        .is_fifo());
}

#[test]
fn sdk_read_activation_rejects_FIFO_before_publishing() {
    let f = Fixture::new();
    let store = f.store.clone();
    let (install, receipt) = (f.install.clone(), f.receipt.clone());
    let (result, observed) = read_fifo(&f.pointer, move || store.activate(&install, &receipt));
    assert!(!observed);
    assert_rejected(result);
    assert!(fs::symlink_metadata(f.pointer)
        .unwrap()
        .file_type()
        .is_fifo());
    assert!(!f
        .store
        .skill_root(SKILL)
        .unwrap()
        .join("previous.json")
        .exists());
}

#[test]
fn sdk_read_rollback_rejects_previous_FIFO_before_changing_current() {
    let f = Fixture::new();
    let previous = f.pointer.with_file_name("previous.json");
    fs::copy(&f.pointer, &previous).unwrap();
    let original = fs::read(&f.pointer).unwrap();
    let store = f.store.clone();
    let (result, observed) = read_fifo(&previous, move || store.rollback(SKILL));
    assert!(!observed);
    assert_rejected(result);
    assert_eq!(fs::read(f.pointer).unwrap(), original);
    assert!(fs::symlink_metadata(previous)
        .unwrap()
        .file_type()
        .is_fifo());
}

#[test]
fn sdk_read_deactivation_rejects_FIFO_without_removing_it() {
    let f = Fixture::new();
    let store = f.store.clone();
    let (result, observed) = read_fifo(&f.pointer, move || store.deactivate_current(SKILL));
    assert!(!observed);
    assert_rejected(result);
    assert!(fs::symlink_metadata(f.pointer)
        .unwrap()
        .file_type()
        .is_fifo());
}

#[test]
fn sdk_read_stored_metadata_rejects_receipt_link() {
    let f = Fixture::new();
    f.replace_with_link(&f.install.join("install-receipt.json"), false);
    assert_rejected(f.store.verified_current_metadata(SKILL));
}

#[test]
fn sdk_read_runtime_rejects_receipt_link() {
    let f = Fixture::new();
    f.replace_with_link(&f.install.join("install-receipt.json"), false);
    assert_rejected(crate::SkillRuntimeResolver::new(f.store.root()).resolve(SKILL));
}

#[test]
fn sdk_read_runtime_rejects_receipt_FIFO_without_a_peer() {
    let f = Fixture::new();
    let resolver = crate::SkillRuntimeResolver::new(f.store.root());
    let (result, observed) = read_fifo(&f.install.join("install-receipt.json"), move || {
        resolver.resolve(SKILL)
    });
    assert!(!observed);
    assert_rejected(result);
}

#[test]
fn sdk_read_runtime_rejects_installed_manifest_link() {
    let f = Fixture::new();
    f.replace_with_link(&f.install.join("skill.toml"), false);
    assert_rejected(crate::SkillRuntimeResolver::new(f.store.root()).resolve(SKILL));
}

#[test]
fn sdk_read_runtime_rejects_installed_manifest_FIFO_without_a_peer() {
    let f = Fixture::new();
    let resolver = crate::SkillRuntimeResolver::new(f.store.root());
    let (result, observed) = read_fifo(&f.install.join("skill.toml"), move || {
        resolver.resolve(SKILL)
    });
    assert!(!observed);
    assert_rejected(result);
}

#[test]
fn sdk_read_source_manifest_keeps_explicit_regular_file_link_support() {
    let f = Fixture::new();
    let manifest = f.install.join("skill.toml");
    f.replace_with_link(&manifest, false);
    assert_eq!(
        PackageManifest::load(&manifest).unwrap().digest().unwrap(),
        f.receipt.manifest_digest
    );
}

#[test]
fn sdk_read_source_manifest_rejects_FIFO_without_a_peer() {
    let f = Fixture::new();
    let manifest = f.install.join("skill.toml");
    let selected = manifest.clone();
    let (result, observed) = read_fifo(&manifest, move || PackageManifest::load(&selected));
    assert!(!observed);
    assert_eq!(result.unwrap_err().code, "manifest_read_failed");
}

#[test]
fn sdk_read_current_directory_is_rejected_without_modification() {
    let f = Fixture::new();
    fs::remove_file(&f.pointer).unwrap();
    fs::create_dir(&f.pointer).unwrap();
    fs::write(f.pointer.join("keep"), b"protected").unwrap();
    assert_rejected(f.store.current_pointer(SKILL));
    assert_eq!(fs::read(f.pointer.join("keep")).unwrap(), b"protected");
}

#[test]
fn sdk_read_current_socket_is_rejected_without_modification() {
    let f = Fixture::new();
    fs::remove_file(&f.pointer).unwrap();
    let _socket = std::os::unix::net::UnixListener::bind(&f.pointer).unwrap();
    assert_rejected(f.store.current_pointer(SKILL));
    assert!(fs::symlink_metadata(f.pointer)
        .unwrap()
        .file_type()
        .is_socket());
}

#[test]
fn sdk_read_missing_and_malformed_current_keep_their_error_contracts() {
    let f = Fixture::new();
    fs::write(&f.pointer, b"malformed").unwrap();
    assert_eq!(
        f.store.current_pointer(SKILL).unwrap_err().code,
        "json_invalid"
    );
    fs::remove_file(&f.pointer).unwrap();
    assert_eq!(
        f.store.current_pointer(SKILL).unwrap_err().code,
        "io_failed"
    );
    assert!(f.store.deactivate_current(SKILL).unwrap().is_none());
}

#[test]
fn sdk_read_regular_pointer_preserves_complete_bytes_and_file_identity() {
    let f = Fixture::new();
    let expected = f.store.current_pointer(SKILL).unwrap();
    let mut raw = vec![b' '; 2 * 1024 * 1024];
    raw.extend_from_slice(&fs::read(&f.pointer).unwrap());
    fs::write(&f.pointer, &raw).unwrap();
    let before = fs::metadata(&f.pointer).unwrap();
    assert_eq!(f.store.current_pointer(SKILL).unwrap(), expected);
    assert_eq!(fs::read(&f.pointer).unwrap(), raw);
    let after = fs::metadata(f.pointer).unwrap();
    assert_eq!(
        (
            before.dev(),
            before.ino(),
            before.mode(),
            before.uid(),
            before.gid()
        ),
        (
            after.dev(),
            after.ino(),
            after.mode(),
            after.uid(),
            after.gid()
        )
    );
}

#[test]
fn sdk_read_exact_pin_ignores_unrelated_symlinked_receipt() {
    let f = Fixture::new();
    let unrelated = f.install.parent().unwrap().join("unrelated-damaged");
    fs::create_dir(&unrelated).unwrap();
    std::os::unix::fs::symlink(
        f.install.join("install-receipt.json"),
        unrelated.join("install-receipt.json"),
    )
    .unwrap();
    let launch = crate::SkillRuntimeResolver::new(f.store.root())
        .resolve_pinned(
            SKILL,
            &f.receipt.version,
            &f.receipt.manifest_digest,
            &f.receipt.digest().unwrap(),
        )
        .unwrap();
    assert_eq!(launch.install_root, f.install.canonicalize().unwrap());
}
