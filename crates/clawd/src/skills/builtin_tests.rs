use super::{execute_builtin_skill, parse_run_cmd_suggestion_payload};
use crate::{
    runtime::state::AppState, AgentRuntimeConfig, SkillViewsSnapshot, ToolsPolicy, DEFAULT_AGENT_ID,
};
use claw_core::config::{AgentConfig, ToolsConfig};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let mut path = std::env::current_dir()
            .expect("current test directory")
            .join("target/clawd-builtin-tests");
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

fn assert_workspace_mutation(output: &str, action: &str, target_path: &str) -> Value {
    let value: Value = serde_json::from_str(output).expect("structured workspace mutation");
    assert_eq!(
        value.get("source").and_then(Value::as_str),
        Some("workspace_mutation")
    );
    assert_eq!(value.get("action").and_then(Value::as_str), Some(action));
    assert_eq!(
        value.get("target_path").and_then(Value::as_str),
        Some(target_path)
    );
    assert!(value.get("checkpoint_id").and_then(Value::as_str).is_some());
    value
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

    assert_workspace_mutation(&output, "append_text", "notes/memo.txt");
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
async fn write_file_accepts_overwrite_mode_token() {
    let root = TempDirGuard::new("write_file_overwrite_mode");
    let path = root.path.join("notes/memo.txt");
    fs::create_dir_all(path.parent().expect("parent")).expect("create notes");
    fs::write(&path, "old content").expect("write initial file");

    let state = test_state(root.path.clone());
    let output = execute_builtin_skill(
        &state,
        "write_file",
        &json!({
            "path": "notes/memo.txt",
            "content": "new content\n",
            "mode": "overwrite"
        }),
    )
    .await
    .expect("overwrite mode should succeed");

    assert_workspace_mutation(&output, "write_text", "notes/memo.txt");
    assert_eq!(
        fs::read_to_string(path).expect("read file"),
        "new content\n"
    );
}

#[tokio::test]
async fn write_file_accepts_append_mode_token() {
    let root = TempDirGuard::new("write_file_append_mode");
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
            "mode": "append"
        }),
    )
    .await
    .expect("append mode should succeed");

    assert_workspace_mutation(&output, "append_text", "notes/memo.txt");
    assert_eq!(
        fs::read_to_string(path).expect("read file"),
        "alpha\nbeta\n"
    );
}

#[tokio::test]
async fn write_file_rejects_conflicting_append_and_mode_tokens() {
    let root = TempDirGuard::new("write_file_conflicting_mode");
    let state = test_state(root.path.clone());

    let err = execute_builtin_skill(
        &state,
        "write_file",
        &json!({
            "path": "notes/memo.txt",
            "content": "beta\n",
            "append": true,
            "mode": "overwrite"
        }),
    )
    .await
    .expect_err("conflicting write mode should fail");

    let structured =
        crate::skills::parse_structured_skill_error(&err).expect("structured write_file error");
    assert_eq!(structured.skill, "write_file");
    assert_eq!(structured.error_kind, "invalid_args");
}

#[tokio::test]
async fn write_file_accepts_create_parents_token() {
    let root = TempDirGuard::new("write_file_create_parents");
    let path = root.path.join("notes/deep/memo.txt");
    let state = test_state(root.path.clone());

    let output = execute_builtin_skill(
        &state,
        "write_file",
        &json!({
            "path": "notes/deep/memo.txt",
            "content": "created\n",
            "create_parents": true
        }),
    )
    .await
    .expect("create_parents write should succeed");

    assert_workspace_mutation(&output, "write_text", "notes/deep/memo.txt");
    assert_eq!(fs::read_to_string(path).expect("read file"), "created\n");
}

