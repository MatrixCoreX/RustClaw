use super::{execute_builtin_skill, parse_run_cmd_suggestion_payload};
use crate::{
    runtime::state::AppState, AgentRuntimeConfig, SkillViewsSnapshot, ToolsPolicy, DEFAULT_AGENT_ID,
};
use claw_core::config::{AgentConfig, ToolsConfig};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let mut path = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time before unix epoch")
            .as_nanos();
        path.push(format!(
            "clawd_builtin_skill_{prefix}_{}_{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn test_state(workspace_root: PathBuf) -> AppState {
    let skills_list = Arc::new(
        ["list_dir"]
            .into_iter()
            .map(str::to_string)
            .collect::<HashSet<_>>(),
    );
    let agents_by_id = HashMap::from([(
        DEFAULT_AGENT_ID.to_string(),
        AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
    )]);
    AppState {
        core: crate::CoreServices {
            agents_by_id: Arc::new(agents_by_id),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                registry: None,
                skills_list,
            }))),
            ..crate::CoreServices::test_default()
        },
        skill_rt: crate::SkillRuntime {
            workspace_root: workspace_root.clone(),
            default_locator_search_dir: workspace_root,
            locator_scan_max_files: 200,
            tools_policy: Arc::new(
                ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
            ),
            ..crate::SkillRuntime::test_default()
        },
        policy: crate::PolicyConfig::test_default(),
        worker: crate::WorkerConfig::test_default(),
        metrics: crate::TaskMetricsRegistry::default(),
        channels: crate::ChannelConfig::default(),
        reload_ctx: crate::ReloadContext::default(),
        ask_states: crate::AskStateRegistry::default(),
    }
}

#[tokio::test]
async fn list_dir_accepts_names_only_arg() {
    let root = TempDirGuard::new("list_dir_names_only");
    fs::write(root.path.join("b.txt"), "b").expect("write b");
    fs::write(root.path.join("a.txt"), "a").expect("write a");

    let state = test_state(root.path.clone());
    let output = execute_builtin_skill(
        &state,
        "list_dir",
        &json!({"path": ".", "names_only": true}),
    )
    .await
    .expect("list_dir should succeed");

    assert_eq!(output, "a.txt\nb.txt");
}

#[tokio::test]
async fn list_dir_accepts_structured_limit_arg() {
    let root = TempDirGuard::new("list_dir_limit");
    fs::write(root.path.join("c.txt"), "c").expect("write c");
    fs::write(root.path.join("a.txt"), "a").expect("write a");
    fs::write(root.path.join("b.txt"), "b").expect("write b");

    let state = test_state(root.path.clone());
    let output = execute_builtin_skill(&state, "list_dir", &json!({"path": ".", "limit": 2}))
        .await
        .expect("list_dir should succeed");

    assert_eq!(output, "a.txt\nb.txt");
}

#[tokio::test]
async fn write_file_append_preserves_existing_content() {
    let root = TempDirGuard::new("write_file_append");
    let path = root.path.join("notes/memo.txt");
    fs::create_dir_all(path.parent().expect("parent")).expect("create notes");
    fs::write(&path, "alpha\n").expect("write initial file");

    let state = test_state(root.path.clone());
    let output = execute_builtin_skill(
        &state,
        "write_file",
        &json!({
            "path": "notes/memo.txt",
            "content": "beta\n",
            "append": true
        }),
    )
    .await
    .expect("append write should succeed");

    assert!(output.starts_with("appended "));
    assert_eq!(
        fs::read_to_string(path).expect("read file"),
        "alpha\nbeta\n"
    );
}

