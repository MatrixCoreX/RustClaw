fn nni_supported_actions() -> Vec<&'static str> {
    vec![
        "pubkey",
        "sign_timestamp",
        "tng_device_pubkey",
        "tng_device_cert",
        "tng_signer_cert",
        "tng_root_cert",
        "sign_challenge",
    ]
}

const NNI_SIMULATION_ENABLE_ACTION: &str = "simulation_enable";
const NNI_SIMULATION_DISABLE_ACTION: &str = "simulation_disable";
const NNI_SIGNATURE_HELPER_TIMEOUT_ENV: &str = "APP_NNI_SIGNATURE_HELPER_TIMEOUT_SECONDS";
const NNI_SIGNATURE_HELPER_TIMEOUT_DEFAULT_SECONDS: u64 = 25;
const NNI_SIGNATURE_HELPER_TIMEOUT_MIN_SECONDS: u64 = 5;
const NNI_SIGNATURE_HELPER_TIMEOUT_MAX_SECONDS: u64 = 120;

fn nni_accepted_actions() -> Vec<&'static str> {
    let mut actions = nni_supported_actions();
    actions.extend([NNI_SIMULATION_ENABLE_ACTION, NNI_SIMULATION_DISABLE_ACTION]);
    actions
}

fn nni_signature_helper_path(state: &AppState) -> PathBuf {
    state
        .skill_rt
        .workspace_root
        .join("pi_app")
        .join("signature.py")
}

fn nni_signature_helper_python() -> String {
    claw_core::product_identity::env_string("CRYPTOAUTHLIB_PYTHON")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "python3".to_string())
}

fn nni_signature_simulator_state_path(state: &AppState) -> PathBuf {
    state
        .skill_rt
        .workspace_root
        .join("data")
        .join("nni")
        .join("signature-simulator.json")
}

fn nni_helper_payload_simulated(payload: &Value) -> bool {
    payload
        .get("simulated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn nni_action_message_key(action: &str) -> &'static str {
    match action {
        NNI_SIMULATION_ENABLE_ACTION => "nni.device_action.simulation_enabled",
        NNI_SIMULATION_DISABLE_ACTION => "nni.device_action.simulation_disabled",
        _ => "nni.device_action.completed",
    }
}

fn nni_hex_fingerprint(hex: &str) -> Option<String> {
    let normalized = hex.trim();
    if normalized.is_empty() {
        return None;
    }
    let keep = normalized.len().min(16);
    Some(normalized[..keep].to_string())
}

fn nni_short_hex(hex: &str) -> Option<String> {
    let normalized = hex.trim();
    if normalized.is_empty() {
        return None;
    }
    if normalized.len() <= 24 {
        return Some(normalized.to_string());
    }
    Some(format!(
        "{}...{}",
        &normalized[..12],
        &normalized[normalized.len().saturating_sub(12)..]
    ))
}

fn nni_signature_helper_log_context(args: &[String]) -> Value {
    json!({
        "action": args.first().map(String::as_str).unwrap_or(""),
        "arg_count": args.len(),
    })
}

fn nni_signature_helper_operation_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

const NNI_HARDWARE_PUBKEY_CACHE_SECONDS: u64 = 10 * 60;

#[derive(Clone)]
struct NniHardwarePubkeyCacheEntry {
    output: NniSignatureHelperOutput,
    expires_at: tokio::time::Instant,
}

fn nni_hardware_pubkey_cache(
) -> &'static Mutex<HashMap<PathBuf, NniHardwarePubkeyCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, NniHardwarePubkeyCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_nni_hardware_pubkey(script_path: &Path) -> Option<NniSignatureHelperOutput> {
    let now = tokio::time::Instant::now();
    let mut cache = nni_hardware_pubkey_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.retain(|_, entry| entry.expires_at > now);
    cache.get(script_path).map(|entry| entry.output.clone())
}

