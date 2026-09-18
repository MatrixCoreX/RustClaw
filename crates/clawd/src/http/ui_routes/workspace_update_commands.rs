async fn detect_workspace_update_conflict_paths(
    workspace_root: &Path,
) -> Result<WorkspaceUpdateConflictPaths, String> {
    let remote_changed = git_workspace_name_list_raw(
        &["diff", "--name-only", "-z", "HEAD", "@{upstream}"],
        &[],
        workspace_root,
    )
    .await?;
    if remote_changed.is_empty() {
        return Ok(WorkspaceUpdateConflictPaths::default());
    }
    let mut tracked_dirty = BTreeSet::new();
    let mut local_untracked = BTreeSet::new();
    for batch in remote_changed.chunks(WORKSPACE_UPDATE_PATH_BATCH_SIZE) {
        let (unstaged, staged, untracked) = tokio::try_join!(
            git_workspace_name_list_raw(
                &["diff", "--name-only", "-z", "--"],
                batch,
                workspace_root
            ),
            git_workspace_name_list_raw(
                &["diff", "--cached", "--name-only", "-z", "--"],
                batch,
                workspace_root,
            ),
            git_workspace_name_list_raw(
                &["ls-files", "--others", "--exclude-standard", "-z", "--"],
                batch,
                workspace_root,
            ),
        )?;
        tracked_dirty.extend(unstaged);
        tracked_dirty.extend(staged);
        local_untracked.extend(untracked);
    }

    // A generated file may already contain the exact incoming upstream content
    // while still appearing dirty against the current local HEAD. Only retain
    // tracked paths whose working-tree content actually differs from upstream.
    let tracked_dirty = tracked_dirty.into_iter().collect::<Vec<_>>();
    let mut tracked_different_from_upstream = BTreeSet::new();
    for batch in tracked_dirty.chunks(WORKSPACE_UPDATE_PATH_BATCH_SIZE) {
        tracked_different_from_upstream.extend(
            git_workspace_name_list_raw(
                &["diff", "--name-only", "-z", "@{upstream}", "--"],
                batch,
                workspace_root,
            )
            .await?,
        );
    }

    Ok(WorkspaceUpdateConflictPaths {
        tracked: tracked_different_from_upstream.into_iter().collect(),
        untracked: local_untracked.into_iter().collect(),
    })
}

async fn git_workspace_name_list_raw(
    args: &[&str],
    scoped_paths: &[String],
    workspace_root: &Path,
) -> Result<Vec<String>, String> {
    let mut command = Command::new("git");
    command
        .args(args)
        .args(scoped_paths)
        .current_dir(workspace_root)
        .stdin(StdProcessStdio::null())
        .stdout(StdProcessStdio::piped())
        .stderr(StdProcessStdio::piped());
    let output = tokio::time::timeout(std::time::Duration::from_secs(60), command.output())
        .await
        .map_err(|_| "workspace_update_git_path_query_timeout:seconds=60".to_string())?
        .map_err(|error| format!("workspace_update_git_path_query_failed:error={error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            truncate_tail(&String::from_utf8_lossy(&output.stderr))
        ));
    }
    parse_git_name_list_bytes(&output.stdout)
}

fn parse_git_name_list_bytes(raw: &[u8]) -> Result<Vec<String>, String> {
    if raw.len() > WORKSPACE_UPDATE_PATH_LIST_MAX_BYTES {
        return Err("workspace_update_git_path_list_byte_limit_exceeded".to_string());
    }
    let mut paths = Vec::new();
    for item in raw.split(|byte| *byte == 0).filter(|item| !item.is_empty()) {
        if paths.len() >= WORKSPACE_UPDATE_PATH_LIST_MAX_ITEMS {
            return Err("workspace_update_git_path_list_item_limit_exceeded".to_string());
        }
        let path = std::str::from_utf8(item)
            .map_err(|_| "workspace_update_git_path_non_utf8".to_string())?;
        if !safe_workspace_relative_git_path(path) {
            return Err("workspace_update_git_path_unsafe_relative".to_string());
        }
        paths.push(path.to_string());
    }
    Ok(paths)
}

