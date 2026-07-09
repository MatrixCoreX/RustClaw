#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum WorkspaceUpdateMode {
    Full,
    UiOnly,
    ClawdOnly,
    ReleaseDeploy,
}

impl WorkspaceUpdateMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::UiOnly => "ui_only",
            Self::ClawdOnly => "clawd_only",
            Self::ReleaseDeploy => "release_deploy",
        }
    }
}

fn workspace_update_status_lock(
    shared: &Mutex<WorkspaceUpdateStatus>,
) -> std::sync::MutexGuard<'_, WorkspaceUpdateStatus> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn workspace_update_control_lock(
    control: &Mutex<WorkspaceUpdateControl>,
) -> std::sync::MutexGuard<'_, WorkspaceUpdateControl> {
    control
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn workspace_update_api_error(
    status: StatusCode,
    error_code: &'static str,
    data: Option<WorkspaceUpdateStatus>,
) -> (StatusCode, Json<ApiResponse<WorkspaceUpdateStatus>>) {
    (
        status,
        Json(ApiResponse {
            ok: false,
            data,
            error: Some(error_code.to_string()),
        }),
    )
}

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
        return workspace_update_api_error(
            StatusCode::FORBIDDEN,
            "workspace_update_admin_required",
            None,
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
    let snapshot = workspace_update_status_lock(shared.as_ref()).clone();
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

    let mut guard = workspace_update_status_lock(shared.as_ref());
    if let Some(local_commit) = local_commit.clone() {
        guard.old_commit = Some(local_commit.clone());
        if matches!(guard.status.as_str(), "idle" | "up_to_date" | "succeeded") {
            guard.new_commit = Some(local_commit);
        }
    }
    if let Some(remote_commit) = remote_commit.clone() {
        guard.remote_commit = Some(remote_commit);
    }
    if matches!(guard.status.as_str(), "idle" | "up_to_date" | "succeeded") {
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
    start_workspace_update_with_mode(state, headers, WorkspaceUpdateMode::Full).await
}

async fn start_workspace_update_ui_only(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<WorkspaceUpdateStatus>>) {
    start_workspace_update_with_mode(state, headers, WorkspaceUpdateMode::UiOnly).await
}

async fn start_workspace_update_clawd_only(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<WorkspaceUpdateStatus>>) {
    start_workspace_update_with_mode(state, headers, WorkspaceUpdateMode::ClawdOnly).await
}

async fn start_workspace_update_release_deploy(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<WorkspaceUpdateStatus>>) {
    start_workspace_update_with_mode(state, headers, WorkspaceUpdateMode::ReleaseDeploy).await
}

async fn start_workspace_update_with_mode(
    state: AppState,
    headers: HeaderMap,
    mode: WorkspaceUpdateMode,
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
        return workspace_update_api_error(
            StatusCode::FORBIDDEN,
            "workspace_update_admin_required",
            None,
        );
    }

    let shared = workspace_update_state();
    let control = workspace_update_control();
    let status = {
        let mut guard = workspace_update_status_lock(shared.as_ref());
        if matches!(guard.status.as_str(), "running" | "restarting") {
            return workspace_update_api_error(
                StatusCode::CONFLICT,
                "workspace_update_already_running",
                Some(guard.clone()),
            );
        }
        *guard = WorkspaceUpdateStatus {
            status: "running".to_string(),
            step: "starting".to_string(),
            mode: mode.as_str().to_string(),
            started_ts: Some(current_unix_ts()),
            ..WorkspaceUpdateStatus::default()
        };
        guard.clone()
    };
    {
        let mut guard = workspace_update_control_lock(control.as_ref());
        guard.cancel_requested = false;
        guard.active_child_pid = None;
    }

    let workspace_root = state.skill_rt.workspace_root.clone();
    tokio::spawn(run_workspace_update_job(
        workspace_root,
        shared,
        control,
        mode,
    ));

    (
        StatusCode::ACCEPTED,
        Json(ApiResponse {
            ok: true,
            data: Some(status),
            error: None,
        }),
    )
}

