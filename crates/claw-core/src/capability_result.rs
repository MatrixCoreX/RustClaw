use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use thiserror::Error;

pub const CAPABILITY_RESULT_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityResultStatus {
    Ok,
    Error,
    Waiting,
    NeedsUser,
}

impl CapabilityResultStatus {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Waiting => "waiting",
            Self::NeedsUser => "needs_user",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDeliveryIntent {
    #[default]
    ModelSynthesis,
    ExactMachine,
    Artifact,
    Silent,
}

impl CapabilityDeliveryIntent {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::ModelSynthesis => "model_synthesis",
            Self::ExactMachine => "exact_machine",
            Self::Artifact => "artifact",
            Self::Silent => "silent",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityDelivery {
    #[serde(default)]
    pub intent: CapabilityDeliveryIntent,
    #[serde(default)]
    pub constraints: JsonValue,
}

impl Default for CapabilityDelivery {
    fn default() -> Self {
        Self {
            intent: CapabilityDeliveryIntent::ModelSynthesis,
            constraints: JsonValue::Object(JsonMap::new()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub id: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    #[serde(default)]
    pub metadata: JsonValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactRef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default)]
    pub metadata: JsonValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuationKind {
    Poll,
    Checkpoint,
    AwaitUser,
}

impl ContinuationKind {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Poll => "poll",
            Self::Checkpoint => "checkpoint",
            Self::AwaitUser => "await_user",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Continuation {
    pub kind: ContinuationKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poll_after_ms: Option<u64>,
    #[serde(default)]
    pub state: JsonValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructuredError {
    pub code: String,
    pub message_key: String,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default)]
    pub details: JsonValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryDirective {
    #[serde(default)]
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityResultEnvelope {
    pub schema_version: u16,
    pub status: CapabilityResultStatus,
    pub capability: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default)]
    pub data: JsonValue,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<JsonValue>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default = "empty_object")]
    pub provenance: JsonValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryDirective>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
    #[serde(default = "empty_object")]
    pub verification: JsonValue,
    #[serde(default)]
    pub evidence: Vec<EvidenceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<StructuredError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation: Option<Continuation>,
    #[serde(default)]
    pub delivery: CapabilityDelivery,
}

impl CapabilityResultEnvelope {
    pub fn ok(capability: impl Into<String>, action: Option<String>, data: JsonValue) -> Self {
        Self {
            schema_version: CAPABILITY_RESULT_SCHEMA_VERSION,
            status: CapabilityResultStatus::Ok,
            capability: capability.into(),
            action,
            data,
            artifacts: Vec::new(),
            page: None,
            truncated: false,
            provenance: empty_object(),
            retry: None,
            effect: None,
            verification: empty_object(),
            evidence: Vec::new(),
            error: None,
            continuation: None,
            delivery: CapabilityDelivery::default(),
        }
    }

    pub fn failed(
        capability: impl Into<String>,
        action: Option<String>,
        error: StructuredError,
    ) -> Self {
        Self {
            schema_version: CAPABILITY_RESULT_SCHEMA_VERSION,
            status: CapabilityResultStatus::Error,
            capability: capability.into(),
            action,
            data: JsonValue::Object(JsonMap::new()),
            artifacts: Vec::new(),
            page: None,
            truncated: false,
            provenance: empty_object(),
            retry: Some(RetryDirective {
                retryable: error.retryable,
                class: None,
                after_ms: None,
            }),
            effect: None,
            verification: empty_object(),
            evidence: Vec::new(),
            error: Some(error),
            continuation: None,
            delivery: CapabilityDelivery::default(),
        }
    }

