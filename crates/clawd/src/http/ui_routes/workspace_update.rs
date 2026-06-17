async fn get_workspace_update(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<WorkspaceUpdateStatus>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err((status, Json(resp))) => {
            return (
                status,
                Json(ApiResponse {
                    ok: resp.ok,
                    data: None,
                    error: resp.error,
                }),
            );
        }
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can view update status".to_string()),
            }),
        );
    }

    let shared = workspace_update_state();
    let status = refresh_workspace_update_versions(&state.skill_rt.workspace_root, shared).await;
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(status),
            error: None,
        }),
    )
}

async fn refresh_workspace_update_versions(
    workspace_root: &Path,
    shared: Arc<Mutex<WorkspaceUpdateStatus>>,
) -> WorkspaceUpdateStatus {
    let snapshot = shared.lock().unwrap().clone();
    if matches!(snapshot.status.as_str(), "running" | "restarting") {
        return snapshot;
    }

    let local_commit =
        run_workspace_update_command("git", &["rev-parse", "--short", "HEAD"], workspace_root, 10)
            .await
            .ok()
            .filter(|out| out.exit_code == Some(0))
            .and_then(|out| first_output_line(&out.stdout_tail));

    let fetch_output =
        run_workspace_update_command("git", &["fetch", "--quiet"], workspace_root, 30)
            .await
            .ok();

    let remote_commit = run_workspace_update_command(
        "git",
        &["rev-parse", "--short", "@{upstream}"],
        workspace_root,
        10,
    )
    .await
    .ok()
    .filter(|out| out.exit_code == Some(0))
    .and_then(|out| first_output_line(&out.stdout_tail));

    let mut guard = shared.lock().unwrap();
    if let Some(local_commit) = local_commit.clone() {
        guard.old_commit = Some(local_commit.clone());
        if matches!(guard.status.as_str(), "idle" | "up_to_date") {
            guard.new_commit = Some(local_commit);
        }
    }
    if let Some(remote_commit) = remote_commit.clone() {
        guard.remote_commit = Some(remote_commit);
    }
    if matches!(guard.status.as_str(), "idle" | "up_to_date") {
        match (local_commit.as_deref(), remote_commit.as_deref()) {
            (Some(local), Some(remote)) if local == remote => {
                guard.status = "up_to_date".to_string();
                guard.step = "already_latest".to_string();
                guard.error = None;
                guard.next_step = None;
            }
            (Some(_), Some(_)) => {
                guard.status = "idle".to_string();
                guard.step = "idle".to_string();
                guard.next_step = None;
            }
            _ => {}
        }
    }
    if let Some(out) = fetch_output {
        if out.exit_code != Some(0) && guard.status == "idle" {
            guard.stderr_tail = out.stderr_tail;
            guard.stdout_tail = out.stdout_tail;
        }
    }
    guard.clone()
}

async fn start_workspace_update(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<WorkspaceUpdateStatus>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err((status, Json(resp))) => {
            return (
                status,
                Json(ApiResponse {
                    ok: resp.ok,
                    data: None,
                    error: resp.error,
                }),
            );
        }
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can update RustClaw".to_string()),
            }),
        );
    }

    let shared = workspace_update_state();
    let status = {
        let mut guard = shared.lock().unwrap();
        if matches!(guard.status.as_str(), "running" | "restarting") {
            return (
                StatusCode::CONFLICT,
                Json(ApiResponse {
                    ok: false,
                    data: Some(guard.clone()),
                    error: Some("workspace update is already running".to_string()),
                }),
            );
        }
        *guard = WorkspaceUpdateStatus {
            status: "running".to_string(),
            step: "starting".to_string(),
            started_ts: Some(current_unix_ts()),
            ..WorkspaceUpdateStatus::default()
        };
        guard.clone()
    };

    let workspace_root = state.skill_rt.workspace_root.clone();
    tokio::spawn(run_workspace_update_job(workspace_root, shared));

    (
        StatusCode::ACCEPTED,
        Json(ApiResponse {
            ok: true,
            data: Some(status),
            error: None,
        }),
    )
}

