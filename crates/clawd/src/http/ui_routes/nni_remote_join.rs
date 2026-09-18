const NNI_HEARTBEAT_INTERVAL_SECONDS: u64 = 9 * 60 + 50;
const NNI_HEARTBEAT_POLL_SECONDS: u64 = 60;
const NNI_HEARTBEAT_NETWORK_RETRY_LIMIT: usize = 3;
const NNI_HEARTBEAT_NETWORK_RETRY_DELAY_SECONDS: u64 = 2;
const NNI_HEARTBEAT_USER_KEY: &str = "clawd-nni-heartbeat";
const NNI_HEARTBEAT_ERROR_HISTORY_LIMIT: usize = 200;
const NNI_RUNTIME_CONFIG_SCHEMA_VERSION: u32 = 2;
const NNI_HEARTBEAT_RUNTIME_STATE_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Serialize)]
struct NniConfigResponse {
    remote_nodes: Vec<String>,
    selected_node_url: Option<String>,
    bancor_service_node_url: Option<String>,
    asset_service_node_url: Option<String>,
    joined: bool,
    asset_owner_pubkey: Option<String>,
    heartbeat_interval_seconds: u64,
    heartbeat_network_retry_limit: usize,
    heartbeat_request_count: u64,
    last_heartbeat_at_ts: Option<u64>,
    last_heartbeat_error: Option<String>,
    last_heartbeat_error_code: Option<String>,
    last_heartbeat_error_at_ts: Option<u64>,
    last_heartbeat_network_failures: u64,
    last_heartbeat_attempt_at_ts: Option<u64>,
    consecutive_heartbeat_failures: u64,
    last_success_node_host: Option<String>,
    network_authorization: String,
    heartbeat_state: String,
    next_heartbeat_due_at_ts: Option<u64>,
    worker_running: bool,
    config_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NniConfigUpdateRequest {
    #[serde(default)]
    remote_nodes: Option<Vec<String>>,
    #[serde(default)]
    selected_node_url: Option<String>,
    #[serde(default)]
    bancor_service_node_url: Option<String>,
    #[serde(default)]
    asset_service_node_url: Option<String>,
    #[serde(default)]
    joined: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NniRuntimeConfig {
    #[serde(default = "nni_runtime_config_schema_version")]
    schema_version: u32,
    #[serde(default)]
    remote_nodes: Vec<String>,
    #[serde(default)]
    selected_node_url: Option<String>,
    #[serde(default)]
    bancor_service_node_url: Option<String>,
    #[serde(default)]
    asset_service_node_url: Option<String>,
    #[serde(default)]
    joined: bool,
    #[serde(default)]
    asset_owner_pubkey: Option<String>,
}

impl Default for NniRuntimeConfig {
    fn default() -> Self {
        Self {
            schema_version: NNI_RUNTIME_CONFIG_SCHEMA_VERSION,
            remote_nodes: Vec::new(),
            selected_node_url: None,
            bancor_service_node_url: None,
            asset_service_node_url: None,
            joined: false,
            asset_owner_pubkey: None,
        }
    }
}

fn nni_runtime_config_schema_version() -> u32 {
    NNI_RUNTIME_CONFIG_SCHEMA_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NniHeartbeatRuntimeState {
    #[serde(default = "nni_heartbeat_runtime_state_schema_version")]
    schema_version: u32,
    #[serde(default)]
    heartbeat_request_count: u64,
    #[serde(default)]
    last_heartbeat_at_ts: Option<u64>,
    #[serde(default)]
    last_heartbeat_error: Option<String>,
    #[serde(default)]
    last_heartbeat_error_code: Option<String>,
    #[serde(default)]
    last_heartbeat_error_at_ts: Option<u64>,
    #[serde(default)]
    last_heartbeat_network_failures: u64,
    #[serde(default)]
    last_heartbeat_attempt_at_ts: Option<u64>,
    #[serde(default)]
    consecutive_heartbeat_failures: u64,
    #[serde(default)]
    last_success_node_host: Option<String>,
    #[serde(default = "nni_unknown_network_authorization")]
    network_authorization: String,
}

impl Default for NniHeartbeatRuntimeState {
    fn default() -> Self {
        Self {
            schema_version: NNI_HEARTBEAT_RUNTIME_STATE_SCHEMA_VERSION,
            heartbeat_request_count: 0,
            last_heartbeat_at_ts: None,
            last_heartbeat_error: None,
            last_heartbeat_error_code: None,
            last_heartbeat_error_at_ts: None,
            last_heartbeat_network_failures: 0,
            last_heartbeat_attempt_at_ts: None,
            consecutive_heartbeat_failures: 0,
            last_success_node_host: None,
            network_authorization: nni_unknown_network_authorization(),
        }
    }
}

fn nni_unknown_network_authorization() -> String {
    "unknown".to_string()
}

fn nni_heartbeat_runtime_state_schema_version() -> u32 {
    NNI_HEARTBEAT_RUNTIME_STATE_SCHEMA_VERSION
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NniLocalJoinRequest {
    node_url: String,
    #[serde(default)]
    asset_owner_pubkey: Option<String>,
    #[serde(default)]
    replace_existing_owner: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NniLocalJoinVerifyRequest {
    task_id: String,
    node_url: String,
    signature: String,
    #[serde(default)]
    owner_signature: Option<String>,
    #[serde(default)]
    previous_owner_signature: Option<String>,
    #[serde(default)]
    replace_existing_owner: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NniOwnerUnbindRequest {
    node_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NniOwnerUnbindVerifyRequest {
    task_id: String,
    node_url: String,
    device_signature: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NniOwnerRecoveryRequest {
    node_url: String,
    asset_owner_pubkey: String,
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    device_signature: Option<String>,
    #[serde(default)]
    owner_signature: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NniRequestRecordsQuery {
    page: Option<usize>,
    per_page: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NniHeartbeatErrorRecord {
    id: u64,
    created_at_ts: Option<u64>,
    error: String,
    network: bool,
}

#[derive(Debug, Serialize)]
struct NniRemoteJoinRequest {
    device_pubkey: String,
    client_user_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    asset_owner_pubkey: Option<String>,
    replace_existing_owner: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct NniRemoteJoinVerifyRequest {
    task_id: String,
    signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner_signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_owner_signature: Option<String>,
}

#[derive(Serialize)]
struct NniRemoteOwnerUnbindRequest {
    device_pubkey: String,
    client_user_key: String,
}

#[derive(Serialize)]
struct NniRemoteOwnerUnbindVerifyRequest {
    task_id: String,
    device_signature: String,
}

#[derive(Serialize)]
struct NniRemoteOwnerRecoveryRequest {
    asset_owner_pubkey: String,
    new_device_pubkey: String,
    client_user_key: String,
}

#[derive(Serialize)]
struct NniRemoteOwnerRecoveryVerifyRequest {
    task_id: String,
    device_signature: String,
    owner_signature: String,
}

#[derive(Debug, Serialize)]
struct NniRemoteHeartbeatRequest {
    device_pubkey: String,
    client_user_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct NniRemoteHeartbeatVerifyRequest {
    task_id: String,
    signature: String,
}

#[derive(Debug, Clone)]
struct NniHeartbeatError {
    code: String,
    message: String,
    network: bool,
}

impl NniHeartbeatError {
    fn network(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            network: true,
        }
    }

    fn non_network(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            network: false,
        }
    }
}

impl std::fmt::Display for NniHeartbeatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for NniHeartbeatError {}

async fn get_nni_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<NniConfigResponse>>) {
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

    match read_nni_config(&state) {
        Ok(config) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(config),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("nni_config_read_failed: {err}")),
            }),
        ),
    }
}

async fn update_nni_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<NniConfigUpdateRequest>,
) -> (StatusCode, Json<ApiResponse<NniConfigResponse>>) {
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

    let remote_nodes = match req.remote_nodes.as_deref() {
        Some(raw_nodes) => match normalize_nni_node_urls(raw_nodes) {
            Ok(urls) => Some(urls),
            Err(err) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(err.to_string()),
                    }),
                );
            }
        },
        None => None,
    };

