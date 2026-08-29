const NNI_ASSET_TRANSFER_SIGNING_SCHEMA_VERSION: u64 = 2;
const NNI_ASSET_TRANSFER_MEMO_MAX_BYTES: usize = 256;
const NNI_ASSET_TRANSFER_HISTORY_DEFAULT_LIMIT: usize = 100;
const NNI_ASSET_TRANSFER_HISTORY_MAX_LIMIT: usize = 100;
const NNI_ASSET_TRANSFER_HISTORY_REMOTE_PAGE_SIZE: usize = 100;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum NniAssetHistorySourceFilter {
    #[default]
    All,
    Transfer,
    Trade,
    Issuance,
}

impl NniAssetHistorySourceFilter {
    fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Transfer => "transfer",
            Self::Trade => "trade",
            Self::Issuance => "issuance",
        }
    }

    fn remote_class(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::Transfer => Some("peer_transfer"),
            Self::Trade => Some("market_trade"),
            Self::Issuance => Some("system_issuance"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum NniAssetHistoryDirectionFilter {
    #[default]
    All,
    Incoming,
    Outgoing,
}

impl NniAssetHistoryDirectionFilter {
    fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Incoming => "incoming",
            Self::Outgoing => "outgoing",
        }
    }

    fn remote_direction(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::Incoming => Some("incoming"),
            Self::Outgoing => Some("outgoing"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct NniAssetTransferHistoryQuery {
    owner_pubkey: String,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    page: Option<usize>,
    #[serde(default)]
    source: NniAssetHistorySourceFilter,
    #[serde(default)]
    direction: NniAssetHistoryDirectionFilter,
}

#[derive(Debug, Deserialize)]
struct NniAssetTransferRequest {
    asset: String,
    amount: String,
    to_asset_owner_pubkey: String,
    #[serde(default)]
    memo: String,
    #[serde(default)]
    authorization_mode: Option<String>,
    #[serde(default)]
    owner_private_key: Option<String>,
}

#[derive(Debug, Serialize)]
struct NniAssetTransferRemoteRequest {
    request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_pubkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from_asset_owner_pubkey: Option<String>,
    to_asset_owner_pubkey: String,
    authorization_mode: String,
    client_user_key: String,
    asset: String,
    amount: String,
    signing_payload_schema_version: u64,
    memo: String,
}

#[derive(Debug, Serialize)]
struct NniAssetTransferVerifyRequest {
    task_id: String,
    transfer_id: String,
    signature: String,
}

#[derive(Debug)]
struct ValidatedAssetTransferPayload {
    signing_payload: String,
    task_id: String,
    transfer_id: String,
}

async fn nni_asset_transfer_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<NniAssetTransferHistoryQuery>,
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
    if !identity.role.eq_ignore_ascii_case("admin") {
        return nni_join_error(
            StatusCode::FORBIDDEN,
            "admin_required",
            json!({"status": "asset_transfer_history_forbidden"}),
        );
    }
    let owner_pubkey = match normalize_nni_owner_public_key(&query.owner_pubkey) {
        Ok(value) => value,
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                error,
                json!({"status": "asset_transfer_history_query_invalid"}),
            )
        }
    };
    let limit = query
        .limit
        .unwrap_or(NNI_ASSET_TRANSFER_HISTORY_DEFAULT_LIMIT)
        .clamp(1, NNI_ASSET_TRANSFER_HISTORY_MAX_LIMIT);
    let page = query.page.unwrap_or(1).clamp(1, 1_000_000);
    let config = match read_nni_config(&state) {
        Ok(config) => config,
        Err(error) => {
            return nni_join_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "nni_config_read_failed",
                json!({"error": error.to_string()}),
            )
        }
    };

    let mut attempts = Vec::new();
    for node_url in nni_asset_service_remote_nodes(&config) {
        match nni_remote_read_with_retry(|| {
            query_nni_asset_transfer_history_for_node(
                &state,
                node_url,
                &owner_pubkey,
                limit,
                page,
                query.source,
                query.direction,
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
        "nni_asset_transfer_history_nodes_unavailable",
        json!({
            "status": "asset_transfer_history_nodes_unavailable",
            "attempts": attempts,
        }),
    )
}

async fn query_nni_asset_transfer_history_for_node(
    state: &AppState,
    node_url: &str,
    owner_pubkey: &str,
    limit: usize,
    page: usize,
    source: NniAssetHistorySourceFilter,
    direction: NniAssetHistoryDirectionFilter,
) -> Result<Value, Value> {
    let endpoint = nni_remote_api_endpoint(node_url, "explorer/transactions");
    let response = state
        .core
        .public_http_client
        .get(endpoint)
        .query(&nni_asset_transfer_history_remote_query(
            owner_pubkey,
            page,
            limit,
            source,
            direction,
        ))
        .timeout(nni_remote_api_timeout())
        .send()
        .await
        .map_err(|error| {
            json!({
                "node_url": node_url,
                "error_code": "nni_asset_transfer_history_network_failed",
                "detail": error.to_string(),
                "retryable": true,
            })
        })?;
    let status = response.status();
    let body = response
        .json::<ApiResponse<Value>>()
        .await
        .map_err(|error| {
            json!({
                "node_url": node_url,
                "http_status": status.as_u16(),
                "error_code": "nni_asset_transfer_history_body_invalid",
                "detail": error.to_string(),
                "retryable": nni_remote_http_status_retryable(status.as_u16()),
            })
        })?;
    if !status.is_success() || !body.ok {
        let error_code =
            nni_remote_api_error_code(&body, "nni_asset_transfer_history_request_failed");
        return Err(json!({
            "node_url": node_url,
            "http_status": status.as_u16(),
            "error_code": error_code,
            "retryable": nni_remote_http_status_retryable(status.as_u16()),
        }));
    }
    let data = body.data.ok_or_else(|| {
        json!({
            "node_url": node_url,
            "error_code": "nni_asset_transfer_history_data_missing",
        })
    })?;
    normalize_asset_transfer_history_response(&data, owner_pubkey, source, direction).map_err(
        |error_code| {
            json!({
                "node_url": node_url,
                "error_code": error_code,
            })
        },
    )
}

fn nni_asset_transfer_history_remote_query(
    owner_pubkey: &str,
    page: usize,
    per_page: usize,
    source: NniAssetHistorySourceFilter,
    direction: NniAssetHistoryDirectionFilter,
) -> Vec<(&'static str, String)> {
    let mut query = vec![
        ("address", owner_pubkey.to_string()),
        ("page", page.to_string()),
        (
            "per_page",
            per_page.min(NNI_ASSET_TRANSFER_HISTORY_REMOTE_PAGE_SIZE).to_string(),
        ),
    ];
    if let Some(transaction_class) = source.remote_class() {
        query.push(("transaction_class", transaction_class.to_string()));
    }
    if let Some(remote_direction) = direction.remote_direction() {
        query.push(("direction", remote_direction.to_string()));
    }
    query
}

fn normalize_asset_transfer_history_response(
    data: &Value,
    owner_pubkey: &str,
    source: NniAssetHistorySourceFilter,
    direction: NniAssetHistoryDirectionFilter,
) -> Result<Value, &'static str> {
    if data.get("schema_version").and_then(Value::as_u64) != Some(1)
        || data.get("status").and_then(Value::as_str) != Some("explorer_transactions")
    {
        return Err("nni_asset_transfer_history_contract_invalid");
    }
    let transactions = data
        .get("transactions")
        .and_then(Value::as_array)
        .ok_or("nni_asset_transfer_history_contract_invalid")?;
    if transactions.len() > NNI_ASSET_TRANSFER_HISTORY_REMOTE_PAGE_SIZE {
        return Err("nni_asset_transfer_history_contract_invalid");
    }
    let page = data
        .get("page")
        .and_then(Value::as_u64)
        .filter(|value| *value >= 1)
        .ok_or("nni_asset_transfer_history_contract_invalid")?;
    let per_page = data
        .get("per_page")
        .and_then(Value::as_u64)
        .filter(|value| (1..=NNI_ASSET_TRANSFER_HISTORY_REMOTE_PAGE_SIZE as u64).contains(value))
        .ok_or("nni_asset_transfer_history_contract_invalid")?;
    let total_transactions = data
        .get("total")
        .and_then(Value::as_u64)
        .ok_or("nni_asset_transfer_history_contract_invalid")?;
    let total_pages = data
        .get("total_pages")
        .and_then(Value::as_u64)
        .ok_or("nni_asset_transfer_history_contract_invalid")?;
    let expected_total_pages = total_transactions.div_ceil(per_page).max(1);
    if total_pages != expected_total_pages {
        return Err("nni_asset_transfer_history_contract_invalid");
    }
    validate_asset_transfer_history_remote_filter(data.get("filter"), source, direction)?;

    let mut projected = Vec::new();
    for transaction in transactions {
        let transaction_kind = transaction
            .get("transaction_kind")
            .and_then(Value::as_str)
            .filter(|value| valid_nni_explorer_machine_token(value))
            .ok_or("nni_asset_transfer_history_contract_invalid")?;
        let transaction_class = classify_nni_asset_history_transaction(transaction_kind);
        if transaction.get("transaction_class").and_then(Value::as_str)
            != Some(transaction_class)
            || !nni_asset_history_source_matches(source, transaction_class)
        {
            return Err("nni_asset_transfer_history_contract_invalid");
        }
        let transaction_id = transaction
            .get("transaction_id")
            .and_then(Value::as_str)
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= 160
                    && value.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-')
                    })
            })
            .ok_or("nni_asset_transfer_history_contract_invalid")?;
        let created_at_unix = transaction
            .get("created_at_unix")
            .and_then(Value::as_i64)
            .filter(|value| (0..=8_640_000_000).contains(value))
            .ok_or("nni_asset_transfer_history_contract_invalid")?;
        let memo = match transaction.get("memo") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) if value.len() <= NNI_ASSET_TRANSFER_MEMO_MAX_BYTES => {
                Some(value.clone())
            }
            _ => return Err("nni_asset_transfer_history_contract_invalid"),
        };
        let flows = transaction
            .get("flows")
            .and_then(Value::as_array)
            .filter(|flows| !flows.is_empty() && flows.len() <= 8)
            .ok_or("nni_asset_transfer_history_contract_invalid")?;
        let mut projected_flows = Vec::new();
        for flow in flows {
            let flow_index = flow
                .get("flow_index")
                .and_then(Value::as_u64)
                .filter(|value| *value <= 100)
                .ok_or("nni_asset_transfer_history_contract_invalid")?;
            let asset = normalize_asset_transfer_asset(
                flow.get("asset")
                    .and_then(Value::as_str)
                    .ok_or("nni_asset_transfer_history_contract_invalid")?,
            )
            .map_err(|_| "nni_asset_transfer_history_contract_invalid")?;
            let amount = flow
                .get("amount")
                .and_then(Value::as_str)
                .ok_or("nni_asset_transfer_history_contract_invalid")?;
            let (normalized_amount, amount_units) = normalize_bancor_amount(amount)
                .map_err(|_| "nni_asset_transfer_history_contract_invalid")?;
            if flow.get("amount_units").and_then(Value::as_str) != Some(amount_units.as_str()) {
                return Err("nni_asset_transfer_history_contract_invalid");
            }
            let (from, from_address) = normalize_asset_transfer_history_account(flow.get("from"))?;
            let (to, to_address) = normalize_asset_transfer_history_account(flow.get("to"))?;
            let owner_is_sender = from_address.as_deref() == Some(owner_pubkey);
            let owner_is_recipient = to_address.as_deref() == Some(owner_pubkey);
            let include = match direction {
                NniAssetHistoryDirectionFilter::All => owner_is_sender || owner_is_recipient,
                NniAssetHistoryDirectionFilter::Incoming => owner_is_recipient,
                NniAssetHistoryDirectionFilter::Outgoing => owner_is_sender,
            };
            if !include {
                continue;
            }
            projected_flows.push(json!({
                "flow_index": flow_index,
                "asset": asset,
                "amount_units": amount_units,
                "amount": normalized_amount,
                "from": from,
                "to": to,
            }));
        }
        if projected_flows.is_empty() {
            return Err("nni_asset_transfer_history_contract_invalid");
        }
        projected.push(json!({
            "transaction_id": transaction_id,
            "transaction_kind": transaction_kind,
            "transaction_class": transaction_class,
            "created_at_unix": created_at_unix,
            "memo": memo,
            "flows": projected_flows,
        }));
    }

    Ok(json!({
        "schema_version": 1,
        "status": "asset_transfer_history",
        "owner_pubkey": owner_pubkey,
        "page": page,
        "per_page": per_page,
        "total_transactions": total_transactions,
        "total_pages": total_pages,
        "source_filter": source.as_str(),
        "direction_filter": direction.as_str(),
        "transactions": projected,
    }))
}

