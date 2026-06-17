const NNI_REMOTE_JOIN_TIMEOUT_SECONDS: u64 = 20;

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
                    if let Some(data) = body.data.as_mut().and_then(Value::as_object_mut) {
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
        .and_then(Value::as_str)
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
    Ok(trimmed
        .strip_suffix("/v1")
        .unwrap_or(trimmed)
        .to_string())
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
