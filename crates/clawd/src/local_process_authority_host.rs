use std::fs::File;
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
#[cfg(target_os = "linux")]
use std::os::unix::fs::{FileExt, MetadataExt};
#[cfg(target_os = "linux")]
use std::os::unix::net::UnixStream;
#[cfg(target_os = "linux")]
use std::os::unix::process::CommandExt;
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, ExitStatus};
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::super::exit::ChildExit;
use super::super::instance::ProcessBirthIdentity;
use super::super::launch_channel::{LaunchChannel, LaunchChannelFailure};
use super::super::owner_authority::{validate_directory, AuthorityFailure, AuthorityOperation};
use super::super::owner_outcome::DurableOwnerOutcome;
use super::super::owner_reference::AuthorityFingerprint;
use super::super::owner_store::DurableOwnerReference;
use super::protocol::{
    AuthorityAction, AuthorityAdmissionError, AuthorityBootstrap, AuthorityResponse,
    AuthorityResponseFailure,
};
use super::BOOTSTRAP_READY;
#[cfg(target_os = "linux")]
use super::HELPER_FLAG;

#[path = "local_process_owner_host_prepare.rs"]
mod owner_prepare;

#[path = "local_process_owner_host_resources.rs"]
mod resources;

#[path = "local_process_owner_host_inventory.rs"]
mod inventory;
pub(crate) use inventory::DurableOwnerInventory;

#[path = "local_process_owner_children.rs"]
mod owner_children;
pub(crate) use owner_children::HostOwnerChild;
use owner_children::OwnerChildReaper;

#[cfg(all(test, target_os = "linux"))]
pub(crate) use tests::hold_resources_discard_reply;

#[cfg(all(test, target_os = "linux"))]
pub(crate) use owner_prepare::launch::HostLaunchFailure;
pub(crate) use owner_prepare::launch::HostResourceLease;

pub(crate) struct OwnerPreparationRequest {
    pub(crate) job_id: uuid::Uuid,
    pub(crate) binding_digest: [u8; 32],
    pub(crate) terminate_grace_millis: u64,
}

enum HelperRole {
    Authority,
    Owner,
}

