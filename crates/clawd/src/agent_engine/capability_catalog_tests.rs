use super::*;

fn fixture_task() -> ClaimedTask {
    ClaimedTask {
        claim_attempt: 0,
        task_id: "capability-catalog-test".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

#[test]
fn authorized_catalog_search_and_expansion_share_contract_hash() {
    let state = AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = fixture_task();
    let entries = catalog_entries_for_task(&state, &task);
    let entry = entries.first().expect("fixture capability");
    let query = entry
        .semantic_tags
        .first()
        .cloned()
        .unwrap_or_else(|| entry.capability_id.clone());
    let search = search_catalog(&state, &task, &query);
    let search_match = search["matches"]
        .as_array()
        .and_then(|matches| {
            matches
                .iter()
                .find(|candidate| candidate["capability_ref"] == entry.capability_ref)
        })
        .expect("searched exact catalog entry");
    let (expanded, groups) =
        expand_catalog(&state, &task, std::slice::from_ref(&entry.capability_ref))
            .expect("expand authorized entry");

    assert_eq!(search["complete"], true);
    assert_eq!(expanded["complete"], true);
    assert_eq!(groups, vec![entry.skill_id.clone()]);
    assert_eq!(
        search_match["contract_sha256"],
        expanded["contracts"][0]["contract_sha256"]
    );
    assert!(expanded["contracts"][0]["contract"]["argument_schema"].is_object());
}

#[test]
fn expansion_cannot_grant_a_hidden_or_unknown_contract() {
    let state = AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let error = expand_catalog(
        &state,
        &fixture_task(),
        &["capability:not_visible/root".to_string()],
    )
    .unwrap_err();
    assert!(error.contains("capability_contract_not_authorized"));
}

#[test]
fn canonical_contract_hash_is_object_order_invariant() {
    let left = json!({"z": [3, 2, 1], "a": {"y": true, "b": "value"}});
    let right = json!({"a": {"b": "value", "y": true}, "z": [3, 2, 1]});
    assert_eq!(canonical_json(&left), canonical_json(&right));
    assert_eq!(
        sha256_hex(canonical_json(&left).as_bytes()),
        sha256_hex(canonical_json(&right).as_bytes())
    );
}

#[test]
fn exact_expansion_returns_action_documentation_with_the_original_contract_digest() {
    let state = AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = fixture_task();
    let entry = catalog_entries_for_task(&state, &task)
        .into_iter()
        .find(|e| e.capability_id == "filesystem.read_text_range")
        .unwrap();
    let (value, _) = expand_catalog(&state, &task, &[entry.capability_ref]).unwrap();
    let expanded = &value["contracts"][0];
    assert_eq!(expanded["contract_sha256"], entry.contract_sha256);
    let text = expanded["action_documentation"]["text"].as_str().unwrap();
    assert!(text.contains("## Boundaries"));
    assert!(!text.contains("<!--a1:"));
    let parameters = text
        .split("## Parameter Contract")
        .nth(1)
        .unwrap()
        .split("## Boundaries")
        .next()
        .unwrap();
    assert!(parameters.contains("| `read_text_range` |"));
    assert!(!parameters.contains("| `write_text` |"));
}

#[test]
fn child_catalog_and_expansion_share_the_same_allowed_capability_scope_as_native_tools() {
    let state = AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let mut task = fixture_task();
    let entries = catalog_entries_for_task(&state, &task);
    assert!(entries.len() > 1);
    let allowed = entries[0].capability_id.clone();
    let forbidden = entries[1].capability_ref.clone();
    task.payload_json=json!({"task_role":"subagent_child","child_task_contract":{"scope":{"allowed_capabilities":[allowed]}}}).to_string();
    let scoped = catalog_entries_for_task(&state, &task);
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].capability_id, allowed);
    assert!(expand_catalog(&state, &task, &[forbidden]).is_err());
}
