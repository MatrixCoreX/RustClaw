use super::*;
#[test]
fn deterministic_failures_do_not_consume_five_attempts() {
    let failure = JobFailure::from_error(&permanent("memory_job_provider_unavailable"));
    assert_eq!(failure.error_code, "memory_job_provider_unavailable");
    assert!(!failure.retryable);
    let unknown =
        JobFailure::from_error(&anyhow::anyhow!("secret prompt and key must never persist"));
    assert!(!serde_json::to_string(&unknown).unwrap().contains("secret"));
}
#[tokio::test]
async fn provider_classification_is_invocation_local_and_quota_is_terminal() {
    use crate::runtime::provider_diagnostics as diagnostics;
    use claw_core::provider_failure_policy::ProviderFailureClass as Class;
    let (failure, _) = diagnostics::capture(async {
        diagnostics::record(Class::QuotaExhausted, true);
        JobFailure::provider("extract_provider")
    })
    .await;
    assert_eq!(
        failure.provider_error_kind.as_deref(),
        Some("quota_exhausted")
    );
    assert!(!failure.retryable);
    assert!(diagnostics::current().is_none());
    let (failure, _) = diagnostics::capture(async {
        diagnostics::record(Class::Timeout, true);
        JobFailure::provider("consolidate_provider")
    })
    .await;
    assert!(failure.retryable);
    assert_eq!(failure.error_code, "memory_job_provider_timeout");
}
#[test]
fn sqlite_busy_and_full_are_distinguished_without_reading_message_text() {
    for (code, expected, retryable) in [
        (rusqlite::ffi::SQLITE_BUSY, "memory_job_database_busy", true),
        (rusqlite::ffi::SQLITE_FULL, "memory_job_storage_full", false),
    ] {
        let error: anyhow::Error =
            rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None).into();
        let failure = JobFailure::from_error(&error);
        assert_eq!(failure.error_code, expected);
        assert_eq!(failure.retryable, retryable);
    }
}
