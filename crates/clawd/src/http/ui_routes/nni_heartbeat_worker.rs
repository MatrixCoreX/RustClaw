fn nni_heartbeat_operation_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn nni_heartbeat_worker_notify() -> &'static Notify {
    static NOTIFY: OnceLock<Notify> = OnceLock::new();
    NOTIFY.get_or_init(Notify::new)
}

fn nni_heartbeat_immediate_request_flag() -> &'static AtomicBool {
    static REQUESTED: AtomicBool = AtomicBool::new(false);
    &REQUESTED
}

fn nni_join_transition_starts_heartbeat(requested_joined: Option<bool>, was_joined: bool) -> bool {
    requested_joined == Some(true) && !was_joined
}

fn nni_heartbeat_is_due(force_immediate: bool, next_due_at_ts: Option<u64>, now: u64) -> bool {
    force_immediate || !next_due_at_ts.is_some_and(|next_due| now < next_due)
}

fn nni_heartbeat_worker_sleep_seconds(next_due_at_ts: Option<u64>, now: u64) -> u64 {
    next_due_at_ts
        .map(|next_due| {
            next_due
                .saturating_sub(now)
                .clamp(1, NNI_HEARTBEAT_POLL_SECONDS)
        })
        .unwrap_or(NNI_HEARTBEAT_POLL_SECONDS)
}

pub(crate) fn spawn_nni_heartbeat_worker(state: AppState) {
    tokio::spawn(async move {
        loop {
            let force_immediate = nni_heartbeat_immediate_request_flag().swap(false, Ordering::AcqRel);
            if let Err(err) = nni_heartbeat_tick(&state, force_immediate).await {
                append_nni_log_event_best_effort(
                    &state,
                    "heartbeat_tick_error",
                    json!({"error": err.to_string()}),
                );
            }
            let now = u64::try_from(current_unix_ts()).unwrap_or_default();
            let next_due_at_ts = read_nni_config(&state)
                .ok()
                .and_then(|config| config.next_heartbeat_due_at_ts);
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(nni_heartbeat_worker_sleep_seconds(
                    next_due_at_ts,
                    now,
                ))) => {}
                _ = nni_heartbeat_worker_notify().notified() => {}
            }
        }
    });
}

async fn nni_heartbeat_tick(state: &AppState, force_immediate: bool) -> anyhow::Result<()> {
    let _guard = nni_heartbeat_operation_lock().lock().await;
    let config = read_nni_config(state)?;
    if !config.joined || nni_selected_remote_node(&config).is_none() {
        return Ok(());
    }
    let now = u64::try_from(current_unix_ts()).unwrap_or_default();
    if !nni_heartbeat_is_due(force_immediate, config.next_heartbeat_due_at_ts, now) {
        return Ok(());
    }

    let selected_nodes = nni_selected_remote_nodes(&config)
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    match nni_recorded_heartbeat(state, &selected_nodes).await {
        Ok(_) => Ok(()),
        Err(error) => {
            if nni_heartbeat_error_is_authorization_rejection(&error.code) {
                write_nni_config(state, None, Some(false))?;
            }
            Err(error.into())
        }
    }
}

fn nni_heartbeat_error_is_authorization_rejection(error_code: &str) -> bool {
    matches!(
        error_code,
        "nni_device_not_authorized"
            | "device_not_authorized"
            | "nni_device_not_registered"
            | "device_not_registered"
            | "nni_public_key_not_allowed"
            | "nni_pubkey_not_allowlisted"
            | "forbidden"
    )
}

fn nni_legacy_remote_error_token(value: &str) -> Option<&str> {
    let value = value.trim();
    let allowed_prefix = value == "forbidden"
        || value.starts_with("nni_")
        || value.starts_with("heartbeat_")
        || value.starts_with("device_");
    let allowed_shape = !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
    (allowed_prefix && allowed_shape).then_some(value)
}

fn nni_remote_api_error_code(body: &ApiResponse<Value>, fallback: &str) -> String {
    body.data
        .as_ref()
        .and_then(|data| data.get("error_code"))
        .and_then(Value::as_str)
        .filter(|code| !code.trim().is_empty())
        .or_else(|| {
            body.error
                .as_deref()
                .and_then(nni_legacy_remote_error_token)
        })
        .unwrap_or(fallback)
        .to_string()
}