    let heartbeat_was_joined = read_nni_runtime_config(&state)
        .map(|config| config.joined)
        .unwrap_or(false);
    let start_heartbeat_now = nni_join_transition_starts_heartbeat(req.joined, heartbeat_was_joined);
    match write_nni_config_with_selected_node(
        &state,
        remote_nodes.as_deref(),
        req.selected_node_url.as_deref(),
        req.bancor_service_node_url.as_deref(),
        req.asset_service_node_url.as_deref(),
        req.joined,
    ) {
        Ok(config) => {
            if start_heartbeat_now {
                nni_heartbeat_immediate_request_flag().store(true, Ordering::Release);
                nni_heartbeat_worker_notify().notify_one();
            }
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(config),
                    error: None,
                }),
            )
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("nni_config_write_failed: {err}")),
            }),
        ),
    }
}

async fn nni_join_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<NniLocalJoinRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_admin(&state, &headers) {
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

    let node_url = if req.node_url.trim().is_empty() {
        let mut record = nni_request_record("nni_join", "failed");
        record.user_key = Some(identity.user_key.clone());
        record.error_code = Some("nni_remote_node_required".to_string());
        record.created_at_ts = Some(u64::try_from(current_unix_ts()).unwrap_or_default());
        record_nni_request_event(&state, record);
        return nni_join_error(
            StatusCode::BAD_REQUEST,
            "nni_remote_node_required",
            json!({"status": "remote_node_required"}),
        );
    } else {
        match normalize_nni_node_url(&req.node_url) {
            Ok(url) => url,
            Err(err) => {
                let mut record = nni_request_record("nni_join", "failed");
                record.user_key = Some(identity.user_key.clone());
                record.error_code = Some(err.to_string());
                record.created_at_ts =
                    Some(u64::try_from(current_unix_ts()).unwrap_or_default());
                record_nni_request_event(&state, record);
                return nni_join_error(
                    StatusCode::BAD_REQUEST,
                    err,
                    json!({"status": "remote_node_invalid"}),
                );
            }
        }
    };

    let configured_owner = read_nni_runtime_config(&state)
        .ok()
        .and_then(|config| config.asset_owner_pubkey);
    let requested_owner = match req.asset_owner_pubkey.as_deref() {
        Some(value) if !value.trim().is_empty() => match normalize_nni_owner_public_key(value) {
            Ok(value) => Some(value),
            Err(error) => {
                return nni_join_error(
                    StatusCode::BAD_REQUEST,
                    error,
                    json!({"status": "asset_owner_invalid"}),
                );
            }
        },
        _ => None,
    };
    let owner_conflict = configured_owner.is_some()
        && requested_owner.is_some()
        && configured_owner != requested_owner;
    if owner_conflict && !req.replace_existing_owner {
        return nni_join_error(
            StatusCode::CONFLICT,
            "nni_asset_owner_conflict",
            json!({"status": "asset_owner_conflict"}),
        );
    }
    if req.replace_existing_owner && !owner_conflict {
        return nni_join_error(
            StatusCode::CONFLICT,
            "nni_asset_owner_rebind_not_required",
            json!({"status": "asset_owner_rebind_not_required"}),
        );
    }
    let asset_owner_pubkey = if req.replace_existing_owner {
        requested_owner
    } else {
        configured_owner.or(requested_owner)
    };

    let device_pubkey = match nni_device_pubkey(&state).await {
        Ok(pubkey) => pubkey,
        Err((status, error, data)) => {
            let mut record = nni_request_record("nni_join", "failed");
            record.user_key = Some(identity.user_key.clone());
            record.compliant = Some(false);
            record.error_code = Some(error.to_string());
            record.created_at_ts = Some(u64::try_from(current_unix_ts()).unwrap_or_default());
            record_nni_request_event(&state, record);
            return nni_join_error(status, error, data);
        }
    };

    let mut attempts = Vec::new();
    for node_url in std::iter::once(node_url) {
        let endpoint = nni_remote_api_endpoint(&node_url, "join/request");
        let response = state
            .core
            .public_http_client
            .post(&endpoint)
            .timeout(nni_remote_api_timeout())
            .json(&NniRemoteJoinRequest {
                device_pubkey: device_pubkey.clone(),
                client_user_key: identity.user_key.clone(),
                asset_owner_pubkey: asset_owner_pubkey.clone(),
                replace_existing_owner: req.replace_existing_owner,
            })
            .send()
            .await;

        match response {
            Ok(resp) => {
                let status = resp.status();
                match resp.json::<ApiResponse<Value>>().await {
                    Ok(mut body) if status.is_success() && body.ok => {
                        let data_ref = body.data.as_ref();
                        let mut record = nni_request_record(
                            "nni_join",
                            data_ref
                                .and_then(|data| data.get("status"))
                                .and_then(Value::as_str)
                                .unwrap_or("challenge_created"),
                        );
                        record.task_id = data_ref
                            .and_then(|data| data.get("task_id"))
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        record.user_key = Some(identity.user_key.clone());
                        record.device_pubkey = Some(device_pubkey.clone());
                        record.node_url = Some(node_url.clone());
                        record.created_at_ts =
                            Some(u64::try_from(current_unix_ts()).unwrap_or_default());
                        record.challenge_present = true;
                        record_nni_request_event(&state, record);
                        let data = body.data.get_or_insert_with(|| json!({}));
                        if let Some(obj) = data.as_object_mut() {
                            obj.insert("node_url".to_string(), Value::String(node_url));
                            obj.insert(
                                "local_device_pubkey".to_string(),
                                Value::String(device_pubkey),
                            );
                        }
                        return (StatusCode::OK, Json(body));
                    }
                    Ok(mut body) => {
                        let data_ref = body.data.as_ref();
                        let remote_error_code =
                            nni_remote_api_error_code(&body, "nni_remote_join_failed");
                        let mut record = nni_request_record(
                            "nni_join",
                            data_ref
                                .and_then(|data| data.get("status"))
                                .and_then(Value::as_str)
                                .unwrap_or("failed"),
                        );
                        record.task_id = data_ref
                            .and_then(|data| data.get("task_id"))
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        record.user_key = Some(identity.user_key.clone());
                        record.device_pubkey = Some(device_pubkey.clone());
                        record.node_url = Some(node_url.clone());
                        record.compliant = Some(false);
                        record.error_code = Some(remote_error_code.clone());
                        record.created_at_ts =
                            Some(u64::try_from(current_unix_ts()).unwrap_or_default());
                        record_nni_request_event(&state, record);
                        if remote_error_code == "nni_asset_device_already_bound" {
                            let existing_owner = body
                                .data
                                .as_ref()
                                .and_then(|data| data.get("asset_owner_pubkey"))
                                .and_then(Value::as_str)
                                .ok_or_else(|| anyhow::anyhow!("nni_remote_asset_owner_missing"))
                                .and_then(|value| {
                                    normalize_nni_owner_public_key(value).map_err(anyhow::Error::msg)
                                });
                            let existing_owner = match existing_owner {
                                Ok(value) => value,
                                Err(error) => {
                                    return nni_join_error(
                                        StatusCode::BAD_GATEWAY,
                                        "nni_remote_asset_owner_invalid",
                                        json!({
                                            "status": "remote_asset_owner_invalid",
                                            "detail": error.to_string(),
                                        }),
                                    );
                                }
                            };
                            if let Err(error) =
                                persist_nni_asset_owner_pubkey(&state, &existing_owner, true)
                            {
                                return nni_join_error(
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    "nni_asset_owner_persist_failed",
                                    json!({
                                        "status": "asset_owner_persist_failed",
                                        "detail": error.to_string(),
                                    }),
                                );
                            }
                            if let Some(data) = body.data.as_mut().and_then(Value::as_object_mut) {
                                data.insert("node_url".to_string(), Value::String(node_url));
                                data.insert("local_binding_restored".to_string(), Value::Bool(true));
                                data.insert("joined".to_string(), Value::Bool(false));
                            }
                            let axum_status = StatusCode::from_u16(status.as_u16())
                                .unwrap_or(StatusCode::CONFLICT);
                            return (axum_status, Json(body));
                        }
                        attempts.push(json!({
                            "node_url": node_url,
                            "http_status": status.as_u16(),
                            "error": body.error,
                            "data": body.data,
                        }));
                    }
                    Err(err) => {
                        let mut record = nni_request_record("nni_join", "failed");
                        record.user_key = Some(identity.user_key.clone());
                        record.device_pubkey = Some(device_pubkey.clone());
                        record.node_url = Some(node_url.clone());
                        record.compliant = Some(false);
                        record.error_code = Some("nni_remote_bad_response".to_string());
                        record.created_at_ts =
                            Some(u64::try_from(current_unix_ts()).unwrap_or_default());
                        record_nni_request_event(&state, record);
                        attempts.push(json!({
                            "node_url": node_url,
                            "http_status": status.as_u16(),
                            "error": format!("nni_remote_bad_response: {err}"),
                        }));
                    }
                }
            }
            Err(err) => {
                let mut record = nni_request_record("nni_join", "failed");
                record.user_key = Some(identity.user_key.clone());
                record.device_pubkey = Some(device_pubkey.clone());
                record.node_url = Some(node_url.clone());
                record.compliant = Some(false);
                record.error_code = Some("nni_remote_request_failed".to_string());
                record.created_at_ts = Some(u64::try_from(current_unix_ts()).unwrap_or_default());
                record_nni_request_event(&state, record);
                attempts.push(json!({
                    "node_url": node_url,
                    "error": format!("nni_remote_request_failed: {err}"),
                }));
            }
        }
    }

    nni_join_error(
        StatusCode::BAD_GATEWAY,
        "nni_remote_nodes_unavailable",
        json!({
            "status": "remote_nodes_unavailable",
            "attempts": attempts,
        }),
    )
}

