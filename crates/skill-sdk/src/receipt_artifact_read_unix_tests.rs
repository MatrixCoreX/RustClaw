use super::*;
use std::os::unix::fs::FileTypeExt;

#[path = "receipt_read_support_tests.rs"]
mod support;
use support::{read_fifo, Fixture, SKILL};

fn trusted_fixture() -> (Fixture, PathBuf) {
    let mut f = Fixture::new();
    // Keep Unix socket paths short even under long macOS temporary roots.
    // A trusted runtime may intentionally live outside the installed package.
    let runtime = f.store.root().join("rt");
    fs::copy(f.install.join(&f.receipt.launch.program), &runtime).unwrap();
    f.receipt.launch.program_scope = LaunchProgramScope::TrustedRuntime;
    f.receipt.launch.program = runtime.display().to_string();
    f.receipt.launch.trusted_runtime_sha256 = Some(digest_file(&runtime).unwrap());
    f.store.write_receipt(&f.install, &f.receipt).unwrap();
    f.store.activate(&f.install, &f.receipt).unwrap();
    crate::SkillRuntimeResolver::new(f.store.root())
        .resolve(SKILL)
        .unwrap();
    (f, runtime)
}

#[test]
fn artifact_digest_public_fifo_is_rejected_without_a_reader_peer() {
    let f = Fixture::new();
    let target = f.install.join(&f.receipt.launch.program);
    let selected = target.clone();
    let (result, observed) = read_fifo(&target, move || digest_file(&selected));
    assert!(!observed, "hashing opened a FIFO reader");
    assert_artifact_invalid(result);
    assert!(fs::symlink_metadata(target).unwrap().file_type().is_fifo());
}

#[test]
fn artifact_digest_verified_fifo_after_type_check_needs_no_reader_peer() {
    let f = Fixture::new();
    let target = f.install.join(&f.receipt.launch.program);
    assert!(fs::metadata(&target).unwrap().is_file());
    let selected = target.clone();
    let (result, observed) = read_fifo(&target, move || digest_verified_file(&selected));
    assert!(!observed, "earlier stat did not prevent a later FIFO open");
    assert_artifact_invalid(result);
}

#[test]
fn artifact_digest_public_source_link_to_fifo_is_rejected_without_a_peer() {
    let f = Fixture::new();
    let target = f.install.join(&f.receipt.launch.program);
    let alias = f.install.join("selected-source");
    link(&target, &alias);
    let (result, observed) = read_fifo(&target, move || digest_file(&alias));
    assert!(!observed);
    assert_artifact_invalid(result);
}

#[test]
fn artifact_digest_verified_link_to_fifo_is_rejected_without_a_peer() {
    let f = Fixture::new();
    let target = f.install.join(&f.receipt.launch.program);
    let alias = f.install.join("verified-path");
    link(&target, &alias);
    let (result, observed) = read_fifo(&target, move || digest_verified_file(&alias));
    assert!(!observed);
    assert_artifact_invalid(result);
}

#[test]
fn artifact_digest_socket_is_rejected_without_changing_it() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("socket");
    let _socket = std::os::unix::net::UnixListener::bind(&path).unwrap();
    assert_artifact_invalid(digest_file(&path));
    assert_artifact_invalid(digest_verified_file(&path));
    assert!(fs::symlink_metadata(path).unwrap().file_type().is_socket());
}

#[test]
fn artifact_digest_device_is_not_accepted_as_an_empty_regular_artifact() {
    let path = Path::new("/dev/null");
    assert!(fs::metadata(path).unwrap().file_type().is_char_device());
    assert_artifact_invalid(digest_file(path));
    assert_artifact_invalid(digest_verified_file(path));
}

#[test]
fn artifact_runtime_trusted_fifo_is_rejected_before_waiting_for_bytes() {
    let (f, target) = trusted_fixture();
    let resolver = crate::SkillRuntimeResolver::new(f.store.root());
    let pointer_before = fs::read(&f.pointer).unwrap();
    let (result, observed) = read_fifo(&target, move || resolver.resolve(SKILL));
    assert!(!observed, "runtime only rejected the FIFO after hashing it");
    assert_artifact_invalid(result);
    assert_eq!(fs::read(f.pointer).unwrap(), pointer_before);
}

#[test]
fn artifact_runtime_trusted_socket_is_rejected_before_reading() {
    let (f, target) = trusted_fixture();
    fs::remove_file(&target).unwrap();
    let _socket = std::os::unix::net::UnixListener::bind(&target).unwrap();
    assert_artifact_invalid(crate::SkillRuntimeResolver::new(f.store.root()).resolve(SKILL));
    assert!(fs::symlink_metadata(target)
        .unwrap()
        .file_type()
        .is_socket());
}

#[test]
fn artifact_rollback_previous_fifo_keeps_current_and_its_existing_error() {
    let f = Fixture::new();
    let before = fs::read(&f.pointer).unwrap();
    fs::write(f.pointer.with_file_name("previous.json"), &before).unwrap();
    let program = f.install.join(&f.receipt.launch.program);
    let store = f.store.clone();
    let (result, observed) = read_fifo(&program, move || store.rollback(SKILL));
    assert!(!observed);
    assert_eq!(result.unwrap_err().code, "rollback_artifact_mismatch");
    assert_eq!(fs::read(f.pointer).unwrap(), before);
}

#[test]
fn artifact_runtime_package_fifo_keeps_current_escape_error_without_reading() {
    let f = Fixture::new();
    let program = f.install.join(&f.receipt.launch.program);
    let resolver = crate::SkillRuntimeResolver::new(f.store.root());
    let (result, observed) = read_fifo(&program, move || resolver.resolve(SKILL));
    assert!(!observed);
    assert_eq!(result.unwrap_err().code, "launch_path_escape");
}