async fn nni_recorded_heartbeat(
    state: &AppState,
    remote_nodes: &[String],
) -> Result<Value, NniHeartbeatError> {
    let config = read_nni_config(state).map_err(|error| {
        NniHeartbeatError::non_network("nni_config_read_failed", error.to_string())
    })?;
    let now = u64::try_from(current_unix_ts()).unwrap_or_default();
    match run_nni_heartbeat_with_network_retries(state, remote_nodes).await {
        Ok(data) => {
            let heartbeat_ts = data
                .get("request_time_ts")
                .and_then(|value| value.as_u64())
                .unwrap_or(now);
            let heartbeat_count = data
                .get("heartbeat_count")
                .and_then(|value| value.as_u64())
                .unwrap_or_else(|| config.heartbeat_request_count.saturating_add(1));
            write_nni_heartbeat_status(
                state,
                NniHeartbeatStatusUpdate {
                    heartbeat_at_ts: Some(heartbeat_ts),
                    attempt_at_ts: Some(now),
                    error: None,
                    error_code: None,
                    error_at_ts: None,
                    error_network: false,
                    request_count: Some(heartbeat_count),
                    network_failures: Some(0),
                    success_node_url: data.get("node_url").and_then(Value::as_str),
                    network_authorization: Some("authorized"),
                },
            )
            .map_err(|error| {
                NniHeartbeatError::non_network(
                    "nni_heartbeat_state_write_failed",
                    error.to_string(),
                )
            })?;
            let remote_status = data
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("accepted");
            let mut record = nni_request_record(
                "nni_heartbeat",
                if remote_status == "heartbeat_accepted" {
                    "accepted"
                } else {
                    remote_status
                },
            );
            record.task_id = data
                .get("task_id")
                .and_then(Value::as_str)
                .map(str::to_string);
            record.user_key = Some(NNI_HEARTBEAT_USER_KEY.to_string());
            record.device_pubkey = data
                .get("device_pubkey")
                .and_then(Value::as_str)
                .or_else(|| data.get("local_device_pubkey").and_then(Value::as_str))
                .map(str::to_string);
            record.node_url = data
                .get("node_url")
                .and_then(Value::as_str)
                .map(str::to_string);
            record.compliant = data
                .get("compliant")
                .and_then(Value::as_bool)
                .or(Some(true));
            record.created_at_ts = data
                .get("verified_at_ts")
                .and_then(Value::as_u64)
                .or(Some(heartbeat_ts));
            record.signature_present = true;
            record.challenge_present = true;
            record_nni_request_event(state, record);
            append_nni_log_event_best_effort(
                state,
                "heartbeat_accepted",
                json!({
                    "heartbeat_ts": heartbeat_ts,
                    "heartbeat_count": heartbeat_count,
                    "node_url": data.get("node_url").and_then(Value::as_str).unwrap_or(""),
                }),
            );
            Ok(data)
        }
        Err(err) => {
            let error_message = err.to_string();
            let network_failures = Some(if err.network {
                u64::try_from(NNI_HEARTBEAT_NETWORK_RETRY_LIMIT).unwrap_or(3)
            } else {
                0
            });
            write_nni_heartbeat_status(
                state,
                NniHeartbeatStatusUpdate {
                    heartbeat_at_ts: None,
                    attempt_at_ts: Some(now),
                    error: Some(&error_message),
                    error_code: Some(&err.code),
                    error_at_ts: Some(now),
                    error_network: err.network,
                    request_count: None,
                    network_failures,
                    success_node_url: None,
                    network_authorization: nni_heartbeat_error_is_authorization_rejection(
                        &err.code,
                    )
                    .then_some("rejected"),
                },
            )
            .map_err(|error| {
                NniHeartbeatError::non_network(
                    "nni_heartbeat_state_write_failed",
                    error.to_string(),
                )
            })?;
            let mut record = nni_request_record("nni_heartbeat", "failed");
            record.user_key = Some(NNI_HEARTBEAT_USER_KEY.to_string());
            record.compliant = Some(false);
            record.error_code = Some(err.code.clone());
            record.created_at_ts = Some(now);
            record_nni_request_event(state, record);
            append_nni_log_event_best_effort(
                state,
                "heartbeat_failed",
                json!({
                    "error": err.to_string(),
                    "error_code": err.code,
                    "network": err.network,
                }),
            );
            Err(err)
        }
    }
}

async fn run_nni_heartbeat_with_network_retries(
    state: &AppState,
    node_urls: &[String],
) -> Result<Value, NniHeartbeatError> {
    let mut last_error: Option<NniHeartbeatError> = None;
    for attempt in 1..=NNI_HEARTBEAT_NETWORK_RETRY_LIMIT {
        match run_nni_heartbeat_once(state, node_urls).await {
            Ok(data) => return Ok(data),
            Err(err) if err.network && attempt < NNI_HEARTBEAT_NETWORK_RETRY_LIMIT => {
                append_nni_log_event_best_effort(
                    state,
                    "heartbeat_network_retry",
                    json!({
                        "attempt": attempt,
                        "retry_limit": NNI_HEARTBEAT_NETWORK_RETRY_LIMIT,
                        "error": err.to_string(),
                    }),
                );
                last_error = Some(err);
                tokio::time::sleep(Duration::from_secs(
                    NNI_HEARTBEAT_NETWORK_RETRY_DELAY_SECONDS,
                ))
                .await;
            }
            Err(err) => return Err(err),
        }
    }
    Err(last_error
        .unwrap_or_else(|| {
            NniHeartbeatError::network(
                "nni_heartbeat_network_retries_exhausted",
                "nni_heartbeat_network_retries_exhausted",
            )
        }))
}

