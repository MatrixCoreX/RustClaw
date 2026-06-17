fn shell_escape_arg(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

fn parse_service_action(raw: &str) -> Option<ServiceAction> {
    match raw {
        "start" => Some(ServiceAction::Start),
        "stop" => Some(ServiceAction::Stop),
        "restart" => Some(ServiceAction::Restart),
        _ => None,
    }
}

fn service_start_script(service: &str) -> Option<&'static str> {
    match service {
        "channel-gateway" | "channel_gateway" => Some("component_start/start-channel-gateway.sh"),
        "telegramd" => Some("component_start/start-telegramd.sh"),
        "whatsappd" => Some("component_start/start-whatsappd.sh"),
        "whatsapp_webd" => Some("component_start/start-whatsapp-webd.sh"),
        "wechatd" => Some("component_start/start-wechatd.sh"),
        "feishud" => Some("component_start/start-feishud.sh"),
        "larkd" => Some("component_start/start-larkd.sh"),
        _ => None,
    }
}

fn service_process_name(service: &str) -> Option<&'static str> {
    match service {
        "channel-gateway" | "channel_gateway" => Some("channel-gateway"),
        "telegramd" => Some("telegramd"),
        "whatsappd" => Some("whatsappd"),
        "whatsapp_webd" => Some("whatsapp_webd"),
        "wechatd" => Some("wechatd"),
        "feishud" => Some("feishud"),
        "larkd" => Some("larkd"),
        _ => None,
    }
}

fn service_pid_file(service: &str) -> Option<&'static str> {
    match service {
        "channel-gateway" | "channel_gateway" => Some("channel-gateway.pid"),
        "telegramd" => Some("telegramd.pid"),
        "whatsappd" => Some("whatsappd.pid"),
        "whatsapp_webd" => Some("whatsapp_webd.pid"),
        "wechatd" => Some("wechatd.pid"),
        "feishud" => Some("feishud.pid"),
        "larkd" => Some("larkd.pid"),
        _ => None,
    }
}

fn service_direct_process_count(service: &str) -> Option<usize> {
    match service {
        "channel-gateway" | "channel_gateway" => {
            channel_gateway_process_stats().map(|(count, _)| count)
        }
        "telegramd" => telegramd_process_stats().map(|(count, _)| count),
        "whatsappd" => whatsappd_process_stats().map(|(count, _)| count),
        "whatsapp_webd" => wa_webd_process_stats().map(|(count, _)| count),
        "wechatd" => wechatd_process_stats().map(|(count, _)| count),
        "feishud" => feishud_process_stats().map(|(count, _)| count),
        "larkd" => larkd_process_stats().map(|(count, _)| count),
        _ => None,
    }
}

fn service_is_gateway_managed(service: &str) -> bool {
    matches!(
        service,
        "telegramd" | "whatsappd" | "whatsapp_webd" | "feishud" | "larkd"
    ) && matches!(service_direct_process_count(service), Some(0) | None)
        && matches!(channel_gateway_process_stats(), Some((count, _)) if count > 0)
}

fn service_extra_process_names_on_stop(service: &str) -> &'static [&'static str] {
    match service {
        "whatsapp_webd" => &["services/wa-web-bridge/index.js", "wa-web-bridge/index.js"],
        _ => &[],
    }
}

fn service_is_running(service: &str) -> bool {
    match service {
        "channel-gateway" | "channel_gateway" => channel_gateway_process_stats()
            .map(|(count, _)| count > 0)
            .unwrap_or(false),
        "telegramd" => {
            let channel_gateway_running = channel_gateway_process_stats()
                .map(|(count, _)| count > 0)
                .unwrap_or(false);
            let legacy_telegramd_running = telegramd_process_stats()
                .map(|(count, _)| count > 0)
                .unwrap_or(false);
            channel_gateway_running || legacy_telegramd_running
        }
        "whatsappd" => whatsappd_process_stats()
            .map(|(count, _)| count > 0)
            .unwrap_or(false),
        "whatsapp_webd" => wa_webd_process_stats()
            .map(|(count, _)| count > 0)
            .unwrap_or(false),
        "wechatd" => wechatd_process_stats()
            .map(|(count, _)| count > 0)
            .unwrap_or(false),
        "feishud" => feishud_process_stats()
            .map(|(count, _)| count > 0)
            .unwrap_or(false),
        "larkd" => larkd_process_stats()
            .map(|(count, _)| count > 0)
            .unwrap_or(false),
        _ => false,
    }
}