#[tokio::test]
async fn write_file_append_line_separates_existing_non_newline_tail() {
    let root = TempDirGuard::new("write_file_append_line_separator");
    let path = root.path.join("notes/memo.txt");
    fs::create_dir_all(path.parent().expect("parent")).expect("create notes");
    fs::write(&path, "alpha").expect("write initial file");

    let state = test_state(root.path.clone());
    execute_builtin_skill(
        &state,
        "write_file",
        &json!({
            "path": "notes/memo.txt",
            "content": "beta\n",
            "append": true
        }),
    )
    .await
    .expect("append write should succeed");

    assert_eq!(
        fs::read_to_string(path).expect("read file"),
        "alpha\nbeta\n"
    );
}

#[tokio::test]
async fn list_dir_missing_locator_is_error_not_success_observation() {
    let root = TempDirGuard::new("list_dir_missing_locator");
    let state = test_state(root.path.clone());
    let err = execute_builtin_skill(
        &state,
        "list_dir",
        &json!({"path": "definitely_missing_directory"}),
    )
    .await
    .expect_err("missing directory should fail");

    let structured =
        crate::skills::parse_structured_skill_error(&err).expect("structured list_dir error");
    assert_eq!(structured.skill, "list_dir");
    assert_eq!(structured.error_kind, "not_found");
    assert!(structured
        .error_text
        .contains("directory not found under system root and project root"));
    assert_eq!(
        structured
            .extra
            .as_ref()
            .and_then(|extra| extra.get("requested_path"))
            .and_then(|value| value.as_str()),
        Some("definitely_missing_directory")
    );
    assert!(crate::skills::is_recoverable_skill_error("list_dir", &err));
}

#[tokio::test]
async fn list_dir_file_target_returns_structured_not_a_directory() {
    let root = TempDirGuard::new("list_dir_file_target");
    fs::write(root.path.join("target.txt"), "x").expect("write target");
    let state = test_state(root.path.clone());

    let err = execute_builtin_skill(&state, "list_dir", &json!({"path": "target.txt"}))
        .await
        .expect_err("file target should fail");

    let structured =
        crate::skills::parse_structured_skill_error(&err).expect("structured list_dir error");
    assert_eq!(structured.skill, "list_dir");
    assert_eq!(structured.error_kind, "not_a_directory");
    assert!(crate::skills::is_recoverable_skill_error("list_dir", &err));
}

#[tokio::test]
async fn remove_file_missing_path_is_structured_but_not_recoverable() {
    let root = TempDirGuard::new("remove_file_missing_path");
    let state = test_state(root.path.clone());

    let err = execute_builtin_skill(&state, "remove_file", &json!({"path": "missing.txt"}))
        .await
        .expect_err("missing remove target should fail");

    let structured =
        crate::skills::parse_structured_skill_error(&err).expect("structured remove_file error");
    assert_eq!(structured.skill, "remove_file");
    assert_eq!(structured.error_kind, "not_found");
    assert!(!crate::skills::is_recoverable_skill_error(
        "remove_file",
        &err
    ));
}

#[tokio::test]
async fn remove_file_keeps_directory_delete_explicit() {
    let root = TempDirGuard::new("remove_file_directory_explicit");
    fs::create_dir_all(root.path.join("scratch")).expect("create scratch");
    fs::write(root.path.join("scratch/note.txt"), "alpha").expect("write scratch file");
    let state = test_state(root.path.clone());

    let err = execute_builtin_skill(&state, "remove_file", &json!({"path": "scratch"}))
        .await
        .expect_err("directory target should require explicit recursive directory args");
    let structured =
        crate::skills::parse_structured_skill_error(&err).expect("structured remove_file error");
    assert_eq!(structured.skill, "remove_file");
    assert_eq!(structured.error_kind, "is_directory");

    let output = execute_builtin_skill(
        &state,
        "remove_file",
        &json!({"path": "scratch", "target_kind": "directory", "recursive": true}),
    )
    .await
    .expect("explicit recursive directory delete");

    assert!(output.contains(root.path.join("scratch").to_string_lossy().as_ref()));
    assert!(!root.path.join("scratch").exists());
}

