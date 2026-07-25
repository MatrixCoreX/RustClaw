const RELEASE_CHECK_SUCCESS_TTL_SECONDS: i64 = 300;
const RELEASE_CHECK_FAILURE_TTL_SECONDS: i64 = 30;
const RELEASE_CHECK_TIMEOUT_SECONDS: u64 = 30;

#[derive(Debug, Default, Deserialize)]
struct WorkspaceUpdateQuery {
    #[serde(default)]
    refresh_release: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum WorkspaceUpdateMode {
    Full,
    UiOnly,
    ClawdOnly,
    ReleaseDeploy,
    SourceCheckout,
}

impl WorkspaceUpdateMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::UiOnly => "ui_only",
            Self::ClawdOnly => "clawd_only",
            Self::ReleaseDeploy => "release_deploy",
            Self::SourceCheckout => "source_checkout",
        }
    }
}

fn workspace_source_update_available(workspace_root: &Path) -> bool {
    workspace_root.join(".git").exists()
}

fn workspace_installation_kind(workspace_root: &Path) -> &'static str {
    if workspace_source_update_available(workspace_root) {
        "source_checkout"
    } else if workspace_root.join(".release-tag").is_file()
        || workspace_root.join("VERSION").is_file()
    {
        "release_package"
    } else {
        "standalone"
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

fn clear_workspace_update_next_step(status: &mut WorkspaceUpdateStatus) {
    status.next_step = None;
    status.next_step_key = None;
    status.next_step_args.clear();
}

fn set_workspace_update_next_step(status: &mut WorkspaceUpdateStatus, key: &str) {
    status.next_step = None;
    status.next_step_key = Some(key.to_string());
    status.next_step_args.clear();
}

fn set_workspace_update_next_step_args(
    status: &mut WorkspaceUpdateStatus,
    key: &str,
    args: BTreeMap<String, Value>,
) {
    status.next_step = None;
    status.next_step_key = Some(key.to_string());
    status.next_step_args = args;
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
    Query(query): Query<WorkspaceUpdateQuery>,
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
    let status = refresh_workspace_update_versions(
        &state.skill_rt.workspace_root,
        shared,
        query.refresh_release,
    )
    .await;
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
    force_release_refresh: bool,
) -> WorkspaceUpdateStatus {
    let snapshot = workspace_update_status_lock(shared.as_ref()).clone();
    if matches!(snapshot.status.as_str(), "running" | "restarting") {
        return snapshot;
    }
    let now = current_unix_ts();
    let release_check_due =
        latest_release_check_due(&snapshot, force_release_refresh, now);
    let source_update_available = workspace_source_update_available(workspace_root);
    let installation_kind = workspace_installation_kind(workspace_root);

    let (git_versions, latest_release_result) = tokio::join!(
        async {
            if !source_update_available {
                return (None, None, None);
            }
            let local_commit = run_workspace_update_command(
                "git",
                &["rev-parse", "--short", "HEAD"],
                workspace_root,
                10,
            )
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
            (local_commit, remote_commit, fetch_output)
        },
        async {
            if release_check_due {
                Some(fetch_latest_release_tag().await)
            } else {
                None
            }
        }
    );
    let (local_commit, remote_commit, fetch_output) = git_versions;

    let mut guard = workspace_update_status_lock(shared.as_ref());
    guard.installation_kind = installation_kind.to_string();
    guard.source_update_available = source_update_available;
    if !source_update_available {
        guard.old_commit = None;
        guard.new_commit = None;
        guard.remote_commit = None;
        if guard.status == "idle" {
            guard.step = "idle".to_string();
            guard.exit_code = None;
            guard.stdout_tail.clear();
            guard.stderr_tail.clear();
            guard.error = None;
            clear_workspace_update_next_step(&mut guard);
        }
    }
    if let Some(local_commit) = local_commit.clone() {
        guard.old_commit = Some(local_commit.clone());
        if matches!(guard.status.as_str(), "idle" | "up_to_date" | "succeeded") {
            guard.new_commit = Some(local_commit);
        }
    }
    if let Some(remote_commit) = remote_commit.clone() {
        guard.remote_commit = Some(remote_commit);
    }
    if let Some(result) = latest_release_result {
        guard.latest_release_checked_ts = Some(current_unix_ts());
        match result {
            Ok(latest_release_tag) => {
                guard.latest_release_tag = Some(latest_release_tag);
                guard.latest_release_check_status = "available".to_string();
                guard.latest_release_check_error = None;
            }
            Err(error) => {
                if !error.can_use_cached_tag() {
                    guard.latest_release_tag = None;
                }
                guard.latest_release_check_status = if guard.latest_release_tag.is_some() {
                    "stale"
                } else {
                    "unavailable"
                }
                .to_string();
                guard.latest_release_check_error = Some(error.as_str().to_string());
            }
        }
    }
    match (local_commit.as_deref(), remote_commit.as_deref()) {
        (Some(local), Some(remote)) if local == remote => {
            guard.status = "up_to_date".to_string();
            guard.step = "already_latest".to_string();
            guard.exit_code = None;
            guard.stdout_tail.clear();
            guard.stderr_tail.clear();
            guard.error = None;
            clear_workspace_update_next_step(&mut guard);
        }
        (Some(_), Some(_))
            if matches!(guard.status.as_str(), "idle" | "up_to_date" | "succeeded") =>
        {
            guard.status = "idle".to_string();
            guard.step = "idle".to_string();
            clear_workspace_update_next_step(&mut guard);
        }
        _ => {}
    }
    if let Some(out) = fetch_output {
        if out.exit_code != Some(0) && guard.status == "idle" {
            guard.stderr_tail = out.stderr_tail;
            guard.stdout_tail = out.stdout_tail;
        }
    }
    guard.clone()
}

fn latest_release_check_due(
    status: &WorkspaceUpdateStatus,
    force_refresh: bool,
    now: i64,
) -> bool {
    if force_refresh {
        return true;
    }
    let ttl = if status.latest_release_check_error.is_some() {
        RELEASE_CHECK_FAILURE_TTL_SECONDS
    } else {
        RELEASE_CHECK_SUCCESS_TTL_SECONDS
    };
    status
        .latest_release_checked_ts
        .map(|checked| now.saturating_sub(checked) >= ttl)
        .unwrap_or(true)
}

fn release_platform_prefixes() -> Option<(&'static str, &'static str)> {
    match std::env::consts::ARCH {
        "aarch64" => Some(("pi-aarch64-", "RustClaw-pi-aarch64-")),
        "x86_64" => Some(("ubuntu-x86_64-", "RustClaw-ubuntu-x86_64-")),
        _ => None,
    }
}

fn select_latest_compatible_release_tag(
    releases: &[Value],
    release_prefix: &str,
    asset_prefix: &str,
) -> Option<String> {
    releases.iter().find_map(|release| {
        if release.get("draft").and_then(Value::as_bool) == Some(true)
            || release.get("prerelease").and_then(Value::as_bool) == Some(true)
        {
            return None;
        }
        let tag = release.get("tag_name")?.as_str()?.trim();
        if !tag.starts_with(release_prefix) {
            return None;
        }
        let expected_archive = format!("RustClaw-{tag}.tar.gz");
        let has_compatible_archive = release
            .get("assets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|asset| asset.get("name").and_then(Value::as_str))
            .any(|name| {
                name.ends_with(".tar.gz")
                    && (name.starts_with(asset_prefix) || name == expected_archive)
            });
        has_compatible_archive.then(|| tag.to_string())
    })
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum LatestReleaseLookupError {
    UnsupportedPlatform,
    ClientBuildFailed,
    RequestTimedOut,
    RequestFailed,
    HttpStatus,
    InvalidResponse,
    CompatibleReleaseNotFound,
}

impl LatestReleaseLookupError {
    fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedPlatform => "release_platform_unsupported",
            Self::ClientBuildFailed => "release_lookup_client_build_failed",
            Self::RequestTimedOut => "release_lookup_timed_out",
            Self::RequestFailed => "release_lookup_request_failed",
            Self::HttpStatus => "release_lookup_http_status",
            Self::InvalidResponse => "release_lookup_invalid_response",
            Self::CompatibleReleaseNotFound => "compatible_release_not_found",
        }
    }

    fn can_use_cached_tag(self) -> bool {
        matches!(
            self,
            Self::ClientBuildFailed
                | Self::RequestTimedOut
                | Self::RequestFailed
                | Self::HttpStatus
                | Self::InvalidResponse
        )
    }
}

async fn fetch_latest_release_tag() -> Result<String, LatestReleaseLookupError> {
    let (release_prefix, asset_prefix) =
        release_platform_prefixes().ok_or(LatestReleaseLookupError::UnsupportedPlatform)?;
    let repo = std::env::var("RUSTCLAW_RELEASE_REPO")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "MatrixCoreX/RustClaw".to_string());
    let url = format!(
        "https://api.github.com/repos/{}/releases?per_page=20",
        repo.trim()
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(RELEASE_CHECK_TIMEOUT_SECONDS))
        .build()
        .map_err(|_| LatestReleaseLookupError::ClientBuildFailed)?;
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header(reqwest::header::USER_AGENT, "RustClaw-version-check")
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                LatestReleaseLookupError::RequestTimedOut
            } else {
                LatestReleaseLookupError::RequestFailed
            }
        })?;
    if !response.status().is_success() {
        return Err(LatestReleaseLookupError::HttpStatus);
    }
    let releases = response
        .json::<Vec<Value>>()
        .await
        .map_err(|_| LatestReleaseLookupError::InvalidResponse)?;
    select_latest_compatible_release_tag(&releases, release_prefix, asset_prefix)
        .ok_or(LatestReleaseLookupError::CompatibleReleaseNotFound)
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

