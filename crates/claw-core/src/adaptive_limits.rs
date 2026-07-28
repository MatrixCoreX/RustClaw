use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::capability_result::is_machine_ref;

pub const LIMIT_HIT_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitClass {
    ModelWindow,
    TaskResource,
    Safety,
    Protocol,
    ExternalService,
    DisplayCache,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitUnit {
    Items,
    Bytes,
    Tokens,
    Calls,
    Milliseconds,
    Depth,
    Percent,
    CostUsdNanos,
    Continuations,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitRecovery {
    None,
    OpaqueContinuation,
    ArtifactRange,
    CheckpointRequeue,
    RetryAfter,
    VerifiedShard,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitHit {
    pub schema_version: u16,
    pub class: LimitClass,
    pub owner: String,
    pub unit: LimitUnit,
    pub configured_value: u64,
    pub observed_value: u64,
    pub reason_code: String,
    pub terminal: bool,
    pub recovery: LimitRecovery,
}

impl LimitHit {
    pub fn validate(&self) -> Result<(), LimitHitValidationError> {
        if self.schema_version != LIMIT_HIT_SCHEMA_VERSION {
            return Err(LimitHitValidationError::UnsupportedSchemaVersion(
                self.schema_version,
            ));
        }
        if !is_machine_ref(&self.owner) {
            return Err(LimitHitValidationError::InvalidOwner);
        }
        if !is_machine_ref(&self.reason_code) {
            return Err(LimitHitValidationError::InvalidReasonCode);
        }
        if self.configured_value == 0 {
            return Err(LimitHitValidationError::InvalidConfiguredValue);
        }
        if !self.terminal && self.recovery == LimitRecovery::None {
            return Err(LimitHitValidationError::MissingRecovery);
        }
        Ok(())
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum LimitHitValidationError {
    #[error("limit_hit_schema_version_unsupported:{0}")]
    UnsupportedSchemaVersion(u16),
    #[error("limit_hit_owner_invalid")]
    InvalidOwner,
    #[error("limit_hit_reason_code_invalid")]
    InvalidReasonCode,
    #[error("limit_hit_configured_value_invalid")]
    InvalidConfiguredValue,
    #[error("limit_hit_recovery_missing")]
    MissingRecovery,
}

#[cfg(test)]
#[path = "adaptive_limits_tests.rs"]
mod tests;
