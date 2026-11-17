use super::*;

#[test]
fn packed_replay_progress_dedup_and_append_survive_terminal_cache_pruning() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task_id = "packed-history";
    state.seed_ask_task_row(task_id, 7, 11, "{}");
    let first=publish_event(&state,task_id,"tool_finished",json!({"skill":"example","requested_action_ref":"example.read","data":"evidence".repeat(4000)})).unwrap().unwrap();
    let progress=publish_event(&state,task_id,"skill_progress",json!({"schema_version":1,"source":"skill_progress","data_only":true,"data":"progress".repeat(4000)})).unwrap().unwrap();
    let mut db = state.core.db.get().unwrap();
    let versions: i64 = db
        .query_row(
            "SELECT count(*) FROM task_event_archive WHERE task_id=?1 AND storage_version=2",
            [task_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(versions, 2);
    db.execute(
        "UPDATE tasks SET status='succeeded' WHERE task_id=?1",
        [task_id],
    )
    .unwrap();
    assert_eq!(
        crate::task_event_storage_maintenance::prune_hot_batch(&mut db, i64::MAX, 128).unwrap(),
        2
    );
    drop(db);
    assert_eq!(
        latest_skill_progress_event(&state, task_id)
            .unwrap()
            .unwrap(),
        progress
    );
    let replay = replay_events_after(&state, task_id, 1).unwrap();
    assert!(replay.archive_recovered);
    assert_eq!(replay.events, vec![progress.clone()]);
    let duplicate=publish_event(&state,task_id,"tool_finished",json!({"skill":"example","requested_action_ref":"example.read","data":"evidence".repeat(4000)})).unwrap().unwrap();
    assert_eq!(duplicate, first);
    let final_event = publish_event(&state, task_id, "task_final", json!({"status":"succeeded"}))
        .unwrap()
        .unwrap();
    assert_eq!(final_event["seq"], 3);
    assert_eq!(final_event["previous_event_hash"], progress["event_hash"]);
    let replay = replay_events_after(&state, task_id, 0).unwrap();
    assert_eq!(replay.events.len(), 3);
    assert_eq!(
        replay.latest_snapshot.unwrap()["source_event_range"]["event_count"],
        3
    );
}

#[test]
fn packed_large_artifacts_are_lossless_and_corruption_is_visible() {
    let state = state();
    let payload = json!({"data":"中🙂".repeat(30_000)});
    let event = publish_event(
        &state,
        "packed-artifact",
        "coding_evidence",
        payload.clone(),
    )
    .unwrap()
    .unwrap();
    let id = event["artifact_refs"][0]["artifact_id"].as_str().unwrap();
    assert_eq!(
        read_event_artifact(&state, "packed-artifact", id)
            .unwrap()
            .unwrap(),
        payload
    );
    let db = state.core.db.get().unwrap();
    let (version,stored,logical):(i64,i64,i64)=db.query_row("SELECT storage_version,length(payload_json)+coalesce(length(storage_blob),0),payload_bytes FROM task_event_artifacts WHERE artifact_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(version, 2);
    assert!(stored < logical / 4);
    db.execute("UPDATE task_event_artifacts SET storage_blob=zeroblob(length(storage_blob)) WHERE artifact_id=?1",[id]).unwrap();
    drop(db);
    assert!(read_event_artifact(&state, "packed-artifact", id).is_err());
}
