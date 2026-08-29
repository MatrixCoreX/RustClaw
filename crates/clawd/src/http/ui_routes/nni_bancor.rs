use sha2::{Digest, Sha256};

const NNI_BANCOR_CANDLE_INTERVALS: [u64; 8] =
    [60, 300, 900, 3_600, 14_400, 86_400, 604_800, 31_536_000];
const NNI_BANCOR_DEFAULT_SLIPPAGE_BPS: u16 = 300;
const NNI_BANCOR_MAX_SLIPPAGE_BPS: u16 = 5_000;
const NNI_BANCOR_MARKET_TRADE_LIMIT: usize = 100;
const NNI_BANCOR_CANDLE_PRICE_KIND: &str = "execution_average_usd_per_aic";
const NNI_BANCOR_DAILY_PRICE_KIND: &str = "pool_marginal_usd_per_aic";

#[derive(Debug, Deserialize)]
struct NniBancorCandlesQuery {
    #[serde(default)]
    interval_seconds: Option<u64>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    end_time_unix: Option<i64>,
}

fn validate_bancor_market_response(data: &Value) -> Result<(), &'static str> {
    let object = data
        .as_object()
        .ok_or("nni_bancor_market_contract_invalid")?;
    let status_is_valid = object
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|value| matches!(value, "open" | "disabled" | "paused"));
    if object.get("schema_version").and_then(Value::as_u64) != Some(1)
        || !status_is_valid
        || object
            .get("market_id")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        || object.get("aic_symbol").and_then(Value::as_str) != Some("AIC")
        || object.get("usd_symbol").and_then(Value::as_str) != Some("USD")
        || object.get("aic_scale").and_then(Value::as_u64) != Some(100_000_000)
        || object.get("usd_scale").and_then(Value::as_u64) != Some(100_000_000)
        || object.get("fee_bps").and_then(Value::as_u64).is_none()
        || object.get("version").and_then(Value::as_u64).is_none()
        || object.get("updated_at_unix").and_then(Value::as_i64).is_none()
    {
        return Err("nni_bancor_market_contract_invalid");
    }
    let current = positive_decimal(object, "marginal_price_usd_per_aic")?;
    let daily = object
        .get("daily_marginal_price")
        .and_then(Value::as_object)
        .ok_or("nni_bancor_market_contract_invalid")?;
    if daily.get("price_kind").and_then(Value::as_str) != Some(NNI_BANCOR_DAILY_PRICE_KIND)
        || daily.get("timezone").and_then(Value::as_str) != Some("UTC")
        || daily
            .get("day_start_unix")
            .and_then(Value::as_i64)
            .is_none_or(|value| value < 0 || value % 86_400 != 0)
        || daily.get("trade_count").and_then(Value::as_u64).is_none()
    {
        return Err("nni_bancor_market_contract_invalid");
    }
    let open = positive_decimal(daily, "open_usd_per_aic")?;
    let high = positive_decimal(daily, "high_usd_per_aic")?;
    let low = positive_decimal(daily, "low_usd_per_aic")?;
    let change = finite_decimal(daily, "change_percent")?;
    if high < open.max(current) || low > open.min(current) || low > high {
        return Err("nni_bancor_market_contract_invalid");
    }
    // The server derives change from exact reserve fractions while the public
    // price fields are intentionally truncated to eight decimals, so the
    // percentage cannot be reconstructed reliably from display strings here.
    if change <= -100.0 {
        return Err("nni_bancor_market_contract_invalid");
    }
    Ok(())
}

fn positive_decimal(
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<f64, &'static str> {
    finite_decimal(object, field).and_then(|value| {
        (value > 0.0)
            .then_some(value)
            .ok_or("nni_bancor_market_contract_invalid")
    })
}

fn finite_decimal(
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<f64, &'static str> {
    object
        .get(field)
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .ok_or("nni_bancor_market_contract_invalid")
}

