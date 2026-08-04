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
            "agent-runtime-package-manager-{name}-{}",
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
fn dry_run_install_emits_monotonic_progress_for_every_package() {
    let mut output = Vec::new();
    let mut emitter = SkillProgressEmitter::new(&mut output, "package-progress");
    execute_with_progress(
        serde_json::json!({
            "action":"install",
            "manager":"apt-get",
            "packages":["jq","curl","git"],
            "dry_run":true,
            "use_sudo":false
        }),
        &mut emitter,
    )
    .expect("dry-run packages");
    let frames = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| {
            skill_sdk::validate_progress_frame_line(line.as_bytes(), "package-progress").unwrap()
        })
        .filter(|frame| frame.detail_key == "package_manager.install.progress")
        .collect::<Vec<_>>();
    assert_eq!(frames.len(), 4);
    assert_eq!(
        frames.iter().map(|frame| frame.current).collect::<Vec<_>>(),
        vec![Some(0), Some(1), Some(2), Some(3)]
    );
    assert!(frames.iter().all(|frame| frame.total == Some(3)));
}

#[test]
fn smart_install_preview_forces_dry_run_and_does_not_write_a_local_log() {
    let root = TempDir::new("smart-preview-no-write");

    let (_text, extra) = execute(serde_json::json!({
        "action": "smart_install_preview",
        "manager": "apt-get",
        "package": "jq",
        "dry_run": false,
        "use_sudo": false
    }))
    .expect("smart preview");

    assert_eq!(extra["action"], "smart_install_preview");
    assert_eq!(extra["dry_run"], true);
    assert!(!root.path.join("logs/install_ops.log").exists());
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

#[test]
fn failed_package_command_preserves_complete_output_behind_artifact_range() {
    let root = TempDir::new("failed-output-artifact");
    let source = "package-manager failure detail\n".repeat(1_000);
    let spill =
        ArtifactSpill::new(root.path.join("artifacts"), SKILL_NAME).expect("create artifact spill");
    let bounded_output = BoundedResult::text(&source, 64, Some(&spill), "package-command-output")
        .expect("bound command output");

    let failure = package_command_failure(
        "install",
        "apt-get",
        &["missing-package".to_string()],
        false,
        false,
        "apt-get install -y missing-package".to_string(),
        100,
        bounded_output,
    );

    assert_eq!(failure.extra["status"], "error");
    assert_eq!(failure.extra["error_code"], "package_command_failed");
    assert_eq!(failure.extra["exit_code"], 100);
    assert_eq!(failure.extra["output_result"]["complete"], false);
    assert_eq!(
        failure.extra["output_result"]["original_size_bytes"],
        source.len() as u64
    );
    let artifact_path = failure.extra["output_result"]["artifacts"][0]["path"]
        .as_str()
        .expect("artifact path");
    assert_eq!(
        std::fs::read_to_string(artifact_path).expect("read complete failure output"),
        source
    );
}