fn normalize_asset_transfer_history_account(
    value: Option<&Value>,
) -> Result<(Value, Option<String>), &'static str> {
    let value = value
        .and_then(Value::as_object)
        .ok_or("nni_asset_transfer_history_contract_invalid")?;
    let account_kind = value
        .get("account_kind")
        .and_then(Value::as_str)
        .ok_or("nni_asset_transfer_history_contract_invalid")?;
    if account_kind == "system" {
        if !matches!(value.get("address"), None | Some(Value::Null)) {
            return Err("nni_asset_transfer_history_contract_invalid");
        }
        return Ok((json!({"account_kind": "system", "address": null}), None));
    }
    if !matches!(account_kind, "asset_owner" | "pool" | "fee") {
        return Err("nni_asset_transfer_history_contract_invalid");
    }
    let address = normalize_nni_owner_public_key(value.get("address").and_then(Value::as_str)
        .ok_or("nni_asset_transfer_history_contract_invalid")?)
        .map_err(|_| "nni_asset_transfer_history_contract_invalid")?;
    Ok((json!({"account_kind": account_kind, "address": address}), Some(address)))
}

fn valid_nni_explorer_machine_token(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
}

fn classify_nni_asset_history_transaction(transaction_kind: &str) -> &'static str {
    match transaction_kind {
        "asset_transfer" => "peer_transfer",
        "bancor_buy" | "bancor_sell" => "market_trade",
        "heartbeat_reward_credit" | "admin_usd_credit" | "market_bootstrap" => {
            "system_issuance"
        }
        _ => "other",
    }
}

