#[derive(Debug, Clone, Serialize)]
struct WebdExposureStatus {
    supported: bool,
    platform: String,
    enabled: bool,
    running: bool,
    listen: String,
    port: u16,
    externally_accessible: bool,
    nginx_compatible: bool,
    restart_scheduled: bool,
}

#[derive(Debug, Deserialize)]
struct WebdExposureUpdateRequest {
    externally_accessible: bool,
}

async fn get_webd_exposure_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<WebdExposureStatus>>) {
    if let Err(response) = require_webd_exposure_admin(&state, &headers) {
        return response;
    }

    match collect_webd_exposure_status(&state.skill_rt.workspace_root, false) {
        Ok(status) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(status),
                error: None,
            }),
        ),
        Err(error) => webd_exposure_error(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

async fn update_webd_exposure(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<WebdExposureUpdateRequest>,
) -> (StatusCode, Json<ApiResponse<WebdExposureStatus>>) {
    if let Err(response) = require_webd_exposure_admin(&state, &headers) {
        return response;
    }
    if !matches!(std::env::consts::OS, "linux" | "macos") {
        return webd_exposure_error(StatusCode::BAD_REQUEST, "webd_exposure_unsupported");
    }

    let workspace_root = &state.skill_rt.workspace_root;
    let config_path = webd_channel_config_path(workspace_root);
    let original = match std::fs::read_to_string(&config_path) {
        Ok(value) => value,
        Err(_) => {
            return webd_exposure_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "webd_config_unavailable",
            );
        }
    };
    let (updated, changed) = match rewrite_webd_exposure(
        &original,
        request.externally_accessible,
    ) {
        Ok(value) => value,
        Err(error) => return webd_exposure_error(StatusCode::BAD_REQUEST, error),
    };

    if changed {
        if atomic_write_webd_config(&config_path, updated.as_bytes()).is_err() {
            return webd_exposure_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "webd_config_write_failed",
            );
        }
        if let Err(error) = schedule_webd_listener_restart(workspace_root) {
            if let Err(rollback_error) = atomic_write_webd_config(&config_path, original.as_bytes()) {
                tracing::error!(
                    error = %error,
                    rollback_error = %rollback_error,
                    "failed to schedule webd restart and restore listener config"
                );
            }
            return webd_exposure_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "webd_restart_schedule_failed",
            );
        }
    }

    match collect_webd_exposure_status(workspace_root, changed) {
        Ok(status) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(status),
                error: None,
            }),
        ),
        Err(error) => webd_exposure_error(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

fn require_webd_exposure_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, Json<ApiResponse<WebdExposureStatus>>)> {
    let identity = require_ui_identity(state, headers).map_err(|(status, Json(response))| {
        (
            status,
            Json(ApiResponse {
                ok: response.ok,
                data: None,
                error: response.error,
            }),
        )
    })?;
    if identity.role.eq_ignore_ascii_case("admin") {
        Ok(())
    } else {
        Err(webd_exposure_error(
            StatusCode::FORBIDDEN,
            "webd_exposure_admin_required",
        ))
    }
}

fn webd_exposure_error(
    status: StatusCode,
    error: impl Into<String>,
) -> (StatusCode, Json<ApiResponse<WebdExposureStatus>>) {
    (
        status,
        Json(ApiResponse {
            ok: false,
            data: None,
            error: Some(error.into()),
        }),
    )
}

