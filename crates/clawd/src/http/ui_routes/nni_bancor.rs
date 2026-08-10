use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use sha2::{Digest, Sha256};

const BASE58_ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

const NNI_BANCOR_CANDLE_INTERVALS: [u64; 8] =
    [60, 300, 900, 3_600, 14_400, 86_400, 604_800, 31_536_000];
const NNI_BANCOR_DEFAULT_SLIPPAGE_BPS: u16 = 50;
const NNI_BANCOR_MAX_SLIPPAGE_BPS: u16 = 5_000;

#[derive(Debug, Deserialize)]
struct NniBancorCandlesQuery {
    #[serde(default)]
    interval_seconds: Option<u64>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    end_time_unix: Option<i64>,
}

fn encode_base58(bytes: &[u8]) -> String {
    let mut digits = Vec::<u8>::new();
    for &byte in bytes {
        let mut carry = u32::from(byte);
        for digit in &mut digits {
            let value = u32::from(*digit) * 256 + carry;
            *digit = (value % 58) as u8;
            carry = value / 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }

    let leading_zeroes = bytes.iter().take_while(|&&byte| byte == 0).count();
    let mut encoded = String::with_capacity(leading_zeroes + digits.len());
    encoded.extend(std::iter::repeat('1').take(leading_zeroes));
    encoded.extend(
        digits
            .iter()
            .rev()
            .map(|&digit| BASE58_ALPHABET[digit as usize] as char),
    );
    encoded
}

fn decode_base58(value: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::<u8>::new();
    for character in value.bytes() {
        let digit = BASE58_ALPHABET
            .iter()
            .position(|&candidate| candidate == character)? as u32;
        let mut carry = digit;
        for byte in &mut bytes {
            let decoded = u32::from(*byte) * 58 + carry;
            *byte = (decoded & 0xff) as u8;
            carry = decoded >> 8;
        }
        while carry > 0 {
            bytes.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }

    let leading_zeroes = value.bytes().take_while(|&byte| byte == b'1').count();
    let mut decoded = vec![0; leading_zeroes];
    decoded.extend(bytes.into_iter().rev());
    Some(decoded)
}

fn compact_bancor_device_pubkey(value: &str) -> Option<String> {
    let normalized = value.trim();
    if normalized.len() == 128 && normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        let raw = hex::decode(normalized).ok()?;
        let mut compressed = Vec::with_capacity(33);
        compressed.push(if raw[63] & 1 == 0 { 0x02 } else { 0x03 });
        compressed.extend_from_slice(&raw[..32]);
        return Some(encode_base58(&compressed));
    }

    let compressed =
        decode_base58(normalized).or_else(|| URL_SAFE_NO_PAD.decode(normalized).ok())?;
    (compressed.len() == 33 && matches!(compressed[0], 0x02 | 0x03))
        .then(|| encode_base58(&compressed))
}

fn sanitize_bancor_market_trade_pubkeys(data: &mut Value) {
    let Some(trades) = data.get_mut("trades").and_then(Value::as_array_mut) else {
        return;
    };
    for trade in trades {
        let Some(record) = trade.as_object_mut() else {
            continue;
        };
        let public_key = record
            .remove("device_pubkey")
            .or_else(|| record.remove("device_public_key"))
            .or_else(|| record.remove("public_key"))
            .and_then(|value| value.as_str().map(str::to_string));
        let provided_compact = record.remove("device_pubkey_compact");
        let provided_masked = record.remove("device_pubkey_masked");
        let compact = public_key
            .as_deref()
            .and_then(compact_bancor_device_pubkey)
            .or_else(|| {
                provided_compact
                    .as_ref()
                    .and_then(Value::as_str)
                    .and_then(compact_bancor_device_pubkey)
            })
            .or_else(|| {
                provided_masked
                    .as_ref()
                    .and_then(Value::as_str)
                    .and_then(compact_bancor_device_pubkey)
            })
            .or_else(|| {
                provided_masked
                    .as_ref()
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "••••••••".to_string());
        record.insert("device_pubkey_compact".to_string(), Value::String(compact));
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct NniBancorQuoteRequest {
    side: String,
    input_amount: String,
    #[serde(default)]
    slippage_bps: Option<u16>,
}

#[derive(Debug, Deserialize, Serialize)]
struct NniBancorTradeRequest {
    side: String,
    input_amount: String,
    #[serde(default)]
    slippage_bps: Option<u16>,
    min_output: String,
}

#[derive(Debug, Serialize)]
struct NniBancorAuthenticatedRequest {
    device_pubkey: String,
    client_user_key: String,
}

#[derive(Debug, Serialize)]
struct NniBancorAccountVerifyRequest {
    task_id: String,
    signature: String,
    page: usize,
    per_page: usize,
}

#[derive(Debug, Serialize)]
struct NniBancorTradeRemoteRequest {
    device_pubkey: String,
    client_user_key: String,
    side: String,
    input_amount: String,
    slippage_bps: u16,
    min_output: String,
}

#[derive(Debug, Serialize)]
struct NniBancorTradeVerifyRequest {
    task_id: String,
    quote_id: String,
    signature: String,
}

async fn nni_bancor_market(
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
    let config = match read_nni_config(&state) {
        Ok(config) => config,
        Err(err) => {
            return nni_join_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "nni_config_read_failed",
                json!({"status": "config_read_failed", "error": err.to_string()}),
            )
        }
    };
    let mut attempts = Vec::new();
    for node_url in &config.remote_nodes {
        let endpoint = format!("{node_url}/v1/nni/server/bancor/market");
        match state
            .core
            .http_client
            .get(&endpoint)
            .timeout(Duration::from_secs(NNI_REMOTE_JOIN_TIMEOUT_SECONDS))
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status();
                match response.json::<ApiResponse<Value>>().await {
                    Ok(body) if status.is_success() && body.ok => {
                        if let Some(mut data) = body.data {
                            if let Some(object) = data.as_object_mut() {
                                object.insert("node_url".to_string(), Value::String(node_url.clone()));
                            }
                            return (StatusCode::OK, Json(ApiResponse { ok: true, data: Some(data), error: None }));
                        }
                    }
                    Ok(body) => attempts.push(json!({
                        "node_url": node_url,
                        "http_status": status.as_u16(),
                        "error_code": body.error.unwrap_or_else(|| "nni_bancor_market_failed".to_string()),
                    })),
                    Err(err) => attempts.push(json!({
                        "node_url": node_url,
                        "http_status": status.as_u16(),
                        "error_code": "nni_bancor_market_body_invalid",
                        "detail": err.to_string(),
                    })),
                }
            }
            Err(err) => attempts.push(json!({
                "node_url": node_url,
                "error_code": "nni_bancor_market_network_failed",
                "detail": err.to_string(),
            })),
        }
    }
    nni_join_error(
        StatusCode::BAD_GATEWAY,
        "nni_bancor_nodes_unavailable",
        json!({
            "status": "bancor_nodes_unavailable", "attempts": attempts,
        }),
    )
}

async fn nni_bancor_candles(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<NniBancorCandlesQuery>,
) -> axum::response::Response {
    if let Err((status, Json(resp))) = require_ui_identity(&state, &headers) {
        return (
            status,
            Json(ApiResponse::<Value> {
                ok: resp.ok,
                data: None,
                error: resp.error,
            }),
        )
            .into_response();
    }
    let (interval_seconds, limit, end_time_unix) = match normalize_bancor_candles_query(&query) {
        Ok(normalized) => normalized,
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                error,
                json!({"status": "candle_query_invalid"}),
            )
            .into_response()
        }
    };
    let config = match read_nni_config(&state) {
        Ok(config) => config,
        Err(err) => {
            return nni_join_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "nni_config_read_failed",
                json!({"status": "config_read_failed", "error": err.to_string()}),
            )
            .into_response()
        }
    };
    let mut attempts = Vec::new();
    for node_url in &config.remote_nodes {
        let mut endpoint = format!(
            "{node_url}/v1/nni/server/bancor/candles?interval_seconds={interval_seconds}&limit={limit}"
        );
        if let Some(end_time_unix) = end_time_unix {
            endpoint.push_str(&format!("&end_time_unix={end_time_unix}"));
        }
        let mut request = state
            .core
            .http_client
            .get(&endpoint)
            .timeout(Duration::from_secs(NNI_REMOTE_JOIN_TIMEOUT_SECONDS));
        if let Some(if_none_match) = headers
            .get(axum::http::header::IF_NONE_MATCH)
            .and_then(|value| value.to_str().ok())
        {
            request = request.header("if-none-match", if_none_match);
        }
        match request.send().await
        {
            Ok(response) => {
                let status = response.status();
                let etag = response
                    .headers()
                    .get("etag")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                if status == StatusCode::NOT_MODIFIED {
                    let mut downstream = StatusCode::NOT_MODIFIED.into_response();
                    downstream.headers_mut().insert(
                        axum::http::header::CACHE_CONTROL,
                        axum::http::HeaderValue::from_static("private, no-cache, must-revalidate"),
                    );
                    if let Some(etag) = etag
                        .as_deref()
                        .and_then(|value| axum::http::HeaderValue::from_str(value).ok())
                    {
                        downstream
                            .headers_mut()
                            .insert(axum::http::header::ETAG, etag);
                    }
                    return downstream;
                }
                match response.json::<ApiResponse<Value>>().await {
                    Ok(body) if status.is_success() && body.ok => {
                        if let Some(mut data) = body.data {
                            if let Some(object) = data.as_object_mut() {
                                object.insert(
                                    "node_url".to_string(),
                                    Value::String(node_url.clone()),
                                );
                            }
                            let mut downstream = (
                                StatusCode::OK,
                                Json(ApiResponse {
                                    ok: true,
                                    data: Some(data),
                                    error: None,
                                }),
                            )
                                .into_response();
                            downstream.headers_mut().insert(
                                axum::http::header::CACHE_CONTROL,
                                axum::http::HeaderValue::from_static(
                                    "private, no-cache, must-revalidate",
                                ),
                            );
                            if let Some(etag) = etag
                                .as_deref()
                                .and_then(|value| axum::http::HeaderValue::from_str(value).ok())
                            {
                                downstream
                                    .headers_mut()
                                    .insert(axum::http::header::ETAG, etag);
                            }
                            return downstream;
                        }
                    }
                    Ok(body) => attempts.push(json!({
                        "node_url": node_url,
                        "http_status": status.as_u16(),
                        "error_code": body.error.unwrap_or_else(|| "nni_bancor_candles_failed".to_string()),
                    })),
                    Err(err) => attempts.push(json!({
                        "node_url": node_url,
                        "http_status": status.as_u16(),
                        "error_code": "nni_bancor_candles_body_invalid",
                        "detail": err.to_string(),
                    })),
                }
            }
            Err(err) => attempts.push(json!({
                "node_url": node_url,
                "error_code": "nni_bancor_candles_network_failed",
                "detail": err.to_string(),
            })),
        }
    }
    nni_join_error(
        StatusCode::BAD_GATEWAY,
        "nni_bancor_candles_nodes_unavailable",
        json!({"status": "bancor_candles_nodes_unavailable", "attempts": attempts}),
    )
    .into_response()
}

