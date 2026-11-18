use std::time::Duration;

use uuid::Uuid;

use super::owner_protocol::{OwnerCredential, OwnerFailure};
use super::owner_runtime::OwnerRuntime;
use super::owner_store::DurableOwnerReference;
use super::process_launch::ProcessLaunch;

enum StartState {
    Prepared(ProcessLaunch),
    Running(OwnerRuntime),
    Failed(OwnerFailure),
}

// Construct only in the dedicated helper. This owns one immutable launch, not
// a deserialized Command. Commit also requires the exact strictly published
// private reference. This is not a host policy grant or a Ready/commit channel.
// No Debug/Clone/serialization.
pub(crate) struct OwnerStart {
    credential: OwnerCredential,
    binding_digest: [u8; 32],
    grace: Duration,
    state: StartState,
}

// Only a consumed local launch failure creates this proof. Control ACKs,
// arbitrary error values and deserialized records cannot construct it.
pub(crate) struct StartupFailureEvidence {
    credential: OwnerCredential,
    binding_digest: [u8; 32],
    error_code: OwnerFailure,
}

impl StartupFailureEvidence {
    pub(super) fn observed(&self) -> (&OwnerCredential, &[u8; 32], OwnerFailure) {
        (&self.credential, &self.binding_digest, self.error_code)
    }
}

impl OwnerStart {
    pub(crate) fn prepare(
        launch: ProcessLaunch,
        job_id: Uuid,
        nonce: Uuid,
        binding_digest: [u8; 32],
        grace: Duration,
    ) -> Result<Self, OwnerFailure> {
        let credential = OwnerRuntime::prepare_credential(job_id, nonce)?;
        if binding_digest == [0; 32] {
            return Err(OwnerFailure::InvalidState);
        }
        Ok(Self {
            credential,
            binding_digest,
            grace,
            state: StartState::Prepared(launch),
        })
    }

    pub(crate) fn credential(&self) -> OwnerCredential {
        self.credential.clone()
    }

    pub(super) fn startup_binding(&self) -> Result<(OwnerCredential, [u8; 32]), OwnerFailure> {
        if !matches!(self.state, StartState::Prepared(_)) {
            return Err(OwnerFailure::InvalidState);
        }
        Ok((self.credential(), self.binding_digest))
    }

    /// # Safety
    /// The caller must satisfy the exclusive one-job owner process contract.
    /// A repeated call returns only the original outcome, never another spawn.
    pub(crate) unsafe fn commit(
        &mut self,
        reference: &DurableOwnerReference,
    ) -> Result<OwnerCredential, OwnerFailure> {
        if !reference.authorizes(&self.credential, &self.binding_digest) {
            return Err(OwnerFailure::CredentialMismatch);
        }
        if matches!(self.state, StartState::Prepared(_)) {
            let StartState::Prepared(launch) = std::mem::replace(
                &mut self.state,
                StartState::Failed(OwnerFailure::InvalidState),
            ) else {
                unreachable!();
            };
            let mut command = launch.into_command();
            self.state = match unsafe {
                OwnerRuntime::spawn_bound(&mut command, self.credential.clone(), self.grace)
            } {
                Ok(runtime) => StartState::Running(runtime),
                Err(error) => StartState::Failed(error),
            };
        }
        match self.state {
            StartState::Running(_) => Ok(self.credential()),
            StartState::Failed(error) => Err(error),
            StartState::Prepared(_) => Err(OwnerFailure::InvalidState),
        }
    }

    pub(crate) fn runtime(&mut self) -> Result<&mut OwnerRuntime, OwnerFailure> {
        match &mut self.state {
            StartState::Running(runtime) => Ok(runtime),
            StartState::Failed(error) => Err(*error),
            StartState::Prepared(_) => Err(OwnerFailure::InvalidState),
        }
    }

    pub(crate) fn startup_failure_evidence(&self) -> Option<StartupFailureEvidence> {
        match self.state {
            StartState::Failed(error_code) => Some(StartupFailureEvidence {
                credential: self.credential(),
                binding_digest: self.binding_digest,
                error_code,
            }),
            StartState::Prepared(_) | StartState::Running(_) => None,
        }
    }
}

#[cfg(test)]
#[path = "local_process_owner_start_tests.rs"]
pub(super) mod tests;
