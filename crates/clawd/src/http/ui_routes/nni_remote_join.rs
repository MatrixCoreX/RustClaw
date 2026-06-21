const NNI_REMOTE_JOIN_TIMEOUT_SECONDS: u64 = 20;
const NNI_HEARTBEAT_INTERVAL_SECONDS: u64 = 15 * 60;
const NNI_HEARTBEAT_POLL_SECONDS: u64 = 60;
const NNI_HEARTBEAT_NETWORK_RETRY_LIMIT: usize = 3;
const NNI_HEARTBEAT_NETWORK_RETRY_DELAY_SECONDS: u64 = 2;
const NNI_HEARTBEAT_USER_KEY: &str = "clawd-nni-heartbeat";
const NNI_HEARTBEAT_ERROR_HISTORY_LIMIT: usize = 200;

#[derive(Debug, Serialize)]
struct NniConfigResponse {
    remote_nodes: Vec<String>,
    joined: bool,
    heartbeat_interval_seconds: u64,
    heartbeat_network_retry_limit: usize,
    heartbeat_request_count: u64,
    last_heartbeat_at_ts: Option<u64>,
    last_heartbeat_error: Option<String>,
    last_heartbeat_error_at_ts: Option<u64>,
    last_heartbeat_network_failures: u64,
    config_path: String,
}

#[derive(Debug, Deserialize)]
struct NniConfigUpdateRequest {
    #[serde(default)]
    remote_nodes: Option<Vec<String>>,
    #[serde(default)]
    joined: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct NniLocalJoinRequest {
    #[serde(default)]
    node_urls: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct NniLocalJoinVerifyRequest {
    task_id: String,
    node_url: String,
    signature: String,
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
}

#[derive(Debug, Serialize, Deserialize)]
struct NniRemoteJoinVerifyRequest {
    task_id: String,
    signature: String,
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

#[derive(Debug)]
struct NniHeartbeatError {
    message: String,
    network: bool,
}

impl NniHeartbeatError {
    fn network(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            network: true,
        }
    }

    fn non_network(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            network: false,
        }
    }
}

impl std::fmt::Display for NniHeartbeatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
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

