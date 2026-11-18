#[cfg(test)]
use std::process::ChildStdin;
use std::process::Command;
use std::time::Duration;

use uuid::Uuid;

use super::instance::observe_process;
use super::owner_protocol::{
    OwnerAction, OwnerCredential, OwnerFailure, OwnerPhase, OwnerReply, OwnerRequest, OwnerSnapshot,
};
use super::owner_scope::OwnedProcessScope;
use super::owner_session::OwnerSession;

pub(crate) struct OwnerRuntime {
    session: OwnerSession,
    scope: OwnedProcessScope,
}

// Only an actual owned scope can create this evidence. A deserialized control
// reply or a root exit alone cannot authorize a durable terminal record.
pub(crate) struct TerminalEvidence {
    credential: OwnerCredential,
    snapshot: OwnerSnapshot,
}

impl TerminalEvidence {
    pub(super) fn observed(&self) -> (&OwnerCredential, OwnerSnapshot) {
        (&self.credential, self.snapshot)
    }
}

impl OwnerRuntime {
    /// # Safety
    /// Must run in an independent one-job owner process satisfying the exact
    /// OwnedProcessScope spawning/reaping contract, never inside the daemon.
    #[cfg(test)]
    pub(crate) unsafe fn spawn(
        command: &mut Command,
        job_id: Uuid,
        nonce: Uuid,
        grace: Duration,
    ) -> Result<Self, OwnerFailure> {
        let credential = Self::prepare_credential(job_id, nonce)?;
        unsafe { Self::spawn_bound(command, credential, grace) }
    }

    pub(crate) fn prepare_credential(
        job_id: Uuid,
        nonce: Uuid,
    ) -> Result<OwnerCredential, OwnerFailure> {
        let credential = OwnerCredential {
            job_id,
            owner: observe_process(std::process::id())
                .map_err(|_| OwnerFailure::ProbeUnavailable)?
                .birth,
            nonce,
        };
        credential.validate()?;
        Ok(credential)
    }

    /// # Safety
    /// Requires the same exclusive owner contract as spawn. The prepared
    /// credential must still identify this exact process before any spawn.
    pub(crate) unsafe fn spawn_bound(
        command: &mut Command,
        credential: OwnerCredential,
        grace: Duration,
    ) -> Result<Self, OwnerFailure> {
        if Self::prepare_credential(credential.job_id, credential.nonce)? != credential {
            return Err(OwnerFailure::CredentialMismatch);
        }
        let session = OwnerSession::new(credential)?;
        let scope =
            unsafe { OwnedProcessScope::spawn(command, grace) }.map_err(OwnerFailure::from)?;
        Ok(Self { session, scope })
    }

    pub(crate) fn credential(&self) -> OwnerCredential {
        self.session.credential()
    }

    #[cfg(test)]
    pub(crate) fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.scope.take_stdin()
    }

    pub(crate) fn handle(&mut self, request: &OwnerRequest) -> Result<OwnerReply, OwnerFailure> {
        let scope = &mut self.scope;
        self.session.handle(request, |action| {
            let snapshot = match action {
                OwnerAction::Inspect => scope.poll(),
                OwnerAction::Cancel => scope.cancel(),
            }
            .map_err(OwnerFailure::from)?;
            OwnerSnapshot::try_from(snapshot)
        })
    }

    pub(crate) fn tick(&mut self) -> Result<OwnerSnapshot, OwnerFailure> {
        OwnerSnapshot::try_from(self.scope.poll().map_err(OwnerFailure::from)?)
    }

    pub(crate) fn terminal_evidence(&mut self) -> Result<Option<TerminalEvidence>, OwnerFailure> {
        let snapshot = self.tick()?;
        Ok(
            (snapshot.phase == OwnerPhase::Terminal).then(|| TerminalEvidence {
                credential: self.credential(),
                snapshot,
            }),
        )
    }
}

#[cfg(test)]
#[path = "local_process_owner_runtime_tests.rs"]
pub(super) mod tests;