async fn nni_bancor_market_trades(
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
    }
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(20).clamp(1, 100);
    let config = match read_nni_config(&state) {
        Ok(config) => config,
        Err(err) => {
            return nni_join_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "nni_config_read_failed",
                json!({"status": "config_read_failed", "error": err.to_string()}),
            )
        }
    };
    let mut attempts = Vec::new();
    for node_url in &config.remote_nodes {
        let endpoint =
            format!("{node_url}/v1/nni/server/bancor/trades?page={page}&per_page={per_page}");
        match state
            .core
            .http_client
            .get(&endpoint)
            .timeout(Duration::from_secs(NNI_REMOTE_JOIN_TIMEOUT_SECONDS))
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status();
                match response.json::<ApiResponse<Value>>().await {
                    Ok(body) if status.is_success() && body.ok => {
                        if let Some(mut data) = body.data {
                            sanitize_bancor_market_trade_pubkeys(&mut data);
                            if let Some(object) = data.as_object_mut() {
                                object.insert(
                                    "node_url".to_string(),
                                    Value::String(node_url.clone()),
                                );
                            }
                            return (
                                StatusCode::OK,
                                Json(ApiResponse {
                                    ok: true,
                                    data: Some(data),
                                    error: None,
                                }),
                            );
                        }
                    }
                    Ok(body) => attempts.push(json!({
                        "node_url": node_url,
                        "http_status": status.as_u16(),
                        "error_code": body.error.unwrap_or_else(|| "nni_bancor_market_trades_failed".to_string()),
                    })),
                    Err(err) => attempts.push(json!({
                        "node_url": node_url,
                        "http_status": status.as_u16(),
                        "error_code": "nni_bancor_market_trades_body_invalid",
                        "detail": err.to_string(),
                    })),
                }
            }
            Err(err) => attempts.push(json!({
                "node_url": node_url,
                "error_code": "nni_bancor_market_trades_network_failed",
                "detail": err.to_string(),
            })),
        }
    }
    nni_join_error(
        StatusCode::BAD_GATEWAY,
        "nni_bancor_market_trades_nodes_unavailable",
        json!({"status": "bancor_market_trades_nodes_unavailable", "attempts": attempts}),
    )
}