    match write_nni_config(&state, remote_nodes.as_deref(), req.joined) {
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

    let node_urls = match normalize_nni_node_urls(&req.node_urls) {
        Ok(urls) if !urls.is_empty() => urls,
        Ok(_) => {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                "nni_remote_node_required",
                json!({"status": "remote_node_required"}),
            );
        }
        Err(err) => {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                err,
                json!({"status": "remote_node_invalid"}),
            );
        }
    };

    let device_pubkey = match nni_device_pubkey(&state).await {
        Ok(pubkey) => pubkey,
        Err((status, error, data)) => return nni_join_error(status, error, data),
    };

    let mut attempts = Vec::new();
    for node_url in node_urls {
        let endpoint = format!("{}/v1/nni/server/join/request", node_url);
        let response = state
            .core
            .http_client
            .post(&endpoint)
            .timeout(Duration::from_secs(NNI_REMOTE_JOIN_TIMEOUT_SECONDS))
            .json(&NniRemoteJoinRequest {
                device_pubkey: device_pubkey.clone(),
                client_user_key: identity.user_key.clone(),
            })
            .send()
            .await;

        match response {
            Ok(resp) => {
                let status = resp.status();
                match resp.json::<ApiResponse<Value>>().await {
                    Ok(mut body) if status.is_success() && body.ok => {
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
                    Ok(body) => attempts.push(json!({
                        "node_url": node_url,
                        "http_status": status.as_u16(),
                        "error": body.error,
                        "data": body.data,
                    })),
                    Err(err) => attempts.push(json!({
                        "node_url": node_url,
                        "http_status": status.as_u16(),
                        "error": format!("nni_remote_bad_response: {err}"),
                    })),
                }
            }
            Err(err) => attempts.push(json!({
                "node_url": node_url,
                "error": format!("nni_remote_request_failed: {err}"),
            })),
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

    let node_url = match normalize_nni_node_url(&req.node_url) {
        Ok(url) => url,
        Err(err) => {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                err,
                json!({"status": "remote_node_invalid"}),
            );
        }
    };
    let endpoint = format!("{}/v1/nni/server/join/verify", node_url);
    let response = state
        .core
        .http_client
        .post(&endpoint)
        .timeout(Duration::from_secs(NNI_REMOTE_JOIN_TIMEOUT_SECONDS))
        .json(&NniRemoteJoinVerifyRequest {
            task_id: req.task_id.trim().to_string(),
            signature: req.signature.trim().to_string(),
        })
        .send()
        .await;

    match response {
        Ok(resp) => {
            let status = resp.status();
            match resp.json::<ApiResponse<Value>>().await {
                Ok(mut body) => {
                    if let Some(data) = body.data.as_mut().and_then(|value| value.as_object_mut()) {
                        data.insert("node_url".to_string(), Value::String(node_url));
                    }
                    let axum_status =
                        StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
                    (axum_status, Json(body))
                }
                Err(err) => nni_join_error(
                    StatusCode::BAD_GATEWAY,
                    "nni_remote_bad_response",
                    json!({"status": "remote_bad_response", "error": err.to_string()}),
                ),
            }
        }
        Err(err) => nni_join_error(
            StatusCode::BAD_GATEWAY,
            "nni_remote_request_failed",
            json!({"status": "remote_request_failed", "error": err.to_string()}),
        ),
    }
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

    let config = match read_nni_config(&state) {
        Ok(config) => config,
        Err(err) => {
            return nni_join_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "nni_config_read_failed",
                json!({"status": "config_read_failed", "error": err.to_string()}),
            );
        }
    };
    if config.remote_nodes.is_empty() {
        return nni_join_error(
            StatusCode::BAD_REQUEST,
            "nni_remote_nodes_unavailable",
            json!({"status": "remote_nodes_unavailable", "attempts": []}),
        );
    }

    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(10).clamp(1, 100);
    let mut attempts = Vec::new();
    for node_url in config.remote_nodes {
        for endpoint_path in ["/v1/nni/server/records", "/v1/nni/server/heartbeat/records"] {
            let endpoint = format!("{node_url}{endpoint_path}?page={page}&per_page={per_page}");
            let response = state
                .core
                .http_client
                .get(&endpoint)
                .timeout(Duration::from_secs(NNI_REMOTE_JOIN_TIMEOUT_SECONDS))
                .send()
                .await;

            match response {
                Ok(resp) => {
                    let status = resp.status();
                    match resp.json::<ApiResponse<Value>>().await {
                        Ok(mut body) if status.is_success() && body.ok => {
                            let data = body.data.get_or_insert_with(|| json!({}));
                            if let Some(obj) = data.as_object_mut() {
                                obj.insert("node_url".to_string(), Value::String(node_url));
                                obj.insert(
                                    "records_endpoint".to_string(),
                                    Value::String(endpoint_path.to_string()),
                                );
                            }
                            return (StatusCode::OK, Json(body));
                        }
                        Ok(body) => attempts.push(json!({
                            "node_url": node_url,
                            "endpoint": endpoint_path,
                            "http_status": status.as_u16(),
                            "error": body.error,
                            "data": body.data,
                        })),
                        Err(err) => attempts.push(json!({
                            "node_url": node_url,
                            "endpoint": endpoint_path,
                            "http_status": status.as_u16(),
                            "error": format!("nni_remote_bad_response: {err}"),
                        })),
                    }
                }
                Err(err) => attempts.push(json!({
                    "node_url": node_url,
                    "endpoint": endpoint_path,
                    "error": format!("nni_remote_request_failed: {err}"),
                })),
            }
        }
    }

    nni_join_error(
        StatusCode::BAD_GATEWAY,
        "nni_request_records_unavailable",
        json!({
            "status": "request_records_unavailable",
            "attempts": attempts,
        }),
    )
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
    let path = state.skill_rt.workspace_root.join("configs/config.toml");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| String::new());
    let parsed: toml::Value =
        toml::from_str(&raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let remote_nodes = parsed
        .get("nni")
        .and_then(|value| value.get("remote_nodes"))
        .and_then(toml_value_string_list)
        .map(|values| normalize_nni_node_urls(&values).unwrap_or_default())
        .unwrap_or_default();
    let joined = parsed
        .get("nni")
        .and_then(|value| value.get("joined"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false);
    let last_heartbeat_at_ts = parsed
        .get("nni")
        .and_then(|value| value.get("last_heartbeat_at_ts"))
        .and_then(toml::Value::as_integer)
        .and_then(|value| u64::try_from(value).ok());
    let heartbeat_request_count = parsed
        .get("nni")
        .and_then(|value| value.get("heartbeat_request_count"))
        .and_then(toml::Value::as_integer)
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(0);
    let last_heartbeat_network_failures = parsed
        .get("nni")
        .and_then(|value| value.get("last_heartbeat_network_failures"))
        .and_then(toml::Value::as_integer)
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(0);
    let last_heartbeat_error = parsed
        .get("nni")
        .and_then(|value| value.get("last_heartbeat_error"))
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let last_heartbeat_error_at_ts = parsed
        .get("nni")
        .and_then(|value| value.get("last_heartbeat_error_at_ts"))
        .and_then(toml::Value::as_integer)
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| *value > 0);
    Ok(NniConfigResponse {
        remote_nodes,
        joined,
        heartbeat_interval_seconds: NNI_HEARTBEAT_INTERVAL_SECONDS,
        heartbeat_network_retry_limit: NNI_HEARTBEAT_NETWORK_RETRY_LIMIT,
        heartbeat_request_count,
        last_heartbeat_at_ts,
        last_heartbeat_error,
        last_heartbeat_error_at_ts,
        last_heartbeat_network_failures,
        config_path: path.display().to_string(),
    })
}