#[tokio::test]
async fn run_cmd_accepts_timeout_seconds_override() {
    let root = TempDirGuard::new("run_cmd_timeout_override");
    let state = test_state(root.path.clone());
    let output = execute_builtin_skill(
        &state,
        "run_cmd",
        &json!({
            "command": "printf ok",
            "timeout_seconds": 1,
            "idle_timeout_seconds": 1,
            "max_output_bytes": 8000
        }),
    )
    .await
    .expect("run_cmd should succeed");

    assert_eq!(output, "ok");
}

#[tokio::test]
async fn run_safe_command_idle_timeout_kills_silent_command() {
    let root = TempDirGuard::new("run_cmd_idle_timeout");
    let err = super::run_safe_command(&root.path, "sleep 2", 4096, 10, 1, 8000, false)
        .await
        .expect_err("silent command should hit idle timeout");

    assert!(err.contains("idle timed out"), "unexpected error: {err}");
}

#[tokio::test]
async fn run_cmd_nonzero_exit_returns_structured_error() {
    let root = TempDirGuard::new("run_cmd_structured_nonzero");
    let state = test_state(root.path.clone());
    let err = execute_builtin_skill(
        &state,
        "run_cmd",
        &json!({
            "command": "printf problem >&2; exit 7",
            "timeout_seconds": 10,
            "idle_timeout_seconds": 10,
            "max_output_bytes": 8000
        }),
    )
    .await
    .expect_err("non-zero command should fail");

    let structured =
        crate::skills::parse_structured_skill_error(&err).expect("structured run_cmd error");
    assert_eq!(structured.skill, "run_cmd");
    assert_eq!(structured.error_kind, "nonzero_exit");
    assert!(structured.error_text.contains("exit code 7"));
    assert_eq!(
        structured
            .extra
            .as_ref()
            .and_then(|extra| extra.get("exit_code"))
            .and_then(|value| value.as_i64()),
        Some(7)
    );
    assert_eq!(
        structured
            .extra
            .as_ref()
            .and_then(|extra| extra.get("exit_category"))
            .and_then(|value| value.as_str()),
        Some("command_reported_failure")
    );
    assert_eq!(
        structured
            .extra
            .as_ref()
            .and_then(|extra| extra.get("exit_classification_source"))
            .and_then(|value| value.as_str()),
        Some("exit_code")
    );
    assert_eq!(
        structured
            .extra
            .as_ref()
            .and_then(|extra| extra.get("stderr"))
            .and_then(|value| value.as_str()),
        Some("problem")
    );
}

#[tokio::test]
async fn run_cmd_sudo_policy_error_stays_policy_block() {
    let root = TempDirGuard::new("run_cmd_policy_sudo");
    let state = test_state(root.path.clone());
    let err = execute_builtin_skill(
        &state,
        "run_cmd",
        &json!({
            "command": "sudo id",
            "timeout_seconds": 10,
            "idle_timeout_seconds": 10,
            "max_output_bytes": 8000
        }),
    )
    .await
    .expect_err("sudo should be policy blocked");

    assert!(
        crate::skills::parse_policy_block_error(&err).is_some(),
        "policy block should stay parseable: {err}"
    );
    assert!(
        crate::skills::parse_structured_skill_error(&err).is_none(),
        "policy block should not be wrapped as run_cmd command failure"
    );
}

#[tokio::test]
async fn run_cmd_command_not_found_uses_exit_code_category() {
    let root = TempDirGuard::new("run_cmd_exit_category_127");
    let state = test_state(root.path.clone());
    let err = execute_builtin_skill(
        &state,
        "run_cmd",
        &json!({
            "command": "definitely_missing_rustclaw_command_for_exit_category",
            "timeout_seconds": 10,
            "idle_timeout_seconds": 10,
            "max_output_bytes": 8000
        }),
    )
    .await
    .expect_err("missing command should fail");

    let structured =
        crate::skills::parse_structured_skill_error(&err).expect("structured run_cmd error");
    assert_eq!(structured.error_kind, "nonzero_exit");
    assert_eq!(
        structured
            .extra
            .as_ref()
            .and_then(|extra| extra.get("exit_code"))
            .and_then(|value| value.as_i64()),
        Some(127)
    );
    assert_eq!(
        structured
            .extra
            .as_ref()
            .and_then(|extra| extra.get("exit_category"))
            .and_then(|value| value.as_str()),
        Some("command_not_found")
    );
}

