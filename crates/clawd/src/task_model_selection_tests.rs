use super::*;

#[test]
fn forged_or_partial_server_stamp_never_selects_a_provider() {
    let state = AppState::test_default_with_fixture_provider();
    for stamp in [
        json!({"schema_version": 1, "provider": "fixture", "model": "fixture"}),
        json!({
            "schema_version": 1,
            "provider": "fixture",
            "model": "fixture",
            "authority": "client"
        }),
        json!({
            "schema_version": 1,
            "provider": "fixture",
            "model": "fixture",
            "authority": "server_validated_model_catalog",
            "extra": true
        }),
    ] {
        let task = ClaimedTask {
            claim_attempt: 1,
            task_id: "task-model-selection".to_string(),
            user_id: 1,
            chat_id: 2,
            user_key: None,
            channel: "ui".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: json!({STAMP_FIELD: stamp}).to_string(),
        };
        assert!(providers_for_task_model_selection(&state, &task)
            .expect("stamp present")
            .is_empty());
    }
}

#[test]
fn submission_removes_untrusted_stamp_without_a_selection() {
    let state = AppState::test_default_with_fixture_provider();
    let mut payload = json!({
        "text": "task",
        STAMP_FIELD: {
            "schema_version": 1,
            "provider": "forged",
            "model": "forged",
            "authority": "server_validated_model_catalog"
        }
    });
    validate_and_stamp_task_model_selection(&state, &mut payload).unwrap();
    assert!(payload.get(STAMP_FIELD).is_none());
}

#[test]
fn selection_request_is_closed_before_catalog_lookup() {
    let state = AppState::test_default_with_fixture_provider();
    let mut extra = json!({
        REQUEST_FIELD: {
            "provider": "minimax",
            "model": "MiniMax-M3",
            "fallback": true
        }
    });
    assert_eq!(
        validate_and_stamp_task_model_selection(&state, &mut extra),
        Err("task_model_selection_additional_field_denied")
    );

    let mut partial = json!({REQUEST_FIELD: {"provider": "minimax"}});
    assert_eq!(
        validate_and_stamp_task_model_selection(&state, &mut partial),
        Err("task_model_id_invalid")
    );
}

#[test]
fn validated_selection_reuses_one_task_scoped_provider_runtime() {
    let state = AppState::test_default_with_fixture_provider();
    let providers = state.core.llm_providers.clone();
    let task_id = "task-model-cache";
    let cached = cache_task_selection(&state, task_id, "fixture", "fixture", providers.clone());
    assert_eq!(cached.len(), 1);
    let task = ClaimedTask {
        claim_attempt: 1,
        task_id: task_id.to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: json!({
            STAMP_FIELD: {
                "schema_version": 1,
                "provider": "fixture",
                "model": "fixture",
                "authority": "server_validated_model_catalog"
            }
        })
        .to_string(),
    };

    let first = providers_for_task_model_selection(&state, &task).unwrap();
    let second = providers_for_task_model_selection(&state, &task).unwrap();

    assert_eq!(first.len(), 1);
    assert!(Arc::ptr_eq(&first[0], &providers[0]));
    assert!(Arc::ptr_eq(&first[0], &second[0]));
    state.clear_task_llm_call_count(task_id);
    assert!(cached_task_selection(&state, task_id, "fixture", "fixture").is_none());
}