async fn nni_bancor_quote(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut request): Json<NniBancorQuoteRequest>,
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
    request.side = match normalize_bancor_side(&request.side) {
        Ok(side) => side.to_string(),
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                error,
                json!({"status": "quote_invalid"}),
            )
        }
    };
    request.input_amount = match normalize_bancor_amount(&request.input_amount) {
        Ok((amount, _)) => amount,
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                error,
                json!({"status": "quote_invalid"}),
            )
        }
    };
    let slippage_bps = match normalize_bancor_slippage_bps(request.slippage_bps) {
        Ok(value) => value,
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                error,
                json!({"status": "quote_invalid"}),
            )
        }
    };
    request.slippage_bps = Some(slippage_bps);
    let config = match read_nni_config(&state) {
        Ok(config) => config,
        Err(err) => {
            return nni_join_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "nni_config_read_failed",
                json!({"error": err.to_string()}),
            )
        }
    };
    let mut attempts = Vec::new();
    for node_url in &config.remote_nodes {
        let endpoint = format!("{node_url}/v1/nni/server/bancor/quote");
        match state.core.http_client.post(&endpoint)
            .timeout(Duration::from_secs(NNI_REMOTE_JOIN_TIMEOUT_SECONDS))
            .json(&request).send().await
        {
            Ok(response) => {
                let status = response.status();
                match response.json::<ApiResponse<Value>>().await {
                    Ok(body) if status.is_success() && body.ok => {
                        if let Some(mut data) = body.data {
                            if let Some(object) = data.as_object_mut() {
                                object.insert("node_url".to_string(), Value::String(node_url.clone()));
                            }
                            return (StatusCode::OK, Json(ApiResponse { ok: true, data: Some(data), error: None }));
                        }
                    }
                    Ok(body) => attempts.push(json!({"node_url": node_url, "http_status": status.as_u16(), "error_code": body.error})),
                    Err(err) => attempts.push(json!({"node_url": node_url, "error_code": "nni_bancor_quote_body_invalid", "detail": err.to_string()})),
                }
            }
            Err(err) => attempts.push(json!({"node_url": node_url, "error_code": "nni_bancor_quote_network_failed", "detail": err.to_string()})),
        }
    }
    nni_join_error(
        StatusCode::BAD_GATEWAY,
        "nni_bancor_quote_nodes_unavailable",
        json!({"attempts": attempts}),
    )
}

