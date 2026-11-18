use serde::{Deserialize, Serialize};

use super::owner_protocol::{OwnerCredential, OwnerPhase, OwnerSnapshot};
use super::owner_reference::{OwnerReference, OwnerReferenceFailure};
use super::owner_runtime::TerminalEvidence;
use super::owner_store::DurableOwnerReference;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OwnerTerminalRecord {
    schema_version: u32,
    pub(super) reference: OwnerReference,
    snapshot: OwnerSnapshot,
}

impl OwnerTerminalRecord {
    pub(super) fn from_evidence(
        reference: OwnerReference,
        evidence: &TerminalEvidence,
    ) -> Result<Self, OwnerReferenceFailure> {
        let (credential, snapshot) = evidence.observed();
        if credential.job_id != reference.job_id || credential.owner != reference.owner {
            return Err(OwnerReferenceFailure::Conflict);
        }
        let record = Self {
            schema_version: 1,
            reference,
            snapshot,
        };
        record.validate()?;
        Ok(record)
    }

    pub(super) fn validate(&self) -> Result<(), OwnerReferenceFailure> {
        self.reference.validate()?;
        self.snapshot
            .validate()
            .map_err(|_| OwnerReferenceFailure::InvalidReference)?;
        if self.schema_version != 1 || self.snapshot.phase != OwnerPhase::Terminal {
            return Err(OwnerReferenceFailure::InvalidReference);
        }
        Ok(())
    }

    pub(super) fn snapshot(&self) -> OwnerSnapshot {
        self.snapshot
    }
}

// Constructed only after authenticated, strictly synchronized publication/read.
// Scope completion is not success of the user's task or its verification.
pub(crate) struct DurableOwnerTerminal {
    record: OwnerTerminalRecord,
    credential: OwnerCredential,
}

impl DurableOwnerTerminal {
    /// # Safety
    /// The pinned protected helper must have authenticated this exact bound
    /// record and confirmed its successful reply/exit. Public JSON is not proof.
    pub(super) unsafe fn from_protected_snapshot(
        reference: &DurableOwnerReference,
        snapshot: OwnerSnapshot,
    ) -> Result<Self, OwnerReferenceFailure> {
        let record = OwnerTerminalRecord {
            schema_version: 1,
            reference: reference.record().clone(),
            snapshot,
        };
        record.validate()?;
        Ok(Self::verified(record, reference.credential()))
    }

    pub(super) fn verified(record: OwnerTerminalRecord, credential: OwnerCredential) -> Self {
        Self { record, credential }
    }

    pub(crate) fn authorizes(
        &self,
        credential: &OwnerCredential,
        binding_digest: &[u8; 32],
    ) -> bool {
        &self.credential == credential && &self.record.reference.binding_digest == binding_digest
    }

    pub(crate) fn snapshot(&self) -> OwnerSnapshot {
        self.record.snapshot()
    }
}

#[cfg(test)]
#[path = "local_process_owner_terminal_tests.rs"]
pub(super) mod tests;

#[cfg(test)]
#[path = "local_process_owner_terminal_store_tests.rs"]
pub(super) mod store_tests;