async fn run_nni_heartbeat_once(
    state: &AppState,
    node_urls: &[String],
) -> Result<Value, NniHeartbeatError> {
    let device_pubkey = nni_device_pubkey(state)
        .await
        .map_err(|(_, error, data)| NniHeartbeatError::non_network(error, data.to_string()))?;
    let mut attempts = Vec::new();
    let mut last_non_network_error = None;
    for node_url in node_urls {
        match run_nni_heartbeat_once_for_node(state, node_url, &device_pubkey).await {
            Ok(mut data) => {
                if let Some(obj) = data.as_object_mut() {
                    obj.insert("node_url".to_string(), Value::String(node_url.clone()));
                    obj.insert(
                        "local_device_pubkey".to_string(),
                        Value::String(device_pubkey.clone()),
                    );
                }
                return Ok(data);
            }
            Err(err) => {
                if !err.network {
                    last_non_network_error = Some(err.clone());
                }
                attempts.push(json!({
                    "node_url": node_url,
                    "network": err.network,
                    "error_code": err.code,
                    "error": err.to_string(),
                }));
            }
        }
    }
    if let Some(error) = last_non_network_error {
        return Err(error);
    }
    Err(NniHeartbeatError::network(
        "nni_heartbeat_all_nodes_failed",
        Value::Array(attempts).to_string(),
    ))
}

async fn run_nni_heartbeat_once_for_node(
    state: &AppState,
    node_url: &str,
    device_pubkey: &str,
) -> Result<Value, NniHeartbeatError> {
    let request_endpoint = nni_remote_api_endpoint(node_url, "heartbeat/request");
    let request_resp = state
        .core
        .public_http_client
        .post(&request_endpoint)
        .timeout(nni_remote_api_timeout())
        .json(&NniRemoteHeartbeatRequest {
            device_pubkey: device_pubkey.to_string(),
            client_user_key: NNI_HEARTBEAT_USER_KEY.to_string(),
        })
        .send()
        .await
        .map_err(|err| {
            NniHeartbeatError::network("heartbeat_request_network_failed", err.to_string())
        })?;
    let request_status = request_resp.status();
    let request_body = request_resp
        .json::<ApiResponse<Value>>()
        .await
        .map_err(|err| {
            NniHeartbeatError::non_network("heartbeat_request_body_failed", err.to_string())
        })?;
    if !request_status.is_success() || !request_body.ok {
        let error_code =
            nni_remote_api_error_code(&request_body, "heartbeat_request_failed");
        return Err(NniHeartbeatError::non_network(
            error_code,
            format!("status={} data={:?}", request_status, request_body.data),
        ));
    }
    let request_data = request_body
        .data
        .ok_or_else(|| {
            NniHeartbeatError::non_network(
                "heartbeat_request_missing_data",
                "heartbeat_request_missing_data",
            )
        })?;
    let task_id = request_data
        .get("task_id")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            NniHeartbeatError::non_network(
                "heartbeat_task_id_missing",
                "heartbeat_task_id_missing",
            )
        })?;
    let challenge = request_data
        .get("challenge")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            NniHeartbeatError::non_network(
                "heartbeat_challenge_missing",
                "heartbeat_challenge_missing",
            )
        })?;

    let sign_output = run_nni_signature_helper(state, &[String::from("sign_challenge"), challenge])
        .await
        .map_err(|err| {
            NniHeartbeatError::non_network("heartbeat_signature_helper_failed", err)
        })?;
    if !sign_output.ok {
        return Err(NniHeartbeatError::non_network(
            "heartbeat_signature_failed",
            sign_output
                .error
                .or_else(
                    || (!sign_output.stderr_tail.is_empty()).then_some(sign_output.stderr_tail),
                )
                .unwrap_or_else(|| "heartbeat_signature_failed".to_string()),
        ));
    }
    let signature = sign_output
        .payload
        .get("signature")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            NniHeartbeatError::non_network(
                "heartbeat_signature_missing",
                "heartbeat_signature_missing",
            )
        })?;

    let verify_endpoint = nni_remote_api_endpoint(node_url, "heartbeat/verify");
    let verify_resp = state
        .core
        .public_http_client
        .post(&verify_endpoint)
        .timeout(nni_remote_api_timeout())
        .json(&NniRemoteHeartbeatVerifyRequest { task_id, signature })
        .send()
        .await
        .map_err(|err| {
            NniHeartbeatError::network("heartbeat_verify_network_failed", err.to_string())
        })?;
    let verify_status = verify_resp.status();
    let verify_body = verify_resp
        .json::<ApiResponse<Value>>()
        .await
        .map_err(|err| {
            NniHeartbeatError::non_network("heartbeat_verify_body_failed", err.to_string())
        })?;
    if !verify_status.is_success() || !verify_body.ok {
        let error_code = nni_remote_api_error_code(&verify_body, "heartbeat_verify_failed");
        return Err(NniHeartbeatError::non_network(
            error_code,
            format!("status={} data={:?}", verify_status, verify_body.data),
        ));
    }
    verify_body
        .data
        .ok_or_else(|| {
            NniHeartbeatError::non_network(
                "heartbeat_verify_missing_data",
                "heartbeat_verify_missing_data",
            )
        })
}