#[tokio::test]
async fn workspace_rewind_restores_a_checkpointed_whole_file_write() {
    let root = TempDirGuard::new("write_file_rewind");
    let path = root.path.join("notes/memo.txt");
    fs::create_dir_all(path.parent().expect("parent")).expect("create notes");
    fs::write(&path, "before\n").expect("seed file");
    let state = test_state(root.path.clone());

    let output = execute_builtin_skill(
        &state,
        "write_file",
        &json!({"path": "notes/memo.txt", "content": "after\n"}),
    )
    .await
    .expect("checkpointed write");
    let mutation = assert_workspace_mutation(&output, "write_text", "notes/memo.txt");
    let checkpoint_id = mutation
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .expect("checkpoint id");
    let rewind = execute_builtin_skill(
        &state,
        "workspace_patch",
        &json!({"action": "rewind", "checkpoint_id": checkpoint_id}),
    )
    .await
    .expect("workspace rewind");
    let rewind: Value = serde_json::from_str(&rewind).expect("rewind result");

    assert_eq!(
        rewind.get("source").and_then(Value::as_str),
        Some("workspace_mutation")
    );
    assert_eq!(
        rewind
            .get("compensates_checkpoint_id")
            .and_then(Value::as_str),
        Some(checkpoint_id)
    );
    assert_eq!(fs::read_to_string(path).expect("restored file"), "before\n");
}

#[tokio::test]
async fn write_file_respects_create_parents_false_token() {
    let root = TempDirGuard::new("write_file_without_create_parents");
    let state = test_state(root.path.clone());

    let err = execute_builtin_skill(
        &state,
        "write_file",
        &json!({
            "path": "notes/deep/memo.txt",
            "content": "created\n",
            "create_parents": false
        }),
    )
    .await
    .expect_err("missing parent without create_parents should fail");

    let structured =
        crate::skills::parse_structured_skill_error(&err).expect("structured write_file error");
    assert_eq!(structured.skill, "write_file");
    assert_eq!(structured.error_kind, "not_found");
    assert!(!root.path.join("notes").exists());
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

    assert_workspace_mutation(&output, "remove_path", "scratch");
    assert!(!root.path.join("scratch").exists());
}

#[tokio::test]
async fn make_dir_accepts_parents_machine_arg() {
    let root = TempDirGuard::new("make_dir_parents");
    let state = test_state(root.path.clone());

    let output = execute_builtin_skill(
        &state,
        "make_dir",
        &json!({"path": "nested/child", "parents": true}),
    )
    .await
    .expect("parents=true should create missing parents");

    assert_workspace_mutation(&output, "make_dir", "nested/child");
    assert!(root.path.join("nested/child").is_dir());
}

