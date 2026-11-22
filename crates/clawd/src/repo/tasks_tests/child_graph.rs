use super::*;

#[test]
fn parent_cancel_uses_persisted_graph_after_checkpoint_replaces_result() {
    assert_graph_cancellation_after_checkpoint(false);
}

#[test]
fn parent_cancel_repairs_terminal_graph_nodes_with_unfinished_task_rows() {
    assert_graph_cancellation_after_checkpoint(true);
}

fn assert_graph_cancellation_after_checkpoint(graph_already_cancelled: bool) {
    let state = state_with_tasks_table();
    let parent_id = "cancel-checkpoint-parent";
    let running_id = "cancel-checkpoint-a-running";
    let queued_id = "cancel-checkpoint-b-queued";
    let completed_id = "cancel-checkpoint-c-completed";
    let unrelated_id = "cancel-checkpoint-unrelated";
    insert_task(&state, parent_id, "running", Some(&json!({})), 1);
    insert_task(&state, unrelated_id, "queued", None, 1);
    let parent = ChildTaskParentContext {
        parent_task_id: parent_id.to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("test-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        execution_policy_stamp: None,
        interactive_approval_available: true,
    };
    let specs = [running_id, queued_id, completed_id]
        .map(|child| sample_repo_child_spec(parent_id, child, true));
    enqueue_child_task_specs(&state, &parent, &specs, 3, 1, 2).expect("enqueue graph");
    {
        let db = state.core.db.get().expect("get db");
        db.execute(
            "UPDATE tasks SET status='running' WHERE task_id=?1",
            rusqlite::params![running_id],
        )
        .expect("start child");
        db.execute(
            "UPDATE tasks SET status='succeeded' WHERE task_id=?1",
            rusqlite::params![completed_id],
        )
        .expect("complete sibling");
        crate::repo::child_task_graph::record_child_graph_task_terminal(
            &db,
            completed_id,
            "succeeded",
            "2",
        )
        .expect("record completed sibling");
        db.execute(
            "UPDATE tasks SET result_json=?2 WHERE task_id=?1",
            rusqlite::params![
                parent_id,
                json!({"checkpoint": {"id": "checkpoint-v1"}}).to_string()
            ],
        )
        .expect("replace parent checkpoint without child ids");
        if graph_already_cancelled {
            crate::repo::child_task_graph::mark_parent_graph_cancelled(&db, parent_id, "2")
                .expect("mark graph cancelled before task rows settle");
        }
    }
    let parent_cancellation = state.worker.register_active_task(parent_id);
    let child_cancellation = state.worker.register_active_task(running_id);

    assert_eq!(
        cancel_task_by_id(&state, parent_id).expect("cancel graph"),
        3
    );
    assert!(parent_cancellation.is_cancelled());
    assert!(child_cancellation.is_cancelled());
    assert_eq!(stored_status(&state, parent_id), "canceled");
    assert_eq!(stored_status(&state, running_id), "canceled");
    assert_eq!(stored_status(&state, queued_id), "canceled");
    assert_eq!(stored_status(&state, completed_id), "succeeded");
    assert_eq!(stored_status(&state, unrelated_id), "queued");
    assert_eq!(
        stored_result_json(&state, running_id)["terminal_reason"],
        "parent_cancelled"
    );
    assert_eq!(
        stored_result_json(&state, queued_id)["terminal_reason"],
        "parent_cancelled"
    );
    assert_eq!(
        cancel_task_by_id(&state, parent_id).expect("repeat cancel"),
        0
    );

    state.worker.unregister_active_task(parent_id);
    state.worker.unregister_active_task(running_id);
    assert_eq!(
        crate::repo::reconcile_cancelled_task_settlements(&state, 10)
            .expect("settle stopped tasks"),
        2
    );
    assert_eq!(
        stored_result_json(&state, running_id)["task_lifecycle"]["state"],
        "cancelled"
    );
}

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
        execution_policy_stamp: None,
        interactive_approval_available: true,
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
        enqueue_child_task_specs(&state, &parent, &specs, 3, 1, 2).expect("enqueue child specs");

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
        execution_policy_stamp: None,
        interactive_approval_available: true,
    };
    let mut verifier =
        sample_repo_child_spec("task-parent-graph-failure", "task-child-verifier", true);
    verifier.scope["dependencies"] = json!(["task-child-writer"]);
    let specs = vec![
        sample_repo_child_spec("task-parent-graph-failure", "task-child-writer", true),
        verifier,
    ];
    enqueue_child_task_specs(&state, &parent, &specs, 2, 1, 2).expect("enqueue graph");

    update_task_failure(&state, "task-parent-graph-failure", 1, "provider_failed")
        .expect("fail graph parent");

    assert_eq!(stored_status(&state, "task-parent-graph-failure"), "failed");
    assert_eq!(stored_status(&state, "task-child-writer"), "canceled");
    assert_eq!(stored_status(&state, "task-child-verifier"), "canceled");
    assert_eq!(
        stored_result_json(&state, "task-child-writer")["task_lifecycle"]["state"],
        "cancelled"
    );
    assert_eq!(
        stored_result_json(&state, "task-child-writer")["task_lifecycle"]["source"],
        "child_task_graph"
    );
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

#[test]
fn parent_failure_waits_for_an_active_child_runtime_before_settling_cancellation() {
    let state = state_with_tasks_table();
    insert_task(
        &state,
        "task-parent-active-child",
        "running",
        Some(&json!({})),
        1,
    );
    set_task_lease(
        &state,
        "task-parent-active-child",
        state.worker.worker_id.as_str(),
        crate::now_ts_u64() as i64 + 300,
        1,
        crate::now_ts_u64() as i64,
    );
    let parent = ChildTaskParentContext {
        parent_task_id: "task-parent-active-child".to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("test-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        execution_policy_stamp: None,
        interactive_approval_available: true,
    };
    let child_id = "task-active-child";
    enqueue_child_task_specs(
        &state,
        &parent,
        &[sample_repo_child_spec(
            "task-parent-active-child",
            child_id,
            true,
        )],
        1,
        1,
        2,
    )
    .expect("enqueue active child");
    let cancellation = state.worker.register_active_task(child_id);

    update_task_failure(&state, "task-parent-active-child", 1, "provider_failed")
        .expect("fail parent");

    assert!(cancellation.is_cancelled());
    assert_eq!(
        stored_result_json(&state, child_id)["task_lifecycle"]["state"],
        "cancel_requested"
    );
    assert_eq!(
        stored_result_json(&state, child_id)["task_lifecycle"]["parent_terminal_reason"],
        "parent_failed"
    );

    state.worker.unregister_active_task(child_id);
    assert_eq!(
        crate::repo::reconcile_cancelled_task_settlements(&state, 10)
            .expect("settle child cancellation"),
        1
    );
    assert_eq!(
        stored_result_json(&state, child_id)["task_lifecycle"]["state"],
        "cancelled"
    );
}