fn read_nni_heartbeat_error_records(
    state: &AppState,
) -> anyhow::Result<Vec<NniHeartbeatErrorRecord>> {
    let path = state.skill_rt.workspace_root.join("configs/config.toml");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| String::new());
    let parsed: toml::Value =
        toml::from_str(&raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let mut records = parse_nni_heartbeat_error_records(&parsed);
    let last_error = parsed
        .get("nni")
        .and_then(|value| value.get("last_heartbeat_error"))
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if let Some(error) = last_error {
        let created_at_ts = parsed
            .get("nni")
            .and_then(|value| value.get("last_heartbeat_error_at_ts"))
            .and_then(toml::Value::as_integer)
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0);
        let already_recorded = records
            .iter()
            .any(|record| record.error == error && record.created_at_ts == created_at_ts);
        if !already_recorded {
            let next_id = records
                .iter()
                .map(|record| record.id)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            records.insert(
                0,
                NniHeartbeatErrorRecord {
                    id: next_id,
                    created_at_ts,
                    error,
                    network: false,
                },
            );
        }
    }
    Ok(records)
}

fn clear_nni_heartbeat_error_records(state: &AppState) -> anyhow::Result<Value> {
    let existing_count = read_nni_heartbeat_error_records(state)?.len();
    let path = state.skill_rt.workspace_root.join("configs/config.toml");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| String::new());
    let mut output = raw;
    output = upsert_section_key_line(
        &output,
        "nni",
        "last_heartbeat_error",
        &toml::Value::String(String::new()).to_string(),
    );
    output = upsert_section_key_line(&output, "nni", "last_heartbeat_error_at_ts", "0");
    output = upsert_section_key_line(&output, "nni", "last_heartbeat_network_failures", "0");
    output = upsert_section_key_line(&output, "nni", "heartbeat_error_records", "[]");
    write_runtime_config_file(state, &output)?;
    Ok(json!({
        "status": "nni_heartbeat_errors_cleared",
        "deleted_records": existing_count,
        "config_path": path.display().to_string(),
    }))
}

