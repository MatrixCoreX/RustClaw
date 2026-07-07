use super::*;

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
