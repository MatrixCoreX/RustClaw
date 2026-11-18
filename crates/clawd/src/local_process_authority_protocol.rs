use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::super::instance::ProcessBirthIdentity;
use super::super::owner_admission::{AdmissionFailure, AdmissionStage};
use super::super::owner_authority::AuthorityFailure;
use super::super::owner_outcome::OwnerObservationRecord;
use super::super::owner_protocol::OwnerCredential;
use super::super::owner_reference::{AuthorityFingerprint, OwnerReference, OwnerReferenceFailure};
use super::super::owner_resources::OwnerResourceBudget;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AuthorityBootstrap {
    pub(super) schema_version: u32,
    pub(super) action: AuthorityAction,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum AuthorityAction {
    Initialize {},
    Resume {
        authority_fingerprint: [u8; 32],
    },
    InventoryResources {
        authority_fingerprint: [u8; 32],
        request_id: Uuid,
    },
    PublishReference {
        authority_fingerprint: [u8; 32],
        job_id: Uuid,
        owner: ProcessBirthIdentity,
        binding_digest: [u8; 32],
    },
    ReadOutcome {
        authority_fingerprint: [u8; 32],
        credential: OwnerCredential,
        binding_digest: [u8; 32],
    },
    LoadReference {
        authority_fingerprint: [u8; 32],
        job_id: Uuid,
        owner: ProcessBirthIdentity,
        binding_digest: [u8; 32],
    },
    HoldResources {
        authority_fingerprint: [u8; 32],
        credential: OwnerCredential,
        binding_digest: [u8; 32],
        budget: OwnerResourceBudget,
    },
    ReadResources {
        authority_fingerprint: [u8; 32],
        credential: OwnerCredential,
        binding_digest: [u8; 32],
    },
}

impl AuthorityBootstrap {
    pub(super) fn decode(bytes: &[u8]) -> Result<Self, AuthorityAdmissionError> {
        if bytes.is_empty() || bytes.len() > super::super::owner_protocol::MAX_CONTROL_BYTES {
            return Err(AuthorityAdmissionError::code(
                AuthorityAdmissionCode::InvalidRequest,
            ));
        }
        let request: Self = serde_json::from_slice(bytes)
            .map_err(|_| AuthorityAdmissionError::code(AuthorityAdmissionCode::InvalidRequest))?;
        let valid = match &request.action {
            AuthorityAction::Initialize {} => true,
            AuthorityAction::Resume {
                authority_fingerprint,
            } => AuthorityFingerprint::from_slice(authority_fingerprint).is_some(),
            AuthorityAction::InventoryResources {
                authority_fingerprint,
                request_id,
            } => {
                AuthorityFingerprint::from_slice(authority_fingerprint).is_some()
                    && !request_id.is_nil()
            }
            AuthorityAction::PublishReference {
                authority_fingerprint,
                job_id,
                owner,
                binding_digest,
            }
            | AuthorityAction::LoadReference {
                authority_fingerprint,
                job_id,
                owner,
                binding_digest,
            } => {
                AuthorityFingerprint::from_slice(authority_fingerprint).is_some()
                    && OwnerReference::new(*job_id, owner.clone(), *binding_digest).is_ok()
            }
            AuthorityAction::HoldResources {
                authority_fingerprint,
                credential,
                binding_digest,
                budget,
            } => {
                AuthorityFingerprint::from_slice(authority_fingerprint).is_some()
                    && credential.validate().is_ok()
                    && OwnerReference::new(
                        credential.job_id,
                        credential.owner.clone(),
                        *binding_digest,
                    )
                    .is_ok()
                    && budget.validate().is_ok()
            }
            AuthorityAction::ReadOutcome {
                authority_fingerprint,
                credential,
                binding_digest,
            }
            | AuthorityAction::ReadResources {
                authority_fingerprint,
                credential,
                binding_digest,
            } => {
                AuthorityFingerprint::from_slice(authority_fingerprint).is_some()
                    && credential.validate().is_ok()
                    && OwnerReference::new(
                        credential.job_id,
                        credential.owner.clone(),
                        *binding_digest,
                    )
                    .is_ok()
            }
        };
        if request.schema_version != 1 || !valid {
            return Err(AuthorityAdmissionError::code(
                AuthorityAdmissionCode::InvalidRequest,
            ));
        }
        Ok(request)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuthorityAdmissionCode {
    InvalidRequest,
    InvalidDirectory,
    InvalidJournal,
    UnsupportedSchema,
    MissingJournal,
    IncompleteInitialization,
    AlreadyInitialized,
    ClaimLost,
    DurabilityUnavailable,
    SequenceExhausted,
    AuthorityMismatch,
    EntropyUnavailable,
    StorageUnavailable,
    AuthorityUnavailable,
    ReferenceUnavailable,
    OwnerUnavailable,
    OwnerIdentityMismatch,
}

// Closed machine fields only. No paths, SQLite prose, keys or control nonces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthorityAdmissionError {
    pub(crate) error_code: AuthorityAdmissionCode,
    admission_stage: Option<AdmissionStage>,
    sqlite_extended_code: Option<i32>,
    os_error: Option<i32>,
    authority_failure: Option<AuthorityFailure>,
    reference_failure: Option<OwnerReferenceFailure>,
}

impl AuthorityAdmissionError {
    pub(super) fn code(error_code: AuthorityAdmissionCode) -> Self {
        Self {
            error_code,
            admission_stage: None,
            sqlite_extended_code: None,
            os_error: None,
            authority_failure: None,
            reference_failure: None,
        }
    }

    pub(super) fn io(error_code: AuthorityAdmissionCode, error: std::io::Error) -> Self {
        Self {
            os_error: error.raw_os_error(),
            ..Self::code(error_code)
        }
    }

    pub(super) fn sqlite(error: rusqlite::Error) -> Self {
        Self {
            sqlite_extended_code: error.sqlite_error().map(|error| error.extended_code),
            ..Self::code(AuthorityAdmissionCode::StorageUnavailable)
        }
    }
}

impl From<AdmissionFailure> for AuthorityAdmissionError {
    fn from(error: AdmissionFailure) -> Self {
        use AuthorityAdmissionCode as Code;
        let code = match error {
            AdmissionFailure::InvalidJournal => Code::InvalidJournal,
            AdmissionFailure::UnsupportedSchema => Code::UnsupportedSchema,
            AdmissionFailure::MissingJournal => Code::MissingJournal,
            AdmissionFailure::IncompleteInitialization => Code::IncompleteInitialization,
            AdmissionFailure::AlreadyInitialized => Code::AlreadyInitialized,
            AdmissionFailure::ClaimLost => Code::ClaimLost,
            AdmissionFailure::DurabilityUnavailable => Code::DurabilityUnavailable,
            AdmissionFailure::SequenceExhausted => Code::SequenceExhausted,
            AdmissionFailure::AuthorityMismatch => Code::AuthorityMismatch,
            AdmissionFailure::EntropyUnavailable => Code::EntropyUnavailable,
            AdmissionFailure::Sqlite { .. } => Code::StorageUnavailable,
            AdmissionFailure::Authority(_) => Code::AuthorityUnavailable,
            AdmissionFailure::Reference(_) => Code::ReferenceUnavailable,
        };
        let mut failure = Self::code(code);
        match error {
            AdmissionFailure::Sqlite {
                stage,
                sqlite_extended_code,
                ..
            } => {
                failure.admission_stage = Some(stage);
                failure.sqlite_extended_code = sqlite_extended_code;
            }
            AdmissionFailure::Authority(detail) => failure.authority_failure = Some(detail),
            AdmissionFailure::Reference(detail) => failure.reference_failure = Some(detail),
            _ => {}
        }
        failure
    }
}

impl From<AuthorityFailure> for AuthorityAdmissionError {
    fn from(error: AuthorityFailure) -> Self {
        Self {
            authority_failure: Some(error),
            ..Self::code(AuthorityAdmissionCode::InvalidDirectory)
        }
    }
}

impl From<OwnerReferenceFailure> for AuthorityAdmissionError {
    fn from(error: OwnerReferenceFailure) -> Self {
        Self {
            reference_failure: Some(error),
            ..Self::code(AuthorityAdmissionCode::ReferenceUnavailable)
        }
    }
}

// Reference credentials cross only the protected helper channel, never public
// status, logs or journals. Initialization/resume retain fingerprint-only replies.
#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum AuthorityResponse {
    Ok {
        schema_version: u32,
        authority_fingerprint: [u8; 32],
    },
    Reference {
        schema_version: u32,
        authority_fingerprint: [u8; 32],
        credential: OwnerCredential,
        binding_digest: [u8; 32],
    },
    Outcome {
        schema_version: u32,
        authority_fingerprint: [u8; 32],
        reference: OwnerReference,
        observation: OwnerObservationRecord,
    },
    Resources {
        schema_version: u32,
        authority_fingerprint: [u8; 32],
        reference: OwnerReference,
        budget: Option<OwnerResourceBudget>,
        observation: OwnerObservationRecord,
    },
    Error {
        schema_version: u32,
        error: AuthorityAdmissionError,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthorityResponseFailure {
    InvalidFrame,
    UnsupportedSchema,
    InvalidFingerprint,
    InvalidReference,
}

impl AuthorityResponse {
    pub(super) fn is_success(&self) -> bool {
        matches!(
            self,
            Self::Ok { .. }
                | Self::Reference { .. }
                | Self::Outcome { .. }
                | Self::Resources { .. }
        )
    }

    pub(super) fn decode(bytes: &[u8]) -> Result<Self, AuthorityResponseFailure> {
        if bytes.is_empty() || bytes.len() > super::super::owner_protocol::MAX_CONTROL_BYTES {
            return Err(AuthorityResponseFailure::InvalidFrame);
        }
        let response: Self =
            serde_json::from_slice(bytes).map_err(|_| AuthorityResponseFailure::InvalidFrame)?;
        let version = match &response {
            Self::Ok { schema_version, .. }
            | Self::Reference { schema_version, .. }
            | Self::Outcome { schema_version, .. }
            | Self::Resources { schema_version, .. }
            | Self::Error { schema_version, .. } => *schema_version,
        };
        if version != 1 {
            return Err(AuthorityResponseFailure::UnsupportedSchema);
        }
        if matches!(&response,
            Self::Ok { authority_fingerprint, .. } | Self::Reference { authority_fingerprint, .. }
                | Self::Outcome { authority_fingerprint, .. }
                | Self::Resources { authority_fingerprint, .. }
            if AuthorityFingerprint::from_slice(authority_fingerprint).is_none())
        {
            return Err(AuthorityResponseFailure::InvalidFingerprint);
        }
        if matches!(&response, Self::Reference { credential, binding_digest, .. }
            if credential.validate().is_err() || *binding_digest == [0; 32])
        {
            return Err(AuthorityResponseFailure::InvalidReference);
        }
        if matches!(&response, Self::Outcome { reference, observation, .. }
            if reference.validate().is_err() || observation.validate().is_err())
        {
            return Err(AuthorityResponseFailure::InvalidReference);
        }
        if matches!(&response, Self::Resources { reference, budget, observation, .. }
            if reference.validate().is_err() || budget.as_ref().is_some_and(|value| value.validate().is_err())
                || observation.validate().is_err())
        {
            return Err(AuthorityResponseFailure::InvalidReference);
        }
        Ok(response)
    }

    pub(super) fn from_result(
        result: Result<AuthorityFingerprint, AuthorityAdmissionError>,
    ) -> Self {
        match result {
            Ok(fingerprint) => Self::Ok {
                schema_version: 1,
                authority_fingerprint: *fingerprint.as_bytes(),
            },
            Err(error) => Self::Error {
                schema_version: 1,
                error,
            },
        }
    }
}

#[cfg(test)]
#[path = "local_process_authority_protocol_tests.rs"]
mod tests;
