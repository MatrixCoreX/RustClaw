use super::*;

#[test]
fn list_file_names_capability_resolves_to_file_only_inventory() {
    let state = test_state_with_registry();
    let actions = vec![AgentAction::CallCapability {
        capability: "filesystem.list_file_names".to_string(),
        args: json!({
            "path": "logs",
            "max_entries": 4,
        }),
    }];

    let normalized = normalize_planned_actions(&state, None, &LoopState::new(1), "", None, actions);

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("path").and_then(Value::as_str), Some("logs"));
    assert_eq!(args.get("max_entries").and_then(Value::as_i64), Some(4));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
}

#[test]
fn list_directory_names_capability_resolves_to_directory_only_inventory() {
    let state = test_state_with_registry();
    let actions = vec![AgentAction::CallCapability {
        capability: "filesystem.list_directory_names".to_string(),
        args: json!({
            "path": "logs",
            "max_entries": 4,
        }),
    }];

    let normalized = normalize_planned_actions(&state, None, &LoopState::new(1), "", None, actions);

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("path").and_then(Value::as_str), Some("logs"));
    assert_eq!(args.get("max_entries").and_then(Value::as_i64), Some(4));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(false));
}
