use super::*;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs};

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra("execution_failed");

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_code"], "execution_failed");
    assert_eq!(
        extra["message_key"],
        "skill.extension_manager.execution_failed"
    );
    assert_eq!(extra["retryable"], false);
}

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
    assert!(skill_dir.join("skill.toml").exists());
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
    write_repo_baseline(&root, &[], true);
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
    assert!(report.manifest_valid);
    assert!(report.build_ok);
    assert_eq!(report.adapter, "cargo");
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
            "entrypoint_source":"fn main() {}"
        }"##;
    let implementation =
        parse_external_skill_implementation_from_text(raw).expect("parse implementation");
    assert!(implementation.readme_md.contains("Generated"));
    assert!(implementation.entrypoint_source.contains("fn main"));
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
            "entrypoint_source":"fn main() {}",
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
        entrypoint_source: "fn main() {}".to_string(),
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
        implementation.entrypoint_source
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn implement_external_skill_writes_manifest_selected_python_entrypoint() {
    let root = temp_test_root();
    let args = json!({
        "skill_name": "python_demo_skill",
        "capability_summary": "Return a small structured result.",
        "actions": ["inspect"],
        "implementation_language": "python"
    });
    scaffold_external_skill(root.clone(), args.as_object().unwrap()).expect("python scaffold");

    let skill_dir = root.join("external_skills").join("python_demo_skill");
    let implementation = ExternalSkillImplementation {
        readme_md: "# python_demo_skill\n\nImplemented.\n".to_string(),
        interface_md:
            "# python_demo_skill Interface Spec\n\n## Capability Summary\n- Implemented.\n"
                .to_string(),
        entrypoint_source: r#"import json
import sys

def respond(request: dict) -> dict:
    return {"request_id": request["request_id"], "status": "ok", "text": "", "error_text": None}

request = json.loads(sys.stdin.readline())
print(json.dumps(respond(request), separators=(",", ":")))
"#
        .to_string(),
    };

    let written = write_external_skill_implementation(
        &skill_dir,
        "python_demo_skill",
        "Return a small structured result.",
        &["inspect".to_string()],
        &implementation,
    )
    .expect("python implementation should be written");

    assert_eq!(written.len(), 3);
    assert_eq!(
        fs::read_to_string(skill_dir.join("src/main.py")).expect("read Python entrypoint"),
        implementation.entrypoint_source
    );
    assert!(!skill_dir.join("src/main.rs").exists());
    assert!(!skill_dir.join("Cargo.toml").exists());
    assert!(!root.join("Cargo.toml").exists());
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
        entrypoint_source: "fn main() {}".to_string(),
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
fn validate_external_skill_runs_sync_check_and_smoke_test() {
    let root = temp_test_root();
    write_repo_baseline(&root, &[], true);
    write_protocol_smoke_skill(&root, "demo_skill");

    let report = validate_external_skill(&root, "demo_skill", &["inspect".to_string()])
        .expect("validate should succeed");
    assert!(report.synced_docs);
    assert!(report.manifest_valid);
    assert!(report.build_ok);
    assert!(report.smoke_test_ok);
    assert_eq!(report.smoke_status, "ok");
    assert_eq!(report.adapter, "cargo");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn registration_uses_host_admission_api_without_mutating_tracked_config() {
    let _guard = WORKSPACE_ROOT_ENV_LOCK.lock().expect("env lock");
    let root = temp_test_root();
    write_repo_baseline(&root, &[], false);
    write_protocol_smoke_skill(&root, "demo_skill");
    let config_path = root.join("configs/config.toml");
    let registry_path = root.join("configs/skills_registry.toml");
    let config_before = fs::read(&config_path).expect("config baseline");
    let registry_before = fs::read(&registry_path).expect("registry baseline");
    let (url, server) = mock_admission_server();
    let previous_url = env::var("AGENT_INTERNAL_ADMISSION_URL").ok();
    let previous_token = env::var("AGENT_INTERNAL_ADMISSION_TOKEN").ok();
    env::set_var("AGENT_INTERNAL_ADMISSION_URL", url);
    env::set_var("AGENT_INTERNAL_ADMISSION_TOKEN", "test-admission-token");

    let report = run_async(register_external_skill(&root, "demo_skill"))
        .expect("host admission should succeed");
    assert_eq!(report.registry_generation, 42);
    assert_eq!(report.build_adapter, "cargo");
    assert_eq!(report.installed_version, "1.2.3");
    assert_eq!(report.receipt_digest.len(), 64);
    assert!(report.enabled);
    let request = server.join().expect("mock admission server");
    assert!(request.contains("x-agent-internal-skill-token: test-admission-token"));
    assert!(request.contains("external_skills/demo_skill"));
    assert!(request.contains("\"enabled\":true"));
    assert_eq!(fs::read(config_path).expect("config after"), config_before);
    assert_eq!(
        fs::read(registry_path).expect("registry after"),
        registry_before
    );

    restore_env_var("AGENT_INTERNAL_ADMISSION_URL", previous_url);
    restore_env_var("AGENT_INTERNAL_ADMISSION_TOKEN", previous_token);

    let _ = fs::remove_dir_all(root);
}

fn mock_admission_server() -> (String, std::thread::JoinHandle<String>) {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock admission server");
    let address = listener.local_addr().expect("mock server address");
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept admission request");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("request read timeout");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = stream.read(&mut buffer).expect("read admission request");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            let text = String::from_utf8_lossy(&request);
            let Some((headers, body)) = text.split_once("\r\n\r\n") else {
                continue;
            };
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            if body.len() >= content_length {
                break;
            }
        }
        let body = serde_json::json!({
            "ok": true,
            "data": {
                "registry_generation": 42,
                "registry_generation_digest": "a".repeat(64),
                "build_adapter": "cargo",
                "package_version": "1.2.3",
                "receipt_digest": "b".repeat(64),
                "enabled": true
            }
        })
        .to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("write admission response");
        String::from_utf8(request).expect("request is UTF-8")
    });
    (format!("http://{address}/v1/internal/skills/admit"), handle)
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
    let destination = root.join("external_skills").join(skill_name);
    skill_sdk::scaffold_skill(&skill_sdk::ScaffoldRequest {
        destination: destination.clone(),
        skill_name: skill_name.to_string(),
        capability_summary: "Protocol smoke-test external skill.".to_string(),
        actions: vec!["inspect".to_string()],
        implementation_language: skill_sdk::ImplementationLanguage::Rust,
        source_root: format!("external_skills/{skill_name}"),
    })
    .expect("write protocol smoke scaffold");
    write_text(
        &destination.join("src/main.rs"),
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