    pub fn validate(&self) -> Result<(), CapabilityResultValidationError> {
        if self.schema_version != CAPABILITY_RESULT_SCHEMA_VERSION {
            return Err(CapabilityResultValidationError::UnsupportedSchemaVersion(
                self.schema_version,
            ));
        }
        if !is_machine_ref(&self.capability) {
            return Err(CapabilityResultValidationError::InvalidCapability);
        }
        if self
            .action
            .as_deref()
            .is_some_and(|action| !is_machine_ref(action))
        {
            return Err(CapabilityResultValidationError::InvalidAction);
        }
        match self.status {
            CapabilityResultStatus::Ok if self.error.is_some() => {
                return Err(CapabilityResultValidationError::UnexpectedError)
            }
            CapabilityResultStatus::Error if self.error.is_none() => {
                return Err(CapabilityResultValidationError::MissingError)
            }
            CapabilityResultStatus::Waiting | CapabilityResultStatus::NeedsUser
                if self.continuation.is_none() =>
            {
                return Err(CapabilityResultValidationError::MissingContinuation)
            }
            _ => {}
        }
        if let Some(error) = self.error.as_ref() {
            if !is_machine_ref(&error.code) {
                return Err(CapabilityResultValidationError::InvalidErrorCode);
            }
            if !is_message_key(&error.message_key) {
                return Err(CapabilityResultValidationError::InvalidMessageKey);
            }
        }
        let mut evidence_ids = HashSet::new();
        for evidence in &self.evidence {
            if !is_machine_ref(&evidence.id) || !is_machine_ref(&evidence.source) {
                return Err(CapabilityResultValidationError::InvalidEvidenceRef);
            }
            if !evidence_ids.insert(evidence.id.as_str()) {
                return Err(CapabilityResultValidationError::DuplicateEvidenceRef);
            }
        }
        for artifact in &self.artifacts {
            if artifact.id.as_deref().is_none_or(|id| id.trim().is_empty())
                && artifact
                    .path
                    .as_deref()
                    .is_none_or(|path| path.trim().is_empty())
                && artifact
                    .uri
                    .as_deref()
                    .is_none_or(|uri| uri.trim().is_empty())
            {
                return Err(CapabilityResultValidationError::UnaddressableArtifact);
            }
        }
        if self.page.as_ref().is_some_and(|page| !page.is_object()) {
            return Err(CapabilityResultValidationError::InvalidPage);
        }
        if !self.provenance.is_object() {
            return Err(CapabilityResultValidationError::InvalidProvenance);
        }
        if self
            .retry
            .as_ref()
            .and_then(|retry| retry.class.as_deref())
            .is_some_and(|class| !is_machine_ref(class))
        {
            return Err(CapabilityResultValidationError::InvalidRetryClass);
        }
        if self
            .effect
            .as_deref()
            .is_some_and(|effect| !is_machine_ref(effect))
        {
            return Err(CapabilityResultValidationError::InvalidEffect);
        }
        if !self.verification.is_object() {
            return Err(CapabilityResultValidationError::InvalidVerification);
        }
        if !self.delivery.constraints.is_object() {
            return Err(CapabilityResultValidationError::InvalidDeliveryConstraints);
        }
        Ok(())
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityResultValidationError {
    #[error("capability_result_schema_version_unsupported:{0}")]
    UnsupportedSchemaVersion(u16),
    #[error("capability_result_capability_invalid")]
    InvalidCapability,
    #[error("capability_result_action_invalid")]
    InvalidAction,
    #[error("capability_result_error_missing")]
    MissingError,
    #[error("capability_result_error_unexpected")]
    UnexpectedError,
    #[error("capability_result_continuation_missing")]
    MissingContinuation,
    #[error("capability_result_error_code_invalid")]
    InvalidErrorCode,
    #[error("capability_result_message_key_invalid")]
    InvalidMessageKey,
    #[error("capability_result_evidence_ref_invalid")]
    InvalidEvidenceRef,
    #[error("capability_result_evidence_ref_duplicate")]
    DuplicateEvidenceRef,
    #[error("capability_result_artifact_unaddressable")]
    UnaddressableArtifact,
    #[error("capability_result_page_invalid")]
    InvalidPage,
    #[error("capability_result_provenance_invalid")]
    InvalidProvenance,
    #[error("capability_result_retry_class_invalid")]
    InvalidRetryClass,
    #[error("capability_result_effect_invalid")]
    InvalidEffect,
    #[error("capability_result_verification_invalid")]
    InvalidVerification,
    #[error("capability_result_delivery_constraints_invalid")]
    InvalidDeliveryConstraints,
}

fn empty_object() -> JsonValue {
    JsonValue::Object(JsonMap::new())
}

pub fn is_machine_ref(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 160
        && value.chars().all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '-' | '.' | ':')
        })
}

fn is_message_key(value: &str) -> bool {
    is_machine_ref(value) && !value.starts_with(':') && !value.ends_with(':')
}

#[cfg(test)]
#[path = "capability_result_tests.rs"]
mod tests;
