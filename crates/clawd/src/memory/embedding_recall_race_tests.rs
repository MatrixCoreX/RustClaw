use super::*;

#[path = "embedding_recall_race_fixtures.rs"]
mod support;

#[tokio::test]
async fn recall_suppression_after_request_cannot_reactivate_a_late_vector() {
    support::rejected(support::Change::Suppress).await;
}

#[tokio::test]
async fn canonical_deletion_after_request_cannot_create_an_active_orphan_vector() {
    support::rejected(support::Change::Delete).await;
}

#[tokio::test]
async fn canonical_correction_after_request_cannot_commit_an_obsolete_vector() {
    support::rejected(support::Change::Correct).await;
}

#[tokio::test]
async fn remote_consent_revocation_after_request_cannot_commit_a_late_vector() {
    support::rejected(support::Change::RevokeConsent).await;
}

#[tokio::test]
async fn replacing_a_worker_lease_after_request_prevents_the_old_vector_commit() {
    support::rejected(support::Change::ReplaceLease).await;
}

#[tokio::test]
async fn projection_digest_change_after_request_prevents_a_stale_vector_commit() {
    support::rejected(support::Change::ChangeProjection).await;
}

#[tokio::test]
async fn unchanged_inflight_source_and_lease_still_commit_once() {
    support::control(false).await;
}

#[tokio::test]
async fn another_owners_suppression_does_not_discard_the_current_response() {
    support::control(true).await;
}
