use serde::{Deserialize, Serialize};

use super::owner_protocol::{OwnerCredential, OwnerFailure, MAX_CONTROL_BYTES};
use super::owner_reference::OwnerReferenceFailure;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StartupRejection {
    MissingReference {},
    Reference { error_code: OwnerReferenceFailure },
    StartupBudgetExpired {},
}

// AttemptFailed does not assert that no payload effect occurred. Only a
// rejection before commit invocation carries that narrower phase evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StartupAck {
    Started {},
    RejectedBeforeSpawn { reason: StartupRejection },
    AttemptFailed { error_code: OwnerFailure },
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum StartupMessage {
    Ready {},
    Commit {},
    Acknowledgment { result: StartupAck },
}

// Private only: no Debug, no public status/journal projection, no prose fields.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StartupFrame {
    pub(super) schema_version: u32,
    pub(super) credential: OwnerCredential,
    pub(super) binding_digest: [u8; 32],
    pub(super) message: StartupMessage,
}

pub(super) fn encode(
    credential: &OwnerCredential,
    binding_digest: [u8; 32],
    message: StartupMessage,
) -> Result<Vec<u8>, OwnerFailure> {
    credential.validate()?;
    if binding_digest == [0; 32] {
        return Err(OwnerFailure::InvalidState);
    }
    let bytes = serde_json::to_vec(&StartupFrame {
        schema_version: 1,
        credential: credential.clone(),
        binding_digest,
        message,
    })
    .map_err(|_| OwnerFailure::InvalidFrame)?;
    if bytes.len() > MAX_CONTROL_BYTES {
        return Err(OwnerFailure::LimitExceeded);
    }
    Ok(bytes)
}

pub(super) fn decode(
    bytes: &[u8],
    expected: &OwnerCredential,
    binding_digest: &[u8; 32],
) -> Result<StartupMessage, OwnerFailure> {
    expected.validate()?;
    if bytes.is_empty() || bytes.len() > MAX_CONTROL_BYTES {
        return Err(OwnerFailure::InvalidFrame);
    }
    let frame: StartupFrame =
        serde_json::from_slice(bytes).map_err(|_| OwnerFailure::InvalidFrame)?;
    if frame.schema_version != 1 {
        return Err(OwnerFailure::UnsupportedSchema);
    }
    frame.credential.validate()?;
    if &frame.credential != expected || &frame.binding_digest != binding_digest {
        return Err(OwnerFailure::CredentialMismatch);
    }
    if frame.binding_digest == [0; 32] {
        return Err(OwnerFailure::InvalidState);
    }
    Ok(frame.message)
}

#[cfg(test)]
#[path = "local_process_startup_protocol_tests.rs"]
mod tests;