async fn nni_bancor_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<NniRequestRecordsQuery>,
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
            )
        }
    };
    let config = match read_nni_config(&state) {
        Ok(config) => config,
        Err(err) => {
            return nni_join_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "nni_config_read_failed",
                json!({"error": err.to_string()}),
            )
        }
    };
    let device_pubkey = match nni_device_pubkey(&state).await {
        Ok(pubkey) => pubkey,
        Err((status, error, data)) => return nni_join_error(status, error, data),
    };
    let page = query.page.unwrap_or(1).clamp(1, 1_000_000);
    let per_page = query.per_page.unwrap_or(20).clamp(1, 100);
    let mut attempts = Vec::new();
    for node_url in &config.remote_nodes {
        match query_nni_bancor_account_for_node(
            &state,
            node_url,
            &device_pubkey,
            &identity.user_key,
            page,
            per_page,
        )
        .await
        {
            Ok(mut data) => {
                if let Some(object) = data.as_object_mut() {
                    object.insert("node_url".to_string(), Value::String(node_url.clone()));
                }
                return (
                    StatusCode::OK,
                    Json(ApiResponse {
                        ok: true,
                        data: Some(data),
                        error: None,
                    }),
                );
            }
            Err(attempt) => attempts.push(attempt),
        }
    }
    nni_join_error(
        StatusCode::BAD_GATEWAY,
        "nni_bancor_account_nodes_unavailable",
        json!({"attempts": attempts}),
    )
}

async fn query_nni_bancor_account_for_node(
    state: &AppState,
    node_url: &str,
    device_pubkey: &str,
    user_key: &str,
    page: usize,
    per_page: usize,
) -> Result<Value, Value> {
    let request_response = state.core.http_client
        .post(format!("{node_url}/v1/nni/server/bancor/account/request"))
        .timeout(Duration::from_secs(NNI_REMOTE_JOIN_TIMEOUT_SECONDS))
        .json(&NniBancorAuthenticatedRequest {
            device_pubkey: device_pubkey.to_string(),
            client_user_key: user_key.to_string(),
        }).send().await
        .map_err(|err| json!({"node_url": node_url, "error_code": "nni_bancor_account_request_network_failed", "detail": err.to_string()}))?;
    let status = request_response.status();
    let body = request_response.json::<ApiResponse<Value>>().await
        .map_err(|err| json!({"node_url": node_url, "error_code": "nni_bancor_account_request_body_invalid", "detail": err.to_string()}))?;
    if !status.is_success() || !body.ok {
        return Err(
            json!({"node_url": node_url, "http_status": status.as_u16(), "error_code": body.error}),
        );
    }
    let data = body.data.ok_or_else(
        || json!({"node_url": node_url, "error_code": "nni_bancor_account_request_data_missing"}),
    )?;
    let task_id = value_string(&data, "task_id", "nni_bancor_account_task_id_missing")?;
    let challenge = value_string(&data, "challenge", "nni_bancor_account_challenge_missing")?;
    let sign_output = run_nni_signature_helper(state, &["sign_challenge".to_string(), challenge]).await
        .map_err(|_| json!({"node_url": node_url, "error_code": "nni_bancor_account_signature_helper_failed"}))?;
    if !sign_output.ok {
        return Err(
            json!({"node_url": node_url, "error_code": "nni_bancor_account_signature_failed"}),
        );
    }
    let signature = value_string(
        &sign_output.payload,
        "signature",
        "nni_bancor_account_signature_missing",
    )?;
    let response = state.core.http_client
        .post(format!("{node_url}/v1/nni/server/bancor/account/verify"))
        .timeout(Duration::from_secs(NNI_REMOTE_JOIN_TIMEOUT_SECONDS))
        .json(&NniBancorAccountVerifyRequest { task_id, signature, page, per_page })
        .send().await
        .map_err(|err| json!({"node_url": node_url, "error_code": "nni_bancor_account_verify_network_failed", "detail": err.to_string()}))?;
    let status = response.status();
    let body = response.json::<ApiResponse<Value>>().await
        .map_err(|err| json!({"node_url": node_url, "error_code": "nni_bancor_account_verify_body_invalid", "detail": err.to_string()}))?;
    if !status.is_success() || !body.ok {
        return Err(
            json!({"node_url": node_url, "http_status": status.as_u16(), "error_code": body.error}),
        );
    }
    body.data.ok_or_else(
        || json!({"node_url": node_url, "error_code": "nni_bancor_account_data_missing"}),
    )
}