fn cache_nni_hardware_pubkey(script_path: PathBuf, output: &NniSignatureHelperOutput) {
    let pubkey_is_valid = output
        .payload
        .get("pubkey")
        .and_then(Value::as_str)
        .is_some_and(is_nni_pubkey_hex);
    if !output.ok || nni_helper_payload_simulated(&output.payload) || !pubkey_is_valid {
        return;
    }
    nni_hardware_pubkey_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(
            script_path,
            NniHardwarePubkeyCacheEntry {
                output: output.clone(),
                expires_at: tokio::time::Instant::now()
                    + Duration::from_secs(NNI_HARDWARE_PUBKEY_CACHE_SECONDS),
            },
        );
}

fn invalidate_nni_hardware_pubkey(script_path: &Path) {
    nni_hardware_pubkey_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(script_path);
}

async fn run_nni_signature_helper(
    state: &AppState,
    args: &[String],
) -> Result<NniSignatureHelperOutput, String> {
    run_nni_signature_helper_with_cache(state, args, true).await
}

async fn run_nni_signature_helper_uncached(
    state: &AppState,
    args: &[String],
) -> Result<NniSignatureHelperOutput, String> {
    run_nni_signature_helper_with_cache(state, args, false).await
}

async fn run_nni_signature_helper_with_cache(
    state: &AppState,
    args: &[String],
    allow_pubkey_cache: bool,
) -> Result<NniSignatureHelperOutput, String> {
    let script_path = nni_signature_helper_path(state);
    let log_context = nni_signature_helper_log_context(args);
    let action = args.first().map(String::as_str).unwrap_or_default();
    let changes_signer = matches!(
        action,
        NNI_SIMULATION_ENABLE_ACTION | NNI_SIMULATION_DISABLE_ACTION
    );
    if !script_path.is_file() {
        append_nni_log_event_best_effort(
            state,
            "signature_helper_missing",
            json!({
                "helper_path": script_path.display().to_string(),
                "context": log_context.clone(),
            }),
        );
        return Err(format!(
            "signature helper not found: {}",
            script_path.display()
        ));
    }
    if allow_pubkey_cache && action == "pubkey" {
        if let Some(output) = logged_cached_nni_hardware_pubkey(
            state,
            &script_path,
            &log_context,
        ) {
            return Ok(output);
        }
    }

    // The secure element and simulator state are single-writer resources. UI,
    // trading, reward, and heartbeat calls may arrive concurrently.
    let _operation_guard = nni_signature_helper_operation_lock().lock().await;
    if changes_signer {
        invalidate_nni_hardware_pubkey(&script_path);
    } else if allow_pubkey_cache && action == "pubkey" {
        if let Some(output) = logged_cached_nni_hardware_pubkey(
            state,
            &script_path,
            &log_context,
        ) {
            return Ok(output);
        }
    }

    let mut cmd = Command::new(nni_signature_helper_python());
    cmd.arg(&script_path)
        .args(args)
        .current_dir(&state.skill_rt.workspace_root)
        .env("PYTHONUNBUFFERED", "1")
        .env(
            "APP_SIGNATURE_SIMULATOR_STATE",
            nni_signature_simulator_state_path(state),
        )
        .stdin(StdProcessStdio::null())
        .stdout(StdProcessStdio::piped())
        .stderr(StdProcessStdio::piped())
        .kill_on_drop(true);

    let timeout_seconds = nni_signature_helper_timeout_seconds();
    let output = match tokio::time::timeout(Duration::from_secs(timeout_seconds), cmd.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => {
            append_nni_log_event_best_effort(
                state,
                "signature_helper_run_error",
                json!({
                    "error": err.to_string(),
                    "context": log_context.clone(),
                }),
            );
            return Err(format!("failed to run signature helper: {err}"));
        }
        Err(_) => {
            append_nni_log_event_best_effort(
                state,
                "signature_helper_timeout",
                json!({
                    "timeout_seconds": timeout_seconds,
                    "context": log_context.clone(),
                }),
            );
            return Err(format!("signature helper timed out after {timeout_seconds}s"));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stdout.is_empty() {
        append_nni_log_event_best_effort(
            state,
            "signature_helper_empty_output",
            json!({
                "exit_code": output.status.code(),
                "stderr_present": !stderr.is_empty(),
                "context": log_context.clone(),
            }),
        );
        return Err(if stderr.is_empty() {
            "signature helper returned empty output".to_string()
        } else {
            stderr
        });
    }

    let payload: Value = serde_json::from_str(&stdout).map_err(|err| {
        append_nni_log_event_best_effort(
            state,
            "signature_helper_non_json_output",
            json!({
                "error": err.to_string(),
                "stdout_bytes": stdout.len(),
                "context": log_context.clone(),
            }),
        );
        format!("signature helper returned non-json output: {err}: {stdout}")
    })?;
    let ok = payload
        .get("ok")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let error = payload
        .get("error")
        .and_then(|value| value.as_str())
        .map(str::to_string);

    append_nni_log_event_best_effort(
        state,
        "signature_helper_result",
        json!({
            "ok": ok,
            "exit_code": output.status.code(),
            "stderr_present": !stderr.is_empty(),
            "context": log_context.clone(),
            "meta": nni_helper_payload_meta(&payload),
        }),
    );

    let result = NniSignatureHelperOutput {
        ok,
        payload,
        error,
        stderr_tail: stderr,
        exit_code: output.status.code(),
    };
    if action == "pubkey" {
        cache_nni_hardware_pubkey(script_path.clone(), &result);
    } else if changes_signer {
        invalidate_nni_hardware_pubkey(&script_path);
    }
    Ok(result)
}

fn logged_cached_nni_hardware_pubkey(
    state: &AppState,
    script_path: &Path,
    log_context: &Value,
) -> Option<NniSignatureHelperOutput> {
    let output = cached_nni_hardware_pubkey(script_path)?;
    append_nni_log_event_best_effort(
        state,
        "signature_helper_cache_hit",
        json!({
            "context": log_context,
            "cache_seconds": NNI_HARDWARE_PUBKEY_CACHE_SECONDS,
            "meta": nni_helper_payload_meta(&output.payload),
        }),
    );
    Some(output)
}

fn normalize_nni_signature_helper_timeout_seconds(raw: Option<&str>) -> u64 {
    raw.and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(NNI_SIGNATURE_HELPER_TIMEOUT_DEFAULT_SECONDS)
        .clamp(
            NNI_SIGNATURE_HELPER_TIMEOUT_MIN_SECONDS,
            NNI_SIGNATURE_HELPER_TIMEOUT_MAX_SECONDS,
        )
}

fn nni_signature_helper_timeout_seconds() -> u64 {
    let configured = std::env::var(NNI_SIGNATURE_HELPER_TIMEOUT_ENV).ok();
    normalize_nni_signature_helper_timeout_seconds(configured.as_deref())
}

async fn detect_nni_signature_chip(
    state: &AppState,
    force_refresh: bool,
) -> Result<NniSignatureHelperOutput, String> {
    let args = [String::from("pubkey")];
    let result = if force_refresh {
        run_nni_signature_helper_uncached(state, &args).await
    } else {
        run_nni_signature_helper(state, &args).await
    };
    if matches!(&result, Ok(output) if output.ok) {
        return result;
    }

    let script_path = nni_signature_helper_path(state);
    if let Some(output) = cached_nni_hardware_pubkey(&script_path) {
        append_nni_log_event_best_effort(
            state,
            "signature_detection_cache_fallback",
            json!({
                "force_refresh": force_refresh,
                "meta": nni_helper_payload_meta(&output.payload),
            }),
        );
        return Ok(output);
    }
    result
}

fn nni_helper_payload_meta(payload: &Value) -> Value {
    json!({
        "slot": payload.get("slot").cloned().unwrap_or(Value::Null),
        "i2c_bus": payload.get("i2c_bus").cloned().unwrap_or(Value::Null),
        "i2c_baud": payload.get("i2c_baud").cloned().unwrap_or(Value::Null),
        "i2c_address": payload.get("i2c_address").cloned().unwrap_or(Value::Null),
        "lib_path": payload.get("lib_path").cloned().unwrap_or(Value::Null),
        "simulated": payload.get("simulated").cloned().unwrap_or(Value::Bool(false)),
        "device_kind": payload.get("device_kind").cloned().unwrap_or(Value::Null),
    })
}

async fn nni_device_snapshot(state: &AppState, force_refresh: bool) -> Value {
    let script_path = nni_signature_helper_path(state);
    let supported_actions = nni_supported_actions();
    if !script_path.is_file() {
        append_nni_log_event_best_effort(
            state,
            "device_status",
            json!({
                "status": "helper_missing",
                "helper_available": false,
                "hardware_chip_present": false,
                "signer_available": false,
            }),
        );
        return json!({
            "nni_available": true,
            "helper_available": false,
            "signature_chip_present": false,
            "hardware_chip_present": false,
            "signer_available": false,
            "local_participation_eligible": false,
            "signer_kind": "unavailable",
            "network_authorization": "unknown",
            "status": "helper_missing",
            "simulated": false,
            "device_kind": "unavailable",
            "simulation_available": false,
            "message_key": "nni.device_status.helper_missing",
            "next_step_key": "nni.device_status.helper_missing.next_step",
            "helper_path": script_path.to_string_lossy(),
            "supported_actions": supported_actions,
        });
    }

    match detect_nni_signature_chip(state, force_refresh).await {
        Ok(output) if output.ok => {
            let simulated = nni_helper_payload_simulated(&output.payload);
            let hardware_chip_present = !simulated;
            let signer_kind = if simulated { "simulated" } else { "hardware" };
            let pubkey = output
                .payload
                .get("pubkey")
                .and_then(Value::as_str)
                .unwrap_or_default();
            append_nni_log_event_best_effort(
                state,
                "device_status",
                json!({
                    "status": if simulated { "simulated" } else { "ready" },
                    "helper_available": true,
                    "hardware_chip_present": hardware_chip_present,
                    "signer_available": true,
                    "signer_kind": signer_kind,
                    "exit_code": output.exit_code,
                }),
            );
            json!({
                "nni_available": true,
                "helper_available": true,
                // Compatibility projection for the existing UI. New runtime
                // contracts use hardware_chip_present and signer_available.
                "signature_chip_present": true,
                "hardware_chip_present": hardware_chip_present,
                "signer_available": true,
                "local_participation_eligible": true,
                "signer_kind": signer_kind,
                "network_authorization": "unknown",
                "status": if simulated { "simulated" } else { "ready" },
                "message_key": if simulated { "nni.device_status.simulated" } else { "nni.device_status.ready" },
                "next_step_key": if simulated { Some("nni.device_status.simulated.next_step") } else { None },
                "simulated": simulated,
                "device_kind": signer_kind,
                "simulation_available": false,
                "helper_path": script_path.to_string_lossy(),
                "supported_actions": supported_actions,
                "pubkey": pubkey,
                "pubkey_preview": nni_short_hex(pubkey),
                "pubkey_fingerprint": nni_hex_fingerprint(pubkey),
                "meta": nni_helper_payload_meta(&output.payload),
                "exit_code": output.exit_code,
            })
        }
        Ok(output) => {
            let reason = output
                .error
                .clone()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| {
                    (!output.stderr_tail.trim().is_empty()).then(|| output.stderr_tail.clone())
                })
                .unwrap_or_else(|| "signature device unavailable".to_string());
            append_nni_log_event_best_effort(
                state,
                "device_status",
                json!({
                    "status": "signature_chip_missing",
                    "helper_available": true,
                    "hardware_chip_present": false,
                    "signer_available": false,
                    "exit_code": output.exit_code,
                    "diagnostic": reason,
                }),
            );
            json!({
                "nni_available": true,
                "helper_available": true,
                "signature_chip_present": false,
                "hardware_chip_present": false,
                "signer_available": false,
                "local_participation_eligible": false,
                "signer_kind": "unavailable",
                "network_authorization": "unknown",
                "status": "signature_chip_missing",
                "simulated": false,
                "device_kind": "unavailable",
                "simulation_available": true,
                "message_key": "nni.device_status.signature_chip_missing",
                "next_step_key": "nni.device_status.signature_chip_missing.next_step",
                "helper_path": script_path.to_string_lossy(),
                "supported_actions": supported_actions,
                "exit_code": output.exit_code,
            })
        }
        Err(err) => {
            append_nni_log_event_best_effort(
                state,
                "device_status",
                json!({
                    "status": "detection_unavailable",
                    "helper_available": true,
                    "hardware_chip_present": false,
                    "signer_available": false,
                    "diagnostic": err,
                }),
            );
            json!({
                "nni_available": true,
                "helper_available": true,
                "signature_chip_present": false,
                "hardware_chip_present": false,
                "signer_available": false,
                "local_participation_eligible": false,
                "signer_kind": "unavailable",
                "network_authorization": "unknown",
                "status": "detection_unavailable",
                "simulated": false,
                "device_kind": "unavailable",
                "simulation_available": false,
                "message_key": "nni.device_status.detection_unavailable",
                "next_step_key": "nni.device_status.detection_unavailable.next_step",
                "helper_path": script_path.to_string_lossy(),
                "supported_actions": supported_actions,
            })
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct NniDeviceStatusQuery {
    #[serde(default)]
    refresh: bool,
}

async fn nni_device_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<NniDeviceStatusQuery>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err((status, Json(resp))) = require_ui_admin(&state, &headers) {
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
            data: Some(nni_device_snapshot(&state, query.refresh).await),
            error: None,
        }),
    )
}

async fn nni_device_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<NniDeviceActionRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err((status, Json(resp))) = require_ui_admin(&state, &headers) {
        return (
            status,
            Json(ApiResponse {
                ok: resp.ok,
                data: None,
                error: resp.error,
            }),
        );
    }

    let action = req.action.trim().to_ascii_lowercase();
    if !nni_accepted_actions().contains(&action.as_str()) {
        append_nni_log_event_best_effort(
            &state,
            "device_action",
            json!({
                "action": action,
                "status": "unsupported_action",
            }),
        );
        return api_error_value(
            StatusCode::BAD_REQUEST,
            format!("unsupported NNI action: {action}"),
        );
    }

    if action == NNI_SIMULATION_ENABLE_ACTION {
        if let Ok(output) = run_nni_signature_helper(&state, &[String::from("pubkey")]).await {
            if output.ok && !nni_helper_payload_simulated(&output.payload) {
                append_nni_log_event_best_effort(
                    &state,
                    "device_action",
                    json!({
                        "action": action,
                        "status": "hardware_signature_chip_present",
                    }),
                );
                return (
                    StatusCode::CONFLICT,
                    Json(ApiResponse {
                        ok: false,
                        data: Some(json!({
                            "action": action,
                            "signature_chip_present": true,
                            "simulated": false,
                            "status": "hardware_signature_chip_present",
                            "message_key": "nni.device_action.simulation_not_needed",
                        })),
                        error: Some("nni_hardware_signature_chip_present".to_string()),
                    }),
                );
            }
        }
    }

    let mut args = vec![action.clone()];
    if action == "sign_timestamp" {
        args.push(req.timestamp.unwrap_or_else(current_unix_ts).to_string());
    } else if action == "sign_challenge" {
        let challenge = req
            .challenge
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let Some(challenge) = challenge else {
            append_nni_log_event_best_effort(
                &state,
                "device_action",
                json!({
                    "action": action,
                    "status": "challenge_missing",
                }),
            );
            return api_error_value(StatusCode::BAD_REQUEST, "nni_challenge_required");
        };
        args.push(challenge.to_string());
    }

    match run_nni_signature_helper(&state, &args).await {
        Ok(output) if output.ok => {
            let simulated = nni_helper_payload_simulated(&output.payload);
            let signature_chip_present = output
                .payload
                .get("signature_chip_present")
                .and_then(Value::as_bool)
                .unwrap_or(action != NNI_SIMULATION_DISABLE_ACTION);
            append_nni_log_event_best_effort(
                &state,
                "device_action",
                json!({
                    "action": action,
                    "status": "ok",
                    "signature_chip_present": signature_chip_present,
                    "simulated": simulated,
                    "exit_code": output.exit_code,
                    "meta": nni_helper_payload_meta(&output.payload),
                }),
            );
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({
                        "action": action,
                        "signature_chip_present": signature_chip_present,
                        "simulated": simulated,
                        "device_kind": output.payload.get("device_kind").cloned().unwrap_or(Value::Null),
                        "message_key": nni_action_message_key(&action),
                        "payload": output.payload,
                        "meta": nni_helper_payload_meta(&output.payload),
                        "exit_code": output.exit_code,
                    })),
                    error: None,
                }),
            )
        }
        Ok(output) => {
            let reason = output
                .error
                .filter(|value| !value.trim().is_empty())
                .or_else(|| (!output.stderr_tail.trim().is_empty()).then_some(output.stderr_tail))
                .unwrap_or_else(|| "signature chip unavailable".to_string());
            let simulation_action = matches!(
                action.as_str(),
                NNI_SIMULATION_ENABLE_ACTION | NNI_SIMULATION_DISABLE_ACTION
            );
            let error_code = output
                .payload
                .get("error_code")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(if simulation_action {
                    "nni_signature_simulation_failed"
                } else {
                    "nni_signature_chip_missing"
                });
            append_nni_log_event_best_effort(
                &state,
                "device_action",
                json!({
                    "action": action,
                    "status": if simulation_action { "simulation_failed" } else { "signature_chip_missing" },
                    "signature_chip_present": false,
                    "exit_code": output.exit_code,
                    "diagnostic": reason,
                }),
            );
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: Some(json!({
                        "action": action,
                        "signature_chip_present": false,
                        "status": if simulation_action { "simulation_failed" } else { "signature_chip_missing" },
                        "message_key": if simulation_action { "nni.device_action.simulation_failed" } else { "nni.device_action.signature_chip_missing" },
                        "exit_code": output.exit_code,
                    })),
                    error: Some(error_code.to_string()),
                }),
            )
        }
        Err(err) => {
            let simulation_action = matches!(
                action.as_str(),
                NNI_SIMULATION_ENABLE_ACTION | NNI_SIMULATION_DISABLE_ACTION
            );
            append_nni_log_event_best_effort(
                &state,
                "device_action",
                json!({
                    "action": action,
                    "status": if simulation_action { "simulation_failed" } else { "signature_chip_missing" },
                    "signature_chip_present": false,
                    "diagnostic": err,
                }),
            );
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: Some(json!({
                        "action": action,
                        "signature_chip_present": false,
                        "status": if simulation_action { "simulation_failed" } else { "signature_chip_missing" },
                        "message_key": if simulation_action { "nni.device_action.simulation_failed" } else { "nni.device_action.signature_chip_missing" },
                    })),
                    error: Some(if simulation_action {
                        "nni_signature_simulation_failed".to_string()
                    } else {
                        "nni_signature_chip_missing".to_string()
                    }),
                }),
            )
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct InternalSkillTokenContext {
    task_id: String,
    user_id: i64,
    chat_id: i64,
    user_key: Option<String>,
    channel: String,
    external_user_id: Option<String>,
    external_chat_id: Option<String>,
    kind: String,
    payload_json: String,
    skill_name: String,
}

