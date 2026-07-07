use super::*;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs};

static WORKSPACE_ROOT_ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn assess_gap_defaults_to_manual_review_for_auto() {
    let args = json!({
        "request": "Add a new reusable integration",
        "mode_hint": "auto"
    });
    let (_, extra) = run_async(execute("req-1", args)).expect("assess_gap should succeed");
    assert_eq!(extra["recommended_mode"], "manual_review");
}

#[test]
fn scaffold_rejects_invalid_skill_name() {
    let args = json!({
        "action": "scaffold_external_skill",
        "skill_name": "Bad-Name",
        "capability_summary": "test"
    });
    let err = run_async(execute("req-2", args)).expect_err("invalid skill name should fail");
    assert!(err.contains("invalid skill_name"));
}

#[test]
fn scaffold_writes_expected_files() {
    let root = temp_test_root();
    let args = json!({
        "skill_name": "demo_skill",
        "capability_summary": "Summarize one narrow capability.",
        "actions": ["inspect", "repair"]
    });
    let (_, extra) = scaffold_external_skill(root.clone(), args.as_object().unwrap())
        .expect("scaffold should succeed");
    let skill_dir = root.join("external_skills").join("demo_skill");
    assert!(skill_dir.join("README.md").exists());
    assert!(skill_dir.join("Cargo.toml").exists());
    assert!(skill_dir.join("INTERFACE.md").exists());
    assert!(skill_dir.join("src/main.rs").exists());
    assert_eq!(extra["skill_name"], "demo_skill");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn scaffolded_skill_is_validate_ready_for_single_action() {
    let _guard = WORKSPACE_ROOT_ENV_LOCK.lock().expect("env lock");
    let root = temp_test_root();
    write_repo_baseline(&root, &["external_skills/demo_skill"], true);
    let args = json!({
        "skill_name": "demo_skill",
        "capability_summary": "Return a short success text for action ping.",
        "actions": ["ping"]
    });
    scaffold_external_skill(root.clone(), args.as_object().unwrap()).expect("scaffold");

    let previous_offline = env::var("CARGO_NET_OFFLINE").ok();
    env::set_var("CARGO_NET_OFFLINE", "true");
    let report = validate_external_skill(&root, "demo_skill", &["ping".to_string()])
        .expect("default scaffold should validate");
    restore_env_var("CARGO_NET_OFFLINE", previous_offline);
    assert!(report.cargo_check_ok);
    assert!(report.smoke_test_ok);
    assert_eq!(report.smoke_status, "ok");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn execute_scaffold_action_prefers_workspace_root_env() {
    let _guard = WORKSPACE_ROOT_ENV_LOCK.lock().expect("env lock");
    let root = temp_test_root();
    let previous = env::var("WORKSPACE_ROOT").ok();
    env::set_var("WORKSPACE_ROOT", &root);

    let args = json!({
        "action": "scaffold_external_skill",
        "skill_name": "env_demo_skill",
        "capability_summary": "Summarize one narrow capability.",
        "actions": ["inspect"]
    });
    let (_, extra) =
        run_async(execute("req-env-scaffold", args)).expect("scaffold action should succeed");

    assert_eq!(
        extra["skill_dir"],
        path_string(&root.join("external_skills").join("env_demo_skill"))
    );
    assert!(root
        .join("external_skills")
        .join("env_demo_skill")
        .join("src/main.rs")
        .exists());

    restore_workspace_root(previous);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn normalize_plan_keeps_paths_under_plan_root() {
    let workspace = temp_test_root();
    let plan = TemporaryFixPlan {
        summary: "demo".to_string(),
        plan_root: String::new(),
        packages: Vec::new(),
        files: vec![TemporaryFixFile {
            path: "runner.py".to_string(),
            content: "print('ok')".to_string(),
        }],
        commands: vec![TemporaryFixCommand {
            runtime: "python3".to_string(),
            script_path: "runner.py".to_string(),
            args: Vec::new(),
            cwd: Some(".".to_string()),
        }],
        notes: Vec::new(),
    };
    let normalized = normalize_plan(&workspace, "req-demo", plan).expect("plan should normalize");
    assert!(normalized.files[0]
        .path
        .starts_with("tmp/extension_manager/"));
    assert_eq!(normalized.files[0].path, normalized.commands[0].script_path);
    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn temporary_fix_execute_requires_confirm() {
    let args = json!({
        "action": "temporary_fix_execute",
        "plan": {
            "summary": "demo",
            "files": [],
            "commands": [],
            "packages": []
        }
    });
    let err = run_async(execute("req-3", args)).expect_err("confirm should be required");
    assert!(err.contains("confirm=true"));
}

#[test]
fn temporary_fix_execute_runs_generated_script() {
    let workspace = temp_test_root();
    let plan = TemporaryFixPlan {
        summary: "run one script".to_string(),
        plan_root: "tmp/extension_manager/test-plan".to_string(),
        packages: Vec::new(),
        files: vec![TemporaryFixFile {
            path: "hello.py".to_string(),
            content: "print('hello from temporary fix')".to_string(),
        }],
        commands: vec![TemporaryFixCommand {
            runtime: "python3".to_string(),
            script_path: "hello.py".to_string(),
            args: Vec::new(),
            cwd: Some(".".to_string()),
        }],
        notes: Vec::new(),
    };
    let normalized = normalize_plan(&workspace, "req-4", plan).expect("plan should normalize");
    let written = write_plan_files(&workspace, &normalized).expect("files should be written");
    assert_eq!(written.len(), 1);
    let runs = run_plan_commands(&workspace, &normalized).expect("command should run");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].exit_code, 0);
    assert_eq!(runs[0].stdout, "hello from temporary fix");
    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn parse_temporary_fix_plan_accepts_schema_valid_json_object() {
    let raw = r#"{
            "summary":"Create a disposable scaffold.",
            "notes":["manual follow-up may still be required"]
        }"#;
    let plan = parse_temporary_fix_plan_from_text(raw).expect("parse temporary fix plan");
    assert_eq!(plan.summary, "Create a disposable scaffold.");
    assert_eq!(
        plan.notes,
        vec!["manual follow-up may still be required".to_string()]
    );
}

#[test]
fn parse_temporary_fix_plan_rejects_extra_fields() {
    let raw = r#"{
            "summary":"Create a disposable scaffold.",
            "unexpected":"drift"
        }"#;
    let err = parse_temporary_fix_plan_from_text(raw).expect_err("schema should reject");
    assert!(err.contains("unexpected field"), "unexpected error: {err}");
}

#[test]
fn fallback_temporary_fix_plan_has_no_side_effect_steps() {
    let plan = fallback_temporary_fix_plan("provider_empty_content");

    assert_eq!(plan.summary, "temporary_fix_plan_dry_run_fallback");
    assert!(plan.packages.is_empty());
    assert!(plan.files.is_empty());
    assert!(plan.commands.is_empty());
    assert!(plan
        .notes
        .iter()
        .any(|note| note == "reason_code=provider_empty_content"));
    assert!(plan.notes.iter().any(|note| note == "dry_run_only=true"));
    assert!(plan
        .notes
        .iter()
        .any(|note| note == "does_not_register=true"));
}

#[test]
fn parse_permanent_extension_plan_accepts_json_object() {
    let raw = r#"{
            "skill_name":"pdf_compare",
            "capability_summary":"Compare two PDF files and summarize differences.",
            "actions":["compare","summarize"],
            "rationale":"Reusable document comparison capability."
        }"#;
    let plan = parse_permanent_extension_plan_from_text(raw).expect("parse permanent plan");
    assert_eq!(plan.skill_name, "pdf_compare");
    assert_eq!(plan.actions, vec!["compare", "summarize"]);
}