fn safe_workspace_relative_git_path(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path).components().all(|component| {
            matches!(
                component,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
}

fn workspace_update_output_detail(out: &WorkspaceUpdateCommandOutput) -> String {
    let stderr = out.stderr_tail.trim();
    if !stderr.is_empty() {
        return stderr.to_string();
    }
    let stdout = out.stdout_tail.trim();
    if !stdout.is_empty() {
        return stdout.to_string();
    }
    format!("exit_code={:?}", out.exit_code)
}

async fn run_workspace_update_command(
    program: &str,
    args: &[&str],
    cwd: &Path,
    timeout_seconds: u64,
) -> Result<WorkspaceUpdateCommandOutput, String> {
    let args = args
        .iter()
        .map(|arg| (*arg).to_string())
        .collect::<Vec<_>>();
    run_workspace_update_command_args(program, &args, cwd, timeout_seconds).await
}

async fn run_workspace_update_command_args(
    program: &str,
    args: &[String],
    cwd: &Path,
    timeout_seconds: u64,
) -> Result<WorkspaceUpdateCommandOutput, String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(StdProcessStdio::null())
        .stdout(StdProcessStdio::piped())
        .stderr(StdProcessStdio::piped());
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_seconds),
        cmd.output(),
    )
    .await
    .map_err(|_| format!("{program} timed out after {timeout_seconds}s"))?
    .map_err(|err| format!("failed to run {program}: {err}"))?;
    Ok(WorkspaceUpdateCommandOutput {
        exit_code: output.status.code(),
        stdout_tail: truncate_tail(&String::from_utf8_lossy(&output.stdout)),
        stderr_tail: truncate_tail(&String::from_utf8_lossy(&output.stderr)),
    })
}

async fn run_workspace_update_command_streaming(
    program: &str,
    args: &[&str],
    cwd: &Path,
    shared: Arc<Mutex<WorkspaceUpdateStatus>>,
    control: Arc<Mutex<WorkspaceUpdateControl>>,
) -> Result<WorkspaceUpdateCommandOutput, String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(StdProcessStdio::null())
        .stdout(StdProcessStdio::piped())
        .stderr(StdProcessStdio::piped());
    crate::skills::place_subprocess_in_own_process_group(&mut cmd);

    let mut child = cmd
        .spawn()
        .map_err(|err| format!("failed to run {program}: {err}"))?;
    let process_group_pid = child.id();
    if let Some(pid) = process_group_pid {
        let mut guard = workspace_update_control_lock(control.as_ref());
        guard.active_child_pid = Some(pid);
    }
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("failed to capture {program} stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("failed to capture {program} stderr"))?;

    let stdout_task = tokio::spawn(read_workspace_update_stream(stdout, shared.clone(), true));
    let stderr_task = tokio::spawn(read_workspace_update_stream(stderr, shared.clone(), false));

    let status = loop {
        if workspace_update_cancel_requested(&control) {
            if let Some(pid) = process_group_pid {
                terminate_workspace_update_process_tree(pid);
            }
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await;
            if let Some(pid) = process_group_pid {
                force_kill_workspace_update_process_tree(pid);
            }
            let _ = child.kill().await;
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await;
            stdout_task.abort();
            stderr_task.abort();
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            finish_workspace_update_canceled(&shared, &control);
            return Err(WORKSPACE_UPDATE_CANCELED_ERROR.to_string());
        }

        match tokio::time::timeout(std::time::Duration::from_millis(500), child.wait()).await {
            Ok(Ok(status)) => break status,
            Ok(Err(err)) => return Err(format!("failed to wait for {program}: {err}")),
            Err(_) => continue,
        }
    };

    cleanup_workspace_update_process_group(process_group_pid).await;
    finish_workspace_update_stream_task(stdout_task).await;
    finish_workspace_update_stream_task(stderr_task).await;
    {
        let mut guard = workspace_update_control_lock(control.as_ref());
        guard.active_child_pid = None;
    }

    let guard = workspace_update_status_lock(shared.as_ref());
    Ok(WorkspaceUpdateCommandOutput {
        exit_code: status.code(),
        stdout_tail: guard.stdout_tail.clone(),
        stderr_tail: guard.stderr_tail.clone(),
    })
}