async fn run_workspace_update_job(
    workspace_root: PathBuf,
    shared: Arc<Mutex<WorkspaceUpdateStatus>>,
) {
    set_workspace_update_step(&shared, "checking_current_version");
    let old_commit = match run_workspace_update_command(
        "git",
        &["rev-parse", "--short", "HEAD"],
        &workspace_root,
        30,
    )
    .await
    {
        Ok(out) if out.exit_code == Some(0) => first_output_line(&out.stdout_tail),
        Ok(out) => {
            fail_workspace_update(
                &shared,
                "git rev-parse failed",
                "请确认 RustClaw 目录是有效 Git 仓库。",
                out,
            );
            return;
        }
        Err(err) => {
            fail_workspace_update_with_error(
                &shared,
                err,
                "请确认当前用户可以在 RustClaw 目录中运行 git。",
            );
            return;
        }
    };
    {
        let mut guard = shared.lock().unwrap();
        guard.old_commit = old_commit.clone();
    }

    set_workspace_update_step(&shared, "checking_remote_version");
    match run_workspace_update_command("git", &["fetch", "--quiet"], &workspace_root, 600).await {
        Ok(out) if out.exit_code == Some(0) => {
            let mut guard = shared.lock().unwrap();
            guard.exit_code = out.exit_code;
            guard.stdout_tail = out.stdout_tail;
            guard.stderr_tail = out.stderr_tail;
        }
        Ok(out) => {
            fail_workspace_update(
                &shared,
                "git fetch failed",
                "更新要求以远端为准；远端检查失败时不会继续编译本地代码。请确认网络、Git remote 和 SSH key 后重试。",
                out,
            );
            return;
        }
        Err(err) => {
            fail_workspace_update_with_error(
                &shared,
                err,
                "更新要求以远端为准；远端检查失败时不会继续编译本地代码。请确认网络、Git remote 和 SSH key 后重试。",
            );
            return;
        }
    }

    let remote_commit = match run_workspace_update_command(
        "git",
        &["rev-parse", "--short", "@{upstream}"],
        &workspace_root,
        30,
    )
    .await
    {
        Ok(out) if out.exit_code == Some(0) => first_output_line(&out.stdout_tail),
        Ok(out) => {
            fail_workspace_update(
                &shared,
                "git rev-parse upstream failed",
                "未能读取 upstream，无法确认远端目标版本。请确认当前分支已设置 upstream 后重试。",
                out,
            );
            return;
        }
        Err(err) => {
            fail_workspace_update_with_error(
                &shared,
                err,
                "未能读取 upstream，无法确认远端目标版本。请确认当前分支已设置 upstream 后重试。",
            );
            return;
        }
    };
    {
        let mut guard = shared.lock().unwrap();
        if let Some(remote_commit) = remote_commit.clone() {
            guard.remote_commit = Some(remote_commit);
        }
    }

    let should_pull =
        old_commit.is_some() && remote_commit.is_some() && old_commit != remote_commit;
    if should_pull {
        set_workspace_update_step(&shared, "pulling_latest_code");
        match run_workspace_update_command("git", &["pull", "--ff-only"], &workspace_root, 600)
            .await
        {
            Ok(out) if out.exit_code == Some(0) => {
                let mut guard = shared.lock().unwrap();
                guard.exit_code = out.exit_code;
                guard.stdout_tail = out.stdout_tail;
                guard.stderr_tail = out.stderr_tail;
            }
            Ok(first_pull_out) => {
                let conflict_paths =
                    match detect_workspace_update_conflict_paths(&workspace_root).await {
                        Ok(paths) => paths,
                        Err(err) => {
                            fail_workspace_update_with_error(
                            &shared,
                            err,
                            "拉取失败，且无法可靠识别冲突文件；已保留本地文件，请手动处理后重试。",
                        );
                            return;
                        }
                    };
                if conflict_paths.is_empty() {
                    fail_workspace_update(
                        &shared,
                        "git pull --ff-only failed",
                        "拉取失败，但没有发现远端变更与本地未提交文件的直接冲突；已保留本地文件，请手动检查分支是否分叉或权限是否正常。",
                        first_pull_out,
                    );
                    return;
                }

                set_workspace_update_step(&shared, "resolving_conflicting_files");
                if let Err(err) =
                    overwrite_workspace_update_conflict_paths(&workspace_root, &conflict_paths)
                        .await
                {
                    fail_workspace_update_with_error(
                        &shared,
                        err,
                        "覆盖冲突文件失败；未冲突的本地文件已保持不动，请手动处理后重试。",
                    );
                    return;
                }
                {
                    let mut guard = shared.lock().unwrap();
                    guard.next_step = Some(format!(
                        "已只覆盖 {} 个冲突路径；其他本地改动和额外文件保持不动，正在重新拉取远端。",
                        conflict_paths.len()
                    ));
                }

                set_workspace_update_step(&shared, "pulling_latest_code");
                match run_workspace_update_command(
                    "git",
                    &["pull", "--ff-only"],
                    &workspace_root,
                    600,
                )
                .await
                {
                    Ok(out) if out.exit_code == Some(0) => {
                        let mut guard = shared.lock().unwrap();
                        guard.exit_code = out.exit_code;
                        guard.stdout_tail = out.stdout_tail;
                        guard.stderr_tail = out.stderr_tail;
                    }
                    Ok(out) => {
                        fail_workspace_update(
                            &shared,
                            "git pull --ff-only failed after resolving conflicts",
                            "已覆盖识别到的冲突文件，但重新拉取仍失败；其他本地文件未动，请查看 Git 输出后手动处理。",
                            out,
                        );
                        return;
                    }
                    Err(err) => {
                        fail_workspace_update_with_error(
                            &shared,
                            err,
                            "已覆盖识别到的冲突文件，但重新拉取仍失败；其他本地文件未动，请查看 Git 输出后手动处理。",
                        );
                        return;
                    }
                }
            }
            Err(err) => {
                fail_workspace_update_with_error(
                    &shared,
                    err,
                    "拉取远端失败；已保留本地文件，请确认 Git 和网络状态后重试。",
                );
                return;
            }
        }
    } else {
        let mut guard = shared.lock().unwrap();
        guard.step = "skipping_pull_latest_code".to_string();
        if old_commit.is_some() && remote_commit.is_some() {
            guard.next_step =
                Some("远端没有新版本；本地文件保持不动，将继续执行完整编译。".to_string());
        }
    }

    set_workspace_update_step(&shared, "checking_new_version");
    if let Ok(out) = run_workspace_update_command(
        "git",
        &["rev-parse", "--short", "HEAD"],
        &workspace_root,
        30,
    )
    .await
    {
        if out.exit_code == Some(0) {
            let mut guard = shared.lock().unwrap();
            guard.new_commit = first_output_line(&out.stdout_tail);
        }
    }

    set_workspace_update_step(&shared, "building_workspace");
    {
        let mut guard = shared.lock().unwrap();
        guard.exit_code = None;
        guard.stdout_tail.clear();
        guard.stderr_tail.clear();
        guard.next_step = Some("正在编译，编译日志会持续刷新。".to_string());
    }
    match run_workspace_update_command_streaming(
        "bash",
        &["./build-all.sh"],
        &workspace_root,
        WORKSPACE_UPDATE_TIMEOUT_SECONDS,
        shared.clone(),
    )
    .await
    {
        Ok(out) if out.exit_code == Some(0) => {
            let mut guard = shared.lock().unwrap();
            guard.exit_code = out.exit_code;
            guard.stdout_tail = out.stdout_tail;
            guard.stderr_tail = out.stderr_tail;
        }
        Ok(out) => {
            fail_workspace_update(
                &shared,
                "./build-all.sh failed",
                "请查看构建日志摘要；修复依赖或编译错误后再重试。",
                out,
            );
            return;
        }
        Err(err) => {
            fail_workspace_update_with_error(
                &shared,
                err,
                "请确认服务器依赖完整，并查看构建日志。",
            );
            return;
        }
    }

    set_workspace_update_step(&shared, "restarting_clawd");
    let workspace = workspace_root.to_string_lossy();
    let script = format!(
        "sleep 2; cd {} && ./start-all-bin.sh release",
        shell_escape_arg(workspace.as_ref())
    );
    let spawn_result = StdCommand::new("nohup")
        .arg("bash")
        .arg("-c")
        .arg(&script)
        .current_dir(&workspace_root)
        .stdin(StdProcessStdio::null())
        .stdout(StdProcessStdio::null())
        .stderr(StdProcessStdio::null())
        .spawn();

    match spawn_result {
        Ok(_) => {
            let mut guard = shared.lock().unwrap();
            guard.status = "restarting".to_string();
            guard.step = "restart_scheduled".to_string();
            guard.finished_ts = Some(current_unix_ts());
            guard.error = None;
            guard.next_step = Some("RustClaw 正在重启，请等待 10-20 秒后刷新页面。".to_string());
        }
        Err(err) => {
            fail_workspace_update_with_error(
                &shared,
                format!("failed to schedule clawd restart: {err}"),
                "构建已完成，但自动重启失败。请在服务器上手动重启 clawd。",
            );
        }
    }
}