fn parse_nni_heartbeat_error_records(parsed: &toml::Value) -> Vec<NniHeartbeatErrorRecord> {
    parsed
        .get("nni")
        .and_then(|value| value.get("heartbeat_error_records"))
        .and_then(toml::Value::as_array)
        .map(|records| {
            records
                .iter()
                .filter_map(parse_nni_heartbeat_error_record)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_nni_heartbeat_error_record(value: &toml::Value) -> Option<NniHeartbeatErrorRecord> {
    let table = value.as_table()?;
    let error = table
        .get("error")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let id = table
        .get("id")
        .and_then(toml::Value::as_integer)
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(0);
    let created_at_ts = table
        .get("created_at_ts")
        .and_then(toml::Value::as_integer)
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| *value > 0);
    let network = table
        .get("network")
        .and_then(toml::Value::as_bool)
        .unwrap_or(false);
    Some(NniHeartbeatErrorRecord {
        id,
        created_at_ts,
        error,
        network,
    })
}

fn render_nni_heartbeat_error_records(records: &[NniHeartbeatErrorRecord]) -> String {
    let values = records
        .iter()
        .map(|record| {
            let mut table = toml::map::Map::new();
            table.insert(
                "id".to_string(),
                toml::Value::Integer(i64::try_from(record.id).unwrap_or(i64::MAX)),
            );
            if let Some(created_at_ts) = record.created_at_ts {
                table.insert(
                    "created_at_ts".to_string(),
                    toml::Value::Integer(i64::try_from(created_at_ts).unwrap_or(i64::MAX)),
                );
            }
            table.insert("error".to_string(), toml::Value::String(record.error.clone()));
            table.insert("network".to_string(), toml::Value::Boolean(record.network));
            toml::Value::Table(table)
        })
        .collect::<Vec<_>>();
    toml::Value::Array(values).to_string()
}

fn write_nni_config(
    state: &AppState,
    remote_nodes: Option<&[String]>,
    joined: Option<bool>,
) -> anyhow::Result<NniConfigResponse> {
    let path = state.skill_rt.workspace_root.join("configs/config.toml");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| String::new());
    let mut output = raw;
    if let Some(remote_nodes) = remote_nodes {
        let rendered_nodes = toml::Value::Array(
            remote_nodes
                .iter()
                .map(|node| toml::Value::String(node.clone()))
                .collect(),
        )
        .to_string();
        output = upsert_section_key_line(&output, "nni", "remote_nodes", &rendered_nodes);
    }
    if let Some(joined) = joined {
        output = upsert_section_key_line(
            &output,
            "nni",
            "joined",
            if joined { "true" } else { "false" },
        );
    }
    write_runtime_config_file(state, &output)?;
    read_nni_config(state)
}

fn write_nni_heartbeat_status(
    state: &AppState,
    heartbeat_at_ts: Option<u64>,
    error: Option<&str>,
    error_at_ts: Option<u64>,
    error_network: Option<bool>,
    request_count: Option<u64>,
    network_failures: Option<u64>,
) -> anyhow::Result<NniConfigResponse> {
    let path = state.skill_rt.workspace_root.join("configs/config.toml");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| String::new());
    let parsed: toml::Value =
        toml::from_str(&raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let mut output = raw;
    if let Some(ts) = heartbeat_at_ts {
        output = upsert_section_key_line(&output, "nni", "last_heartbeat_at_ts", &ts.to_string());
    }
    if let Some(count) = request_count {
        output = upsert_section_key_line(
            &output,
            "nni",
            "heartbeat_request_count",
            &count.to_string(),
        );
    }
    if let Some(count) = network_failures {
        output = upsert_section_key_line(
            &output,
            "nni",
            "last_heartbeat_network_failures",
            &count.to_string(),
        );
    }
    let rendered_error = toml::Value::String(error.unwrap_or_default().to_string()).to_string();
    output = upsert_section_key_line(&output, "nni", "last_heartbeat_error", &rendered_error);
    output = upsert_section_key_line(
        &output,
        "nni",
        "last_heartbeat_error_at_ts",
        &error_at_ts.unwrap_or_default().to_string(),
    );
    if let Some(error) = error.map(str::trim).filter(|value| !value.is_empty()) {
        let mut records = parse_nni_heartbeat_error_records(&parsed);
        let next_id = records
            .iter()
            .map(|record| record.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        records.insert(
            0,
            NniHeartbeatErrorRecord {
                id: next_id,
                created_at_ts: error_at_ts,
                error: error.to_string(),
                network: error_network.unwrap_or(false),
            },
        );
        records.truncate(NNI_HEARTBEAT_ERROR_HISTORY_LIMIT);
        let rendered_records = render_nni_heartbeat_error_records(&records);
        output = upsert_section_key_line(
            &output,
            "nni",
            "heartbeat_error_records",
            &rendered_records,
        );
    }
    write_runtime_config_file(state, &output)?;
    read_nni_config(state)
}

pub(crate) fn spawn_nni_heartbeat_worker(state: AppState) {
    tokio::spawn(async move {
        loop {
            if let Err(err) = nni_heartbeat_tick(&state).await {
                tracing::warn!("nni heartbeat tick failed: {err}");
            }
            tokio::time::sleep(Duration::from_secs(NNI_HEARTBEAT_POLL_SECONDS)).await;
        }
    });
}

async fn nni_heartbeat_tick(state: &AppState) -> anyhow::Result<()> {
    let config = read_nni_config(state)?;
    if !config.joined || config.remote_nodes.is_empty() {
        return Ok(());
    }
    let now = u64::try_from(current_unix_ts()).unwrap_or_default();
    if config
        .last_heartbeat_at_ts
        .is_some_and(|last| now.saturating_sub(last) < NNI_HEARTBEAT_INTERVAL_SECONDS)
    {
        return Ok(());
    }

    match run_nni_heartbeat_with_network_retries(state, &config.remote_nodes).await {
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
                Some(heartbeat_ts),
                None,
                None,
                None,
                Some(heartbeat_count),
                Some(0),
            )?;
            tracing::info!(
                "nni heartbeat accepted: ts={} count={} node={}",
                heartbeat_ts,
                heartbeat_count,
                data.get("node_url")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
            );
        }
        Err(err) => {
            let heartbeat_at_ts = err.network.then_some(now);
            let network_failures = Some(if err.network {
                u64::try_from(NNI_HEARTBEAT_NETWORK_RETRY_LIMIT).unwrap_or(3)
            } else {
                0
            });
            write_nni_heartbeat_status(
                state,
                heartbeat_at_ts,
                Some(&err.to_string()),
                Some(now),
                Some(err.network),
                None,
                network_failures,
            )?;
            return Err(err.into());
        }
    }
    Ok(())
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
                tracing::warn!(
                    "nni heartbeat network attempt {attempt}/{} failed: {err}",
                    NNI_HEARTBEAT_NETWORK_RETRY_LIMIT
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
        .unwrap_or_else(|| NniHeartbeatError::network("nni_heartbeat_network_retries_exhausted")))
}

