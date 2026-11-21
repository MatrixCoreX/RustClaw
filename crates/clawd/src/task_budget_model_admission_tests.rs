use super::*;
fn task(tokens: u64) -> ClaimedTask {
    ClaimedTask { claim_attempt:1, task_id:uuid::Uuid::new_v4().to_string(), user_id:1,chat_id:1,user_key:None,channel:"ui".into(),external_user_id:None,external_chat_id:None,kind:"ask".into(),payload_json:json!({"task_role":"subagent_child","child_task_contract":{"budget":{"max_tokens":tokens}}}).to_string() }
}
#[test]
fn small_total_budget_is_rejected_before_call_or_provider_attempt() {
    let state = AppState::test_default_with_fixture_provider();
    let task = task(4096);
    let mut hints = ChatRequestHints::default();
    assert!(prepare(&state, &task, "prompt", "plan", &mut hints).is_err());
    assert_eq!(state.task_llm_call_count(&task.task_id), 0);
    let db = state.core.db.get().unwrap();
    let rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM task_model_token_reservations",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 0);
}
#[test]
fn retries_finalizers_and_restart_share_the_frozen_ceiling_and_unknown_usage() {
    let state = AppState::test_default_with_fixture_provider();
    let task = task(19000);
    let mut hints = ChatRequestHints::default();
    let guard = prepare(&state, &task, "prompt", "plan", &mut hints).unwrap();
    let attempt = guard.start(&mut hints, "fixture", "model").unwrap();
    assert!(hints.max_tokens.unwrap() <= 8192);
    drop(attempt);
    // A cancelled or unknown first attempt remains reserved after constructing a new logical call.
    assert!(prepare(
        &state,
        &task,
        "prompt",
        "verifier",
        &mut ChatRequestHints::default()
    )
    .is_err());
}