fn nni_asset_history_source_matches(
    source: NniAssetHistorySourceFilter,
    transaction_class: &str,
) -> bool {
    match source {
        NniAssetHistorySourceFilter::All => true,
        NniAssetHistorySourceFilter::Transfer => transaction_class == "peer_transfer",
        NniAssetHistorySourceFilter::Trade => transaction_class == "market_trade",
        NniAssetHistorySourceFilter::Issuance => transaction_class == "system_issuance",
    }
}

fn validate_asset_transfer_history_remote_filter(
    value: Option<&Value>,
    source: NniAssetHistorySourceFilter,
    direction: NniAssetHistoryDirectionFilter,
) -> Result<(), &'static str> {
    let filter = value
        .and_then(Value::as_object)
        .ok_or("nni_asset_transfer_history_contract_invalid")?;
    if !matches!(filter.get("transaction_kind"), None | Some(Value::Null))
        || filter.get("transaction_class").and_then(Value::as_str) != source.remote_class()
        || filter.get("direction").and_then(Value::as_str) != direction.remote_direction()
    {
        return Err("nni_asset_transfer_history_contract_invalid");
    }
    Ok(())
}

async fn nni_asset_transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut request): Json<NniAssetTransferRequest>,
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
    if !identity.role.eq_ignore_ascii_case("admin") {
        return nni_join_error(
            StatusCode::FORBIDDEN,
            "admin_required",
            json!({"status": "asset_transfer_forbidden"}),
        );
    }
    let asset = match normalize_asset_transfer_asset(&request.asset) {
        Ok(value) => value,
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                error,
                json!({"status": "asset_transfer_invalid"}),
            )
        }
    };
    let (amount, amount_units) = match normalize_bancor_amount(&request.amount) {
        Ok(value) => value,
        Err(_) => {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                "nni_asset_transfer_amount_invalid",
                json!({"status": "asset_transfer_invalid"}),
            )
        }
    };
    let to_owner_pubkey = match normalize_nni_owner_public_key(&request.to_asset_owner_pubkey) {
        Ok(value) => value,
        Err(error) => {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                error,
                json!({"status": "asset_transfer_invalid"}),
            )
        }
    };
    if request.memo.len() > NNI_ASSET_TRANSFER_MEMO_MAX_BYTES {
        return nni_join_error(
            StatusCode::BAD_REQUEST,
            "nni_asset_transfer_memo_too_long",
            json!({"status": "asset_transfer_invalid"}),
        );
    }
    let authorization_mode =
        match normalize_asset_transfer_authorization_mode(request.authorization_mode.as_deref()) {
            Ok(value) => value,
            Err(error) => {
                return nni_join_error(
                    StatusCode::BAD_REQUEST,
                    error,
                    json!({"status": "asset_transfer_invalid"}),
                )
            }
        };
    let config = match read_nni_config(&state) {
        Ok(config) => config,
        Err(error) => {
            return nni_join_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "nni_config_read_failed",
                json!({"error": error.to_string()}),
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
    let (device_pubkey, from_owner_pubkey) = if authorization_mode == "asset_owner" {
        let Some(private_key) = owner_private_key.as_deref() else {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                "nni_owner_private_key_required",
                json!({"status": "asset_transfer_invalid"}),
            );
        };
        let owner_pubkey = match nni_owner_public_key_from_private(private_key) {
            Ok(value) => value,
            Err(error) => {
                return nni_join_error(
                    StatusCode::BAD_REQUEST,
                    error,
                    json!({"status": "asset_transfer_invalid"}),
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
                json!({"status": "asset_transfer_invalid"}),
            );
        }
        (None, owner_pubkey)
    } else {
        if owner_private_key.is_some() {
            return nni_join_error(
                StatusCode::BAD_REQUEST,
                "nni_owner_private_key_unexpected",
                json!({"status": "asset_transfer_invalid"}),
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
            Ok(value) => value,
            Err((status, error, data)) => return nni_join_error(status, error, data),
        };
        (Some(device_pubkey), owner_pubkey)
    };
    if from_owner_pubkey == to_owner_pubkey {
        return nni_join_error(
            StatusCode::BAD_REQUEST,
            "nni_asset_transfer_same_account",
            json!({"status": "asset_transfer_invalid"}),
        );
    }

    let request_id = uuid::Uuid::new_v4().to_string();
    let mut attempts = Vec::new();
    for node_url in nni_asset_service_remote_nodes(&config) {
        let remote_request = NniAssetTransferRemoteRequest {
            request_id: request_id.clone(),
            device_pubkey: device_pubkey.clone(),
            from_asset_owner_pubkey: (authorization_mode == "asset_owner")
                .then(|| from_owner_pubkey.clone()),
            to_asset_owner_pubkey: to_owner_pubkey.clone(),
            authorization_mode: authorization_mode.to_string(),
            client_user_key: identity.user_key.clone(),
            asset: asset.to_string(),
            amount: amount.clone(),
            signing_payload_schema_version: NNI_ASSET_TRANSFER_SIGNING_SCHEMA_VERSION,
            memo: request.memo.clone(),
        };
        match execute_nni_asset_transfer_for_node(
            &state,
            node_url,
            device_pubkey.as_deref(),
            &from_owner_pubkey,
            &to_owner_pubkey,
            authorization_mode,
            asset,
            &amount_units,
            &request.memo,
            &request_id,
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
                let terminal = attempt
                    .get("terminal")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                attempts.push(attempt);
                if terminal {
                    break;
                }
            }
        }
    }
    if let Some(terminal) = attempts
        .last()
        .filter(|attempt| attempt.get("terminal").and_then(Value::as_bool) == Some(true))
    {
        let status = terminal
            .get("http_status")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .and_then(|value| StatusCode::from_u16(value).ok())
            .unwrap_or(StatusCode::BAD_GATEWAY);
        let error_code = terminal
            .get("error_code")
            .and_then(Value::as_str)
            .unwrap_or("nni_asset_transfer_failed")
            .to_string();
        return nni_join_error(status, error_code, json!({"attempts": attempts}));
    }
    nni_join_error(
        StatusCode::BAD_GATEWAY,
        "nni_asset_transfer_nodes_unavailable",
        json!({"attempts": attempts}),
    )
}

#[allow(clippy::too_many_arguments)]
async fn execute_nni_asset_transfer_for_node(
    state: &AppState,
    node_url: &str,
    device_pubkey: Option<&str>,
    from_owner_pubkey: &str,
    to_owner_pubkey: &str,
    authorization_mode: &str,
    asset: &str,
    amount_units: &str,
    memo: &str,
    request_id: &str,
    request: &NniAssetTransferRemoteRequest,
    owner_private_key: Option<&mut String>,
) -> Result<Value, Value> {
    let response = state
        .core
        .public_http_client
        .post(nni_remote_api_endpoint(node_url, "assets/transfer/request"))
        .timeout(nni_remote_api_timeout())
        .json(request)
        .send()
        .await
        .map_err(|error| {
            json!({
                "node_url": node_url,
                "error_code": "nni_asset_transfer_request_network_failed",
                "detail": error.to_string(),
                "retryable": true,
            })
        })?;
    let status = response.status();
    let body = response
        .json::<ApiResponse<Value>>()
        .await
        .map_err(|error| {
            json!({
                "node_url": node_url,
                "error_code": "nni_asset_transfer_request_body_invalid",
                "detail": error.to_string(),
                "retryable": true,
            })
        })?;
    if !status.is_success() || !body.ok {
        let error_code = nni_remote_api_error_code(&body, "nni_asset_transfer_request_failed");
        let retry_after_seconds = body
            .data
            .as_ref()
            .and_then(|data| data.get("retry_after_seconds"))
            .and_then(Value::as_u64);
        return Err(json!({
            "node_url": node_url,
            "http_status": status.as_u16(),
            "error_code": error_code,
            "retryable": nni_remote_http_status_retryable(status.as_u16()),
            "terminal": status.is_client_error(),
            "retry_after_seconds": retry_after_seconds,
        }));
    }
    let data = body.data.ok_or_else(|| {
        json!({
            "node_url": node_url,
            "error_code": "nni_asset_transfer_request_data_missing",
        })
    })?;
    let validated = validate_asset_transfer_signing_payload(
        &data,
        device_pubkey,
        from_owner_pubkey,
        to_owner_pubkey,
        authorization_mode,
        asset,
        amount_units,
        memo,
        request_id,
    )
    .map_err(|error| json!({"node_url": node_url, "error_code": error, "terminal": true}))?;

    let signature = if authorization_mode == "asset_owner" {
        let private_key = owner_private_key.ok_or_else(|| {
            json!({
                "node_url": node_url,
                "error_code": "nni_owner_private_key_required",
                "terminal": true,
            })
        })?;
        let (signing_pubkey, signature) =
            sign_nni_owner_payload(private_key, &validated.signing_payload).map_err(
                |error| json!({"node_url": node_url, "error_code": error, "terminal": true}),
            )?;
        if signing_pubkey != from_owner_pubkey {
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
        .map_err(|_| {
            json!({
                "node_url": node_url,
                "error_code": "nni_asset_transfer_signature_helper_failed",
            })
        })?;
        if !sign_output.ok {
            return Err(json!({
                "node_url": node_url,
                "error_code": "nni_asset_transfer_signature_failed",
            }));
        }
        value_string(
            &sign_output.payload,
            "signature",
            "nni_asset_transfer_signature_missing",
        )?
    };

    let verify_request = NniAssetTransferVerifyRequest {
        task_id: validated.task_id,
        transfer_id: validated.transfer_id,
        signature,
    };
    let mut last_transport_error = None;
    for attempt in 0..2 {
        let response = match state
            .core
            .public_http_client
            .post(nni_remote_api_endpoint(node_url, "assets/transfer/verify"))
            .timeout(nni_remote_api_timeout())
            .json(&verify_request)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                last_transport_error = Some(json!({
                    "node_url": node_url,
                    "error_code": "nni_asset_transfer_outcome_unknown",
                    "detail": error.to_string(),
                    "terminal": true,
                }));
                if attempt == 0 {
                    continue;
                }
                break;
            }
        };
        let status = response.status();
        let body = match response.json::<ApiResponse<Value>>().await {
            Ok(body) => body,
            Err(error) => {
                last_transport_error = Some(json!({
                    "node_url": node_url,
                    "error_code": "nni_asset_transfer_outcome_unknown",
                    "detail": error.to_string(),
                    "terminal": true,
                }));
                if attempt == 0 {
                    continue;
                }
                break;
            }
        };
        if !status.is_success() || !body.ok {
            if status.is_server_error() && attempt == 0 {
                continue;
            }
            let error_code = nni_remote_api_error_code(&body, "nni_asset_transfer_outcome_unknown");
            let retry_after_seconds = body
                .data
                .as_ref()
                .and_then(|data| data.get("retry_after_seconds"))
                .and_then(Value::as_u64);
            return Err(json!({
                "node_url": node_url,
                "http_status": status.as_u16(),
                "error_code": error_code,
                "terminal": true,
                "retry_after_seconds": retry_after_seconds,
            }));
        }
        let data = body.data.ok_or_else(|| {
            json!({
                "node_url": node_url,
                "error_code": "nni_asset_transfer_outcome_unknown",
                "terminal": true,
            })
        })?;
        if data.get("request_id").and_then(Value::as_str) != Some(request_id) {
            return Err(json!({
                "node_url": node_url,
                "error_code": "nni_asset_transfer_request_id_mismatch",
                "terminal": true,
            }));
        }
        return Ok(data);
    }
    Err(last_transport_error.unwrap_or_else(|| {
        json!({
            "node_url": node_url,
            "error_code": "nni_asset_transfer_outcome_unknown",
            "terminal": true,
        })
    }))
}

