use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

#[cfg(target_os = "linux")]
pub(crate) use super::authority_entry::host::hold_resources_discard_reply;
#[cfg(target_os = "linux")]
pub(crate) use super::authority_entry::host::AuthorityHost;
#[cfg(target_os = "linux")]
pub(crate) use super::authority_entry::host::HostLaunchFailure;
#[cfg(target_os = "linux")]
pub(crate) use super::authority_entry::host::OwnerPreparationRequest;
#[cfg(target_os = "linux")]
pub(crate) use super::entry::tests::process::deadline;
#[cfg(target_os = "linux")]
pub(crate) use super::entry::tests::process::keyless::{KeylessFixture, TestControlClient};
#[cfg(target_os = "linux")]
pub(crate) use super::entry::tests::process::OwnedHelper as OwnedPreparedHelper;
#[cfg(target_os = "linux")]
pub(crate) struct OwnedHostHelper(pub(crate) super::authority_entry::host::HostOwnerChild);

#[cfg(target_os = "linux")]
impl OwnedHostHelper {
    pub(crate) fn wait(&mut self) -> std::process::ExitStatus {
        let end = deadline();
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                return status;
            }
            assert!(
                std::time::Instant::now() < end,
                "owned_host_helper_not_exited"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for OwnedHostHelper {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let end = deadline();
            while matches!(self.0.try_wait(), Ok(None)) && std::time::Instant::now() < end {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }
}
pub(crate) use super::launch_channel::package_custody_binding;
#[cfg(target_os = "linux")]
pub(crate) use super::owner_host_handshake::CommitObservation;
pub(crate) use super::owner_protocol::OwnerCredential;
#[cfg(target_os = "linux")]
pub(crate) use super::owner_protocol::{OwnerAction, OwnerPhase};
#[cfg(target_os = "linux")]
pub(crate) use super::owner_reference::AuthorityFingerprint;
#[cfg(target_os = "linux")]
pub(crate) use super::process_launch::{PayloadIo, ProcessLaunch};
#[cfg(target_os = "linux")]
pub(crate) use super::startup_protocol::StartupAck;
#[cfg(target_os = "linux")]
pub(crate) use crate::process_invocation::ProcessInvocation;

#[cfg(target_os = "linux")]
pub(crate) fn authority_host() -> AuthorityHost {
    AuthorityHost::from_test_executable(File::open(super::entry::tests::executable()).unwrap())
        .unwrap()
}

#[cfg(target_os = "linux")]
pub(crate) fn authority_directory(parent: &Path) -> std::path::PathBuf {
    parent.join(super::owner_authority::DIRECTORY)
}

#[cfg(target_os = "linux")]
pub(crate) fn is_startup_failure(
    outcome: super::owner_outcome::DurableOwnerOutcome,
    credential: &OwnerCredential,
    binding: &[u8; 32],
) -> bool {
    matches!(outcome, super::owner_outcome::DurableOwnerOutcome::StartupFailed(failure)
        if failure.authorizes(credential, binding)
        && failure.error_code() == super::owner_protocol::OwnerFailure::SpawnFailed)
}

#[cfg(target_os = "linux")]
pub(crate) fn is_terminal(
    outcome: super::owner_outcome::DurableOwnerOutcome,
    credential: &OwnerCredential,
    binding: &[u8; 32],
) -> bool {
    matches!(outcome, super::owner_outcome::DurableOwnerOutcome::Terminal(terminal)
        if terminal.authorizes(credential, binding)
        && terminal.snapshot().phase == OwnerPhase::Terminal
        && terminal.snapshot().completion.is_some())
}

// A wire/descriptor fixture only. Real SDK collection lives in runner's bridge tests.
pub(crate) fn custody_fixture(
    lease: File,
    path: &Path,
    credential: &OwnerCredential,
    binding: [u8; 32],
) -> crate::package_custody::PackageVersionCustody {
    let binding = package_custody_binding(credential, binding).unwrap();
    let metadata = lease.metadata().unwrap();
    let mut record = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    record.write_all(&serde_json::to_vec(&serde_json::json!({
        "schema_version": 1, "binding": binding, "skill_name": "lease_fixture", "install_dir": "fixture-version",
        "lease_device": metadata.dev(), "lease_inode": metadata.ino(), "released": false,
    })).unwrap()).unwrap();
    record.sync_all().unwrap();
    crate::package_custody::PackageVersionCustody::from_process_handles([lease, record], binding)
        .unwrap()
}