impl HelperRole {
    #[cfg(target_os = "linux")]
    fn flag(self) -> &'static str {
        match self {
            Self::Authority => HELPER_FLAG,
            Self::Owner => super::super::entry::HELPER_FLAG,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthorityHostPhase {
    Prepare,
    ProtectedReady,
    Transfer,
    Outcome,
    Exit,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AuthorityHostReason {
    #[cfg(not(target_os = "linux"))]
    Unsupported,
    InvalidExecutable,
    InvalidRequest,
    InvalidDirectory(AuthorityFailure),
    DeadlineElapsed,
    Io(Option<i32>),
    Transport(LaunchChannelFailure),
    InvalidProtectionBarrier,
    Response(AuthorityResponseFailure),
    ResumeFingerprintMismatch,
    ResponseKindMismatch,
    ReferenceBindingMismatch,
    OutcomeBindingMismatch,
    Admission(AuthorityAdmissionError),
    Inventory(super::inventory::InventoryFailure),
    HelperExit(ChildExit),
}

// Diagnostics contain only closed machine fields. Neither type is a host
// policy grant or proof that private directories are fenced from payloads.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AuthorityHostFailure {
    pub(crate) phase: AuthorityHostPhase,
    pub(crate) reason: AuthorityHostReason,
}

impl AuthorityHostFailure {
    fn new(phase: AuthorityHostPhase, reason: AuthorityHostReason) -> Self {
        Self { phase, reason }
    }

    pub(crate) fn request_may_have_been_sent(&self) -> bool {
        matches!(
            self.phase,
            AuthorityHostPhase::Transfer | AuthorityHostPhase::Outcome | AuthorityHostPhase::Exit
        )
    }
}

pub(crate) struct AuthorityDirectories {
    keys: File,
    journal: File,
}

impl AuthorityDirectories {
    /// # Safety
    /// The host must continuously fence both directories from every managed
    /// payload, including pre-cutover survivors and native filesystem tools.
    /// These metadata checks do not establish that host admission proof.
    pub(crate) unsafe fn from_fenced(
        keys: File,
        journal: File,
    ) -> Result<Self, AuthorityHostFailure> {
        for directory in [&keys, &journal] {
            validate_directory(directory, AuthorityOperation::AdmitParent).map_err(|error| {
                AuthorityHostFailure::new(
                    AuthorityHostPhase::Prepare,
                    AuthorityHostReason::InvalidDirectory(error),
                )
            })?;
        }
        Ok(Self { keys, journal })
    }
}

// Only a pinned executable and public identity live in the host. No master
// key, reference store, control credential or auto-initialization fallback.
pub(crate) struct AuthorityHost {
    #[cfg(target_os = "linux")]
    executable: File,
    #[cfg(target_os = "linux")]
    owner_children: std::sync::Mutex<Option<OwnerChildReaper>>,
}

impl AuthorityHost {
    pub(crate) fn current() -> Result<Self, AuthorityHostFailure> {
        #[cfg(target_os = "linux")]
        {
            // The running inode, not a pathname which deployment may replace.
            let file = File::open("/proc/self/exe").map_err(|error| {
                AuthorityHostFailure::new(
                    AuthorityHostPhase::Prepare,
                    AuthorityHostReason::Io(error.raw_os_error()),
                )
            })?;
            Self::from_pinned_executable(file)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(AuthorityHostFailure::new(
                AuthorityHostPhase::Prepare,
                AuthorityHostReason::Unsupported,
            ))
        }
    }

    #[cfg(target_os = "linux")]
    fn from_pinned_executable(executable: File) -> Result<Self, AuthorityHostFailure> {
        let metadata = executable.metadata().map_err(|error| {
            AuthorityHostFailure::new(
                AuthorityHostPhase::Prepare,
                AuthorityHostReason::Io(error.raw_os_error()),
            )
        })?;
        let mut header = [0; 4];
        if !metadata.is_file()
            || metadata.mode() & 0o111 == 0
            || executable.read_at(&mut header, 0).ok() != Some(4)
            || header != *b"\x7fELF"
        {
            return Err(AuthorityHostFailure::new(
                AuthorityHostPhase::Prepare,
                AuthorityHostReason::InvalidExecutable,
            ));
        }
        Ok(Self {
            executable,
            owner_children: std::sync::Mutex::new(None),
        })
    }

    #[cfg(all(test, target_os = "linux"))]
    pub(in crate::local_process_owner) fn from_test_executable(
        executable: File,
    ) -> Result<Self, AuthorityHostFailure> {
        Self::from_pinned_executable(executable)
    }

    // Run on the host's blocking executor, not an async reactor thread. The
    // caller owns the only waiter for this immediate helper child. A lost
    // response never authorizes adoption, reinitialization or business replay.
    pub(crate) fn initialize(
        &self,
        directories: AuthorityDirectories,
        deadline: Instant,
    ) -> Result<AuthorityFingerprint, AuthorityHostFailure> {
        fingerprint_response(self.call(directories, AuthorityAction::Initialize {}, deadline)?)
    }

    pub(crate) fn resume(
        &self,
        directories: AuthorityDirectories,
        expected: AuthorityFingerprint,
        deadline: Instant,
    ) -> Result<AuthorityFingerprint, AuthorityHostFailure> {
        fingerprint_response(self.call(
            directories,
            AuthorityAction::Resume {
                authority_fingerprint: *expected.as_bytes(),
            },
            deadline,
        )?)
    }

    /// # Safety
    /// The exact independently execed job owner must have passed its protected-
    /// ready barrier. The host retains its startup and resource ownership; this
    /// publication is neither a payload-start grant nor a live/terminal proof.
    pub(crate) unsafe fn publish_reference(
        &self,
        directories: AuthorityDirectories,
        expected: AuthorityFingerprint,
        job_id: uuid::Uuid,
        owner: ProcessBirthIdentity,
        binding_digest: [u8; 32],
        deadline: Instant,
    ) -> Result<DurableOwnerReference, AuthorityHostFailure> {
        let response = self.call(
            directories,
            AuthorityAction::PublishReference {
                authority_fingerprint: *expected.as_bytes(),
                job_id,
                owner,
                binding_digest,
            },
            deadline,
        )?;
        // call checked this exact pinned channel, request binding and exit.
        unsafe { Self::reference_response(response) }
    }

    // Explicit recovery never calls the publisher: an absent reference remains
    // an error even when a process with the supplied birth is currently alive.
    pub(crate) fn load_reference(
        &self,
        directories: AuthorityDirectories,
        expected: AuthorityFingerprint,
        job_id: uuid::Uuid,
        owner: ProcessBirthIdentity,
        binding_digest: [u8; 32],
        deadline: Instant,
    ) -> Result<DurableOwnerReference, AuthorityHostFailure> {
        let response = self.call(
            directories,
            AuthorityAction::LoadReference {
                authority_fingerprint: *expected.as_bytes(),
                job_id,
                owner,
                binding_digest,
            },
            deadline,
        )?;
        unsafe { Self::reference_response(response) }
    }

    /// # Safety
    /// Requires a successful call with its pinned channel, exact request binding
    /// and helper exit. A freely decoded response is not a durable reference.
    unsafe fn reference_response(
        response: AuthorityResponse,
    ) -> Result<DurableOwnerReference, AuthorityHostFailure> {
        let AuthorityResponse::Reference {
            credential,
            binding_digest,
            ..
        } = response
        else {
            return Err(AuthorityHostFailure::new(
                AuthorityHostPhase::Outcome,
                AuthorityHostReason::ResponseKindMismatch,
            ));
        };
        unsafe { DurableOwnerReference::from_protected_reference(credential, binding_digest) }
            .map_err(|_| {
                AuthorityHostFailure::new(
                    AuthorityHostPhase::Outcome,
                    AuthorityHostReason::Response(AuthorityResponseFailure::InvalidReference),
                )
            })
    }

    // Recovery/terminal lookup only. Normal live polls use the private owner
    // channel; this dedicated admission journal must not be written every poll.
    // None is pending/unknown completion, never settled/no-effect evidence.
    pub(crate) fn read_outcome(
        &self,
        directories: AuthorityDirectories,
        expected: AuthorityFingerprint,
        reference: &DurableOwnerReference,
        deadline: Instant,
    ) -> Result<Option<DurableOwnerOutcome>, AuthorityHostFailure> {
        let record = reference.record();
        let response = self.call(
            directories,
            AuthorityAction::ReadOutcome {
                authority_fingerprint: *expected.as_bytes(),
                credential: reference.credential(),
                binding_digest: record.binding_digest,
            },
            deadline,
        )?;
        let AuthorityResponse::Outcome { observation, .. } = response else {
            return Err(AuthorityHostFailure::new(
                AuthorityHostPhase::Outcome,
                AuthorityHostReason::ResponseKindMismatch,
            ));
        };
        // call checked this exact pinned channel, request binding and exit.
        unsafe { observation.into_protected_outcome(reference) }.map_err(|_| {
            AuthorityHostFailure::new(
                AuthorityHostPhase::Outcome,
                AuthorityHostReason::Response(AuthorityResponseFailure::InvalidReference),
            )
        })
    }

    fn call(
        &self,
        directories: AuthorityDirectories,
        action: AuthorityAction,
        deadline: Instant,
    ) -> Result<AuthorityResponse, AuthorityHostFailure> {
        let (action, channel, mut child) = self.begin_call(directories, action, deadline)?;
        let bytes = channel.receive_control(deadline).map_err(|error| {
            AuthorityHostFailure::new(
                AuthorityHostPhase::Outcome,
                AuthorityHostReason::Transport(error),
            )
        })?;
        let response = AuthorityResponse::decode(&bytes).map_err(|error| {
            AuthorityHostFailure::new(
                AuthorityHostPhase::Outcome,
                AuthorityHostReason::Response(error),
            )
        })?;
        drop(channel);
        let status = child.wait(deadline)?;
        accept_response(&action, response, status)
    }

    fn begin_call(
        &self,
        directories: AuthorityDirectories,
        action: AuthorityAction,
        deadline: Instant,
    ) -> Result<(AuthorityAction, LaunchChannel, AuthorityChild), AuthorityHostFailure> {
        use AuthorityHostPhase as Phase;
        use AuthorityHostReason as Reason;
        check_deadline(deadline, Phase::Prepare)?;
        let bytes = serde_json::to_vec(&AuthorityBootstrap {
            schema_version: 1,
            action,
        })
        .map_err(|_| AuthorityHostFailure::new(Phase::Prepare, Reason::InvalidRequest))?;
        let request = AuthorityBootstrap::decode(&bytes)
            .map_err(|error| AuthorityHostFailure::new(Phase::Prepare, Reason::Admission(error)))?;
        let (channel, child) = self.spawn(HelperRole::Authority)?;
        let ready = channel.receive_control(deadline).map_err(|error| {
            AuthorityHostFailure::new(Phase::ProtectedReady, Reason::Transport(error))
        })?;
        if ready != BOOTSTRAP_READY {
            return Err(AuthorityHostFailure::new(
                Phase::ProtectedReady,
                Reason::InvalidProtectionBarrier,
            ));
        }
        check_deadline(deadline, Phase::ProtectedReady)?;
        let channel = channel
            .send_bootstrap(
                &bytes,
                [directories.keys.into(), directories.journal.into()],
                deadline,
            )
            .map_err(|error| {
                AuthorityHostFailure::new(Phase::Transfer, Reason::Transport(error))
            })?;
        Ok((request.action, channel, child))
    }

    #[cfg(target_os = "linux")]
    fn spawn(
        &self,
        role: HelperRole,
    ) -> Result<(LaunchChannel, AuthorityChild), AuthorityHostFailure> {
        let io = |error: std::io::Error| {
            AuthorityHostFailure::new(
                AuthorityHostPhase::Prepare,
                AuthorityHostReason::Io(error.raw_os_error()),
            )
        };
        let (host, peer) = UnixStream::pair().map_err(io)?;
        let executable = duplicate_above_bootstrap(&self.executable).map_err(io)?;
        let peer = duplicate_above_bootstrap(&peer).map_err(io)?;
        let mut command = Command::new(format!("/proc/self/fd/{}", executable.as_raw_fd()));
        command
            .arg(role.flag())
            .env_clear()
            .current_dir("/")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(executable.as_raw_fd(), libc::F_GETFD) < 0
                    || libc::dup2(peer.as_raw_fd(), 3) != 3
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = AuthorityChild(Some(command.spawn().map_err(io)?));
        drop(command);
        let channel = LaunchChannel::new(host).map_err(|error| {
            AuthorityHostFailure::new(
                AuthorityHostPhase::ProtectedReady,
                AuthorityHostReason::Transport(error),
            )
        })?;
        Ok((channel, child))
    }

    #[cfg(not(target_os = "linux"))]
    fn spawn(
        &self,
        _role: HelperRole,
    ) -> Result<(LaunchChannel, AuthorityChild), AuthorityHostFailure> {
        Err(AuthorityHostFailure::new(
            AuthorityHostPhase::Prepare,
            AuthorityHostReason::Unsupported,
        ))
    }

    fn owner_child_reaper(&self) -> Result<OwnerChildReaper, AuthorityHostFailure> {
        #[cfg(target_os = "linux")]
        {
            let failure = |errno| {
                AuthorityHostFailure::new(
                    AuthorityHostPhase::Prepare,
                    AuthorityHostReason::Io(errno),
                )
            };
            let mut reaper = self.owner_children.lock().map_err(|_| failure(None))?;
            if reaper.is_none() {
                *reaper =
                    Some(OwnerChildReaper::new().map_err(|error| failure(error.raw_os_error()))?);
            }
            Ok(reaper
                .as_ref()
                .expect("initialized owner child reaper")
                .clone())
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(AuthorityHostFailure::new(
                AuthorityHostPhase::Prepare,
                AuthorityHostReason::Unsupported,
            ))
        }
    }
}

fn accept_response(
    action: &AuthorityAction,
    response: AuthorityResponse,
    status: ExitStatus,
) -> Result<AuthorityResponse, AuthorityHostFailure> {
    use AuthorityHostPhase as Phase;
    use AuthorityHostReason as Reason;
    let expected_success = response.is_success();
    if status.success() != expected_success || (!expected_success && status.code() != Some(1)) {
        return Err(AuthorityHostFailure::new(
            Phase::Exit,
            Reason::HelperExit(ChildExit {
                code: status.code(),
                signal: status.signal(),
            }),
        ));
    }
    match &response {
        AuthorityResponse::Error { error, .. } => {
            return Err(AuthorityHostFailure::new(
                Phase::Outcome,
                Reason::Admission(error.clone()),
            ))
        }
        AuthorityResponse::Ok {
            authority_fingerprint,
            ..
        } => {
            if !matches!(
                action,
                AuthorityAction::Initialize {} | AuthorityAction::Resume { .. }
            ) {
                return Err(AuthorityHostFailure::new(
                    Phase::Outcome,
                    Reason::ResponseKindMismatch,
                ));
            }
            let fingerprint = AuthorityFingerprint::from_slice(authority_fingerprint)
                .expect("validated authority response");
            if matches!(action, AuthorityAction::Resume { authority_fingerprint: expected }
                    if expected != fingerprint.as_bytes())
            {
                return Err(AuthorityHostFailure::new(
                    Phase::Outcome,
                    Reason::ResumeFingerprintMismatch,
                ));
            }
        }
        AuthorityResponse::Reference {
            authority_fingerprint,
            credential,
            binding_digest,
            ..
        } => {
            let (expected, job_id, owner, binding) = match action {
                AuthorityAction::PublishReference {
                    authority_fingerprint,
                    job_id,
                    owner,
                    binding_digest,
                }
                | AuthorityAction::LoadReference {
                    authority_fingerprint,
                    job_id,
                    owner,
                    binding_digest,
                } => (authority_fingerprint, job_id, owner, binding_digest),
                _ => {
                    return Err(AuthorityHostFailure::new(
                        Phase::Outcome,
                        Reason::ResponseKindMismatch,
                    ))
                }
            };
            if authority_fingerprint != expected
                || credential.job_id != *job_id
                || credential.owner != *owner
                || binding_digest != binding
            {
                return Err(AuthorityHostFailure::new(
                    Phase::Outcome,
                    Reason::ReferenceBindingMismatch,
                ));
            }
        }
        AuthorityResponse::Outcome {
            authority_fingerprint,
            reference,
            ..
        } => {
            let AuthorityAction::ReadOutcome {
                authority_fingerprint: expected,
                credential,
                binding_digest,
            } = action
            else {
                return Err(AuthorityHostFailure::new(
                    Phase::Outcome,
                    Reason::ResponseKindMismatch,
                ));
            };
            if authority_fingerprint != expected
                || reference.job_id != credential.job_id
                || reference.owner != credential.owner
                || reference.binding_digest != *binding_digest
            {
                return Err(AuthorityHostFailure::new(
                    Phase::Outcome,
                    Reason::OutcomeBindingMismatch,
                ));
            }
        }
        AuthorityResponse::Resources { .. } => resources::accept_resources(action, &response)?,
    }
    Ok(response)
}

fn fingerprint_response(
    response: AuthorityResponse,
) -> Result<AuthorityFingerprint, AuthorityHostFailure> {
    match response {
        AuthorityResponse::Ok {
            authority_fingerprint,
            ..
        } => AuthorityFingerprint::from_slice(&authority_fingerprint).ok_or_else(|| {
            AuthorityHostFailure::new(
                AuthorityHostPhase::Outcome,
                AuthorityHostReason::Response(AuthorityResponseFailure::InvalidFingerprint),
            )
        }),
        _ => Err(AuthorityHostFailure::new(
            AuthorityHostPhase::Outcome,
            AuthorityHostReason::ResponseKindMismatch,
        )),
    }
}

#[cfg(target_os = "linux")]
fn duplicate_above_bootstrap(file: &impl AsRawFd) -> std::io::Result<OwnedFd> {
    let fd = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 6) };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

fn check_deadline(
    deadline: Instant,
    phase: AuthorityHostPhase,
) -> Result<(), AuthorityHostFailure> {
    if Instant::now() >= deadline {
        Err(AuthorityHostFailure::new(
            phase,
            AuthorityHostReason::DeadlineElapsed,
        ))
    } else {
        Ok(())
    }
}

struct AuthorityChild(Option<Child>);
impl AuthorityChild {
    fn into_child(mut self) -> Child {
        self.0.take().expect("owned immediate child")
    }

    fn wait(&mut self, deadline: Instant) -> Result<ExitStatus, AuthorityHostFailure> {
        loop {
            if let Some(status) = self
                .0
                .as_mut()
                .expect("owned immediate child")
                .try_wait()
                .map_err(|error| {
                    AuthorityHostFailure::new(
                        AuthorityHostPhase::Exit,
                        AuthorityHostReason::Io(error.raw_os_error()),
                    )
                })?
            {
                return Ok(status);
            }
            check_deadline(deadline, AuthorityHostPhase::Exit)?;
            std::thread::sleep(
                Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
}
impl Drop for AuthorityChild {
    fn drop(&mut self) {
        // Only this immediate, unreaped child is owned. Never signal a saved
        // group or signal at all after an unavailable/reaped observation.
        if let Some(child) = self.0.as_mut() {
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

#[cfg(test)]
#[path = "local_process_authority_host_tests.rs"]
mod tests;