fn validate_asset_transfer_signing_payload(
    response: &Value,
    expected_device_pubkey: Option<&str>,
    expected_from_owner_pubkey: &str,
    expected_to_owner_pubkey: &str,
    expected_authorization_mode: &str,
    expected_asset: &str,
    expected_amount_units: &str,
    expected_memo: &str,
    expected_request_id: &str,
) -> Result<ValidatedAssetTransferPayload, &'static str> {
    let signing_payload = response
        .get("signing_payload")
        .and_then(Value::as_str)
        .ok_or("nni_asset_transfer_signing_payload_missing")?;
    if signing_payload.len() > 4096 {
        return Err("nni_asset_transfer_signing_payload_too_large");
    }
    let payload: Value = serde_json::from_str(signing_payload)
        .map_err(|_| "nni_asset_transfer_signing_payload_invalid")?;
    let object = payload
        .as_object()
        .ok_or("nni_asset_transfer_signing_payload_invalid")?;
    let expected_keys = BTreeSet::from([
        "schema_version",
        "action",
        "server_identity",
        "transfer_id",
        "task_id",
        "device_pubkey",
        "from_asset_owner_pubkey",
        "to_asset_owner_pubkey",
        "authorization_epoch",
        "authorization_mode",
        "asset",
        "amount_units",
        "memo",
        "nonce",
        "expires_at_unix",
    ]);
    if object.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected_keys {
        return Err("nni_asset_transfer_signing_payload_fields_invalid");
    }
    if response.get("request_id").and_then(Value::as_str) != Some(expected_request_id) {
        return Err("nni_asset_transfer_request_id_mismatch");
    }
    let payload_device_pubkey = payload
        .get("device_pubkey")
        .and_then(Value::as_str)
        .ok_or("nni_asset_transfer_signing_payload_binding_invalid")?;
    if payload_device_pubkey.len() != 128
        || !payload_device_pubkey
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || expected_device_pubkey.is_some_and(|expected| payload_device_pubkey != expected)
        || payload.get("schema_version").and_then(Value::as_u64)
            != Some(NNI_ASSET_TRANSFER_SIGNING_SCHEMA_VERSION)
        || payload.get("action").and_then(Value::as_str) != Some("nni_asset_transfer")
        || payload.get("server_identity").and_then(Value::as_str) != Some("nni-server-v1")
        || payload
            .get("from_asset_owner_pubkey")
            .and_then(Value::as_str)
            != Some(expected_from_owner_pubkey)
        || payload.get("to_asset_owner_pubkey").and_then(Value::as_str)
            != Some(expected_to_owner_pubkey)
        || payload
            .get("authorization_epoch")
            .and_then(Value::as_u64)
            .is_none_or(|value| value == 0)
        || payload.get("authorization_mode").and_then(Value::as_str)
            != Some(expected_authorization_mode)
        || payload.get("asset").and_then(Value::as_str) != Some(expected_asset)
        || payload.get("amount_units").and_then(Value::as_str) != Some(expected_amount_units)
        || payload.get("memo").and_then(Value::as_str) != Some(expected_memo)
        || expected_memo.len() > NNI_ASSET_TRANSFER_MEMO_MAX_BYTES
    {
        return Err("nni_asset_transfer_signing_payload_binding_invalid");
    }
    for field in [
        "transfer_id",
        "task_id",
        "device_pubkey",
        "from_asset_owner_pubkey",
        "to_asset_owner_pubkey",
        "authorization_epoch",
        "authorization_mode",
        "asset",
        "amount_units",
        "memo",
        "expires_at_unix",
    ] {
        if payload.get(field) != response.get(field) {
            return Err("nni_asset_transfer_signing_payload_response_mismatch");
        }
    }
    let nonce = payload
        .get("nonce")
        .and_then(Value::as_str)
        .ok_or("nni_asset_transfer_signing_payload_nonce_invalid")?;
    if nonce.len() != 32 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("nni_asset_transfer_signing_payload_nonce_invalid");
    }
    let expires = payload
        .get("expires_at_unix")
        .and_then(Value::as_i64)
        .ok_or("nni_asset_transfer_signing_payload_expiry_invalid")?;
    let now = current_unix_ts();
    if expires < now || expires > now.saturating_add(600) {
        return Err("nni_asset_transfer_signing_payload_expiry_invalid");
    }
    let digest = format!("{:x}", Sha256::digest(signing_payload.as_bytes()));
    if response
        .get("signing_payload_digest")
        .and_then(Value::as_str)
        != Some(digest.as_str())
    {
        return Err("nni_asset_transfer_signing_payload_digest_invalid");
    }
    let task_id = payload
        .get("task_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("nni_asset_transfer_task_id_missing")?
        .to_string();
    let transfer_id = payload
        .get("transfer_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("nni_asset_transfer_id_missing")?
        .to_string();
    Ok(ValidatedAssetTransferPayload {
        signing_payload: signing_payload.to_string(),
        task_id,
        transfer_id,
    })
}

fn normalize_asset_transfer_asset(value: &str) -> Result<&'static str, &'static str> {
    match value.trim().to_ascii_uppercase().as_str() {
        "AIC" => Ok("AIC"),
        "USD" => Ok("USD"),
        _ => Err("nni_asset_transfer_asset_invalid"),
    }
}

fn normalize_asset_transfer_authorization_mode(
    value: Option<&str>,
) -> Result<&'static str, &'static str> {
    match value
        .unwrap_or("delegated_hardware")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "delegated_hardware" => Ok("delegated_hardware"),
        "asset_owner" => Ok("asset_owner"),
        _ => Err("nni_asset_transfer_authorization_mode_invalid"),
    }
}

