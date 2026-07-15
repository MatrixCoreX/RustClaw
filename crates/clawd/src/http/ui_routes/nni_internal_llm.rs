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

fn nni_signature_helper_path(state: &AppState) -> PathBuf {
    state
        .skill_rt
        .workspace_root
        .join("pi_app")
        .join("signature.py")
}

fn nni_signature_helper_python() -> String {
    std::env::var("RUSTCLAW_CRYPTOAUTHLIB_PYTHON")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "python3".to_string())
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

async fn run_nni_signature_helper(
    state: &AppState,
    args: &[String],
) -> Result<NniSignatureHelperOutput, String> {
    let script_path = nni_signature_helper_path(state);
    let log_context = nni_signature_helper_log_context(args);
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

    let mut cmd = Command::new(nni_signature_helper_python());
    cmd.arg(&script_path)
        .args(args)
        .current_dir(&state.skill_rt.workspace_root)
        .env("PYTHONUNBUFFERED", "1")
        .stdin(StdProcessStdio::null())
        .stdout(StdProcessStdio::piped())
        .stderr(StdProcessStdio::piped())
        .kill_on_drop(true);

    let output = match tokio::time::timeout(
        Duration::from_secs(NNI_SIGNATURE_HELPER_TIMEOUT_SECONDS),
        cmd.output(),
    )
    .await
    {
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
                    "timeout_seconds": NNI_SIGNATURE_HELPER_TIMEOUT_SECONDS,
                    "context": log_context.clone(),
                }),
            );
            return Err(format!(
                "signature helper timed out after {NNI_SIGNATURE_HELPER_TIMEOUT_SECONDS}s"
            ));
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

    Ok(NniSignatureHelperOutput {
        ok,
        payload,
        error,
        stderr_tail: stderr,
        exit_code: output.status.code(),
    })
}

fn nni_helper_payload_meta(payload: &Value) -> Value {
    json!({
        "slot": payload.get("slot").cloned().unwrap_or(Value::Null),
        "i2c_bus": payload.get("i2c_bus").cloned().unwrap_or(Value::Null),
        "i2c_baud": payload.get("i2c_baud").cloned().unwrap_or(Value::Null),
        "i2c_address": payload.get("i2c_address").cloned().unwrap_or(Value::Null),
        "lib_path": payload.get("lib_path").cloned().unwrap_or(Value::Null),
    })
}

async fn nni_device_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err((status, Json(resp))) = require_ui_identity(&state, &headers) {
        return (
            status,
            Json(ApiResponse {
                ok: resp.ok,
                data: None,
                error: resp.error,
            }),
        );
    }

    let script_path = nni_signature_helper_path(&state);
    let supported_actions = nni_supported_actions();
    if !script_path.is_file() {
        append_nni_log_event_best_effort(
            &state,
            "device_status",
            json!({
                "status": "helper_missing",
                "helper_available": false,
                "signature_chip_present": false,
                "helper_path": script_path.to_string_lossy(),
            }),
        );
        return (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({
                    "nni_available": true,
                    "helper_available": false,
                    "signature_chip_present": false,
                    "status": "helper_missing",
                    "message_key": "nni.device_status.helper_missing",
                    "next_step_key": "nni.device_status.helper_missing.next_step",
                    "helper_path": script_path.to_string_lossy(),
                    "supported_actions": supported_actions,
                })),
                error: None,
            }),
        );
    }

    match run_nni_signature_helper(&state, &[String::from("pubkey")]).await {
        Ok(output) if output.ok => {
            let pubkey = output
                .payload
                .get("pubkey")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            append_nni_log_event_best_effort(
                &state,
                "device_status",
                json!({
                    "status": "ready",
                    "helper_available": true,
                    "signature_chip_present": true,
                    "exit_code": output.exit_code,
                }),
            );
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({
                        "nni_available": true,
                        "helper_available": true,
                        "signature_chip_present": true,
                        "status": "ready",
                        "message_key": "nni.device_status.ready",
                        "helper_path": script_path.to_string_lossy(),
                        "supported_actions": supported_actions,
                        "pubkey": pubkey,
                        "pubkey_preview": nni_short_hex(pubkey),
                        "pubkey_fingerprint": nni_hex_fingerprint(pubkey),
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
                .clone()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| {
                    (!output.stderr_tail.trim().is_empty()).then(|| output.stderr_tail.clone())
                })
                .unwrap_or_else(|| "signature chip unavailable".to_string());
            append_nni_log_event_best_effort(
                &state,
                "device_status",
                json!({
                    "status": "signature_chip_missing",
                    "helper_available": true,
                    "signature_chip_present": false,
                    "exit_code": output.exit_code,
                    "error_present": !reason.trim().is_empty(),
                }),
            );
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({
                        "nni_available": true,
                        "helper_available": true,
                        "signature_chip_present": false,
                        "status": "signature_chip_missing",
                        "message_key": "nni.device_status.signature_chip_missing",
                        "next_step_key": "nni.device_status.signature_chip_missing.next_step",
                        "helper_path": script_path.to_string_lossy(),
                        "supported_actions": supported_actions,
                        "error": reason,
                        "exit_code": output.exit_code,
                    })),
                    error: None,
                }),
            )
        }
        Err(err) => {
            append_nni_log_event_best_effort(
                &state,
                "device_status",
                json!({
                    "status": "signature_chip_missing",
                    "helper_available": true,
                    "signature_chip_present": false,
                    "error": err,
                }),
            );
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({
                        "nni_available": true,
                        "helper_available": true,
                        "signature_chip_present": false,
                        "status": "signature_chip_missing",
                        "message_key": "nni.device_status.signature_chip_missing",
                        "next_step_key": "nni.device_status.signature_chip_missing.next_step",
                        "helper_path": script_path.to_string_lossy(),
                        "supported_actions": supported_actions,
                        "error": err,
                    })),
                    error: None,
                }),
            )
        }
    }
}

