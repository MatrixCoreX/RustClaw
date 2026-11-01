use serde::{Deserialize, Serialize};

use super::owner_no_commit::{DurableOwnerNoCommit, OwnerNoCommitRecord};
use super::owner_protocol::{OwnerCredential, OwnerFailure, OwnerPhase, OwnerSnapshot};
use super::owner_reference::{OwnerReference, OwnerReferenceFailure};
use super::owner_start::StartupFailureEvidence;
use super::owner_store::DurableOwnerReference;
use super::owner_terminal::{DurableOwnerTerminal, OwnerTerminalRecord};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OwnerStartupFailureRecord {
    schema_version: u32,
    pub(super) reference: OwnerReference,
    error_code: OwnerFailure,
}

impl OwnerStartupFailureRecord {
    pub(super) fn from_evidence(
        reference: OwnerReference,
        evidence: &StartupFailureEvidence,
    ) -> Result<Self, OwnerReferenceFailure> {
        let (credential, binding, error_code) = evidence.observed();
        if credential.job_id != reference.job_id
            || credential.owner != reference.owner
            || binding != &reference.binding_digest
        {
            return Err(OwnerReferenceFailure::Conflict);
        }
        let record = Self {
            schema_version: 1,
            reference,
            error_code,
        };
        record.validate()?;
        Ok(record)
    }

    pub(super) fn validate(&self) -> Result<(), OwnerReferenceFailure> {
        self.reference.validate()?;
        if self.schema_version != 1 {
            return Err(OwnerReferenceFailure::InvalidReference);
        }
        Ok(())
    }

    pub(super) fn error_code(&self) -> OwnerFailure {
        self.error_code
    }
}

pub(super) enum OwnerOutcomeRecord {
    Terminal(OwnerTerminalRecord),
    StartupFailed(OwnerStartupFailureRecord),
    NotCommitted(OwnerNoCommitRecord),
}

impl OwnerOutcomeRecord {
    pub(super) fn reference(&self) -> &OwnerReference {
        match self {
            Self::Terminal(record) => &record.reference,
            Self::StartupFailed(record) => &record.reference,
            Self::NotCommitted(record) => &record.reference,
        }
    }
}

// All outcomes compete for one immutable store entry. A startup failure is
// not scope-terminal evidence, task success, an effect-free proof or a retry grant.
pub(crate) enum DurableOwnerOutcome {
    Terminal(DurableOwnerTerminal),
    StartupFailed(DurableOwnerStartupFailure),
    NotCommitted(DurableOwnerNoCommit),
}

// Private wire data, not a proof type. Only the protected authority host may
// turn a checked reply into the non-deserializable durable outcome below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum OwnerObservationRecord {
    Pending {},
    Terminal { snapshot: OwnerSnapshot },
    StartupFailed { error_code: OwnerFailure },
    NotCommitted {},
}

impl OwnerObservationRecord {
    pub(super) fn validate(&self) -> Result<(), OwnerReferenceFailure> {
        if let Self::Terminal { snapshot } = self {
            snapshot
                .validate()
                .map_err(|_| OwnerReferenceFailure::InvalidReference)?;
            if snapshot.phase != OwnerPhase::Terminal {
                return Err(OwnerReferenceFailure::InvalidReference);
            }
        }
        Ok(())
    }

    pub(super) fn from_verified_outcome(outcome: Option<&DurableOwnerOutcome>) -> Self {
        match outcome {
            None => Self::Pending {},
            Some(DurableOwnerOutcome::Terminal(terminal)) => Self::Terminal {
                snapshot: terminal.snapshot(),
            },
            Some(DurableOwnerOutcome::StartupFailed(failure)) => Self::StartupFailed {
                error_code: failure.error_code(),
            },
            Some(DurableOwnerOutcome::NotCommitted(_)) => Self::NotCommitted {},
        }
    }

    /// # Safety
    /// The protected helper channel, exact request binding and successful exit
    /// must already be verified. Deserialization alone never establishes this.
    pub(super) unsafe fn into_protected_outcome(
        self,
        reference: &DurableOwnerReference,
    ) -> Result<Option<DurableOwnerOutcome>, OwnerReferenceFailure> {
        self.validate()?;
        Ok(match self {
            Self::Pending {} => None,
            Self::Terminal { snapshot } => Some(DurableOwnerOutcome::Terminal(unsafe {
                DurableOwnerTerminal::from_protected_snapshot(reference, snapshot)?
            })),
            Self::StartupFailed { error_code } => {
                Some(DurableOwnerOutcome::StartupFailed(unsafe {
                    DurableOwnerStartupFailure::from_protected_failure(reference, error_code)?
                }))
            }
            Self::NotCommitted {} => Some(DurableOwnerOutcome::NotCommitted(unsafe {
                DurableOwnerNoCommit::from_protected_reference(reference)?
            })),
        })
    }
}

pub(crate) struct DurableOwnerStartupFailure {
    record: OwnerStartupFailureRecord,
    credential: OwnerCredential,
}

impl DurableOwnerStartupFailure {
    /// # Safety
    /// Requires the same authenticated private-helper reply and exact bound
    /// reference as the terminal conversion, never a public error response.
    unsafe fn from_protected_failure(
        reference: &DurableOwnerReference,
        error_code: OwnerFailure,
    ) -> Result<Self, OwnerReferenceFailure> {
        let record = OwnerStartupFailureRecord {
            schema_version: 1,
            reference: reference.record().clone(),
            error_code,
        };
        record.validate()?;
        Ok(Self::verified(record, reference.credential()))
    }

    pub(super) fn verified(record: OwnerStartupFailureRecord, credential: OwnerCredential) -> Self {
        Self { record, credential }
    }

    pub(crate) fn authorizes(
        &self,
        credential: &OwnerCredential,
        binding_digest: &[u8; 32],
    ) -> bool {
        &self.credential == credential && &self.record.reference.binding_digest == binding_digest
    }

    pub(crate) fn error_code(&self) -> OwnerFailure {
        self.record.error_code()
    }
}

#[cfg(test)]
#[path = "local_process_owner_outcome_tests.rs"]
pub(super) mod tests;
