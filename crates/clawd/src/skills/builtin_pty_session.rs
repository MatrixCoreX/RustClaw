use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tokio::process::Command;

use crate::ClaimedTask;

const SESSION_ROOT: &str = ".rustclaw/pty_sessions";
const DEFAULT_PAGE_BYTES: usize = 64 * 1024;
const MAX_PAGE_BYTES: usize = 1024 * 1024;
const MIN_SESSION_OUTPUT_BYTES: u64 = 1024;
const MAX_SESSION_OUTPUT_BYTES: u64 = 1024 * 1024 * 1024;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub(super) struct PtySessionError {
    pub(super) kind: &'static str,
    pub(super) code: &'static str,
    pub(super) extra: Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct PtyLaunchSpec {
    schema_version: u32,
    session_id: String,
    task_id: String,
    owner_user_id: i64,
    owner_chat_id: i64,
    owner_channel: String,
    program: String,
    args: Vec<String>,
    cwd: String,
    env_clear: bool,
    env: BTreeMap<String, Option<String>>,
    rows: u16,
    cols: u16,
    created_at: i64,
    expires_at: i64,
    idle_timeout_seconds: u64,
    max_output_bytes: u64,
}

#[derive(Debug, Deserialize)]
struct PtySessionMetadata {
    #[serde(default)]
    status: String,
    #[serde(default)]
    pid: Option<u32>,
    #[serde(default)]
    exit_code: Option<u32>,
    #[serde(default)]
    heartbeat_at: i64,
    #[serde(default)]
    output_bytes: u64,
    #[serde(default)]
    rows: u16,
    #[serde(default)]
    cols: u16,
    #[serde(default)]
    reason_code: Option<String>,
}

pub(super) fn is_existing_session_action(action: &str) -> bool {
    matches!(
        action,
        "terminal_write"
            | "terminal_poll"
            | "terminal_resize"
            | "terminal_signal"
            | "terminal_terminate"
    )
}

pub(super) async fn start_session(
    workspace_root: &Path,
    task: Option<&ClaimedTask>,
    prepared: &Command,
    rows: u16,
    cols: u16,
    expires_in_seconds: u64,
    idle_timeout_seconds: u64,
    max_output_bytes: u64,
) -> Result<String, PtySessionError> {
    let session_id = uuid::Uuid::new_v4().to_string();
    let session_dir = session_dir(workspace_root, &session_id)?;
    create_session_directories(workspace_root, &session_dir)?;

    let now = crate::now_ts_u64() as i64;
    let spec = launch_spec_from_command(
        prepared,
        task,
        session_id.clone(),
        rows.clamp(2, 500),
        cols.clamp(2, 1000),
        now,
        now.saturating_add(expires_in_seconds.clamp(1, 7 * 24 * 3600) as i64),
        idle_timeout_seconds.clamp(5, 24 * 3600),
        max_output_bytes.clamp(MIN_SESSION_OUTPUT_BYTES, MAX_SESSION_OUTPUT_BYTES),
    )?;
    atomic_write_json(&session_dir.join("launch.json"), &spec)
        .map_err(|err| io_error("pty_launch_spec_write_failed", &session_dir, err))?;
    spawn_session_runner(workspace_root, &session_dir).await?;

    let deadline = tokio::time::Instant::now() + CONTROL_TIMEOUT;
    loop {
        if let Some(metadata) = read_metadata(&session_dir) {
            return serde_json::to_string(&session_projection(&spec, &metadata, 0))
                .map_err(|_| machine_error("pty_session_projection_failed"));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(machine_error("pty_session_start_timeout"));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub(super) async fn execute_existing_session_action(
    workspace_root: &Path,
    task: Option<&ClaimedTask>,
    action: &str,
    args: &Map<String, Value>,
) -> Result<String, PtySessionError> {
    let session_id = required_session_id(args)?;
    let session_dir = session_dir(workspace_root, session_id)?;
    validate_existing_session_dir(workspace_root, &session_dir)?;
    let spec = read_launch_spec(&session_dir)?;
    ensure_owner(task, &spec)?;
    match action {
        "terminal_poll" => poll_session(&session_dir, &spec, args),
        "terminal_write" | "terminal_resize" | "terminal_signal" | "terminal_terminate" => {
            send_control(&session_dir, action, args).await
        }
        _ => Err(machine_error("pty_session_action_unsupported")),
    }
}

fn launch_spec_from_command(
    command: &Command,
    task: Option<&ClaimedTask>,
    session_id: String,
    rows: u16,
    cols: u16,
    created_at: i64,
    expires_at: i64,
    idle_timeout_seconds: u64,
    max_output_bytes: u64,
) -> Result<PtyLaunchSpec, PtySessionError> {
    let command = command.as_std();
    let program = os_string(command.get_program(), "pty_program_non_utf8")?;
    let args = command
        .get_args()
        .map(|arg| os_string(arg, "pty_arg_non_utf8"))
        .collect::<Result<Vec<_>, _>>()?;
    let cwd = command
        .get_current_dir()
        .ok_or_else(|| machine_error("pty_cwd_missing"))
        .and_then(|path| os_string(path.as_os_str(), "pty_cwd_non_utf8"))?;
    let mut env = BTreeMap::new();
    for (key, value) in command.get_envs() {
        env.insert(
            os_string(key, "pty_env_key_non_utf8")?,
            value
                .map(|value| os_string(value, "pty_env_value_non_utf8"))
                .transpose()?,
        );
    }
    Ok(PtyLaunchSpec {
        schema_version: 1,
        session_id,
        task_id: task.map(|task| task.task_id.clone()).unwrap_or_default(),
        owner_user_id: task.map(|task| task.user_id).unwrap_or_default(),
        owner_chat_id: task.map(|task| task.chat_id).unwrap_or_default(),
        owner_channel: task.map(|task| task.channel.clone()).unwrap_or_default(),
        program,
        args,
        cwd,
        env_clear: crate::skills::skill_runner_env_strict_enabled(),
        env,
        rows,
        cols,
        created_at,
        expires_at,
        idle_timeout_seconds,
        max_output_bytes,
    })
}

async fn spawn_session_runner(
    workspace_root: &Path,
    session_dir: &Path,
) -> Result<(), PtySessionError> {
    let runner = locate_session_runner(workspace_root)
        .ok_or_else(|| machine_error("pty_session_runner_missing"))?;
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(session_dir.join("runner.log"))
        .map_err(|err| io_error("pty_runner_log_open_failed", session_dir, err))?;
    let stderr = stdout
        .try_clone()
        .map_err(|err| io_error("pty_runner_log_clone_failed", session_dir, err))?;
    let mut command = Command::new(&runner);
    command
        .arg("--session-dir")
        .arg(session_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .kill_on_drop(false);
    crate::skills::place_subprocess_in_own_process_group(&mut command);
    let child = command
        .spawn()
        .map_err(|err| io_error("pty_session_runner_spawn_failed", &runner, err))?;
    if let Some(pid) = child.id() {
        let _ = fs::write(session_dir.join("runner_pid"), pid.to_string());
    }
    drop(child);
    Ok(())
}

fn locate_session_runner(workspace_root: &Path) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("RUSTCLAW_PTY_SESSION_RUNNER")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
    {
        return Some(path);
    }
    if let Ok(current) = std::env::current_exe() {
        if let Some(parent) = current.parent() {
            let direct = parent.join("pty-session-runner");
            if direct.is_file() {
                return Some(direct);
            }
            if parent.file_name().is_some_and(|name| name == "deps") {
                let debug = parent
                    .parent()
                    .map(|parent| parent.join("pty-session-runner"));
                if debug.as_ref().is_some_and(|path| path.is_file()) {
                    return debug;
                }
            }
        }
    }
    [
        workspace_root.join("target/release/pty-session-runner"),
        workspace_root.join("target/debug/pty-session-runner"),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

async fn send_control(
    session_dir: &Path,
    action: &str,
    args: &Map<String, Value>,
) -> Result<String, PtySessionError> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let mut request = json!({
        "schema_version": 1,
        "request_id": request_id,
        "action": action,
        "created_at": crate::now_ts_u64(),
    });
    let request_obj = request.as_object_mut().expect("request is object");
    for key in [
        "data",
        "data_base64",
        "append_newline",
        "rows",
        "cols",
        "signal",
    ] {
        if let Some(value) = args.get(key) {
            request_obj.insert(key.to_string(), value.clone());
        }
    }
    let request_path = session_dir
        .join("controls")
        .join(format!("{request_id}.json"));
    atomic_write_json(&request_path, &request)
        .map_err(|err| io_error("pty_control_write_failed", &request_path, err))?;
    let response_path = session_dir
        .join("responses")
        .join(format!("{request_id}.json"));
    let deadline = tokio::time::Instant::now() + CONTROL_TIMEOUT;
    loop {
        if let Ok(bytes) = fs::read(&response_path) {
            let _ = fs::remove_file(&response_path);
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|_| machine_error("pty_control_response_invalid"))?;
            return Ok(value.to_string());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(machine_error("pty_control_timeout"));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn poll_session(
    session_dir: &Path,
    spec: &PtyLaunchSpec,
    args: &Map<String, Value>,
) -> Result<String, PtySessionError> {
    let metadata =
        read_metadata(session_dir).ok_or_else(|| machine_error("pty_metadata_missing"))?;
    let output_path = session_dir.join("output.bin");
    let total_bytes = fs::metadata(&output_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let cursor = args
        .get("cursor")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(total_bytes);
    let max_bytes = args
        .get("max_bytes")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(DEFAULT_PAGE_BYTES)
        .clamp(256, MAX_PAGE_BYTES);
    let mut bytes = Vec::new();
    if let Ok(mut file) = File::open(&output_path) {
        file.seek(SeekFrom::Start(cursor))
            .and_then(|_| {
                file.take(max_bytes as u64)
                    .read_to_end(&mut bytes)
                    .map(|_| ())
            })
            .map_err(|err| io_error("pty_output_read_failed", &output_path, err))?;
    }
    let (encoding, content, emitted_bytes) = encode_page(&bytes);
    let end_cursor = cursor.saturating_add(emitted_bytes as u64);
    let observed_total = fs::metadata(&output_path)
        .map(|metadata| metadata.len())
        .unwrap_or(total_bytes);
    let has_more = end_cursor < observed_total;
    let mut projection = session_projection(spec, &metadata, observed_total);
    let obj = projection.as_object_mut().expect("projection is object");
    obj.insert("content".to_string(), Value::String(content));
    obj.insert("encoding".to_string(), Value::String(encoding.to_string()));
    obj.insert("returned_bytes".to_string(), json!(emitted_bytes));
    obj.insert(
        "snapshot_hash".to_string(),
        Value::String(sha256_file(&output_path).unwrap_or_default()),
    );
    obj.insert("truncated".to_string(), Value::Bool(cursor > 0 || has_more));
    obj.insert(
        "page".to_string(),
        json!({
            "cursor": cursor,
            "start_byte": cursor,
            "end_byte": end_cursor,
            "total_bytes": observed_total,
            "limit_bytes": max_bytes,
            "has_more": has_more,
            "next_cursor": has_more.then_some(end_cursor),
        }),
    );
    Ok(projection.to_string())
}

fn session_projection(
    spec: &PtyLaunchSpec,
    metadata: &PtySessionMetadata,
    output_bytes: u64,
) -> Value {
    let heartbeat_stale = metadata.status == "running"
        && (crate::now_ts_u64() as i64).saturating_sub(metadata.heartbeat_at) > 15;
    json!({
        "schema_version": 1,
        "session_id": spec.session_id,
        "task_id": spec.task_id,
        "status": if heartbeat_stale { "runner_unreachable" } else { metadata.status.as_str() },
        "pid": metadata.pid,
        "exit_code": metadata.exit_code,
        "reason_code": metadata.reason_code,
        "heartbeat_at": metadata.heartbeat_at,
        "heartbeat_stale": heartbeat_stale,
        "created_at": spec.created_at,
        "expires_at": spec.expires_at,
        "idle_timeout_seconds": spec.idle_timeout_seconds,
        "max_output_bytes": spec.max_output_bytes,
        "rows": metadata.rows,
        "cols": metadata.cols,
        "output_bytes": output_bytes.max(metadata.output_bytes),
        "retryable": metadata.status == "running",
        "operations": [
            "terminal_write",
            "terminal_poll",
            "terminal_resize",
            "terminal_signal",
            "terminal_terminate"
        ],
    })
}

fn read_launch_spec(session_dir: &Path) -> Result<PtyLaunchSpec, PtySessionError> {
    let path = session_dir.join("launch.json");
    let bytes =
        fs::read(&path).map_err(|err| io_error("pty_launch_spec_read_failed", &path, err))?;
    serde_json::from_slice(&bytes).map_err(|_| machine_error("pty_launch_spec_invalid"))
}

fn read_metadata(session_dir: &Path) -> Option<PtySessionMetadata> {
    fs::read(session_dir.join("metadata.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

fn ensure_owner(task: Option<&ClaimedTask>, spec: &PtyLaunchSpec) -> Result<(), PtySessionError> {
    let owner_matches = match task {
        Some(task) => {
            task.user_id == spec.owner_user_id
                && task.chat_id == spec.owner_chat_id
                && task.channel == spec.owner_channel
        }
        None => spec.owner_user_id == 0 && spec.owner_chat_id == 0,
    };
    if owner_matches {
        Ok(())
    } else {
        Err(machine_error("pty_session_owner_mismatch"))
    }
}

fn session_dir(workspace_root: &Path, session_id: &str) -> Result<PathBuf, PtySessionError> {
    if session_id.is_empty()
        || session_id.len() > 96
        || !session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(machine_error("pty_session_id_invalid"));
    }
    Ok(workspace_root.join(SESSION_ROOT).join(session_id))
}

fn create_session_directories(
    workspace_root: &Path,
    session_dir: &Path,
) -> Result<(), PtySessionError> {
    let state_root = workspace_root.join(".rustclaw");
    let sessions_root = state_root.join("pty_sessions");
    ensure_real_directory(&state_root)?;
    ensure_real_directory(&sessions_root)?;
    fs::create_dir(session_dir)
        .map_err(|err| io_error("pty_session_create_failed", session_dir, err))?;
    set_private_directory_permissions(&sessions_root)?;
    set_private_directory_permissions(session_dir)?;
    if let Err(err) = fs::create_dir(session_dir.join("controls"))
        .and_then(|_| fs::create_dir(session_dir.join("responses")))
    {
        let _ = fs::remove_dir_all(session_dir);
        return Err(io_error("pty_session_create_failed", session_dir, err));
    }
    validate_existing_session_dir(workspace_root, session_dir)
}

fn ensure_real_directory(path: &Path) -> Result<(), PtySessionError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(machine_error("pty_session_state_path_unsafe"))
        }
        Ok(_) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => fs::create_dir(path)
            .map_err(|err| io_error("pty_session_state_create_failed", path, err)),
        Err(err) => Err(io_error("pty_session_state_inspect_failed", path, err)),
    }
}

fn validate_existing_session_dir(
    workspace_root: &Path,
    session_dir: &Path,
) -> Result<(), PtySessionError> {
    for path in [
        workspace_root.join(".rustclaw"),
        workspace_root.join(SESSION_ROOT),
        session_dir.to_path_buf(),
    ] {
        let metadata = fs::symlink_metadata(&path)
            .map_err(|err| io_error("pty_session_state_inspect_failed", &path, err))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(machine_error("pty_session_state_path_unsafe"));
        }
    }
    let workspace = workspace_root
        .canonicalize()
        .map_err(|err| io_error("pty_workspace_canonicalize_failed", workspace_root, err))?;
    let session = session_dir
        .canonicalize()
        .map_err(|err| io_error("pty_session_canonicalize_failed", session_dir, err))?;
    if !session.starts_with(&workspace) {
        return Err(machine_error("pty_session_workspace_escape"));
    }
    Ok(())
}

fn required_session_id(args: &Map<String, Value>) -> Result<&str, PtySessionError> {
    args.get("session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| machine_error("pty_session_id_missing"))
}

fn encode_page(bytes: &[u8]) -> (&'static str, String, usize) {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return ("utf-8", text.to_string(), bytes.len());
    }
    for trim in 1..=3.min(bytes.len()) {
        let candidate = &bytes[..bytes.len() - trim];
        if let Ok(text) = std::str::from_utf8(candidate) {
            return ("utf-8", text.to_string(), candidate.len());
        }
    }
    ("base64", BASE64_STANDARD.encode(bytes), bytes.len())
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn atomic_write_json(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "pty_path_parent_missing")
    })?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = File::create(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> Result<(), PtySessionError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|err| io_error("pty_session_permissions_failed", path, err))
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) -> Result<(), PtySessionError> {
    Ok(())
}

fn os_string(value: &std::ffi::OsStr, code: &'static str) -> Result<String, PtySessionError> {
    value
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| machine_error(code))
}

fn machine_error(code: &'static str) -> PtySessionError {
    PtySessionError {
        kind: "pty_session_error",
        code,
        extra: json!({"error_code": code}),
    }
}

fn io_error(code: &'static str, path: &Path, error: std::io::Error) -> PtySessionError {
    PtySessionError {
        kind: "pty_session_io_error",
        code,
        extra: json!({
            "error_code": code,
            "path": path.display().to_string(),
            "io_error_kind": format!("{:?}", error.kind()).to_ascii_lowercase(),
        }),
    }
}

#[cfg(test)]
#[path = "builtin_pty_session_tests.rs"]
mod tests;