async fn nni_join_verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<NniLocalJoinVerifyRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_admin(&state, &headers) {
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

    let node_url = match normalize_nni_node_url(&req.node_url) {
        Ok(url) => url,
        Err(err) => {
            let mut record = nni_request_record("nni_join", "failed");
            record.task_id = Some(req.task_id.trim().to_string()).filter(|value| !value.is_empty());
            record.user_key = Some(identity.user_key.clone());
            record.error_code = Some(err.to_string());
            record.created_at_ts = Some(u64::try_from(current_unix_ts()).unwrap_or_default());
            record.signature_present = !req.signature.trim().is_empty();
            record_nni_request_event(&state, record);
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                err,
                json!({"status": "remote_node_invalid"}),
            );
        }
    };
    let external_owner_signature = match req.owner_signature.as_deref() {
        Some(value) => match normalize_nni_owner_signature(value) {
            Ok(value) => Some(value),
            Err(error) => {
                return nni_join_error(
                    StatusCode::BAD_REQUEST,
                    error,
                    json!({"status": "owner_signature_invalid"}),
                );
            }
        },
        None => None,
    };
    let previous_owner_signature = match req.previous_owner_signature.as_deref() {
        Some(value) => match normalize_nni_owner_signature(value) {
            Ok(value) => Some(value),
            Err(error) => {
                return nni_join_error(
                    StatusCode::BAD_REQUEST,
                    error,
                    json!({"status": "previous_owner_signature_invalid"}),
                );
            }
        },
        None => None,
    };
    if previous_owner_signature.is_some() {
        return nni_join_error(
            StatusCode::BAD_REQUEST,
            "nni_previous_owner_signature_unexpected",
            json!({"status": "previous_owner_signature_unexpected"}),
        );
    }
    if req.replace_existing_owner && external_owner_signature.is_none() {
        return nni_join_error(
            StatusCode::BAD_REQUEST,
            "nni_target_owner_signature_required",
            json!({"status": "target_owner_signature_required"}),
        );
    }
    let owner_signature = external_owner_signature;
    let endpoint = nni_remote_api_endpoint(&node_url, "join/verify");
    let response = state
        .core
        .public_http_client
        .post(&endpoint)
        .timeout(nni_remote_api_timeout())
        .json(&NniRemoteJoinVerifyRequest {
            task_id: req.task_id.trim().to_string(),
            signature: req.signature.trim().to_string(),
            owner_signature,
            previous_owner_signature,
        })
        .send()
        .await;

    match response {
        Ok(resp) => {
            let status = resp.status();
            match resp.json::<ApiResponse<Value>>().await {
                Ok(mut body) => {
                    if status.is_success() && body.ok {
                        if let Some(owner_pubkey) = body
                            .data
                            .as_ref()
                            .and_then(|data| data.get("asset_owner_pubkey"))
                            .and_then(Value::as_str)
                        {
                            let normalized_owner = match normalize_nni_owner_public_key(owner_pubkey) {
                                Ok(value) => value,
                                Err(error) => {
                                    return nni_join_error(
                                        StatusCode::BAD_GATEWAY,
                                        error,
                                        json!({"status": "remote_asset_owner_invalid"}),
                                    );
                                }
                            };
                            if let Err(error) = persist_nni_asset_owner_pubkey(
                                &state,
                                &normalized_owner,
                                req.replace_existing_owner,
                            ) {
                                return nni_join_error(
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    "nni_asset_owner_persist_failed",
                                    json!({"status": "asset_owner_persist_failed", "detail": error.to_string()}),
                                );
                            }
                        }
                    }
                    let data_ref = body.data.as_ref();
                    let remote_status = data_ref
                        .and_then(|data| data.get("status"))
                        .and_then(Value::as_str)
                        .unwrap_or(if status.is_success() && body.ok {
                            "accepted"
                        } else {
                            "failed"
                        });
                    let mut record = nni_request_record(
                        "nni_join",
                        if remote_status == "joined" {
                            "accepted"
                        } else {
                            remote_status
                        },
                    );
                    record.task_id = data_ref
                        .and_then(|data| data.get("task_id"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or_else(|| {
                            Some(req.task_id.trim().to_string()).filter(|value| !value.is_empty())
                        });
                    record.user_key = Some(identity.user_key.clone());
                    record.device_pubkey = data_ref
                        .and_then(|data| data.get("device_pubkey"))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    record.node_url = Some(node_url.clone());
                    record.compliant = data_ref
                        .and_then(|data| data.get("compliant"))
                        .and_then(Value::as_bool)
                        .or_else(|| (status.is_success() && body.ok).then_some(true));
                    record.error_code = (!(status.is_success() && body.ok)).then(|| {
                        nni_remote_api_error_code(&body, "nni_remote_verify_failed")
                    });
                    record.created_at_ts = data_ref
                        .and_then(|data| data.get("verified_at_ts"))
                        .and_then(Value::as_u64)
                        .or_else(|| Some(u64::try_from(current_unix_ts()).unwrap_or_default()));
                    record.signature_present = !req.signature.trim().is_empty();
                    record.challenge_present = true;
                    record_nni_request_event(&state, record);
                    if let Some(data) = body.data.as_mut().and_then(|value| value.as_object_mut()) {
                        data.insert("node_url".to_string(), Value::String(node_url));
                    }
                    let axum_status =
                        StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
                    (axum_status, Json(body))
                }
                Err(err) => {
                    let mut record = nni_request_record("nni_join", "failed");
                    record.task_id =
                        Some(req.task_id.trim().to_string()).filter(|value| !value.is_empty());
                    record.user_key = Some(identity.user_key.clone());
                    record.node_url = Some(node_url);
                    record.compliant = Some(false);
                    record.error_code = Some("nni_remote_bad_response".to_string());
                    record.created_at_ts =
                        Some(u64::try_from(current_unix_ts()).unwrap_or_default());
                    record.signature_present = !req.signature.trim().is_empty();
                    record.challenge_present = true;
                    record_nni_request_event(&state, record);
                    nni_join_error(
                        StatusCode::BAD_GATEWAY,
                        "nni_remote_bad_response",
                        json!({"status": "remote_bad_response", "error": err.to_string()}),
                    )
                }
            }
        }
        Err(err) => {
            let mut record = nni_request_record("nni_join", "failed");
            record.task_id = Some(req.task_id.trim().to_string()).filter(|value| !value.is_empty());
            record.user_key = Some(identity.user_key);
            record.node_url = Some(node_url);
            record.compliant = Some(false);
            record.error_code = Some("nni_remote_request_failed".to_string());
            record.created_at_ts = Some(u64::try_from(current_unix_ts()).unwrap_or_default());
            record.signature_present = !req.signature.trim().is_empty();
            record.challenge_present = true;
            record_nni_request_event(&state, record);
            nni_join_error(
                StatusCode::BAD_GATEWAY,
                "nni_remote_request_failed",
                json!({"status": "remote_request_failed", "error": err.to_string()}),
            )
        }
    }
}

async fn nni_owner_recover(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<NniOwnerRecoveryRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_admin(&state, &headers) {
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
    let node_url = match normalize_nni_node_url(&req.node_url) {
        Ok(value) => value,
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                error,
                json!({"status": "remote_node_invalid"}),
            );
        }
    };
    let owner_pubkey = match normalize_nni_owner_public_key(&req.asset_owner_pubkey) {
        Ok(value) => value,
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                error,
                json!({"status": "asset_owner_invalid"}),
            );
        }
    };

    let verify_requested = req.task_id.is_some()
        || req.device_signature.is_some()
        || req.owner_signature.is_some();
    if verify_requested {
        let task_id = match req
            .task_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty() && value.len() <= 160)
        {
            Some(value) => value.to_string(),
            None => {
                return nni_join_error(
                    StatusCode::BAD_REQUEST,
                    "nni_owner_recovery_task_id_required",
                    json!({"status": "recovery_verify_invalid"}),
                )
            }
        };
        let device_signature = match req
            .device_signature
            .as_deref()
            .ok_or("nni_owner_recovery_device_signature_required")
            .and_then(normalize_nni_device_signature)
        {
            Ok(value) => value,
            Err(error) => {
                return nni_join_error(
                    StatusCode::BAD_REQUEST,
                    error,
                    json!({"status": "recovery_verify_invalid"}),
                )
            }
        };
        let owner_signature = match req
            .owner_signature
            .as_deref()
            .ok_or("nni_owner_recovery_owner_signature_required")
            .and_then(normalize_nni_owner_signature)
        {
            Ok(value) => value,
            Err(error) => {
                return nni_join_error(
                    StatusCode::BAD_REQUEST,
                    error,
                    json!({"status": "recovery_verify_invalid"}),
                )
            }
        };
        let verify_response = match state
            .core
            .public_http_client
            .post(nni_remote_api_endpoint(&node_url, "asset-owner/recovery/verify"))
            .timeout(nni_remote_api_timeout())
            .json(&NniRemoteOwnerRecoveryVerifyRequest {
                task_id,
                device_signature,
                owner_signature,
            })
            .send()
            .await
        {
            Ok(value) => value,
            Err(error) => {
                return nni_join_error(
                    StatusCode::BAD_GATEWAY,
                    "nni_owner_recovery_verify_failed",
                    json!({"status": "remote_request_failed", "detail": error.to_string()}),
                )
            }
        };
        let verify_status = verify_response.status();
        let mut verify_body = match verify_response.json::<ApiResponse<Value>>().await {
            Ok(value) => value,
            Err(error) => {
                return nni_join_error(
                    StatusCode::BAD_GATEWAY,
                    "nni_owner_recovery_verify_response_invalid",
                    json!({"status": "remote_bad_response", "detail": error.to_string()}),
                )
            }
        };
        if verify_status.is_success() && verify_body.ok {
            if verify_body
                .data
                .as_ref()
                .and_then(|data| data.get("asset_owner_pubkey"))
                .and_then(Value::as_str)
                != Some(owner_pubkey.as_str())
            {
                return nni_join_error(
                    StatusCode::BAD_GATEWAY,
                    "nni_owner_recovery_identity_changed",
                    json!({"status": "owner_identity_changed"}),
                );
            }
            if let Err(error) = persist_nni_asset_owner_pubkey(&state, &owner_pubkey, false) {
                return nni_join_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "nni_asset_owner_persist_failed",
                    json!({"status": "asset_owner_persist_failed", "detail": error.to_string()}),
                );
            }
            if let Some(data) = verify_body.data.as_mut().and_then(Value::as_object_mut) {
                data.insert("node_url".to_string(), Value::String(node_url));
            }
        }
        return (
            StatusCode::from_u16(verify_status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            Json(verify_body),
        );
    }

    let device_pubkey = match nni_device_pubkey(&state).await {
        Ok(value) => value,
        Err((status, error, data)) => return nni_join_error(status, error, data),
    };

    let challenge_response = match state
        .core
        .public_http_client
        .post(nni_remote_api_endpoint(&node_url, "asset-owner/recovery/request"))
        .timeout(nni_remote_api_timeout())
        .json(&NniRemoteOwnerRecoveryRequest {
            asset_owner_pubkey: owner_pubkey.clone(),
            new_device_pubkey: device_pubkey.clone(),
            client_user_key: identity.user_key,
        })
        .send()
        .await
    {
        Ok(value) => value,
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_GATEWAY,
                "nni_owner_recovery_request_failed",
                json!({"status": "remote_request_failed", "detail": error.to_string()}),
            );
        }
    };
    let challenge_status = challenge_response.status();
    let challenge_body = match challenge_response.json::<ApiResponse<Value>>().await {
        Ok(value) => value,
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_GATEWAY,
                "nni_owner_recovery_response_invalid",
                json!({"status": "remote_bad_response", "detail": error.to_string()}),
            );
        }
    };
    if !challenge_status.is_success() || !challenge_body.ok {
        let error = nni_remote_api_error_code(
            &challenge_body,
            "nni_owner_recovery_request_rejected",
        );
        return nni_join_error(
            StatusCode::from_u16(challenge_status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            error,
            challenge_body.data.unwrap_or_else(|| json!({"status": "recovery_rejected"})),
        );
    }
    let Some(mut challenge_data) = challenge_body.data else {
        return nni_join_error(
            StatusCode::BAD_GATEWAY,
            "nni_owner_recovery_challenge_missing",
            json!({"status": "remote_bad_response"}),
        );
    };
    let Some(task_id) = challenge_data
        .get("task_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 160)
        .map(str::to_string)
    else {
        return nni_join_error(
            StatusCode::BAD_GATEWAY,
            "nni_owner_recovery_task_id_missing",
            json!({"status": "remote_bad_response"}),
        );
    };
    let Some(signing_payload) = challenge_data
        .get("signing_payload")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 4096)
        .map(str::to_string)
    else {
        return nni_join_error(
            StatusCode::BAD_GATEWAY,
            "nni_owner_recovery_payload_missing",
            json!({"status": "remote_bad_response"}),
        );
    };
    let payload: Value = match serde_json::from_str(&signing_payload) {
        Ok(value) => value,
        Err(_) => {
            return nni_join_error(
                StatusCode::BAD_GATEWAY,
                "nni_owner_recovery_payload_invalid",
                json!({"status": "remote_bad_response"}),
            )
        }
    };
    if payload.get("schema_version").and_then(Value::as_u64) != Some(1)
        || payload.get("action").and_then(Value::as_str) != Some("rotate_asset_device")
        || payload.get("server_identity").and_then(Value::as_str) != Some("nni-server-v1")
        || payload.get("task_id").and_then(Value::as_str) != Some(task_id.as_str())
        || payload.get("asset_owner_pubkey").and_then(Value::as_str) != Some(owner_pubkey.as_str())
        || payload.get("device_pubkey").and_then(Value::as_str) != Some(device_pubkey.as_str())
        || challenge_data.get("asset_owner_pubkey").and_then(Value::as_str)
            != Some(owner_pubkey.as_str())
        || challenge_data.get("new_device_pubkey").and_then(Value::as_str)
            != Some(device_pubkey.as_str())
    {
        return nni_join_error(
            StatusCode::BAD_GATEWAY,
            "nni_owner_recovery_payload_binding_invalid",
            json!({"status": "remote_bad_response"}),
        );
    }
    let hardware_signature = match run_nni_signature_helper(
        &state,
        &["sign_challenge".to_string(), signing_payload],
    )
    .await
    {
        Ok(output) if output.ok => output
            .payload
            .get("signature")
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    };
    let Some(device_signature) = hardware_signature else {
        return nni_join_error(
            StatusCode::BAD_GATEWAY,
            "nni_owner_recovery_device_signature_failed",
            json!({"status": "device_signature_failed"}),
        );
    };
    if let Some(data) = challenge_data.as_object_mut() {
        data.insert("node_url".to_string(), Value::String(node_url));
        data.insert(
            "device_signature".to_string(),
            Value::String(device_signature),
        );
    }
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(challenge_data),
            error: None,
        }),
    )
}

