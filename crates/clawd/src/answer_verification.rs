use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::AnswerVerifierOut;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnswerVerificationStatus {
    NotRequired,
    Passed,
    Failed,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnswerVerificationRecord {
    pub(crate) schema_version: u8,
    pub(crate) status: AnswerVerificationStatus,
    pub(crate) reason_code: String,
    pub(crate) retryable: bool,
    pub(crate) candidate_sha256: String,
}

impl AnswerVerificationRecord {
    pub(crate) fn matches(&self, candidate: &str) -> bool {
        self.candidate_sha256 == candidate_digest(candidate)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum AnswerVerificationOutcome {
    NotRequired,
    Passed(Option<AnswerVerifierOut>),
    Failed(AnswerVerifierOut),
    Unavailable {
        error_code: &'static str,
        retryable: bool,
    },
}

impl AnswerVerificationOutcome {
    pub(crate) fn from_verdict(verdict: AnswerVerifierOut) -> Self {
        if verdict.pass {
            Self::Passed(Some(verdict))
        } else {
            Self::Failed(verdict)
        }
    }

    pub(crate) fn accepts_candidate(&self) -> bool {
        matches!(self, Self::NotRequired | Self::Passed(_))
    }

    pub(crate) fn verdict(&self) -> Option<&AnswerVerifierOut> {
        match self {
            Self::Passed(Some(verdict)) | Self::Failed(verdict) => Some(verdict),
            _ => None,
        }
    }

    pub(crate) fn record(&self, candidate: &str) -> AnswerVerificationRecord {
        let (status, reason_code, retryable) = match self {
            Self::NotRequired => (AnswerVerificationStatus::NotRequired, "not_required", false),
            Self::Passed(None) => (AnswerVerificationStatus::Passed, "local_evidence", false),
            Self::Passed(Some(_)) => (AnswerVerificationStatus::Passed, "verifier_verdict", false),
            Self::Failed(_) => (AnswerVerificationStatus::Failed, "evidence_rejected", false),
            Self::Unavailable {
                error_code,
                retryable,
            } => (
                AnswerVerificationStatus::Unavailable,
                *error_code,
                *retryable,
            ),
        };
        AnswerVerificationRecord {
            schema_version: 1,
            status,
            reason_code: reason_code.to_string(),
            retryable,
            candidate_sha256: candidate_digest(candidate),
        }
    }
}

fn candidate_digest(candidate: &str) -> String {
    format!("{:x}", Sha256::digest(candidate.as_bytes()))
}

#[cfg(test)]
#[path = "answer_verification_tests.rs"]
mod tests;