async fn nni_bancor_trade(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut request): Json<NniBancorTradeRequest>,
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
            )
        }
    };
    request.side = match normalize_bancor_side(&request.side) {
        Ok(side) => side.to_string(),
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                error,
                json!({"status": "trade_invalid"}),
            )
        }
    };
    let (normalized_input_amount, expected_input_units) =
        match normalize_bancor_amount(&request.input_amount) {
            Ok(value) => value,
            Err(error) => {
                return nni_join_error(
                    StatusCode::BAD_REQUEST,
                    error,
                    json!({"status": "trade_invalid"}),
                )
            }
        };
    request.input_amount = normalized_input_amount;
    let (normalized_min_output, expected_min_output_units) =
        match normalize_bancor_amount(&request.min_output) {
            Ok(value) => value,
            Err(error) => {
                return nni_join_error(
                    StatusCode::BAD_REQUEST,
                    error,
                    json!({"status": "trade_invalid"}),
                )
            }
        };
    request.min_output = normalized_min_output;
    let slippage_bps = match normalize_bancor_slippage_bps(request.slippage_bps) {
        Ok(value) => value,
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                error,
                json!({"status": "trade_invalid"}),
            )
        }
    };
    let config = match read_nni_config(&state) {
        Ok(config) => config,
        Err(err) => {
            return nni_join_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "nni_config_read_failed",
                json!({"error": err.to_string()}),
            )
        }
    };
    let device_pubkey = match nni_device_pubkey(&state).await {
        Ok(pubkey) => pubkey,
        Err((status, error, data)) => return nni_join_error(status, error, data),
    };
    let mut attempts = Vec::new();
    for node_url in &config.remote_nodes {
        let remote_request = NniBancorTradeRemoteRequest {
            device_pubkey: device_pubkey.clone(),
            client_user_key: identity.user_key.clone(),
            side: request.side.clone(),
            input_amount: request.input_amount.clone(),
            slippage_bps,
            min_output: request.min_output.clone(),
        };
        match execute_nni_bancor_trade_for_node(
            &state,
            node_url,
            &device_pubkey,
            &request.side,
            &expected_input_units,
            &expected_min_output_units,
            &remote_request,
        )
        .await
        {
            Ok(mut data) => {
                if let Some(object) = data.as_object_mut() {
                    object.insert("node_url".to_string(), Value::String(node_url.clone()));
                }
                return (
                    StatusCode::OK,
                    Json(ApiResponse {
                        ok: true,
                        data: Some(data),
                        error: None,
                    }),
                );
            }
            Err(attempt) => {
                let terminal = nni_bancor_attempt_is_terminal(&attempt);
                attempts.push(attempt);
                if terminal {
                    break;
                }
            }
        }
    }
    nni_join_error(
        StatusCode::BAD_GATEWAY,
        "nni_bancor_trade_nodes_unavailable",
        json!({"attempts": attempts}),
    )
}