async fn nni_owner_unbind_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<NniOwnerUnbindRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_admin(&state, &headers) {
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
    if let Err(error) = clear_nni_asset_owner_binding(&state) {
        return nni_join_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "nni_asset_owner_clear_failed",
            json!({"status": "asset_owner_clear_failed", "detail": error.to_string()}),
        );
    }
    let node_url = match normalize_nni_node_url(&req.node_url) {
        Ok(value) => value,
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                error,
                json!({"status": "remote_node_invalid"}),
            );
        }
    };
    let device_pubkey = match nni_device_pubkey(&state).await {
        Ok(value) => value,
        Err((status, error, data)) => return nni_join_error(status, error, data),
    };
    let response = match state
        .core
        .public_http_client
        .post(nni_remote_api_endpoint(
            &node_url,
            "asset-owner/unbind/request",
        ))
        .timeout(nni_remote_api_timeout())
        .json(&NniRemoteOwnerUnbindRequest {
            device_pubkey,
            client_user_key: identity.user_key,
        })
        .send()
        .await
    {
        Ok(value) => value,
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_GATEWAY,
                "nni_owner_unbind_request_failed",
                json!({
                    "status": "remote_request_failed",
                    "detail": error.to_string(),
                    "local_binding_cleared": true,
                }),
            );
        }
    };
    let status = response.status();
    let mut body = match response.json::<ApiResponse<Value>>().await {
        Ok(value) => value,
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_GATEWAY,
                "nni_owner_unbind_response_invalid",
                json!({
                    "status": "remote_bad_response",
                    "detail": error.to_string(),
                    "local_binding_cleared": true,
                }),
            );
        }
    };
    if let Some(data) = body.data.as_mut().and_then(Value::as_object_mut) {
        data.insert("node_url".to_string(), Value::String(node_url));
        data.insert("local_binding_cleared".to_string(), Value::Bool(true));
    }
    (
        StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
        Json(body),
    )
}

