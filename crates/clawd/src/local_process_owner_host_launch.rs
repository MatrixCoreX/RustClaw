use super::super::{
    AuthorityChild, AuthorityDirectories, AuthorityFingerprint, AuthorityHost,
    AuthorityHostFailure, HostOwnerChild,
};
use super::PreparedHostOwner;
use crate::local_process_owner::launch_channel::LaunchChannelFailure;
use crate::local_process_owner::owner_handshake::HandshakeFailure;
use crate::local_process_owner::owner_host_handshake::{
    HostCommittedStart, HostPendingStart, HostReadyStart,
};
use crate::local_process_owner::owner_protocol::OwnerCredential;
use crate::local_process_owner::owner_store::DurableOwnerReference;
use crate::local_process_owner::process_launch::ProcessLaunch;
use crate::local_process_owner::OwnerResourceBudget;
use crate::package_custody::PackageVersionCustody;
use std::time::Instant;

#[path = "local_process_owner_submission.rs"]
mod submission;
pub(crate) use submission::HostSubmissionGuard;
use submission::HostSubmissionPermit;

#[path = "local_process_owner_host_package.rs"]
mod package;
use package::UntransferredOwnerPackage;

// The owner component need not know the scheduler. The actual host supplies
// its already admitted lease; this contract cannot grant execution permission.
pub(crate) trait HostResourceLease {
    type Failure: std::fmt::Debug;

    fn budget(&self) -> OwnerResourceBudget;

    fn escrow(
        self,
        reference: &DurableOwnerReference,
        execution_binding: &[u8; 32],
    ) -> Result<(), Self::Failure>;
}

#[derive(Debug)]
pub(crate) enum HostLaunchFailure<E> {
    DeadlineElapsed,
    CallerAbandoned,
    ChildCustodyUnavailable,
    Transfer(LaunchChannelFailure),
    Handshake(HandshakeFailure),
    InvalidBinding,
    Resource(E),
    Authority(AuthorityHostFailure),
    AlreadySettled,
}

// Drop order closes the startup channel before releasing an uncommitted
// reservation. No payload may execute until commit transmission begins.
#[must_use]
pub(crate) struct HostReadyLaunch<R: HostResourceLease> {
    startup: HostReadyStart,
    resources: R,
    credential: OwnerCredential,
    binding: [u8; 32],
    deadline: Instant,
    submission: HostSubmissionPermit,
    #[cfg(test)]
    before_submit: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(test)]
    after_submit: Option<Box<dyn FnOnce() + Send>>,
}

impl PreparedHostOwner {
    // Transfer exact child custody before package/launch transfer. The returned
    // observer may be dropped without losing the waiter; it is not scope proof.
    pub(crate) fn ready_launch<R: HostResourceLease>(
        mut self,
        launch: ProcessLaunch,
        package: Option<PackageVersionCustody>,
        resources: R,
        deadline: Instant,
    ) -> Result<
        (
            HostOwnerChild,
            HostReadyLaunch<R>,
            DurableOwnerReference,
            HostSubmissionGuard,
        ),
        HostLaunchFailure<R::Failure>,
    > {
        let credential = self.reference().credential();
        let binding = self.reference().record().binding_digest;
        let package = UntransferredOwnerPackage::new(package, &credential, binding)
            .map_err(HostLaunchFailure::Transfer)?;
        // Retaining an already prepared child is cleanup, not permission to
        // transfer or start after the original admission deadline has expired.
        let child = match self.reaper.retain(
            self.child
                .take()
                .expect("prepared owner child")
                .into_child(),
        ) {
            Ok(child) => child,
            Err(child) => {
                self.child = Some(AuthorityChild(Some(child)));
                drop(self.channel.take());
                return Err(HostLaunchFailure::ChildCustodyUnavailable);
            }
        };
        let deadline = deadline.min(self.deadline);
        if Instant::now() >= deadline {
            drop(self.channel.take());
            return Err(HostLaunchFailure::DeadlineElapsed);
        }
        let channel = self
            .channel
            .take()
            .expect("prepared owner channel")
            .send_package_lease(package.into_transfer(), &credential, deadline)
            .map_err(HostLaunchFailure::Transfer)?
            .send(launch, credential.clone(), deadline)
            .map_err(HostLaunchFailure::Transfer)?;
        let pending = HostPendingStart::new(channel, credential.clone(), binding)
            .map_err(HostLaunchFailure::Handshake)?;
        #[cfg(test)]
        if let Some(pause) = self.before_ready.take() {
            pause();
        }
        let startup = pending
            .wait_ready(deadline)
            .map_err(HostLaunchFailure::Handshake)?;
        let (caller, submission) = HostSubmissionGuard::pair();
        Ok((
            child,
            HostReadyLaunch {
                startup,
                resources,
                credential,
                binding,
                deadline,
                submission,
                #[cfg(test)]
                before_submit: None,
                #[cfg(test)]
                after_submit: None,
            },
            self.reference.take().expect("prepared owner reference"),
            caller,
        ))
    }
}

impl<R: HostResourceLease> HostReadyLaunch<R> {
    pub(crate) fn commit(
        self,
        authority: &AuthorityHost,
        directories: AuthorityDirectories,
        expected: AuthorityFingerprint,
        reference: &DurableOwnerReference,
        deadline: Instant,
    ) -> Result<HostCommittedStart, HostLaunchFailure<R::Failure>> {
        let deadline = deadline.min(self.deadline);
        if Instant::now() >= deadline {
            return Err(HostLaunchFailure::DeadlineElapsed);
        }
        if !reference.authorizes(&self.credential, &self.binding) {
            return Err(HostLaunchFailure::InvalidBinding);
        }
        if !self.submission.is_open() {
            return Err(HostLaunchFailure::CallerAbandoned);
        }
        let durable = authority
            .hold_resources(
                directories,
                expected,
                reference,
                self.resources.budget(),
                deadline,
            )
            .map_err(HostLaunchFailure::Authority)?;
        if durable.outcome().is_some() {
            return Err(HostLaunchFailure::AlreadySettled);
        }
        #[cfg(test)]
        if let Some(pause) = self.before_submit {
            pause();
        }
        if Instant::now() >= deadline {
            return Err(HostLaunchFailure::DeadlineElapsed);
        }
        if !self.submission.enter() {
            return Err(HostLaunchFailure::CallerAbandoned);
        }
        self.resources
            .escrow(reference, &self.binding)
            .map_err(HostLaunchFailure::Resource)?;
        #[cfg(test)]
        if let Some(pause) = self.after_submit {
            pause();
        }
        // Any uncertain send retains escrow. A startup ACK is not eligible to
        // release it; only the existing authenticated outcome can settle it.
        self.startup
            .commit(reference, deadline)
            .map_err(HostLaunchFailure::Handshake)
    }

    #[cfg(test)]
    pub(crate) fn before_submit_for_test(&mut self, pause: impl FnOnce() + Send + 'static) {
        self.before_submit = Some(Box::new(pause));
    }

    #[cfg(test)]
    pub(crate) fn after_submit_for_test(&mut self, pause: impl FnOnce() + Send + 'static) {
        self.after_submit = Some(Box::new(pause));
    }
}
