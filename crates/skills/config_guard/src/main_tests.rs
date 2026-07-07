use super::*;
use serde_json::json;

#[test]
fn error_extra_merges_machine_contract_and_details() {
    let extra = error_extra_with_details(
        "not_found",
        Some(json!({
            "operation": "read_config",
            "path": "/tmp/missing.toml"
        })),
    );

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_kind"], "not_found");
    assert_eq!(extra["message_key"], "skill.config_guard.not_found");
    assert_eq!(extra["retryable"], false);
    assert_eq!(extra["operation"], "read_config");
    assert_eq!(extra["path"], "/tmp/missing.toml");
}

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "rustclaw_config_guard_{name}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("configs")).expect("create temp configs");
    root
}

#[test]
fn resolve_config_path_uses_existing_requested_file() {
    let root = temp_root("existing_requested");
    let requested = root.join("custom.toml");
    std::fs::write(&requested, "[tools]\n").expect("write requested config");
    let obj = json!({ "path": requested.display().to_string() });
    let resolved = resolve_config_path(&root, obj.as_object().expect("object"));

    assert_eq!(resolved, requested);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn resolve_config_path_falls_back_for_missing_configs_config_toml() {
    let root = temp_root("missing_requested");
    let default_path = root.join("configs/config.toml");
    std::fs::write(&default_path, "[tools]\n").expect("write default config");
    let obj = json!({ "path": root.join("rustclaw/configs/config.toml").display().to_string() });
    let resolved = resolve_config_path(&root, obj.as_object().expect("object"));

    assert_eq!(resolved, default_path);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn execute_returns_structured_extra_fields() {
    let root = temp_root("structured_extra");
    let path = root.join("config.toml");
    std::fs::write(&path, "[tools]\nallow_sudo = true\n").expect("write config");
    let out = execute(json!({ "path": path.display().to_string() })).expect("execute");

    assert_eq!(out.get("action").and_then(Value::as_str), Some("scan"));
    assert_eq!(out.get("risk_count").and_then(Value::as_u64), Some(2));
    assert!(out
        .get("risks")
        .and_then(Value::as_array)
        .is_some_and(|risks| risks.iter().any(|risk| risk == "tools.allow_sudo=true")));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn execute_missing_file_returns_structured_error_kind() {
    let root = temp_root("missing_file_error");
    let path = root.join("missing.toml");
    let err = execute(json!({ "path": path.display().to_string() })).expect_err("error");

    assert_eq!(err.kind, "not_found");
    assert_eq!(
        err.extra
            .as_ref()
            .and_then(|extra| extra.get("error_kind"))
            .and_then(Value::as_str),
        Some("not_found")
    );
    let _ = std::fs::remove_dir_all(root);
}