async fn nni_owner_unbind_verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<NniOwnerUnbindVerifyRequest>,
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
    let node_url = match normalize_nni_node_url(&req.node_url) {
        Ok(value) => value,
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                error,
                json!({"status": "remote_node_invalid"}),
            );
        }
    };
    let task_id = req.task_id.trim();
    if task_id.is_empty() {
        return nni_join_error(
            StatusCode::BAD_REQUEST,
            "nni_asset_unbind_task_id_required",
            json!({"status": "task_id_required"}),
        );
    }
    let device_signature = match normalize_nni_device_signature(&req.device_signature) {
        Ok(value) => value,
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                error,
                json!({"status": "device_signature_invalid"}),
            );
        }
    };
    let response = match state
        .core
        .public_http_client
        .post(nni_remote_api_endpoint(
            &node_url,
            "asset-owner/unbind/verify",
        ))
        .timeout(nni_remote_api_timeout())
        .json(&NniRemoteOwnerUnbindVerifyRequest {
            task_id: task_id.to_string(),
            device_signature,
        })
        .send()
        .await
    {
        Ok(value) => value,
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_GATEWAY,
                "nni_owner_unbind_verify_failed",
                json!({"status": "remote_request_failed", "detail": error.to_string()}),
            );
        }
    };
    let status = response.status();
    let mut body = match response.json::<ApiResponse<Value>>().await {
        Ok(value) => value,
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_GATEWAY,
                "nni_owner_unbind_verify_response_invalid",
                json!({"status": "remote_bad_response", "detail": error.to_string()}),
            );
        }
    };
    if status.is_success() && body.ok {
        if let Err(error) = clear_nni_asset_owner_binding(&state) {
            return nni_join_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "nni_asset_owner_clear_failed",
                json!({"status": "asset_owner_clear_failed", "detail": error.to_string()}),
            );
        }
        if let Some(data) = body.data.as_mut().and_then(Value::as_object_mut) {
            data.insert("node_url".to_string(), Value::String(node_url));
            data.insert("joined".to_string(), Value::Bool(false));
        }
    }
    (
        StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
        Json(body),
    )
}

