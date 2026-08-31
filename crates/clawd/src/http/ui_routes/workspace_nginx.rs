#[derive(Debug, Clone, Serialize)]
struct NginxUiStatus {
    supported: bool,
    platform: String,
    installed: bool,
    running: bool,
    configured: bool,
    ui_deployed: bool,
    clawd_exposure: &'static str,
    local_https_supported: bool,
    local_https_prepared: bool,
    local_https_enabled: bool,
    local_https_ca_fingerprint_sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LocalHttpsEnableRequest {
    ca_fingerprint_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
struct LocalMdnsStatus {
    supported: bool,
    platform: String,
    hostname: String,
    mdns_name: String,
    responder_installed: bool,
    responder_running: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalMdnsUpdateRequest {
    hostname: String,
}

#[derive(Debug, Serialize)]
struct LocalMdnsUpdateResult {
    status: LocalMdnsStatus,
    previous_mdns_name: String,
    https_certificate_refreshed: bool,
    https_refresh_error_code: Option<&'static str>,
}

static LOCAL_MDNS_UPDATE_RUNNING: AtomicBool = AtomicBool::new(false);

struct LocalMdnsUpdateLease;

impl Drop for LocalMdnsUpdateLease {
    fn drop(&mut self) {
        LOCAL_MDNS_UPDATE_RUNNING.store(false, Ordering::Release);
    }
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

async fn get_local_mdns_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<LocalMdnsStatus>>) {
    if let Err((status, Json(resp))) = require_admin_ui_identity(&state, &headers) {
        return (
            status,
            Json(ApiResponse {
                ok: resp.ok,
                data: None,
                error: resp.error,
            }),
        );
    }
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(collect_local_mdns_status()),
            error: None,
        }),
    )
}

async fn update_local_mdns(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<LocalMdnsUpdateRequest>,
) -> (StatusCode, Json<ApiResponse<LocalMdnsUpdateResult>>) {
    if let Err((status, Json(resp))) = require_admin_ui_identity(&state, &headers) {
        return (
            status,
            Json(ApiResponse {
                ok: resp.ok,
                data: None,
                error: resp.error,
            }),
        );
    }
    let Some(hostname) = normalize_local_mdns_hostname(&request.hostname) else {
        return local_mdns_api_error(StatusCode::BAD_REQUEST, "local_mdns_hostname_invalid");
    };
    if LOCAL_MDNS_UPDATE_RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return local_mdns_api_error(StatusCode::CONFLICT, "local_mdns_update_in_progress");
    }
    let _lease = LocalMdnsUpdateLease;
    let previous = collect_local_mdns_status();
    let workspace_root = state.skill_rt.workspace_root.clone();
    let args = vec![
        "./scripts/configure-local-mdns.sh".to_string(),
        "--set".to_string(),
        hostname.clone(),
    ];
    let command = run_workspace_update_command_args("bash", &args, &workspace_root, 300).await;
    match command {
        Ok(output) if output.exit_code == Some(0) => {}
        Ok(_) => {
            return local_mdns_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "local_mdns_update_failed",
            );
        }
        Err(_) => {
            return local_mdns_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "local_mdns_command_failed",
            );
        }
    }

    let nginx = collect_nginx_ui_status();
    let mut https_certificate_refreshed = false;
    let mut https_refresh_error_code = None;
    if nginx.local_https_prepared {
        let mut https_args = vec![
            "./scripts/configure-local-lan-https.sh".to_string(),
            "--hostname".to_string(),
            hostname,
        ];
        if !nginx.local_https_enabled {
            https_args.push("--prepare-only".to_string());
        }
        match run_workspace_update_command_args("bash", &https_args, &workspace_root, 600).await {
            Ok(output) if output.exit_code == Some(0) => https_certificate_refreshed = true,
            _ => https_refresh_error_code = Some("local_mdns_https_refresh_failed"),
        }
    }

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(LocalMdnsUpdateResult {
                status: collect_local_mdns_status(),
                previous_mdns_name: previous.mdns_name,
                https_certificate_refreshed,
                https_refresh_error_code,
            }),
            error: None,
        }),
    )
}

fn require_admin_ui_identity(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthIdentity, (StatusCode, Json<ApiResponse<Value>>)> {
    let identity = require_ui_identity(state, headers)?;
    if !identity.role.eq_ignore_ascii_case("admin") {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("admin_required".to_string()),
            }),
        ));
    }
    Ok(identity)
}

fn local_mdns_api_error(
    status: StatusCode,
    error: &str,
) -> (StatusCode, Json<ApiResponse<LocalMdnsUpdateResult>>) {
    (
        status,
        Json(ApiResponse {
            ok: false,
            data: None,
            error: Some(error.to_string()),
        }),
    )
}

