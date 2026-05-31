use super::*;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "rustclaw_config_edit_{name}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("configs")).expect("create temp config dir");
    root
}

#[test]
fn plan_does_not_modify_toml() {
    let root = temp_root("plan");
    let path = root.join("configs/config.toml");
    let original = "[skills]\nskill_switches = { photo_organize = false }\n";
    std::fs::write(&path, original).expect("write config");

    let out = plan_config_change(
        &root,
        json!({
            "path": "configs/config.toml",
            "field_path": "skills.skill_switches.photo_organize",
            "value": true
        })
        .as_object()
        .expect("object"),
        false,
    )
    .expect("plan");

    assert_eq!(out["would_change"], true);
    assert_eq!(
        std::fs::read_to_string(&path).expect("read config"),
        original
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn apply_toml_field_and_read_back() {
    let root = temp_root("apply");
    let path = root.join("configs/config.toml");
    std::fs::write(
        &path,
        "# keep comment\n[skills]\nskill_switches = { photo_organize = false }\n",
    )
    .expect("write config");

    let out = apply_config_change(
        &root,
        json!({
            "path": "configs/config.toml",
            "field_path": "skills.skill_switches.photo_organize",
            "value": true
        })
        .as_object()
        .expect("object"),
        false,
    )
    .expect("apply");
    assert_eq!(out["applied"], true);
    let after = std::fs::read_to_string(&path).expect("read config");
    assert!(after.contains("# keep comment"));

    let back = read_back(
        &root,
        json!({
            "path": "configs/config.toml",
            "field_path": "skills.skill_switches.photo_organize"
        })
        .as_object()
        .expect("object"),
        false,
    )
    .expect("read back");
    assert_eq!(back["exists"], true);
    assert_eq!(back["value"], true);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn apply_creates_nested_toml_field() {
    let root = temp_root("nested");
    let path = root.join("configs/config.toml");
    std::fs::write(&path, "[skills]\n").expect("write config");

    apply_config_change(
        &root,
        json!({
            "path": "configs/config.toml",
            "field_path": "skills.skill_switches.new_skill",
            "value": true
        })
        .as_object()
        .expect("object"),
        false,
    )
    .expect("apply");
    let back = read_back(
        &root,
        json!({
            "path": "configs/config.toml",
            "field_path": "skills.skill_switches.new_skill"
        })
        .as_object()
        .expect("object"),
        false,
    )
    .expect("read back");
    assert_eq!(back["value"], true);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rejects_path_traversal() {
    let root = temp_root("path");
    let err = validate_config(
        &root,
        json!({ "path": "../outside.toml" })
            .as_object()
            .expect("object"),
        false,
    )
    .expect_err("path traversal should fail");
    assert_eq!(err.kind, "path_denied");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn sensitive_values_are_redacted() {
    let root = temp_root("secret");
    let path = root.join("configs/config.toml");
    std::fs::write(&path, "[llm.openai]\napi_key = \"REPLACE_ME_OPENAI\"\n").expect("write config");

    let out = plan_config_change(
        &root,
        json!({
            "path": "configs/config.toml",
            "field_path": "llm.openai.api_key",
            "value": "sk-test"
        })
        .as_object()
        .expect("object"),
        false,
    )
    .expect("plan");
    assert_eq!(out["new_value"], "<redacted>");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn guard_config_does_not_invent_missing_full_access_risk() {
    let root = temp_root("guard_missing_full_access");
    let path = root.join("configs/image.toml");
    std::fs::write(
        &path,
        "[image_vision]\nprovider = \"minimax\"\nmodel = \"MiniMax-M2.7\"\n",
    )
    .expect("write config");

    let out = guard_config(
        &root,
        json!({
            "path": "configs/image.toml",
            "format": "toml"
        })
        .as_object()
        .expect("object"),
        false,
    )
    .expect("guard");

    assert_eq!(out["risk_count"], 0);
    assert_eq!(out["risks"].as_array().expect("risks array").len(), 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn guard_config_reports_structured_registry_risks() {
    let root = temp_root("guard_registry");
    let path = root.join("configs/skills_registry.toml");
    std::fs::write(
        &path,
        r#"
[[skills]]
name = "alpha"
enabled = true
planner_visible = true
aliases = ["same"]
risk_level = "high"
requires_confirmation = false

[[skills]]
name = "alpha"
enabled = true
planner_visible = true
prompt_file = "prompts/skills/alpha.md"
aliases = ["same"]
"#,
    )
    .expect("write registry");

    let out = guard_config(
        &root,
        json!({
            "path": "configs/skills_registry.toml",
            "format": "toml"
        })
        .as_object()
        .expect("object"),
        false,
    )
    .expect("guard");
    let risks = out["risks"].as_array().expect("risks array");
    let joined = risks
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(joined.contains("duplicate skill name: alpha"));
    assert!(joined.contains("alias same is shared by alpha and alpha"));
    assert!(joined.contains("enabled planner-visible skill alpha is missing prompt_file"));
    assert!(joined
        .contains("enabled high-risk or side-effect skill alpha explicitly disables confirmation"));
    let _ = std::fs::remove_dir_all(root);
}
