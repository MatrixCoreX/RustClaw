use std::time::Instant;

use super::launch_channel::{LaunchChannel, LaunchChannelFailure};
use super::owner_protocol::{OwnerCredential, OwnerFailure};
use super::owner_start::OwnerStart;
use super::owner_store::{DurableOwnerReference, OwnerReferenceStore};
use super::startup_protocol::{self, StartupAck, StartupMessage, StartupRejection};

#[path = "local_process_owner_preparation.rs"]
mod preparation;
pub(crate) use preparation::{OwnerPendingPreparation, OwnerPreparationFailure};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HandshakeFailure {
    Transport(LaunchChannelFailure),
    Protocol(OwnerFailure),
}

#[cfg(test)]
use super::owner_host_handshake::{CommitObservation, HostPendingStart};

pub(crate) struct OwnerPreparedStart {
    channel: LaunchChannel,
    start: OwnerStart,
    credential: OwnerCredential,
    binding_digest: [u8; 32],
}

// Only consuming an unstarted preparation or prepared launch creates this
// evidence. No transport error, acknowledgment or decoded record can create it.
pub(crate) struct NoCommitEvidence {
    credential: OwnerCredential,
    binding_digest: [u8; 32],
}

impl NoCommitEvidence {
    pub(super) fn observed(&self) -> (&OwnerCredential, &[u8; 32]) {
        (&self.credential, &self.binding_digest)
    }
}

#[must_use = "retain an attempted owner's runtime even if acknowledgment delivery fails"]
pub(crate) enum OwnerTransfer {
    NotCommitted {
        error: HandshakeFailure,
        evidence: NoCommitEvidence,
    },
    Rejected {
        reason: StartupRejection,
        acknowledgment: Result<(), HandshakeFailure>,
        evidence: NoCommitEvidence,
    },
    Attempted {
        start: OwnerStart,
        reference: DurableOwnerReference,
        result: StartupAck,
        acknowledgment: Result<(), HandshakeFailure>,
    },
}

impl OwnerPreparedStart {
    pub(crate) fn new(start: OwnerStart, channel: LaunchChannel) -> Result<Self, OwnerFailure> {
        let (credential, binding_digest) = start.startup_binding()?;
        Ok(Self {
            channel,
            start,
            credential,
            binding_digest,
        })
    }

    pub(super) fn abandon(self) -> NoCommitEvidence {
        NoCommitEvidence {
            credential: self.credential,
            binding_digest: self.binding_digest,
        }
    }

    /// # Safety
    /// This is an exclusive one-job helper, satisfying OwnerStart::commit's
    /// contract. Never reuse it after rejection/abandonment. Hold attempted
    /// runtime ownership through terminal proof even when the peer disappears.
    pub(crate) unsafe fn wait_commit(
        mut self,
        store: &OwnerReferenceStore,
        deadline: Instant,
    ) -> OwnerTransfer {
        if let Err(error) = self.before_commit(deadline) {
            return OwnerTransfer::NotCommitted {
                error,
                evidence: self.abandon(),
            };
        }
        let reference = match store.load(self.credential.job_id) {
            Ok(Some(reference)) => reference,
            other => {
                let reason = match other {
                    Ok(None) => StartupRejection::MissingReference {},
                    Err(error_code) => StartupRejection::Reference { error_code },
                    Ok(Some(_)) => unreachable!(),
                };
                let acknowledgment = self.send(
                    StartupMessage::Acknowledgment {
                        result: StartupAck::RejectedBeforeSpawn { reason },
                    },
                    deadline,
                );
                return OwnerTransfer::Rejected {
                    reason,
                    acknowledgment,
                    evidence: self.abandon(),
                };
            }
        };
        if Instant::now() >= deadline {
            let reason = StartupRejection::StartupBudgetExpired {};
            let acknowledgment = self.send(
                StartupMessage::Acknowledgment {
                    result: StartupAck::RejectedBeforeSpawn { reason },
                },
                deadline,
            );
            return OwnerTransfer::Rejected {
                reason,
                acknowledgment,
                evidence: self.abandon(),
            };
        }
        let result = match unsafe { self.start.commit(&reference) } {
            Ok(_) => StartupAck::Started {},
            Err(error_code) => StartupAck::AttemptFailed { error_code },
        };
        let acknowledgment = self.send(StartupMessage::Acknowledgment { result }, deadline);
        // In particular, no `?` here may drop a running scope on send failure.
        OwnerTransfer::Attempted {
            start: self.start,
            reference,
            result,
            acknowledgment,
        }
    }

    fn before_commit(&self, deadline: Instant) -> Result<(), HandshakeFailure> {
        self.send(StartupMessage::Ready {}, deadline)?;
        let bytes = self
            .channel
            .receive_control(deadline)
            .map_err(HandshakeFailure::Transport)?;
        let message = startup_protocol::decode(&bytes, &self.credential, &self.binding_digest)
            .map_err(HandshakeFailure::Protocol)?;
        if message != (StartupMessage::Commit {}) {
            return Err(HandshakeFailure::Protocol(OwnerFailure::InvalidState));
        }
        Ok(())
    }

    fn send(&self, message: StartupMessage, deadline: Instant) -> Result<(), HandshakeFailure> {
        let bytes = startup_protocol::encode(&self.credential, self.binding_digest, message)
            .map_err(HandshakeFailure::Protocol)?;
        self.channel
            .send_control(&bytes, deadline)
            .map_err(HandshakeFailure::Transport)
    }
}

#[cfg(test)]
#[path = "local_process_owner_handshake_tests.rs"]
pub(super) mod tests;
