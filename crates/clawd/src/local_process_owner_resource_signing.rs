use super::super::owner_resources::OwnerResourceRecord;
use super::*;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedResources {
    record: OwnerResourceRecord,
    authenticator: [u8; 32],
}

impl OwnerAuthorityKey {
    pub(in crate::local_process_owner) fn sign_resources(
        &self,
        record: OwnerResourceRecord,
    ) -> Result<Vec<u8>, OwnerReferenceFailure> {
        record.validate()?;
        let authenticator = self
            .authenticate_serialized(b"owner-resources-v1\0", &record)?
            .finalize()
            .into_bytes()
            .into();
        let bytes = serde_json::to_vec(&SignedResources {
            record,
            authenticator,
        })
        .map_err(|_| OwnerReferenceFailure::InvalidReference)?;
        if bytes.len() > MAX_REFERENCE_BYTES {
            return Err(OwnerReferenceFailure::InvalidReference);
        }
        Ok(bytes)
    }

    pub(in crate::local_process_owner) fn verify_resources(
        &self,
        bytes: &[u8],
    ) -> Result<OwnerResourceRecord, OwnerReferenceFailure> {
        if bytes.is_empty() || bytes.len() > MAX_REFERENCE_BYTES {
            return Err(OwnerReferenceFailure::InvalidReference);
        }
        let signed: SignedResources =
            serde_json::from_slice(bytes).map_err(|_| OwnerReferenceFailure::InvalidReference)?;
        signed.record.validate()?;
        self.authenticate_serialized(b"owner-resources-v1\0", &signed.record)?
            .verify_slice(&signed.authenticator)
            .map_err(|_| OwnerReferenceFailure::AuthenticationFailed)?;
        Ok(signed.record)
    }
}