async fn execute_nni_bancor_trade_for_node(
    state: &AppState,
    node_url: &str,
    device_pubkey: &str,
    side: &str,
    expected_input_units: &str,
    expected_min_output_units: &str,
    request: &NniBancorTradeRemoteRequest,
) -> Result<Value, Value> {
    let response = state.core.http_client
        .post(format!("{node_url}/v1/nni/server/bancor/trade/request"))
        .timeout(Duration::from_secs(NNI_REMOTE_JOIN_TIMEOUT_SECONDS))
        .json(request).send().await
        .map_err(|err| json!({"node_url": node_url, "error_code": "nni_bancor_trade_request_network_failed", "detail": err.to_string()}))?;
    let status = response.status();
    let body = response.json::<ApiResponse<Value>>().await
        .map_err(|err| json!({"node_url": node_url, "error_code": "nni_bancor_trade_request_body_invalid", "detail": err.to_string()}))?;
    if !status.is_success() || !body.ok {
        return Err(
            json!({"node_url": node_url, "http_status": status.as_u16(), "error_code": body.error}),
        );
    }
    let data = body.data.ok_or_else(
        || json!({"node_url": node_url, "error_code": "nni_bancor_trade_request_data_missing"}),
    )?;
    let validated = validate_bancor_signing_payload(
        &data,
        device_pubkey,
        side,
        expected_input_units,
        expected_min_output_units,
    )
    .map_err(|error| json!({"node_url": node_url, "error_code": error}))?;
    let sign_output = run_nni_signature_helper(
        state,
        &[
            "sign_challenge".to_string(),
            validated.signing_payload.clone(),
        ],
    )
    .await
    .map_err(
        |_| json!({"node_url": node_url, "error_code": "nni_bancor_trade_signature_helper_failed"}),
    )?;
    if !sign_output.ok {
        return Err(
            json!({"node_url": node_url, "error_code": "nni_bancor_trade_signature_failed"}),
        );
    }
    let signature = value_string(
        &sign_output.payload,
        "signature",
        "nni_bancor_trade_signature_missing",
    )?;
    let response = state
        .core
        .http_client
        .post(format!("{node_url}/v1/nni/server/bancor/trade/verify"))
        .timeout(Duration::from_secs(NNI_REMOTE_JOIN_TIMEOUT_SECONDS))
        .json(&NniBancorTradeVerifyRequest {
            task_id: validated.task_id,
            quote_id: validated.quote_id,
            signature,
        })
        .send()
        .await
        .map_err(|err| {
            json!({
                "node_url": node_url,
                "error_code": "nni_bancor_trade_outcome_unknown",
                "detail": err.to_string(),
                "terminal": true,
            })
        })?;
    let status = response.status();
    let body = response.json::<ApiResponse<Value>>().await.map_err(|err| {
        json!({
            "node_url": node_url,
            "error_code": "nni_bancor_trade_outcome_unknown",
            "detail": err.to_string(),
            "terminal": true,
        })
    })?;
    if !status.is_success() || !body.ok {
        return Err(json!({
            "node_url": node_url,
            "http_status": status.as_u16(),
            "error_code": body.error,
            "terminal": true,
        }));
    }
    body.data.ok_or_else(|| {
        json!({
            "node_url": node_url,
            "error_code": "nni_bancor_trade_outcome_unknown",
            "terminal": true,
        })
    })
}

fn nni_bancor_attempt_is_terminal(attempt: &Value) -> bool {
    attempt
        .get("terminal")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

#[derive(Debug)]
struct ValidatedBancorSigningPayload {
    signing_payload: String,
    task_id: String,
    quote_id: String,
}

fn validate_bancor_signing_payload(
    response: &Value,
    expected_pubkey: &str,
    expected_side: &str,
    expected_input_units: &str,
    expected_min_output_units: &str,
) -> Result<ValidatedBancorSigningPayload, &'static str> {
    let signing_payload = response
        .get("signing_payload")
        .and_then(Value::as_str)
        .ok_or("nni_bancor_signing_payload_missing")?;
    if signing_payload.len() > 4096 {
        return Err("nni_bancor_signing_payload_too_large");
    }
    let payload: Value =
        serde_json::from_str(signing_payload).map_err(|_| "nni_bancor_signing_payload_invalid")?;
    let object = payload
        .as_object()
        .ok_or("nni_bancor_signing_payload_invalid")?;
    let expected_keys = BTreeSet::from([
        "schema_version",
        "action",
        "server_identity",
        "market_id",
        "market_version",
        "quote_id",
        "task_id",
        "device_pubkey",
        "side",
        "input_units",
        "fee_units",
        "fee_bps",
        "output_units",
        "min_output_units",
        "nonce",
        "expires_at_unix",
    ]);
    let actual_keys = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    if actual_keys != expected_keys {
        return Err("nni_bancor_signing_payload_fields_invalid");
    }
    if payload.get("schema_version").and_then(Value::as_u64) != Some(1)
        || payload.get("action").and_then(Value::as_str) != Some("nni_bancor_trade")
        || payload.get("server_identity").and_then(Value::as_str) != Some("nni-server-v1")
        || payload.get("device_pubkey").and_then(Value::as_str) != Some(expected_pubkey)
        || payload.get("side").and_then(Value::as_str) != Some(expected_side)
        || payload.get("input_units").and_then(Value::as_str) != Some(expected_input_units)
        || payload.get("min_output_units").and_then(Value::as_str)
            != Some(expected_min_output_units)
    {
        return Err("nni_bancor_signing_payload_binding_invalid");
    }
    for field in [
        "market_id",
        "market_version",
        "quote_id",
        "task_id",
        "input_units",
        "fee_bps",
        "fee_units",
        "output_units",
        "min_output_units",
        "expires_at_unix",
    ] {
        if payload.get(field) != response.get(field) {
            return Err("nni_bancor_signing_payload_response_mismatch");
        }
    }
    let nonce = payload
        .get("nonce")
        .and_then(Value::as_str)
        .ok_or("nni_bancor_signing_payload_nonce_invalid")?;
    if nonce.len() != 32 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("nni_bancor_signing_payload_nonce_invalid");
    }
    let expires = payload
        .get("expires_at_unix")
        .and_then(Value::as_i64)
        .ok_or("nni_bancor_signing_payload_expiry_invalid")?;
    let now = current_unix_ts();
    if expires < now || expires > now.saturating_add(600) {
        return Err("nni_bancor_signing_payload_expiry_invalid");
    }
    let digest = format!("{:x}", Sha256::digest(signing_payload.as_bytes()));
    if response
        .get("signing_payload_digest")
        .and_then(Value::as_str)
        != Some(digest.as_str())
    {
        return Err("nni_bancor_signing_payload_digest_invalid");
    }
    let task_id = payload
        .get("task_id")
        .and_then(Value::as_str)
        .ok_or("nni_bancor_trade_task_id_missing")?
        .to_string();
    let quote_id = payload
        .get("quote_id")
        .and_then(Value::as_str)
        .ok_or("nni_bancor_trade_quote_id_missing")?
        .to_string();
    Ok(ValidatedBancorSigningPayload {
        signing_payload: signing_payload.to_string(),
        task_id,
        quote_id,
    })
}

