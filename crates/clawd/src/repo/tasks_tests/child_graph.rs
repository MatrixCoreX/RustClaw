use super::*;

#[test]
fn enqueue_child_specs_persists_and_serializes_current_workspace_writers() {
    let state = state_with_tasks_table();
    insert_task(
        &state,
        "task-parent-role-profile",
        "running",
        Some(&json!({})),
        1,
    );
    let parent = ChildTaskParentContext {
        parent_task_id: "task-parent-role-profile".to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("test-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
    };
    let mut first_write =
        sample_repo_child_spec("task-parent-role-profile", "task-child-role-write-1", true);
    first_write.role = "workspace_writer".to_string();
    first_write.permission_profile = ChildTaskPermissionProfile::LocalCurrentWorkspace;
    let mut second_write =
        sample_repo_child_spec("task-parent-role-profile", "task-child-role-write-2", false);
    second_write.role = "workspace_writer".to_string();
    second_write.permission_profile = ChildTaskPermissionProfile::LocalCurrentWorkspace;
    let read_only =
        sample_repo_child_spec("task-parent-role-profile", "task-child-role-read-1", false);
    let specs = vec![first_write, second_write, read_only];

    let summary =
        enqueue_child_task_specs(&state, &parent, &specs, 3, 1).expect("enqueue child specs");

    assert_eq!(summary["status"], "scheduled");
    assert_eq!(summary["queued_child_count"], 3);
    assert_eq!(summary["scheduler"]["decision"], "persisted_graph");
    assert_eq!(summary["scheduler"]["ready_child_count"], 2);
    assert_eq!(summary["scheduler"]["blocked_child_count"], 1);
    assert_eq!(
        summary["scheduler"]["blocked_child_tasks"][0]["child_task_id"],
        "task-child-role-write-2"
    );
    assert_eq!(
        summary["scheduler"]["blocked_child_tasks"][0]["readiness"],
        "blocked_dependency"
    );
    assert_eq!(stored_status(&state, "task-child-role-write-1"), "queued");
    assert_eq!(stored_status(&state, "task-child-role-read-1"), "queued");
    assert_eq!(stored_status(&state, "task-child-role-write-2"), "queued");
    let parent_result = stored_result_json(&state, "task-parent-role-profile");
    assert_eq!(
        parent_result["child_task_ids"][0],
        "task-child-role-write-1"
    );
    assert_eq!(
        parent_result["child_task_ids"][1],
        "task-child-role-write-2"
    );
    assert_eq!(parent_result["child_task_ids"][2], "task-child-role-read-1");
    let first_claim = claim_next_task(&state)
        .expect("claim first ready graph node")
        .expect("first ready graph node");
    let second_claim = claim_next_task(&state)
        .expect("claim second ready graph node")
        .expect("second ready graph node");
    assert_eq!(first_claim.task_id, "task-child-role-write-1");
    assert_eq!(second_claim.task_id, "task-child-role-read-1");
    assert!(
        claim_next_task(&state)
            .expect("blocked graph claim")
            .is_none(),
        "dependency-blocked writer must not be claimed"
    );
}

#[test]
fn parent_failure_cancels_unfinished_graph_and_publishes_snapshot() {
    let state = state_with_tasks_table();
    insert_task(
        &state,
        "task-parent-graph-failure",
        "running",
        Some(&json!({})),
        1,
    );
    set_task_lease(
        &state,
        "task-parent-graph-failure",
        state.worker.worker_id.as_str(),
        crate::now_ts_u64() as i64 + 300,
        1,
        crate::now_ts_u64() as i64,
    );
    let parent = ChildTaskParentContext {
        parent_task_id: "task-parent-graph-failure".to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("test-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
    };
    let mut verifier =
        sample_repo_child_spec("task-parent-graph-failure", "task-child-verifier", true);
    verifier.scope["dependencies"] = json!(["task-child-writer"]);
    let specs = vec![
        sample_repo_child_spec("task-parent-graph-failure", "task-child-writer", true),
        verifier,
    ];
    enqueue_child_task_specs(&state, &parent, &specs, 2, 1).expect("enqueue graph");

    update_task_failure(&state, "task-parent-graph-failure", 1, "provider_failed")
        .expect("fail graph parent");

    assert_eq!(stored_status(&state, "task-parent-graph-failure"), "failed");
    assert_eq!(stored_status(&state, "task-child-writer"), "canceled");
    assert_eq!(stored_status(&state, "task-child-verifier"), "canceled");
    let db = state.core.db.get().expect("get db");
    let graph = crate::repo::child_task_graph::graph_snapshot(&db, "task-parent-graph-failure")
        .expect("graph snapshot")
        .expect("graph");
    drop(db);
    assert_eq!(graph["status"], "parent_failed");
    assert!(graph["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .all(|node| node["readiness"] == "canceled"));
    let events =
        crate::task_event_transport::replay_events_after(&state, "task-parent-graph-failure", 0)
            .expect("replay graph events");
    assert!(events.events.iter().any(|event| {
        event.get("event_kind").and_then(serde_json::Value::as_str) == Some("subagent_graph")
    }));
}
