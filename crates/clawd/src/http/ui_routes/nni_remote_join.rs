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
struct NniLocalJoinRequest {
    node_url: String,
    #[serde(default)]
    asset_owner_pubkey: Option<String>,
    #[serde(default)]
    replace_existing_owner: bool,
}

#[derive(Deserialize)]
struct NniLocalJoinVerifyRequest {
    task_id: String,
    node_url: String,
    signature: String,
    #[serde(default)]
    signing_payload: Option<String>,
    #[serde(default)]
    owner_private_key: Option<String>,
    #[serde(default)]
    owner_signature: Option<String>,
    #[serde(default)]
    previous_owner_signature: Option<String>,
    #[serde(default)]
    replace_existing_owner: bool,
}

#[derive(Debug, Deserialize)]
struct NniOwnerUnbindRequest {
    node_url: String,
}

#[derive(Debug, Deserialize)]
struct NniOwnerUnbindVerifyRequest {
    task_id: String,
    node_url: String,
    device_signature: String,
}

#[derive(Deserialize)]
struct NniOwnerRecoveryRequest {
    node_url: String,
    owner_private_key: String,
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

    match write_nni_config_with_selected_node(
        &state,
        remote_nodes.as_deref(),
        req.selected_node_url.as_deref(),
        req.bancor_service_node_url.as_deref(),
        req.asset_service_node_url.as_deref(),
        req.joined,
    ) {
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
            .http_client
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
                    Ok(body) => {
                        let data_ref = body.data.as_ref();
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
                        record.error_code = Some(nni_remote_api_error_code(
                            &body,
                            "nni_remote_join_failed",
                        ));
                        record.created_at_ts =
                            Some(u64::try_from(current_unix_ts()).unwrap_or_default());
                        record_nni_request_event(&state, record);
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
    Json(mut req): Json<NniLocalJoinVerifyRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let mut owner_private_key = req.owner_private_key.take().map(Zeroizing::new);
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
    if owner_private_key.is_some() && external_owner_signature.is_some() {
        return nni_join_error(
            StatusCode::BAD_REQUEST,
            "nni_owner_signature_source_conflict",
            json!({"status": "owner_signature_source_conflict"}),
        );
    }
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
    if req.replace_existing_owner && owner_private_key.is_some() {
        return nni_join_error(
            StatusCode::BAD_REQUEST,
            "nni_owner_private_key_unexpected",
            json!({"status": "owner_private_key_unexpected"}),
        );
    }
    if req.replace_existing_owner && external_owner_signature.is_none() {
        return nni_join_error(
            StatusCode::BAD_REQUEST,
            "nni_target_owner_signature_required",
            json!({"status": "target_owner_signature_required"}),
        );
    }
    let (derived_owner_pubkey, owner_signature) = match owner_private_key.as_mut() {
        Some(private_key) => {
            let Some(signing_payload) = req
                .signing_payload
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty() && value.len() <= 4096)
            else {
                return nni_join_error(
                    StatusCode::BAD_REQUEST,
                    "nni_owner_signing_payload_required",
                    json!({"status": "owner_signing_payload_required"}),
                );
            };
            match sign_nni_owner_payload(&mut **private_key, signing_payload) {
                Ok((public_key, signature)) => (Some(public_key), Some(signature)),
                Err(error) => {
                    return nni_join_error(
                        StatusCode::BAD_REQUEST,
                        error,
                        json!({"status": "owner_private_key_invalid"}),
                    );
                }
            }
        }
        None => (None, external_owner_signature),
    };
    let endpoint = nni_remote_api_endpoint(&node_url, "join/verify");
    let response = state
        .core
        .http_client
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
                            if derived_owner_pubkey
                                .as_ref()
                                .is_some_and(|derived| derived != &normalized_owner)
                            {
                                return nni_join_error(
                                    StatusCode::CONFLICT,
                                    "nni_asset_owner_signature_mismatch",
                                    json!({"status": "asset_owner_signature_mismatch"}),
                                );
                            }
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
                    record.error_code = Some(nni_remote_api_error_code(
                        &body,
                        "nni_remote_verify_failed",
                    ));
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
    let mut owner_private_key = Zeroizing::new(req.owner_private_key);
    let requested_node_url = req.node_url;
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
    let node_url = match normalize_nni_node_url(&requested_node_url) {
        Ok(value) => value,
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                error,
                json!({"status": "remote_node_invalid"}),
            );
        }
    };
    let owner_pubkey = match nni_owner_public_key_from_private(&owner_private_key) {
        Ok(value) => value,
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                error,
                json!({"status": "owner_private_key_invalid"}),
            );
        }
    };
    let device_pubkey = match nni_device_pubkey(&state).await {
        Ok(value) => value,
        Err((status, error, data)) => return nni_join_error(status, error, data),
    };

    let challenge_response = match state
        .core
        .http_client
        .post(nni_remote_api_endpoint(&node_url, "asset-owner/recovery/request"))
        .timeout(nni_remote_api_timeout())
        .json(&NniRemoteOwnerRecoveryRequest {
            asset_owner_pubkey: owner_pubkey.clone(),
            new_device_pubkey: device_pubkey,
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
    let Some(challenge_data) = challenge_body.data else {
        return nni_join_error(
            StatusCode::BAD_GATEWAY,
            "nni_owner_recovery_challenge_missing",
            json!({"status": "remote_bad_response"}),
        );
    };
    let Some(task_id) = challenge_data
        .get("task_id")
        .and_then(Value::as_str)
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

    let (derived_owner_pubkey, owner_signature) =
        match sign_nni_owner_payload(&mut owner_private_key, &signing_payload) {
            Ok(value) => value,
            Err(error) => {
                return nni_join_error(
                    StatusCode::BAD_REQUEST,
                    error,
                    json!({"status": "owner_private_key_invalid"}),
                );
            }
        };
    if derived_owner_pubkey != owner_pubkey {
        return nni_join_error(
            StatusCode::CONFLICT,
            "nni_owner_recovery_identity_changed",
            json!({"status": "owner_identity_changed"}),
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

    let verify_response = match state
        .core
        .http_client
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
            );
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
            );
        }
    };
    if verify_status.is_success() && verify_body.ok {
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
    (
        StatusCode::from_u16(verify_status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
        Json(verify_body),
    )
}

async fn nni_owner_unbind_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<NniOwnerUnbindRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
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
        .http_client
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
                "nni_owner_unbind_response_invalid",
                json!({"status": "remote_bad_response", "detail": error.to_string()}),
            );
        }
    };
    if let Some(data) = body.data.as_mut().and_then(Value::as_object_mut) {
        data.insert("node_url".to_string(), Value::String(node_url));
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
        .http_client
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
    if let Err((status, Json(resp))) = require_ui_identity(&state, &headers) {
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
    if let Err((status, Json(resp))) = require_ui_identity(&state, &headers) {
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
    if let Err((status, Json(resp))) = require_ui_identity(&state, &headers) {
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
    if let Err((status, Json(resp))) = require_ui_identity(&state, &headers) {
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
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err("nni_remote_node_scheme_invalid");
    }
    Ok(trimmed.strip_suffix("/v1").unwrap_or(trimmed).to_string())
}

fn read_nni_config(state: &AppState) -> anyhow::Result<NniConfigResponse> {
    let path = nni_runtime_config_path(state);
    let config = read_nni_runtime_config(state)?;
    let heartbeat_state = read_nni_heartbeat_runtime_state(state)?;
    let heartbeat_state_token = if !config.joined {
        "disabled"
    } else if heartbeat_state.network_authorization == "rejected" {
        "rejected"
    } else if heartbeat_state.last_heartbeat_error.is_some() {
        if heartbeat_state.last_heartbeat_network_failures > 0 {
            "waiting_network"
        } else {
            "degraded"
        }
    } else if heartbeat_state.last_heartbeat_at_ts.is_some() {
        "active"
    } else {
        "enabling"
    };
    let now = u64::try_from(current_unix_ts()).unwrap_or_default();
    let next_heartbeat_due_at_ts = nni_next_heartbeat_due_at_ts(
        config.joined,
        heartbeat_state_token,
        &heartbeat_state,
        now,
    );
    Ok(NniConfigResponse {
        selected_node_url: config.selected_node_url,
        bancor_service_node_url: config.bancor_service_node_url,
        asset_service_node_url: config.asset_service_node_url,
        remote_nodes: config.remote_nodes,
        joined: config.joined,
        asset_owner_pubkey: config.asset_owner_pubkey,
        heartbeat_interval_seconds: NNI_HEARTBEAT_INTERVAL_SECONDS,
        heartbeat_network_retry_limit: NNI_HEARTBEAT_NETWORK_RETRY_LIMIT,
        heartbeat_request_count: heartbeat_state.heartbeat_request_count,
        last_heartbeat_at_ts: heartbeat_state.last_heartbeat_at_ts,
        last_heartbeat_error: heartbeat_state.last_heartbeat_error,
        last_heartbeat_error_code: heartbeat_state.last_heartbeat_error_code,
        last_heartbeat_error_at_ts: heartbeat_state.last_heartbeat_error_at_ts,
        last_heartbeat_network_failures: heartbeat_state.last_heartbeat_network_failures,
        last_heartbeat_attempt_at_ts: heartbeat_state.last_heartbeat_attempt_at_ts,
        consecutive_heartbeat_failures: heartbeat_state.consecutive_heartbeat_failures,
        last_success_node_host: heartbeat_state.last_success_node_host,
        network_authorization: heartbeat_state.network_authorization,
        heartbeat_state: heartbeat_state_token.to_string(),
        next_heartbeat_due_at_ts,
        worker_running: true,
        config_path: path.display().to_string(),
    })
}

fn nni_next_heartbeat_due_at_ts(
    joined: bool,
    heartbeat_state: &str,
    runtime_state: &NniHeartbeatRuntimeState,
    now: u64,
) -> Option<u64> {
    if !joined {
        return None;
    }
    let due = match heartbeat_state {
        "active" => runtime_state
            .last_heartbeat_at_ts
            .map(|base| base.saturating_add(NNI_HEARTBEAT_INTERVAL_SECONDS)),
        "enabling" | "waiting_network" | "degraded" => runtime_state
            .last_heartbeat_attempt_at_ts
            .map(|base| base.saturating_add(NNI_HEARTBEAT_POLL_SECONDS)),
        _ => return None,
    };
    Some(due.unwrap_or(now))
}

fn nni_runtime_config_path(state: &AppState) -> PathBuf {
    state
        .skill_rt
        .workspace_root
        .join("data")
        .join("nni")
        .join("runtime-config.json")
}

fn read_nni_runtime_config(state: &AppState) -> anyhow::Result<NniRuntimeConfig> {
    let path = nni_runtime_config_path(state);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Some(legacy) = read_legacy_nni_config(state)? {
                write_nni_runtime_config(state, &legacy)?;
                return Ok(legacy);
            }
            return Ok(NniRuntimeConfig::default());
        }
        Err(error) => return Err(error.into()),
    };
    if raw.trim().is_empty() {
        return Ok(NniRuntimeConfig::default());
    }
    let mut config: NniRuntimeConfig = serde_json::from_str(&raw)?;
    let migrated = config.schema_version == 1;
    if migrated {
        config.schema_version = NNI_RUNTIME_CONFIG_SCHEMA_VERSION;
    } else if config.schema_version != NNI_RUNTIME_CONFIG_SCHEMA_VERSION {
        anyhow::bail!(
            "nni_runtime_config_schema_unsupported:{}",
            config.schema_version
        );
    }
    config.remote_nodes = normalize_nni_node_urls(&config.remote_nodes)
        .map_err(|error| anyhow::anyhow!(error))?;
    config.selected_node_url = normalize_selected_nni_node(
        config.selected_node_url.as_deref(),
        &config.remote_nodes,
    )?;
    config.asset_service_node_url = normalize_selected_nni_node(
        config
            .asset_service_node_url
            .as_deref()
            .or(config.selected_node_url.as_deref()),
        &config.remote_nodes,
    )?;
    config.bancor_service_node_url = normalize_selected_nni_node(
        config
            .bancor_service_node_url
            .as_deref()
            .or(config.selected_node_url.as_deref()),
        &config.remote_nodes,
    )?;
    config.asset_owner_pubkey = config
        .asset_owner_pubkey
        .as_deref()
        .map(normalize_nni_owner_public_key)
        .transpose()
        .map_err(anyhow::Error::msg)?;
    if migrated {
        write_nni_runtime_config(state, &config)?;
    }
    Ok(config)
}

fn read_legacy_nni_config(state: &AppState) -> anyhow::Result<Option<NniRuntimeConfig>> {
    let path = state.skill_rt.workspace_root.join("configs/config.toml");
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let parsed: toml::Value = toml::from_str(&raw)?;
    let Some(nni) = parsed.get("nni") else {
        return Ok(None);
    };
    let remote_nodes = nni
        .get("remote_nodes")
        .and_then(toml_value_string_list)
        .map(|values| normalize_nni_node_urls(&values))
        .transpose()
        .map_err(|error| anyhow::anyhow!(error))?
        .unwrap_or_default();
    let joined = nni
        .get("joined")
        .and_then(toml::Value::as_bool)
        .unwrap_or(false);
    Ok(Some(NniRuntimeConfig {
        schema_version: NNI_RUNTIME_CONFIG_SCHEMA_VERSION,
        selected_node_url: remote_nodes.first().cloned(),
        bancor_service_node_url: remote_nodes.first().cloned(),
        asset_service_node_url: remote_nodes.first().cloned(),
        remote_nodes,
        joined,
        asset_owner_pubkey: None,
    }))
}

fn persist_nni_asset_owner_pubkey(
    state: &AppState,
    owner_pubkey: &str,
    replace_existing: bool,
) -> anyhow::Result<()> {
    let normalized = normalize_nni_owner_public_key(owner_pubkey).map_err(anyhow::Error::msg)?;
    let mut config = read_nni_runtime_config(state)?;
    if !replace_existing
        && config
        .asset_owner_pubkey
        .as_ref()
        .is_some_and(|current| current != &normalized)
    {
        anyhow::bail!("nni_asset_owner_conflict");
    }
    config.asset_owner_pubkey = Some(normalized);
    write_nni_runtime_config(state, &config)
}

fn clear_nni_asset_owner_binding(state: &AppState) -> anyhow::Result<()> {
    let mut config = read_nni_runtime_config(state)?;
    config.asset_owner_pubkey = None;
    config.joined = false;
    write_nni_runtime_config(state, &config)
}

fn write_nni_runtime_config(
    state: &AppState,
    config: &NniRuntimeConfig,
) -> anyhow::Result<()> {
    let path = nni_runtime_config_path(state);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("nni_runtime_config_parent_missing"))?;
    fs::create_dir_all(parent)?;
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(
        ".runtime-config.{}.{}.tmp",
        std::process::id(),
        suffix
    ));
    let result = (|| -> anyhow::Result<()> {
        let mut bytes = serde_json::to_vec_pretty(config)?;
        bytes.push(b'\n');
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, &path)?;
        if let Ok(directory) = fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn nni_heartbeat_runtime_state_path(state: &AppState) -> PathBuf {
    state
        .skill_rt
        .workspace_root
        .join("data")
        .join("nni")
        .join("heartbeat-state.json")
}

fn read_nni_heartbeat_runtime_state(state: &AppState) -> anyhow::Result<NniHeartbeatRuntimeState> {
    let path = nni_heartbeat_runtime_state_path(state);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(NniHeartbeatRuntimeState::default());
        }
        Err(error) => return Err(error.into()),
    };
    if raw.trim().is_empty() {
        return Ok(NniHeartbeatRuntimeState::default());
    }
    let mut runtime_state: NniHeartbeatRuntimeState = serde_json::from_str(&raw)?;
    if !matches!(runtime_state.schema_version, 1 | NNI_HEARTBEAT_RUNTIME_STATE_SCHEMA_VERSION) {
        anyhow::bail!(
            "nni_heartbeat_runtime_state_schema_unsupported:{}",
            runtime_state.schema_version
        );
    }
    runtime_state.schema_version = NNI_HEARTBEAT_RUNTIME_STATE_SCHEMA_VERSION;
    if runtime_state.network_authorization.trim().is_empty() {
        runtime_state.network_authorization = nni_unknown_network_authorization();
    }
    Ok(runtime_state)
}