#[cfg(test)]
#[path = "nni_asset_transfer_history_tests.rs"]
mod nni_asset_transfer_history_tests;

#[cfg(test)]
mod nni_asset_transfer_tests {
    use super::*;

    #[test]
    fn validates_exact_asset_transfer_signing_contract() {
        let owner = generate_nni_owner_key_pair();
        let recipient = generate_nni_owner_key_pair();
        let expires = current_unix_ts() + 120;
        let payload = json!({
            "schema_version": 2,
            "action": "nni_asset_transfer",
            "server_identity": "nni-server-v1",
            "transfer_id": "asset-transfer-test",
            "task_id": "nni-asset-transfer-test",
            "device_pubkey": "aa".repeat(64),
            "from_asset_owner_pubkey": owner.public_key,
            "to_asset_owner_pubkey": recipient.public_key,
            "authorization_epoch": 1,
            "authorization_mode": "delegated_hardware",
            "asset": "AIC",
            "amount_units": "100000000",
            "memo": "device order #42",
            "nonce": "bb".repeat(16),
            "expires_at_unix": expires,
        })
        .to_string();
        let digest = format!("{:x}", Sha256::digest(payload.as_bytes()));
        let response = json!({
            "request_id": "0c42e3f7-f5f0-43ff-bc55-ab032daf7eaf",
            "task_id": "nni-asset-transfer-test",
            "transfer_id": "asset-transfer-test",
            "device_pubkey": "aa".repeat(64),
            "from_asset_owner_pubkey": owner.public_key,
            "to_asset_owner_pubkey": recipient.public_key,
            "authorization_epoch": 1,
            "authorization_mode": "delegated_hardware",
            "asset": "AIC",
            "amount_units": "100000000",
            "memo": "device order #42",
            "expires_at_unix": expires,
            "signing_payload": payload,
            "signing_payload_digest": digest,
        });
        assert!(validate_asset_transfer_signing_payload(
            &response,
            Some(&"aa".repeat(64)),
            response["from_asset_owner_pubkey"].as_str().unwrap(),
            response["to_asset_owner_pubkey"].as_str().unwrap(),
            "delegated_hardware",
            "AIC",
            "100000000",
            "device order #42",
            "0c42e3f7-f5f0-43ff-bc55-ab032daf7eaf",
        )
        .is_ok());
        assert_eq!(
            validate_asset_transfer_signing_payload(
                &response,
                Some(&"aa".repeat(64)),
                response["from_asset_owner_pubkey"].as_str().unwrap(),
                response["to_asset_owner_pubkey"].as_str().unwrap(),
                "delegated_hardware",
                "AIC",
                "100000000",
                "device order #42",
                "89f1d6c2-82d7-48c7-922c-d72d102dbf36",
            )
            .unwrap_err(),
            "nni_asset_transfer_request_id_mismatch",
        );
    }

