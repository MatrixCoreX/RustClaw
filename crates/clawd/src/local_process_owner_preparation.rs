use super::{
    LaunchChannel, LaunchChannelFailure, NoCommitEvidence, OwnerCredential, OwnerFailure,
    OwnerPreparedStart, OwnerStart,
};
use crate::package_custody::PackageVersionCustody;
use std::time::{Duration, Instant};

pub(crate) enum OwnerPreparationFailure {
    Transport(LaunchChannelFailure),
    Owner(OwnerFailure),
}

// Private state, not a decoded claim that no execution occurred.
pub(crate) struct OwnerPendingPreparation {
    channel: Option<LaunchChannel>,
    credential: OwnerCredential,
    binding: [u8; 32],
    grace: Duration,
}

pub(crate) struct AbandonedOwnerPreparation {
    pub(crate) error: OwnerPreparationFailure,
    pub(crate) evidence: NoCommitEvidence,
    pub(crate) package: Option<PackageVersionCustody>,
}

impl OwnerPendingPreparation {
    /// # Safety
    /// The exclusive one-job helper has validated its private bootstrap,
    /// authority and control endpoint. This is its sole preparation; no other
    /// path has attempted or may attempt this job. It must not be constructed
    /// from a public request/error or alongside an escaped launch/start object.
    pub(crate) unsafe fn new(
        channel: LaunchChannel,
        credential: OwnerCredential,
        binding: [u8; 32],
        grace: Duration,
    ) -> Result<Self, OwnerFailure> {
        credential.validate()?;
        if binding == [0; 32] {
            return Err(OwnerFailure::InvalidState);
        }
        let current = super::super::instance::observe_process(std::process::id())
            .map_err(|_| OwnerFailure::ProbeUnavailable)?;
        if current.birth != credential.owner {
            return Err(OwnerFailure::CredentialMismatch);
        }
        Ok(Self {
            channel: Some(channel),
            credential,
            binding,
            grace,
        })
    }

    pub(crate) fn receive(
        mut self,
        deadline: Instant,
    ) -> Result<(OwnerPreparedStart, Option<PackageVersionCustody>), AbandonedOwnerPreparation>
    {
        let received = self
            .channel
            .take()
            .expect("unstarted channel")
            .receive_package_lease(&self.credential, self.binding, deadline);
        let (package, channel) = match received {
            Ok(value) => value,
            Err(error) => return Err(self.abandon(OwnerPreparationFailure::Transport(error), None)),
        };
        let (launch, channel) = match channel.receive(&self.credential, deadline) {
            Ok(value) => value,
            Err(error) => {
                return Err(self.abandon(OwnerPreparationFailure::Transport(error), package))
            }
        };
        let start = match OwnerStart::prepare(
            launch,
            self.credential.job_id,
            self.credential.nonce,
            self.binding,
            self.grace,
        ) {
            Ok(start) => start,
            Err(error) => return Err(self.abandon(OwnerPreparationFailure::Owner(error), package)),
        };
        match OwnerPreparedStart::new(start, channel) {
            Ok(prepared) => Ok((prepared, package)),
            Err(error) => Err(self.abandon(OwnerPreparationFailure::Owner(error), package)),
        }
    }

    fn abandon(
        self,
        error: OwnerPreparationFailure,
        package: Option<PackageVersionCustody>,
    ) -> AbandonedOwnerPreparation {
        AbandonedOwnerPreparation {
            error,
            evidence: NoCommitEvidence {
                credential: self.credential,
                binding_digest: self.binding,
            },
            package,
        }
    }
}

#[cfg(test)]
#[path = "local_process_owner_preparation_tests.rs"]
mod tests;
