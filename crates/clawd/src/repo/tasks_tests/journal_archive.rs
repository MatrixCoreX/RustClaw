use super::*;

#[test]
fn task_query_restores_archived_fields_and_preserves_public_projection() {
    let state = state_with_tasks_table();
    let id = Uuid::new_v4().to_string();
    let original = json!({"text":"result","task_lifecycle":{"state":"completed"},
        "task_journal":{"summary":{"verified":true},"trace":{
            "rounds":["journal evidence".repeat(4000)],"event_stream":{"events":[1,2,3]},
            "step_results":[{"step_id":"step_1","status":"ok"}]}}});
    insert_task(&state, &id, "succeeded", Some(&original), 10);
    let before = get_task_query_record(&state, &id).unwrap().unwrap().0;
    {
        let mut db = state.core.db.get().unwrap();
        assert_eq!(
            crate::task_journal_archive::compact_batch(&mut db, 64, 100_000)
                .unwrap()
                .compacted_rows,
            1
        );
        let raw: String = db
            .query_row(
                "SELECT result_json FROM tasks WHERE task_id=?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&raw)
            .unwrap()
            .pointer("/task_journal/trace/rounds")
            .is_none());
    }
    let after = get_task_query_record(&state, &id).unwrap().unwrap().0;
    assert_eq!(after.result_json, Some(original));
    assert_eq!(after.lifecycle, before.lifecycle);
    assert_eq!(
        serde_json::to_value(&after.goal).unwrap(),
        serde_json::to_value(&before.goal).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&after.status).unwrap(),
        serde_json::to_value(&before.status).unwrap()
    );
    let db = state.core.db.get().unwrap();
    db.execute(
        "UPDATE runtime_journal_archives SET storage_blob=x'00' WHERE task_id=?1",
        [&id],
    )
    .unwrap();
    drop(db);
    assert!(get_task_query_record(&state, &id).is_err());
}