fn runtime_profile_default() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

fn spawn_background_shell(cmd: &str) -> std::io::Result<()> {
    Command::new("bash")
        .arg("-lc")
        .arg(cmd)
        .stdin(StdProcessStdio::null())
        .stdout(StdProcessStdio::null())
        .stderr(StdProcessStdio::null())
        .spawn()?;
    Ok(())
}

fn validate_service_start_readiness(state: &AppState, service: &str) -> Result<(), String> {
    match service {
        "feishud" => {
            let config = load_feishu_config_response(state, None)
                .map_err(|err| format!("read feishu config failed: {err}"))?;
            if !config.enabled {
                return Err("service disabled".to_string());
            }
            if config.app_id.trim().is_empty() || config.app_secret.trim().is_empty() {
                return Err("feishu app_id/app_secret are required".to_string());
            }
            if config.mode.eq_ignore_ascii_case("webhook")
                && !config.verification_token_configured
                && !config.encrypt_key_configured
            {
                return Err(
                    "feishu webhook mode requires verification_token or encrypt_key".to_string(),
                );
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

async fn control_service(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((service, action)): AxumPath<(String, String)>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let action = match parse_service_action(action.trim()) {
        Some(v) => v,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("action must be start, stop, or restart".to_string()),
                }),
            );
        }
    };

    if service_start_script(service.as_str()).is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("unsupported service".to_string()),
            }),
        );
    }

    match action {
        ServiceAction::Start => {
            if let Err(err) = validate_service_start_readiness(&state, service.as_str()) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(err),
                    }),
                );
            }
            if service_is_running(service.as_str()) {
                return (
                    StatusCode::OK,
                    Json(ApiResponse {
                        ok: true,
                        data: Some(json!({
                            "service": service,
                            "action": "start",
                            "status": "already_running"
                        })),
                        error: None,
                    }),
                );
            }
            let profile = std::env::var("RUSTCLAW_START_PROFILE")
                .ok()
                .filter(|v| matches!(v.as_str(), "debug" | "release"))
                .unwrap_or_else(|| runtime_profile_default().to_string());
            let Some(script_name) = service_start_script(service.as_str()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("unsupported service".to_string()),
                    }),
                );
            };
            let workspace = state.skill_rt.workspace_root.to_string_lossy();
            let log_file = format!("logs/{}.log", service);
            let cmd = format!(
                "cd {} && mkdir -p logs .pids && nohup ./{} {} > {} 2>&1 &",
                shell_escape_arg(workspace.as_ref()),
                script_name,
                shell_escape_arg(profile.as_str()),
                shell_escape_arg(log_file.as_str())
            );
            if let Err(err) = spawn_background_shell(&cmd) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("failed to start service process: {err}")),
                    }),
                );
            }
            // The start command may return success even if script preflight exits quickly
            // (for example, service disabled or missing required config). Verify process is up.
            tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
            if !service_is_running(service.as_str()) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!(
                            "service did not enter running state: {service}. check logs/{service}.log and channel config"
                        )),
                    }),
                );
            }
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({
                        "service": service,
                        "action": "start",
                        "status": "starting",
                        "profile": profile
                    })),
                    error: None,
                }),
            )
        }
        ServiceAction::Stop => {
            if service_is_gateway_managed(service.as_str()) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!(
                            "{service} is currently managed by channel-gateway and cannot be stopped from the per-service button"
                        )),
                    }),
                );
            }
            let Some(process_name) = service_process_name(service.as_str()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("unsupported service".to_string()),
                    }),
                );
            };
            let mut killed = 0usize;
            if let Some(pids) = daemon_process_pids_by_name(process_name) {
                for pid in pids {
                    let cmd = format!("kill -TERM {} >/dev/null 2>&1 || true", pid);
                    let _ = Command::new("bash").arg("-lc").arg(cmd).output().await;
                    killed += 1;
                }
            }
            for extra_name in service_extra_process_names_on_stop(service.as_str()) {
                if let Some(pids) = daemon_process_pids_by_name(extra_name) {
                    for pid in pids {
                        let cmd = format!("kill -TERM {} >/dev/null 2>&1 || true", pid);
                        let _ = Command::new("bash").arg("-lc").arg(cmd).output().await;
                        killed += 1;
                    }
                }
            }
            if killed == 0 && !service_is_running(service.as_str()) {
                return (
                    StatusCode::OK,
                    Json(ApiResponse {
                        ok: true,
                        data: Some(json!({
                            "service": service,
                            "action": "stop",
                            "status": "already_stopped"
                        })),
                        error: None,
                    }),
                );
            }
            let Some(pid_file) = service_pid_file(service.as_str()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("unsupported service".to_string()),
                    }),
                );
            };
            let workspace = state.skill_rt.workspace_root.to_string_lossy();
            let cmd = format!(
                "cd {} && rm -f .pids/{}",
                shell_escape_arg(workspace.as_ref()),
                shell_escape_arg(pid_file)
            );
            let output = match Command::new("bash").arg("-lc").arg(cmd).output().await {
                Ok(v) => v,
                Err(err) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiResponse {
                            ok: false,
                            data: None,
                            error: Some(format!("failed to stop service process: {err}")),
                        }),
                    );
                }
            };
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let detail = if !stderr.is_empty() { stderr } else { stdout };
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("service stop command failed: {detail}")),
                    }),
                );
            }
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({
                        "service": service,
                        "action": "stop",
                        "status": "stopped"
                    })),
                    error: None,
                }),
            )
        }
        ServiceAction::Restart => {
            if service_is_gateway_managed(service.as_str()) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!(
                            "{service} is currently managed by channel-gateway and cannot be restarted from the per-service button"
                        )),
                    }),
                );
            }
            if let Err(err) = validate_service_start_readiness(&state, service.as_str()) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(err),
                    }),
                );
            }
            let Some(process_name) = service_process_name(service.as_str()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("unsupported service".to_string()),
                    }),
                );
            };
            if let Some(pids) = daemon_process_pids_by_name(process_name) {
                for pid in pids {
                    let cmd = format!("kill -TERM {} >/dev/null 2>&1 || true", pid);
                    let _ = Command::new("bash").arg("-lc").arg(cmd).output().await;
                }
            }
            for extra_name in service_extra_process_names_on_stop(service.as_str()) {
                if let Some(pids) = daemon_process_pids_by_name(extra_name) {
                    for pid in pids {
                        let cmd = format!("kill -TERM {} >/dev/null 2>&1 || true", pid);
                        let _ = Command::new("bash").arg("-lc").arg(cmd).output().await;
                    }
                }
            }
            if let Some(pid_file) = service_pid_file(service.as_str()) {
                let workspace = state.skill_rt.workspace_root.to_string_lossy();
                let cmd = format!(
                    "cd {} && rm -f .pids/{}",
                    shell_escape_arg(workspace.as_ref()),
                    shell_escape_arg(pid_file)
                );
                let _ = Command::new("bash").arg("-lc").arg(cmd).output().await;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let profile = std::env::var("RUSTCLAW_START_PROFILE")
                .ok()
                .filter(|v| matches!(v.as_str(), "debug" | "release"))
                .unwrap_or_else(|| runtime_profile_default().to_string());
            let Some(script_name) = service_start_script(service.as_str()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("unsupported service".to_string()),
                    }),
                );
            };
            let workspace = state.skill_rt.workspace_root.to_string_lossy();
            let log_file = format!("logs/{}.log", service);
            let cmd = format!(
                "cd {} && mkdir -p logs .pids && nohup ./{} {} > {} 2>&1 &",
                shell_escape_arg(workspace.as_ref()),
                script_name,
                shell_escape_arg(profile.as_str()),
                shell_escape_arg(log_file.as_str())
            );
            if let Err(err) = spawn_background_shell(&cmd) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("failed to start service process: {err}")),
                    }),
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
            if !service_is_running(service.as_str()) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!(
                            "service did not enter running state after restart: {service}. check logs/{service}.log"
                        )),
                    }),
                );
            }
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({
                        "service": service,
                        "action": "restart",
                        "status": "restarted",
                        "profile": profile
                    })),
                    error: None,
                }),
            )
        }
    }
}

