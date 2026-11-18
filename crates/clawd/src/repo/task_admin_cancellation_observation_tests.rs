use super::*;
use crate::local_process_job::observation_test_support::LiveJobFixture;

#[test]
fn cancellation_observation_cannot_settle_a_live_process_with_an_early_exit_record() {
    let mut fixture = LiveJobFixture::new();
    fixture.request_cancel();
    fixture.write("exit_code", "0");
    let result = json!({"cancel_adapter_result": {
        "adapter_kind": "local_process_poll", "status": "already_terminal",
        "job_dir": fixture.path(),
    }});
    let settled = cancel_adapter_has_settled(&result);
    let adapter = cancel_local_process_job(
        &format!("local_process:{}", fixture.path().display()),
        1000,
        3600,
    );
    let live = crate::local_process_job::process_identity_state(fixture.path(), fixture.pid());
    fixture.finish();
    assert_eq!(
        live,
        crate::local_process_job::ProcessIdentityState::AliveVerified
    );
    assert!(!settled, "live_scope_was_settled");
    assert_eq!(adapter["status"], "already_requested");
    assert!(local_process_cancel_has_settled(fixture.path()));
}

#[test]
fn cancellation_observation_missing_pid_is_unknown_not_settled() {
    let mut fixture = LiveJobFixture::new();
    fixture.request_cancel();
    std::fs::remove_file(fixture.path().join("pid")).unwrap();
    let result = json!({"cancel_adapter_result": {
        "adapter_kind": "local_process_poll", "status": "already_requested",
        "job_dir": fixture.path(),
    }});
    let settled = cancel_adapter_has_settled(&result);
    fixture.finish();
    assert!(!settled, "unknown_scope_was_settled");
}

#[test]
fn cancellation_observation_does_not_publish_settlement_while_the_process_lives() {
    let mut fixture = LiveJobFixture::new();
    fixture.request_cancel();
    fixture.write("exit_code", "0");
    let result = json!({
        "task_lifecycle": {"state": "cancel_requested"},
        "cancel_adapter_result": {"adapter_kind": "local_process_poll",
            "status": "already_terminal", "job_dir": fixture.path()},
    });
    let state = state_with_tasks_table();
    let task_id = "cancellation-observation-database";
    insert_running_task(&state, task_id, &result);
    let outcome = {
        let db = state.core.db.get().unwrap();
        db.execute(
            "UPDATE tasks SET status = 'canceled' WHERE task_id = ?1",
            [task_id],
        )
        .unwrap();
        settle_cancelled_task_if_ready_in_db(&state, &db, task_id, 1000)
    };
    let stored = stored_result_json(&state, task_id);
    fixture.finish();
    assert_eq!(stored, result);
    assert!(!outcome.unwrap().settled);
}
