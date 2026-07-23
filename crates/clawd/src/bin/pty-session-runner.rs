use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::Deserialize;
use serde_json::{json, Value};

const MAX_CONTROL_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Deserialize)]
struct LaunchSpec {
    schema_version: u32,
    session_id: String,
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
struct ControlRequest {
    schema_version: u32,
    request_id: String,
    action: String,
    #[serde(default)]
    data: Option<String>,
    #[serde(default)]
    data_base64: Option<String>,
    #[serde(default)]
    append_newline: bool,
    #[serde(default)]
    rows: Option<u16>,
    #[serde(default)]
    cols: Option<u16>,
    #[serde(default)]
    signal: Option<String>,
}

struct RuntimeState {
    status: &'static str,
    reason_code: Option<&'static str>,
    exit_code: Option<u32>,
    exit_signal: Option<String>,
    rows: u16,
    cols: u16,
}

fn main() -> Result<()> {
    let session_dir = parse_session_dir()?;
    let spec: LaunchSpec = serde_json::from_slice(
        &fs::read(session_dir.join("launch.json")).context("pty_launch_spec_read_failed")?,
    )
    .context("pty_launch_spec_invalid")?;
    if spec.schema_version != 1 {
        return Err(anyhow!("pty_launch_schema_unsupported"));
    }
    validate_launch_identity(&session_dir, &spec)?;
    fs::create_dir_all(session_dir.join("controls")).context("pty_controls_create_failed")?;
    fs::create_dir_all(session_dir.join("responses")).context("pty_responses_create_failed")?;

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: spec.rows,
            cols: spec.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("pty_open_failed")?;
    let mut command = CommandBuilder::new(&spec.program);
    command.args(&spec.args);
    command.cwd(&spec.cwd);
    if spec.env_clear {
        command.env_clear();
    }
    for (key, value) in &spec.env {
        match value {
            Some(value) => command.env(key, value),
            None => command.env_remove(key),
        }
    }
    let output_path = session_dir.join("output.bin");
    let existing_output_bytes = fs::metadata(&output_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let output = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&output_path)
        .context("pty_output_open_failed")?;
    let mut child = pair
        .slave
        .spawn_command(command)
        .context("pty_child_spawn_failed")?;
    drop(pair.slave);
    let child_pid = child.process_id();
    let mut writer = pair
        .master
        .take_writer()
        .context("pty_writer_open_failed")?;
    let mut reader = pair
        .master
        .try_clone_reader()
        .context("pty_reader_open_failed")?;
    let output_bytes = Arc::new(AtomicU64::new(
        existing_output_bytes.min(spec.max_output_bytes),
    ));
    let output_failed = Arc::new(AtomicBool::new(false));
    let last_activity = Arc::new(AtomicI64::new(now_ts()));
    let output_counter = Arc::clone(&output_bytes);
    let output_failure = Arc::clone(&output_failed);
    let output_activity = Arc::clone(&last_activity);
    let max_output_bytes = spec.max_output_bytes;
    let reader_thread = std::thread::spawn(move || {
        let mut output = output;
        let mut buffer = [0_u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    let written = output_counter.load(Ordering::Relaxed);
                    let remaining = max_output_bytes.saturating_sub(written) as usize;
                    if remaining == 0 {
                        break;
                    }
                    let accepted = read.min(remaining);
                    if output.write_all(&buffer[..accepted]).is_err() || output.flush().is_err() {
                        output_failure.store(true, Ordering::Relaxed);
                        break;
                    }
                    output_counter.fetch_add(accepted as u64, Ordering::Relaxed);
                    output_activity.store(now_ts(), Ordering::Relaxed);
                    if accepted < read {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        if output.sync_all().is_err() {
            output_failure.store(true, Ordering::Relaxed);
        }
    });

    let mut runtime = RuntimeState {
        status: "running",
        reason_code: None,
        exit_code: None,
        exit_signal: None,
        rows: spec.rows,
        cols: spec.cols,
    };
    write_metadata(
        &session_dir,
        &spec,
        &runtime,
        child_pid,
        output_bytes.load(Ordering::Relaxed),
    )?;

    loop {
        process_controls(
            &session_dir,
            &mut writer,
            pair.master.as_ref(),
            child.as_mut(),
            child_pid,
            &last_activity,
            &mut runtime,
        )?;
        if let Some(status) = child.try_wait().context("pty_child_poll_failed")? {
            runtime.status = if status.success() {
                "succeeded"
            } else {
                "failed"
            };
            runtime.exit_code = Some(status.exit_code());
            runtime.exit_signal = status.signal().map(str::to_string);
            runtime.reason_code.get_or_insert("pty_child_exited");
        } else {
            let now = now_ts();
            let idle_for = now.saturating_sub(last_activity.load(Ordering::Relaxed));
            if output_failed.load(Ordering::Relaxed) {
                let _ = terminate_process_group(child.as_mut(), child_pid);
                runtime.status = "failed";
                runtime.reason_code = Some("pty_output_write_failed");
            } else if output_bytes.load(Ordering::Relaxed) >= spec.max_output_bytes {
                let _ = terminate_process_group(child.as_mut(), child_pid);
                runtime.status = "failed";
                runtime.reason_code = Some("pty_output_limit");
            } else if now >= spec.expires_at {
                let _ = terminate_process_group(child.as_mut(), child_pid);
                runtime.status = "expired";
                runtime.reason_code = Some("pty_hard_timeout");
            } else if idle_for >= spec.idle_timeout_seconds as i64 {
                let _ = terminate_process_group(child.as_mut(), child_pid);
                runtime.status = "expired";
                runtime.reason_code = Some("pty_idle_timeout");
            }
        }
        if runtime.status != "running" {
            break;
        }
        write_metadata(
            &session_dir,
            &spec,
            &runtime,
            child_pid,
            output_bytes.load(Ordering::Relaxed),
        )?;
        std::thread::sleep(Duration::from_millis(100));
    }

    drop(writer);
    let _ = reader_thread.join();
    write_metadata(
        &session_dir,
        &spec,
        &runtime,
        child_pid,
        output_bytes.load(Ordering::Relaxed),
    )?;
    Ok(())
}

fn process_controls(
    session_dir: &Path,
    writer: &mut dyn Write,
    master: &dyn portable_pty::MasterPty,
    child: &mut dyn portable_pty::Child,
    child_pid: Option<u32>,
    last_activity: &AtomicI64,
    runtime: &mut RuntimeState,
) -> Result<()> {
    let mut requests = fs::read_dir(session_dir.join("controls"))
        .context("pty_controls_read_failed")?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect::<Vec<_>>();
    requests.sort();
    for path in requests {
        if fs::metadata(&path)
            .map(|metadata| metadata.len() > MAX_CONTROL_BYTES)
            .unwrap_or(true)
        {
            let _ = fs::remove_file(path);
            continue;
        }
        let request: ControlRequest = match fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        {
            Some(request) => request,
            None => {
                let _ = fs::remove_file(path);
                continue;
            }
        };
        if request.schema_version != 1 || !valid_machine_id(&request.request_id, 96) {
            let _ = fs::remove_file(path);
            continue;
        }
        let response = execute_control(&request, writer, master, child, child_pid, runtime);
        let response_path = session_dir
            .join("responses")
            .join(format!("{}.json", request.request_id));
        atomic_write_json(&response_path, &response)?;
        let _ = fs::remove_file(path);
        last_activity.store(now_ts(), Ordering::Relaxed);
    }
    Ok(())
}

fn execute_control(
    request: &ControlRequest,
    writer: &mut dyn Write,
    master: &dyn portable_pty::MasterPty,
    child: &mut dyn portable_pty::Child,
    child_pid: Option<u32>,
    runtime: &mut RuntimeState,
) -> Value {
    let result = match request.action.as_str() {
        "terminal_write" => {
            let mut data = match (&request.data, &request.data_base64) {
                (Some(data), _) => data.as_bytes().to_vec(),
                (_, Some(data)) => match BASE64_STANDARD.decode(data) {
                    Ok(data) => data,
                    Err(_) => return error_response(request, "pty_input_base64_invalid"),
                },
                _ => return error_response(request, "pty_input_missing"),
            };
            if request.append_newline {
                data.push(b'\n');
            }
            writer
                .write_all(&data)
                .and_then(|_| writer.flush())
                .map(|_| {
                    json!({
                        "bytes_written": data.len(),
                        "append_newline": request.append_newline,
                    })
                })
                .map_err(|_| ())
        }
        "terminal_resize" => {
            let rows = request.rows.unwrap_or(runtime.rows).clamp(2, 500);
            let cols = request.cols.unwrap_or(runtime.cols).clamp(2, 1000);
            master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map(|_| {
                    runtime.rows = rows;
                    runtime.cols = cols;
                    json!({"rows": rows, "cols": cols})
                })
                .map_err(|_| ())
        }
        "terminal_signal" => {
            let signal = request
                .signal
                .as_deref()
                .unwrap_or("TERM")
                .trim()
                .to_ascii_uppercase();
            send_signal(child_pid, &signal)
                .map(|_| json!({"signal": signal}))
                .map_err(|_| ())
        }
        "terminal_terminate" => terminate_process_group(child, child_pid)
            .map(|_| {
                runtime.reason_code = Some("pty_terminated");
                json!({"termination_requested": true})
            })
            .map_err(|_| ()),
        _ => return error_response(request, "pty_control_action_unsupported"),
    };
    match result {
        Ok(data) => json!({
            "schema_version": 1,
            "request_id": request.request_id,
            "action": request.action,
            "status": "ok",
            "data": data,
        }),
        Err(_) => error_response(request, "pty_control_execution_failed"),
    }
}

#[cfg(unix)]
fn send_signal(pid: Option<u32>, signal: &str) -> std::io::Result<()> {
    let signal_arg = match signal {
        "TERM" | "INT" | "HUP" | "KILL" | "WINCH" | "CONT" | "STOP" => {
            format!("-{signal}")
        }
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "pty_signal_invalid",
            ))
        }
    };
    let pid = pid.ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "pty_child_pid_missing")
    })?;
    let group_status = std::process::Command::new("kill")
        .arg(signal_arg)
        .arg("--")
        .arg(format!("-{pid}"))
        .status()?;
    if group_status.success() {
        Ok(())
    } else {
        let direct_status = std::process::Command::new("kill")
            .arg(format!("-{signal}"))
            .arg(pid.to_string())
            .status()?;
        if direct_status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other("pty_signal_failed"))
        }
    }
}