async fn restart_system(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can restart RustClaw".to_string()),
            }),
        );
    }

    if std::path::Path::new("/.dockerenv").exists() {
        let mut cmd = Command::new("bash");
        cmd.arg("-lc")
            .arg("sleep 1 && kill -TERM 1 >/dev/null 2>&1")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        if let Err(err) = cmd.spawn() {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("failed to schedule restart: {err}")),
                }),
            );
        }

        return (
            StatusCode::ACCEPTED,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({
                    "status": "restarting",
                    "mode": "docker",
                })),
                error: None,
            }),
        );
    }

    match schedule_binary_restart_with_start_all(&state) {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({
                    "status": "restarting",
                    "mode": "binary",
                    "script": "start-all-bin.sh",
                    "log": "logs/restart-system.log",
                })),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(err),
            }),
        ),
    }
}

async fn pi_app_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let model = raspberry_pi_model();
    let script_path = state.skill_rt.workspace_root.join("pi_app/run-small-screen.sh");
    let script_exists = script_path.exists();
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "available": model.is_some() && script_exists,
                "is_raspberry_pi": model.is_some(),
                "model": model,
                "script_exists": script_exists,
            })),
            error: None,
        }),
    )
}

async fn restart_pi_app(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can restart Pi App".to_string()),
            }),
        );
    }

    let Some(model) = raspberry_pi_model() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: Some(json!({
                    "status": "unsupported_platform",
                    "is_raspberry_pi": false,
                })),
                error: Some("pi_app_restart_unavailable".to_string()),
            }),
        );
    };

    match schedule_pi_app_restart(&state) {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({
                    "status": "restarting",
                    "model": model,
                    "log": "logs/pi-app-restart.log",
                })),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(err),
            }),
        ),
    }
}