#[test]
fn parse_permanent_extension_plan_rejects_extra_fields() {
    let raw = r#"{
            "skill_name":"pdf_compare",
            "capability_summary":"Compare two PDF files and summarize differences.",
            "rationale":"Reusable document comparison capability.",
            "unexpected":"drift"
        }"#;
    let err = parse_permanent_extension_plan_from_text(raw).expect_err("schema should reject");
    assert!(err.contains("unexpected field"), "unexpected error: {err}");
}

#[test]
fn parse_external_skill_implementation_accepts_json_object() {
    let raw = r##"{
            "readme_md":"# demo\n\nGenerated.",
            "interface_md":"# demo Interface Spec\n\n## Capability Summary\n- demo",
            "main_rs":"fn main() {}"
        }"##;
    let implementation =
        parse_external_skill_implementation_from_text(raw).expect("parse implementation");
    assert!(implementation.readme_md.contains("Generated"));
    assert!(implementation.main_rs.contains("fn main"));
}

#[test]
fn parse_external_skill_implementation_rejects_missing_required_field() {
    let raw = r##"{
            "readme_md":"# demo\n\nGenerated.",
            "interface_md":"# demo Interface Spec\n\n## Capability Summary\n- demo"
        }"##;
    let err = parse_external_skill_implementation_from_text(raw).expect_err("schema should reject");
    assert!(
        err.contains("missing required field"),
        "unexpected error: {err}"
    );
}

