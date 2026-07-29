use super::*;

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra("execution_failed");

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_code"], "execution_failed");
    assert!(extra.get("error_kind").is_none());
    assert_eq!(
        extra["message_key"],
        "skill.install_module.execution_failed"
    );
    assert_eq!(extra["retryable"], false);
}

#[test]
fn dry_run_python_module_returns_structured_plan_without_installing() {
    let root = tempfile::tempdir().expect("temp workspace");
    let (text, extra) = install_modules(
        serde_json::json!({
            "modules": ["requests"],
            "ecosystem": "python",
            "scope": "tool_cache",
            "dry_run": true
        }),
        Some(&serde_json::json!({"workspace_root": root.path()})),
    )
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
    assert!(extra
        .get("commands")
        .and_then(serde_json::Value::as_array)
        .and_then(|commands| commands.first())
        .and_then(serde_json::Value::as_str)
        .is_some_and(
            |command| command.contains("pip install --target") && command.ends_with(" requests")
        ));
    assert_eq!(extra["scope"], "tool_cache");
    assert_eq!(extra["would_write"], false);
    assert!(extra["target_files"][0]
        .as_str()
        .is_some_and(|path| path.contains("data/tool-cache/modules/python/requests/latest")));
    assert!(!root.path().join("data/tool-cache").exists());
}

#[test]
fn preview_install_action_is_always_non_mutating() {
    let root = tempfile::tempdir().expect("temp workspace");
    let (text, extra) = install_modules(
        serde_json::json!({
            "action": "preview_install",
            "modules": ["requests"],
            "ecosystem": "python",
            "scope": "tool_cache",
            "dry_run": false
        }),
        Some(&serde_json::json!({"workspace_root": root.path()})),
    )
    .expect("preview module install");

    assert!(text.contains("action=preview_install"));
    assert!(text.contains("dry_run=true"));
    assert_eq!(extra["action"], "preview_install");
    assert_eq!(extra["dry_run"], true);
    assert_eq!(extra["modules"], serde_json::json!(["requests"]));
    assert_eq!(extra["would_write"], false);
    assert!(!root.path().join("data/tool-cache").exists());
}

#[test]
fn project_preview_targets_manifest_without_global_install_flags() {
    let root = tempfile::tempdir().expect("temp workspace");
    std::fs::write(root.path().join("package.json"), "{}").expect("manifest");

    let (_text, extra) = install_modules(
        serde_json::json!({
            "action": "preview_install",
            "module": "typescript",
            "ecosystem": "node",
            "scope": "project"
        }),
        Some(&serde_json::json!({"workspace_root": root.path()})),
    )
    .expect("project preview");

    assert_eq!(extra["scope"], "project");
    assert_eq!(
        extra["command_argv"][0],
        serde_json::json!(["npm", "install", "--save", "typescript"])
    );
    assert!(extra["commands"][0]
        .as_str()
        .is_some_and(|command| !command.contains(" -g") && !command.contains("--global")));
    assert_eq!(
        extra["target_files"][0],
        root.path().join("package.json").display().to_string()
    );
}

#[test]
fn tool_cache_plans_never_use_implicit_host_global_flags() {
    let root = tempfile::tempdir().expect("temp workspace");
    for ecosystem in ["python", "node", "rust", "go"] {
        let (_text, extra) = install_modules(
            serde_json::json!({
                "action": "preview_install",
                "module": if ecosystem == "go" { "example.com/acme/tool" } else { "demo-tool" },
                "ecosystem": ecosystem,
                "scope": "tool_cache"
            }),
            Some(&serde_json::json!({"workspace_root": root.path()})),
        )
        .expect("tool cache preview");
        let command = extra["commands"][0].as_str().expect("command");
        assert!(!command.contains(" --user "), "{command}");
        assert!(!command.contains(" -g "), "{command}");
        if ecosystem == "rust" {
            assert!(command.contains(" --root "), "{command}");
        }
        if ecosystem == "go" {
            assert!(command.starts_with("GOBIN="), "{command}");
        }
    }
}