fn set_workspace_update_step(shared: &Arc<Mutex<WorkspaceUpdateStatus>>, step: &str) {
    let mut guard = shared.lock().unwrap();
    guard.status = "running".to_string();
    guard.step = step.to_string();
}

fn fail_workspace_update(
    shared: &Arc<Mutex<WorkspaceUpdateStatus>>,
    error: &str,
    next_step: &str,
    output: WorkspaceUpdateCommandOutput,
) {
    let mut guard = shared.lock().unwrap();
    guard.status = "failed".to_string();
    guard.finished_ts = Some(current_unix_ts());
    guard.exit_code = output.exit_code;
    guard.stdout_tail = output.stdout_tail;
    guard.stderr_tail = output.stderr_tail;
    guard.error = Some(error.to_string());
    guard.next_step = Some(next_step.to_string());
}

fn fail_workspace_update_with_error(
    shared: &Arc<Mutex<WorkspaceUpdateStatus>>,
    error: impl Into<String>,
    next_step: &str,
) {
    let mut guard = shared.lock().unwrap();
    guard.status = "failed".to_string();
    guard.finished_ts = Some(current_unix_ts());
    guard.error = Some(error.into());
    guard.next_step = Some(next_step.to_string());
}

async fn detect_workspace_update_conflict_paths(
    workspace_root: &Path,
) -> Result<WorkspaceUpdateConflictPaths, String> {
    let remote_changed = git_workspace_name_list(
        &["diff", "--name-only", "HEAD", "@{upstream}"],
        workspace_root,
    )
    .await?;
    let local_unstaged = git_workspace_name_list(&["diff", "--name-only"], workspace_root).await?;
    let local_staged =
        git_workspace_name_list(&["diff", "--cached", "--name-only"], workspace_root).await?;
    let local_untracked = git_workspace_name_list(
        &["ls-files", "--others", "--exclude-standard"],
        workspace_root,
    )
    .await?;

    let remote_changed = remote_changed.into_iter().collect::<BTreeSet<_>>();
    let mut tracked_dirty = local_unstaged.into_iter().collect::<BTreeSet<_>>();
    tracked_dirty.extend(local_staged);

    Ok(WorkspaceUpdateConflictPaths {
        tracked: tracked_dirty
            .into_iter()
            .filter(|path| remote_changed.contains(path))
            .collect(),
        untracked: local_untracked
            .into_iter()
            .filter(|path| remote_changed.contains(path))
            .collect(),
    })
}

