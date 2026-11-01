use super::owner_custody::{OwnerResourceBinding, OwnerResourceFailure};
use super::*;
use crate::local_process_owner::DurableOwnerInventory;

impl ResourceBroker {
    // Internal startup barrier, not a configuration switch or permission grant.
    // Ordinary host activation must supply its separately proven private host.
    pub(crate) fn for_owner_recovery() -> Self {
        let broker = Self::default();
        broker.state.lock().unwrap().owner_recovery_pending = true;
        broker
    }

    /// # Safety
    /// The caller owns exclusive host admission from the inventory snapshot
    /// through this commit. No other admitted host or in-flight launch may add
    /// reservations outside that snapshot; prior reconciliation observations
    /// have been drained. Private-directory metadata alone
    /// does not establish this whole-host condition.
    pub(crate) unsafe fn complete_owner_recovery(
        &self,
        inventory: DurableOwnerInventory,
    ) -> Result<usize, OwnerResourceFailure> {
        let mut leases = BTreeMap::new();
        let mut custody = BTreeMap::new();
        let mut bindings = BTreeSet::new();
        for (index, (reference, observation)) in inventory.into_entries().into_iter().enumerate() {
            if !observation.authorizes(&reference) || observation.outcome().is_some() {
                return Err(OwnerResourceFailure::InvalidBinding);
            }
            let binding = OwnerResourceBinding::checked(&reference, observation.binding_digest())?;
            if !bindings.insert(binding) {
                return Err(OwnerResourceFailure::InvalidBinding);
            }
            let budget = observation
                .budget()
                .ok_or(OwnerResourceFailure::MissingReservationEvidence)?;
            budget
                .validate()
                .map_err(|_| OwnerResourceFailure::InvalidBinding)?;
            let id = u64::try_from(index)
                .ok()
                .and_then(|index| index.checked_add(1))
                .ok_or(OwnerResourceFailure::ReservationUnavailable)?;
            leases.insert(
                id,
                ReservedResources {
                    memory_mib: budget.memory_mib,
                    cpu_cores: budget.cpu_cores,
                    network_slots: budget.network_slots,
                    provider_slots: budget.provider_slots,
                    browser_slots: budget.browser_slots,
                    heavy: budget.heavy,
                    background: budget.background,
                },
            );
            custody.insert(id, binding);
        }
        let restored = leases.len();
        let mut state = self.state.lock().unwrap();
        if !state.owner_recovery_pending
            || !state.leases.is_empty()
            || !state.owner_custody.is_empty()
            || !state.owner_closed.is_empty()
            || state.next_lease_id != 0
        {
            return Err(OwnerResourceFailure::NotRecovering);
        }
        state.next_lease_id = restored as u64;
        state.leases = leases;
        state.owner_custody = custody;
        // Publish readiness only after the whole verified inventory is applied.
        state.owner_recovery_pending = false;
        Ok(restored)
    }
}

#[cfg(test)]
#[path = "resource_scheduler_owner_recovery_tests.rs"]
mod tests;
