use super::{ReservedResources, ResourceBroker, ResourceLease};
use crate::local_process_owner::owner_store::DurableOwnerReference;
use crate::local_process_owner::{
    DurableOwnerOutcome, DurableOwnerResources, HostResourceLease, OwnerResourceBudget,
};
use sha2::{Digest, Sha256};

// This is an in-memory association, not a control credential, authorization
// grant, persisted restore record or independently constructible proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct OwnerResourceBinding([u8; 32]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnerResourceFailure {
    InvalidBinding,
    ReservationUnavailable,
    OutcomeMismatch,
    MissingReservationEvidence,
    RecoveryPending,
    NotRecovering,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OwnerResourceRecovery {
    pub(crate) restored: usize,
    pub(crate) released: usize,
}

impl OwnerResourceBinding {
    pub(super) fn checked(
        reference: &DurableOwnerReference,
        execution_binding: &[u8; 32],
    ) -> Result<Self, OwnerResourceFailure> {
        let credential = reference.credential();
        if !reference.authorizes(&credential, execution_binding) {
            return Err(OwnerResourceFailure::InvalidBinding);
        }
        let credential =
            serde_json::to_vec(&credential).map_err(|_| OwnerResourceFailure::InvalidBinding)?;
        let mut digest = Sha256::new();
        digest.update(b"owner-resource-reservation-v1\0");
        digest.update(credential);
        digest.update(execution_binding);
        Ok(Self(digest.finalize().into()))
    }
}

impl HostResourceLease for ResourceLease {
    type Failure = OwnerResourceFailure;

    fn budget(&self) -> OwnerResourceBudget {
        OwnerResourceBudget {
            memory_mib: self.reserved.memory_mib,
            cpu_cores: self.reserved.cpu_cores,
            network_slots: self.reserved.network_slots,
            provider_slots: self.reserved.provider_slots,
            browser_slots: self.reserved.browser_slots,
            heavy: self.reserved.heavy,
            background: self.reserved.background,
        }
    }

    fn escrow(
        self,
        reference: &DurableOwnerReference,
        execution_binding: &[u8; 32],
    ) -> Result<(), Self::Failure> {
        let binding = OwnerResourceBinding::checked(reference, execution_binding)?;
        {
            let mut state = self.broker.state.lock().unwrap();
            if state.owner_recovery_pending {
                return Err(OwnerResourceFailure::RecoveryPending);
            }
            if !state.leases.contains_key(&self.lease_id)
                || state.owner_custody.contains_key(&self.lease_id)
                || state.owner_closed.contains(&binding)
            {
                return Err(OwnerResourceFailure::ReservationUnavailable);
            }
            state.owner_custody.insert(self.lease_id, binding);
        }
        // Drop may now discard the caller's lease object, not the reservation.
        // The broker has no Arc cycle and stores no private credential.
        Ok(())
    }
}

impl ResourceBroker {
    pub(crate) fn reconcile_owner_resources(
        &self,
        reference: &DurableOwnerReference,
        observation: &DurableOwnerResources,
    ) -> Result<OwnerResourceRecovery, OwnerResourceFailure> {
        if !observation.authorizes(reference) {
            return Err(OwnerResourceFailure::InvalidBinding);
        }
        if let Some(outcome) = observation.outcome() {
            let released =
                self.settle_owner_reservations(reference, observation.binding_digest(), outcome)?;
            return Ok(OwnerResourceRecovery {
                restored: 0,
                released,
            });
        }
        let binding = OwnerResourceBinding::checked(reference, observation.binding_digest())?;
        let mut state = self.state.lock().unwrap();
        if state.owner_recovery_pending {
            return Err(OwnerResourceFailure::RecoveryPending);
        }
        if state.owner_closed.contains(&binding) {
            return Ok(OwnerResourceRecovery {
                restored: 0,
                released: 0,
            });
        }
        let budget = observation
            .budget()
            .ok_or(OwnerResourceFailure::MissingReservationEvidence)?;
        budget
            .validate()
            .map_err(|_| OwnerResourceFailure::InvalidBinding)?;
        if state
            .owner_custody
            .values()
            .any(|existing| *existing == binding)
        {
            return Ok(OwnerResourceRecovery {
                restored: 0,
                released: 0,
            });
        }
        let id = state
            .next_lease_id
            .checked_add(1)
            .ok_or(OwnerResourceFailure::ReservationUnavailable)?;
        if state.leases.contains_key(&id) {
            return Err(OwnerResourceFailure::ReservationUnavailable);
        }
        state.leases.insert(
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
        state.owner_custody.insert(id, binding);
        state.next_lease_id = id;
        Ok(OwnerResourceRecovery {
            restored: 1,
            released: 0,
        })
    }

    pub(crate) fn settle_owner_reservations(
        &self,
        reference: &DurableOwnerReference,
        execution_binding: &[u8; 32],
        outcome: &DurableOwnerOutcome,
    ) -> Result<usize, OwnerResourceFailure> {
        let binding = OwnerResourceBinding::checked(reference, execution_binding)?;
        let credential = reference.credential();
        let authorized = match outcome {
            DurableOwnerOutcome::Terminal(terminal) => {
                terminal.authorizes(&credential, execution_binding)
            }
            DurableOwnerOutcome::StartupFailed(failure) => {
                failure.authorizes(&credential, execution_binding)
            }
            DurableOwnerOutcome::NotCommitted(proof) => {
                proof.authorizes(&credential, execution_binding)
            }
        };
        if !authorized {
            return Err(OwnerResourceFailure::OutcomeMismatch);
        }
        let mut state = self.state.lock().unwrap();
        if state.owner_recovery_pending {
            return Err(OwnerResourceFailure::RecoveryPending);
        }
        // Keep closure for this broker's lifetime: an older protected reply may
        // still arrive, even if this broker never held the original reservation.
        state.owner_closed.insert(binding);
        let leases = state
            .owner_custody
            .iter()
            .filter_map(|(id, owned)| (*owned == binding).then_some(*id))
            .collect::<Vec<_>>();
        let mut released = 0;
        for id in leases {
            state.owner_custody.remove(&id);
            released += usize::from(state.leases.remove(&id).is_some());
        }
        Ok(released)
    }
}