#[tokio::test]
async fn run_safe_command_truncates_noisy_command_output() {
    let root = TempDirGuard::new("run_cmd_output_limit");
    let output = super::run_safe_command(
        &root.path,
        "python3 - <<'PY'\nprint('A' * 2000)\nPY",
        4096,
        10,
        10,
        128,
        false,
    )
    .await
    .expect("noisy command should return truncated output");

    assert!(output.ends_with("..."), "missing ellipsis: {output:?}");
    assert!(
        output.len() <= 132,
        "output should be bounded, len={}: {output:?}",
        output.len()
    );
}

#[test]
fn detached_background_detection_ignores_common_redirections() {
    assert!(super::looks_detached_background_command(
        "python3 -m http.server 64884 --bind 127.0.0.1 > /dev/null 2>&1 &"
    ));
    assert!(!super::looks_detached_background_command(
        "curl -s http://127.0.0.1:8787/ >/dev/null 2>&1"
    ));
    assert!(super::looks_detached_background_command(
        "nohup python3 -m http.server 64884 >/tmp/demo.log 2>&1 & disown"
    ));
    assert!(!super::looks_detached_background_command(
        "python3 -m http.server 64884 --bind 127.0.0.1 > /dev/null 2>&1 & sleep 1 && curl -s http://127.0.0.1:64884/"
    ));
}

#[tokio::test]
async fn run_safe_command_detaches_background_http_server() {
    let root = TempDirGuard::new("run_cmd_detach_http_server");
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind temp port");
    let port = listener.local_addr().expect("port").port();
    drop(listener);

    let command = format!(
        "cd {} && python3 -m http.server {port} --bind 127.0.0.1 > /dev/null 2>&1 & echo started",
        root.path.display()
    );
    let output = super::run_safe_command(&root.path, &command, 4096, 30, 30, 8000, false)
        .await
        .expect("background run_cmd should detach");
    assert!(
        output.contains("started") || output.contains("detached=1"),
        "unexpected output: {output}"
    );

    let mut connected = false;
    for _ in 0..40 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            connected = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(connected, "http server should listen on port {port}");

    let _ = std::process::Command::new("bash")
        .arg("-lc")
        .arg(format!("kill $(lsof -ti tcp:{port}) 2>/dev/null || true"))
        .status();
}

#[test]
fn run_cmd_suggestion_schema_drift() {
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../prompts/schemas/run_cmd_suggestion.schema.json"
    ))
    .expect("schema json");
    let properties = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("properties");
    for field in ["command", "confidence", "reason"] {
        assert!(properties.contains_key(field), "missing property {field}");
    }
    let required = schema
        .get("required")
        .and_then(|v| v.as_array())
        .expect("required");
    for field in ["command", "confidence", "reason"] {
        assert!(
            required.iter().any(|v| v.as_str() == Some(field)),
            "missing required field {field}"
        );
    }
}

#[test]
fn run_cmd_suggestion_schema_rejects_missing_reason() {
    let err = parse_run_cmd_suggestion_payload(r#"{"command":"pwd","confidence":0.92}"#)
        .expect_err("schema should reject missing reason");
    assert!(err.contains("missing required field `reason`"));
}

#[test]
fn run_cmd_suggestion_schema_rejects_extra_property() {
    let err = parse_run_cmd_suggestion_payload(
        r#"{"command":"pwd","confidence":0.92,"reason":"show cwd","extra":true}"#,
    )
    .expect_err("schema should reject unexpected property");
    assert!(err.contains("unexpected property `extra`"));
}
