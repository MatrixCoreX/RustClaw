use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::protocol::AuthorityAdmissionError;
use crate::local_process_owner::owner_protocol::{OwnerCredential, MAX_CONTROL_BYTES};
use crate::local_process_owner::owner_reference::AuthorityFingerprint;
use crate::local_process_owner::owner_resources::OwnerResourceBudget;
use crate::local_process_owner::owner_store::inventory::MAX_PENDING_OWNERS;

// Private wire frames, never logs, task output or freely constructible proof.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum InventoryFrame {
    Begin {
        schema_version: u32,
        fingerprint: [u8; 32],
        request_id: Uuid,
    },
    Item {
        schema_version: u32,
        fingerprint: [u8; 32],
        request_id: Uuid,
        index: u64,
        credential: OwnerCredential,
        binding: [u8; 32],
        budget: OwnerResourceBudget,
    },
    Complete {
        schema_version: u32,
        fingerprint: [u8; 32],
        request_id: Uuid,
        examined: u64,
        pending: u64,
        digest: [u8; 32],
    },
    Error {
        schema_version: u32,
        error: AuthorityAdmissionError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InventoryFailure {
    InvalidFrame,
    InvalidSequence,
    BindingMismatch,
    LimitExceeded,
    Incomplete,
    Admission(AuthorityAdmissionError),
}

pub(super) struct InventoryRow {
    pub(super) credential: OwnerCredential,
    pub(super) binding: [u8; 32],
    pub(super) budget: OwnerResourceBudget,
}

pub(super) struct InventoryDecoder {
    fingerprint: [u8; 32],
    request_id: Uuid,
    began: bool,
    failed: bool,
    complete: Option<u64>,
    digest: Sha256,
    rows: Vec<InventoryRow>,
    jobs: std::collections::BTreeSet<Uuid>,
}

impl InventoryDecoder {
    pub(super) fn new(fingerprint: AuthorityFingerprint, request_id: Uuid) -> Self {
        Self {
            fingerprint: *fingerprint.as_bytes(),
            request_id,
            began: false,
            failed: false,
            complete: None,
            digest: Sha256::new(),
            rows: Vec::new(),
            jobs: Default::default(),
        }
    }

    pub(super) fn push(&mut self, bytes: &[u8]) -> Result<bool, InventoryFailure> {
        let result = self.apply(bytes);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn apply(&mut self, bytes: &[u8]) -> Result<bool, InventoryFailure> {
        if bytes.is_empty()
            || bytes.len() > MAX_CONTROL_BYTES
            || self.complete.is_some()
            || self.failed
        {
            return Err(InventoryFailure::InvalidFrame);
        }
        let frame: InventoryFrame =
            serde_json::from_slice(bytes).map_err(|_| InventoryFailure::InvalidFrame)?;
        let (version, fingerprint, request_id) = match &frame {
            InventoryFrame::Begin {
                schema_version,
                fingerprint,
                request_id,
            }
            | InventoryFrame::Item {
                schema_version,
                fingerprint,
                request_id,
                ..
            }
            | InventoryFrame::Complete {
                schema_version,
                fingerprint,
                request_id,
                ..
            } => (*schema_version, *fingerprint, *request_id),
            InventoryFrame::Error {
                schema_version: 1,
                error,
            } => return Err(InventoryFailure::Admission(error.clone())),
            InventoryFrame::Error { .. } => return Err(InventoryFailure::InvalidFrame),
        };
        if version != 1 || fingerprint != self.fingerprint || request_id != self.request_id {
            return Err(InventoryFailure::BindingMismatch);
        }
        match frame {
            InventoryFrame::Begin { .. } if !self.began => {
                self.began = true;
            }
            InventoryFrame::Item {
                index,
                credential,
                binding,
                budget,
                ..
            } if self.began => {
                if self.rows.len() >= MAX_PENDING_OWNERS {
                    return Err(InventoryFailure::LimitExceeded);
                }
                if index != self.rows.len() as u64
                    || credential.validate().is_err()
                    || binding == [0; 32]
                    || budget.validate().is_err()
                    || !self.jobs.insert(credential.job_id)
                {
                    return Err(InventoryFailure::InvalidSequence);
                }
                self.rows.push(InventoryRow {
                    credential,
                    binding,
                    budget,
                });
            }
            InventoryFrame::Complete {
                examined,
                pending,
                digest,
                ..
            } if self.began => {
                let actual: [u8; 32] = self.digest.clone().finalize().into();
                if pending != self.rows.len() as u64 || examined < pending || actual != digest {
                    return Err(InventoryFailure::Incomplete);
                }
                self.complete = Some(examined);
                return Ok(true);
            }
            _ => return Err(InventoryFailure::InvalidSequence),
        }
        self.digest.update(bytes);
        Ok(false)
    }

    // Still untrusted wire rows. Only the host's successful pinned helper exit
    // can promote these into the non-deserializable resource inventory.
    pub(super) fn finish(self) -> Result<(u64, Vec<InventoryRow>), InventoryFailure> {
        if self.failed {
            return Err(InventoryFailure::Incomplete);
        }
        Ok((
            self.complete.ok_or(InventoryFailure::Incomplete)?,
            self.rows,
        ))
    }
}

#[cfg(test)]
#[path = "local_process_authority_inventory_tests.rs"]
mod tests;