async fn nni_request_records(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<NniRequestRecordsQuery>,
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
    };

    match read_nni_request_records(&state) {
        Ok(records) => {
            let page = query.page.unwrap_or(1).max(1);
            let per_page = query.per_page.unwrap_or(10).clamp(1, 100);
            let total = records.len();
            let total_pages = total.div_ceil(per_page).max(1);
            let start = page.saturating_sub(1).saturating_mul(per_page).min(total);
            let end = start.saturating_add(per_page).min(total);
            let page_records = records[start..end].to_vec();
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({
                        "status": "local_request_records",
                        "page": page,
                        "per_page": per_page,
                        "total": total,
                        "total_pages": total_pages,
                        "records": page_records,
                    })),
                    error: None,
                }),
            )
        }
        Err(err) => nni_join_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "nni_request_records_read_failed",
            json!({"status": "request_records_read_failed", "error": err.to_string()}),
        ),
    }
}

async fn nni_clear_request_records(
    State(state): State<AppState>,
    headers: HeaderMap,
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
    };

    match clear_nni_request_records(&state) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(err) => nni_join_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "nni_request_records_clear_failed",
            json!({"status": "request_records_clear_failed", "error": err.to_string()}),
        ),
    }
}

async fn nni_heartbeat_errors(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<NniRequestRecordsQuery>,
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
    };

    match read_nni_heartbeat_error_records(&state) {
        Ok(records) => {
            let page = query.page.unwrap_or(1).max(1);
            let per_page = query.per_page.unwrap_or(10).clamp(1, 100);
            let total = records.len();
            let total_pages = total.div_ceil(per_page).max(1);
            let start = page.saturating_sub(1).saturating_mul(per_page).min(total);
            let end = start.saturating_add(per_page).min(total);
            let page_records = records[start..end].to_vec();
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({
                        "status": "ok",
                        "page": page,
                        "per_page": per_page,
                        "total": total,
                        "total_pages": total_pages,
                        "records": page_records,
                    })),
                    error: None,
                }),
            )
        }
        Err(err) => nni_join_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "nni_heartbeat_errors_read_failed",
            json!({"status": "heartbeat_errors_read_failed", "error": err.to_string()}),
        ),
    }
}

