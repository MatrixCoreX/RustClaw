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
        "skill.package_manager.execution_failed"
    );
    assert_eq!(extra["retryable"], false);
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rustclaw-package-manager-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn detects_npm_project_from_package_lock() {
    let root = TempDir::new("npm");
    std::fs::write(root.path.join("package.json"), "{}").expect("write manifest");
    std::fs::write(root.path.join("package-lock.json"), "{}").expect("write lock");

    let detected = detect_project_manager(&root.path).expect("project manager");

    assert_eq!(detected.manager, "npm");
    assert_eq!(detected.marker, "package-lock.json");
}

#[test]
fn detects_cargo_project_from_manifest() {
    let root = TempDir::new("cargo");
    std::fs::write(root.path.join("Cargo.toml"), "[package]\nname=\"demo\"\n")
        .expect("write cargo manifest");

    let detected = detect_project_manager(&root.path).expect("project manager");

    assert_eq!(detected.manager, "cargo");
    assert_eq!(detected.marker, "Cargo.toml");
}

#[test]
fn detect_response_includes_machine_availability_fields() {
    let (text, extra) = execute(serde_json::json!({"action": "detect"})).expect("detect");

    assert!(text.contains("manager="));
    assert!(text.contains("available="));
    assert!(text.contains("version_present="));
    assert!(extra
        .get("manager")
        .and_then(serde_json::Value::as_str)
        .is_some());
    assert!(extra
        .get("available")
        .and_then(serde_json::Value::as_bool)
        .is_some());
    assert!(extra
        .get("version_present")
        .and_then(serde_json::Value::as_bool)
        .is_some());
}

#[test]
fn dry_run_install_accepts_structured_module_alias() {
    let (text, extra) = execute(serde_json::json!({
        "action": "install",
        "manager": "apt-get",
        "modules": ["jq"],
        "dry_run": true,
        "use_sudo": false
    }))
    .expect("dry-run install");

    assert!(text.contains("package=jq"));
    assert!(text.contains("dry_run=true"));
    assert_eq!(
        extra.get("package").and_then(serde_json::Value::as_str),
        Some("jq")
    );
    assert_eq!(
        extra
            .get("packages")
            .and_then(serde_json::Value::as_array)
            .and_then(|packages| packages.first())
            .and_then(serde_json::Value::as_str),
        Some("jq")
    );
    assert_eq!(
        extra.get("dry_run").and_then(serde_json::Value::as_bool),
        Some(true)
    );
}

#[test]
fn dry_run_uninstall_returns_machine_fields() {
    let (text, extra) = execute(serde_json::json!({
        "action": "uninstall",
        "manager": "apt-get",
        "package": "jq",
        "dry_run": true,
        "use_sudo": false
    }))
    .expect("dry-run uninstall");

    assert!(text.contains("action=uninstall"));
    assert!(text.contains("package=jq"));
    assert!(text.contains("dry_run=true"));
    assert_eq!(
        extra.get("action").and_then(serde_json::Value::as_str),
        Some("uninstall")
    );
    assert_eq!(
        extra.get("package").and_then(serde_json::Value::as_str),
        Some("jq")
    );
}
