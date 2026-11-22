use super::*;

#[test]
fn incomplete_required_argument_bindings_are_rejected_before_any_child_and_can_be_repaired() {
    let state = persistent_test_state();
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "parent-argument-binding-repair".into(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("test-key".into()),
        channel: "ui".into(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".into(),
        payload_json: serde_json::json!({"text":"inspect declared scope"}).to_string(),
    };
    insert_running_parent_task(&state, &task);
    let mut loop_state = LoopState::new();
    install_test_task_budget(&mut loop_state);
    let mut args = serde_json::json!({"action":"inline_readonly","role":"review","objective":"inspect selected range",
    "context_refs":["README.md"],"allowed_capabilities":["filesystem.read_text_range"],
    "capability_argument_constraints":[{"capability":"filesystem.read_text_range","allowed_args":[{
        "type":"object","required":["path","mode","start_line","end_line"],"properties":{"end_line":{"enum":[20]}}
    }]}]});
    let config = SubagentRuntimeConfig::default();
    let error = record_durable_readonly_child_task_from_args(
        &state,
        &task,
        &mut loop_state,
        1,
        1,
        &args,
        &config,
    )
    .unwrap_err();
    assert_eq!(error, SUBAGENT_STOP_SIGNAL_INVALID_ARGUMENT_CONSTRAINTS);
    let observation = loop_state.task_observations.last().unwrap();
    assert_eq!(observation["retryable"], true);
    assert_eq!(observation["side_effect_applied"], false);
    let issues = observation["field_issues"].as_array().unwrap();
    assert_eq!(issues.len(), 3);
    for name in ["path", "mode", "start_line"] {
        assert!(issues.iter().any(|v| v["path"]
            .as_str()
            .unwrap()
            .ends_with(&format!("/properties/{name}"))));
    }
    {
        let db = state.core.db.get().unwrap();
        let count: i64 = db
            .query_row(
                "SELECT count(*) FROM child_task_graph_nodes WHERE parent_task_id=?1",
                [&task.task_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
    args["capability_argument_constraints"][0]["allowed_args"][0]["properties"] = serde_json::json!({
        "path":{"enum":["README.md"]},"mode":{"enum":["range"]},
        "start_line":{"type":"integer","minimum":10,"maximum":20},"end_line":{"enum":[20]}});
    record_durable_readonly_child_task_from_args(
        &state,
        &task,
        &mut loop_state,
        2,
        1,
        &args,
        &config,
    )
    .unwrap();
    let db = state.core.db.get().unwrap();
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM child_task_graph_nodes WHERE parent_task_id=?1",
            [&task.task_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

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
        "capability_argument_constraints":[{"capability":"filesystem.read_text_range","allowed_args":[{
            "type":"object","required":["path","start_line","end_line"],"properties":{
                "path":{"enum":["README.md"]},"start_line":{"type":"integer","minimum":10,"maximum":20},
                "end_line":{"type":"integer","minimum":10,"maximum":20}}}]}],
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
            payload["child_task_contract"]["scope"]["capability_argument_constraints"],
            args["capability_argument_constraints"]
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
