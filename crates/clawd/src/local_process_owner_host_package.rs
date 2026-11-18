use crate::local_process_owner::launch_channel::{package_custody_binding, LaunchChannelFailure};
use crate::local_process_owner::owner_protocol::OwnerCredential;
use crate::package_custody::PackageVersionCustody;

// Sole host custody before the first transfer attempt. Never wrap a received
// capsule or recreate this guard after entering the transport.
pub(super) struct UntransferredOwnerPackage(Option<PackageVersionCustody>);

impl UntransferredOwnerPackage {
    pub(super) fn new(
        package: Option<PackageVersionCustody>,
        credential: &OwnerCredential,
        binding: [u8; 32],
    ) -> Result<Self, LaunchChannelFailure> {
        let expected = package_custody_binding(credential, binding)?;
        if package
            .as_ref()
            .is_some_and(|value| value.binding() != expected)
        {
            return Err(LaunchChannelFailure::CredentialMismatch);
        }
        Ok(Self(package))
    }

    pub(super) fn into_transfer(mut self) -> Option<PackageVersionCustody> {
        self.0.take()
    }
}

impl Drop for UntransferredOwnerPackage {
    fn drop(&mut self) {
        if let Some(package) = self.0.as_ref() {
            if let Err(error) = package.release() {
                // A corrupt/unwritable record remains retained, not repaired.
                tracing::debug!(?error, "owner_untransferred_package_release_unavailable");
            }
        }
    }
}

#[cfg(test)]
#[path = "local_process_owner_host_package_tests.rs"]
mod tests;
