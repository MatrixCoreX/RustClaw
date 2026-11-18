use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::exit::ChildExit;
use super::instance::{encode_birth_record, ProcessBirthIdentity};
use super::owner_scope::{ScopeFailure, ScopeSnapshot};

pub(super) const MAX_CONTROL_BYTES: usize = 4096;

// This credential crosses only the private owner handoff/control channel.
// Deliberately omit Debug and never include it in public status or journals.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OwnerCredential {
    pub(crate) job_id: Uuid,
    pub(crate) owner: ProcessBirthIdentity,
    pub(crate) nonce: Uuid,
}

impl OwnerCredential {
    pub(crate) fn validate(&self) -> Result<(), OwnerFailure> {
        if self.job_id.is_nil()
            || self.nonce.get_version_num() != 4
            || self.nonce.get_variant() != uuid::Variant::RFC4122
            || encode_birth_record(&self.owner).is_err()
        {
            Err(OwnerFailure::InvalidCredential)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OwnerAction {
    Inspect,
    Cancel,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OwnerRequest {
    pub(crate) schema_version: u32,
    pub(crate) credential: OwnerCredential,
    pub(crate) request_id: u64,
    pub(crate) action: OwnerAction,
}

pub(crate) fn decode_request(bytes: &[u8]) -> Result<OwnerRequest, OwnerFailure> {
    if bytes.is_empty() || bytes.len() > MAX_CONTROL_BYTES {
        return Err(OwnerFailure::InvalidFrame);
    }
    let request: OwnerRequest =
        serde_json::from_slice(bytes).map_err(|_| OwnerFailure::InvalidFrame)?;
    if request.schema_version != 1 {
        return Err(OwnerFailure::UnsupportedSchema);
    }
    request.credential.validate()?;
    if request.request_id == 0 {
        return Err(OwnerFailure::SequenceGap);
    }
    Ok(request)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OwnerPhase {
    Running,
    Cancelling,
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OwnerCompletion {
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OwnerSnapshot {
    pub(crate) phase: OwnerPhase,
    pub(crate) completion: Option<OwnerCompletion>,
    pub(crate) root_exit: Option<ChildExit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OwnerFailure {
    InvalidFrame,
    UnsupportedSchema,
    InvalidCredential,
    CredentialMismatch,
    ReplayConflict,
    SequenceGap,
    SequenceExhausted,
    InvalidState,
    OwnerContextInUse,
    InvalidSignalDisposition,
    InvalidDescriptor,
    LimitExceeded,
    SpawnFailed,
    OwnershipLost,
    ProbeUnavailable,
    SignalFailed,
    Unsupported,
    TransportUnavailable,
}

impl OwnerSnapshot {
    pub(crate) fn validate(&self) -> Result<(), OwnerFailure> {
        if (self.phase == OwnerPhase::Terminal)
            != (self.completion.is_some() && self.root_exit.is_some())
            || (self.phase != OwnerPhase::Terminal && self.completion.is_some())
        {
            return Err(OwnerFailure::InvalidState);
        }
        if let Some(exit) = self.root_exit {
            match (exit.code, exit.signal) {
                (Some(code), None) if (0..=255).contains(&code) => {}
                (None, Some(signal)) if signal > 0 => {}
                _ => return Err(OwnerFailure::InvalidState),
            }
        }
        Ok(())
    }
}

impl From<ScopeFailure> for OwnerFailure {
    fn from(error: ScopeFailure) -> Self {
        match error {
            ScopeFailure::Unsupported => Self::Unsupported,
            ScopeFailure::OwnerContextInUse => Self::OwnerContextInUse,
            ScopeFailure::InvalidSignalDisposition => Self::InvalidSignalDisposition,
            ScopeFailure::InvalidDescriptor => Self::InvalidDescriptor,
            ScopeFailure::SpawnFailed => Self::SpawnFailed,
            ScopeFailure::ProbeUnavailable => Self::ProbeUnavailable,
            ScopeFailure::OwnershipLost => Self::OwnershipLost,
            ScopeFailure::SignalFailed => Self::SignalFailed,
            ScopeFailure::LimitExceeded => Self::LimitExceeded,
        }
    }
}

impl TryFrom<ScopeSnapshot> for OwnerSnapshot {
    type Error = OwnerFailure;

    fn try_from(scope: ScopeSnapshot) -> Result<Self, Self::Error> {
        let snapshot = Self {
            phase: if scope.complete {
                OwnerPhase::Terminal
            } else if scope.cancel_requested {
                OwnerPhase::Cancelling
            } else {
                OwnerPhase::Running
            },
            completion: scope.complete.then_some(if scope.cancel_requested {
                OwnerCompletion::Cancelled
            } else {
                OwnerCompletion::Completed
            }),
            root_exit: scope.root_exit,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum OwnerOutcome {
    Ok { snapshot: OwnerSnapshot },
    Error { error_code: OwnerFailure },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OwnerReply {
    pub(crate) schema_version: u32,
    pub(crate) request_id: u64,
    pub(crate) outcome: OwnerOutcome,
}

pub(crate) fn encode_reply(reply: &OwnerReply) -> Result<Vec<u8>, OwnerFailure> {
    validate_reply(reply)?;
    let bytes = serde_json::to_vec(reply).map_err(|_| OwnerFailure::InvalidFrame)?;
    if bytes.len() > MAX_CONTROL_BYTES {
        Err(OwnerFailure::InvalidFrame)
    } else {
        Ok(bytes)
    }
}

#[cfg(test)]
pub(crate) fn decode_reply(bytes: &[u8]) -> Result<OwnerReply, OwnerFailure> {
    if bytes.is_empty() || bytes.len() > MAX_CONTROL_BYTES {
        return Err(OwnerFailure::InvalidFrame);
    }
    let reply: OwnerReply =
        serde_json::from_slice(bytes).map_err(|_| OwnerFailure::InvalidFrame)?;
    validate_reply(&reply)?;
    Ok(reply)
}

fn validate_reply(reply: &OwnerReply) -> Result<(), OwnerFailure> {
    if reply.schema_version != 1 {
        return Err(OwnerFailure::UnsupportedSchema);
    }
    if reply.request_id == 0 {
        return Err(OwnerFailure::InvalidFrame);
    }
    if let OwnerOutcome::Ok { snapshot } = reply.outcome {
        snapshot.validate()?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "local_process_owner_protocol_tests.rs"]
mod tests;
