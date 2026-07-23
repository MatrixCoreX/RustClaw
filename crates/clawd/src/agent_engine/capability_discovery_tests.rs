use super::*;

#[test]
fn group_loader_requires_one_or_two_machine_tokens() {
    assert_eq!(
        parse_requested_groups(&json!({"groups": []})).unwrap_err(),
        "capability_group_load_count_invalid"
    );
    assert_eq!(
        parse_requested_groups(&json!({"groups": ["crypto", "weather", "kb"]})).unwrap_err(),
        "capability_group_load_count_invalid"
    );
    assert_eq!(
        parse_requested_groups(&json!({"groups": ["weather group"]})).unwrap_err(),
        "capability_group_load_token_invalid"
    );
    assert_eq!(
        parse_requested_groups(&json!({"groups": ["weather", "weather"]})).unwrap(),
        vec!["weather".to_string()]
    );
    assert_eq!(
        parse_requested_groups(&json!({"groups": ["weather", "crypto"]})).unwrap(),
        vec!["crypto".to_string(), "weather".to_string()]
    );
}

#[test]
fn group_loader_expands_only_exact_registry_groups() {
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "capability-loader".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut loop_state = LoopState::new();
    loop_state.round_no = 1;
    let loadable = crate::capability_map::planner_loadable_capability_group_names_for_task(
        &state,
        &task,
        &loop_state.loaded_capability_skills,
    );
    let group = loadable
        .first()
        .cloned()
        .expect("fixture must expose an on-demand registry group");
    let mut executed = 0;
    let decision = handle_capability_group_load(
        &state,
        &task,
        &mut loop_state,
        &json!({"groups": [&group]}),
        &format!("load:{group}"),
        1,
        1,
        &mut executed,
    )
    .unwrap();
    assert!(matches!(decision, ActionLoopDecision::StopRound(_)));
    assert_eq!(executed, 1);
    assert!(loop_state.loaded_capability_skills.contains(&group));
    assert!(loop_state
        .last_output
        .as_deref()
        .is_some_and(|output| output.contains(&format!("\"loaded_groups\":[\"{group}\"]"))));

    let error = match handle_capability_group_load(
        &state,
        &task,
        &mut loop_state,
        &json!({"groups": ["not_registered"]}),
        "load:invalid",
        2,
        2,
        &mut executed,
    ) {
        Ok(_) => panic!("unknown registry group must be rejected"),
        Err(error) => error,
    };
    assert!(error.contains("capability_group_not_loadable"));
}
