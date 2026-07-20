use super::*;

#[test]
fn error_extra_merges_machine_contract_and_details() {
    let extra = error_extra_with_details(
        "io_error",
        Some(json!({
            "operation": "read_file",
            "path": "/tmp/missing.txt"
        })),
    );

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_kind"], "io_error");
    assert_eq!(extra["message_key"], "skill.system_basic.io_error");
    assert_eq!(extra["retryable"], false);
    assert_eq!(extra["operation"], "read_file");
    assert_eq!(extra["path"], "/tmp/missing.txt");
}

fn temp_root(name: &str) -> PathBuf {
    let mut root = std::env::temp_dir();
    root.push(format!(
        "rustclaw_system_basic_{name}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create temp root");
    root
}

#[test]
fn runtime_status_returns_kernel_release_scalar() {
    let mut obj = Map::new();
    obj.insert("kind".to_string(), json!("uname_r"));

    let out = runtime_status(&obj).expect("kernel runtime status");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(
        value.get("kind").and_then(Value::as_str),
        Some("kernel_release")
    );
    let field_value = value
        .get("field_value")
        .and_then(Value::as_str)
        .expect("field value");
    assert!(!field_value.trim().is_empty());
    assert_eq!(
        value.get("command_output").and_then(Value::as_str),
        Some(field_value)
    );
}

#[test]
fn runtime_status_returns_current_time_scalar() {
    let mut obj = Map::new();
    obj.insert("kind".to_string(), json!("system_time"));

    let out = runtime_status(&obj).expect("current time runtime status");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(
        value.get("kind").and_then(Value::as_str),
        Some("current_time")
    );
    let field_value = value
        .get("field_value")
        .and_then(Value::as_str)
        .expect("field value");
    assert!(field_value.contains('T'));
    assert!(field_value.ends_with('Z'));
    assert_eq!(
        value.get("command_output").and_then(Value::as_str),
        Some(field_value)
    );
}

#[test]
fn resolve_path_blocks_absolute_outside_workspace_without_permission() {
    let root = temp_root("deny_abs");
    let denied = resolve_path(&root, "/etc/passwd", false).expect_err("should deny");
    assert_eq!(denied.kind, "path_denied");
    assert_eq!(denied.message, "path is outside workspace");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn resolve_path_allows_absolute_outside_workspace_with_permission() {
    let root = temp_root("allow_abs");
    let resolved = resolve_path(&root, "/etc/passwd", true).expect("should allow");
    assert_eq!(resolved, PathBuf::from("/etc/passwd"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn path_batch_facts_resolves_case_insensitive_leaf() {
    let root = temp_root("path_facts_case_leaf");
    let dir = root.join("reports");
    std::fs::create_dir_all(&dir).expect("create reports");
    std::fs::write(dir.join("Report.MD"), "ok").expect("write report");
    let mut obj = Map::new();
    obj.insert(
        "paths".to_string(),
        json!([root.join("reports/report.md").display().to_string()]),
    );
    obj.insert("fields".to_string(), json!(["exists", "size"]));

    let out = path_batch_facts(&root, &obj, true).expect("path facts");
    let value: Value = serde_json::from_str(&out).expect("json");
    assert_eq!(
        value.get("fields").and_then(Value::as_array).map(Vec::len),
        Some(2)
    );
    let fact = value
        .get("facts")
        .and_then(Value::as_array)
        .and_then(|facts| facts.first())
        .expect("first fact");
    assert_eq!(fact.get("exists").and_then(Value::as_bool), Some(true));
    assert_eq!(
        fact.get("resolved_from_case_insensitive")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert!(fact
        .get("fact")
        .and_then(|inner| inner.get("resolved_path"))
        .and_then(Value::as_str)
        .is_some_and(|path| path.ends_with("reports/Report.MD")));
    assert_eq!(
        value.get("basename").and_then(Value::as_str),
        Some("Report.MD")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn path_batch_facts_resolves_unique_stem_leaf() {
    let root = temp_root("path_facts_stem_leaf");
    let dir = root.join("stem_unique");
    std::fs::create_dir_all(&dir).expect("create stem dir");
    std::fs::write(dir.join("ABCD.txt"), "ok").expect("write target");
    let mut obj = Map::new();
    obj.insert(
        "paths".to_string(),
        json!([root.join("stem_unique/abcd").display().to_string()]),
    );

    let out = path_batch_facts(&root, &obj, true).expect("path facts");
    let value: Value = serde_json::from_str(&out).expect("json");
    let fact = value
        .get("facts")
        .and_then(Value::as_array)
        .and_then(|facts| facts.first())
        .expect("first fact");
    assert_eq!(fact.get("exists").and_then(Value::as_bool), Some(true));
    assert_eq!(
        fact.get("resolved_from_stem").and_then(Value::as_bool),
        Some(true)
    );
    assert!(fact
        .get("fact")
        .and_then(|inner| inner.get("resolved_path"))
        .and_then(Value::as_str)
        .is_some_and(|path| path.ends_with("stem_unique/ABCD.txt")));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn compare_paths_returns_exists_and_same_path_fields() {
    let root = temp_root("compare_paths_machine_fields");
    std::fs::write(root.join("left.txt"), "same").expect("write left");
    std::fs::write(root.join("right.txt"), "same").expect("write right");
    let mut obj = Map::new();
    obj.insert("left_path".to_string(), json!("left.txt"));
    obj.insert("right_path".to_string(), json!("right.txt"));

    let out = compare_paths(&root, &obj, false).expect("compare paths");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(
        value.get("action").and_then(Value::as_str),
        Some("compare_paths")
    );
    assert_eq!(
        value
            .get("left")
            .and_then(|left| left.get("exists"))
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        value
            .get("right")
            .and_then(|right| right.get("exists"))
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        value
            .get("comparison")
            .and_then(|comparison| comparison.get("same_path"))
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        value
            .get("field_value")
            .and_then(|field_value| field_value.get("same_path"))
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        value
            .get("field_value")
            .and_then(|field_value| field_value.get("left_exists"))
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        value
            .get("field_value")
            .and_then(|field_value| field_value.get("right_exists"))
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        value
            .get("comparison")
            .and_then(|comparison| comparison.get("same_content"))
            .and_then(Value::as_bool),
        Some(true)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn tree_summary_returns_machine_readable_directory_rows() {
    let root = temp_root("tree_summary_rows");
    std::fs::create_dir_all(root.join("docs/archive")).expect("create docs");
    std::fs::create_dir_all(root.join("tmp/cache")).expect("create tmp");
    std::fs::write(root.join("docs/service_notes.md"), "service notes").expect("write docs file");
    std::fs::write(root.join("docs/archive/README.txt"), "archive").expect("write archive file");
    std::fs::write(root.join("tmp/cache.dat"), "cache").expect("write tmp file");

    let mut obj = Map::new();
    obj.insert("path".to_string(), json!("."));
    obj.insert("max_depth".to_string(), json!(2));
    obj.insert("max_children_per_dir".to_string(), json!(12));

    let out = tree_summary(&root, &obj, false).expect("tree summary");
    let value: Value = serde_json::from_str(&out).expect("json");
    let rows = value
        .get("summary_rows")
        .and_then(Value::as_array)
        .expect("summary rows");
    let candidates = value
        .get("candidates")
        .and_then(Value::as_array)
        .expect("candidate rows");

    assert_eq!(
        value.get("count").and_then(Value::as_u64),
        Some(rows.len() as u64)
    );
    assert_eq!(candidates, rows);

    let docs = rows
        .iter()
        .find(|row| row.get("path").and_then(Value::as_str) == Some("docs"))
        .expect("docs row");
    assert_eq!(docs.get("name").and_then(Value::as_str), Some("docs"));
    assert_eq!(docs.get("file_count").and_then(Value::as_u64), Some(1));
    assert_eq!(docs.get("dir_count").and_then(Value::as_u64), Some(1));
    assert_eq!(docs.get("truncated").and_then(Value::as_bool), Some(false));

    let archive = rows
        .iter()
        .find(|row| row.get("path").and_then(Value::as_str) == Some("docs/archive"))
        .expect("archive row");
    assert_eq!(archive.get("file_count").and_then(Value::as_u64), Some(1));
    assert_eq!(
        archive.get("omitted_children").and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        archive.get("truncated").and_then(Value::as_bool),
        Some(true)
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn path_batch_facts_keeps_ambiguous_stem_missing() {
    let root = temp_root("path_facts_ambiguous_stem");
    let dir = root.join("stem_ambiguous");
    std::fs::create_dir_all(&dir).expect("create stem dir");
    std::fs::write(dir.join("ABCD.txt"), "one").expect("write first");
    std::fs::write(dir.join("abcd.md"), "two").expect("write second");
    let mut obj = Map::new();
    obj.insert(
        "paths".to_string(),
        json!([root.join("stem_ambiguous/abcd").display().to_string()]),
    );

    let out = path_batch_facts(&root, &obj, true).expect("path facts");
    let value: Value = serde_json::from_str(&out).expect("json");
    let fact = value
        .get("facts")
        .and_then(Value::as_array)
        .and_then(|facts| facts.first())
        .expect("first fact");
    assert_eq!(fact.get("exists").and_then(Value::as_bool), Some(false));
    assert_eq!(fact.get("kind").and_then(Value::as_str), Some("missing"));
    assert_eq!(
        fact.get("error_code").and_then(Value::as_str),
        Some("path_not_found")
    );
    assert_eq!(fact.get("error").and_then(Value::as_str), Some("not found"));
    assert!(value.get("basename").is_some_and(Value::is_null));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn extract_field_supports_array_filter_segments_for_toml() {
    let root = temp_root("extract_field_toml_filter");
    let target = root.join("skills_registry.toml");
    std::fs::write(
        &target,
        r#"
[[skills]]
name = "read_file"
planner_kind = "tool"

[[skills]]
name = "stock"
planner_kind = "skill"

[[skills]]
name = "run_cmd"
planner_kind = "tool"
"#,
    )
    .expect("write toml");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!(target.display().to_string()));
    obj.insert("format".to_string(), json!("toml"));
    obj.insert(
        "field_path".to_string(),
        json!("skills[?(@.name=='run_cmd')].planner_kind"),
    );

    let out = extract_field(&root, &obj, true).expect("extract field");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(value.get("exists").and_then(Value::as_bool), Some(true));
    assert_eq!(
        value.get("value_text").and_then(Value::as_str),
        Some("tool")
    );
    assert_eq!(
        value.get("value_type").and_then(Value::as_str),
        Some("string")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn extract_field_resolves_array_item_key_path_for_toml() {
    let root = temp_root("extract_field_array_item_key");
    let target = root.join("skills_registry.toml");
    std::fs::write(
        &target,
        r#"
[[skills]]
name = "read_file"
planner_kind = "tool"

[[skills]]
name = "stock"
planner_kind = "skill"

[[skills]]
name = "run_cmd"
planner_kind = "tool"
"#,
    )
    .expect("write toml");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!(target.display().to_string()));
    obj.insert("format".to_string(), json!("toml"));
    obj.insert("field_path".to_string(), json!("run_cmd.planner_kind"));

    let out = extract_field(&root, &obj, true).expect("extract field");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(value.get("exists").and_then(Value::as_bool), Some(true));
    assert_eq!(
        value.get("value_text").and_then(Value::as_str),
        Some("tool")
    );
    assert_eq!(
        value.get("resolved_field_path").and_then(Value::as_str),
        Some("skills[name=run_cmd].planner_kind")
    );
    assert_eq!(
        value.get("match_strategy").and_then(Value::as_str),
        Some("array_item_key_path")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn extract_field_resolves_array_item_identity_for_toml() {
    let root = temp_root("extract_field_array_item_identity");
    let target = root.join("skills_registry.toml");
    std::fs::write(
        &target,
        r#"
[[skills]]
name = "read_file"
planner_kind = "tool"

[[skills]]
name = "run_cmd"
planner_kind = "tool"
runner_name = "run-cmd-skill"
"#,
    )
    .expect("write toml");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!(target.display().to_string()));
    obj.insert("format".to_string(), json!("toml"));
    obj.insert("field_path".to_string(), json!("run_cmd"));

    let out = extract_field(&root, &obj, true).expect("extract field");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(value.get("exists").and_then(Value::as_bool), Some(true));
    assert_eq!(
        value
            .get("value")
            .and_then(Value::as_object)
            .and_then(|obj| obj.get("planner_kind"))
            .and_then(Value::as_str),
        Some("tool")
    );
    assert_eq!(
        value.get("resolved_field_path").and_then(Value::as_str),
        Some("skills[name=run_cmd]")
    );
    assert_eq!(
        value.get("match_strategy").and_then(Value::as_str),
        Some("array_item_identity")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn extract_field_keeps_ambiguous_array_item_identity_missing() {
    let root = temp_root("extract_field_ambiguous_array_item_identity");
    let target = root.join("skills_registry.toml");
    std::fs::write(
        &target,
        r#"
[[skills]]
name = "run_cmd"
planner_kind = "tool"

[[aliases]]
name = "run_cmd"
target = "system.run_command"
"#,
    )
    .expect("write toml");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!(target.display().to_string()));
    obj.insert("format".to_string(), json!("toml"));
    obj.insert("field_path".to_string(), json!("run_cmd"));

    let out = extract_field(&root, &obj, true).expect("extract field");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(value.get("exists").and_then(Value::as_bool), Some(false));
    assert_eq!(value.get("match_count").and_then(Value::as_u64), Some(2));
    assert_eq!(
        value.get("match_strategy").and_then(Value::as_str),
        Some("array_item_identity")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn extract_field_resolves_unique_bare_key_in_nested_toml() {
    let root = temp_root("extract_field_unique_bare_key");
    let target = root.join("config.toml");
    std::fs::write(
        &target,
        r#"
[llm]
selected_vendor = "mimo"
selected_model = "mimo-v2.5-pro"
"#,
    )
    .expect("write toml");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!(target.display().to_string()));
    obj.insert("format".to_string(), json!("toml"));
    obj.insert("field_path".to_string(), json!("selected_vendor"));

    let out = extract_field(&root, &obj, true).expect("extract field");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(value.get("exists").and_then(Value::as_bool), Some(true));
    assert_eq!(
        value.get("value_text").and_then(Value::as_str),
        Some("mimo")
    );
    assert_eq!(
        value.get("resolved_field_path").and_then(Value::as_str),
        Some("llm.selected_vendor")
    );
    assert_eq!(
        value.get("match_strategy").and_then(Value::as_str),
        Some("unique_bare_key")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn extract_field_resolves_unique_suffix_bare_key_in_nested_toml() {
    let root = temp_root("extract_field_unique_suffix_bare_key");
    let target = root.join("config.toml");
    std::fs::write(
        &target,
        r#"
[llm]
selected_vendor = "mimo"
selected_model = "mimo-v2.5-pro"
"#,
    )
    .expect("write toml");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!(target.display().to_string()));
    obj.insert("format".to_string(), json!("toml"));
    obj.insert("field_path".to_string(), json!("vendor"));

    let out = extract_field(&root, &obj, true).expect("extract field");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(value.get("exists").and_then(Value::as_bool), Some(true));
    assert_eq!(
        value.get("value_text").and_then(Value::as_str),
        Some("mimo")
    );
    assert_eq!(
        value.get("resolved_field_path").and_then(Value::as_str),
        Some("llm.selected_vendor")
    );
    assert_eq!(
        value.get("match_strategy").and_then(Value::as_str),
        Some("unique_bare_key_suffix")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn extract_field_resolves_parent_scoped_suffix_key_in_nested_toml() {
    let root = temp_root("extract_field_parent_scoped_suffix_key");
    let target = root.join("config.toml");
    std::fs::write(
        &target,
        r#"
[llm]
selected_vendor = "minimax"
selected_model = "MiniMax-M2.7"
"#,
    )
    .expect("write toml");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!(target.display().to_string()));
    obj.insert("format".to_string(), json!("toml"));
    obj.insert("field_path".to_string(), json!("llm.vendor"));

    let out = extract_field(&root, &obj, true).expect("extract field");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(value.get("exists").and_then(Value::as_bool), Some(true));
    assert_eq!(
        value.get("value_text").and_then(Value::as_str),
        Some("minimax")
    );
    assert_eq!(
        value.get("resolved_field_path").and_then(Value::as_str),
        Some("llm.selected_vendor")
    );
    assert_eq!(
        value.get("match_strategy").and_then(Value::as_str),
        Some("parent_scoped_key_suffix")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn extract_field_resolves_missing_parent_leaf_suffix_key() {
    let root = temp_root("extract_field_missing_parent_leaf_suffix_key");
    let target = root.join("config.toml");
    std::fs::write(
        &target,
        r#"
[llm]
selected_vendor = "minimax"
selected_model = "MiniMax-M2.7"
"#,
    )
    .expect("write toml");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!(target.display().to_string()));
    obj.insert("format".to_string(), json!("toml"));
    obj.insert("field_path".to_string(), json!("model.vendor"));

    let out = extract_field(&root, &obj, true).expect("extract field");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(value.get("exists").and_then(Value::as_bool), Some(true));
    assert_eq!(
        value.get("value_text").and_then(Value::as_str),
        Some("minimax")
    );
    assert_eq!(
        value.get("resolved_field_path").and_then(Value::as_str),
        Some("llm.selected_vendor")
    );
    assert_eq!(
        value.get("match_strategy").and_then(Value::as_str),
        Some("missing_parent_leaf_key_suffix")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn extract_field_resolves_nested_json_schema_properties_path() {
    let root = temp_root("extract_field_json_schema_properties_path");
    let target = root.join("schema.json");
    std::fs::write(
        &target,
        r#"{
  "type": "object",
  "properties": {
    "reference_resolution": {
      "type": "object",
      "properties": {
        "target": {
          "type": "string",
          "enum": [
            "none",
            "current_action_result",
            "current_turn_locator"
          ]
        }
      }
    }
  }
}"#,
    )
    .expect("write json");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!(target.display().to_string()));
    obj.insert("format".to_string(), json!("json"));
    obj.insert("field_path".to_string(), json!("properties.target.enum"));

    let out = extract_field(&root, &obj, true).expect("extract field");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(value.get("exists").and_then(Value::as_bool), Some(true));
    assert_eq!(
        value.get("resolved_field_path").and_then(Value::as_str),
        Some("properties.reference_resolution.properties.target.enum")
    );
    assert_eq!(
        value.get("match_strategy").and_then(Value::as_str),
        Some("json_schema_properties_path")
    );
    assert_eq!(
        value
            .get("value")
            .and_then(Value::as_array)
            .and_then(|items| items.get(1))
            .and_then(Value::as_str),
        Some("current_action_result")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn structured_keys_array_includes_object_identity_values() {
    let root = temp_root("structured_keys_array_identity");
    let target = root.join("skills_registry.toml");
    std::fs::write(
        &target,
        r#"
[[skills]]
name = "fs_basic"
planner_kind = "tool"

[[skills]]
name = "config_basic"
planner_kind = "tool"
"#,
    )
    .expect("write toml");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!(target.display().to_string()));
    obj.insert("field_path".to_string(), json!("skills"));

    let out = structured_keys(&root, &obj, true).expect("structured keys");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(
        value.get("container_type").and_then(Value::as_str),
        Some("array")
    );
    assert_eq!(
        value
            .get("identity_values")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(Value::as_str),
        Some("fs_basic")
    );
    assert_eq!(
        value
            .get("indices_preview")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("identity_value"))
            .and_then(Value::as_str),
        Some("fs_basic")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn extract_field_does_not_suffix_match_object_values() {
    let root = temp_root("extract_field_suffix_object_value");
    let target = root.join("config.toml");
    std::fs::write(
        &target,
        r#"
[tools.by_provider.openai]
allow = []
deny = []
"#,
    )
    .expect("write toml");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!(target.display().to_string()));
    obj.insert("format".to_string(), json!("toml"));
    obj.insert("field_path".to_string(), json!("provider"));

    let out = extract_field(&root, &obj, true).expect("extract field");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(value.get("exists").and_then(Value::as_bool), Some(false));
    assert_eq!(value.get("match_count").and_then(Value::as_u64), Some(0));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn extract_fields_resolves_suffix_scalars_without_object_matches() {
    let root = temp_root("extract_fields_suffix_scalars");
    let target = root.join("config.toml");
    std::fs::write(
        &target,
        r#"
[llm]
selected_vendor = "minimax"
selected_model = "MiniMax-M2.7"

[tools.by_provider.openai]
allow = []
deny = []
"#,
    )
    .expect("write toml");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!(target.display().to_string()));
    obj.insert("format".to_string(), json!("toml"));
    obj.insert(
        "field_paths".to_string(),
        json!(["llm.vendor", "provider", "selected_model"]),
    );

    let out = extract_fields(&root, &obj, true).expect("extract fields");
    let value: Value = serde_json::from_str(&out).expect("json");
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .expect("results");

    assert_eq!(
        results[0]
            .get("resolved_field_path")
            .and_then(Value::as_str),
        Some("llm.selected_vendor")
    );
    assert_eq!(
        results[0].get("value_text").and_then(Value::as_str),
        Some("minimax")
    );
    assert_eq!(
        results[1].get("exists").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        results[2]
            .get("resolved_field_path")
            .and_then(Value::as_str),
        Some("llm.selected_model")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn extract_fields_resolves_model_catalog_machine_aliases() {
    let root = temp_root("model_catalog_aliases");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("create configs");
    std::fs::write(
        config_dir.join("config.toml"),
        r#"
[llm]
selected_vendor = "minimax"
selected_model = "MiniMax-M3"

[llm.minimax]
model = "MiniMax-M3"
base_url = "https://api.minimaxi.com/v1"
context_window_tokens = 1000000
timeout_seconds = 180
models = ["MiniMax-M3", "MiniMax-M2.7"]
"#,
    )
    .expect("write config");
    std::fs::write(
        config_dir.join("image.toml"),
        r#"
[image_vision]
minimax_models = ["MiniMax-M3"]

[image_generation]
minimax_models = ["image-01"]
"#,
    )
    .expect("write image config");
    std::fs::write(
        config_dir.join("audio.toml"),
        r#"
[audio_synthesize]
minimax_models = ["speech-2.8-turbo"]
"#,
    )
    .expect("write audio config");
    std::fs::write(
        config_dir.join("video.toml"),
        r#"
[video_generation]
minimax_models = ["MiniMax-Hailuo-2.3"]
"#,
    )
    .expect("write video config");
    std::fs::write(
        config_dir.join("music.toml"),
        r#"
[music_generation]
minimax_models = ["music-2.6"]
"#,
    )
    .expect("write music config");

    let mut obj = Map::new();
    obj.insert("path".to_string(), json!("configs/config.toml"));
    obj.insert(
        "field_paths".to_string(),
        json!([
            "providers.MiniMax",
            "providers.MiniMax.selected_model",
            "providers.MiniMax.text_model",
            "providers.MiniMax.text.context_window_tokens",
            "providers.MiniMax.models.M3",
            "providers.MiniMax.models",
            "providers.MiniMax.base_url",
            "providers.MiniMax.context_window_tokens",
            "providers.MiniMax.models.M3.understanding_inputs",
            "providers.MiniMax.models.M3.generation_boundary",
            "models.MiniMax-M3.capabilities.generation",
            "providers.minimax.m3.understanding_inputs",
            "providers.minimax.m3.generation_boundary"
        ]),
    );

    let out = extract_fields(&root, &obj, false).expect("extract aliases");
    let value: Value = serde_json::from_str(&out).expect("json");
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .expect("results");
    assert_eq!(results.len(), 13);
    assert!(results
        .iter()
        .all(|item| item.get("exists").and_then(Value::as_bool) == Some(true)));
    let provider = results[0].get("value").expect("provider object");
    assert_eq!(
        provider.get("model").and_then(Value::as_str),
        Some("MiniMax-M3")
    );
    assert!(provider
        .get("understanding_inputs")
        .and_then(Value::as_array)
        .expect("provider understanding inputs")
        .iter()
        .any(|item| item.as_str() == Some("video")));
    assert!(provider
        .get("models")
        .and_then(Value::as_array)
        .expect("provider models")
        .iter()
        .any(|item| item.get("model").and_then(Value::as_str) == Some("MiniMax-M3")));
    assert_eq!(
        results[1].get("value_text").and_then(Value::as_str),
        Some("MiniMax-M3")
    );
    assert_eq!(
        results[2].get("value_text").and_then(Value::as_str),
        Some("MiniMax-M3")
    );
    assert_eq!(
        results[3].get("value").and_then(Value::as_i64),
        Some(1_000_000)
    );
    assert_eq!(
        results[4]
            .get("value")
            .and_then(|value| value.get("model"))
            .and_then(Value::as_str),
        Some("MiniMax-M3")
    );
    assert!(results[5]
        .get("value")
        .and_then(Value::as_array)
        .expect("provider models object")
        .iter()
        .any(|item| {
            item.get("understanding_inputs")
                .and_then(Value::as_array)
                .is_some_and(|inputs| inputs.iter().any(|input| input.as_str() == Some("image")))
        }));
    let understanding = results[8]
        .get("value")
        .and_then(Value::as_array)
        .expect("understanding inputs");
    assert!(understanding
        .iter()
        .any(|item| item.as_str() == Some("image")));
    assert!(understanding
        .iter()
        .any(|item| item.as_str() == Some("video")));
    let generation = results[9].get("value").expect("generation boundary");
    assert_eq!(
        generation.get("execution_contract").and_then(Value::as_str),
        Some("async_start_poll_cancel")
    );
    let skills = generation
        .get("media_skill_capabilities")
        .and_then(Value::as_array)
        .expect("media skills");
    assert!(skills
        .iter()
        .any(|item| item.as_str() == Some("image.generate")));
    assert!(skills
        .iter()
        .any(|item| item.as_str() == Some("music.generate")));
    assert_eq!(
        results[10].get("match_strategy").and_then(Value::as_str),
        Some("model_catalog_model_alias")
    );
    assert!(results[11]
        .get("value")
        .and_then(Value::as_array)
        .expect("direct provider model inputs")
        .iter()
        .any(|item| item.as_str() == Some("video")));
    assert_eq!(
        results[12]
            .get("value")
            .and_then(|value| value.get("execution_contract"))
            .and_then(Value::as_str),
        Some("async_start_poll_cancel")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn extract_field_keeps_ambiguous_bare_key_missing() {
    let root = temp_root("extract_field_ambiguous_bare_key");
    let target = root.join("config.toml");
    std::fs::write(
        &target,
        r#"
[primary]
name = "alpha"

[secondary]
name = "beta"
"#,
    )
    .expect("write toml");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!(target.display().to_string()));
    obj.insert("format".to_string(), json!("toml"));
    obj.insert("field_path".to_string(), json!("name"));

    let out = extract_field(&root, &obj, true).expect("extract field");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(value.get("exists").and_then(Value::as_bool), Some(false));
    assert_eq!(value.get("match_count").and_then(Value::as_u64), Some(2));
    assert_eq!(
        value.get("match_strategy").and_then(Value::as_str),
        Some("unique_bare_key")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn lookup_field_value_supports_bracket_index_and_filter() {
    let value = json!({
        "items": [
            {"name": "alpha", "versions": [{"kind": "old", "value": 1}]},
            {"name": "beta", "versions": [{"kind": "new", "value": 2}]}
        ]
    });

    assert_eq!(
        lookup_field_value(&value, "items[1].versions[0].value").and_then(Value::as_i64),
        Some(2)
    );
    assert_eq!(
        lookup_field_value(
            &value,
            "items[?(@.name==\"beta\")].versions[?(@.kind=='new')].value"
        )
        .and_then(Value::as_i64),
        Some(2)
    );
    assert_eq!(
        lookup_field_value(&value, "items.[name=beta].versions.[kind=new].value")
            .and_then(Value::as_i64),
        Some(2)
    );
}

#[test]
fn read_range_uses_range_mode_when_line_bounds_are_present() {
    let root = temp_root("read_range_bounds");
    let target = root.join("README.md");
    std::fs::write(&target, "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n").expect("write readme");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!(target.display().to_string()));
    obj.insert("start_line".to_string(), json!(1));
    obj.insert("end_line".to_string(), json!(8));

    let out = read_range(&root, &obj, true).expect("read range");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(value.get("mode").and_then(Value::as_str), Some("range"));
    assert_eq!(value.get("requested_n").and_then(Value::as_u64), Some(20));
    assert_eq!(value.get("start_line").and_then(Value::as_u64), Some(1));
    assert_eq!(value.get("end_line").and_then(Value::as_u64), Some(8));
    assert_eq!(value.get("total_lines").and_then(Value::as_u64), Some(10));
    assert_eq!(value.get("line_count").and_then(Value::as_u64), Some(10));
    assert_eq!(value.get("first_line").and_then(Value::as_str), Some("1"));
    assert!(value
        .get("excerpt")
        .and_then(Value::as_str)
        .is_some_and(|excerpt| excerpt.contains("8|8") && !excerpt.contains("9|9")));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn read_range_tail_does_not_project_an_unobserved_first_line() {
    let root = temp_root("read_range_tail_first_line");
    let target = root.join("README.md");
    std::fs::write(&target, "first\nsecond\nthird\n").expect("write readme");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!(target.display().to_string()));
    obj.insert("mode".to_string(), json!("tail"));
    obj.insert("n".to_string(), json!(1));

    let out = read_range(&root, &obj, true).expect("read range");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(value.get("line_count").and_then(Value::as_u64), Some(3));
    assert!(value.get("first_line").is_none());
    assert!(value
        .get("excerpt")
        .and_then(Value::as_str)
        .is_some_and(|excerpt| excerpt == "3|third"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn read_range_last_non_empty_projects_the_final_content_line() {
    let root = temp_root("read_range_last_non_empty");
    let target = root.join("service_notes.md");
    std::fs::write(
        &target,
        "# Service Notes\n\nOperators verify the release.\n \n",
    )
    .expect("write service notes");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!(target.display().to_string()));
    obj.insert("mode".to_string(), json!("last_non_empty"));

    let out = read_range(&root, &obj, true).expect("read last non-empty line");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(
        value.get("mode").and_then(Value::as_str),
        Some("last_non_empty")
    );
    assert_eq!(value.get("exists").and_then(Value::as_bool), Some(true));
    assert_eq!(value.get("line_number").and_then(Value::as_u64), Some(3));
    assert_eq!(
        value.get("line_text").and_then(Value::as_str),
        Some("Operators verify the release.")
    );
    assert_eq!(
        value.get("excerpt").and_then(Value::as_str),
        Some("3|Operators verify the release.")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn read_range_last_non_empty_reports_missing_for_blank_file() {
    let root = temp_root("read_range_last_non_empty_blank");
    let target = root.join("blank.txt");
    std::fs::write(&target, "\n \n\t\n").expect("write blank file");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!(target.display().to_string()));
    obj.insert("mode".to_string(), json!("last_non_empty"));

    let out = read_range(&root, &obj, true).expect("read blank file");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(value.get("exists").and_then(Value::as_bool), Some(false));
    assert!(value.get("line_number").is_some_and(Value::is_null));
    assert!(value.get("line_text").is_some_and(Value::is_null));
    assert_eq!(value.get("excerpt").and_then(Value::as_str), Some(""));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn read_range_title_field_selector_projects_markdown_heading() {
    let root = temp_root("read_range_title_selector");
    let target = root.join("service_notes.md");
    std::fs::write(
        &target,
        "# Service Notes\n\nRustClaw test fixture service notes.\n",
    )
    .expect("write markdown");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!(target.display().to_string()));
    obj.insert("field_selector".to_string(), json!("title"));
    obj.insert("mode".to_string(), json!("head"));
    obj.insert("n".to_string(), json!(10));

    let out = read_range(&root, &obj, true).expect("read range");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(
        value.get("field_selector").and_then(Value::as_str),
        Some("title")
    );
    assert_eq!(value.get("exists").and_then(Value::as_bool), Some(true));
    assert_eq!(
        value.get("field_value").and_then(Value::as_str),
        Some("Service Notes")
    );
    assert_eq!(
        value.get("title").and_then(Value::as_str),
        Some("Service Notes")
    );
    assert_eq!(
        value.get("value_text").and_then(Value::as_str),
        Some("Service Notes")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn read_range_title_field_selector_reports_missing_without_heading() {
    let root = temp_root("read_range_title_selector_missing");
    let target = root.join("note.txt");
    std::fs::write(&target, "plain note\nwithout markdown heading\n").expect("write note");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!(target.display().to_string()));
    obj.insert("field_selector".to_string(), json!("title"));

    let out = read_range(&root, &obj, true).expect("read range");
    let value: Value = serde_json::from_str(&out).expect("json");

    assert_eq!(
        value.get("field_selector").and_then(Value::as_str),
        Some("title")
    );
    assert_eq!(value.get("exists").and_then(Value::as_bool), Some(false));
    assert!(value.get("title").is_some_and(Value::is_null));
    assert!(value.get("field_value").is_some_and(Value::is_null));
    assert_eq!(value.get("value_text").and_then(Value::as_str), Some(""));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn read_range_compacts_internal_model_io_json_lines_by_default() {
    let root = temp_root("read_range_model_io_compact");
    let target = root.join("model_io.log");
    let line = json!({
        "task_id": "task-1",
        "vendor": "minimax",
        "model": "MiniMax-M2.7",
        "status": "ok",
        "prompt": "SECRET_PROMPT_SHOULD_NOT_BE_VISIBLE",
        "raw_response": "RAW_RESPONSE_SHOULD_NOT_BE_VISIBLE",
        "request_payload": {"messages": [{"role": "user", "content": "payload body"}]},
        "response": "{\"steps\":[]}",
        "usage": {"total_tokens": 12}
    })
    .to_string();
    std::fs::write(&target, format!("plain\n{line}\n")).expect("write model io log");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!(target.display().to_string()));
    obj.insert("mode".to_string(), json!("tail"));
    obj.insert("n".to_string(), json!(1));

    let out = read_range(&root, &obj, true).expect("read range");
    let value: Value = serde_json::from_str(&out).expect("json");
    let excerpt = value
        .get("excerpt")
        .and_then(Value::as_str)
        .expect("excerpt");

    assert!(excerpt.contains("task-1"));
    assert!(excerpt.contains("omitted_fields"));
    assert!(excerpt.contains("response_preview"));
    assert!(!excerpt.contains("SECRET_PROMPT_SHOULD_NOT_BE_VISIBLE"));
    assert!(!excerpt.contains("RAW_RESPONSE_SHOULD_NOT_BE_VISIBLE"));
    assert!(!excerpt.contains("payload body"));
    assert_eq!(
        value
            .get("line_safety")
            .and_then(|safety| safety.get("compacted_lines"))
            .and_then(Value::as_u64),
        Some(1)
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn find_path_match_includes_resolved_path() {
    let root = temp_root("find_path_resolved_path");
    let dir = root.join("case_only");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let target = dir.join("Report.MD");
    std::fs::write(&target, "hello").expect("write target");
    let mut obj = Map::new();
    obj.insert("root".to_string(), json!("case_only"));
    obj.insert("name".to_string(), json!("report.md"));
    obj.insert("match_mode".to_string(), json!("exact"));
    obj.insert("target_kind".to_string(), json!("file"));

    let out = find_path(&root, &obj, false).expect("find path");
    let value: Value = serde_json::from_str(&out).expect("json");
    let first = value
        .get("matches")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .expect("first match");

    assert_eq!(
        first.get("path").and_then(Value::as_str),
        Some("case_only/Report.MD")
    );
    assert_eq!(
        first.get("resolved_path").and_then(Value::as_str),
        Some(target.to_string_lossy().as_ref())
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn read_range_directory_error_is_structured() {
    let root = temp_root("read_range_directory_error");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!("."));

    let err = read_range(&root, &obj, true).expect_err("directory read should fail");
    assert_eq!(err.kind, "is_directory");
    assert!(err.message.contains("target is a directory"));

    let resp = handle(Req {
        request_id: "structured-dir".to_string(),
        args: json!({"action": "read_range", "path": "."}),
        context: Some(json!({"allow_path_outside_workspace": true})),
    });
    assert_eq!(resp.status, "error");
    assert_eq!(resp.error_kind.as_deref(), Some("is_directory"));
    assert_eq!(resp.platform.as_deref(), Some(std::env::consts::OS));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn inventory_missing_path_error_is_structured() {
    let root = temp_root("inventory_missing_error");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!("missing-directory"));

    let err = inventory_dir(&root, &obj, true).expect_err("missing directory should fail");
    assert_eq!(err.kind, "not_found");
    assert!(err
        .extra
        .as_ref()
        .and_then(|extra| extra.get("operation"))
        .and_then(Value::as_str)
        .is_some_and(|operation| operation == "metadata"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn inventory_dir_accepts_limit_alias_for_max_entries() {
    let root = temp_root("inventory_limit_alias");
    for name in ["a.log", "b.log", "c.log"] {
        std::fs::write(root.join(name), name).expect("write file");
    }
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!("."));
    obj.insert("names_only".to_string(), json!(true));
    obj.insert("files_only".to_string(), json!(true));
    obj.insert("limit".to_string(), json!(2));

    let out = inventory_dir(&root, &obj, true).expect("inventory");
    let value: Value = serde_json::from_str(&out).expect("json");
    let names = value.get("names").and_then(Value::as_array).expect("names");
    assert_eq!(names.len(), 2);
    assert_eq!(
        value.pointer("/counts/total").and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        value
            .pointer("/names_by_kind/files")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        value
            .pointer("/names_by_kind/dirs")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn inventory_dir_sorts_names_descending() {
    let root = temp_root("inventory_name_desc");
    for name in ["a.log", "c.log", "b.log"] {
        std::fs::write(root.join(name), name).expect("write file");
    }
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!("."));
    obj.insert("names_only".to_string(), json!(true));
    obj.insert("files_only".to_string(), json!(true));
    obj.insert("sort_by".to_string(), json!("name_desc"));
    obj.insert("limit".to_string(), json!(2));

    let out = inventory_dir(&root, &obj, true).expect("inventory");
    let value: Value = serde_json::from_str(&out).expect("json");
    let names: Vec<_> = value
        .get("names")
        .and_then(Value::as_array)
        .expect("names")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(names, vec!["c.log", "b.log"]);
    assert_eq!(
        value.get("sort_by").and_then(Value::as_str),
        Some("name_desc")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn inventory_dir_reports_size_summary_for_matched_files() {
    let root = temp_root("inventory_size_summary");
    std::fs::write(root.join("a-small.json"), b"{}").expect("write small");
    std::fs::write(root.join("z-large.json"), b"{\"items\":[1,2,3,4,5]}").expect("write large");
    std::fs::write(root.join("middle.txt"), b"ignored by ext filter").expect("write txt");
    std::fs::create_dir(root.join("nested")).expect("create dir");

    let mut obj = Map::new();
    obj.insert("path".to_string(), json!("."));
    obj.insert("files_only".to_string(), json!(true));
    obj.insert("ext_filter".to_string(), json!("json"));
    obj.insert("sort_by".to_string(), json!("name"));
    obj.insert("max_entries".to_string(), json!(1));

    let out = inventory_dir(&root, &obj, true).expect("inventory");
    let value: Value = serde_json::from_str(&out).expect("json");
    assert_eq!(
        value
            .pointer("/size_summary/matched_file_count")
            .and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        value
            .pointer("/size_summary/largest_file/name")
            .and_then(Value::as_str),
        Some("z-large.json")
    );
    assert_eq!(
        value
            .pointer("/size_summary/smallest_file/name")
            .and_then(Value::as_str),
        Some("a-small.json")
    );
    assert_eq!(
        value.get("entries").and_then(Value::as_array).map(Vec::len),
        Some(1)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn ext_filter_blank_string_means_no_filter() {
    let mut obj = Map::new();
    obj.insert("ext_filter".to_string(), json!(""));

    assert!(ext_filters(&obj).is_empty());
}

#[test]
fn ext_filter_normalizes_arrays_and_ignores_blank_items() {
    let mut obj = Map::new();
    obj.insert("ext_filter".to_string(), json!([" .MD ", "", ".toml"]));

    assert_eq!(ext_filters(&obj), vec!["md", "toml"]);
}

#[test]
fn context_permission_reads_nested_or_legacy_flag() {
    assert!(context_allows_path_outside_workspace(Some(&json!({
        "permissions": {"allow_path_outside_workspace": true}
    }))));
    assert!(context_allows_path_outside_workspace(Some(&json!({
        "allow_path_outside_workspace": true
    }))));
    assert!(!context_allows_path_outside_workspace(Some(&json!({
        "permissions": {"allow_path_outside_workspace": false}
    }))));
    assert!(!context_allows_path_outside_workspace(None));
}

#[test]
fn validate_structured_reports_parse_success_without_listing_keys() {
    let root = temp_root("validate_structured_ok");
    std::fs::write(
        root.join("config.toml"),
        "[llm]\nselected_vendor = \"mimo\"\n",
    )
    .expect("write toml");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!("config.toml"));
    obj.insert("format".to_string(), json!("toml"));

    let out = validate_structured(&root, &obj, true).expect("validate");
    let value: Value = serde_json::from_str(&out).expect("json");
    assert_eq!(
        value.get("action").and_then(Value::as_str),
        Some("validate_structured")
    );
    assert_eq!(value.get("valid").and_then(Value::as_bool), Some(true));
    assert!(value.get("keys").is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn validate_structured_reports_parse_failure_as_structured_output() {
    let root = temp_root("validate_structured_fail");
    std::fs::write(
        root.join("config.toml"),
        "[llm\nselected_vendor = \"mimo\"\n",
    )
    .expect("write toml");
    let mut obj = Map::new();
    obj.insert("path".to_string(), json!("config.toml"));
    obj.insert("format".to_string(), json!("toml"));

    let out = validate_structured(&root, &obj, true).expect("validate");
    let value: Value = serde_json::from_str(&out).expect("json");
    assert_eq!(value.get("valid").and_then(Value::as_bool), Some(false));
    assert_eq!(
        value.get("error_kind").and_then(Value::as_str),
        Some("invalid_data")
    );
    assert!(value
        .get("error_text")
        .and_then(Value::as_str)
        .is_some_and(|text| text.contains("toml parse failed")));
    let _ = std::fs::remove_dir_all(root);
}