async fn nni_device_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<NniDeviceActionRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err((status, Json(resp))) = require_ui_identity(&state, &headers) {
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
    if !nni_supported_actions().contains(&action.as_str()) {
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
            append_nni_log_event_best_effort(
                &state,
                "device_action",
                json!({
                    "action": action,
                    "status": "ok",
                    "signature_chip_present": true,
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
                        "signature_chip_present": true,
                        "message_key": "nni.device_action.completed",
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
            append_nni_log_event_best_effort(
                &state,
                "device_action",
                json!({
                    "action": action,
                    "status": "signature_chip_missing",
                    "signature_chip_present": false,
                    "exit_code": output.exit_code,
                    "error_present": !reason.trim().is_empty(),
                }),
            );
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: Some(json!({
                        "action": action,
                        "signature_chip_present": false,
                        "status": "signature_chip_missing",
                        "message_key": "nni.device_action.signature_chip_missing",
                        "exit_code": output.exit_code,
                    })),
                    error: Some(reason),
                }),
            )
        }
        Err(err) => {
            append_nni_log_event_best_effort(
                &state,
                "device_action",
                json!({
                    "action": action,
                    "status": "signature_chip_missing",
                    "signature_chip_present": false,
                    "error": err,
                }),
            );
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: Some(json!({
                        "action": action,
                        "signature_chip_present": false,
                        "status": "signature_chip_missing",
                        "message_key": "nni.device_action.signature_chip_missing",
                    })),
                    error: Some(err),
                }),
            )
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct InternalLlmTokenContext {
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
    let token = headers
        .get("x-rustclaw-internal-llm-token")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let Some(token) = token else {
        return api_error_value(StatusCode::UNAUTHORIZED, "missing internal llm token");
    };

    let token_payload = match claw_core::secrets::redeem_secret_token_reference(token) {
        Ok(Some(value)) => value,
        Ok(None) => {
            return api_error_value(StatusCode::UNAUTHORIZED, "invalid internal llm token");
        }
        Err(err) => {
            return api_error_value(
                StatusCode::UNAUTHORIZED,
                format!("internal llm token rejected: {err}"),
            );
        }
    };
    let token_ctx: InternalLlmTokenContext = match serde_json::from_str(&token_payload) {
        Ok(value) => value,
        Err(err) => {
            return api_error_value(
                StatusCode::UNAUTHORIZED,
                format!("internal llm token payload invalid: {err}"),
            );
        }
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
