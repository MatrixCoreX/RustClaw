use super::*;

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra("execution_failed");

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_kind"], "execution_failed");
    assert_eq!(
        extra["message_key"],
        "skill.install_module.execution_failed"
    );
    assert_eq!(extra["retryable"], false);
}

#[test]
fn dry_run_python_module_returns_structured_plan_without_installing() {
    let (text, extra) = install_modules(serde_json::json!({
        "modules": ["requests"],
        "ecosystem": "python",
        "dry_run": true
    }))
    .expect("dry-run module install");

    assert!(text.contains("skill=install_module"));
    assert!(text.contains("module=requests"));
    assert!(text.contains("dry_run=true"));
    assert_eq!(
        extra.get("skill").and_then(serde_json::Value::as_str),
        Some("install_module")
    );
    assert_eq!(
        extra.get("module").and_then(serde_json::Value::as_str),
        Some("requests")
    );
    assert_eq!(
        extra.get("dry_run").and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        extra
            .get("commands")
            .and_then(serde_json::Value::as_array)
            .and_then(|commands| commands.first())
            .and_then(serde_json::Value::as_str),
        Some("python3 -m pip install --user requests")
    );
}
