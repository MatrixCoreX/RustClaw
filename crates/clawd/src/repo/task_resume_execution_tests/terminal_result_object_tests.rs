use super::*;
#[test]
fn large_terminal_snapshot_and_task_row_publish_atomically() {
    for reject_update in [false, true] {
        let state = state_with_tasks_table();
        let now = 10_000;
        let task = "large-terminal-object";
        let checkpoint = "large-terminal-checkpoint";
        let mut seed = terminal_projection_seed(
            task,
            checkpoint,
            "executing_async_poll",
            "poll_async_job",
            "async_poll_adapter_pending",
            "ready_to_poll_async_job",
            "async_poll_completed",
            now,
        );
        seed["text"] = json!("Existing visible reply");
        seed["messages"] = json!(["Existing visible reply"]);
        insert_task(&state, task, "running", Some(&seed), now);
        activate_resume_owner(&state, task, checkpoint, now, now + 30);
        claim_recorded_paused_checkpoint_resume_dispatch_result_internal(
            &state,
            task,
            checkpoint,
            "executing_async_poll",
            "poll_async_job",
            "async_poll_adapter_pending",
            "ready_to_poll_async_job",
            "async_poll_completed",
            now + 1,
            10,
        )
        .unwrap()
        .unwrap();
        let db = state.core.db.get().unwrap();
        crate::task_event_transport::ensure_event_schema(&db).unwrap();
        if reject_update {
            db.execute_batch("CREATE TRIGGER reject_terminal_update BEFORE UPDATE ON tasks WHEN NEW.status='succeeded' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        }
        drop(db);
        let final_result =
            json!({"status":"ok","output":"Complete immutable result ".repeat(2000)});
        let payload = json!({"schema_version":1,"task_id":task,"checkpoint_id":checkpoint,"executor_state":"executing_async_poll","executor_action":"poll_async_job","executor_status":"async_poll_adapter_pending","dispatch_state":"ready_to_poll_async_job","executor_result_status":"async_poll_completed","result_projection_state":"project_async_poll_completed","final_result_json":final_result});
        let committed =
            record_claimed_paused_checkpoint_resume_dispatch_result_projection_internal(
                &state,
                CLAIM_ATTEMPT,
                task,
                checkpoint,
                "executing_async_poll",
                "poll_async_job",
                "async_poll_adapter_pending",
                "ready_to_poll_async_job",
                "async_poll_completed",
                &payload,
                now + 2,
            )
            .unwrap();
        assert_eq!(committed, !reject_update);
        let (status, _, result) = stored_task_status_error_result(&state, task);
        let db = state.core.db.get().unwrap();
        if reject_update {
            assert_eq!(status, "running");
            let count: i64 = db
                .query_row(
                    "SELECT COUNT(*) FROM task_event_artifacts WHERE task_id=?1",
                    [task],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                count, 0,
                "rejected task publication must roll back the new snapshot"
            );
        } else {
            assert_eq!(status, "succeeded");
            assert_eq!(result["text"], "Existing visible reply");
            let projection = &result["task_lifecycle"]["resume_executor_result_projection"];
            assert!(projection.get("final_result_json").is_none());
            assert_eq!(
                crate::task_result_objects::read_snapshot(
                    &db,
                    task,
                    &projection["final_result_ref"]
                )
                .unwrap(),
                final_result
            );
        }
    }
}