#[test]
fn parse_external_skill_implementation_rejects_extra_fields() {
    let raw = r##"{
            "readme_md":"# demo\n\nGenerated.",
            "interface_md":"# demo Interface Spec\n\n## Capability Summary\n- demo",
            "main_rs":"fn main() {}",
            "unexpected":"drift"
        }"##;
    let err = parse_external_skill_implementation_from_text(raw).expect_err("schema should reject");
    assert!(err.contains("unexpected field"), "unexpected error: {err}");
}

#[test]
fn implement_external_skill_writes_generated_files() {
    let root = temp_test_root();
    let args = json!({
        "skill_name": "demo_skill",
        "capability_summary": "Summarize one narrow capability.",
        "actions": ["inspect", "repair"]
    });
    scaffold_external_skill(root.clone(), args.as_object().unwrap()).expect("scaffold");

    let skill_dir = root.join("external_skills").join("demo_skill");
    let implementation = ExternalSkillImplementation {
        readme_md: "# demo_skill\n\nImplemented.".to_string(),
        interface_md: "# demo_skill Interface Spec\n\n## Capability Summary\n- Implemented."
            .to_string(),
        main_rs: "fn main() {}".to_string(),
    };
    let written = write_external_skill_implementation(
        &skill_dir,
        "demo_skill",
        "Summarize one narrow capability.",
        &["inspect".to_string(), "repair".to_string()],
        &implementation,
    )
    .expect("implementation should be written");
    assert_eq!(written.len(), 3);
    assert_eq!(
        fs::read_to_string(skill_dir.join("README.md")).expect("read README"),
        implementation.readme_md
    );
    assert_eq!(
        fs::read_to_string(skill_dir.join("INTERFACE.md")).expect("read INTERFACE"),
        implementation.interface_md
    );
    assert_eq!(
        fs::read_to_string(skill_dir.join("src/main.rs")).expect("read main"),
        implementation.main_rs
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn implement_external_skill_refuses_to_overwrite_non_scaffold_files() {
    let root = temp_test_root();
    let args = json!({
        "skill_name": "demo_skill",
        "capability_summary": "Summarize one narrow capability.",
        "actions": ["inspect", "repair"]
    });
    scaffold_external_skill(root.clone(), args.as_object().unwrap()).expect("scaffold");

    let skill_dir = root.join("external_skills").join("demo_skill");
    fs::write(skill_dir.join("README.md"), "# user edited\n").expect("should modify readme");
    let implementation = ExternalSkillImplementation {
        readme_md: "# demo_skill\n\nImplemented.".to_string(),
        interface_md: "# demo_skill Interface Spec\n\n## Capability Summary\n- Implemented."
            .to_string(),
        main_rs: "fn main() {}".to_string(),
    };
    let err = write_external_skill_implementation(
        &skill_dir,
        "demo_skill",
        "Summarize one narrow capability.",
        &["inspect".to_string(), "repair".to_string()],
        &implementation,
    )
    .expect_err("user-edited files should not be overwritten");
    assert!(err.contains("refusing to overwrite non-scaffold file"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn parse_single_json_line_accepts_single_line_object() {
    let parsed = parse_single_json_line(r#"{"request_id":"demo","status":"ok","text":"done"}"#)
        .expect("single json line should parse");
    assert_eq!(parsed["status"], "ok");
}

#[test]
fn parse_single_json_line_rejects_multi_line_noise() {
    let parsed = parse_single_json_line(
        "warning: build output\n{\"request_id\":\"demo\",\"status\":\"ok\",\"text\":\"done\"}",
    );
    assert!(parsed.is_none());
}

#[test]
fn add_workspace_member_text_inserts_external_skill_once() {
    let raw = "[workspace]\nmembers = [\n    \"crates/clawd\",\n]\n";
    let (updated, changed) =
        add_workspace_member_text(raw, "external_skills/demo_skill").expect("insert member");
    assert!(changed);
    assert!(updated.contains("\"external_skills/demo_skill\","));
    let (_, changed_again) =
        add_workspace_member_text(&updated, "external_skills/demo_skill").expect("idempotent");
    assert!(!changed_again);
}

#[test]
fn add_registry_entry_text_appends_conservative_runner_entry() {
    let raw = "[[skills]]\nname = \"clawd\"\n";
    let (updated, changed) = add_registry_entry_text(raw, "demo_skill");
    assert!(changed);
    assert!(updated.contains("name = \"demo_skill\""));
    assert!(updated.contains("planner_kind = \"skill\""));
    assert!(updated.contains("description = \"External skill demo_skill"));
    assert!(updated.contains("semantic_tags = []"));
    assert!(updated.contains("requires_confirmation = true"));
}

#[test]
fn upsert_skill_switches_line_updates_existing_switches() {
    let raw =
        "[skills]\nskill_switches = { extension_manager = false }\nskills_list = [\"run_cmd\"]\n";
    let mut switches = collect_skill_switches_from_text(raw);
    switches.insert("demo_skill".to_string(), true);
    let rendered = render_switches_inline_table(&switches);
    let updated = upsert_skill_switches_line(raw, &rendered);
    assert!(updated.contains("demo_skill = true"));
    assert!(updated.contains("extension_manager = false"));
}

#[test]
fn validate_external_skill_runs_sync_check_and_smoke_test() {
    let root = temp_test_root();
    write_repo_baseline(&root, &["external_skills/demo_skill"], true);
    write_protocol_smoke_skill(&root, "demo_skill");

    let report = validate_external_skill(&root, "demo_skill", &["inspect".to_string()])
        .expect("validate should succeed");
    assert!(report.synced_docs);
    assert!(report.cargo_check_ok);
    assert!(report.smoke_test_ok);
    assert_eq!(report.smoke_status, "ok");
    assert_eq!(report.smoke_text, "smoke ok");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn register_external_skill_updates_workspace_registry_and_switches() {
    let root = temp_test_root();
    write_repo_baseline(&root, &[], false);

    let first = register_external_skill(&root, "demo_skill").expect("register should succeed");
    assert!(first.workspace_member_added);
    assert!(first.registry_entry_added);
    assert!(first.switch_recorded_enabled);
    assert!(!first.matrix_admission_eligible);

    let cargo_toml = fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    assert!(cargo_toml.contains("\"external_skills/demo_skill\","));

    let registry = fs::read_to_string(root.join("configs/skills_registry.toml"))
        .expect("read skills_registry.toml");
    assert!(registry.contains("name = \"demo_skill\""));
    assert!(registry.contains("planner_kind = \"skill\""));
    assert!(registry.contains("requires_confirmation = true"));
    assert!(registry.contains("matrix_admission = { eligible = false"));

    let config = fs::read_to_string(root.join("configs/config.toml")).expect("read config");
    assert!(config.contains("demo_skill = true"));

    let second =
        register_external_skill(&root, "demo_skill").expect("second register should succeed");
    assert!(!second.workspace_member_added);
    assert!(!second.registry_entry_added);
    assert!(!second.switch_recorded_enabled);
    assert!(!second.matrix_admission_eligible);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn register_external_skill_rolls_back_when_config_write_fails() {
    let root = temp_test_root();
    write_repo_baseline(&root, &[], false);

    let config_path = root.join("configs/config.toml");
    let original_config = fs::read_to_string(&config_path).expect("read config");
    let mut perms = fs::metadata(&config_path)
        .expect("config metadata")
        .permissions();
    perms.set_readonly(true);
    fs::set_permissions(&config_path, perms).expect("set config readonly");

    let err = register_external_skill(&root, "demo_skill")
        .expect_err("register should fail when config write fails");
    assert!(err.contains("rolled back prior workspace metadata changes"));

    let cargo_toml = fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    assert!(!cargo_toml.contains("\"external_skills/demo_skill\","));

    let registry = fs::read_to_string(root.join("configs/skills_registry.toml"))
        .expect("read skills_registry.toml");
    assert!(!registry.contains("name = \"demo_skill\""));

    let config = fs::read_to_string(&config_path).expect("read config after failure");
    assert_eq!(config, original_config);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn execute_register_action_prefers_workspace_root_env() {
    let _guard = WORKSPACE_ROOT_ENV_LOCK.lock().expect("env lock");
    let root = temp_test_root();
    write_repo_baseline(&root, &[], false);
    write_protocol_smoke_skill(&root, "env_demo_skill");

    let previous = env::var("WORKSPACE_ROOT").ok();
    env::set_var("WORKSPACE_ROOT", &root);
    let register_args = json!({
        "action": "register_external_skill",
        "confirm": true,
        "skill_name": "env_demo_skill"
    });
    let (_, extra) = run_async(execute("req-env-register", register_args))
        .expect("register action should succeed");

    assert_eq!(extra["skill_name"], "env_demo_skill");
    assert_eq!(extra["default_enabled"], true);
    assert_eq!(extra["release_build_ok"], true);
    let cargo_toml = fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    assert!(cargo_toml.contains("\"external_skills/env_demo_skill\","));
    let config = fs::read_to_string(root.join("configs/config.toml")).expect("read config");
    assert!(config.contains("env_demo_skill = true"));

    restore_workspace_root(previous);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn enable_external_skill_builds_release_binary_and_enables_switch() {
    let root = temp_test_root();
    write_repo_baseline(&root, &["external_skills/demo_skill"], false);
    write_protocol_smoke_skill(&root, "demo_skill");

    let report =
        enable_external_skill(&root, "demo_skill").expect("enable should build successfully");
    assert!(report.switch_enabled);
    assert!(report.release_build_ok);
    assert!(report.reload_required);
    assert!(PathBuf::from(&report.release_binary_path).exists());

    let config = fs::read_to_string(root.join("configs/config.toml")).expect("read config");
    assert!(config.contains("demo_skill = true"));

    let second = enable_external_skill(&root, "demo_skill")
        .expect("second enable should still build successfully");
    assert!(!second.switch_enabled);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn enable_external_skill_ignores_workspace_level_patch_noise() {
    let root = temp_test_root();
    write_repo_baseline(&root, &["external_skills/demo_skill"], false);
    let cargo_toml_path = root.join("Cargo.toml");
    let mut cargo_toml = fs::read_to_string(&cargo_toml_path).expect("read Cargo.toml");
    cargo_toml.push_str("\n[patch.crates-io]\nopen-lark = { path = \"patches/open-lark\" }\n");
    fs::write(&cargo_toml_path, cargo_toml).expect("write Cargo.toml");
    write_protocol_smoke_skill(&root, "demo_skill");

    let report = enable_external_skill(&root, "demo_skill")
        .expect("enable should build from isolated staging dir");
    assert!(report.release_build_ok);
    assert!(PathBuf::from(&report.release_binary_path).exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn enable_external_skill_rolls_back_binary_when_config_write_fails() {
    let root = temp_test_root();
    write_repo_baseline(&root, &["external_skills/demo_skill"], false);
    write_protocol_smoke_skill(&root, "demo_skill");

    let config_path = root.join("configs/config.toml");
    let original_config = fs::read_to_string(&config_path).expect("read config");
    let mut perms = fs::metadata(&config_path)
        .expect("config metadata")
        .permissions();
    perms.set_readonly(true);
    fs::set_permissions(&config_path, perms).expect("set config readonly");

    let err = enable_external_skill(&root, "demo_skill")
        .expect_err("enable should fail when config write fails");
    assert!(err.contains("rolled back release binary"));

    let config = fs::read_to_string(&config_path).expect("read config after failure");
    assert_eq!(config, original_config);
    assert!(!config.contains("demo_skill = true"));

    let binary_path = root.join("target/release/demo-skill-skill");
    assert!(!binary_path.exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn external_skill_flow_reaches_enable_after_scaffold_and_implement() {
    let _guard = WORKSPACE_ROOT_ENV_LOCK.lock().expect("env lock");
    let root = temp_test_root();
    write_repo_baseline(&root, &[], true);
    let args = json!({
        "skill_name": "flow_demo_skill",
        "capability_summary": "Reply to ping with a short grounded success message.",
        "actions": ["ping"]
    });
    scaffold_external_skill(root.clone(), args.as_object().unwrap()).expect("scaffold");

    let skill_dir = root.join("external_skills").join("flow_demo_skill");
    let implementation = ExternalSkillImplementation {
            readme_md: "# flow_demo_skill\n\nGenerated ping demo.\n".to_string(),
            interface_md: "# flow_demo_skill Interface Spec\n\n## Capability Summary\n- Reply to ping with a short grounded success message.\n\n## Actions\n### ping\n- Required args: none\n- Optional args: none\n".to_string(),
            main_rs: protocol_smoke_main_rs("flow enabled ok"),
        };
    write_external_skill_implementation(
        &skill_dir,
        "flow_demo_skill",
        "Reply to ping with a short grounded success message.",
        &["ping".to_string()],
        &implementation,
    )
    .expect("implementation should be written");

    let previous_offline = env::var("CARGO_NET_OFFLINE").ok();
    env::set_var("CARGO_NET_OFFLINE", "true");
    let validation_result =
        validate_external_skill(&root, "flow_demo_skill", &["ping".to_string()]);
    let validation = match validation_result {
        Ok(report) => report,
        Err(err) => {
            restore_env_var("CARGO_NET_OFFLINE", previous_offline);
            panic!("validate should succeed: {err}");
        }
    };
    assert!(validation.cargo_check_ok);
    assert!(validation.smoke_test_ok);

    let registration =
        register_external_skill(&root, "flow_demo_skill").expect("register should succeed");
    assert!(registration.workspace_member_added);
    assert!(registration.registry_entry_added);
    assert!(registration.switch_recorded_enabled);

    let enable_result = enable_external_skill(&root, "flow_demo_skill");
    restore_env_var("CARGO_NET_OFFLINE", previous_offline);
    let enable = enable_result.expect("enable should succeed");
    assert!(enable.release_build_ok);
    assert!(PathBuf::from(&enable.release_binary_path).exists());

    let cargo_toml = fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    assert!(cargo_toml.contains("\"external_skills/flow_demo_skill\","));
    let registry =
        fs::read_to_string(root.join("configs/skills_registry.toml")).expect("read registry");
    assert!(registry.contains("name = \"flow_demo_skill\""));
    let config = fs::read_to_string(root.join("configs/config.toml")).expect("read config");
    assert!(config.contains("flow_demo_skill = true"));

    let _ = fs::remove_dir_all(root);
}

fn temp_test_root() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should move forward")
        .as_nanos();
    let root = env::temp_dir().join(format!(
        "extension-manager-skill-test-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("external_skills")).expect("temp root should be created");
    root
}

fn write_repo_baseline(root: &Path, workspace_members: &[&str], with_sync_script: bool) {
    let members = workspace_members
        .iter()
        .map(|member| format!("    \"{member}\","))
        .collect::<Vec<_>>()
        .join("\n");
    write_text(
        &root.join("Cargo.toml"),
        &format!("[workspace]\nmembers = [\n{members}\n]\nresolver = \"2\"\n"),
    );
    write_text(&root.join("configs/skills_registry.toml"), "");
    write_text(
        &root.join("configs/config.toml"),
        "[skills]\nskill_switches = { extension_manager = false }\nskills_list = [\"run_cmd\"]\n",
    );
    if with_sync_script {
        write_text(
            &root.join("scripts/sync_skill_docs.py"),
            "print('sync ok')\n",
        );
    }
}

fn write_protocol_smoke_skill(root: &Path, skill_name: &str) {
    let binary_name = format!("{}-skill", skill_name.replace('_', "-"));
    write_text(
        &root
            .join("external_skills")
            .join(skill_name)
            .join("README.md"),
        &format!("# {skill_name}\n\nProtocol smoke-test external skill.\n"),
    );
    write_text(
            &root
                .join("external_skills")
                .join(skill_name)
                .join("INTERFACE.md"),
            &format!(
                "# {skill_name} Interface Spec\n\n## Capability Summary\n- Protocol smoke-test external skill.\n\n## Actions\n- `inspect`: smoke action.\n"
            ),
        );
    write_text(
            &root.join("external_skills").join(skill_name).join("Cargo.toml"),
            &format!(
                "[package]\nname = \"{binary_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[[bin]]\nname = \"{binary_name}\"\npath = \"src/main.rs\"\n"
            ),
        );
    write_text(
        &root
            .join("external_skills")
            .join(skill_name)
            .join("src/main.rs"),
        &protocol_smoke_main_rs("smoke ok"),
    );
}

fn protocol_smoke_main_rs(text: &str) -> String {
    let escaped_text = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        r#"use std::io::{{self, Read}};

fn extract_request_id(raw: &str) -> String {{
    let marker = "\"request_id\":\"";
    if let Some(start) = raw.find(marker) {{
        let rest = &raw[start + marker.len()..];
        if let Some(end) = rest.find('"') {{
            return rest[..end].to_string();
        }}
    }}
    "unknown".to_string()
}}

fn main() {{
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let request_id = extract_request_id(&input);
    println!(
        "{{{{\"request_id\":\"{{}}\",\"status\":\"ok\",\"text\":\"{escaped_text}\",\"error_text\":null}}}}",
        request_id
    );
}}
"#
    )
}

fn write_text(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should exist");
    }
    fs::write(path, content).expect("write should succeed");
}

fn restore_workspace_root(previous: Option<String>) {
    restore_env_var("WORKSPACE_ROOT", previous);
}

fn restore_env_var(key: &str, previous: Option<String>) {
    if let Some(value) = previous {
        env::set_var(key, value);
    } else {
        env::remove_var(key);
    }
}

fn run_async<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime should build")
        .block_on(future)
}