async fn cancel_workspace_update(
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
        return workspace_update_api_error(
            StatusCode::FORBIDDEN,
            "workspace_update_admin_required",
            None,
        );
    }

    let shared = workspace_update_state();
    let control = workspace_update_control();
    let pid = {
        let mut control_guard = workspace_update_control_lock(control.as_ref());
        let mut status_guard = workspace_update_status_lock(shared.as_ref());
        if status_guard.status != "running" {
            return workspace_update_api_error(
                StatusCode::CONFLICT,
                "workspace_update_not_running",
                Some(status_guard.clone()),
            );
        }
        control_guard.cancel_requested = true;
        status_guard.step = "cancel_requested".to_string();
        status_guard.next_step = Some("正在停止当前编译进程。".to_string());
        control_guard.active_child_pid
    };

    if let Some(pid) = pid {
        terminate_workspace_update_process_tree(pid);
    }

    let status = workspace_update_status_lock(shared.as_ref()).clone();
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
    control: Arc<Mutex<WorkspaceUpdateControl>>,
    mode: WorkspaceUpdateMode,
) {
    match mode {
        WorkspaceUpdateMode::Full => {}
        WorkspaceUpdateMode::UiOnly => {
            run_workspace_update_ui_only_job(workspace_root, shared, control).await;
            return;
        }
        WorkspaceUpdateMode::ClawdOnly => {
            run_workspace_update_clawd_only_job(workspace_root, shared, control).await;
            return;
        }
        WorkspaceUpdateMode::ReleaseDeploy => {
            run_workspace_update_release_deploy_job(workspace_root, shared, control).await;
            return;
        }
    }

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
    if finish_workspace_update_if_canceled(&shared, &control) {
        return;
    }
    {
        let mut guard = workspace_update_status_lock(shared.as_ref());
        guard.old_commit = old_commit.clone();
    }

    set_workspace_update_step(&shared, "checking_remote_version");
    match run_workspace_update_command("git", &["fetch", "--quiet"], &workspace_root, 600).await {
        Ok(out) if out.exit_code == Some(0) => {
            let mut guard = workspace_update_status_lock(shared.as_ref());
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
    if finish_workspace_update_if_canceled(&shared, &control) {
        return;
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
        let mut guard = workspace_update_status_lock(shared.as_ref());
        if let Some(remote_commit) = remote_commit.clone() {
            guard.remote_commit = Some(remote_commit);
        }
    }
    if finish_workspace_update_if_canceled(&shared, &control) {
        return;
    }

    let should_pull =
        old_commit.is_some() && remote_commit.is_some() && old_commit != remote_commit;
    if should_pull {
        set_workspace_update_step(&shared, "pulling_latest_code");
        match run_workspace_update_command("git", &["pull", "--ff-only"], &workspace_root, 600)
            .await
        {
            Ok(out) if out.exit_code == Some(0) => {
                let mut guard = workspace_update_status_lock(shared.as_ref());
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
                    let mut guard = workspace_update_status_lock(shared.as_ref());
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
                        let mut guard = workspace_update_status_lock(shared.as_ref());
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
        if finish_workspace_update_if_canceled(&shared, &control) {
            return;
        }
    } else {
        let mut guard = workspace_update_status_lock(shared.as_ref());
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
            let mut guard = workspace_update_status_lock(shared.as_ref());
            guard.new_commit = first_output_line(&out.stdout_tail);
        }
    }
    if finish_workspace_update_if_canceled(&shared, &control) {
        return;
    }

    set_workspace_update_step(&shared, "building_workspace");
    {
        let mut guard = workspace_update_status_lock(shared.as_ref());
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
        control.clone(),
    )
    .await
    {
        Ok(out) if out.exit_code == Some(0) => {
            let mut guard = workspace_update_status_lock(shared.as_ref());
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
            if err == WORKSPACE_UPDATE_CANCELED_ERROR
                || finish_workspace_update_if_canceled(&shared, &control)
            {
                return;
            }
            fail_workspace_update_with_error(
                &shared,
                err,
                "请确认服务器依赖完整，并查看构建日志。",
            );
            return;
        }
    }
    if finish_workspace_update_if_canceled(&shared, &control) {
        return;
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
            let mut guard = workspace_update_status_lock(shared.as_ref());
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

async fn run_workspace_update_ui_only_job(
    workspace_root: PathBuf,
    shared: Arc<Mutex<WorkspaceUpdateStatus>>,
    control: Arc<Mutex<WorkspaceUpdateControl>>,
) {
    record_workspace_update_current_version(&workspace_root, &shared).await;
    if finish_workspace_update_if_canceled(&shared, &control) {
        return;
    }

    set_workspace_update_step(&shared, "building_ui");
    reset_workspace_update_build_logs(&shared);
    match run_workspace_update_command_streaming(
        "bash",
        &["./build-ui-nginx.sh"],
        &workspace_root,
        WORKSPACE_UPDATE_TIMEOUT_SECONDS,
        shared.clone(),
        control.clone(),
    )
    .await
    {
        Ok(out) if out.exit_code == Some(0) => {
            finish_workspace_update_succeeded(&shared, "ui_build_succeeded", out);
        }
        Ok(out) => {
            fail_workspace_update(
                &shared,
                "./build-ui-nginx.sh failed",
                "请查看 UI 编译日志；修复依赖或编译错误后再重试。",
                out,
            );
        }
        Err(err) => {
            if err == WORKSPACE_UPDATE_CANCELED_ERROR
                || finish_workspace_update_if_canceled(&shared, &control)
            {
                return;
            }
            fail_workspace_update_with_error(
                &shared,
                err,
                "请确认 UI 依赖完整，并查看编译日志。",
            );
        }
    }
}

async fn run_workspace_update_clawd_only_job(
    workspace_root: PathBuf,
    shared: Arc<Mutex<WorkspaceUpdateStatus>>,
    control: Arc<Mutex<WorkspaceUpdateControl>>,
) {
    record_workspace_update_current_version(&workspace_root, &shared).await;
    if finish_workspace_update_if_canceled(&shared, &control) {
        return;
    }

    set_workspace_update_step(&shared, "building_clawd");
    reset_workspace_update_build_logs(&shared);
    match run_workspace_update_command_streaming(
        "bash",
        &[
            "-lc",
            r#"set -euo pipefail; if [[ -f "$HOME/.cargo/env" ]]; then . "$HOME/.cargo/env"; fi; cargo build -p clawd --release"#,
        ],
        &workspace_root,
        WORKSPACE_UPDATE_TIMEOUT_SECONDS,
        shared.clone(),
        control.clone(),
    )
    .await
    {
        Ok(out) if out.exit_code == Some(0) => {
            let mut guard = workspace_update_status_lock(shared.as_ref());
            guard.exit_code = out.exit_code;
            guard.stdout_tail = out.stdout_tail;
            guard.stderr_tail = out.stderr_tail;
        }
        Ok(out) => {
            fail_workspace_update(
                &shared,
                "cargo build -p clawd --release failed",
                "请查看 clawd 编译日志；修复 Rust 编译错误后再重试。",
                out,
            );
            return;
        }
        Err(err) => {
            if err == WORKSPACE_UPDATE_CANCELED_ERROR
                || finish_workspace_update_if_canceled(&shared, &control)
            {
                return;
            }
            fail_workspace_update_with_error(&shared, err, "请确认 Rust 依赖完整，并查看编译日志。");
            return;
        }
    }
    if finish_workspace_update_if_canceled(&shared, &control) {
        return;
    }

    set_workspace_update_step(&shared, "restarting_clawd");
    match schedule_workspace_update_clawd_restart(&workspace_root) {
        Ok(()) => {
            let mut guard = workspace_update_status_lock(shared.as_ref());
            guard.status = "restarting".to_string();
            guard.step = "clawd_restart_scheduled".to_string();
            guard.finished_ts = Some(current_unix_ts());
            guard.error = None;
            guard.next_step = Some("RustClaw 正在重启，请等待 10-20 秒后刷新页面。".to_string());
        }
        Err(err) => {
            fail_workspace_update_with_error(
                &shared,
                err,
                "clawd 构建已完成，但自动重启失败。请在服务器上手动重启 clawd。",
            );
        }
    }
}

async fn run_workspace_update_release_deploy_job(
    workspace_root: PathBuf,
    shared: Arc<Mutex<WorkspaceUpdateStatus>>,
    control: Arc<Mutex<WorkspaceUpdateControl>>,
) {
    record_workspace_update_current_version(&workspace_root, &shared).await;
    if finish_workspace_update_if_canceled(&shared, &control) {
        return;
    }

    set_workspace_update_step(&shared, "downloading_release");
    reset_workspace_update_build_logs(&shared);
    {
        let mut guard = workspace_update_status_lock(shared.as_ref());
        guard.next_step = Some("release_deploy_downloading".to_string());
    }
    match run_workspace_update_command_streaming(
        "bash",
        &["-lc", release_deploy_script()],
        &workspace_root,
        WORKSPACE_UPDATE_TIMEOUT_SECONDS,
        shared.clone(),
        control.clone(),
    )
    .await
    {
        Ok(out) if out.exit_code == Some(0) => {
            let mut guard = workspace_update_status_lock(shared.as_ref());
            guard.exit_code = out.exit_code;
            guard.stdout_tail = out.stdout_tail;
            guard.stderr_tail = out.stderr_tail;
        }
        Ok(out) => {
            fail_workspace_update(
                &shared,
                "release deploy failed",
                "release_deploy_check_network_or_permissions",
                out,
            );
            return;
        }
        Err(err) => {
            if err == WORKSPACE_UPDATE_CANCELED_ERROR
                || finish_workspace_update_if_canceled(&shared, &control)
            {
                return;
            }
            fail_workspace_update_with_error(
                &shared,
                err,
                "release_deploy_check_network_or_permissions",
            );
            return;
        }
    }
    if finish_workspace_update_if_canceled(&shared, &control) {
        return;
    }

    set_workspace_update_step(&shared, "restarting_clawd");
    match schedule_workspace_update_clawd_restart(&workspace_root) {
        Ok(()) => {
            let mut guard = workspace_update_status_lock(shared.as_ref());
            guard.status = "restarting".to_string();
            guard.step = "release_restart_scheduled".to_string();
            guard.finished_ts = Some(current_unix_ts());
            guard.error = None;
            guard.next_step = Some("release_deploy_restart_scheduled".to_string());
        }
        Err(err) => {
            fail_workspace_update_with_error(
                &shared,
                err,
                "release_deploy_restart_failed",
            );
        }
    }
}

fn release_deploy_script() -> &'static str {
    r#"
set -euo pipefail

repo="${RUSTCLAW_RELEASE_REPO:-MatrixCoreX/RustClaw}"
arch="$(uname -m)"
case "$arch" in
  aarch64|arm64)
    release_prefix="pi-aarch64-"
    asset_prefix="RustClaw-pi-aarch64-"
    platform_label="pi-aarch64"
    ;;
  x86_64|amd64)
    release_prefix="ubuntu-x86_64-"
    asset_prefix="RustClaw-ubuntu-x86_64-"
    platform_label="ubuntu-x86_64"
    ;;
  *)
    echo "unsupported_release_arch=$arch" >&2
    exit 1
    ;;
esac

echo "release_repo=$repo"
echo "release_platform=$platform_label"

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/rustclaw-release-deploy.XXXXXX")"
cleanup() {
  rm -rf "$work_dir"
}
trap cleanup EXIT

meta_file="$work_dir/release.json"
python3 - "$repo" "$release_prefix" "$asset_prefix" <<'PY' > "$meta_file"
import json
import sys
import urllib.request

repo, release_prefix, asset_prefix = sys.argv[1:]
url = f"https://api.github.com/repos/{repo}/releases?per_page=50"
req = urllib.request.Request(
    url,
    headers={
        "Accept": "application/vnd.github+json",
        "User-Agent": "RustClaw-release-deploy",
    },
)
with urllib.request.urlopen(req, timeout=30) as resp:
    releases = json.load(resp)

for release in releases:
    tag = str(release.get("tag_name") or "")
    if not tag.startswith(release_prefix):
        continue
    archive = None
    checksum = None
    for asset in release.get("assets") or []:
        name = str(asset.get("name") or "")
        if name.endswith(".sha256"):
            checksum = asset
            continue
        if name.endswith(".tar.gz") and (
            name.startswith(asset_prefix) or name == f"RustClaw-{tag}.tar.gz"
        ):
            archive = asset
    if archive:
        print(json.dumps({
            "tag": tag,
            "archive_name": archive.get("name"),
            "archive_url": archive.get("browser_download_url"),
            "checksum_name": checksum.get("name") if checksum else "",
            "checksum_url": checksum.get("browser_download_url") if checksum else "",
        }))
        break
else:
    raise SystemExit(f"no release asset found for prefix {release_prefix}")
PY

python3 - "$meta_file" > "$work_dir/release.env" <<'PY'
import json
import shlex
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
    meta = json.load(f)
for key, value in meta.items():
    print(f"{key.upper()}={shlex.quote(str(value or ''))}")
PY
. "$work_dir/release.env"

echo "release_tag=$TAG"
echo "release_asset=$ARCHIVE_NAME"

archive_path="$work_dir/$ARCHIVE_NAME"
python3 - "$ARCHIVE_URL" "$archive_path" <<'PY'
import sys
import urllib.request

url, output = sys.argv[1:]
req = urllib.request.Request(url, headers={"User-Agent": "RustClaw-release-deploy"})
with urllib.request.urlopen(req, timeout=120) as resp, open(output, "wb") as f:
    while True:
        chunk = resp.read(1024 * 1024)
        if not chunk:
            break
        f.write(chunk)
PY

if [[ -n "${CHECKSUM_URL:-}" ]]; then
  checksum_path="$work_dir/$CHECKSUM_NAME"
  python3 - "$CHECKSUM_URL" "$checksum_path" <<'PY'
import sys
import urllib.request

url, output = sys.argv[1:]
req = urllib.request.Request(url, headers={"User-Agent": "RustClaw-release-deploy"})
with urllib.request.urlopen(req, timeout=30) as resp, open(output, "wb") as f:
    f.write(resp.read())
PY
  (cd "$work_dir" && sha256sum -c "$CHECKSUM_NAME")
else
  echo "release_checksum=missing"
fi

extract_dir="$work_dir/extract"
mkdir -p "$extract_dir"
tar -xzf "$archive_path" -C "$extract_dir"
package_dir="$extract_dir/RustClaw"
if [[ ! -x "$package_dir/target/release/clawd" ]]; then
  echo "release package missing target/release/clawd" >&2
  exit 1
fi

echo "deploying_binaries"
mkdir -p target/release
cp -a "$package_dir/target/release/." target/release/
chmod +x target/release/* 2>/dev/null || true

if [[ -d "$package_dir/UI/dist" ]]; then
  echo "deploying_ui_dist"
  mkdir -p UI
  rm -rf UI/dist
  cp -a "$package_dir/UI/dist" UI/dist
fi

for dir in prompts migrations scripts component_start pi_app; do
  if [[ -d "$package_dir/$dir" ]]; then
    echo "deploying_dir=$dir"
    rm -rf "$dir"
    mkdir -p "$(dirname "$dir")"
    cp -a "$package_dir/$dir" "$dir"
  fi
done

if [[ -d "$package_dir/services/wa-web-bridge" ]]; then
  echo "deploying_dir=services/wa-web-bridge"
  mkdir -p services
  rm -rf services/wa-web-bridge
  cp -a "$package_dir/services/wa-web-bridge" services/wa-web-bridge
fi

for file in README.md rustclaw install-rustclaw-cmd.sh start-all.sh start-all-bin.sh stop-rustclaw.sh; do
  if [[ -e "$package_dir/$file" ]]; then
    echo "deploying_file=$file"
    cp -a "$package_dir/$file" "$file"
  fi
done

chmod +x rustclaw install-rustclaw-cmd.sh start-all.sh start-all-bin.sh stop-rustclaw.sh 2>/dev/null || true

echo "preserved_runtime_dirs=configs,data,logs,.pids"
echo "deployed_release_tag=$TAG"
"#
}

async fn record_workspace_update_current_version(
    workspace_root: &Path,
    shared: &Arc<Mutex<WorkspaceUpdateStatus>>,
) {
    set_workspace_update_step(shared, "checking_current_version");
    if let Ok(out) =
        run_workspace_update_command("git", &["rev-parse", "--short", "HEAD"], workspace_root, 30)
            .await
    {
        if out.exit_code == Some(0) {
            let local_commit = first_output_line(&out.stdout_tail);
            let mut guard = workspace_update_status_lock(shared.as_ref());
            guard.old_commit = local_commit.clone();
            guard.new_commit = local_commit;
        }
    }
}

fn reset_workspace_update_build_logs(shared: &Arc<Mutex<WorkspaceUpdateStatus>>) {
    let mut guard = workspace_update_status_lock(shared.as_ref());
    guard.exit_code = None;
    guard.stdout_tail.clear();
    guard.stderr_tail.clear();
    guard.next_step = Some("正在编译，编译日志会持续刷新。".to_string());
}

fn finish_workspace_update_succeeded(
    shared: &Arc<Mutex<WorkspaceUpdateStatus>>,
    step: &str,
    output: WorkspaceUpdateCommandOutput,
) {
    let mut guard = workspace_update_status_lock(shared.as_ref());
    guard.status = "succeeded".to_string();
    guard.step = step.to_string();
    guard.finished_ts = Some(current_unix_ts());
    guard.exit_code = output.exit_code;
    guard.stdout_tail = output.stdout_tail;
    guard.stderr_tail = output.stderr_tail;
    guard.error = None;
    guard.next_step = None;
}

fn schedule_workspace_update_clawd_restart(workspace_root: &Path) -> Result<(), String> {
    let script_path = workspace_root.join("component_start/start-clawd.sh");
    if !script_path.exists() {
        return Err("component_start/start-clawd.sh not found in workspace root".to_string());
    }

    let workspace = workspace_root.to_string_lossy();
    let script = format!(
        "sleep 2; cd {} && mkdir -p logs .pids; \
         if [ -f .pids/clawd.pid ]; then \
           pid=\"$(cat .pids/clawd.pid 2>/dev/null || true)\"; \
           case \"$pid\" in ''|*[!0-9]*) ;; *) \
             if kill -0 \"$pid\" >/dev/null 2>&1; then \
               kill \"$pid\" >/dev/null 2>&1 || true; sleep 1; \
               kill -9 \"$pid\" >/dev/null 2>&1 || true; \
             fi; \
           esac; \
           rm -f .pids/clawd.pid; \
         fi; \
         pkill -TERM -f '[t]arget/release/clawd|cargo run -p clawd' >/dev/null 2>&1 || true; \
         sleep 1; \
         RUSTCLAW_SKIP_BANNER=1 nohup bash ./component_start/start-clawd.sh release > logs/restart-clawd.log 2>&1 &",
        shell_escape_arg(workspace.as_ref())
    );
    let spawn_result = StdCommand::new("nohup")
        .arg("bash")
        .arg("-c")
        .arg(&script)
        .current_dir(workspace_root)
        .stdin(StdProcessStdio::null())
        .stdout(StdProcessStdio::null())
        .stderr(StdProcessStdio::null())
        .spawn();

    spawn_result
        .map(|_| ())
        .map_err(|err| format!("failed to schedule clawd restart: {err}"))
}

fn set_workspace_update_step(shared: &Arc<Mutex<WorkspaceUpdateStatus>>, step: &str) {
    let mut guard = workspace_update_status_lock(shared.as_ref());
    guard.status = "running".to_string();
    guard.step = step.to_string();
}

const WORKSPACE_UPDATE_CANCELED_ERROR: &str = "workspace_update_canceled";

fn workspace_update_cancel_requested(control: &Arc<Mutex<WorkspaceUpdateControl>>) -> bool {
    workspace_update_control_lock(control.as_ref()).cancel_requested
}

fn finish_workspace_update_if_canceled(
    shared: &Arc<Mutex<WorkspaceUpdateStatus>>,
    control: &Arc<Mutex<WorkspaceUpdateControl>>,
) -> bool {
    if !workspace_update_cancel_requested(control) {
        return false;
    }
    finish_workspace_update_canceled(shared, control);
    true
}

fn finish_workspace_update_canceled(
    shared: &Arc<Mutex<WorkspaceUpdateStatus>>,
    control: &Arc<Mutex<WorkspaceUpdateControl>>,
) {
    {
        let mut guard = workspace_update_control_lock(control.as_ref());
        guard.active_child_pid = None;
    }
    let mut guard = workspace_update_status_lock(shared.as_ref());
    guard.status = "canceled".to_string();
    guard.step = "canceled".to_string();
    guard.finished_ts = Some(current_unix_ts());
    guard.exit_code = None;
    guard.error = Some("workspace update canceled by user".to_string());
    guard.next_step = Some("编译已停止；可以修复问题后重新编译。".to_string());
}

fn fail_workspace_update(
    shared: &Arc<Mutex<WorkspaceUpdateStatus>>,
    error: &str,
    next_step: &str,
    output: WorkspaceUpdateCommandOutput,
) {
    let mut guard = workspace_update_status_lock(shared.as_ref());
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
    let mut guard = workspace_update_status_lock(shared.as_ref());
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
    control: Arc<Mutex<WorkspaceUpdateControl>>,
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
    if let Some(pid) = child.id() {
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

    let started = std::time::Instant::now();
    let status = loop {
        if workspace_update_cancel_requested(&control) {
            if let Some(pid) = child.id() {
                terminate_workspace_update_process_tree(pid);
            }
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            finish_workspace_update_canceled(&shared, &control);
            return Err(WORKSPACE_UPDATE_CANCELED_ERROR.to_string());
        }
        if started.elapsed() >= std::time::Duration::from_secs(timeout_seconds) {
            if let Some(pid) = child.id() {
                terminate_workspace_update_process_tree(pid);
            }
            let _ = child.kill().await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            let mut guard = workspace_update_control_lock(control.as_ref());
            guard.active_child_pid = None;
            return Err(format!("{program} timed out after {timeout_seconds}s"));
        }

        match tokio::time::timeout(std::time::Duration::from_millis(500), child.wait()).await {
            Ok(Ok(status)) => break status,
            Ok(Err(err)) => return Err(format!("failed to wait for {program}: {err}")),
            Err(_) => continue,
        }
    };

    let _ = stdout_task.await;
    let _ = stderr_task.await;
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

fn terminate_workspace_update_process_tree(pid: u32) {
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