async fn nni_clear_heartbeat_errors(
    State(state): State<AppState>,
    headers: HeaderMap,
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
    };

    match clear_nni_heartbeat_error_records(&state) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(err) => nni_join_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "nni_heartbeat_errors_clear_failed",
            json!({"status": "heartbeat_errors_clear_failed", "error": err.to_string()}),
        ),
    }
}

async fn nni_device_pubkey(state: &AppState) -> Result<String, (StatusCode, &'static str, Value)> {
    let pubkey_output = match run_nni_signature_helper(state, &[String::from("pubkey")]).await {
        Ok(output) if output.ok => output,
        Ok(output) => {
            return Err((
                StatusCode::BAD_GATEWAY,
                "nni_device_pubkey_unavailable",
                json!({
                    "status": "device_pubkey_unavailable",
                    "exit_code": output.exit_code,
                    "error": output.error.or_else(|| (!output.stderr_tail.is_empty()).then_some(output.stderr_tail)),
                }),
            ));
        }
        Err(err) => {
            return Err((
                StatusCode::BAD_GATEWAY,
                "nni_signature_helper_failed",
                json!({
                    "status": "signature_helper_failed",
                    "error": err,
                }),
            ));
        }
    };
    let Some(device_pubkey) = pubkey_output
        .payload
        .get("pubkey")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    else {
        return Err((
            StatusCode::BAD_GATEWAY,
            "nni_device_pubkey_missing",
            json!({"status": "device_pubkey_missing"}),
        ));
    };
    if !is_nni_pubkey_hex(&device_pubkey) {
        return Err((
            StatusCode::BAD_GATEWAY,
            "nni_device_pubkey_invalid",
            json!({"status": "device_pubkey_invalid"}),
        ));
    }
    Ok(device_pubkey)
}

