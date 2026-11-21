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