fn normalize_bancor_side(value: &str) -> Result<&'static str, &'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "buy" => Ok("buy"),
        "sell" => Ok("sell"),
        _ => Err("nni_bancor_side_invalid"),
    }
}

fn normalize_bancor_candles_query(
    query: &NniBancorCandlesQuery,
) -> Result<(u64, usize, Option<i64>), &'static str> {
    let interval_seconds = query.interval_seconds.unwrap_or(300);
    if !NNI_BANCOR_CANDLE_INTERVALS.contains(&interval_seconds) {
        return Err("nni_bancor_candle_interval_invalid");
    }
    let limit = query.limit.unwrap_or(120);
    if !(1..=300).contains(&limit) {
        return Err("nni_bancor_candle_limit_invalid");
    }
    if query.end_time_unix.is_some_and(|value| value < 0) {
        return Err("nni_bancor_candle_end_time_invalid");
    }
    Ok((interval_seconds, limit, query.end_time_unix))
}

fn normalize_bancor_amount(value: &str) -> Result<(String, String), &'static str> {
    let value = value.trim();
    let (whole, fraction) = match value.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (value, ""),
    };
    if whole.is_empty()
        || whole.len() > 18
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || (whole.len() > 1 && whole.starts_with('0'))
        || fraction.len() > 4
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("nni_bancor_amount_invalid");
    }
    let whole_value = whole
        .parse::<u128>()
        .map_err(|_| "nni_bancor_amount_invalid")?;
    let fraction_padded = format!("{fraction:0<4}");
    let fraction_value = if fraction_padded.is_empty() {
        0
    } else {
        fraction_padded
            .parse::<u128>()
            .map_err(|_| "nni_bancor_amount_invalid")?
    };
    let units = whole_value
        .checked_mul(10_000)
        .and_then(|value| value.checked_add(fraction_value))
        .ok_or("nni_bancor_amount_invalid")?;
    if units == 0 || units > i64::MAX as u128 {
        return Err("nni_bancor_amount_invalid");
    }
    Ok((
        format!("{whole_value}.{:04}", fraction_value),
        units.to_string(),
    ))
}

fn normalize_bancor_slippage_bps(value: Option<u16>) -> Result<u16, &'static str> {
    let value = value.unwrap_or(NNI_BANCOR_DEFAULT_SLIPPAGE_BPS);
    (value <= NNI_BANCOR_MAX_SLIPPAGE_BPS)
        .then_some(value)
        .ok_or("nni_bancor_slippage_bps_invalid")
}

fn value_string(value: &Value, field: &str, error: &'static str) -> Result<String, Value> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| json!({"error_code": error}))
}

#[cfg(test)]
mod nni_bancor_unit_tests {
    use super::*;

