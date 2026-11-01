use serde::{Deserialize, Serialize};

use super::owner_handshake::NoCommitEvidence;
use super::owner_protocol::OwnerCredential;
use super::owner_reference::{OwnerReference, OwnerReferenceFailure};
use super::owner_store::DurableOwnerReference;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OwnerNoCommitRecord {
    schema_version: u32,
    pub(super) reference: OwnerReference,
}

impl OwnerNoCommitRecord {
    pub(super) fn from_evidence(
        reference: OwnerReference,
        evidence: &NoCommitEvidence,
    ) -> Result<Self, OwnerReferenceFailure> {
        let (credential, binding) = evidence.observed();
        let writer = super::instance::observe_process(std::process::id())
            .map_err(|_| OwnerReferenceFailure::AuthenticationFailed)?;
        if writer.birth != credential.owner {
            return Err(OwnerReferenceFailure::AuthenticationFailed);
        }
        if credential.job_id != reference.job_id
            || credential.owner != reference.owner
            || binding != &reference.binding_digest
        {
            return Err(OwnerReferenceFailure::Conflict);
        }
        let record = Self {
            schema_version: 1,
            reference,
        };
        record.validate()?;
        Ok(record)
    }

    pub(super) fn validate(&self) -> Result<(), OwnerReferenceFailure> {
        if self.schema_version != 1 {
            return Err(OwnerReferenceFailure::InvalidReference);
        }
        self.reference.validate()
    }
}

// No attempt occurred. This is not scope success or an automatic retry grant.
// Deserialization cannot create the authenticated proof.
pub(crate) struct DurableOwnerNoCommit {
    record: OwnerNoCommitRecord,
    credential: OwnerCredential,
}

impl DurableOwnerNoCommit {
    /// # Safety
    /// Requires a validated pinned protected helper reply, exact authority and
    /// request binding, and confirmed helper exit, never public decoded data.
    pub(super) unsafe fn from_protected_reference(
        reference: &DurableOwnerReference,
    ) -> Result<Self, OwnerReferenceFailure> {
        let record = OwnerNoCommitRecord {
            schema_version: 1,
            reference: reference.record().clone(),
        };
        record.validate()?;
        Ok(Self::verified(record, reference.credential()))
    }

    pub(super) fn verified(record: OwnerNoCommitRecord, credential: OwnerCredential) -> Self {
        Self { record, credential }
    }

    pub(crate) fn authorizes(
        &self,
        credential: &OwnerCredential,
        binding_digest: &[u8; 32],
    ) -> bool {
        &self.credential == credential && &self.record.reference.binding_digest == binding_digest
    }
}

#[cfg(test)]
#[path = "local_process_owner_no_commit_tests.rs"]
mod tests;
