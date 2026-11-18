use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::instance::{encode_birth_record, ProcessBirthIdentity};
use super::owner_no_commit::OwnerNoCommitRecord;
use super::owner_outcome::{OwnerOutcomeRecord, OwnerStartupFailureRecord};
use super::owner_protocol::OwnerCredential;
use super::owner_terminal::OwnerTerminalRecord;

pub(super) const MAX_REFERENCE_BYTES: usize = 8192;
type Authenticator = Hmac<Sha256>;

#[path = "local_process_owner_resource_signing.rs"]
mod resource_signing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OwnerReferenceFailure {
    InvalidKey,
    InvalidReference,
    AuthenticationFailed,
    Conflict,
    InvalidDirectory,
    StorageUnavailable,
    Busy,
    MissingResourceBudget,
    InventoryLimitExceeded,
}

// Supplied by the host credential broker. No default, file writer, Debug,
// Clone, or serialization: persistent host key custody is not this type's job.
pub(crate) struct OwnerAuthorityKey(Zeroizing<[u8; 32]>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AuthorityFingerprint([u8; 32]);

impl AuthorityFingerprint {
    pub(crate) fn from_slice(bytes: &[u8]) -> Option<Self> {
        let bytes: [u8; 32] = bytes.try_into().ok()?;
        (bytes != [0; 32]).then_some(Self(bytes))
    }

    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OwnerReference {
    pub(super) schema_version: u32,
    pub(super) job_id: Uuid,
    pub(super) owner: ProcessBirthIdentity,
    pub(super) binding_digest: [u8; 32],
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedReference {
    reference: OwnerReference,
    authenticator: [u8; 32],
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum SignedOwnerOutcome {
    NotCommitted {
        receipt: OwnerNoCommitRecord,
        authenticator: [u8; 32],
    },
    Terminal {
        receipt: OwnerTerminalRecord,
        authenticator: [u8; 32],
    },
    StartupFailed {
        receipt: OwnerStartupFailureRecord,
        authenticator: [u8; 32],
    },
}

impl OwnerReference {
    pub(super) fn new(
        job_id: Uuid,
        owner: ProcessBirthIdentity,
        binding_digest: [u8; 32],
    ) -> Result<Self, OwnerReferenceFailure> {
        let reference = Self {
            schema_version: 1,
            job_id,
            owner,
            binding_digest,
        };
        reference.validate()?;
        Ok(reference)
    }

    pub(super) fn validate(&self) -> Result<(), OwnerReferenceFailure> {
        if self.schema_version != 1
            || self.job_id.is_nil()
            || self.binding_digest == [0; 32]
            || encode_birth_record(&self.owner).is_err()
        {
            return Err(OwnerReferenceFailure::InvalidReference);
        }
        Ok(())
    }
}

impl OwnerAuthorityKey {
    #[cfg(test)]
    pub(crate) fn new(key: [u8; 32]) -> Result<Self, OwnerReferenceFailure> {
        Self::from_zeroizing(Zeroizing::new(key))
    }

    pub(crate) fn from_zeroizing(key: Zeroizing<[u8; 32]>) -> Result<Self, OwnerReferenceFailure> {
        if *key == [0; 32] {
            return Err(OwnerReferenceFailure::InvalidKey);
        }
        Ok(Self(key))
    }

    pub(super) fn fingerprint(&self) -> Result<AuthorityFingerprint, OwnerReferenceFailure> {
        let mut authenticator = Authenticator::new_from_slice(self.0.as_ref())
            .map_err(|_| OwnerReferenceFailure::InvalidKey)?;
        authenticator.update(b"owner-authority-fingerprint-v1\0");
        AuthorityFingerprint::from_slice(&authenticator.finalize().into_bytes())
            .ok_or(OwnerReferenceFailure::InvalidKey)
    }

    pub(crate) fn credential_for(
        &self,
        job_id: Uuid,
        owner: ProcessBirthIdentity,
        binding_digest: [u8; 32],
    ) -> Result<OwnerCredential, OwnerReferenceFailure> {
        let reference = OwnerReference::new(job_id, owner, binding_digest)?;
        self.credential(&reference)
    }

    pub(super) fn credential(
        &self,
        reference: &OwnerReference,
    ) -> Result<OwnerCredential, OwnerReferenceFailure> {
        let mut bytes: [u8; 16] = self
            .authenticate(b"owner-control-credential-v1\0", reference)?
            .finalize()
            .into_bytes()[..16]
            .try_into()
            .unwrap();
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Ok(OwnerCredential {
            job_id: reference.job_id,
            owner: reference.owner.clone(),
            nonce: Uuid::from_bytes(bytes),
        })
    }

    pub(super) fn sign(&self, reference: OwnerReference) -> Result<Vec<u8>, OwnerReferenceFailure> {
        let authenticator = self
            .authenticate(b"owner-reference-v1\0", &reference)?
            .finalize()
            .into_bytes()
            .into();
        let bytes = serde_json::to_vec(&SignedReference {
            reference,
            authenticator,
        })
        .map_err(|_| OwnerReferenceFailure::InvalidReference)?;
        if bytes.len() > MAX_REFERENCE_BYTES {
            return Err(OwnerReferenceFailure::InvalidReference);
        }
        Ok(bytes)
    }

    pub(super) fn verify(&self, bytes: &[u8]) -> Result<OwnerReference, OwnerReferenceFailure> {
        if bytes.is_empty() || bytes.len() > MAX_REFERENCE_BYTES {
            return Err(OwnerReferenceFailure::InvalidReference);
        }
        let signed: SignedReference =
            serde_json::from_slice(bytes).map_err(|_| OwnerReferenceFailure::InvalidReference)?;
        self.authenticate(b"owner-reference-v1\0", &signed.reference)?
            .verify_slice(&signed.authenticator)
            .map_err(|_| OwnerReferenceFailure::AuthenticationFailed)?;
        Ok(signed.reference)
    }

    pub(super) fn sign_terminal(
        &self,
        receipt: OwnerTerminalRecord,
    ) -> Result<Vec<u8>, OwnerReferenceFailure> {
        receipt.validate()?;
        let authenticator = self
            .authenticate_serialized(b"owner-terminal-v1\0", &receipt)?
            .finalize()
            .into_bytes()
            .into();
        let bytes = serde_json::to_vec(&SignedOwnerOutcome::Terminal {
            receipt,
            authenticator,
        })
        .map_err(|_| OwnerReferenceFailure::InvalidReference)?;
        if bytes.len() > MAX_REFERENCE_BYTES {
            return Err(OwnerReferenceFailure::InvalidReference);
        }
        Ok(bytes)
    }

    pub(super) fn sign_startup_failure(
        &self,
        receipt: OwnerStartupFailureRecord,
    ) -> Result<Vec<u8>, OwnerReferenceFailure> {
        receipt.validate()?;
        let authenticator = self
            .authenticate_serialized(b"owner-startup-failure-v1\0", &receipt)?
            .finalize()
            .into_bytes()
            .into();
        let bytes = serde_json::to_vec(&SignedOwnerOutcome::StartupFailed {
            receipt,
            authenticator,
        })
        .map_err(|_| OwnerReferenceFailure::InvalidReference)?;
        if bytes.len() > MAX_REFERENCE_BYTES {
            return Err(OwnerReferenceFailure::InvalidReference);
        }
        Ok(bytes)
    }

    pub(super) fn sign_no_commit(
        &self,
        receipt: OwnerNoCommitRecord,
    ) -> Result<Vec<u8>, OwnerReferenceFailure> {
        receipt.validate()?;
        let authenticator = self
            .authenticate_serialized(b"owner-no-commit-v1\0", &receipt)?
            .finalize()
            .into_bytes()
            .into();
        let bytes = serde_json::to_vec(&SignedOwnerOutcome::NotCommitted {
            receipt,
            authenticator,
        })
        .map_err(|_| OwnerReferenceFailure::InvalidReference)?;
        if bytes.len() > MAX_REFERENCE_BYTES {
            return Err(OwnerReferenceFailure::InvalidReference);
        }
        Ok(bytes)
    }

    #[cfg(test)]
    pub(super) fn verify_terminal(
        &self,
        bytes: &[u8],
    ) -> Result<OwnerTerminalRecord, OwnerReferenceFailure> {
        match self.verify_outcome(bytes)? {
            OwnerOutcomeRecord::Terminal(record) => Ok(record),
            OwnerOutcomeRecord::StartupFailed(_) | OwnerOutcomeRecord::NotCommitted(_) => {
                Err(OwnerReferenceFailure::Conflict)
            }
        }
    }

    pub(super) fn verify_outcome(
        &self,
        bytes: &[u8],
    ) -> Result<OwnerOutcomeRecord, OwnerReferenceFailure> {
        if bytes.is_empty() || bytes.len() > MAX_REFERENCE_BYTES {
            return Err(OwnerReferenceFailure::InvalidReference);
        }
        let signed: SignedOwnerOutcome =
            serde_json::from_slice(bytes).map_err(|_| OwnerReferenceFailure::InvalidReference)?;
        match signed {
            SignedOwnerOutcome::NotCommitted {
                receipt,
                authenticator,
            } => {
                receipt.validate()?;
                self.authenticate_serialized(b"owner-no-commit-v1\0", &receipt)?
                    .verify_slice(&authenticator)
                    .map_err(|_| OwnerReferenceFailure::AuthenticationFailed)?;
                Ok(OwnerOutcomeRecord::NotCommitted(receipt))
            }
            SignedOwnerOutcome::Terminal {
                receipt,
                authenticator,
            } => {
                receipt.validate()?;
                self.authenticate_serialized(b"owner-terminal-v1\0", &receipt)?
                    .verify_slice(&authenticator)
                    .map_err(|_| OwnerReferenceFailure::AuthenticationFailed)?;
                Ok(OwnerOutcomeRecord::Terminal(receipt))
            }
            SignedOwnerOutcome::StartupFailed {
                receipt,
                authenticator,
            } => {
                receipt.validate()?;
                self.authenticate_serialized(b"owner-startup-failure-v1\0", &receipt)?
                    .verify_slice(&authenticator)
                    .map_err(|_| OwnerReferenceFailure::AuthenticationFailed)?;
                Ok(OwnerOutcomeRecord::StartupFailed(receipt))
            }
        }
    }

    fn authenticate(
        &self,
        domain: &[u8],
        reference: &OwnerReference,
    ) -> Result<Authenticator, OwnerReferenceFailure> {
        reference.validate()?;
        self.authenticate_serialized(domain, reference)
    }

    fn authenticate_serialized(
        &self,
        domain: &[u8],
        record: &impl Serialize,
    ) -> Result<Authenticator, OwnerReferenceFailure> {
        let bytes =
            serde_json::to_vec(record).map_err(|_| OwnerReferenceFailure::InvalidReference)?;
        let mut authenticator = Authenticator::new_from_slice(self.0.as_ref())
            .map_err(|_| OwnerReferenceFailure::InvalidKey)?;
        authenticator.update(domain);
        authenticator.update(&bytes);
        Ok(authenticator)
    }
}

#[cfg(test)]
#[path = "local_process_owner_reference_tests.rs"]
mod tests;
