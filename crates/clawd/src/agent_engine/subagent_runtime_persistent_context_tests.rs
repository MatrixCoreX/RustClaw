use super::*;
#[test]
fn single_batch_and_inherited_context_budgets_are_explicitly_rejected() {
    let task = ClaimedTask {
        claim_attempt: 1,
        task_id: uuid::Uuid::new_v4().to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "ui".into(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".into(),
        payload_json: "{}".into(),
    };
    for args in [
        json!({"role":"explorer","objective":"inspect","budget":{"max_context_chars":4096}}),
        json!({"budget":{"max_context_chars":4096},"children":[{"role":"explorer","objective":"inspect"}]}),
        json!({"children":[{"role":"explorer","objective":"inspect","budget":{"max_context_chars":4096}}]}),
    ] {
        assert_eq!(
            persistent_child_specs(&task, &args, &SubagentRuntimeConfig::default()).unwrap_err(),
            "subagent_durable_max_context_chars_unsupported"
        );
    }
}

#[test]
fn invalid_batch_contract_is_rejected_before_creating_children_with_field_paths() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = ClaimedTask {
        claim_attempt: 1,
        task_id: uuid::Uuid::new_v4().to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "ui".into(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".into(),
        payload_json: "{}".into(),
    };
    let args = json!({"children":[{"role":"explorer","objective":"inspect","context_refs":["fixture.rs"],
        "allowed_capabilities":["filesystem.read_text_range"],"result_contract":{"output_format":"machine_json",
        "result_schema":{"schema_version":"1","properties":{"line":{"minimum":"1"}}}}}]});
    let mut loop_state = LoopState::new();
    assert_eq!(
        record_durable_readonly_child_task_from_args(
            &state,
            &task,
            &mut loop_state,
            1,
            1,
            &args,
            &SubagentRuntimeConfig::default()
        )
        .unwrap_err(),
        SUBAGENT_STOP_SIGNAL_INVALID_RESULT_CONTRACT
    );
    let observation = loop_state.task_observations.last().unwrap();
    assert_eq!(observation["retryable"], true);
    assert_eq!(observation["provider_called"], false);
    assert_eq!(observation["side_effect_applied"], false);
    assert!(observation["field_issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["path"] == "/children/0/result_contract/schema_version"));
    let db = state.core.db.get().unwrap();
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM child_task_graph_nodes WHERE parent_task_id=?1",
            [&task.task_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    assert!(loop_state
        .task_budget_slice
        .as_ref()
        .is_none_or(|budget| budget.allocations.is_empty()));
}