async fn start_workspace_update_source_checkout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<WorkspaceUpdateStatus>>) {
    start_workspace_update_with_mode(state, headers, WorkspaceUpdateMode::SourceCheckout).await
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
        let source_update_available =
            workspace_source_update_available(&state.skill_rt.workspace_root);
        guard.installation_kind =
            workspace_installation_kind(&state.skill_rt.workspace_root).to_string();
        guard.source_update_available = source_update_available;
        match mode {
            WorkspaceUpdateMode::Full
            | WorkspaceUpdateMode::UiOnly
            | WorkspaceUpdateMode::ClawdOnly
                if !source_update_available =>
            {
                return workspace_update_api_error(
                    StatusCode::PRECONDITION_FAILED,
                    "workspace_update_source_checkout_required",
                    Some(guard.clone()),
                );
            }
            WorkspaceUpdateMode::SourceCheckout if source_update_available => {
                return workspace_update_api_error(
                    StatusCode::CONFLICT,
                    "workspace_update_source_checkout_already_enabled",
                    Some(guard.clone()),
                );
            }
            _ => {}
        }
        *guard = begin_workspace_update_status(&guard, mode);
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

fn begin_workspace_update_status(
    previous: &WorkspaceUpdateStatus,
    mode: WorkspaceUpdateMode,
) -> WorkspaceUpdateStatus {
    WorkspaceUpdateStatus {
        status: "running".to_string(),
        step: "starting".to_string(),
        mode: mode.as_str().to_string(),
        installation_kind: previous.installation_kind.clone(),
        source_update_available: previous.source_update_available,
        started_ts: Some(current_unix_ts()),
        latest_release_tag: previous.latest_release_tag.clone(),
        latest_release_check_status: previous.latest_release_check_status.clone(),
        latest_release_check_error: previous.latest_release_check_error.clone(),
        latest_release_checked_ts: previous.latest_release_checked_ts,
        ..WorkspaceUpdateStatus::default()
    }
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
        set_workspace_update_next_step(&mut status_guard, "workspace_update.cancel_requested");
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
        WorkspaceUpdateMode::SourceCheckout => {
            run_workspace_update_source_checkout_job(workspace_root, shared, control).await;
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
                "workspace_update.invalid_git_repo",
                out,
            );
            return;
        }
        Err(err) => {
            fail_workspace_update_with_error(
                &shared,
                err,
                "workspace_update.git_unavailable",
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
                "workspace_update.remote_fetch_required_failed",
                out,
            );
            return;
        }
        Err(err) => {
            fail_workspace_update_with_error(
                &shared,
                err,
                "workspace_update.remote_fetch_required_failed",
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
                "workspace_update.upstream_missing",
                out,
            );
            return;
        }
        Err(err) => {
            fail_workspace_update_with_error(
                &shared,
                err,
                "workspace_update.upstream_missing",
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
                            "workspace_update.pull_conflict_detection_failed",
                        );
                            return;
                        }
                    };
                if conflict_paths.is_empty() {
                    fail_workspace_update(
                        &shared,
                        "git pull --ff-only failed",
                        "workspace_update.pull_failed_no_conflicts",
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
                        "workspace_update.conflict_overwrite_failed",
                    );
                    return;
                }
                {
                    let mut guard = workspace_update_status_lock(shared.as_ref());
                    let mut args = BTreeMap::new();
                    args.insert("count".to_string(), json!(conflict_paths.len()));
                    set_workspace_update_next_step_args(
                        &mut guard,
                        "workspace_update.conflicts_overwritten_retrying_pull",
                        args,
                    );
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
                            "workspace_update.pull_failed_after_conflict_overwrite",
                            out,
                        );
                        return;
                    }
                    Err(err) => {
                        fail_workspace_update_with_error(
                            &shared,
                            err,
                            "workspace_update.pull_failed_after_conflict_overwrite",
                        );
                        return;
                    }
                }
            }
            Err(err) => {
                fail_workspace_update_with_error(
                    &shared,
                    err,
                    "workspace_update.pull_failed_preserved",
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
            set_workspace_update_next_step(
                &mut guard,
                "workspace_update.no_remote_changes_building",
            );
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
        set_workspace_update_next_step(&mut guard, "workspace_update.build_logs_refreshing");
    }
    match run_workspace_update_command_streaming(
        "bash",
        &["./build-all.sh"],
        &workspace_root,
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
                "workspace_update.full_build_failed",
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
                "workspace_update.full_build_dependency_check",
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
            set_workspace_update_next_step(&mut guard, "workspace_update.restart_wait");
        }
        Err(err) => {
            fail_workspace_update_with_error(
                &shared,
                format!("failed to schedule clawd restart: {err}"),
                "workspace_update.full_restart_failed",
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
        &["./build-ui-nginx.sh", "--deploy-if-configured"],
        &workspace_root,
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
                "workspace_update.ui_build_failed",
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
                "workspace_update.ui_dependency_check",
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
                "workspace_update.clawd_build_failed",
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
                "workspace_update.clawd_dependency_check",
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
            guard.step = "clawd_restart_scheduled".to_string();
            guard.finished_ts = Some(current_unix_ts());
            guard.error = None;
            set_workspace_update_next_step(&mut guard, "workspace_update.restart_wait");
        }
        Err(err) => {
            fail_workspace_update_with_error(
                &shared,
                err,
                "workspace_update.clawd_restart_failed",
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
        set_workspace_update_next_step(
            &mut guard,
            "workspace_update.release_deploy_downloading",
        );
    }
    match run_workspace_update_command_streaming(
        "bash",
        &[
            "./deploy-github-release.sh",
            "--root",
            ".",
            "--no-restart",
        ],
        &workspace_root,
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
                "workspace_update.release_deploy_check_network_or_permissions",
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
                "workspace_update.release_deploy_check_network_or_permissions",
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
            set_workspace_update_next_step(
                &mut guard,
                "workspace_update.release_deploy_restart_scheduled",
            );
        }
        Err(err) => {
            fail_workspace_update_with_error(
                &shared,
                err,
                "workspace_update.release_deploy_restart_failed",
            );
        }
    }
}

async fn run_workspace_update_source_checkout_job(
    workspace_root: PathBuf,
    shared: Arc<Mutex<WorkspaceUpdateStatus>>,
    control: Arc<Mutex<WorkspaceUpdateControl>>,
) {
    set_workspace_update_step(&shared, "cloning_source_checkout");
    reset_workspace_update_build_logs(&shared);
    {
        let mut guard = workspace_update_status_lock(shared.as_ref());
        set_workspace_update_next_step(
            &mut guard,
            "workspace_update.source_checkout_cloning",
        );
    }
    match run_workspace_update_command_streaming(
        "bash",
        &[
            "./scripts/switch-to-source-checkout.sh",
            "--root",
            ".",
        ],
        &workspace_root,
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
            guard.installation_kind = "source_checkout".to_string();
            guard.source_update_available = true;
        }
        Ok(out) => {
            fail_workspace_update(
                &shared,
                "source_checkout_migration_failed",
                "workspace_update.source_checkout_failed",
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
                "workspace_update.source_checkout_failed",
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
            guard.step = "source_checkout_restart_scheduled".to_string();
            guard.finished_ts = Some(current_unix_ts());
            guard.error = None;
            set_workspace_update_next_step(
                &mut guard,
                "workspace_update.source_checkout_restart_scheduled",
            );
        }
        Err(err) => {
            fail_workspace_update_with_error(
                &shared,
                err,
                "workspace_update.source_checkout_restart_failed",
            );
        }
    }
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
    set_workspace_update_next_step(&mut guard, "workspace_update.build_logs_refreshing");
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
    clear_workspace_update_next_step(&mut guard);
}

fn schedule_workspace_update_clawd_restart(workspace_root: &Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("INVOCATION_ID").is_some() {
        return schedule_workspace_update_systemd_restart();
    }

    schedule_workspace_update_direct_restart(workspace_root)
}

#[cfg(target_os = "linux")]
fn schedule_workspace_update_systemd_restart() -> Result<(), String> {
    let service_unit =
        std::env::var("RUSTCLAW_SYSTEMD_UNIT").unwrap_or_else(|_| "rustclaw.service".to_string());
    if !is_safe_systemd_unit_name(&service_unit) {
        return Err("RUSTCLAW_SYSTEMD_UNIT contains unsupported characters".to_string());
    }

    let systemctl = ["/usr/bin/systemctl", "/bin/systemctl"]
        .into_iter()
        .find(|path| Path::new(path).is_file())
        .ok_or_else(|| "systemctl not found for systemd-managed restart".to_string())?;
    let transient_unit = format!(
        "rustclaw-restart-{}-{}",
        std::process::id(),
        current_unix_ts()
    );
    let status = StdCommand::new("systemd-run")
        .arg("--quiet")
        .arg(format!("--unit={transient_unit}"))
        .arg("--on-active=2s")
        .arg("--property=Type=oneshot")
        .arg(systemctl)
        .arg("restart")
        .arg(&service_unit)
        .stdin(StdProcessStdio::null())
        .stdout(StdProcessStdio::null())
        .stderr(StdProcessStdio::null())
        .status()
        .map_err(|err| format!("failed to schedule systemd restart: {err}"))?;
    if !status.success() {
        return Err(format!(
            "systemd-run failed to schedule restart for {service_unit}"
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn is_safe_systemd_unit_name(unit: &str) -> bool {
    !unit.is_empty()
        && unit
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'@'))
}

fn schedule_workspace_update_direct_restart(workspace_root: &Path) -> Result<(), String> {
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
    set_workspace_update_next_step(&mut guard, "workspace_update.canceled");
}

fn fail_workspace_update(
    shared: &Arc<Mutex<WorkspaceUpdateStatus>>,
    error: &str,
    next_step_key: &str,
    output: WorkspaceUpdateCommandOutput,
) {
    let mut guard = workspace_update_status_lock(shared.as_ref());
    guard.status = "failed".to_string();
    guard.finished_ts = Some(current_unix_ts());
    guard.exit_code = output.exit_code;
    guard.stdout_tail = output.stdout_tail;
    guard.stderr_tail = output.stderr_tail;
    guard.error = Some(error.to_string());
    set_workspace_update_next_step(&mut guard, next_step_key);
}

fn fail_workspace_update_with_error(
    shared: &Arc<Mutex<WorkspaceUpdateStatus>>,
    error: impl Into<String>,
    next_step_key: &str,
) {
    let mut guard = workspace_update_status_lock(shared.as_ref());
    guard.status = "failed".to_string();
    guard.finished_ts = Some(current_unix_ts());
    guard.error = Some(error.into());
    set_workspace_update_next_step(&mut guard, next_step_key);
}

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
            git_workspace_name_list_raw(&["diff", "--name-only", "-z", "--"], batch, workspace_root),
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

    Ok(WorkspaceUpdateConflictPaths {
        tracked: tracked_dirty.into_iter().collect(),
        untracked: local_untracked.into_iter().collect(),
    })
}

async fn overwrite_workspace_update_conflict_paths(
    workspace_root: &Path,
    paths: &WorkspaceUpdateConflictPaths,
) -> Result<(), String> {
    for batch in paths.tracked.chunks(WORKSPACE_UPDATE_PATH_BATCH_SIZE) {
        let mut args = vec![
            "restore".to_string(),
            "--source".to_string(),
            "HEAD".to_string(),
            "--staged".to_string(),
            "--worktree".to_string(),
            "--".to_string(),
        ];
        args.extend(batch.iter().cloned());
        let out = run_workspace_update_command_args("git", &args, workspace_root, 600).await?;
        if out.exit_code != Some(0) {
            return Err(format!(
                "git restore conflict files failed: {}",
                workspace_update_output_detail(&out)
            ));
        }
    }

    for batch in paths.untracked.chunks(WORKSPACE_UPDATE_PATH_BATCH_SIZE) {
        let mut args = vec!["clean".to_string(), "-fd".to_string(), "--".to_string()];
        args.extend(batch.iter().cloned());
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