fn validate_bancor_candles_response(
    data: &Value,
    expected_interval_seconds: u64,
    expected_limit: usize,
) -> Result<(), &'static str> {
    let object = data
        .as_object()
        .ok_or("nni_bancor_candles_contract_invalid")?;
    if object.get("schema_version").and_then(Value::as_u64) != Some(1)
        || object.get("status").and_then(Value::as_str) != Some("bancor_candles")
        || object
            .get("market_id")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        || object
            .get("market_version")
            .and_then(Value::as_u64)
            .is_none()
        || object
            .get("market_created_at_unix")
            .and_then(Value::as_i64)
            .is_none_or(|value| value < 0)
        || object.get("price_kind").and_then(Value::as_str) != Some(NNI_BANCOR_CANDLE_PRICE_KIND)
        || object.get("interval_seconds").and_then(Value::as_u64) != Some(expected_interval_seconds)
        || object.get("price_scale").and_then(Value::as_u64) != Some(1_000_000_000_000)
        || object.get("price_decimal_places").and_then(Value::as_u64) != Some(12)
    {
        return Err("nni_bancor_candles_contract_invalid");
    }
    let range_start = object
        .get("start_time_unix")
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0)
        .ok_or("nni_bancor_candles_contract_invalid")?;
    let range_end = object
        .get("end_time_unix")
        .and_then(Value::as_i64)
        .filter(|value| *value >= range_start)
        .ok_or("nni_bancor_candles_contract_invalid")?;
    let candles = object
        .get("candles")
        .and_then(Value::as_array)
        .ok_or("nni_bancor_candles_contract_invalid")?;
    if candles.len() > expected_limit {
        return Err("nni_bancor_candles_contract_invalid");
    }
    let interval_seconds = i64::try_from(expected_interval_seconds)
        .map_err(|_| "nni_bancor_candles_contract_invalid")?;
    let mut previous_end = None;
    for candle in candles {
        let candle = candle
            .as_object()
            .ok_or("nni_bancor_candles_contract_invalid")?;
        let bucket_start = candle
            .get("bucket_start_unix")
            .and_then(Value::as_i64)
            .filter(|value| *value >= range_start)
            .ok_or("nni_bancor_candles_contract_invalid")?;
        let bucket_end = candle
            .get("bucket_end_unix")
            .and_then(Value::as_i64)
            .filter(|value| *value > bucket_start && *value <= range_end)
            .ok_or("nni_bancor_candles_contract_invalid")?;
        let bucket_span = bucket_end - bucket_start;
        let span_is_valid = if expected_interval_seconds == 31_536_000 {
            (31_536_000..=31_622_400).contains(&bucket_span)
        } else {
            bucket_span == interval_seconds
        };
        if !span_is_valid || previous_end.is_some_and(|value| bucket_start < value) {
            return Err("nni_bancor_candles_contract_invalid");
        }
        previous_end = Some(bucket_end);

        let mut prices = [0.0_f64; 4];
        for (index, field) in ["open", "high", "low", "close"].iter().enumerate() {
            prices[index] = candle
                .get(*field)
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|value| value.is_finite() && *value > 0.0)
                .ok_or("nni_bancor_candles_contract_invalid")?;
        }
        let [open, high, low, close] = prices;
        if high < open.max(close) || low > open.min(close) || low > high {
            return Err("nni_bancor_candles_contract_invalid");
        }
        for field in ["aic_volume_units", "usd_volume_units"] {
            if candle
                .get(field)
                .and_then(Value::as_str)
                .is_none_or(|value| {
                    value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit())
                })
            {
                return Err("nni_bancor_candles_contract_invalid");
            }
        }
        for field in ["aic_volume", "usd_volume"] {
            if candle
                .get(field)
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<f64>().ok())
                .is_none_or(|value| !value.is_finite() || value < 0.0)
            {
                return Err("nni_bancor_candles_contract_invalid");
            }
        }
        let trade_count = candle
            .get("trade_count")
            .and_then(Value::as_u64)
            .ok_or("nni_bancor_candles_contract_invalid")?;
        let has_trades = candle
            .get("has_trades")
            .and_then(Value::as_bool)
            .ok_or("nni_bancor_candles_contract_invalid")?;
        if has_trades != (trade_count > 0) {
            return Err("nni_bancor_candles_contract_invalid");
        }
    }
    Ok(())
}

fn normalize_bancor_market_trades(data: &mut Value) {
    let Some(object) = data.as_object_mut() else {
        return;
    };
    object.remove("page");
    object.remove("per_page");
    object.remove("total");
    object.remove("total_pages");
    object.insert(
        "limit".to_string(),
        Value::Number(NNI_BANCOR_MARKET_TRADE_LIMIT.into()),
    );
    let Some(trades) = object.get_mut("trades").and_then(Value::as_array_mut) else {
        return;
    };
    trades.retain_mut(|trade| {
        let Some(record) = trade.as_object_mut() else {
            return false;
        };
        record.remove("device_pubkey");
        record.remove("device_public_key");
        record.remove("public_key");
        record.remove("device_pubkey_compact");
        record.remove("device_pubkey_masked");
        record.remove("asset_owner_pubkey_masked");
        let owner = record
            .remove("asset_owner_pubkey")
            .and_then(|value| value.as_str().map(str::to_string))
            .and_then(|value| normalize_nni_owner_public_key(&value).ok());
        if let Some(owner) = owner {
            record.insert("asset_owner_pubkey".to_string(), Value::String(owner));
            true
        } else {
            false
        }
    });
    trades.truncate(NNI_BANCOR_MARKET_TRADE_LIMIT);
}

