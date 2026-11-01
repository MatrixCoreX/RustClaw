use super::*;
use crate::package_custody::{PackageCustodyBinding, PackageVersionCustody};
use sha2::{Digest, Sha256};

const MAGIC: &[u8; 4] = b"OPL2";

// Private, exact-job metadata only. A lease is retained by the owner, never
// part of payload IO, permission grants or evidence of scope completion.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageLeaseFrame {
    schema_version: u32,
    credential: OwnerCredential,
    has_lease: bool,
}

pub(crate) fn package_custody_binding(
    credential: &OwnerCredential,
    execution_binding_digest: [u8; 32],
) -> Result<PackageCustodyBinding, LaunchChannelFailure> {
    credential
        .validate()
        .map_err(|_| LaunchChannelFailure::InvalidCredential)?;
    let bytes =
        serde_json::to_vec(credential).map_err(|_| LaunchChannelFailure::InvalidCredential)?;
    let binding = PackageCustodyBinding {
        job_id: credential.job_id,
        execution_binding_digest,
        owner_reference_digest: Sha256::digest(bytes).into(),
    };
    binding
        .validate()
        .map_err(|_| LaunchChannelFailure::InvalidCredential)?;
    Ok(binding)
}

impl LaunchChannel {
    pub(crate) fn send_package_lease(
        self,
        lease: Option<PackageVersionCustody>,
        credential: &OwnerCredential,
        deadline: Instant,
    ) -> Result<Self, LaunchChannelFailure> {
        credential
            .validate()
            .map_err(|_| LaunchChannelFailure::InvalidCredential)?;
        let handles = lease
            .map(|lease| {
                let actual = lease.binding();
                let expected =
                    package_custody_binding(credential, actual.execution_binding_digest)?;
                if actual != expected {
                    return Err(LaunchChannelFailure::CredentialMismatch);
                }
                lease
                    .into_process_handles()
                    .map_err(|_| LaunchChannelFailure::InvalidDescriptor)
            })
            .transpose()?;
        let body = serde_json::to_vec(&PackageLeaseFrame {
            schema_version: 2,
            credential: credential.clone(),
            has_lease: handles.is_some(),
        })
        .map_err(|_| LaunchChannelFailure::InvalidFrame)?;
        if body.len() > MAX_CONTROL_BYTES {
            return Err(LaunchChannelFailure::LimitExceeded);
        }
        let mut header = [0; 8];
        header[..4].copy_from_slice(MAGIC);
        header[4..].copy_from_slice(&(body.len() as u32).to_be_bytes());
        let handles = handles
            .iter()
            .flatten()
            .map(AsFd::as_fd)
            .collect::<Vec<_>>();
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2))];
        let mut ancillary = SendAncillaryBuffer::new(&mut space);
        if !handles.is_empty() && !ancillary.push(SendAncillaryMessage::ScmRights(&handles)) {
            return Err(LaunchChannelFailure::InvalidDescriptor);
        }
        let sent = self.send_piece(&header, &mut ancillary, deadline)?;
        self.send_bytes(&header[sent..], deadline)?;
        self.send_bytes(&body, deadline)?;
        Ok(self)
    }

    pub(crate) fn receive_package_lease(
        self,
        expected: &OwnerCredential,
        execution_binding_digest: [u8; 32],
        deadline: Instant,
    ) -> Result<(Option<PackageVersionCustody>, Self), LaunchChannelFailure> {
        expected
            .validate()
            .map_err(|_| LaunchChannelFailure::InvalidCredential)?;
        let mut header = [0; 8];
        let mut handles = Vec::new();
        let count = self.receive_piece(&mut header, Some(&mut handles), deadline)?;
        self.receive_bytes(&mut header[count..], deadline)?;
        if &header[..4] != MAGIC {
            return Err(LaunchChannelFailure::InvalidFrame);
        }
        let length = u32::from_be_bytes(header[4..].try_into().unwrap()) as usize;
        if length == 0 || length > MAX_CONTROL_BYTES {
            return Err(LaunchChannelFailure::LimitExceeded);
        }
        let mut body = vec![0; length];
        self.receive_bytes(&mut body, deadline)?;
        let frame: PackageLeaseFrame =
            serde_json::from_slice(&body).map_err(|_| LaunchChannelFailure::InvalidFrame)?;
        if frame.schema_version != 2 {
            return Err(LaunchChannelFailure::UnsupportedSchema);
        }
        frame
            .credential
            .validate()
            .map_err(|_| LaunchChannelFailure::InvalidCredential)?;
        if &frame.credential != expected {
            return Err(LaunchChannelFailure::CredentialMismatch);
        }
        if handles.len() != 2 * usize::from(frame.has_lease) {
            return Err(LaunchChannelFailure::InvalidDescriptor);
        }
        let lease = if frame.has_lease {
            let record = File::from(handles.pop().unwrap());
            let version = File::from(handles.pop().unwrap());
            Some(
                PackageVersionCustody::from_process_handles(
                    [version, record],
                    package_custody_binding(expected, execution_binding_digest)?,
                )
                .map_err(|_| LaunchChannelFailure::InvalidDescriptor)?,
            )
        } else {
            None
        };
        remaining(deadline)?;
        Ok((lease, self))
    }
}

#[cfg(test)]
#[path = "local_process_owner_package_lease_tests.rs"]
mod tests;
