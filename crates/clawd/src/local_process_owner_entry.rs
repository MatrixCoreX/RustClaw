use std::ffi::OsStr;
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::net::{UnixDatagram, UnixStream};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::launch_channel::{LaunchChannel, LaunchChannelFailure};
use super::owner_authority::{AuthorityFailure, OwnerAuthorityBroker};
use super::owner_handshake::{
    HandshakeFailure, OwnerPendingPreparation, OwnerPreparationFailure, OwnerTransfer,
};
use super::owner_privacy::OwnerProcessPrivacy;
use super::owner_protocol::{OwnerFailure, OwnerPhase};
use super::owner_reference::{AuthorityFingerprint, OwnerReferenceFailure};
use super::owner_scope::ScopeFailure;
use super::owner_transport::OwnerDatagram;
use super::startup_protocol::{StartupAck, StartupRejection};

pub(super) const HELPER_FLAG: &str = "--process-owner";
const BOOTSTRAP_FD: i32 = 3;
pub(super) const BOOTSTRAP_READY: &[u8] = b"owner_bootstrap_ready_v1";

// Private bootstrap metadata only; no command line, key bytes, paths or prose.
// The host must have admitted the authority journal and sandbox policy before
// transferring private descriptors after this helper's protected-ready barrier.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OwnerBootstrap {
    pub(super) schema_version: u32,
    pub(super) job_id: Uuid,
    pub(super) authority_fingerprint: [u8; 32],
    pub(super) binding_digest: [u8; 32],
    pub(super) terminate_grace_millis: u64,
}

impl OwnerBootstrap {
    pub(super) fn decode(bytes: &[u8]) -> Result<(Self, AuthorityFingerprint), EntryFailure> {
        if bytes.is_empty() || bytes.len() > super::owner_protocol::MAX_CONTROL_BYTES {
            return Err(EntryFailure::InvalidBootstrap);
        }
        let bootstrap: Self =
            serde_json::from_slice(bytes).map_err(|_| EntryFailure::InvalidBootstrap)?;
        let fingerprint = AuthorityFingerprint::from_slice(&bootstrap.authority_fingerprint)
            .ok_or(EntryFailure::InvalidBootstrap)?;
        if bootstrap.schema_version != 1
            || bootstrap.job_id.is_nil()
            || bootstrap.binding_digest == [0; 32]
        {
            return Err(EntryFailure::InvalidBootstrap);
        }
        Ok((bootstrap, fingerprint))
    }
}

#[derive(Debug)]
pub(crate) enum EntryFailure {
    InvalidInvocation,
    InvalidDescriptor,
    DescriptorIsolation { os_error: Option<i32> },
    InvalidBootstrap,
    AuthorityAdmissionDenied,
    Privacy(ScopeFailure),
    Authority(AuthorityFailure),
    Reference(OwnerReferenceFailure),
    Transport(LaunchChannelFailure),
    Handshake(HandshakeFailure),
    Rejected(StartupRejection),
    Owner(OwnerFailure),
    PackageCustody(crate::package_custody::PackageCustodyFailure),
}

impl std::fmt::Display for EntryFailure {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInvocation => output.write_str("process_owner_invalid_invocation"),
            Self::InvalidDescriptor => output.write_str("process_owner_invalid_descriptor"),
            Self::DescriptorIsolation { os_error } => {
                diagnostic(output, "process_owner_descriptor_isolation", os_error)
            }
            Self::InvalidBootstrap => output.write_str("process_owner_invalid_bootstrap"),
            Self::AuthorityAdmissionDenied => {
                output.write_str("process_authority_admission_denied")
            }
            Self::Privacy(error) => diagnostic(output, "process_owner_privacy", error),
            Self::Authority(error) => diagnostic(output, "process_owner_authority", error),
            Self::Reference(error) => diagnostic(output, "process_owner_reference", error),
            Self::Transport(error) => diagnostic(output, "process_owner_transport", error),
            Self::Handshake(error) => diagnostic(output, "process_owner_handshake", error),
            Self::Rejected(error) => diagnostic(output, "process_owner_rejected", error),
            Self::Owner(error) => diagnostic(output, "process_owner_runtime", error),
            Self::PackageCustody(error) => {
                diagnostic(output, "process_owner_package_custody", error)
            }
        }
    }
}

fn diagnostic(
    output: &mut std::fmt::Formatter<'_>,
    code: &str,
    detail: &impl std::fmt::Debug,
) -> std::fmt::Result {
    write!(output, "{code}:{detail:?}")
}

