use super::*;
use crate::local_process_owner::{DurableOwnerResources, OwnerResourceBudget};

impl AuthorityHost {
    pub(crate) fn hold_resources(
        &self,
        directories: AuthorityDirectories,
        expected: AuthorityFingerprint,
        reference: &DurableOwnerReference,
        budget: OwnerResourceBudget,
        deadline: Instant,
    ) -> Result<DurableOwnerResources, AuthorityHostFailure> {
        let record = reference.record();
        let response = self.call(
            directories,
            AuthorityAction::HoldResources {
                authority_fingerprint: *expected.as_bytes(),
                credential: reference.credential(),
                binding_digest: record.binding_digest,
                budget,
            },
            deadline,
        )?;
        Self::resources_response(reference, response)
    }

    pub(crate) fn read_resources(
        &self,
        directories: AuthorityDirectories,
        expected: AuthorityFingerprint,
        reference: &DurableOwnerReference,
        deadline: Instant,
    ) -> Result<DurableOwnerResources, AuthorityHostFailure> {
        let record = reference.record();
        let response = self.call(
            directories,
            AuthorityAction::ReadResources {
                authority_fingerprint: *expected.as_bytes(),
                credential: reference.credential(),
                binding_digest: record.binding_digest,
            },
            deadline,
        )?;
        Self::resources_response(reference, response)
    }

    fn resources_response(
        reference: &DurableOwnerReference,
        response: AuthorityResponse,
    ) -> Result<DurableOwnerResources, AuthorityHostFailure> {
        let AuthorityResponse::Resources {
            budget,
            observation,
            ..
        } = response
        else {
            return Err(AuthorityHostFailure::new(
                AuthorityHostPhase::Outcome,
                AuthorityHostReason::ResponseKindMismatch,
            ));
        };
        // call validated the exact pinned channel, request binding and exit.
        unsafe { DurableOwnerResources::from_protected_observation(reference, budget, observation) }
            .map_err(|_| {
                AuthorityHostFailure::new(
                    AuthorityHostPhase::Outcome,
                    AuthorityHostReason::Response(AuthorityResponseFailure::InvalidReference),
                )
            })
    }
}

pub(super) fn accept_resources(
    action: &AuthorityAction,
    response: &AuthorityResponse,
) -> Result<(), AuthorityHostFailure> {
    let AuthorityResponse::Resources {
        authority_fingerprint,
        reference,
        budget,
        ..
    } = response
    else {
        return Err(AuthorityHostFailure::new(
            AuthorityHostPhase::Outcome,
            AuthorityHostReason::ResponseKindMismatch,
        ));
    };
    let (fingerprint, job, owner, binding, held) = match action {
        AuthorityAction::HoldResources {
            authority_fingerprint,
            credential,
            binding_digest,
            budget,
        } => (
            authority_fingerprint,
            &credential.job_id,
            &credential.owner,
            binding_digest,
            Some(budget),
        ),
        AuthorityAction::ReadResources {
            authority_fingerprint,
            credential,
            binding_digest,
        } => (
            authority_fingerprint,
            &credential.job_id,
            &credential.owner,
            binding_digest,
            None,
        ),
        _ => {
            return Err(AuthorityHostFailure::new(
                AuthorityHostPhase::Outcome,
                AuthorityHostReason::ResponseKindMismatch,
            ))
        }
    };
    if authority_fingerprint != fingerprint
        || reference.job_id != *job
        || reference.owner != *owner
        || reference.binding_digest != *binding
        || held.is_some_and(|value| budget.as_ref() != Some(value))
    {
        return Err(AuthorityHostFailure::new(
            AuthorityHostPhase::Outcome,
            AuthorityHostReason::OutcomeBindingMismatch,
        ));
    }
    Ok(())
}