fn raspberry_pi_model() -> Option<String> {
    for path in [
        "/proc/device-tree/model",
        "/sys/firmware/devicetree/base/model",
    ] {
        if let Ok(raw) = fs::read_to_string(path) {
            let model = raw.trim_matches(char::from(0)).trim().to_string();
            if model.to_ascii_lowercase().contains("raspberry pi") {
                return Some(model);
            }
        }
    }
    if let Ok(raw) = fs::read_to_string("/proc/cpuinfo") {
        let lower = raw.to_ascii_lowercase();
        if lower.contains("raspberry pi") {
            let model = raw
                .lines()
                .find_map(|line| line.split_once(':').and_then(|(key, value)| {
                    key.trim()
                        .eq_ignore_ascii_case("model")
                        .then(|| value.trim().to_string())
                }))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "Raspberry Pi".to_string());
            return Some(model);
        }
    }
    None
}

fn schedule_pi_app_restart(state: &AppState) -> Result<(), String> {
    let pi_app_dir = state.skill_rt.workspace_root.join("pi_app");
    let script_path = pi_app_dir.join("run-small-screen.sh");
    if !script_path.exists() {
        return Err("pi_app/run-small-screen.sh not found".to_string());
    }
    let workspace = state.skill_rt.workspace_root.to_string_lossy();
    let pi_app = pi_app_dir.to_string_lossy();
    let script = format!(
        "cd {} && mkdir -p logs && (pkill -TERM -f '[r]ustclaw_small_screen.py|[r]ustclaw-small-screen' >/dev/null 2>&1 || true); sleep 1; cd {} && DISPLAY=${{DISPLAY:-:0}} nohup ./run-small-screen.sh > ../logs/pi-app-restart.log 2>&1 &",
        shell_escape_arg(workspace.as_ref()),
        shell_escape_arg(pi_app.as_ref())
    );
    let mut cmd = StdCommand::new("nohup");
    cmd.arg("bash")
        .arg("-c")
        .arg(&script)
        .current_dir(&state.skill_rt.workspace_root)
        .stdin(StdProcessStdio::null())
        .stdout(StdProcessStdio::null())
        .stderr(StdProcessStdio::null());
    if let Err(err) = cmd.spawn() {
        return Err(format!("failed to schedule Pi App restart: {err}"));
    }
    Ok(())
}

fn schedule_binary_restart_with_start_all(state: &AppState) -> Result<(), String> {
    let script_path = state.skill_rt.workspace_root.join("start-all-bin.sh");
    if !script_path.exists() {
        return Err("start-all-bin.sh not found in workspace root".to_string());
    }

    let workspace = state.skill_rt.workspace_root.to_string_lossy();
    let script = format!(
        "sleep 2; cd {} && mkdir -p logs .pids && RUSTCLAW_SKIP_BANNER=1 bash ./start-all-bin.sh release > logs/restart-system.log 2>&1",
        shell_escape_arg(workspace.as_ref())
    );
    let mut cmd = StdCommand::new("nohup");
    cmd.arg("bash")
        .arg("-c")
        .arg(&script)
        .current_dir(&state.skill_rt.workspace_root)
        .stdin(StdProcessStdio::null())
        .stdout(StdProcessStdio::null())
        .stderr(StdProcessStdio::null());

    if let Err(err) = cmd.spawn() {
        return Err(format!("failed to schedule restart: {err}"));
    }
    Ok(())
}
