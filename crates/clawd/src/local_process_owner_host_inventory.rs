use super::*;
use crate::local_process_owner::authority_entry::inventory::InventoryDecoder;
use crate::local_process_owner::owner_outcome::OwnerObservationRecord;
use crate::local_process_owner::DurableOwnerResources;

// Completeness is established only by the pinned stream and helper exit. No
// Debug/Clone/serde and no constructor from a vector of per-owner proofs.
pub(crate) struct DurableOwnerInventory {
    examined: u64,
    entries: Vec<(DurableOwnerReference, DurableOwnerResources)>,
}

impl DurableOwnerInventory {
    pub(crate) fn examined(&self) -> u64 {
        self.examined
    }
    pub(crate) fn pending(&self) -> usize {
        self.entries.len()
    }
    pub(crate) fn into_entries(self) -> Vec<(DurableOwnerReference, DurableOwnerResources)> {
        self.entries
    }
}

impl AuthorityHost {
    pub(crate) fn inventory_resources(
        &self,
        directories: AuthorityDirectories,
        expected: AuthorityFingerprint,
        deadline: Instant,
    ) -> Result<DurableOwnerInventory, AuthorityHostFailure> {
        let request_id = uuid::Uuid::new_v4();
        let (_, channel, mut child) = self.begin_call(
            directories,
            AuthorityAction::InventoryResources {
                authority_fingerprint: *expected.as_bytes(),
                request_id,
            },
            deadline,
        )?;
        let error = |failure| {
            AuthorityHostFailure::new(
                AuthorityHostPhase::Outcome,
                AuthorityHostReason::Inventory(failure),
            )
        };
        let mut decoder = InventoryDecoder::new(expected, request_id);
        loop {
            let bytes = channel.receive_control(deadline).map_err(|failure| {
                AuthorityHostFailure::new(
                    AuthorityHostPhase::Outcome,
                    AuthorityHostReason::Transport(failure),
                )
            })?;
            if decoder.push(&bytes).map_err(error)? {
                break;
            }
        }
        drop(channel);
        let status = child.wait(deadline)?;
        if !status.success() {
            return Err(AuthorityHostFailure::new(
                AuthorityHostPhase::Exit,
                AuthorityHostReason::HelperExit(ChildExit {
                    code: status.code(),
                    signal: status.signal(),
                }),
            ));
        }
        let (examined, rows) = decoder.finish().map_err(error)?;
        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            // Complete was exact-bound and the pinned protected helper exited
            // successfully. Neither decoded rows nor an EOF alone prove this.
            let reference = unsafe {
                DurableOwnerReference::from_protected_reference(row.credential, row.binding)
            }
            .map_err(|_| error(super::super::inventory::InventoryFailure::InvalidFrame))?;
            let observation = unsafe {
                DurableOwnerResources::from_protected_observation(
                    &reference,
                    Some(row.budget),
                    OwnerObservationRecord::Pending {},
                )
            }
            .map_err(|_| error(super::super::inventory::InventoryFailure::InvalidFrame))?;
            entries.push((reference, observation));
        }
        Ok(DurableOwnerInventory { examined, entries })
    }
}