fn is_nni_pubkey_hex(pubkey_hex: &str) -> bool {
    pubkey_hex.len() == 128 && pubkey_hex.as_bytes().iter().all(u8::is_ascii_hexdigit)
}

fn normalize_nni_device_signature(value: &str) -> Result<String, &'static str> {
    let normalized = value.trim();
    if !is_nni_pubkey_hex(normalized) {
        return Err("nni_signature_invalid");
    }
    Ok(normalized.to_ascii_lowercase())
}

fn normalize_nni_node_urls(raw_urls: &[String]) -> Result<Vec<String>, &'static str> {
    let mut urls = Vec::new();
    for raw in raw_urls {
        let url = normalize_nni_node_url(raw)?;
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    Ok(urls)
}

fn normalize_nni_node_url(raw: &str) -> Result<String, &'static str> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("nni_remote_node_required");
    }
    let trimmed = trimmed.strip_suffix("/v1").unwrap_or(trimmed);
    let url = crate::public_http_client::validate_public_https_base_url(trimmed).map_err(
        |error| match error {
            "remote_url_https_required" => "nni_remote_node_https_required",
            "remote_url_non_public_address" => "nni_remote_node_non_public_address",
            "remote_url_credentials_forbidden" | "remote_url_components_forbidden" => {
                "nni_remote_node_components_invalid"
            }
            _ => "nni_remote_node_invalid",
        },
    )?;
    if url.path() != "/" && !url.path().is_empty() {
        return Err("nni_remote_node_path_invalid");
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

include!("nni_runtime_state.rs");

include!("nni_heartbeat_worker.rs");

fn toml_value_string_list(value: &toml::Value) -> Option<Vec<String>> {
    value.as_array().map(|items| {
        items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect()
    })
}

fn nni_join_error(
    status: StatusCode,
    error: impl Into<String>,
    data: Value,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    (
        status,
        Json(ApiResponse {
            ok: false,
            data: Some(data),
            error: Some(error.into()),
        }),
    )
}
