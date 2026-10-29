use super::*;

fn fixture() -> (crate::AppState, crate::ClaimedTask, Value) {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let id = uuid::Uuid::new_v4().to_string();
    let task = crate::ClaimedTask {
        task_id: id.clone(),
        claim_attempt: 1,
        user_id: 42,
        chat_id: 7,
        user_key: None,
        channel: "ui".into(),
        external_user_id: None,
        external_chat_id: None,
        kind: "run_skill".into(),
        payload_json: "{}".into(),
    };
    state.core.db.get().unwrap().execute(
        "INSERT INTO tasks(task_id,user_id,chat_id,channel,kind,payload_json,status,created_at,updated_at,lease_owner,lease_expires_at,claim_attempt)
         VALUES(?1,42,7,'ui','run_skill','{}','running','1','1',?2,9223372036854775807,1)",
        rusqlite::params![id,state.worker.worker_id],
    ).unwrap();
    let job = format!("local_process:{}", uuid::Uuid::new_v4());
    let extra = json!({"pending_async_job":{"job_id":job,"provider":"local_process",
        "status":"accepted","poll_after_seconds":1,"expires_at":crate::now_ts_u64()+900,
        "cancel_ref":"local_process:/tmp/prepared-fixture","message_key":"clawd.task.async_job_pending"}});
    (state, task, extra)
}

#[test]
fn prepared_checkpoint_is_poll_only_and_never_claims_an_executed_effect() {
    let (state, task, extra) = fixture();
    persist(&state, &task, Some(&extra)).unwrap();
    let result = crate::repo::get_task_delivery_record(&state, &task.task_id)
        .unwrap()
        .unwrap()
        .result_json
        .unwrap();
    let checkpoint = crate::task_lifecycle::task_checkpoint_from_result_json(&result).unwrap();
    assert_eq!(checkpoint.resume_entrypoint, ResumeEntrypoint::PollAsyncJob);
    assert!(checkpoint.completed_side_effect_refs.is_empty());
    assert!(checkpoint.last_successful_step.is_none());
    assert_eq!(checkpoint.observations[0]["execution_observed"], false);
    assert!(checkpoint.pending_action.is_none());
    assert_eq!(result["task_lifecycle"]["async_start_phase"], "prepared");
    assert!(crate::repo::is_task_claim_active(&state, &task.task_id, 1).unwrap());
}

#[test]
fn stale_or_missing_start_claim_cannot_publish_a_checkpoint() {
    let (state, mut task, extra) = fixture();
    task.claim_attempt += 1;
    assert!(persist(&state, &task, Some(&extra)).is_err());
    let result = crate::repo::get_task_delivery_record(&state, &task.task_id)
        .unwrap()
        .unwrap()
        .result_json;
    assert!(result.is_none());
    task.task_id = uuid::Uuid::new_v4().to_string();
    assert!(persist(&state, &task, Some(&extra)).is_err());
}

#[tokio::test]
async fn planner_owned_tasks_do_not_replace_their_continuation() {
    let (state, mut task, extra) = fixture();
    task.kind = "ask".into();
    persist_prepared_direct_start(&state, &task, Some(extra))
        .await
        .unwrap();
    assert!(crate::repo::get_task_delivery_record(&state, &task.task_id)
        .unwrap()
        .unwrap()
        .result_json
        .is_none());
}