    #[test]
    fn amount_normalization_is_four_decimal_and_rejects_float_syntax() {
        assert_eq!(
            normalize_bancor_amount("100").unwrap(),
            ("100.0000".to_string(), "1000000".to_string())
        );
        assert_eq!(normalize_bancor_amount("0.0001").unwrap().1, "1");
        assert_eq!(
            normalize_bancor_amount("922337203685477.5807").unwrap().1,
            i64::MAX.to_string()
        );
        assert!(normalize_bancor_amount("0").is_err());
        assert!(normalize_bancor_amount("0.0000").is_err());
        assert!(normalize_bancor_amount("922337203685477.5808").is_err());
        assert!(normalize_bancor_amount("1e2").is_err());
        assert!(normalize_bancor_amount("1.00001").is_err());
    }

    #[test]
    fn slippage_allows_explicit_large_trade_protection_up_to_fifty_percent() {
        assert_eq!(normalize_bancor_slippage_bps(None), Ok(50));
        assert_eq!(normalize_bancor_slippage_bps(Some(0)), Ok(0));
        assert_eq!(normalize_bancor_slippage_bps(Some(5_000)), Ok(5_000));
        assert_eq!(
            normalize_bancor_slippage_bps(Some(5_001)),
            Err("nni_bancor_slippage_bps_invalid")
        );
    }

    #[test]
    fn candle_query_accepts_only_supported_intervals_and_bounded_limits() {
        let query = NniBancorCandlesQuery {
            interval_seconds: None,
            limit: None,
            end_time_unix: None,
        };
        assert_eq!(normalize_bancor_candles_query(&query), Ok((300, 120, None)));
        for interval_seconds in [604_800, 31_536_000] {
            let supported = NniBancorCandlesQuery {
                interval_seconds: Some(interval_seconds),
                limit: Some(300),
                end_time_unix: None,
            };
            assert_eq!(
                normalize_bancor_candles_query(&supported),
                Ok((interval_seconds, 300, None))
            );
        }
        let invalid = NniBancorCandlesQuery {
            interval_seconds: Some(61),
            limit: Some(301),
            end_time_unix: None,
        };
        assert_eq!(
            normalize_bancor_candles_query(&invalid),
            Err("nni_bancor_candle_interval_invalid")
        );
    }

    #[test]
    fn device_signing_is_restricted_to_the_bound_bancor_contract() {
        let expires_at = current_unix_ts() + 120;
        let payload = json!({
            "schema_version": 1,
            "action": "nni_bancor_trade",
            "server_identity": "nni-server-v1",
            "market_id": "point-usd-v1",
            "market_version": 7,
            "quote_id": "quote-1",
            "task_id": "task-1",
            "device_pubkey": "aa".repeat(64),
            "side": "sell",
            "input_units": "10000",
            "fee_bps": 0,
            "fee_units": "0",
            "output_units": "1",
            "min_output_units": "1",
            "nonce": "11".repeat(16),
            "expires_at_unix": expires_at,
        });
        let signing_payload = serde_json::to_string(&payload).unwrap();
        let digest = format!("{:x}", Sha256::digest(signing_payload.as_bytes()));
        let response = json!({
            "signing_payload": signing_payload,
            "signing_payload_digest": digest,
            "market_id": "point-usd-v1",
            "market_version": 7,
            "quote_id": "quote-1",
            "task_id": "task-1",
            "input_units": "10000",
            "fee_bps": 0,
            "fee_units": "0",
            "output_units": "1",
            "min_output_units": "1",
            "expires_at_unix": expires_at,
        });
        assert!(
            validate_bancor_signing_payload(&response, &"aa".repeat(64), "sell", "10000", "1",)
                .is_ok()
        );
        assert_eq!(
            validate_bancor_signing_payload(&response, &"aa".repeat(64), "sell", "20000", "1",)
                .unwrap_err(),
            "nni_bancor_signing_payload_binding_invalid",
        );

        let mut arbitrary = payload;
        arbitrary.as_object_mut().unwrap().insert(
            "arbitrary_command".to_string(),
            Value::String("sign me".to_string()),
        );
        let arbitrary_payload = serde_json::to_string(&arbitrary).unwrap();
        let mut arbitrary_response = response;
        arbitrary_response["signing_payload"] = Value::String(arbitrary_payload.clone());
        arbitrary_response["signing_payload_digest"] = Value::String(format!(
            "{:x}",
            Sha256::digest(arbitrary_payload.as_bytes()),
        ));
        assert_eq!(
            validate_bancor_signing_payload(
                &arbitrary_response,
                &"aa".repeat(64),
                "sell",
                "10000",
                "1",
            )
            .unwrap_err(),
            "nni_bancor_signing_payload_fields_invalid",
        );
    }

    #[test]
    fn ambiguous_verify_result_stops_multi_node_trade_fallback() {
        assert!(nni_bancor_attempt_is_terminal(&json!({
            "error_code": "nni_bancor_trade_outcome_unknown",
            "terminal": true,
        })));
        assert!(!nni_bancor_attempt_is_terminal(&json!({
            "error_code": "nni_bancor_trade_request_network_failed",
        })));
    }
}
