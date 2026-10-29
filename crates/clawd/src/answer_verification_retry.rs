use crate::answer_verifier::AnswerVerificationRecord;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const INVALID_RESPONSE_LIMIT: u32 = 3;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RetryState {
    schema_version: u8,
    candidate_sha256: String,
    invalid_response_count: u32,
}

pub(super) struct RetryPlan {
    pub(super) state: RetryState,
    pub(super) retry_after_seconds: u64,
    pub(super) needs_user: bool,
    pub(super) reason_code: String,
}

pub(super) fn plan(
    record: &AnswerVerificationRecord,
    previous: Option<&Value>,
    provider_retry_after: Option<u64>,
) -> RetryPlan {
    let mut state = RetryState {
        schema_version: 1,
        candidate_sha256: record.candidate_sha256.clone(),
        invalid_response_count: 0,
    };
    if let Some(previous) = previous {
        let prior = serde_json::from_value::<RetryState>(previous.clone());
        let Ok(prior) = prior else {
            return invalid_state(state);
        };
        if prior.schema_version != 1
            || prior.invalid_response_count > INVALID_RESPONSE_LIMIT
            || prior.candidate_sha256.len() != 64
            || !prior
                .candidate_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return invalid_state(state);
        }
        if prior.candidate_sha256 == state.candidate_sha256 {
            state = prior;
        }
    }
    if record.reason_code == "answer_verifier_response_invalid" {
        state.invalid_response_count = state
            .invalid_response_count
            .saturating_add(1)
            .min(INVALID_RESPONSE_LIMIT);
    }
    let exhausted = state.invalid_response_count >= INVALID_RESPONSE_LIMIT;
    let short_delay = match state.invalid_response_count {
        1 => 2,
        2 => 5,
        _ => 60,
    };
    let delay = if record.reason_code == "answer_verifier_response_invalid" {
        short_delay
    } else {
        60
    };
    RetryPlan {
        state,
        retry_after_seconds: provider_retry_after.unwrap_or(delay).max(1),
        needs_user: !record.retryable || exhausted,
        reason_code: if exhausted {
            "answer_verifier_response_retry_exhausted".to_string()
        } else {
            record.reason_code.clone()
        },
    }
}

fn invalid_state(state: RetryState) -> RetryPlan {
    RetryPlan {
        state,
        retry_after_seconds: 60,
        needs_user: true,
        reason_code: "verification_retry_state_invalid".to_string(),
    }
}

#[cfg(test)]
#[path = "answer_verification_retry_tests.rs"]
mod tests;