fn redeem_internal_skill_token(
    headers: &HeaderMap,
) -> Result<InternalSkillTokenContext, (StatusCode, Json<ApiResponse<Value>>)> {
    let token = headers
        .get(claw_core::product_identity::INTERNAL_SKILL_TOKEN_HEADER)
        .or_else(|| headers.get(claw_core::product_identity::INTERNAL_LLM_TOKEN_HEADER))
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| api_error_value(StatusCode::UNAUTHORIZED, "missing internal skill token"))?;
    let payload = claw_core::secrets::redeem_secret_token_reference(token)
        .map_err(|error| {
            api_error_value(
                StatusCode::UNAUTHORIZED,
                format!("internal skill token rejected: {error}"),
            )
        })?
        .ok_or_else(|| api_error_value(StatusCode::UNAUTHORIZED, "invalid internal skill token"))?;
    serde_json::from_str(&payload).map_err(|error| {
        api_error_value(
            StatusCode::UNAUTHORIZED,
            format!("internal skill token payload invalid: {error}"),
        )
    })
}

fn api_error_value(
    status: StatusCode,
    error: impl Into<String>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    (
        status,
        Json(ApiResponse {
            ok: false,
            data: None,
            error: Some(error.into()),
        }),
    )
}

async fn internal_llm_text(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<InternalLlmTextRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let token_ctx = match redeem_internal_skill_token(&headers) {
        Ok(value) => value,
        Err(response) => return response,
    };

    let requested_skill = req.skill_name.trim();
    if !requested_skill.is_empty() && requested_skill != token_ctx.skill_name {
        return api_error_value(
            StatusCode::FORBIDDEN,
            format!(
                "internal llm token is scoped to skill `{}`, not `{requested_skill}`",
                token_ctx.skill_name
            ),
        );
    }

    let prompt_source = req.prompt_source.trim();
    if prompt_source.is_empty() {
        return api_error_value(StatusCode::BAD_REQUEST, "prompt_source is required");
    }
    let prompt = if !req.prompt.trim().is_empty() {
        req.prompt.trim().to_string()
    } else if !req.system.trim().is_empty() || !req.user.trim().is_empty() {
        format!(
            "System:\n{}\n\nUser:\n{}",
            req.system.trim(),
            req.user.trim()
        )
    } else {
        return api_error_value(StatusCode::BAD_REQUEST, "prompt or system/user is required");
    };
    let task = ClaimedTask {
        claim_attempt: 0,
        task_id: token_ctx.task_id,
        user_id: token_ctx.user_id,
        chat_id: token_ctx.chat_id,
        user_key: token_ctx.user_key,
        channel: token_ctx.channel,
        external_user_id: token_ctx.external_user_id,
        external_chat_id: token_ctx.external_chat_id,
        kind: token_ctx.kind,
        payload_json: token_ctx.payload_json,
    };
    let providers = match internal_llm_text_providers(&state, &task, &req) {
        Ok(providers) => providers,
        Err(err) => return api_error_value(StatusCode::BAD_REQUEST, err),
    };
    let selected_model = providers
        .first()
        .map(|provider| provider.config.model.clone())
        .unwrap_or_default();
    let selected_provider = providers
        .first()
        .map(|provider| provider.config.name.clone())
        .unwrap_or_default();
    let hints = crate::ChatRequestHints {
        temperature: req.temperature,
        max_tokens: req.max_tokens,
        ..Default::default()
    };

    match crate::llm_gateway::run_with_fallback_on_providers_with_hints(
        &state,
        &task,
        &prompt,
        prompt_source,
        hints,
        providers,
    )
    .await
    {
        Ok(text) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!(InternalLlmTextResponse {
                    text,
                    prompt_source: prompt_source.to_string(),
                    model: selected_model,
                    provider: selected_provider,
                })),
                error: None,
            }),
        ),
        Err(err) => api_error_value(
            StatusCode::BAD_GATEWAY,
            format!("internal llm call failed: {err}"),
        ),
    }
}

