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
    super::super::finalize_ask_result(
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