#[tokio::test]
async fn make_dir_parents_false_does_not_create_missing_parents() {
    let root = TempDirGuard::new("make_dir_no_parents");
    let state = test_state(root.path.clone());

    let err = execute_builtin_skill(
        &state,
        "make_dir",
        &json!({"path": "nested/child", "parents": false}),
    )
    .await
    .expect_err("parents=false should not create missing parents");

    let structured =
        crate::skills::parse_structured_skill_error(&err).expect("structured make_dir error");
    assert_eq!(structured.skill, "make_dir");
    assert_eq!(structured.error_kind, "not_found");
    assert!(!root.path.join("nested/child").exists());
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
async fn run_cmd_accepts_planner_action_metadata() {
    let root = TempDirGuard::new("run_cmd_planner_action_metadata");
    let state = test_state(root.path.clone());
    let output = execute_builtin_skill(
        &state,
        "run_cmd",
        &json!({
            "action": "inspect_cli_help",
            "command": "printf ok",
            "timeout_seconds": 1,
            "idle_timeout_seconds": 1,
            "max_output_bytes": 8000
        }),
    )
    .await
    .expect("run_cmd should ignore planner action metadata");

    assert_eq!(output, "ok");
}

#[cfg(unix)]
#[tokio::test]
async fn run_cmd_async_start_uses_dedicated_process_group() {
    let root = TempDirGuard::new("run_cmd_async_process_group");
    let state = test_state(root.path.clone());
    let job_dir = root.path.join("async-job");
    let output = execute_builtin_skill(
        &state,
        "run_cmd",
        &json!({
            "command": "sleep 30",
            "async_start": true,
            "_clawd_async_job_id": "local_process:test-process-group",
            "_clawd_async_job_dir": job_dir.display().to_string()
        }),
    )
    .await
    .expect("async run_cmd should start");
    assert!(output.contains("\"status\":\"accepted\""));

    let pid = fs::read_to_string(job_dir.join("pid"))
        .expect("pid file")
        .trim()
        .parse::<u32>()
        .expect("pid number");
    let mut pgid = None;
    for _ in 0..20 {
        pgid = process_group_id(pid);
        if pgid.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(pgid, Some(pid));
    let _ = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(format!("-{pid}"))
        .status();
}

#[cfg(unix)]
#[tokio::test]
async fn run_cmd_async_job_stops_at_checkpoint_expiry() {
    let root = TempDirGuard::new("run_cmd_async_expiry");
    let state = test_state(root.path.clone());
    let job_dir = root.path.join("async-job");
    let started = Instant::now();
    let output = execute_builtin_skill(
        &state,
        "run_cmd",
        &json!({
            "command": "exec sleep 30",
            "async_start": true,
            "_clawd_async_job_id": "local_process:test-expiry",
            "_clawd_async_job_dir": job_dir.display().to_string(),
            "_clawd_async_expires_at": crate::now_ts_u64().saturating_add(1)
        }),
    )
    .await
    .expect("async run_cmd should start");
    assert!(output.contains("\"status\":\"accepted\""));

    let finished_at_path = job_dir.join("finished_at");
    for _ in 0..60 {
        if finished_at_path.is_file() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let exit_code_path = job_dir.join("exit_code");
    let exit_code = fs::read_to_string(&exit_code_path)
        .expect("expiry should publish an exit code")
        .trim()
        .parse::<i32>()
        .expect("numeric exit code");
    // GNU coreutils reports 124; the supported uutils build reports 125 after sending TERM.
    assert!(
        matches!(exit_code, 124 | 125),
        "unsupported timeout exit code: {exit_code}"
    );
    assert!(started.elapsed() >= Duration::from_millis(800));
    assert!(started.elapsed() < Duration::from_secs(4));
    assert!(finished_at_path.is_file());
}

#[cfg(unix)]
fn process_group_id(pid: u32) -> Option<u32> {
    let output = std::process::Command::new("ps")
        .arg("-o")
        .arg("pgid=")
        .arg("-p")
        .arg(pid.to_string())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .ok()
}

#[tokio::test]
async fn run_safe_command_idle_timeout_kills_silent_command() {
    let root = TempDirGuard::new("run_cmd_idle_timeout");
    let err = super::run_safe_command(&root.path, "sleep 2", 4096, 10, 1, 8000, false)
        .await
        .expect_err("silent command should hit idle timeout");

    assert!(
        err.contains("run_cmd.idle_timeout"),
        "unexpected error: {err}"
    );
    assert!(err.contains("seconds=1"), "unexpected error: {err}");
}

#[tokio::test]
async fn run_safe_command_rejects_sudo_without_explicit_policy() {
    let root = TempDirGuard::new("run_cmd_sudo_policy");
    let err = super::run_safe_command(&root.path, "sudo -n true", 4096, 10, 10, 8000, false)
        .await
        .expect_err("sudo must be rejected before process dispatch");
    assert!(err.contains("sudo_not_allowed"), "unexpected error: {err}");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn run_safe_command_idle_timeout_leaves_no_live_child_process() {
    let root = TempDirGuard::new("run_cmd_idle_timeout_child_cleanup");
    let command = "sleep 30 & child=$!; printf '%s' \"$child\" > child.pid; wait \"$child\"";
    let err = super::run_safe_command(&root.path, command, 4096, 10, 1, 8000, false)
        .await
        .expect_err("silent process group should hit idle timeout");
    assert!(
        err.contains("run_cmd.idle_timeout"),
        "unexpected error: {err}"
    );

    let child_pid = fs::read_to_string(root.path.join("child.pid"))
        .expect("child pid file")
        .trim()
        .parse::<u32>()
        .expect("child pid");
    for _ in 0..50 {
        if !linux_process_is_live(child_pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let _ = std::process::Command::new("kill")
        .args(["-KILL", &child_pid.to_string()])
        .status();
    panic!("timed-out command left a live child process pid={child_pid}");
}

#[cfg(target_os = "linux")]
fn linux_process_is_live(pid: u32) -> bool {
    let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    stat.rsplit_once(") ")
        .and_then(|(_, tail)| tail.chars().next())
        .is_some_and(|state| state != 'Z' && state != 'X')
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
    assert!(structured.error_text.contains("run_cmd.nonzero_exit"));
    assert!(structured.error_text.contains("exit_code=7"));
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

#[test]
fn runtime_checkpoint_claim_detection_requires_real_background_operator() {
    assert!(super::run_cmd_claims_runtime_checkpoint_without_async_start(
        "OUTFILE=/tmp/async.out; sleep 2 > \"$OUTFILE\" 2>&1 & PID=$!; echo checkpoint_id=job-$PID; echo poll_ref=$OUTFILE; echo next_check_after=2s"
    ));
    assert!(super::run_cmd_claims_runtime_checkpoint_without_async_start(
        "nohup bash -c 'sleep 2 && echo done' >/tmp/demo.log 2>&1 & PID=$!; printf '{\"checkpoint_id\":\"ckpt-%s\",\"poll_ref\":\"pid:%s\",\"next_check_after\":2}\\n' \"$PID\" \"$PID\""
    ));
    assert!(
        !super::run_cmd_claims_runtime_checkpoint_without_async_start(
            "printf '%s\\n' 'checkpoint_id=demo poll_ref=/tmp/demo next_check_after=2s'"
        )
    );
    assert!(!super::run_cmd_claims_runtime_checkpoint_without_async_start(
        "curl 'http://127.0.0.1:8787/?a=1&b=2' && echo checkpoint_id=demo && echo poll_ref=/tmp/demo"
    ));
}

#[tokio::test]
async fn run_cmd_rejects_shell_faked_runtime_checkpoint_without_async_start() {
    let root = TempDirGuard::new("run_cmd_faked_checkpoint");
    let state = test_state(root.path.clone());

    let err = execute_builtin_skill(
        &state,
        "run_cmd",
        &json!({
            "command": "OUTFILE=/tmp/async.out; sleep 2 > \"$OUTFILE\" 2>&1 & PID=$!; echo checkpoint_id=job-$PID; echo poll_ref=$OUTFILE; echo next_check_after=2s"
        }),
    )
    .await
    .expect_err("fake checkpoint shell should be rejected");

    assert!(
        err.contains("\"error_kind\":\"async_start_required\""),
        "{err}"
    );
    assert!(
        err.contains("\"message_key\":\"clawd.run_cmd.async_start_required\""),
        "{err}"
    );
}

#[tokio::test]
async fn run_cmd_rejects_unmanaged_terminal_background_process() {
    let root = TempDirGuard::new("run_cmd_unmanaged_background");
    let state = test_state(root.path.clone());

    let err = execute_builtin_skill(
        &state,
        "run_cmd",
        &json!({
            "command": "nohup sleep 30 >/dev/null 2>&1 & disown"
        }),
    )
    .await
    .expect_err("unmanaged background process should be rejected");

    assert!(
        err.contains("\"error_kind\":\"async_start_required\""),
        "{err}"
    );
    assert!(
        err.contains("\"unmanaged_detached_background\":true"),
        "{err}"
    );
    assert!(err.contains("\"faked_runtime_checkpoint\":false"), "{err}");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn bounded_local_service_validation_runs_in_one_workspace_sandbox() {
    if !std::path::Path::new("/usr/bin/bwrap").is_file()
        && !std::path::Path::new("/bin/bwrap").is_file()
    {
        return;
    }
    let root = TempDirGuard::new("run_cmd_bounded_local_service");
    fs::write(root.path.join("index.html"), "bounded-service-ok\n").expect("write index");
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind temp port");
    let port = listener.local_addr().expect("port").port();
    drop(listener);
    let command = format!(
        "python3 -m http.server {port} --bind 127.0.0.1 >/dev/null 2>&1 & \
         pid=$!; trap 'kill \"$pid\" >/dev/null 2>&1 || true' EXIT; \
         ready=0; for _ in $(seq 1 30); do \
           if curl -fsS http://127.0.0.1:{port}/ > response.txt 2>/dev/null; then ready=1; break; fi; \
           sleep 0.1; \
         done; \
         test \"$ready\" = 1; grep -q bounded-service-ok response.txt; \
         printf VALIDATION_PASSED"
    );

    let output = super::run_safe_command_with_sandbox(
        &root.path,
        &command,
        4096,
        10,
        10,
        8000,
        false,
        claw_core::config::ToolSandboxMode::WorkspaceWrite,
        &root.path,
    )
    .await
    .expect("bounded service validation");

    assert_eq!(output, "VALIDATION_PASSED");
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