#[derive(Debug, Deserialize, Serialize)]
struct NniBancorQuoteRequest {
    side: String,
    input_amount: String,
    #[serde(default)]
    slippage_bps: Option<u16>,
}

#[derive(Deserialize)]
struct NniBancorTradeRequest {
    side: String,
    input_amount: String,
    #[serde(default)]
    slippage_bps: Option<u16>,
    min_output: String,
    #[serde(default)]
    authorization_mode: Option<String>,
    #[serde(default)]
    owner_private_key: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    device_pubkey: Option<String>,
    asset_owner_pubkey: String,
    authorization_mode: String,
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

#[derive(Clone, Copy)]
enum NniFinancialNodeScope {
    Bancor,
    Assets,
}

fn nni_financial_remote_nodes(
    config: &NniConfigResponse,
    scope: NniFinancialNodeScope,
) -> Vec<&String> {
    match scope {
        NniFinancialNodeScope::Bancor => nni_bancor_service_remote_nodes(config),
        NniFinancialNodeScope::Assets => nni_asset_service_remote_nodes(config),
    }
}

async fn nni_bancor_market(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    nni_financial_market(state, headers, NniFinancialNodeScope::Bancor).await
}

async fn nni_assets_market(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    nni_financial_market(state, headers, NniFinancialNodeScope::Assets).await
}

async fn nni_financial_market(
    state: AppState,
    headers: HeaderMap,
    scope: NniFinancialNodeScope,
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
    for node_url in nni_financial_remote_nodes(&config, scope) {
        let endpoint = nni_remote_api_endpoint(node_url, "bancor/market");
        match state
            .core
            .public_http_client
            .get(&endpoint)
            .timeout(nni_remote_api_timeout())
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status();
                match response.json::<ApiResponse<Value>>().await {
                    Ok(body) if status.is_success() && body.ok => {
                        if let Some(mut data) = body.data {
                            if let Err(error_code) = validate_bancor_market_response(&data) {
                                attempts.push(json!({
                                    "node_url": node_url,
                                    "http_status": status.as_u16(),
                                    "error_code": error_code,
                                }));
                                continue;
                            }
                            if let Some(object) = data.as_object_mut() {
                                object.insert("node_url".to_string(), Value::String(node_url.clone()));
                            }
                            return (StatusCode::OK, Json(ApiResponse { ok: true, data: Some(data), error: None }));
                        }
                    }
                    Ok(body) => {
                        let error_code =
                            nni_remote_api_error_code(&body, "nni_bancor_market_failed");
                        attempts.push(json!({
                            "node_url": node_url,
                            "http_status": status.as_u16(),
                            "error_code": error_code,
                        }));
                    }
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
    for node_url in nni_bancor_service_remote_nodes(&config) {
        let mut endpoint = nni_remote_api_endpoint(
            node_url,
            &format!("bancor/candles?interval_seconds={interval_seconds}&limit={limit}"),
        );
        if let Some(end_time_unix) = end_time_unix {
            endpoint.push_str(&format!("&end_time_unix={end_time_unix}"));
        }
        let mut request = state
            .core
            .public_http_client
            .get(&endpoint)
            .timeout(nni_remote_api_timeout());
        if let Some(if_none_match) = headers
            .get(axum::http::header::IF_NONE_MATCH)
            .and_then(|value| value.to_str().ok())
        {
            request = request.header("if-none-match", if_none_match);
        }
        match request.send().await {
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
                            if let Err(error_code) =
                                validate_bancor_candles_response(&data, interval_seconds, limit)
                            {
                                attempts.push(json!({
                                    "node_url": node_url,
                                    "http_status": status.as_u16(),
                                    "error_code": error_code,
                                }));
                                continue;
                            }
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
                    Ok(body) => {
                        let error_code =
                            nni_remote_api_error_code(&body, "nni_bancor_candles_failed");
                        attempts.push(json!({
                            "node_url": node_url,
                            "http_status": status.as_u16(),
                            "error_code": error_code,
                        }));
                    }
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
    for node_url in nni_bancor_service_remote_nodes(&config) {
        let endpoint = nni_remote_api_endpoint(node_url, "bancor/trades");
        match state
            .core
            .public_http_client
            .get(&endpoint)
            .timeout(nni_remote_api_timeout())
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status();
                match response.json::<ApiResponse<Value>>().await {
                    Ok(body) if status.is_success() && body.ok => {
                        if let Some(mut data) = body.data {
                            normalize_bancor_market_trades(&mut data);
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
                    Ok(body) => {
                        let error_code = nni_remote_api_error_code(
                            &body,
                            "nni_bancor_market_trades_failed",
                        );
                        attempts.push(json!({
                            "node_url": node_url,
                            "http_status": status.as_u16(),
                            "error_code": error_code,
                        }));
                    }
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
    for node_url in nni_bancor_service_remote_nodes(&config) {
        let endpoint = nni_remote_api_endpoint(node_url, "bancor/quote");
        match state.core.public_http_client.post(&endpoint)
            .timeout(nni_remote_api_timeout())
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
                    Ok(body) => {
                        let error_code =
                            nni_remote_api_error_code(&body, "nni_bancor_quote_failed");
                        attempts.push(json!({
                            "node_url": node_url,
                            "http_status": status.as_u16(),
                            "error_code": error_code,
                        }));
                    }
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
    nni_financial_account(state, headers, query, NniFinancialNodeScope::Bancor).await
}

async fn nni_assets_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<NniRequestRecordsQuery>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    nni_financial_account(state, headers, query, NniFinancialNodeScope::Assets).await
}

async fn nni_financial_account(
    state: AppState,
    headers: HeaderMap,
    query: NniRequestRecordsQuery,
    scope: NniFinancialNodeScope,
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
    for node_url in nni_financial_remote_nodes(&config, scope) {
        match nni_remote_read_with_retry(|| {
            query_nni_bancor_account_for_node(
                &state,
                node_url,
                &device_pubkey,
                &identity.user_key,
                page,
                per_page,
            )
        })
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
    let request_response = state.core.public_http_client
        .post(nni_remote_api_endpoint(node_url, "bancor/account/request"))
        .timeout(nni_remote_api_timeout())
        .json(&NniBancorAuthenticatedRequest {
            device_pubkey: device_pubkey.to_string(),
            client_user_key: user_key.to_string(),
        }).send().await
        .map_err(|err| json!({"node_url": node_url, "error_code": "nni_bancor_account_request_network_failed", "detail": err.to_string(), "retryable": true}))?;
    let status = request_response.status();
    let body = request_response.json::<ApiResponse<Value>>().await
        .map_err(|err| json!({"node_url": node_url, "error_code": "nni_bancor_account_request_body_invalid", "detail": err.to_string(), "retryable": true}))?;
    if !status.is_success() || !body.ok {
        let error_code =
            nni_remote_api_error_code(&body, "nni_bancor_account_request_failed");
        return Err(
            json!({"node_url": node_url, "http_status": status.as_u16(), "error_code": error_code, "retryable": nni_remote_http_status_retryable(status.as_u16())}),
        );
    }
    let data = body.data.ok_or_else(
        || json!({"node_url": node_url, "error_code": "nni_bancor_account_request_data_missing"}),
    )?;
    let task_id = value_string(&data, "task_id", "nni_bancor_account_task_id_missing")?;
    let challenge = value_string(&data, "challenge", "nni_bancor_account_challenge_missing")?;
    let sign_output = run_nni_signature_helper(state, &["sign_challenge".to_string(), challenge]).await
        .map_err(|_| json!({"node_url": node_url, "error_code": "nni_bancor_account_signature_helper_failed", "retryable": true}))?;
    if !sign_output.ok {
        return Err(
            json!({"node_url": node_url, "error_code": "nni_bancor_account_signature_failed", "retryable": true}),
        );
    }
    let signature = value_string(
        &sign_output.payload,
        "signature",
        "nni_bancor_account_signature_missing",
    )?;
    let response = state.core.public_http_client
        .post(nni_remote_api_endpoint(node_url, "bancor/account/verify"))
        .timeout(nni_remote_api_timeout())
        .json(&NniBancorAccountVerifyRequest { task_id, signature, page, per_page })
        .send().await
        .map_err(|err| json!({"node_url": node_url, "error_code": "nni_bancor_account_verify_network_failed", "detail": err.to_string(), "retryable": true}))?;
    let status = response.status();
    let body = response.json::<ApiResponse<Value>>().await
        .map_err(|err| json!({"node_url": node_url, "error_code": "nni_bancor_account_verify_body_invalid", "detail": err.to_string(), "retryable": true}))?;
    if !status.is_success() || !body.ok {
        let error_code =
            nni_remote_api_error_code(&body, "nni_bancor_account_verify_failed");
        return Err(
            json!({"node_url": node_url, "http_status": status.as_u16(), "error_code": error_code, "retryable": nni_remote_http_status_retryable(status.as_u16())}),
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
    let mut owner_private_key = request.owner_private_key.take().map(Zeroizing::new);
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
    let authorization_mode = match normalize_bancor_authorization_mode(
        request.authorization_mode.as_deref(),
    ) {
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
    let configured_owner = match config.asset_owner_pubkey.as_deref() {
        Some(value) => match normalize_nni_owner_public_key(value) {
            Ok(value) => Some(value),
            Err(error) => {
                return nni_join_error(
                    StatusCode::CONFLICT,
                    error,
                    json!({"status": "asset_owner_invalid"}),
                )
            }
        },
        None => None,
    };
    let (device_pubkey, asset_owner_pubkey) = if authorization_mode == "asset_owner" {
        let Some(private_key) = owner_private_key.as_deref() else {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                "nni_owner_private_key_required",
                json!({"status": "trade_invalid"}),
            );
        };
        let owner_pubkey = match nni_owner_public_key_from_private(private_key) {
            Ok(value) => value,
            Err(error) => {
                return nni_join_error(
                    StatusCode::BAD_REQUEST,
                    error,
                    json!({"status": "trade_invalid"}),
                )
            }
        };
        if configured_owner
            .as_ref()
            .is_some_and(|configured| configured != &owner_pubkey)
        {
            return nni_join_error(
                StatusCode::CONFLICT,
                "nni_owner_private_key_mismatch",
                json!({"status": "trade_invalid"}),
            );
        }
        (None, owner_pubkey)
    } else {
        if owner_private_key.is_some() {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                "nni_owner_private_key_unexpected",
                json!({"status": "trade_invalid"}),
            );
        }
        let owner_pubkey = match configured_owner {
            Some(value) => value,
            None => {
                return nni_join_error(
                    StatusCode::CONFLICT,
                    "nni_asset_owner_required",
                    json!({"status": "asset_owner_required"}),
                )
            }
        };
        let device_pubkey = match nni_device_pubkey(&state).await {
            Ok(pubkey) => pubkey,
            Err((status, error, data)) => return nni_join_error(status, error, data),
        };
        (Some(device_pubkey), owner_pubkey)
    };
    let mut attempts = Vec::new();
    for node_url in nni_bancor_service_remote_nodes(&config) {
        let remote_request = NniBancorTradeRemoteRequest {
            device_pubkey: device_pubkey.clone(),
            asset_owner_pubkey: asset_owner_pubkey.clone(),
            authorization_mode: authorization_mode.to_string(),
            client_user_key: identity.user_key.clone(),
            side: request.side.clone(),
            input_amount: request.input_amount.clone(),
            slippage_bps,
            min_output: request.min_output.clone(),
        };
        match execute_nni_bancor_trade_for_node(
            &state,
            node_url,
            device_pubkey.as_deref(),
            &asset_owner_pubkey,
            authorization_mode,
            &request.side,
            &expected_input_units,
            &expected_min_output_units,
            &remote_request,
            owner_private_key.as_deref_mut(),
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
    device_pubkey: Option<&str>,
    asset_owner_pubkey: &str,
    authorization_mode: &str,
    side: &str,
    expected_input_units: &str,
    expected_min_output_units: &str,
    request: &NniBancorTradeRemoteRequest,
    owner_private_key: Option<&mut String>,
) -> Result<Value, Value> {
    let response = state.core.public_http_client
        .post(nni_remote_api_endpoint(node_url, "bancor/trade/request"))
        .timeout(nni_remote_api_timeout())
        .json(request).send().await
        .map_err(|err| json!({"node_url": node_url, "error_code": "nni_bancor_trade_request_network_failed", "detail": err.to_string()}))?;
    let status = response.status();
    let body = response.json::<ApiResponse<Value>>().await
        .map_err(|err| json!({"node_url": node_url, "error_code": "nni_bancor_trade_request_body_invalid", "detail": err.to_string()}))?;
    if !status.is_success() || !body.ok {
        let error_code =
            nni_remote_api_error_code(&body, "nni_bancor_trade_request_failed");
        return Err(
            json!({"node_url": node_url, "http_status": status.as_u16(), "error_code": error_code}),
        );
    }
    let data = body.data.ok_or_else(
        || json!({"node_url": node_url, "error_code": "nni_bancor_trade_request_data_missing"}),
    )?;
    let validated = validate_bancor_signing_payload(
        &data,
        device_pubkey,
        asset_owner_pubkey,
        authorization_mode,
        side,
        expected_input_units,
        expected_min_output_units,
    )
    .map_err(|error| json!({"node_url": node_url, "error_code": error}))?;
    let signature = if authorization_mode == "asset_owner" {
        let private_key = owner_private_key.ok_or_else(|| {
            json!({
                "node_url": node_url,
                "error_code": "nni_owner_private_key_required",
                "terminal": true,
            })
        })?;
        let (signing_pubkey, signature) = sign_nni_owner_payload(
            private_key,
            &validated.signing_payload,
        )
        .map_err(|error| {
            json!({"node_url": node_url, "error_code": error, "terminal": true})
        })?;
        if signing_pubkey != asset_owner_pubkey {
            return Err(json!({
                "node_url": node_url,
                "error_code": "nni_owner_private_key_mismatch",
                "terminal": true,
            }));
        }
        signature
    } else {
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
        value_string(
            &sign_output.payload,
            "signature",
            "nni_bancor_trade_signature_missing",
        )?
    };
    let response = state
        .core
        .public_http_client
        .post(nni_remote_api_endpoint(node_url, "bancor/trade/verify"))
        .timeout(nni_remote_api_timeout())
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
        let error_code =
            nni_remote_api_error_code(&body, "nni_bancor_trade_outcome_unknown");
        return Err(json!({
            "node_url": node_url,
            "http_status": status.as_u16(),
            "error_code": error_code,
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
    expected_device_pubkey: Option<&str>,
    expected_asset_owner_pubkey: &str,
    expected_authorization_mode: &str,
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
        "execution_policy",
        "quoted_market_version",
        "quote_id",
        "task_id",
        "device_pubkey",
        "asset_owner_pubkey",
        "authorization_epoch",
        "authorization_mode",
        "side",
        "input_units",
        "max_fee_bps",
        "quoted_fee_units",
        "quoted_output_units",
        "min_output_units",
        "nonce",
        "expires_at_unix",
    ]);
    let actual_keys = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    if actual_keys != expected_keys {
        return Err("nni_bancor_signing_payload_fields_invalid");
    }
    let payload_device_pubkey = payload
        .get("device_pubkey")
        .and_then(Value::as_str)
        .ok_or("nni_bancor_signing_payload_binding_invalid")?;
    if payload_device_pubkey.len() != 128
        || !payload_device_pubkey
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || expected_device_pubkey
            .is_some_and(|expected| payload_device_pubkey != expected)
    {
        return Err("nni_bancor_signing_payload_binding_invalid");
    }
    if payload.get("schema_version").and_then(Value::as_u64) != Some(2)
        || payload.get("action").and_then(Value::as_str) != Some("nni_bancor_trade")
        || payload.get("server_identity").and_then(Value::as_str) != Some("nni-server-v1")
        || payload.get("execution_policy").and_then(Value::as_str)
            != Some("current_reserves_with_min_output")
        || payload.get("asset_owner_pubkey").and_then(Value::as_str)
            != Some(expected_asset_owner_pubkey)
        || payload.get("authorization_epoch").and_then(Value::as_u64).is_none_or(|value| value == 0)
        || payload.get("authorization_mode").and_then(Value::as_str)
            != Some(expected_authorization_mode)
        || payload.get("side").and_then(Value::as_str) != Some(expected_side)
        || payload.get("input_units").and_then(Value::as_str) != Some(expected_input_units)
        || payload.get("min_output_units").and_then(Value::as_str)
            != Some(expected_min_output_units)
    {
        return Err("nni_bancor_signing_payload_binding_invalid");
    }
    for field in [
        "market_id",
        "quote_id",
        "task_id",
        "device_pubkey",
        "asset_owner_pubkey",
        "authorization_epoch",
        "authorization_mode",
        "input_units",
        "min_output_units",
        "expires_at_unix",
    ] {
        if payload.get(field) != response.get(field) {
            return Err("nni_bancor_signing_payload_response_mismatch");
        }
    }
    for (payload_field, response_field) in [
        ("quoted_market_version", "market_version"),
        ("max_fee_bps", "fee_bps"),
        ("quoted_fee_units", "fee_units"),
        ("quoted_output_units", "output_units"),
    ] {
        if payload.get(payload_field) != response.get(response_field) {
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

fn normalize_bancor_authorization_mode(value: Option<&str>) -> Result<&'static str, &'static str> {
    match value
        .unwrap_or("delegated_hardware")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "delegated_hardware" => Ok("delegated_hardware"),
        "asset_owner" => Ok("asset_owner"),
        _ => Err("nni_bancor_authorization_mode_invalid"),
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
        || fraction.len() > 8
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("nni_bancor_amount_invalid");
    }
    let whole_value = whole
        .parse::<u128>()
        .map_err(|_| "nni_bancor_amount_invalid")?;
    let fraction_padded = format!("{fraction:0<8}");
    let fraction_value = if fraction_padded.is_empty() {
        0
    } else {
        fraction_padded
            .parse::<u128>()
            .map_err(|_| "nni_bancor_amount_invalid")?
    };
    let units = whole_value
        .checked_mul(100_000_000)
        .and_then(|value| value.checked_add(fraction_value))
        .ok_or("nni_bancor_amount_invalid")?;
    if units == 0 || units > i64::MAX as u128 {
        return Err("nni_bancor_amount_invalid");
    }
    Ok((
        format!("{whole_value}.{:08}", fraction_value),
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
    fn amount_normalization_is_eight_decimal_and_rejects_float_syntax() {
        assert_eq!(
            normalize_bancor_amount("100").unwrap(),
            ("100.00000000".to_string(), "10000000000".to_string())
        );
        assert_eq!(normalize_bancor_amount("0.00000001").unwrap().1, "1");
        assert_eq!(
            normalize_bancor_amount("92233720368.54775807").unwrap().1,
            i64::MAX.to_string()
        );
        assert!(normalize_bancor_amount("0").is_err());
        assert!(normalize_bancor_amount("0.00000000").is_err());
        assert!(normalize_bancor_amount("92233720368.54775808").is_err());
        assert!(normalize_bancor_amount("1e2").is_err());
        assert!(normalize_bancor_amount("1.000000001").is_err());
    }

    #[test]
    fn slippage_allows_explicit_large_trade_protection_up_to_fifty_percent() {
        assert_eq!(normalize_bancor_slippage_bps(None), Ok(300));
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
    fn market_response_requires_consistent_utc_daily_marginal_statistics() {
        let valid = json!({
            "schema_version": 1,
            "status": "open",
            "market_id": "aic-usd-v1",
            "aic_symbol": "AIC",
            "usd_symbol": "USD",
            "aic_scale": 100_000_000,
            "usd_scale": 100_000_000,
            "aic_reserve_units": "9990000000000000",
            "aic_reserve": "99900000.00000000",
            "usd_reserve_units": "1001000000000",
            "usd_reserve": "10010.00000000",
            "marginal_price_usd_per_aic": "0.00010020",
            "daily_marginal_price": {
                "price_kind": "pool_marginal_usd_per_aic",
                "timezone": "UTC",
                "day_start_unix": 1_799_971_200,
                "open_usd_per_aic": "0.00010000",
                "high_usd_per_aic": "0.00010100",
                "low_usd_per_aic": "0.00009900",
                "change_percent": "0.20",
                "trade_count": 3,
            },
            "fee_bps": 50,
            "version": 3,
            "updated_at_unix": 1_800_000_000,
        });
        assert_eq!(validate_bancor_market_response(&valid), Ok(()));

        for field in ["price_kind", "timezone", "day_start_unix", "high_usd_per_aic"] {
            let mut invalid = valid.clone();
            invalid["daily_marginal_price"].as_object_mut().unwrap().remove(field);
            assert_eq!(
                validate_bancor_market_response(&invalid),
                Err("nni_bancor_market_contract_invalid"),
                "{field} must be required",
            );
        }
        let mut inconsistent = valid.clone();
        inconsistent["daily_marginal_price"]["change_percent"] =
            Value::String("-100.00".to_string());
        assert_eq!(
            validate_bancor_market_response(&inconsistent),
            Err("nni_bancor_market_contract_invalid"),
        );
    }

    #[test]
    fn candle_response_requires_price_semantics_and_market_series_identity() {
        let valid = json!({
            "schema_version": 1,
            "status": "bancor_candles",
            "market_id": "aic-usd-v1",
            "market_version": 7,
            "market_created_at_unix": 1_800_000_000,
            "price_kind": "execution_average_usd_per_aic",
            "interval_seconds": 300,
            "start_time_unix": 1_800_000_000,
            "end_time_unix": 1_800_000_300,
            "price_scale": 1_000_000_000_000_u64,
            "price_decimal_places": 12,
            "candles": [],
        });
        assert_eq!(validate_bancor_candles_response(&valid, 300, 120), Ok(()));

        for field in ["market_version", "market_created_at_unix", "price_kind"] {
            let mut invalid = valid.clone();
            invalid.as_object_mut().unwrap().remove(field);
            assert_eq!(
                validate_bancor_candles_response(&invalid, 300, 120),
                Err("nni_bancor_candles_contract_invalid"),
                "{field} must be required",
            );
        }
        let mut wrong_price_kind = valid.clone();
        wrong_price_kind["price_kind"] =
            Value::String("post_trade_marginal_usd_per_aic".to_string());
        assert_eq!(
            validate_bancor_candles_response(&wrong_price_kind, 300, 120),
            Err("nni_bancor_candles_contract_invalid"),
        );
    }

    #[test]
    fn candle_response_must_match_the_requested_interval_and_page_limit() {
        let payload = json!({
            "schema_version": 1,
            "status": "bancor_candles",
            "market_id": "aic-usd-v1",
            "market_version": 7,
            "market_created_at_unix": 1_800_000_000,
            "price_kind": "execution_average_usd_per_aic",
            "interval_seconds": 60,
            "start_time_unix": 1_800_000_000,
            "end_time_unix": 1_800_000_120,
            "price_scale": 1_000_000_000_000_u64,
            "price_decimal_places": 12,
            "candles": [{}, {}],
        });
        assert_eq!(
            validate_bancor_candles_response(&payload, 300, 2),
            Err("nni_bancor_candles_contract_invalid"),
        );
        assert_eq!(
            validate_bancor_candles_response(&payload, 60, 1),
            Err("nni_bancor_candles_contract_invalid"),
        );
    }

    #[test]
    fn candle_response_rejects_malformed_buckets_prices_and_trade_state() {
        let valid_candle = json!({
            "bucket_start_unix": 1_800_000_000,
            "bucket_end_unix": 1_800_000_300,
            "open": "0.000100000000",
            "high": "0.000110000000",
            "low": "0.000090000000",
            "close": "0.000105000000",
            "aic_volume_units": "100000000",
            "aic_volume": "1.00000000",
            "usd_volume_units": "10000",
            "usd_volume": "0.00010000",
            "trade_count": 1,
            "has_trades": true,
        });
        let envelope = |candles: Value| {
            json!({
                "schema_version": 1,
                "status": "bancor_candles",
                "market_id": "aic-usd-v1",
                "market_version": 7,
                "market_created_at_unix": 1_800_000_000,
                "price_kind": "execution_average_usd_per_aic",
                "interval_seconds": 300,
                "start_time_unix": 1_800_000_000,
                "end_time_unix": 1_800_000_600,
                "price_scale": 1_000_000_000_000_u64,
                "price_decimal_places": 12,
                "candles": candles,
            })
        };
        assert_eq!(
            validate_bancor_candles_response(
                &envelope(Value::Array(vec![valid_candle.clone()])),
                300,
                120,
            ),
            Ok(()),
        );

        let mut reversed_range = valid_candle.clone();
        reversed_range["bucket_end_unix"] = Value::Number(1_799_999_999.into());
        let mut invalid_high = valid_candle.clone();
        invalid_high["high"] = Value::String("0.000099000000".to_string());
        let mut inconsistent_trade_state = valid_candle.clone();
        inconsistent_trade_state["has_trades"] = Value::Bool(false);
        for candle in [reversed_range, invalid_high, inconsistent_trade_state] {
            assert_eq!(
                validate_bancor_candles_response(&envelope(Value::Array(vec![candle])), 300, 120,),
                Err("nni_bancor_candles_contract_invalid"),
            );
        }

        let mut later = valid_candle.clone();
        later["bucket_start_unix"] = Value::Number(1_800_000_200.into());
        later["bucket_end_unix"] = Value::Number(1_800_000_500.into());
        assert_eq!(
            validate_bancor_candles_response(
                &envelope(Value::Array(vec![valid_candle, later])),
                300,
                120,
            ),
            Err("nni_bancor_candles_contract_invalid"),
        );
    }

    #[test]
    fn device_signing_is_restricted_to_the_bound_bancor_contract() {
        let expires_at = current_unix_ts() + 120;
        let asset_owner_pubkey = "5p78kHbL33Rn3JWkTWRE2B9uz6gy4r1KbfAKLNQGE3ovLY8E9M";
        let payload = json!({
            "schema_version": 2,
            "action": "nni_bancor_trade",
            "server_identity": "nni-server-v1",
            "market_id": "aic-usd-v1",
            "execution_policy": "current_reserves_with_min_output",
            "quoted_market_version": 7,
            "quote_id": "quote-1",
            "task_id": "task-1",
            "device_pubkey": "aa".repeat(64),
            "asset_owner_pubkey": asset_owner_pubkey,
            "authorization_epoch": 1,
            "authorization_mode": "delegated_hardware",
            "side": "sell",
            "input_units": "10000",
            "max_fee_bps": 0,
            "quoted_fee_units": "0",
            "quoted_output_units": "1",
            "min_output_units": "1",
            "nonce": "11".repeat(16),
            "expires_at_unix": expires_at,
        });
        let signing_payload = serde_json::to_string(&payload).unwrap();
        let digest = format!("{:x}", Sha256::digest(signing_payload.as_bytes()));
        let response = json!({
            "signing_payload": signing_payload,
            "signing_payload_digest": digest,
            "market_id": "aic-usd-v1",
            "market_version": 7,
            "quote_id": "quote-1",
            "task_id": "task-1",
            "device_pubkey": "aa".repeat(64),
            "asset_owner_pubkey": asset_owner_pubkey,
            "authorization_epoch": 1,
            "authorization_mode": "delegated_hardware",
            "input_units": "10000",
            "fee_bps": 0,
            "fee_units": "0",
            "output_units": "1",
            "min_output_units": "1",
            "expires_at_unix": expires_at,
        });
        assert!(
            validate_bancor_signing_payload(
                &response,
                Some(&"aa".repeat(64)),
                asset_owner_pubkey,
                "delegated_hardware",
                "sell",
                "10000",
                "1",
            )
                .is_ok()
        );
        assert_eq!(
            validate_bancor_signing_payload(
                &response,
                Some(&"aa".repeat(64)),
                asset_owner_pubkey,
                "delegated_hardware",
                "sell",
                "20000",
                "1",
            )
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
                Some(&"aa".repeat(64)),
                asset_owner_pubkey,
                "delegated_hardware",
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
