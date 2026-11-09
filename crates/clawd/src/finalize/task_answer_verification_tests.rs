use super::*;
use serde_json::json;

#[test]
fn verification_checkpoint_conversion_does_not_override_pause_or_other_waits() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        claim_attempt: 1,
        task_id: "verification-checkpoint-controls".into(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "ui".into(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".into(),
        payload_json: "{}".into(),
    };
    let provider_wait = claw_core::provider_failure_policy::PROVIDER_WAIT_RESUME_REASON;
    for (phase, reason) in [
        ("paused", provider_wait),
        ("needs_user", provider_wait),
        ("background", "pending_async_job"),
        ("waiting", "pending_async_job"),
        ("waiting", "operator_pause"),
    ] {
        let mut journal = TaskJournal::for_task(&task.task_id, "ask", "fixture");
        journal.record_answer_verification(
            crate::answer_verifier::AnswerVerificationOutcome::Unavailable {
                error_code: "answer_verifier_provider_unavailable",
                retryable: true,
            },
            "candidate",
        );
        let original = json!({"state":phase,"resume_reason":reason,"checkpoint_id":"owned"});
        journal.task_lifecycle = Some(original.clone());
        assert!(!prepare_verification_checkpoint_for_model_wait(
            &state,
            &task,
            &mut journal
        ));
        assert_eq!(journal.task_lifecycle, Some(original));
        assert!(journal.task_checkpoint.is_none());
        assert!(!journal.answer_verification_checkpoint_prepared);
    }
}