fn normalize_local_mdns_hostname(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    let hostname = normalized.strip_suffix(".local").unwrap_or(&normalized);
    let valid = !hostname.is_empty()
        && hostname.len() <= 63
        && !hostname.starts_with('-')
        && !hostname.ends_with('-')
        && hostname
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    valid.then(|| hostname.to_string())
}

fn collect_local_mdns_status() -> LocalMdnsStatus {
    let platform = std::env::consts::OS.to_string();
    let supported = matches!(std::env::consts::OS, "linux" | "macos");
    let hostname = current_local_mdns_hostname().unwrap_or_default();
    let mdns_name = if hostname.is_empty() {
        String::new()
    } else {
        format!("{hostname}.local")
    };
    let (responder_installed, responder_running) = if cfg!(target_os = "macos") {
        (
            Path::new("/usr/sbin/mDNSResponder").exists(),
            daemon_process_pids_by_name("mDNSResponder").is_some_and(|pids| !pids.is_empty()),
        )
    } else if cfg!(target_os = "linux") {
        (
            ["/usr/sbin/avahi-daemon", "/usr/local/sbin/avahi-daemon"]
                .iter()
                .any(|path| Path::new(path).is_file()),
            daemon_process_pids_by_name("avahi-daemon").is_some_and(|pids| !pids.is_empty()),
        )
    } else {
        (false, false)
    };
    LocalMdnsStatus {
        supported,
        platform,
        hostname,
        mdns_name,
        responder_installed,
        responder_running,
    }
}

fn current_local_mdns_hostname() -> Option<String> {
    let output = if cfg!(target_os = "macos") {
        StdCommand::new("scutil")
            .args(["--get", "LocalHostName"])
            .output()
            .ok()
    } else {
        StdCommand::new("hostname").arg("-s").output().ok()
    }?;
    if !output.status.success() {
        return None;
    }
    normalize_local_mdns_hostname(&String::from_utf8_lossy(&output.stdout))
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

async fn start_local_https_prepare(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<WorkspaceUpdateStatus>>) {
    start_workspace_update_with_mode(state, headers, WorkspaceUpdateMode::LocalHttpsPrepare).await
}

async fn start_local_https_enable(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<LocalHttpsEnableRequest>,
) -> (StatusCode, Json<ApiResponse<WorkspaceUpdateStatus>>) {
    let expected = local_https_ca_fingerprint();
    let submitted = request.ca_fingerprint_sha256.trim();
    let fingerprint_matches = match expected.as_deref() {
        Some(fingerprint) => fingerprint.eq_ignore_ascii_case(submitted),
        None => submitted.is_empty(),
    };
    if !fingerprint_matches {
        return workspace_update_api_error(
            StatusCode::CONFLICT,
            "local_https_ca_fingerprint_mismatch",
            None,
        );
    }
    start_workspace_update_with_mode(state, headers, WorkspaceUpdateMode::LocalHttpsEnable).await
}

async fn start_local_https_restore(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<WorkspaceUpdateStatus>>) {
    start_workspace_update_with_mode(state, headers, WorkspaceUpdateMode::LocalHttpsRestore).await
}

async fn download_local_https_ca() -> axum::response::Response {
    let certificate_path = local_https_public_root().join("local-device-ca.crt");
    let Ok(certificate) = std::fs::read(certificate_path) else {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<Value> {
                ok: false,
                data: None,
                error: Some("local_https_ca_not_prepared".to_string()),
            }),
        )
            .into_response();
    };
    let mut response = axum::response::Response::new(axum::body::Body::from(certificate));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/x-x509-ca-cert"),
    );
    response.headers_mut().insert(
        axum::http::header::CONTENT_DISPOSITION,
        axum::http::HeaderValue::from_static("attachment; filename=\"local-device-ca.crt\""),
    );
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        axum::http::HeaderValue::from_static("nosniff"),
    );
    response
}

fn collect_nginx_ui_status() -> NginxUiStatus {
    let supported = matches!(std::env::consts::OS, "linux" | "macos");
    let installed = nginx_executable_exists();
    let running =
        installed && daemon_process_pids_by_name("nginx").is_some_and(|pids| !pids.is_empty());
    let config = nginx_config_candidates()
        .into_iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
        .filter(|content| nginx_config_is_agent_site(content));
    let configured = config.is_some();
    let ui_deployed = config
        .as_deref()
        .and_then(nginx_ui_root_from_config)
        .is_some_and(|root| root.join("index.html").is_file());
    let local_https_prepared = local_https_public_root()
        .join("local-device-ca.crt")
        .is_file()
        && local_https_ca_fingerprint().is_some();
    let local_https_enabled = config.as_deref().is_some_and(nginx_local_https_is_enabled);

    NginxUiStatus {
        supported,
        platform: std::env::consts::OS.to_string(),
        installed,
        running,
        configured,
        ui_deployed,
        clawd_exposure: "loopback_only",
        local_https_supported: cfg!(target_os = "linux"),
        local_https_prepared,
        local_https_enabled,
        local_https_ca_fingerprint_sha256: local_https_ca_fingerprint(),
    }
}

