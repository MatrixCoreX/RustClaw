//! Candidate-bound retry state is independent from answer rendering.
use super::*;
use crate::answer_verifier::AnswerVerificationOutcome;
use serde_json::json;

fn unavailable(code: &'static str, retryable: bool, candidate: &str) -> AnswerVerificationRecord {
    AnswerVerificationOutcome::Unavailable {
        error_code: code,
        retryable,
    }
    .record(candidate)
}

#[test]
fn invalid_responses_allow_only_two_short_retries_across_serialized_state() {
    let record = unavailable("answer_verifier_response_invalid", true, "candidate");
    let first = plan(&record, None, None);
    assert!(!first.needs_user);
    assert_eq!(first.retry_after_seconds, 2);
    let persisted: Value =
        serde_json::from_slice(&serde_json::to_vec(&first.state).unwrap()).unwrap();
    let second = plan(&record, Some(&persisted), None);
    assert!(!second.needs_user);
    assert_eq!(second.retry_after_seconds, 5);
    let third = plan(&record, Some(&json!(second.state)), None);
    assert!(third.needs_user);
    assert_eq!(
        third.reason_code,
        "answer_verifier_response_retry_exhausted"
    );
    assert_eq!(json!(third.state)["invalid_response_count"], 3);
}

#[test]
fn provider_cooldown_overrides_short_format_retries() {
    let record = unavailable("answer_verifier_response_invalid", true, "candidate");
    let retry = plan(&record, None, Some(900));
    assert_eq!(retry.retry_after_seconds, 900);
    assert!(!retry.needs_user);
}

#[test]
fn provider_outage_preserves_count_without_short_retries() {
    let invalid = unavailable("answer_verifier_response_invalid", true, "candidate");
    let first = plan(&invalid, None, None);
    let provider = unavailable("answer_verifier_provider_unavailable", true, "candidate");
    let outage = plan(&provider, Some(&json!(first.state)), Some(120));
    assert_eq!(outage.retry_after_seconds, 120);
    assert_eq!(json!(outage.state)["invalid_response_count"], 1);
    let retry = plan(&invalid, Some(&json!(outage.state)), None);
    assert_eq!(json!(retry.state)["invalid_response_count"], 2);
}

#[test]
fn unknown_or_provider_failures_keep_the_existing_default_wait() {
    for code in [
        "answer_verifier_provider_unavailable",
        "fixture_unknown_failure",
    ] {
        let retry = plan(&unavailable(code, true, "candidate"), None, None);
        assert_eq!(retry.retry_after_seconds, 60);
        assert_eq!(json!(retry.state)["invalid_response_count"], 0);
        assert!(!retry.needs_user);
    }
}

#[test]
fn a_new_candidate_does_not_inherit_another_candidates_format_failures() {
    let record = unavailable("answer_verifier_response_invalid", true, "old");
    let mut previous = json!(plan(&record, None, None).state);
    previous["invalid_response_count"] = json!(3);
    let retry = plan(
        &unavailable("answer_verifier_response_invalid", true, "new"),
        Some(&previous),
        None,
    );
    assert_eq!(retry.retry_after_seconds, 2);
    assert!(!retry.needs_user);
    assert_eq!(json!(retry.state)["invalid_response_count"], 1);
}

#[test]
fn malformed_or_future_metadata_never_silently_resets_automatic_retries() {
    let record = unavailable("answer_verifier_response_invalid", true, "candidate");
    let seed = json!(plan(&record, None, None).state);
    let mut invalid = vec![Value::Null, json!({}), json!("invalid")];
    for (field, value) in [
        ("schema_version", json!(2)),
        ("invalid_response_count", json!(4)),
        ("invalid_response_count", json!(-1)),
        ("invalid_response_count", json!(true)),
        ("candidate_sha256", json!("invalid")),
        ("unexpected", json!(1)),
    ] {
        let mut value_with_bad_field = seed.clone();
        value_with_bad_field[field] = value;
        invalid.push(value_with_bad_field);
    }
    for previous in invalid {
        let retry = plan(&record, Some(&previous), None);
        assert!(retry.needs_user);
        assert_eq!(retry.reason_code, "verification_retry_state_invalid");
    }
}

#[test]
fn configuration_failure_never_becomes_an_automatic_retry() {
    let retry = plan(
        &unavailable("answer_verifier_prompt_unavailable", false, "candidate"),
        None,
        Some(1),
    );
    assert!(retry.needs_user);
    assert_eq!(retry.reason_code, "answer_verifier_prompt_unavailable");
}

#[test]
fn retry_metadata_contains_hashes_not_the_candidate_content() {
    let record = unavailable(
        "answer_verifier_response_invalid",
        true,
        "private candidate content",
    );
    let encoded = json!(plan(&record, None, None).state);
    assert!(!encoded.to_string().contains("private candidate content"));
    assert_eq!(encoded["candidate_sha256"].as_str().unwrap().len(), 64);
}
