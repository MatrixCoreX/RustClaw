use super::*;
use crate::answer_verifier::AnswerVerificationOutcome;

fn task() -> ClaimedTask {
    ClaimedTask {
        claim_attempt: 0,
        task_id: "verification-checkpoint".into(),
        user_id: 7,
        chat_id: 8,
        user_key: None,
        channel: "ui".into(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".into(),
        payload_json: "{}".into(),
    }
}

#[test]
fn unavailable_provider_preserves_loop_checkpoint_without_replayable_action() {
    let state = AppState::test_default_with_fixture_provider();
    let task = task();
    let mut loop_state = crate::agent_engine::LoopState::default();
    loop_state.round_no = 2;
    loop_state
        .successful_action_fingerprints
        .insert("opaque-completed-action".into(), 1);
    let mut journal = TaskJournal::for_task(&task.task_id, "ask", "opaque");
    journal.answer_verification_checkpoint =
        Some(crate::agent_engine::verification_checkpoint_payload(&task, &loop_state).into());
    journal.record_answer_verification(
        AnswerVerificationOutcome::Unavailable {
            error_code: "answer_verifier_provider_unavailable",
            retryable: true,
        },
        "pending-answer",
    );
    assert!(checkpoint_unavailable_verification(
        &state,
        &task,
        &mut journal
    ));
    assert_eq!(journal.task_lifecycle.as_ref().unwrap()["state"], "waiting");
    let value = journal.task_checkpoint.as_ref().unwrap();
    assert_eq!(
        value["completed_side_effect_refs"],
        json!(["opaque-completed-action"])
    );
    assert_eq!(
        value["boundary_context"]["agent_loop_resume_state"]["stage"],
        "verification"
    );
    assert!(value["pending_action"].is_null());
    assert_eq!(value["resume_entrypoint"], "next_planner_round");
    let _: crate::task_lifecycle::TaskCheckpoint = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(
        journal.to_summary_json()["answer_verification"]["status"],
        "unavailable"
    );
}

#[test]
fn missing_configuration_requires_user_without_automatic_retry() {
    let state = AppState::test_default_with_fixture_provider();
    let task = task();
    let mut journal = TaskJournal::for_task(&task.task_id, "ask", "opaque");
    journal.record_answer_verification(
        AnswerVerificationOutcome::Unavailable {
            error_code: "answer_verifier_prompt_unavailable",
            retryable: false,
        },
        "pending-answer",
    );
    assert!(checkpoint_unavailable_verification(
        &state,
        &task,
        &mut journal
    ));
    let lifecycle = journal.task_lifecycle.as_ref().unwrap();
    assert_eq!(lifecycle["state"], "needs_user");
    assert_eq!(lifecycle["manual_resume_required"], true);
    assert!(lifecycle["next_check_after"].is_null());
    let _: crate::task_lifecycle::TaskCheckpoint =
        serde_json::from_value(journal.task_checkpoint.clone().unwrap()).unwrap();
}

#[test]
fn accepted_verification_does_not_create_a_waiting_task() {
    let state = AppState::test_default_with_fixture_provider();
    let task = task();
    let mut journal = TaskJournal::new("opaque");
    journal.record_answer_verification(AnswerVerificationOutcome::Passed(None), "answer");
    assert!(!checkpoint_unavailable_verification(
        &state,
        &task,
        &mut journal
    ));
    assert!(journal.task_lifecycle.is_none());
}

#[test]
fn duplicate_finalization_counts_one_invalid_response_and_survives_merge() {
    let state = AppState::test_default_with_fixture_provider();
    let task = task();
    let mut journal = TaskJournal::for_task(&task.task_id, "ask", "fixture");
    journal.record_answer_verification(
        AnswerVerificationOutcome::Unavailable {
            error_code: "answer_verifier_response_invalid",
            retryable: true,
        },
        "candidate",
    );
    assert!(checkpoint_unavailable_verification(
        &state,
        &task,
        &mut journal
    ));
    let first = journal.task_checkpoint.clone().unwrap();
    assert_eq!(
        first["boundary_context"]["verification_retry"]["invalid_response_count"],
        1
    );
    assert!(
        journal.task_lifecycle.as_ref().unwrap()["next_check_after"]
            .as_i64()
            .unwrap()
            <= crate::now_ts_u64() as i64 + 2
    );
    assert!(checkpoint_unavailable_verification(
        &state,
        &task,
        &mut journal
    ));
    assert_eq!(journal.task_checkpoint.as_ref().unwrap(), &first);
    let mut merged = TaskJournal::for_task(&task.task_id, "ask", "fixture");
    merged.merge_from(&journal);
    assert!(checkpoint_unavailable_verification(
        &state,
        &task,
        &mut merged
    ));
    assert_eq!(merged.task_checkpoint.as_ref().unwrap(), &first);
}

#[test]
fn new_verifier_results_increment_durable_count_but_repeated_finalize_does_not() {
    let state = AppState::test_default_with_fixture_provider();
    let task = task();
    let mut previous = None;
    for count in 1..=3 {
        let mut journal = TaskJournal::for_task(&task.task_id, "ask", "fixture");
        journal.task_checkpoint = previous;
        journal.record_answer_verification(
            AnswerVerificationOutcome::Unavailable {
                error_code: "answer_verifier_response_invalid",
                retryable: true,
            },
            "candidate",
        );
        assert!(checkpoint_unavailable_verification(
            &state,
            &task,
            &mut journal
        ));
        assert_eq!(
            journal.task_checkpoint.as_ref().unwrap()["boundary_context"]["verification_retry"]
                ["invalid_response_count"],
            count
        );
        let lifecycle = journal.task_lifecycle.as_ref().unwrap();
        if count == 3 {
            assert_eq!(lifecycle["state"], "needs_user");
            assert!(lifecycle["next_check_after"].is_null());
            assert_eq!(
                lifecycle["reason_code"],
                "answer_verifier_response_retry_exhausted"
            );
        } else {
            assert_eq!(lifecycle["state"], "waiting");
        }
        assert!(journal.task_checkpoint.as_ref().unwrap()["pending_action"].is_null());
        assert!(checkpoint_unavailable_verification(
            &state,
            &task,
            &mut journal
        ));
        assert_eq!(
            journal.task_checkpoint.as_ref().unwrap()["boundary_context"]["verification_retry"]
                ["invalid_response_count"],
            count
        );
        previous = Some(
            serde_json::from_slice(&serde_json::to_vec(&journal.task_checkpoint.unwrap()).unwrap())
                .unwrap(),
        );
    }
}

#[test]
fn short_retry_never_overrides_the_provider_cooldown() {
    let state = AppState::test_default_with_fixture_provider();
    let task = task();
    state.note_task_provider_blocker(
        &task.task_id,
        crate::TaskProviderBlocker {
            provider: "fixture".into(),
            status_code: "rate_limited".into(),
            retry_after_seconds: 900,
            external_provider_blocked: true,
            message_key: "provider.rate_limited".into(),
        },
    );
    let mut journal = TaskJournal::for_task(&task.task_id, "ask", "fixture");
    journal.record_answer_verification(
        AnswerVerificationOutcome::Unavailable {
            error_code: "answer_verifier_response_invalid",
            retryable: true,
        },
        "candidate",
    );
    let before = crate::now_ts_u64() as i64;
    assert!(checkpoint_unavailable_verification(
        &state,
        &task,
        &mut journal
    ));
    let lifecycle = journal.task_lifecycle.as_ref().unwrap();
    assert!(lifecycle["next_check_after"].as_i64().unwrap() >= before + 900);
    assert_eq!(lifecycle["provider_blocker_active"], true);
    assert_eq!(lifecycle["verification_retry"]["invalid_response_count"], 1);
}

#[tokio::test]
async fn finalization_persists_waiting_instead_of_success_after_completed_effect_and_verifier_outage(
) {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = task();
    state.core.db.get().unwrap().execute(
        "INSERT INTO tasks (task_id,user_id,chat_id,channel,kind,payload_json,status,created_at,updated_at,lease_owner,claim_attempt)
         VALUES (?1,7,8,'ui','ask','{}','running','1','1',?2,0)",rusqlite::params![task.task_id,state.worker.worker_id]).unwrap();
    let mut journal = TaskJournal::for_task(&task.task_id, "ask", "fixture request");
    journal
        .capability_results
        .push(claw_core::capability_result::CapabilityResultEnvelope::ok(
            "fixture.write",
            None,
            json!({"written":true}),
        ));
    let mut loop_state = crate::agent_engine::LoopState::default();
    loop_state
        .successful_action_fingerprints
        .insert("completed-effect".into(), 1);
    journal.answer_verification_checkpoint =
        Some(crate::agent_engine::verification_checkpoint_payload(&task, &loop_state).into());
    journal.record_answer_verification(
        AnswerVerificationOutcome::Unavailable {
            error_code: "answer_verifier_provider_unavailable",
            retryable: true,
        },
        "candidate",
    );
    let reply = crate::AskReply::non_llm("candidate".into()).with_task_journal(journal);
    crate::finalize::finalize_ask_result(
        &state,
        &task,
        &json!({}),
        "fixture request",
        "fixture context",
        None,
        "fixture request",
        None,
        &[],
        None,
        Ok(reply),
    )
    .await
    .unwrap();
    let db = state.core.db.get().unwrap();
    let (status, result): (String, String) = db
        .query_row(
            "SELECT status,result_json FROM tasks WHERE task_id=?1",
            [&task.task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "running");
    let result: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(
        result["task_journal"]["summary"]["answer_verification"]["status"],
        "unavailable"
    );
    assert_eq!(
        result["task_journal"]["summary"]["execution_status"],
        "succeeded"
    );
    assert_eq!(
        result["task_journal"]["summary"]["task_lifecycle"]["state"],
        "waiting"
    );
    assert_eq!(
        result["task_journal"]["summary"]["task_checkpoint"]["completed_side_effect_refs"],
        json!(["completed-effect"])
    );
    assert_eq!(result["text"], "");
}