fn internal_llm_text_providers(
    state: &AppState,
    task: &ClaimedTask,
    req: &InternalLlmTextRequest,
) -> Result<Vec<Arc<LlmProviderRuntime>>, String> {
    let vendor = req
        .vendor
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let model = req
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if vendor.is_none() && model.is_none() {
        return Ok(state.task_llm_providers(task));
    }

    let config_path = state.reload_ctx.config_path_for_reload.trim();
    if config_path.is_empty() {
        return Err("internal llm model override requires a loaded config path".to_string());
    }
    let config = claw_core::config::AppConfig::load(config_path)
        .map_err(|err| format!("load config for internal llm override failed: {err}"))?;
    let providers = crate::llm_gateway::build_providers_for_selection(&config, vendor, model);
    if providers.is_empty() {
        let vendor_label = vendor.unwrap_or("<default>");
        let model_label = model.unwrap_or("<default>");
        return Err(format!(
            "no llm provider matched internal override vendor={vendor_label} model={model_label}"
        ));
    }
    Ok(providers)
}

#[derive(Debug, Deserialize)]
struct CreateAuthKeyRequest {
    #[serde(default)]
    role: String,
}

#[derive(Debug, Deserialize)]
struct UpdateAuthKeyRequest {
    role: Option<String>,
    enabled: Option<bool>,
}