    #[test]
    fn rejects_recipient_or_amount_substitution_before_signing() {
        let owner = generate_nni_owner_key_pair();
        let recipient = generate_nni_owner_key_pair();
        let other = generate_nni_owner_key_pair();
        let expires = current_unix_ts() + 120;
        let payload = json!({
            "schema_version": 2,
            "action": "nni_asset_transfer",
            "server_identity": "nni-server-v1",
            "transfer_id": "asset-transfer-test",
            "task_id": "nni-asset-transfer-test",
            "device_pubkey": "aa".repeat(64),
            "from_asset_owner_pubkey": owner.public_key,
            "to_asset_owner_pubkey": recipient.public_key,
            "authorization_epoch": 1,
            "authorization_mode": "delegated_hardware",
            "asset": "USD",
            "amount_units": "100000000",
            "memo": "invoice-7",
            "nonce": "bb".repeat(16),
            "expires_at_unix": expires,
        })
        .to_string();
        let response = json!({
            "request_id": "0c42e3f7-f5f0-43ff-bc55-ab032daf7eaf",
            "task_id": "nni-asset-transfer-test",
            "transfer_id": "asset-transfer-test",
            "device_pubkey": "aa".repeat(64),
            "from_asset_owner_pubkey": owner.public_key,
            "to_asset_owner_pubkey": recipient.public_key,
            "authorization_epoch": 1,
            "authorization_mode": "delegated_hardware",
            "asset": "USD",
            "amount_units": "100000000",
            "memo": "invoice-7",
            "expires_at_unix": expires,
            "signing_payload_digest": format!("{:x}", Sha256::digest(payload.as_bytes())),
            "signing_payload": payload,
        });
        assert_eq!(
            validate_asset_transfer_signing_payload(
                &response,
                Some(&"aa".repeat(64)),
                response["from_asset_owner_pubkey"].as_str().unwrap(),
                &other.public_key,
                "delegated_hardware",
                "USD",
                "100000000",
                "invoice-7",
                "0c42e3f7-f5f0-43ff-bc55-ab032daf7eaf",
            )
            .unwrap_err(),
            "nni_asset_transfer_signing_payload_binding_invalid",
        );
        assert_eq!(
            validate_asset_transfer_signing_payload(
                &response,
                Some(&"aa".repeat(64)),
                response["from_asset_owner_pubkey"].as_str().unwrap(),
                response["to_asset_owner_pubkey"].as_str().unwrap(),
                "delegated_hardware",
                "USD",
                "200000000",
                "invoice-7",
                "0c42e3f7-f5f0-43ff-bc55-ab032daf7eaf",
            )
            .unwrap_err(),
            "nni_asset_transfer_signing_payload_binding_invalid",
        );
        assert_eq!(
            validate_asset_transfer_signing_payload(
                &response,
                Some(&"aa".repeat(64)),
                response["from_asset_owner_pubkey"].as_str().unwrap(),
                response["to_asset_owner_pubkey"].as_str().unwrap(),
                "delegated_hardware",
                "USD",
                "100000000",
                "invoice-8",
                "0c42e3f7-f5f0-43ff-bc55-ab032daf7eaf",
            )
            .unwrap_err(),
            "nni_asset_transfer_signing_payload_binding_invalid",
        );
    }
}