async fn overwrite_workspace_update_conflict_paths(
    workspace_root: &Path,
    paths: &WorkspaceUpdateConflictPaths,
) -> Result<(), String> {
    if !paths.tracked.is_empty() {
        let mut args = vec![
            "restore".to_string(),
            "--source".to_string(),
            "HEAD".to_string(),
            "--staged".to_string(),
            "--worktree".to_string(),
            "--".to_string(),
        ];
        args.extend(paths.tracked.iter().cloned());
        let out = run_workspace_update_command_args("git", &args, workspace_root, 600).await?;
        if out.exit_code != Some(0) {
            return Err(format!(
                "git restore conflict files failed: {}",
                workspace_update_output_detail(&out)
            ));
        }
    }

    if !paths.untracked.is_empty() {
        let mut args = vec!["clean".to_string(), "-fd".to_string(), "--".to_string()];
        args.extend(paths.untracked.iter().cloned());
        let out = run_workspace_update_command_args("git", &args, workspace_root, 600).await?;
        if out.exit_code != Some(0) {
            return Err(format!(
                "git clean conflict files failed: {}",
                workspace_update_output_detail(&out)
            ));
        }
    }

    Ok(())
}

async fn git_workspace_name_list(
    args: &[&str],
    workspace_root: &Path,
) -> Result<Vec<String>, String> {
    let out = run_workspace_update_command("git", args, workspace_root, 60).await?;
    if out.exit_code != Some(0) {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            workspace_update_output_detail(&out)
        ));
    }
    parse_git_name_list_output(&out.stdout_tail)
}

fn parse_git_name_list_output(raw: &str) -> Result<Vec<String>, String> {
    if raw.starts_with("... output truncated ...") {
        return Err("git path list output is too large to process safely".to_string());
    }
    Ok(raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
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
    timeout_seconds: u64,
    shared: Arc<Mutex<WorkspaceUpdateStatus>>,
) -> Result<WorkspaceUpdateCommandOutput, String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(StdProcessStdio::null())
        .stdout(StdProcessStdio::piped())
        .stderr(StdProcessStdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|err| format!("failed to run {program}: {err}"))?;
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

    let status = match tokio::time::timeout(
        std::time::Duration::from_secs(timeout_seconds),
        child.wait(),
    )
    .await
    {
        Ok(Ok(status)) => status,
        Ok(Err(err)) => return Err(format!("failed to wait for {program}: {err}")),
        Err(_) => {
            let _ = child.kill().await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            return Err(format!("{program} timed out after {timeout_seconds}s"));
        }
    };

    let _ = stdout_task.await;
    let _ = stderr_task.await;

    let guard = shared.lock().unwrap();
    Ok(WorkspaceUpdateCommandOutput {
        exit_code: status.code(),
        stdout_tail: guard.stdout_tail.clone(),
        stderr_tail: guard.stderr_tail.clone(),
    })
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
    let mut guard = shared.lock().unwrap();
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
