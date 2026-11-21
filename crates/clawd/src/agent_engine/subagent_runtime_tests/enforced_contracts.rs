use super::*;

#[test]
fn frozen_limits_and_terminal_graph_refresh_do_not_need_new_delegation() {
    for terminal in ["succeeded", "failed"] {
        let state = persistent_test_state();
        let task = crate::ClaimedTask {
            claim_attempt: 0,
            task_id: format!("parent-auto-refresh-{terminal}"),
            user_id: 42,
            chat_id: 7,
            user_key: Some("test-key".into()),
            channel: "ui".into(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".into(),
            payload_json: serde_json::json!({"text":"parent"}).to_string(),
        };
        insert_running_parent_task(&state, &task);
        let mut loop_state = LoopState::new();
        install_test_task_budget(&mut loop_state);
        let args = serde_json::json!({"action":"inline_readonly","role":"review",
        "objective":"inspect selected scope","context_refs":["README.md"],
        "allowed_capabilities":["filesystem.read_text_range"],
        "budget":{"max_rounds":6,"max_tool_calls":6,"capability_call_limits":[
            {"capability":"filesystem.read_text_range","max_calls":2}
        ]}});
        record_durable_readonly_child_task_from_args(
            &state,
            &task,
            &mut loop_state,
            1,
            1,
            &args,
            &SubagentRuntimeConfig::default(),
        )
        .unwrap();
        let id = loop_state.task_observations.last().unwrap()["child_task_ids"][0]
            .as_str()
            .unwrap()
            .to_string();
        // Resume begins after the worker has persisted the parent's checkpoint.
        {
            let db = state.core.db.get().unwrap();
            db.execute("UPDATE tasks SET result_json=?2 WHERE task_id=?1",rusqlite::params![
                task.task_id,serde_json::json!({"child_task_ids":[id],"task_checkpoint":loop_state.task_checkpoint}).to_string()
            ]).unwrap();
        }
        let (_, payload) = child_task_row(&state, &id);
        assert_eq!(
            payload["child_task_contract"]["scope"]["budget_request"]
                ["max_tokens_explicitly_requested"],
            false
        );
        assert_eq!(
            payload["child_task_contract"]["scope"]["capability_call_limits"][0]["max_calls"],
            2
        );
        assert_eq!(
            resume_child_graph_before_planner(&state, &task, &mut loop_state).unwrap(),
            Some("waiting")
        );
        {
            let db = state.core.db.get().unwrap();
            db.execute("UPDATE tasks SET status=?2,result_json=?3 WHERE task_id=?1",rusqlite::params![id,terminal,
                serde_json::json!({"child_task_result":{"schema_version":2,"child_task_id":id,"parent_task_id":task.task_id,
                    "status":terminal,"role":"review","required":true,"evidence_refs":[],"finding_refs":[],
                    "structured_result":{"findings":[],"evidence":[]}}}).to_string()]).unwrap();
            // Intentionally omit the terminal graph callback: resume must
            // recover the durable completed task without new delegation.
        }
        assert_eq!(
            resume_child_graph_before_planner(&state, &task, &mut loop_state).unwrap(),
            Some(if terminal == "succeeded" {
                "ready"
            } else {
                "blocked"
            })
        );
        assert_eq!(
            loop_state.task_observations.last().unwrap()["model_observation"]["results"][0]
                ["budget_request"]["max_tokens_explicitly_requested"],
            false
        );
        assert!(loop_state.task_checkpoint.is_none());
        assert_eq!(
            loop_state.executed_step_results.last().unwrap().is_ok(),
            terminal == "succeeded"
        );
        assert_eq!(
            resume_child_graph_before_planner(&state, &task, &mut loop_state).unwrap(),
            None
        );
        let db = state.core.db.get().unwrap();
        let count: i64 = db
            .query_row(
                "SELECT count(*) FROM child_task_graph_nodes WHERE parent_task_id=?1",
                [&task.task_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}
