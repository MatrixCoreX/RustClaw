use super::*;

#[test]
fn summarize_structured_treats_root_token_as_document_root() {
    let root = temp_root("summarize_structured_root_token");
    std::fs::create_dir_all(&root).expect("create root");
    let path = root.join("settings.json");
    std::fs::write(&path, r#"{"app":{"name":"demo"},"enabled":true}"#).expect("write JSON fixture");
    let obj = serde_json::json!({
        "path": "settings.json",
        "format": "json",
        "field_path": "root",
    })
    .as_object()
    .expect("object")
    .clone();

    let out = summarize_structured(&root, &obj, false).expect("structured summary");
    let value: Value = serde_json::from_str(&out).expect("summary response");
    assert_eq!(value["exists"], true);
    assert!(value["node_count"].as_u64().unwrap_or(0) >= 4);
    let _ = std::fs::remove_dir_all(root);
}

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rustclaw-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

#[test]
fn structured_summary_returns_counts_and_paths_without_scalar_values() {
    let root = temp_root("structured-summary");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(
        root.join("config.toml"),
        r#"
[providers.alpha]
api_key = ""
enabled = false

[providers.beta]
api_key = "private-value"
enabled = true

[skills.switches]
weather = false
rss = true
"#,
    )
    .expect("write config");

    let result = summarize_structured(
        &root,
        json!({"path":"config.toml", "format":"toml"})
            .as_object()
            .expect("args"),
        false,
    )
    .expect("summarize");
    let value: Value = serde_json::from_str(&result).expect("json result");

    assert_eq!(value["scan_complete"], true);
    assert_eq!(value["empty_string_count"], 1);
    assert_eq!(value["false_boolean_count"], 2);
    assert_eq!(
        value["empty_string_paths"],
        json!(["providers.alpha.api_key"])
    );
    assert_eq!(
        value["false_boolean_paths"],
        json!(["providers.alpha.enabled", "skills.switches.weather"])
    );
    assert!(!result.contains("private-value"));

    std::fs::remove_dir_all(root).expect("remove root");
}

#[test]
fn structured_summary_can_scope_counts_to_one_machine_field_path() {
    let root = temp_root("structured-summary-scope");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(
        root.join("config.json"),
        r#"{"skills":{"switches":{"a":false,"b":true,"c":false}},"other":false}"#,
    )
    .expect("write config");

    let result = summarize_structured(
        &root,
        json!({"path":"config.json", "field_path":"skills.switches"})
            .as_object()
            .expect("args"),
        false,
    )
    .expect("summarize");
    let value: Value = serde_json::from_str(&result).expect("json result");

    assert_eq!(value["field_path"], "skills.switches");
    assert_eq!(value["false_boolean_count"], 2);
    assert_eq!(
        value["false_boolean_paths"],
        json!(["skills.switches.a", "skills.switches.c"])
    );

    std::fs::remove_dir_all(root).expect("remove root");
}