async fn run_nni_heartbeat_once(
    state: &AppState,
    node_urls: &[String],
) -> Result<Value, NniHeartbeatError> {
    let device_pubkey = nni_device_pubkey(state)
        .await
        .map_err(|(_, error, data)| NniHeartbeatError::non_network(format!("{error}: {data}")))?;
    let mut attempts = Vec::new();
    let mut network_only = true;
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
                    network_only = false;
                }
                attempts.push(json!({
                    "node_url": node_url,
                    "network": err.network,
                    "error": err.to_string(),
                }));
            }
        }
    }
    Err(NniHeartbeatError {
        message: format!("nni_heartbeat_all_nodes_failed: {}", Value::Array(attempts)),
        network: network_only,
    })
}

async fn run_nni_heartbeat_once_for_node(
    state: &AppState,
    node_url: &str,
    device_pubkey: &str,
) -> Result<Value, NniHeartbeatError> {
    let request_endpoint = format!("{}/v1/nni/server/heartbeat/request", node_url);
    let request_resp = state
        .core
        .http_client
        .post(&request_endpoint)
        .timeout(Duration::from_secs(NNI_REMOTE_JOIN_TIMEOUT_SECONDS))
        .json(&NniRemoteHeartbeatRequest {
            device_pubkey: device_pubkey.to_string(),
            client_user_key: NNI_HEARTBEAT_USER_KEY.to_string(),
        })
        .send()
        .await
        .map_err(|err| {
            NniHeartbeatError::network(format!("heartbeat_request_network_failed: {err}"))
        })?;
    let request_status = request_resp.status();
    let request_body = request_resp
        .json::<ApiResponse<Value>>()
        .await
        .map_err(|err| {
            NniHeartbeatError::non_network(format!("heartbeat_request_body_failed: {err}"))
        })?;
    if !request_status.is_success() || !request_body.ok {
        return Err(NniHeartbeatError::non_network(format!(
            "heartbeat_request_failed: status={} error={:?} data={:?}",
            request_status, request_body.error, request_body.data
        )));
    }
    let request_data = request_body
        .data
        .ok_or_else(|| NniHeartbeatError::non_network("heartbeat_request_missing_data"))?;
    let task_id = request_data
        .get("task_id")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| NniHeartbeatError::non_network("heartbeat_task_id_missing"))?;
    let challenge = request_data
        .get("challenge")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| NniHeartbeatError::non_network("heartbeat_challenge_missing"))?;

    let sign_output = run_nni_signature_helper(state, &[String::from("sign_challenge"), challenge])
        .await
        .map_err(|err| {
            NniHeartbeatError::non_network(format!("heartbeat_signature_helper_failed: {err}"))
        })?;
    if !sign_output.ok {
        return Err(NniHeartbeatError::non_network(format!(
            "heartbeat_signature_failed: {}",
            sign_output
                .error
                .or_else(
                    || (!sign_output.stderr_tail.is_empty()).then_some(sign_output.stderr_tail),
                )
                .unwrap_or_else(|| "signature helper returned error".to_string())
        )));
    }
    let signature = sign_output
        .payload
        .get("signature")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| NniHeartbeatError::non_network("heartbeat_signature_missing"))?;

    let verify_endpoint = format!("{}/v1/nni/server/heartbeat/verify", node_url);
    let verify_resp = state
        .core
        .http_client
        .post(&verify_endpoint)
        .timeout(Duration::from_secs(NNI_REMOTE_JOIN_TIMEOUT_SECONDS))
        .json(&NniRemoteHeartbeatVerifyRequest { task_id, signature })
        .send()
        .await
        .map_err(|err| {
            NniHeartbeatError::network(format!("heartbeat_verify_network_failed: {err}"))
        })?;
    let verify_status = verify_resp.status();
    let verify_body = verify_resp
        .json::<ApiResponse<Value>>()
        .await
        .map_err(|err| {
            NniHeartbeatError::non_network(format!("heartbeat_verify_body_failed: {err}"))
        })?;
    if !verify_status.is_success() || !verify_body.ok {
        return Err(NniHeartbeatError::non_network(format!(
            "heartbeat_verify_failed: status={} error={:?} data={:?}",
            verify_status, verify_body.error, verify_body.data
        )));
    }
    verify_body
        .data
        .ok_or_else(|| NniHeartbeatError::non_network("heartbeat_verify_missing_data"))
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