fn helper_selected(
    args: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<bool, EntryFailure> {
    helper_selected_for_flag(args, HELPER_FLAG)
}

pub(super) fn helper_selected_for_flag(
    args: impl IntoIterator<Item = std::ffi::OsString>,
    flag: &str,
) -> Result<bool, EntryFailure> {
    let mut args = args.into_iter();
    if args.next().as_deref() != Some(OsStr::new(flag)) {
        return Ok(false);
    }
    if args.next().is_some() {
        return Err(EntryFailure::InvalidInvocation);
    }
    Ok(true)
}

pub(crate) fn run_if_requested() -> Option<Result<(), EntryFailure>> {
    match helper_selected(std::env::args_os().skip(1)) {
        Ok(false) => return None,
        Err(error) => return Some(Err(error)),
        Ok(true) => {}
    }
    // Main calls this before allocator tuning, Tokio, config/key loading, or
    // any daemon thread. Never invoke the helper body inside a live daemon.
    Some(unsafe { run_exclusive_helper() })
}

/// # Safety
/// Only an independently execed helper before private input may call this.
/// It closes inherited descriptors above the bootstrap slot process-wide.
pub(super) unsafe fn take_inherited_channel() -> Result<LaunchChannel, EntryFailure> {
    validate_unix_socket(BOOTSTRAP_FD, libc::SOCK_STREAM)?;
    let flags = unsafe { libc::fcntl(BOOTSTRAP_FD, libc::F_GETFD) };
    if flags < 0
        || unsafe { libc::fcntl(BOOTSTRAP_FD, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0
    {
        return Err(EntryFailure::InvalidDescriptor);
    }
    #[cfg(target_os = "linux")]
    if unsafe { libc::syscall(libc::SYS_close_range, 4_u32, u32::MAX, 0_u32) } != 0 {
        return Err(EntryFailure::DescriptorIsolation {
            os_error: std::io::Error::last_os_error().raw_os_error(),
        });
    }
    // Only the empty bootstrap channel crosses exec. Authority/control FDs
    // arrive via SCM_RIGHTS after process protection, never as inherited slots.
    let channel = unsafe { UnixStream::from_raw_fd(BOOTSTRAP_FD) };
    LaunchChannel::new(channel).map_err(EntryFailure::Transport)
}

fn validate_unix_socket(descriptor: i32, expected_kind: i32) -> Result<(), EntryFailure> {
    let mut kind = 0_i32;
    let mut length = std::mem::size_of_val(&kind) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            descriptor,
            libc::SOL_SOCKET,
            libc::SO_TYPE,
            (&mut kind as *mut i32).cast(),
            &mut length,
        )
    } != 0
        || kind != expected_kind
        || length as usize != std::mem::size_of_val(&kind)
    {
        return Err(EntryFailure::InvalidDescriptor);
    }
    let mut address = unsafe { std::mem::zeroed::<libc::sockaddr_un>() };
    let mut length = std::mem::size_of_val(&address) as libc::socklen_t;
    if unsafe {
        libc::getsockname(
            descriptor,
            (&mut address as *mut libc::sockaddr_un).cast(),
            &mut length,
        )
    } != 0
        || address.sun_family as i32 != libc::AF_UNIX
    {
        return Err(EntryFailure::InvalidDescriptor);
    }
    Ok(())
}

unsafe fn run_exclusive_helper() -> Result<(), EntryFailure> {
    let privacy = unsafe { OwnerProcessPrivacy::protect() }.map_err(EntryFailure::Privacy)?;
    let deadline = Instant::now() + Duration::from_secs(120);
    let channel = unsafe { take_inherited_channel()? };
    channel
        .send_control(BOOTSTRAP_READY, deadline)
        .map_err(EntryFailure::Transport)?;
    let (bytes, [parent, control], channel) = channel
        .receive_bootstrap(deadline)
        .map_err(EntryFailure::Transport)?;
    let (bootstrap, fingerprint) = OwnerBootstrap::decode(&bytes)?;
    privacy.validate().map_err(EntryFailure::Privacy)?;
    validate_unix_socket(control.as_raw_fd(), libc::SOCK_DGRAM)?;
    let control = OwnerDatagram::new(UnixDatagram::from(control)).map_err(EntryFailure::Owner)?;
    let store = OwnerAuthorityBroker::from_admitted_directory(File::from(parent))
        .map_err(EntryFailure::Authority)?
        .load_expected(&fingerprint)
        .map_err(EntryFailure::Authority)?;
    let birth = super::instance::observe_process(std::process::id())
        .map_err(|_| EntryFailure::Owner(OwnerFailure::ProbeUnavailable))?
        .birth;
    let credential = store
        .credential_for(bootstrap.job_id, birth, bootstrap.binding_digest)
        .map_err(EntryFailure::Reference)?;
    // All bootstrap/control/authority checks precede this unique unstarted
    // token. Only consuming it can close a pre-Ready failure without an attempt.
    let pending = unsafe {
        OwnerPendingPreparation::new(
            channel,
            credential.clone(),
            bootstrap.binding_digest,
            Duration::from_millis(bootstrap.terminate_grace_millis),
        )
    }
    .map_err(EntryFailure::Owner)?;
    let (prepared, package_lease) = match pending.receive(deadline) {
        Ok(value) => value,
        Err(abandoned) => {
            persist_no_commit(&store, &abandoned.evidence, abandoned.package.as_ref())?;
            return Err(match abandoned.error {
                OwnerPreparationFailure::Transport(error) => EntryFailure::Transport(error),
                OwnerPreparationFailure::Owner(error) => EntryFailure::Owner(error),
            });
        }
    };
    let (mut start, reference) = match unsafe { prepared.wait_commit(&store, deadline) } {
        OwnerTransfer::NotCommitted { error, evidence } => {
            persist_no_commit(&store, &evidence, package_lease.as_ref())?;
            return Err(EntryFailure::Handshake(error));
        }
        OwnerTransfer::Rejected {
            reason,
            acknowledgment,
            evidence,
        } => {
            if let Err(error) = acknowledgment {
                tracing::debug!(?error, "owner_rejection_ack_unavailable");
            }
            persist_no_commit(&store, &evidence, package_lease.as_ref())?;
            return Err(EntryFailure::Rejected(reason));
        }
        OwnerTransfer::Attempted {
            start,
            reference,
            result,
            acknowledgment,
        } => {
            if let Err(error) = acknowledgment {
                tracing::debug!(?error, "owner_start_ack_unavailable");
            }
            match result {
                StartupAck::Started {} => (start, reference),
                StartupAck::AttemptFailed { error_code } => {
                    let evidence = start
                        .startup_failure_evidence()
                        .ok_or(EntryFailure::Owner(OwnerFailure::InvalidState))?;
                    if evidence.observed().2 != error_code {
                        return Err(EntryFailure::Owner(OwnerFailure::InvalidState));
                    }
                    let mut backoff = Duration::from_millis(100);
                    loop {
                        match store.publish_startup_failure(&reference, &evidence) {
                            Ok(receipt) => {
                                if !receipt.authorizes(&credential, &bootstrap.binding_digest)
                                    || receipt.error_code() != error_code
                                {
                                    return Err(EntryFailure::Reference(
                                        OwnerReferenceFailure::Conflict,
                                    ));
                                }
                                if release_package_custody(package_lease.as_ref()).is_ok() {
                                    break;
                                }
                            }
                            Err(failure) => tracing::debug!(
                                ?failure,
                                "owner_startup_outcome_persistence_unavailable"
                            ),
                        }
                        std::thread::sleep(backoff);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                    return Err(EntryFailure::Owner(error_code));
                }
                StartupAck::RejectedBeforeSpawn { reason } => {
                    release_package_custody(package_lease.as_ref())?;
                    return Err(EntryFailure::Rejected(reason));
                }
            }
        }
    };
    let runtime = start.runtime().map_err(EntryFailure::Owner)?;
    let mut persist_after = Instant::now();
    let mut persistence_backoff = Duration::from_millis(100);
    loop {
        let step = control.pump(runtime);
        tracing::debug!(
            received = step.received,
            rejected = step.rejected,
            replies_failed = step.replies_failed,
            transport_unavailable = step.transport_unavailable,
            "owner_control_observation"
        );
        // A lost host/ack never drops a running owner or re-executes its payload.
        // Only actual owned quiescence may mint a terminal persistence attempt.
        if step
            .observation
            .is_ok_and(|snapshot| snapshot.phase == OwnerPhase::Terminal)
            && Instant::now() >= persist_after
        {
            if let Ok(Some(evidence)) = runtime.terminal_evidence() {
                match store.publish_terminal(&reference, &evidence) {
                    Ok(terminal) => {
                        if !terminal.authorizes(&credential, &bootstrap.binding_digest)
                            || terminal.snapshot().phase != OwnerPhase::Terminal
                        {
                            return Err(EntryFailure::Reference(OwnerReferenceFailure::Conflict));
                        }
                        if release_package_custody(package_lease.as_ref()).is_ok() {
                            return Ok(());
                        }
                        persist_after = Instant::now() + persistence_backoff;
                        persistence_backoff =
                            (persistence_backoff * 2).min(Duration::from_secs(30));
                    }
                    Err(_) => {
                        persist_after = Instant::now() + persistence_backoff;
                        persistence_backoff =
                            (persistence_backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn persist_no_commit(
    store: &super::owner_store::OwnerReferenceStore,
    evidence: &super::owner_handshake::NoCommitEvidence,
    package: Option<&crate::package_custody::PackageVersionCustody>,
) -> Result<(), EntryFailure> {
    let (credential, binding) = evidence.observed();
    let mut backoff = Duration::from_millis(100);
    loop {
        let result = store
            .load(credential.job_id)
            .and_then(|reference| reference.ok_or(OwnerReferenceFailure::InvalidReference))
            .and_then(|reference| store.publish_no_commit(&reference, evidence));
        match result {
            Ok(receipt) => {
                if !receipt.authorizes(credential, binding) {
                    return Err(EntryFailure::Reference(OwnerReferenceFailure::Conflict));
                }
                if release_package_custody(package).is_ok() {
                    return Ok(());
                }
            }
            Err(failure) => {
                tracing::debug!(?failure, "owner_no_commit_persistence_unavailable");
            }
        }
        std::thread::sleep(backoff);
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

fn release_package_custody(
    lease: Option<&crate::package_custody::PackageVersionCustody>,
) -> Result<(), EntryFailure> {
    if let Some(lease) = lease {
        lease.release().map_err(|error| {
            tracing::debug!(?error, "owner_package_custody_release_unavailable");
            EntryFailure::PackageCustody(error)
        })?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "local_process_owner_entry_tests.rs"]
pub(super) mod tests;
