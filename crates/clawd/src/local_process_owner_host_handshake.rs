use std::time::Instant;

use super::launch_channel::LaunchChannel;
use super::owner_handshake::HandshakeFailure;
use super::owner_protocol::{OwnerCredential, OwnerFailure};
use super::owner_store::DurableOwnerReference;
use super::startup_protocol::{self, StartupAck, StartupMessage};

pub(crate) struct HostPendingStart {
    channel: LaunchChannel,
    credential: OwnerCredential,
    binding_digest: [u8; 32],
}
pub(crate) struct HostReadyStart(HostPendingStart);
pub(crate) struct HostCommittedStart {
    pending: HostPendingStart,
    send_failure: Option<HandshakeFailure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommitObservation {
    Acknowledged(StartupAck),
    // Once commit transmission begins, loss is not proof that execution did
    // not start. A caller must reconcile, never automatically spawn again.
    Uncertain(HandshakeFailure),
}

impl HostPendingStart {
    pub(crate) fn new(
        channel: LaunchChannel,
        credential: OwnerCredential,
        binding_digest: [u8; 32],
    ) -> Result<Self, HandshakeFailure> {
        startup_protocol::encode(&credential, binding_digest, StartupMessage::Ready {})
            .map_err(HandshakeFailure::Protocol)?;
        Ok(Self {
            channel,
            credential,
            binding_digest,
        })
    }

    pub(crate) fn wait_ready(self, deadline: Instant) -> Result<HostReadyStart, HandshakeFailure> {
        if self.receive(deadline)? != (StartupMessage::Ready {}) {
            return Err(HandshakeFailure::Protocol(OwnerFailure::InvalidState));
        }
        Ok(HostReadyStart(self))
    }

    fn receive(&self, deadline: Instant) -> Result<StartupMessage, HandshakeFailure> {
        let bytes = self
            .channel
            .receive_control(deadline)
            .map_err(HandshakeFailure::Transport)?;
        startup_protocol::decode(&bytes, &self.credential, &self.binding_digest)
            .map_err(HandshakeFailure::Protocol)
    }

    fn send(&self, message: StartupMessage, deadline: Instant) -> Result<(), HandshakeFailure> {
        let bytes = startup_protocol::encode(&self.credential, self.binding_digest, message)
            .map_err(HandshakeFailure::Protocol)?;
        self.channel
            .send_control(&bytes, deadline)
            .map_err(HandshakeFailure::Transport)
    }
}

impl HostReadyStart {
    pub(crate) fn commit(
        self,
        reference: &DurableOwnerReference,
        deadline: Instant,
    ) -> Result<HostCommittedStart, HandshakeFailure> {
        if !reference.authorizes(&self.0.credential, &self.0.binding_digest) {
            return Err(HandshakeFailure::Protocol(OwnerFailure::CredentialMismatch));
        }
        let send_failure = self.0.send(StartupMessage::Commit {}, deadline).err();
        Ok(HostCommittedStart {
            pending: self.0,
            send_failure,
        })
    }
}

impl HostCommittedStart {
    pub(crate) fn wait_started(self, deadline: Instant) -> CommitObservation {
        if let Some(error) = self.send_failure {
            return CommitObservation::Uncertain(error);
        }
        match self.pending.receive(deadline) {
            Ok(StartupMessage::Acknowledgment { result }) => {
                CommitObservation::Acknowledged(result)
            }
            Ok(_) => {
                CommitObservation::Uncertain(HandshakeFailure::Protocol(OwnerFailure::InvalidState))
            }
            Err(error) => CommitObservation::Uncertain(error),
        }
    }
}
