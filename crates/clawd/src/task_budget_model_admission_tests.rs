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

#[test]
fn known_usage_is_settled_exactly_and_a_later_call_cannot_raise_the_frozen_ceiling() {
    let state = AppState::test_default_with_fixture_provider();
    let task = task(19000);
    let mut hints = ChatRequestHints::default();
    let guard = prepare(&state, &task, "prompt", "plan", &mut hints).unwrap();
    let attempt = guard.start(&mut hints, "fixture", "model").unwrap();
    attempt.finish(
        Some(&LlmUsageSnapshot {
            input_tokens: Some(1000),
            output_tokens: Some(500),
            ..Default::default()
        }),
        None,
        0,
        200,
    );
    assert_eq!(
        charged_tokens(&state.core.db.get().unwrap(), &task.task_id).unwrap(),
        Some(1500)
    );
    let attempt = guard.start(&mut hints, "fixture", "model").unwrap();
    drop(attempt);
    let mut raised = task.clone();
    raised.payload_json =
        json!({"child_task_contract":{"budget":{"max_tokens":999999}}}).to_string();
    assert!(prepare(
        &state,
        &raised,
        "prompt",
        "verifier",
        &mut ChatRequestHints::default()
    )
    .is_err());
}

#[test]
fn parent_calls_respect_persisted_child_reservations() {
    use crate::task_budget_contract::{
        BudgetAllocationKind, BudgetHardCeilings, BudgetUnits, TaskBudgetProfile, TaskBudgetSlice,
    };
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = task(100000);
    state.seed_ask_task_row(&task.task_id, 1, 1, &task.payload_json);
    let mut slice = TaskBudgetSlice::new(
        TaskBudgetProfile::General,
        30000,
        BudgetHardCeilings {
            total_tokens: 100000,
            ..Default::default()
        },
    );
    slice
        .allocate(
            "child:1".into(),
            "child:1".into(),
            BudgetAllocationKind::ChildTask,
            BudgetUnits {
                tokens: 95000,
                ..Default::default()
            },
        )
        .unwrap();
    state.core.db.get().unwrap().execute("UPDATE tasks SET result_json=?2 WHERE task_id=?1",params![task.task_id,json!({"task_checkpoint":{"boundary_context":{"task_budget_slice":slice.to_machine_json()}}}).to_string()]).unwrap();
    assert!(prepare(
        &state,
        &task,
        "prompt",
        "plan",
        &mut ChatRequestHints::default()
    )
    .is_err());
}
