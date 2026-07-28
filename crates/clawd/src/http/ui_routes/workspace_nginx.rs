#[derive(Debug, Clone, Serialize)]
struct NginxUiStatus {
    supported: bool,
    platform: String,
    installed: bool,
    running: bool,
    configured: bool,
    ui_deployed: bool,
    clawd_exposure: &'static str,
}

async fn get_nginx_ui_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<NginxUiStatus>>) {
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
                error: Some("nginx_admin_required".to_string()),
            }),
        );
    }

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(collect_nginx_ui_status()),
            error: None,
        }),
    )
}

async fn start_workspace_update_nginx_enable(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<WorkspaceUpdateStatus>>) {
    start_workspace_update_with_mode(state, headers, WorkspaceUpdateMode::NginxEnable).await
}

async fn start_workspace_update_nginx_disable(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<WorkspaceUpdateStatus>>) {
    start_workspace_update_with_mode(state, headers, WorkspaceUpdateMode::NginxDisable).await
}

fn collect_nginx_ui_status() -> NginxUiStatus {
    let supported = matches!(std::env::consts::OS, "linux" | "macos");
    let installed = nginx_executable_exists();
    let running = installed
        && daemon_process_pids_by_name("nginx")
            .is_some_and(|pids| !pids.is_empty());
    let config = nginx_config_candidates()
        .into_iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
        .filter(|content| nginx_config_is_rustclaw_site(content));
    let configured = config.is_some();
    let ui_deployed = config
        .as_deref()
        .and_then(nginx_ui_root_from_config)
        .is_some_and(|root| root.join("index.html").is_file());

    NginxUiStatus {
        supported,
        platform: std::env::consts::OS.to_string(),
        installed,
        running,
        configured,
        ui_deployed,
        clawd_exposure: "loopback_only",
    }
}

fn nginx_executable_exists() -> bool {
    let path_match = std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|path| path.join("nginx").is_file())
    });
    path_match
        || [
            "/usr/sbin/nginx",
            "/sbin/nginx",
            "/usr/local/sbin/nginx",
            "/usr/local/bin/nginx",
            "/opt/homebrew/bin/nginx",
        ]
        .iter()
        .any(|path| Path::new(path).is_file())
}

fn nginx_config_candidates() -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        return vec![
            PathBuf::from("/opt/homebrew/etc/nginx/servers/rustclaw-ui.conf"),
            PathBuf::from("/usr/local/etc/nginx/servers/rustclaw-ui.conf"),
        ];
    }
    vec![
        PathBuf::from("/etc/nginx/sites-enabled/rustclaw-ui.conf"),
        PathBuf::from("/etc/nginx/conf.d/rustclaw-ui.conf"),
    ]
}

fn nginx_config_is_rustclaw_site(content: &str) -> bool {
    let proxy_upstreams = content
        .split("proxy_pass ")
        .skip(1)
        .filter_map(|segment| segment.split(';').next().map(str::trim))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    content.lines().any(|line| line.contains("RustClaw UI"))
        && content.contains("location ^~ /v1/")
        && content.contains("location ^~ /webd/")
        && proxy_upstreams.len() >= 2
        && proxy_upstreams
            .iter()
            .all(|upstream| !proxy_upstream_targets_internal_clawd(upstream))
}

fn proxy_upstream_targets_internal_clawd(upstream: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(upstream) else {
        return false;
    };
    if url.port_or_known_default() != Some(8787) {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let normalized_host = host.trim_start_matches('[').trim_end_matches(']');
    normalized_host.eq_ignore_ascii_case("localhost")
        || normalized_host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn nginx_ui_root_from_config(content: &str) -> Option<PathBuf> {
    content.lines().find_map(|line| {
        let value = line.trim().strip_prefix("root ")?.strip_suffix(';')?.trim();
        (!value.is_empty()).then(|| PathBuf::from(value))
    })
}

async fn run_workspace_update_nginx_job(
    workspace_root: PathBuf,
    shared: Arc<Mutex<WorkspaceUpdateStatus>>,
    control: Arc<Mutex<WorkspaceUpdateControl>>,
) {
    record_workspace_update_current_version(&workspace_root, &shared).await;
    if finish_workspace_update_if_canceled(&shared, &control) {
        return;
    }

    set_workspace_update_step(&shared, "enabling_nginx");
    reset_workspace_update_build_logs(&shared);

    let script = "./deploy-ui-nginx.sh";
    let args = &["./deploy-ui-nginx.sh", "--upgrade-nginx"];
    match run_workspace_update_command_streaming(
        "bash",
        args,
        &workspace_root,
        shared.clone(),
        control.clone(),
    )
    .await
    {
        Ok(out) if out.exit_code == Some(0) => {
            finish_workspace_update_succeeded(&shared, "nginx_enabled", out);
        }
        Ok(out) => {
            fail_workspace_update(
                &shared,
                "nginx_enable_failed",
                "workspace_update.nginx_enable_failed",
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
                format!("{script}: {err}"),
                "workspace_update.nginx_command_failed",
            );
        }
    }
}

async fn run_workspace_update_nginx_disable_job(
    workspace_root: PathBuf,
    shared: Arc<Mutex<WorkspaceUpdateStatus>>,
    control: Arc<Mutex<WorkspaceUpdateControl>>,
) {
    record_workspace_update_current_version(&workspace_root, &shared).await;
    if finish_workspace_update_if_canceled(&shared, &control) {
        return;
    }

    set_workspace_update_step(&shared, "disabling_nginx");
    reset_workspace_update_build_logs(&shared);
    match run_workspace_update_command_streaming(
        "bash",
        &["./scripts/disable-nginx-web.sh"],
        &workspace_root,
        shared.clone(),
        control.clone(),
    )
    .await
    {
        Ok(out) if out.exit_code == Some(0) => {
            finish_workspace_update_succeeded(&shared, "nginx_disabled", out);
        }
        Ok(out) => {
            fail_workspace_update(
                &shared,
                "nginx_disable_failed",
                "workspace_update.nginx_disable_failed",
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
                format!("./scripts/disable-nginx-web.sh: {err}"),
                "workspace_update.nginx_command_failed",
            );
        }
    }
}
