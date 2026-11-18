use super::*;
use crate::local_process_owner::entry::OwnerBootstrap;
use crate::local_process_owner::instance::observe_process;
use crate::local_process_owner::owner_transport::OwnerDatagram;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixDatagram;

#[path = "local_process_owner_host_launch.rs"]
pub(super) mod launch;

// Preparation has not transferred a launch and can never commit a payload.
// No Debug/Clone/serialization of the private channel or reference.
#[must_use]
pub(crate) struct PreparedHostOwner {
    channel: Option<LaunchChannel>,
    child: Option<AuthorityChild>,
    reaper: OwnerChildReaper,
    reference: Option<DurableOwnerReference>,
    deadline: Instant,
    #[cfg(test)]
    before_ready: Option<Box<dyn FnOnce() + Send>>,
}

impl PreparedHostOwner {
    #[cfg(test)]
    pub(crate) fn refuse_child_registration_for_test(&mut self) {
        self.reaper = OwnerChildReaper::closed_for_test();
    }

    #[cfg(test)]
    pub(crate) fn before_ready_for_test(&mut self, pause: impl FnOnce() + Send + 'static) {
        self.before_ready = Some(Box::new(pause));
    }

    pub(crate) fn reference(&self) -> &DurableOwnerReference {
        self.reference.as_ref().expect("prepared owner reference")
    }

    // The caller must retain immediate-child and resource/package custody
    // through commit uncertainty. A Child or ACK is never a terminal proof.
    pub(crate) fn into_launch_parts(mut self) -> (Child, LaunchChannel, DurableOwnerReference) {
        (
            self.child
                .take()
                .expect("prepared owner child")
                .into_child(),
            self.channel.take().expect("prepared owner channel"),
            self.reference.take().expect("prepared owner reference"),
        )
    }
}

impl Drop for PreparedHostOwner {
    fn drop(&mut self) {
        // EOF first permits the pinned helper to stop before force cleanup of
        // this immediate child. No launch has crossed this preparation API.
        drop(self.channel.take());
        if let Some(child) = self.child.as_mut() {
            let _ = child.wait(self.deadline);
        }
    }
}

impl AuthorityHost {
    // Blocking-executor operation, under the existing caller-owned admission
    // budget. AuthorityDirectories carries the unsafe continuous-fencing
    // obligation; this method never creates that grant from filesystem metadata.
    pub(crate) fn prepare_owner(
        &self,
        directories: AuthorityDirectories,
        expected: AuthorityFingerprint,
        request: OwnerPreparationRequest,
        control: UnixDatagram,
        deadline: Instant,
    ) -> Result<PreparedHostOwner, AuthorityHostFailure> {
        use AuthorityHostPhase as Phase;
        use AuthorityHostReason as Reason;
        check_deadline(deadline, Phase::Prepare)?;
        let bytes = serde_json::to_vec(&OwnerBootstrap {
            schema_version: 1,
            job_id: request.job_id,
            authority_fingerprint: *expected.as_bytes(),
            binding_digest: request.binding_digest,
            terminate_grace_millis: request.terminate_grace_millis,
        })
        .map_err(|_| AuthorityHostFailure::new(Phase::Prepare, Reason::InvalidRequest))?;
        OwnerBootstrap::decode(&bytes)
            .map_err(|_| AuthorityHostFailure::new(Phase::Prepare, Reason::InvalidRequest))?;
        OwnerDatagram::validate_endpoint(&control)
            .map_err(|_| AuthorityHostFailure::new(Phase::Prepare, Reason::InvalidRequest))?;
        let parent = control
            .local_addr()
            .ok()
            .and_then(|address| {
                address
                    .as_pathname()
                    .and_then(std::path::Path::parent)
                    .map(std::path::Path::to_path_buf)
            })
            .and_then(|path| std::fs::symlink_metadata(path).ok())
            .ok_or_else(|| AuthorityHostFailure::new(Phase::Prepare, Reason::InvalidRequest))?;
        let mut fenced = false;
        for directory in [&directories.keys, &directories.journal] {
            let metadata = directory.metadata().map_err(|error| {
                AuthorityHostFailure::new(Phase::Prepare, Reason::Io(error.raw_os_error()))
            })?;
            fenced |= parent.dev() == metadata.dev() && parent.ino() == metadata.ino();
        }
        if !fenced {
            return Err(AuthorityHostFailure::new(
                Phase::Prepare,
                Reason::InvalidRequest,
            ));
        }
        let keys = directories.keys.try_clone().map_err(|error| {
            AuthorityHostFailure::new(Phase::Prepare, Reason::Io(error.raw_os_error()))
        })?;
        check_deadline(deadline, Phase::Prepare)?;
        let reaper = self.owner_child_reaper()?;
        check_deadline(deadline, Phase::Prepare)?;
        let (channel, child) = self.spawn(HelperRole::Owner)?;
        let ready = channel.receive_control(deadline).map_err(|error| {
            AuthorityHostFailure::new(Phase::ProtectedReady, Reason::Transport(error))
        })?;
        if ready != crate::local_process_owner::entry::BOOTSTRAP_READY {
            return Err(AuthorityHostFailure::new(
                Phase::ProtectedReady,
                Reason::InvalidProtectionBarrier,
            ));
        }
        check_deadline(deadline, Phase::ProtectedReady)?;
        let birth = observe_process(child.0.as_ref().expect("owned immediate child").id())
            .map_err(|_| {
                AuthorityHostFailure::new(Phase::ProtectedReady, Reason::ReferenceBindingMismatch)
            })?
            .birth;
        // The independently execed exact child has passed its protected-ready
        // barrier. Publication itself rechecks the live birth and binding.
        let reference = unsafe {
            self.publish_reference(
                directories,
                expected,
                request.job_id,
                birth,
                request.binding_digest,
                deadline,
            )
        }?;
        let channel = channel
            .send_bootstrap(&bytes, [keys.into(), control.into()], deadline)
            .map_err(|error| {
                AuthorityHostFailure::new(Phase::Transfer, Reason::Transport(error))
            })?;
        Ok(PreparedHostOwner {
            channel: Some(channel),
            child: Some(child),
            reaper,
            reference: Some(reference),
            deadline,
            #[cfg(test)]
            before_ready: None,
        })
    }
}

#[cfg(test)]
#[path = "local_process_owner_host_prepare_tests.rs"]
mod tests;