fn collect_webd_exposure_status(
    workspace_root: &Path,
    restart_scheduled: bool,
) -> Result<WebdExposureStatus, &'static str> {
    let raw = std::fs::read_to_string(webd_channel_config_path(workspace_root))
        .map_err(|_| "webd_config_unavailable")?;
    let document = raw
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| "webd_config_invalid")?;
    let webd = document
        .get("webd")
        .and_then(toml_edit::Item::as_table)
        .ok_or("webd_config_invalid")?;
    let enabled = webd
        .get("enabled")
        .and_then(toml_edit::Item::as_bool)
        .unwrap_or(false);
    let listen = webd
        .get("listen")
        .and_then(toml_edit::Item::as_str)
        .unwrap_or("0.0.0.0:8788")
        .to_string();
    let address = parse_webd_listen_address(&listen)?;

    Ok(WebdExposureStatus {
        supported: matches!(std::env::consts::OS, "linux" | "macos"),
        platform: std::env::consts::OS.to_string(),
        enabled,
        running: daemon_process_pids_by_name("webd").is_some_and(|pids| !pids.is_empty()),
        listen,
        port: address.port(),
        externally_accessible: !address.ip().is_loopback(),
        nginx_compatible: true,
        restart_scheduled,
    })
}

fn rewrite_webd_exposure(
    raw: &str,
    externally_accessible: bool,
) -> Result<(String, bool), &'static str> {
    let mut document = raw
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| "webd_config_invalid")?;
    let listen = document
        .get("webd")
        .and_then(toml_edit::Item::as_table)
        .and_then(|table| table.get("listen"))
        .and_then(toml_edit::Item::as_str)
        .unwrap_or("0.0.0.0:8788");
    let current = parse_webd_listen_address(listen)?;
    let host = if externally_accessible {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    };
    let next = format!("{host}:{}", current.port());
    if listen == next {
        return Ok((raw.to_string(), false));
    }
    if document.get("webd").and_then(toml_edit::Item::as_table).is_none() {
        document["webd"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    document["webd"]["listen"] = toml_edit::value(next);
    Ok((document.to_string(), true))
}

fn parse_webd_listen_address(listen: &str) -> Result<std::net::SocketAddr, &'static str> {
    listen
        .trim()
        .parse::<std::net::SocketAddr>()
        .map_err(|_| "webd_listen_invalid")
}

fn webd_channel_config_path(workspace_root: &Path) -> PathBuf {
    let directory = claw_core::product_identity::env_os("CHANNEL_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root.join("configs/channels"));
    let directory = if directory.is_absolute() {
        directory
    } else {
        workspace_root.join(directory)
    };
    directory.join("webd.toml")
}

fn atomic_write_webd_config(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "webd config has no parent")
    })?;
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(".webd.toml.{}.{}.tmp", std::process::id(), suffix));
    let result = (|| {
        std::fs::write(&temporary, bytes)?;
        if let Ok(metadata) = std::fs::metadata(path) {
            std::fs::set_permissions(&temporary, metadata.permissions())?;
        }
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn schedule_webd_listener_restart(workspace_root: &Path) -> Result<(), String> {
    let start_script = workspace_root.join("component_start/start-webd.sh");
    if !start_script.is_file() {
        return Err("component_start/start-webd.sh not found".to_string());
    }
    let running_pids = daemon_process_pids_by_name("webd").unwrap_or_default();
    let stop_commands = running_pids
        .iter()
        .map(|pid| format!("kill -TERM {pid} >/dev/null 2>&1 || true; "))
        .collect::<String>();
    let workspace = shell_escape_arg(workspace_root.to_string_lossy().as_ref());
    let profile = claw_core::product_identity::env_string("START_PROFILE")
        .ok()
        .filter(|value| value == "release")
        .unwrap_or_else(|| "release".to_string());
    let script = format!(
        "sleep 2; cd {workspace} && mkdir -p logs .pids; \
         {stop_commands}\
         sleep 1; rm -f .pids/webd.pid; \
         APP_SKIP_BANNER=1 nohup bash ./component_start/start-webd.sh {} >> logs/webd.log 2>&1 & \
         echo $! > .pids/webd.pid",
        shell_escape_arg(&profile)
    );
    StdCommand::new("nohup")
        .arg("bash")
        .arg("-c")
        .arg(script)
        .current_dir(workspace_root)
        .stdin(StdProcessStdio::null())
        .stdout(StdProcessStdio::null())
        .stderr(StdProcessStdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("failed to schedule webd restart: {error}"))
}