#[cfg(not(unix))]
fn send_signal(_pid: Option<u32>, _signal: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "pty_signal_unsupported",
    ))
}

fn terminate_process_group(
    child: &mut dyn portable_pty::Child,
    child_pid: Option<u32>,
) -> std::io::Result<()> {
    send_signal(child_pid, "KILL").or_else(|_| child.kill())
}

fn error_response(request: &ControlRequest, error_code: &str) -> Value {
    json!({
        "schema_version": 1,
        "request_id": request.request_id,
        "action": request.action,
        "status": "error",
        "error_code": error_code,
    })
}

fn write_metadata(
    session_dir: &Path,
    spec: &LaunchSpec,
    runtime: &RuntimeState,
    child_pid: Option<u32>,
    output_bytes: u64,
) -> Result<()> {
    atomic_write_json(
        &session_dir.join("metadata.json"),
        &json!({
            "schema_version": 1,
            "session_id": spec.session_id,
            "status": runtime.status,
            "reason_code": runtime.reason_code,
            "runner_pid": std::process::id(),
            "pid": child_pid,
            "exit_code": runtime.exit_code,
            "exit_signal": runtime.exit_signal,
            "heartbeat_at": now_ts(),
            "created_at": spec.created_at,
            "expires_at": spec.expires_at,
            "idle_timeout_seconds": spec.idle_timeout_seconds,
            "max_output_bytes": spec.max_output_bytes,
            "output_bytes": output_bytes,
            "rows": runtime.rows,
            "cols": runtime.cols,
        }),
    )
    .context("pty_metadata_write_failed")
}

fn validate_launch_identity(session_dir: &Path, spec: &LaunchSpec) -> Result<()> {
    let directory_id = session_dir
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("pty_session_directory_invalid"))?;
    if !valid_machine_id(&spec.session_id, 96) || directory_id != spec.session_id {
        return Err(anyhow!("pty_session_identity_mismatch"));
    }
    Ok(())
}

fn valid_machine_id(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn parse_session_dir() -> Result<PathBuf> {
    let mut args = std::env::args_os().skip(1);
    match (
        args.next().and_then(|arg| arg.into_string().ok()),
        args.next(),
    ) {
        (Some(flag), Some(path)) if flag == "--session-dir" => Ok(PathBuf::from(path)),
        _ => Err(anyhow!("pty_session_dir_arg_missing")),
    }
}

fn atomic_write_json(path: &Path, value: &Value) -> std::io::Result<()> {
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

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
