use serde::{Deserialize, Serialize};

use super::owner_outcome::{DurableOwnerOutcome, OwnerObservationRecord};
use super::owner_protocol::OwnerCredential;
use super::owner_reference::{OwnerReference, OwnerReferenceFailure};
use super::owner_store::DurableOwnerReference;

// Physical accounting only, never execution or credential permissions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OwnerResourceBudget {
    pub(crate) memory_mib: u64,
    pub(crate) cpu_cores: usize,
    pub(crate) network_slots: usize,
    pub(crate) provider_slots: usize,
    pub(crate) browser_slots: usize,
    pub(crate) heavy: bool,
    pub(crate) background: bool,
}

impl OwnerResourceBudget {
    pub(crate) fn validate(&self) -> Result<(), OwnerReferenceFailure> {
        if self.memory_mib == 0
            || self.memory_mib > 1_048_576
            || self.cpu_cores == 0
            || self.cpu_cores > 4_096
            || self.network_slots > 4_096
            || self.provider_slots > 4_096
            || self.browser_slots > 64
        {
            return Err(OwnerReferenceFailure::InvalidReference);
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OwnerResourceRecord {
    pub(super) schema_version: u32,
    pub(super) reference: OwnerReference,
    pub(super) budget: OwnerResourceBudget,
}

impl OwnerResourceRecord {
    pub(super) fn validate(&self) -> Result<(), OwnerReferenceFailure> {
        if self.schema_version != 1 {
            return Err(OwnerReferenceFailure::InvalidReference);
        }
        self.reference.validate()?;
        self.budget.validate()
    }
}

// Only the exact protected helper reply creates this observation. A decoded
// budget, public file, PID or missing result is not recovery authority.
pub(crate) struct DurableOwnerResources {
    credential: OwnerCredential,
    binding_digest: [u8; 32],
    budget: Option<OwnerResourceBudget>,
    outcome: Option<DurableOwnerOutcome>,
}

impl DurableOwnerResources {
    /// # Safety
    /// Requires a pinned protected helper channel, exact request/authority
    /// binding and successful exit, not a freely decoded response.
    pub(super) unsafe fn from_protected_observation(
        reference: &DurableOwnerReference,
        budget: Option<OwnerResourceBudget>,
        observation: OwnerObservationRecord,
    ) -> Result<Self, OwnerReferenceFailure> {
        if let Some(budget) = &budget {
            budget.validate()?;
        }
        let outcome = unsafe { observation.into_protected_outcome(reference) }?;
        Ok(Self {
            credential: reference.credential(),
            binding_digest: reference.record().binding_digest,
            budget,
            outcome,
        })
    }

    pub(crate) fn authorizes(&self, reference: &DurableOwnerReference) -> bool {
        reference.authorizes(&self.credential, &self.binding_digest)
    }

    pub(crate) fn binding_digest(&self) -> &[u8; 32] {
        &self.binding_digest
    }
    pub(crate) fn budget(&self) -> Option<&OwnerResourceBudget> {
        self.budget.as_ref()
    }
    pub(crate) fn outcome(&self) -> Option<&DurableOwnerOutcome> {
        self.outcome.as_ref()
    }
}

#[cfg(test)]
#[path = "local_process_owner_resources_tests.rs"]
mod tests;