fn write_nni_heartbeat_runtime_state(
    state: &AppState,
    runtime_state: &NniHeartbeatRuntimeState,
) -> anyhow::Result<()> {
    let path = nni_heartbeat_runtime_state_path(state);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("nni_heartbeat_runtime_state_parent_missing"))?;
    fs::create_dir_all(parent)?;
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(
        ".heartbeat-state.{}.{}.tmp",
        std::process::id(),
        suffix
    ));
    let result = (|| -> anyhow::Result<()> {
        let mut bytes = serde_json::to_vec_pretty(runtime_state)?;
        bytes.push(b'\n');
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, &path)?;
        if let Ok(directory) = fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn read_nni_heartbeat_error_records(
    state: &AppState,
) -> anyhow::Result<Vec<NniHeartbeatErrorRecord>> {
    let mut records = read_nni_heartbeat_error_records_from_log(state)?;
    records.sort_by(|left, right| {
        let ts_order = right
            .created_at_ts
            .unwrap_or_default()
            .cmp(&left.created_at_ts.unwrap_or_default());
        ts_order.then_with(|| right.id.cmp(&left.id))
    });
    records.truncate(NNI_HEARTBEAT_ERROR_HISTORY_LIMIT);
    Ok(records)
}

fn clear_nni_heartbeat_error_records(state: &AppState) -> anyhow::Result<Value> {
    let existing_count = read_nni_heartbeat_error_records(state)?.len();
    let mut runtime_state = read_nni_heartbeat_runtime_state(state)?;
    runtime_state.last_heartbeat_error = None;
    runtime_state.last_heartbeat_error_code = None;
    runtime_state.last_heartbeat_error_at_ts = None;
    runtime_state.last_heartbeat_network_failures = 0;
    runtime_state.consecutive_heartbeat_failures = 0;
    write_nni_heartbeat_runtime_state(state, &runtime_state)?;
    rewrite_nni_log_without_event_kinds(
        state,
        &[
            "heartbeat_error_record",
            "heartbeat_failed",
            "heartbeat_tick_error",
            "heartbeat_network_retry",
        ],
    )?;
    Ok(json!({
        "status": "nni_heartbeat_errors_cleared",
        "deleted_records": existing_count,
        "runtime_state_path": nni_heartbeat_runtime_state_path(state).display().to_string(),
        "log_path": nni_log_path(state).display().to_string(),
    }))
}

fn read_nni_heartbeat_error_records_from_log(
    state: &AppState,
) -> anyhow::Result<Vec<NniHeartbeatErrorRecord>> {
    Ok(read_nni_log_payloads(state, "heartbeat_error_record")?
        .into_iter()
        .filter_map(|payload| serde_json::from_value::<NniHeartbeatErrorRecord>(payload).ok())
        .collect())
}

fn record_nni_heartbeat_error_event(
    state: &AppState,
    error: &str,
    created_at_ts: Option<u64>,
    network: bool,
) {
    let next_id = read_nni_heartbeat_error_records(state)
        .unwrap_or_default()
        .iter()
        .map(|record| record.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let record = NniHeartbeatErrorRecord {
        id: next_id,
        created_at_ts,
        error: error.to_string(),
        network,
    };
    append_nni_log_event_best_effort(
        state,
        "heartbeat_error_record",
        serde_json::to_value(record).unwrap_or_else(|_| json!({})),
    );
}

fn write_nni_config(
    state: &AppState,
    remote_nodes: Option<&[String]>,
    joined: Option<bool>,
) -> anyhow::Result<NniConfigResponse> {
    write_nni_config_with_selected_node(state, remote_nodes, None, None, None, joined)
}

fn write_nni_config_with_selected_node(
    state: &AppState,
    remote_nodes: Option<&[String]>,
    selected_node_url: Option<&str>,
    bancor_service_node_url: Option<&str>,
    asset_service_node_url: Option<&str>,
    joined: Option<bool>,
) -> anyhow::Result<NniConfigResponse> {
    let mut config = read_nni_runtime_config(state)?;
    let previous_selected_node_url = config.selected_node_url.clone();
    if let Some(remote_nodes) = remote_nodes {
        config.remote_nodes = normalize_nni_node_urls(remote_nodes)
            .map_err(|error| anyhow::anyhow!(error))?;
    }
    let next_selected_node_url = normalize_selected_nni_node(
        selected_node_url.or(config.selected_node_url.as_deref()),
        &config.remote_nodes,
    )?;
    if config.joined
        && joined != Some(false)
        && previous_selected_node_url != next_selected_node_url
    {
        anyhow::bail!("nni_selected_node_change_requires_stop");
    }
    config.selected_node_url = next_selected_node_url;
    let current_bancor_node = config
        .bancor_service_node_url
        .as_deref()
        .filter(|candidate| config.remote_nodes.iter().any(|node| node == *candidate));
    config.bancor_service_node_url = normalize_selected_nni_node(
        bancor_service_node_url
            .or(current_bancor_node)
            .or(config.selected_node_url.as_deref()),
        &config.remote_nodes,
    )?;
    let current_asset_node = config
        .asset_service_node_url
        .as_deref()
        .filter(|candidate| config.remote_nodes.iter().any(|node| node == *candidate));
    config.asset_service_node_url = normalize_selected_nni_node(
        asset_service_node_url
            .or(current_asset_node)
            .or(config.selected_node_url.as_deref()),
        &config.remote_nodes,
    )?;
    if let Some(joined) = joined {
        config.joined = joined;
    }
    write_nni_runtime_config(state, &config)?;
    read_nni_config(state)
}

fn normalize_selected_nni_node(
    selected_node_url: Option<&str>,
    remote_nodes: &[String],
) -> anyhow::Result<Option<String>> {
    if remote_nodes.is_empty() {
        return Ok(None);
    }
    let selected = selected_node_url
        .map(normalize_nni_node_url)
        .transpose()
        .map_err(|error| anyhow::anyhow!(error))?;
    match selected {
        Some(selected) if remote_nodes.contains(&selected) => Ok(Some(selected)),
        Some(_) => anyhow::bail!("nni_selected_node_not_bound"),
        None => Ok(remote_nodes.first().cloned()),
    }
}

struct NniHeartbeatStatusUpdate<'a> {
    heartbeat_at_ts: Option<u64>,
    attempt_at_ts: Option<u64>,
    error: Option<&'a str>,
    error_code: Option<&'a str>,
    error_at_ts: Option<u64>,
    error_network: bool,
    request_count: Option<u64>,
    network_failures: Option<u64>,
    success_node_url: Option<&'a str>,
    network_authorization: Option<&'a str>,
}

fn nni_node_host(node_url: &str) -> Option<String> {
    reqwest::Url::parse(node_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
}

fn write_nni_heartbeat_status(
    state: &AppState,
    update: NniHeartbeatStatusUpdate<'_>,
) -> anyhow::Result<NniConfigResponse> {
    let mut runtime_state = read_nni_heartbeat_runtime_state(state)?;
    if let Some(ts) = update.heartbeat_at_ts {
        runtime_state.last_heartbeat_at_ts = Some(ts);
        runtime_state.consecutive_heartbeat_failures = 0;
    }
    if let Some(ts) = update.attempt_at_ts {
        runtime_state.last_heartbeat_attempt_at_ts = Some(ts);
    }
    if let Some(count) = update.request_count {
        runtime_state.heartbeat_request_count = count;
    }
    if let Some(count) = update.network_failures {
        runtime_state.last_heartbeat_network_failures = count;
    }
    runtime_state.last_heartbeat_error = update
        .error
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    runtime_state.last_heartbeat_error_code = update
        .error_code
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    runtime_state.last_heartbeat_error_at_ts = update.error_at_ts.filter(|value| *value > 0);
    if runtime_state.last_heartbeat_error.is_some() {
        runtime_state.consecutive_heartbeat_failures =
            runtime_state.consecutive_heartbeat_failures.saturating_add(1);
    }
    if let Some(node_url) = update.success_node_url {
        runtime_state.last_success_node_host = nni_node_host(node_url);
    }
    if let Some(authorization) = update.network_authorization {
        runtime_state.network_authorization = authorization.to_string();
    }
    write_nni_heartbeat_runtime_state(state, &runtime_state)?;
    if let Some(error) = update
        .error
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        record_nni_heartbeat_error_event(
            state,
            error,
            update.error_at_ts,
            update.error_network,
        );
    }
    read_nni_config(state)
}

fn nni_heartbeat_operation_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
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
            if let Err(err) = nni_heartbeat_tick(&state).await {
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
            tokio::time::sleep(Duration::from_secs(nni_heartbeat_worker_sleep_seconds(
                next_due_at_ts,
                now,
            )))
            .await;
        }
    });
}

async fn nni_heartbeat_tick(state: &AppState) -> anyhow::Result<()> {
    let _guard = nni_heartbeat_operation_lock().lock().await;
    let config = read_nni_config(state)?;
    if !config.joined || nni_selected_remote_node(&config).is_none() {
        return Ok(());
    }
    let now = u64::try_from(current_unix_ts()).unwrap_or_default();
    if config
        .next_heartbeat_due_at_ts
        .is_some_and(|next_due| now < next_due)
    {
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
        .http_client
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
        .http_client
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