fn local_https_public_root() -> PathBuf {
    std::env::var_os("APP_LAN_HTTPS_PUBLIC_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/agent-runtime/public"))
}

fn local_https_ca_fingerprint() -> Option<String> {
    let value =
        std::fs::read_to_string(local_https_public_root().join("local-device-ca.sha256")).ok()?;
    normalize_local_https_fingerprint(&value)
}

fn normalize_local_https_fingerprint(value: &str) -> Option<String> {
    let value = value.trim();
    let valid = value.len() == 95
        && value.chars().enumerate().all(|(index, ch)| {
            if (index + 1) % 3 == 0 {
                ch == ':'
            } else {
                ch.is_ascii_hexdigit()
            }
        });
    valid.then(|| value.to_ascii_uppercase())
}

fn nginx_local_https_is_enabled(content: &str) -> bool {
    content.contains("Agent Runtime UI: local-CA HTTPS entry")
        && content.lines().any(|line| {
            let line = line.trim();
            line.starts_with("listen ") && line.contains("443") && line.contains("ssl")
        })
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
            PathBuf::from("/opt/homebrew/etc/nginx/servers/agent-runtime-ui.conf"),
            PathBuf::from("/usr/local/etc/nginx/servers/agent-runtime-ui.conf"),
        ];
    }
    vec![
        PathBuf::from("/etc/nginx/sites-enabled/agent-runtime-ui.conf"),
        PathBuf::from("/etc/nginx/conf.d/agent-runtime-ui.conf"),
    ]
}

fn nginx_config_is_agent_site(content: &str) -> bool {
    let proxy_upstreams = content
        .split("proxy_pass ")
        .skip(1)
        .filter_map(|segment| segment.split(';').next().map(str::trim))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    content.to_ascii_lowercase().contains("agent runtime ui")
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

async fn run_local_https_prepare_job(
    workspace_root: PathBuf,
    shared: Arc<Mutex<WorkspaceUpdateStatus>>,
    control: Arc<Mutex<WorkspaceUpdateControl>>,
) {
    run_local_https_script_job(
        workspace_root,
        shared,
        control,
        "preparing_local_https",
        &["./scripts/configure-local-lan-https.sh", "--prepare-only"],
        "local_https_prepared",
        "local_https_prepare_failed",
        "workspace_update.local_https_prepare_failed",
    )
    .await;
}

async fn run_local_https_enable_job(
    workspace_root: PathBuf,
    shared: Arc<Mutex<WorkspaceUpdateStatus>>,
    control: Arc<Mutex<WorkspaceUpdateControl>>,
) {
    run_local_https_script_job(
        workspace_root,
        shared,
        control,
        "enabling_local_https",
        &["./scripts/configure-local-lan-https.sh"],
        "local_https_enabled",
        "local_https_enable_failed",
        "workspace_update.local_https_enable_failed",
    )
    .await;
}

async fn run_local_https_restore_job(
    workspace_root: PathBuf,
    shared: Arc<Mutex<WorkspaceUpdateStatus>>,
    control: Arc<Mutex<WorkspaceUpdateControl>>,
) {
    run_local_https_script_job(
        workspace_root,
        shared,
        control,
        "restoring_local_http",
        &["./scripts/restore-local-lan-http.sh"],
        "local_https_restored",
        "local_https_restore_failed",
        "workspace_update.local_https_restore_failed",
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn run_local_https_script_job(
    workspace_root: PathBuf,
    shared: Arc<Mutex<WorkspaceUpdateStatus>>,
    control: Arc<Mutex<WorkspaceUpdateControl>>,
    step: &str,
    args: &[&str],
    success_step: &str,
    error_code: &str,
    error_key: &str,
) {
    record_workspace_update_current_version(&workspace_root, &shared).await;
    if finish_workspace_update_if_canceled(&shared, &control) {
        return;
    }
    set_workspace_update_step(&shared, step);
    reset_workspace_update_build_logs(&shared);
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
            finish_workspace_update_succeeded(&shared, success_step, out);
        }
        Ok(out) => fail_workspace_update(&shared, error_code, error_key, out),
        Err(err) => {
            if err == WORKSPACE_UPDATE_CANCELED_ERROR
                || finish_workspace_update_if_canceled(&shared, &control)
            {
                return;
            }
            fail_workspace_update_with_error(&shared, format!("{}: {err}", args[0]), error_key);
        }
    }
}