async fn cleanup_workspace_update_process_group(process_group_pid: Option<u32>) {
    let Some(pid) = process_group_pid.filter(|pid| workspace_update_process_group_exists(*pid))
    else {
        return;
    };
    terminate_workspace_update_process_tree(pid);
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    if workspace_update_process_group_exists(pid) {
        force_kill_workspace_update_process_tree(pid);
    }
}

async fn finish_workspace_update_stream_task(mut task: tokio::task::JoinHandle<()>) {
    if tokio::time::timeout(std::time::Duration::from_secs(2), &mut task)
        .await
        .is_err()
    {
        task.abort();
        let _ = task.await;
    }
}

fn terminate_workspace_update_process_tree(pid: u32) {
    if signal_workspace_update_process_group(pid, libc::SIGTERM) {
        return;
    }
    let pid_text = pid.to_string();
    for _ in 0..3 {
        let _ = StdCommand::new("pkill")
            .args(["-TERM", "-P", pid_text.as_str()])
            .stdout(StdProcessStdio::null())
            .stderr(StdProcessStdio::null())
            .status();
    }
    let _ = StdCommand::new("kill")
        .args(["-TERM", pid_text.as_str()])
        .stdout(StdProcessStdio::null())
        .stderr(StdProcessStdio::null())
        .status();
}

fn force_kill_workspace_update_process_tree(pid: u32) {
    if signal_workspace_update_process_group(pid, libc::SIGKILL) {
        return;
    }
    let pid_text = pid.to_string();
    let _ = StdCommand::new("pkill")
        .args(["-KILL", "-P", pid_text.as_str()])
        .stdout(StdProcessStdio::null())
        .stderr(StdProcessStdio::null())
        .status();
    let _ = StdCommand::new("kill")
        .args(["-KILL", pid_text.as_str()])
        .stdout(StdProcessStdio::null())
        .stderr(StdProcessStdio::null())
        .status();
}

#[cfg(unix)]
fn signal_workspace_update_process_group(pid: u32, signal: i32) -> bool {
    let Ok(process_group) = i32::try_from(pid) else {
        return false;
    };
    if process_group <= 0 {
        return false;
    }
    // SAFETY: a negative PID targets the dedicated process group created for
    // this build. No pointer or shared-memory invariants are involved.
    unsafe { libc::kill(-process_group, signal) == 0 }
}

#[cfg(not(unix))]
fn signal_workspace_update_process_group(_pid: u32, _signal: i32) -> bool {
    false
}

#[cfg(unix)]
fn workspace_update_process_group_exists(pid: u32) -> bool {
    let Ok(process_group) = i32::try_from(pid) else {
        return false;
    };
    if process_group <= 0 {
        return false;
    }
    if unsafe { libc::kill(-process_group, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn workspace_update_process_group_exists(_pid: u32) -> bool {
    false
}

async fn read_workspace_update_stream<R>(
    reader: R,
    shared: Arc<Mutex<WorkspaceUpdateStatus>>,
    is_stdout: bool,
) where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut reader = reader;
    let mut buf = [0_u8; 4096];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&buf[..n]);
                append_workspace_update_log_chunk(&shared, is_stdout, &chunk);
            }
            Err(err) => {
                append_workspace_update_log_chunk(
                    &shared,
                    false,
                    &format!("failed to read build log stream: {err}"),
                );
                break;
            }
        }
    }
}

fn append_workspace_update_log_chunk(
    shared: &Arc<Mutex<WorkspaceUpdateStatus>>,
    is_stdout: bool,
    chunk: &str,
) {
    if chunk.is_empty() {
        return;
    }
    let mut guard = workspace_update_status_lock(shared.as_ref());
    let target = if is_stdout {
        &mut guard.stdout_tail
    } else {
        &mut guard.stderr_tail
    };
    target.push_str(&chunk.replace('\r', "\n"));
    let truncated = truncate_tail(target.as_str());
    *target = truncated;
}

fn truncate_tail(raw: &str) -> String {
    let chars = raw.chars().collect::<Vec<_>>();
    if chars.len() <= WORKSPACE_UPDATE_LOG_MAX_CHARS {
        return raw.to_string();
    }
    let tail = chars[chars.len() - WORKSPACE_UPDATE_LOG_MAX_CHARS..]
        .iter()
        .collect::<String>();
    format!("... output truncated ...\n{tail}")
}

fn first_output_line(raw: &str) -> Option<String> {
    raw.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}
