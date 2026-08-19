#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum InternalNniAction {
    Status,
    DeviceStatus,
    HeartbeatStatus,
    HeartbeatEnable,
    HeartbeatDisable,
    HeartbeatNow,
    NetworkStats,
    MyRewards,
    BancorMarket,
    BancorAccount,
    BancorMarketTrades,
    BancorCandles,
    BancorQuote,
}

impl InternalNniAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::DeviceStatus => "device_status",
            Self::HeartbeatStatus => "heartbeat_status",
            Self::HeartbeatEnable => "heartbeat_enable",
            Self::HeartbeatDisable => "heartbeat_disable",
            Self::HeartbeatNow => "heartbeat_now",
            Self::NetworkStats => "network_stats",
            Self::MyRewards => "my_rewards",
            Self::BancorMarket => "bancor_market",
            Self::BancorAccount => "bancor_account",
            Self::BancorMarketTrades => "bancor_market_trades",
            Self::BancorCandles => "bancor_candles",
            Self::BancorQuote => "bancor_quote",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InternalNniActionRequest {
    action: InternalNniAction,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    interval: Option<String>,
    #[serde(default)]
    end_time_ts: Option<i64>,
    #[serde(default)]
    side: Option<String>,
    #[serde(default)]
    pay_asset: Option<String>,
    #[serde(default)]
    pay_amount: Option<String>,
    #[serde(default)]
    slippage_bps: Option<u16>,
}

fn validate_internal_nni_action_request(
    request: &InternalNniActionRequest,
) -> Result<(), NniSkillDomainError> {
    let mut supplied = Vec::new();
    if request.limit.is_some() {
        supplied.push("limit");
    }
    if request.interval.is_some() {
        supplied.push("interval");
    }
    if request.end_time_ts.is_some() {
        supplied.push("end_time_ts");
    }
    if request.side.is_some() {
        supplied.push("side");
    }
    if request.pay_asset.is_some() {
        supplied.push("pay_asset");
    }
    if request.pay_amount.is_some() {
        supplied.push("pay_amount");
    }
    if request.slippage_bps.is_some() {
        supplied.push("slippage_bps");
    }
    let allowed: &[&str] = match request.action {
        InternalNniAction::MyRewards
        | InternalNniAction::BancorAccount
        | InternalNniAction::BancorMarketTrades => &["limit"],
        InternalNniAction::BancorCandles => &["limit", "interval", "end_time_ts"],
        InternalNniAction::BancorQuote => {
            &["side", "pay_asset", "pay_amount", "slippage_bps"]
        }
        _ => &[],
    };
    let invalid: Vec<&str> = supplied
        .into_iter()
        .filter(|field| !allowed.contains(field))
        .collect();
    if invalid.is_empty() {
        Ok(())
    } else {
        Err(
            NniSkillDomainError::new(
                StatusCode::BAD_REQUEST,
                "nni_argument_invalid",
                false,
                json!({"action": request.action.as_str(), "invalid_fields": invalid}),
            )
            .pre_dispatch(Some("replan_arguments")),
        )
    }
}

#[derive(Debug)]
struct NniSkillDomainError {
    status: StatusCode,
    error_code: String,
    retryable: bool,
    details: Value,
    failure_phase: Option<&'static str>,
    side_effect_applied: Option<bool>,
    recovery_action: Option<&'static str>,
}

impl NniSkillDomainError {
    fn new(
        status: StatusCode,
        error_code: impl Into<String>,
        retryable: bool,
        details: Value,
    ) -> Self {
        Self {
            status,
            error_code: error_code.into(),
            retryable,
            details,
            failure_phase: Some("execution_no_effect"),
            side_effect_applied: Some(false),
            recovery_action: None,
        }
    }

    fn pre_dispatch(mut self, recovery_action: Option<&'static str>) -> Self {
        self.failure_phase = Some("pre_dispatch");
        self.recovery_action = recovery_action;
        self
    }

    fn provider_rejected(mut self) -> Self {
        self.failure_phase = Some("provider_rejected");
        self.recovery_action = None;
        self
    }

    fn uncertain(mut self) -> Self {
        self.failure_phase = None;
        self.side_effect_applied = None;
        self.recovery_action = Some("reconcile_before_retry");
        self
    }
}

fn nni_skill_error_response(
    action: InternalNniAction,
    error: NniSkillDomainError,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let error_code = error.error_code;
    let mut details = sanitize_nni_skill_data(error.details);
    nni_skill_enrich_utc_timestamps(&mut details);
    (
        error.status,
        Json(ApiResponse {
            ok: false,
            data: Some(json!({
                "schema_version": 1,
                "source_skill": "nni",
                "status": "error",
                "action": action.as_str(),
                "error_code": error_code,
                "message_key": format!("skill.nni.{error_code}"),
                "retryable": error.retryable,
                "failure_phase": error.failure_phase,
                "side_effect_applied": error.side_effect_applied,
                "recovery_action": error.recovery_action,
                "details": details,
            })),
            error: Some(error_code),
        }),
    )
}

fn nni_skill_ok_response(
    action: InternalNniAction,
    data: Value,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let observed_at_ts = current_unix_ts();
    let mut data = sanitize_nni_skill_data(data);
    nni_skill_enrich_utc_timestamps(&mut data);
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "schema_version": 1,
                "source_skill": "nni",
                "status": "ok",
                "action": action.as_str(),
                "observed_at_ts": observed_at_ts,
                "observed_at_utc": nni_skill_utc_timestamp(observed_at_ts),
                "data": data,
            })),
            error: None,
        }),
    )
}

fn nni_skill_limit(limit: Option<usize>, default: usize, maximum: usize) -> Result<usize, NniSkillDomainError> {
    let limit = limit.unwrap_or(default);
    if !(1..=maximum).contains(&limit) {
        return Err(
            NniSkillDomainError::new(
                StatusCode::BAD_REQUEST,
                "nni_argument_invalid",
                false,
                json!({"field": "limit", "minimum": 1, "maximum": maximum}),
            )
            .pre_dispatch(Some("replan_arguments")),
        );
    }
    Ok(limit)
}

fn nni_skill_interval_seconds(interval: Option<&str>) -> Result<u64, NniSkillDomainError> {
    let token = interval.unwrap_or("5m").trim().to_ascii_lowercase();
    match token.as_str() {
        "1m" => Ok(60),
        "5m" => Ok(300),
        "15m" => Ok(900),
        "1h" => Ok(3_600),
        "4h" => Ok(14_400),
        "1d" => Ok(86_400),
        "1w" => Ok(604_800),
        "1y" => Ok(31_536_000),
        _ => Err(
            NniSkillDomainError::new(
                StatusCode::BAD_REQUEST,
                "nni_argument_invalid",
                false,
                json!({
                    "field": "interval",
                    "allowed": ["1m", "5m", "15m", "1h", "4h", "1d", "1w", "1y"],
                }),
            )
            .pre_dispatch(Some("replan_arguments")),
        ),
    }
}

fn nni_skill_device_projection(snapshot: &Value) -> Value {
    let signer_kind = snapshot
        .get("signer_kind")
        .cloned()
        .unwrap_or_else(|| Value::String("unavailable".to_string()));
    let simulation_enabled = signer_kind.as_str() == Some("simulated");
    json!({
        "helper_available": snapshot.get("helper_available").cloned().unwrap_or(Value::Bool(false)),
        "hardware_chip_present": snapshot.get("hardware_chip_present").cloned().unwrap_or(Value::Bool(false)),
        "signer_available": snapshot.get("signer_available").cloned().unwrap_or(Value::Bool(false)),
        "local_participation_eligible": snapshot.get("local_participation_eligible").cloned().unwrap_or(Value::Bool(false)),
        "signer_kind": signer_kind,
        "network_authorization": snapshot.get("network_authorization").cloned().unwrap_or_else(|| Value::String("unknown".to_string())),
        "simulation_enabled": simulation_enabled,
        "simulation_enable_available": snapshot.get("simulation_available").cloned().unwrap_or(Value::Bool(false)),
        "pubkey_preview": snapshot.get("pubkey_preview").cloned().unwrap_or(Value::Null),
        "pubkey_fingerprint": snapshot.get("pubkey_fingerprint").cloned().unwrap_or(Value::Null),
        "status": snapshot.get("status").cloned().unwrap_or_else(|| Value::String("unavailable".to_string())),
    })
}

fn nni_skill_heartbeat_projection(config: &NniConfigResponse) -> Value {
    json!({
        "desired_enabled": config.joined,
        "effective_state": config.heartbeat_state,
        "worker_running": config.worker_running,
        "heartbeat_interval_seconds": config.heartbeat_interval_seconds,
        "request_count": config.heartbeat_request_count,
        "last_attempt_at_ts": config.last_heartbeat_attempt_at_ts,
        "last_success_at_ts": config.last_heartbeat_at_ts,
        "next_due_at_ts": config.next_heartbeat_due_at_ts,
        "consecutive_failures": config.consecutive_heartbeat_failures,
        "network_failure_count": config.last_heartbeat_network_failures,
        "last_error_code": config.last_heartbeat_error_code,
        "last_error_at_ts": config.last_heartbeat_error_at_ts,
        "selected_node_count": usize::from(config.selected_node_url.is_some()),
        "selected_node_url": config.selected_node_url,
        "last_success_node_host": config.last_success_node_host,
        "network_authorization": config.network_authorization,
    })
}

fn nni_skill_require_signer(snapshot: &Value) -> Result<(), NniSkillDomainError> {
    if snapshot
        .get("signer_available")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(());
    }
    let helper_available = snapshot
        .get("helper_available")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Err(
        NniSkillDomainError::new(
            StatusCode::PRECONDITION_FAILED,
            if helper_available {
                "nni_signature_device_unavailable"
            } else {
                "nni_signature_helper_unavailable"
            },
            false,
            nni_skill_device_projection(snapshot),
        )
        .pre_dispatch(Some("configure_device_signer")),
    )
}

fn nni_skill_attempts_error(
    error_code: &'static str,
    attempts: Vec<Value>,
) -> NniSkillDomainError {
    let authorization_rejected = !attempts.is_empty()
        && attempts.iter().all(|attempt| {
            attempt
                .get("error_code")
                .and_then(Value::as_str)
                .is_some_and(nni_heartbeat_error_is_authorization_rejection)
        });
    let retryable = attempts.iter().any(nni_skill_attempt_is_retryable);
    let error = NniSkillDomainError::new(
        if authorization_rejected {
            StatusCode::PRECONDITION_FAILED
        } else {
            StatusCode::BAD_GATEWAY
        },
        if authorization_rejected {
            "nni_device_not_authorized"
        } else {
            error_code
        },
        retryable,
        json!({"attempts": attempts}),
    );
    if authorization_rejected {
        error.provider_rejected()
    } else {
        error
    }
}

fn nni_skill_attempt_is_retryable(attempt: &Value) -> bool {
    if attempt.get("retryable").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    if attempt
        .get("http_status")
        .and_then(Value::as_u64)
        .is_some_and(|status| status == 429 || status >= 500)
    {
        return true;
    }
    matches!(
        attempt.get("error_code").and_then(Value::as_str),
        Some(
            "nni_reward_request_network_failed"
                | "nni_reward_verify_network_failed"
                | "nni_bancor_account_request_network_failed"
                | "nni_bancor_account_verify_network_failed"
                | "nni_remote_query_network_failed"
        )
    )
}

fn nni_skill_remote_error_code(response: &ApiResponse<Value>) -> String {
    nni_remote_api_error_code(response, "nni_remote_query_failed")
}

async fn nni_skill_rewards_data(
    state: &AppState,
    user_key: &str,
    limit: usize,
) -> Result<Value, NniSkillDomainError> {
    let config = read_nni_config(state).map_err(|error| {
        NniSkillDomainError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "nni_config_read_failed",
            false,
            json!({"detail": error.to_string()}),
        )
    })?;
    if nni_selected_remote_node(&config).is_none() {
        return Err(NniSkillDomainError::new(
            StatusCode::PRECONDITION_FAILED,
            "nni_remote_node_unconfigured",
            false,
            json!({"selected_node_count": 0}),
        ));
    }
    let device_pubkey = nni_device_pubkey(state)
        .await
        .map_err(|(status, error, data)| NniSkillDomainError::new(status, error, false, data))?;
    let mut attempts = Vec::new();
    for node_url in nni_selected_remote_nodes(&config) {
        match query_nni_rewards_for_node(state, node_url, &device_pubkey, user_key, 1, limit).await {
            Ok(mut data) => {
                if let Some(object) = data.as_object_mut() {
                    object.insert("node_url".to_string(), Value::String(node_url.clone()));
                    let returned_count = object
                        .get("records")
                        .and_then(Value::as_array)
                        .map(Vec::len)
                        .unwrap_or_default();
                    object.insert("returned_count".to_string(), json!(returned_count));
                    object.insert(
                        "truncated".to_string(),
                        json!(nni_skill_reward_result_truncated(object)),
                    );
                }
                return Ok(data);
            }
            Err(error) => attempts.push(error),
        }
    }
    Err(nni_skill_attempts_error("nni_rewards_query_failed", attempts))
}

fn nni_skill_reward_result_truncated(object: &serde_json::Map<String, Value>) -> bool {
    let page = object.get("page").and_then(Value::as_u64).unwrap_or(1);
    let total_pages = object
        .get("total_pages")
        .and_then(Value::as_u64)
        .unwrap_or(page);
    let history_truncated = object
        .get("history_truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    page < total_pages || history_truncated
}

async fn nni_skill_bancor_account_data(
    state: &AppState,
    user_key: &str,
    limit: usize,
) -> Result<Value, NniSkillDomainError> {
    let config = read_nni_config(state).map_err(|error| {
        NniSkillDomainError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "nni_config_read_failed",
            false,
            json!({"detail": error.to_string()}),
        )
    })?;
    if nni_selected_remote_node(&config).is_none() {
        return Err(NniSkillDomainError::new(
            StatusCode::PRECONDITION_FAILED,
            "nni_remote_node_unconfigured",
            false,
            json!({"selected_node_count": 0}),
        ));
    }
    let device_pubkey = nni_device_pubkey(state)
        .await
        .map_err(|(status, error, data)| NniSkillDomainError::new(status, error, false, data))?;
    let mut attempts = Vec::new();
    for node_url in nni_selected_remote_nodes(&config) {
        match query_nni_bancor_account_for_node(
            state,
            node_url,
            &device_pubkey,
            user_key,
            1,
            limit,
        )
        .await
        {
            Ok(mut data) => {
                if let Some(object) = data.as_object_mut() {
                    object.insert("node_url".to_string(), Value::String(node_url.clone()));
                    let returned_count = object
                        .get("trades")
                        .and_then(Value::as_array)
                        .map(Vec::len)
                        .unwrap_or_default();
                    object.insert("returned_count".to_string(), json!(returned_count));
                    object.insert("truncated".to_string(), json!(returned_count >= limit));
                }
                return Ok(data);
            }
            Err(error) => attempts.push(error),
        }
    }
    Err(nni_skill_attempts_error("nni_bancor_query_failed", attempts))
}

async fn nni_skill_public_node_data(
    state: &AppState,
    path: &str,
    body: Option<&Value>,
) -> Result<Value, NniSkillDomainError> {
    let config = read_nni_config(state).map_err(|error| {
        NniSkillDomainError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "nni_config_read_failed",
            false,
            json!({"detail": error.to_string()}),
        )
    })?;
    if nni_selected_remote_node(&config).is_none() {
        return Err(NniSkillDomainError::new(
            StatusCode::PRECONDITION_FAILED,
            "nni_remote_node_unconfigured",
            false,
            json!({"selected_node_count": 0}),
        ));
    }
    let mut attempts = Vec::new();
    for node_url in nni_selected_remote_nodes(&config) {
        let endpoint = nni_remote_api_endpoint(node_url, path);
        let request = if let Some(body) = body {
            state.core.http_client.post(&endpoint).json(body)
        } else {
            state.core.http_client.get(&endpoint)
        }
        .timeout(nni_remote_api_timeout());
        match request.send().await {
            Ok(response) => {
                let status = response.status();
                if response
                    .content_length()
                    .is_some_and(|size| size > NNI_SKILL_REMOTE_RESPONSE_MAX_BYTES as u64)
                {
                    attempts.push(json!({
                        "node_host": nni_node_host(node_url),
                        "http_status": status.as_u16(),
                        "error_code": "nni_response_too_large",
                    }));
                    continue;
                }
                let bytes = match response.bytes().await {
                    Ok(bytes) if bytes.len() <= NNI_SKILL_REMOTE_RESPONSE_MAX_BYTES => bytes,
                    Ok(_) => {
                        attempts.push(json!({
                            "node_host": nni_node_host(node_url),
                            "http_status": status.as_u16(),
                            "error_code": "nni_response_too_large",
                        }));
                        continue;
                    }
                    Err(error) => {
                        attempts.push(json!({
                            "node_host": nni_node_host(node_url),
                            "http_status": status.as_u16(),
                            "error_code": "nni_response_contract_invalid",
                            "detail": error.to_string(),
                        }));
                        continue;
                    }
                };
                match serde_json::from_slice::<ApiResponse<Value>>(&bytes) {
                    Ok(response) if status.is_success() && response.ok => {
                        if let Some(mut data) = response.data {
                            if let Some(object) = data.as_object_mut() {
                                object.insert("node_url".to_string(), Value::String(node_url.clone()));
                            }
                            return Ok(data);
                        }
                        attempts.push(json!({
                            "node_host": nni_node_host(node_url),
                            "error_code": "nni_response_contract_invalid",
                        }));
                    }
                    Ok(response) => {
                        let error_code = nni_skill_remote_error_code(&response);
                        attempts.push(json!({
                            "node_host": nni_node_host(node_url),
                            "http_status": status.as_u16(),
                            "error_code": error_code,
                        }));
                    }
                    Err(error) => attempts.push(json!({
                        "node_host": nni_node_host(node_url),
                        "http_status": status.as_u16(),
                        "error_code": "nni_response_contract_invalid",
                        "detail": error.to_string(),
                    })),
                }
            }
            Err(error) => attempts.push(json!({
                "node_host": nni_node_host(node_url),
                "error_code": "nni_remote_query_network_failed",
                "detail": error.to_string(),
            })),
        }
    }
    Err(nni_skill_attempts_error("nni_bancor_query_failed", attempts))
}

async fn nni_skill_network_stats_data(state: &AppState) -> Result<Value, NniSkillDomainError> {
    let config = read_nni_config(state).map_err(|error| {
        NniSkillDomainError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "nni_config_read_failed",
            false,
            json!({"detail": error.to_string()}),
        )
    })?;
    if nni_selected_remote_node(&config).is_none() {
        return Err(NniSkillDomainError::new(
            StatusCode::PRECONDITION_FAILED,
            "nni_remote_node_unconfigured",
            false,
            json!({"selected_node_count": 0}),
        ));
    }

    let mut attempts = Vec::new();
    for node_url in nni_selected_remote_nodes(&config) {
        match nni_remote_read_with_retry(|| query_nni_network_stats_for_node(state, node_url)).await {
            Ok(mut data) => {
                if let Some(object) = data.as_object_mut() {
                    object.insert("node_url".to_string(), Value::String(node_url.clone()));
                }
                return Ok(data);
            }
            Err(error) => attempts.push(error),
        }
    }
    Err(nni_skill_attempts_error(
        "nni_network_stats_query_failed",
        attempts,
    ))
}

fn sanitize_nni_skill_data(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(sanitize_nni_skill_data)
                .collect(),
        ),
        Value::Object(mut object) => {
            if let Some(node_url) = object.remove("node_url").and_then(|value| value.as_str().map(str::to_string)) {
                object.insert(
                    "node_host".to_string(),
                    nni_node_host(&node_url).map(Value::String).unwrap_or(Value::Null),
                );
            }
            let device_pubkey = object
                .remove("device_pubkey")
                .or_else(|| object.remove("local_device_pubkey"));
            object.remove("local_device_pubkey");
            if let Some(pubkey) = device_pubkey
                .and_then(|value| value.as_str().map(str::to_string))
            {
                object.insert(
                    "device_pubkey_fingerprint".to_string(),
                    nni_hex_fingerprint(&pubkey)
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                );
            }
            if let Some(pubkey) = object
                .remove("asset_owner_pubkey")
                .and_then(|value| value.as_str().map(str::to_string))
            {
                object.insert(
                    "asset_owner_pubkey_fingerprint".to_string(),
                    nni_hex_fingerprint(&pubkey)
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                );
            }
            if let Some(compact) = object
                .remove("device_pubkey_compact")
                .and_then(|value| value.as_str().map(str::to_string))
            {
                let preview = if compact.chars().count() > 18 {
                    format!(
                        "{}...{}",
                        compact.chars().take(10).collect::<String>(),
                        compact
                            .chars()
                            .rev()
                            .take(6)
                            .collect::<String>()
                            .chars()
                            .rev()
                            .collect::<String>()
                    )
                } else {
                    compact
                };
                object.insert("device_pubkey_preview".to_string(), Value::String(preview));
            }
            for key in [
                "signature",
                "challenge",
                "signing_payload",
                "helper_path",
                "config_path",
                "runtime_state_path",
                "log_path",
                "lib_path",
                "token",
            ] {
                object.remove(key);
            }
            Value::Object(
                object
                    .into_iter()
                    .map(|(key, value)| (key, sanitize_nni_skill_data(value)))
                    .collect(),
            )
        }
        scalar => scalar,
    }
}

fn nni_skill_utc_timestamp(timestamp: i64) -> Option<String> {
    chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp, 0).map(|value| {
        value.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    })
}

fn nni_skill_enrich_utc_timestamps(value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                nni_skill_enrich_utc_timestamps(value);
            }
        }
        Value::Object(object) => {
            let mut utc_fields = Vec::new();
            for (key, value) in object.iter_mut() {
                nni_skill_enrich_utc_timestamps(value);
                let utc_key = key
                    .strip_suffix("_unix")
                    .or_else(|| key.strip_suffix("_ts"))
                    .map(|prefix| format!("{prefix}_utc"));
                if let (Some(utc_key), Some(timestamp)) = (utc_key, value.as_i64()) {
                    if timestamp < 0 {
                        continue;
                    }
                    if let Some(timestamp) = nni_skill_utc_timestamp(timestamp) {
                        utc_fields.push((utc_key, Value::String(timestamp)));
                    }
                }
            }
            for (key, value) in utc_fields {
                object.entry(key).or_insert(value);
            }
        }
        _ => {}
    }
}

async fn nni_skill_heartbeat_enable(state: &AppState) -> Result<Value, NniSkillDomainError> {
    let device = nni_device_snapshot(state, false).await;
    nni_skill_require_signer(&device)?;
    let _guard = nni_heartbeat_operation_lock().lock().await;
    let existing = read_nni_config(state).map_err(|error| {
        NniSkillDomainError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "nni_config_read_failed",
            false,
            json!({"detail": error.to_string()}),
        )
    })?;
    if nni_selected_remote_node(&existing).is_none() {
        return Err(NniSkillDomainError::new(
            StatusCode::PRECONDITION_FAILED,
            "nni_remote_node_unconfigured",
            false,
            json!({"selected_node_count": 0}),
        ));
    }
    if existing.joined {
        return Ok(json!({
            "changed": false,
            "device": nni_skill_device_projection(&device),
            "heartbeat": nni_skill_heartbeat_projection(&existing),
        }));
    }
    let enabled = write_nni_config(state, None, Some(true)).map_err(|error| {
        NniSkillDomainError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "nni_heartbeat_state_write_failed",
            false,
            json!({"detail": error.to_string()}),
        )
    })?;
    let selected_nodes = enabled.selected_node_url.iter().cloned().collect::<Vec<_>>();
    match nni_recorded_heartbeat(state, &selected_nodes).await {
        Ok(_) => {
            let current = read_nni_config(state).map_err(|error| {
                NniSkillDomainError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "nni_config_read_failed",
                    false,
                    json!({"detail": error.to_string()}),
                )
            })?;
            Ok(json!({
                "changed": true,
                "device": nni_skill_device_projection(&device),
                "heartbeat": nni_skill_heartbeat_projection(&current),
            }))
        }
        Err(error) if error.network => {
            let current = read_nni_config(state).map_err(|read_error| {
                NniSkillDomainError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "nni_config_read_failed",
                    false,
                    json!({"detail": read_error.to_string()}),
                )
            })?;
            Ok(json!({
                "changed": true,
                "waiting_for_retry": true,
                "device": nni_skill_device_projection(&device),
                "heartbeat": nni_skill_heartbeat_projection(&current),
            }))
        }
        Err(error) => {
            let rollback = write_nni_config(state, None, Some(false));
            let authorization_rejected =
                nni_heartbeat_error_is_authorization_rejection(&error.code);
            let domain_error = NniSkillDomainError::new(
                StatusCode::PRECONDITION_FAILED,
                if authorization_rejected {
                    "nni_device_not_authorized"
                } else {
                    "nni_heartbeat_request_failed"
                },
                false,
                json!({
                    "runtime_error_code": error.code,
                    "device": nni_skill_device_projection(&device),
                    "rollback_error": rollback.as_ref().err().map(ToString::to_string),
                }),
            );
            if rollback.is_err() {
                Err(domain_error.uncertain())
            } else if authorization_rejected {
                Err(domain_error.provider_rejected())
            } else {
                Err(domain_error)
            }
        }
    }
}

async fn nni_skill_heartbeat_disable(state: &AppState) -> Result<Value, NniSkillDomainError> {
    let _guard = nni_heartbeat_operation_lock().lock().await;
    let existing = read_nni_config(state).map_err(|error| {
        NniSkillDomainError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "nni_config_read_failed",
            false,
            json!({"detail": error.to_string()}),
        )
    })?;
    let changed = existing.joined;
    let current = write_nni_config(state, None, Some(false)).map_err(|error| {
        NniSkillDomainError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "nni_heartbeat_state_write_failed",
            false,
            json!({"detail": error.to_string()}),
        )
    })?;
    Ok(json!({
        "changed": changed,
        "heartbeat": nni_skill_heartbeat_projection(&current),
    }))
}

async fn nni_skill_heartbeat_now(state: &AppState) -> Result<Value, NniSkillDomainError> {
    let device = nni_device_snapshot(state, false).await;
    nni_skill_require_signer(&device)?;
    let _guard = nni_heartbeat_operation_lock().lock().await;
    let config = read_nni_config(state).map_err(|error| {
        NniSkillDomainError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "nni_config_read_failed",
            false,
            json!({"detail": error.to_string()}),
        )
    })?;
    if nni_selected_remote_node(&config).is_none() {
        return Err(NniSkillDomainError::new(
            StatusCode::PRECONDITION_FAILED,
            "nni_remote_node_unconfigured",
            false,
            json!({"selected_node_count": 0}),
        ));
    }
    if config.last_heartbeat_attempt_at_ts.is_some_and(|last| {
        u64::try_from(current_unix_ts())
            .unwrap_or_default()
            .saturating_sub(last)
            < 30
    }) {
        return Err(NniSkillDomainError::new(
            StatusCode::CONFLICT,
            "nni_operation_in_progress",
            true,
            json!({"retry_after_seconds": 30}),
        ));
    }
    let selected_nodes = config.selected_node_url.iter().cloned().collect::<Vec<_>>();
    nni_recorded_heartbeat(state, &selected_nodes)
        .await
        .map_err(|error| {
            let effect_is_uncertain = matches!(
                error.code.as_str(),
                "heartbeat_verify_network_failed"
                    | "heartbeat_verify_body_failed"
                    | "heartbeat_verify_missing_data"
                    | "nni_heartbeat_state_write_failed"
            );
            let authorization_rejected =
                nni_heartbeat_error_is_authorization_rejection(&error.code);
            let domain_error = NniSkillDomainError::new(
                if error.network {
                    StatusCode::BAD_GATEWAY
                } else {
                    StatusCode::PRECONDITION_FAILED
                },
                if error.network {
                    "nni_heartbeat_network_unavailable"
                } else if nni_heartbeat_error_is_authorization_rejection(&error.code) {
                    "nni_device_not_authorized"
                } else {
                    "nni_heartbeat_request_failed"
                },
                error.network,
                json!({"runtime_error_code": error.code}),
            );
            if effect_is_uncertain {
                domain_error.uncertain()
            } else if authorization_rejected {
                domain_error.provider_rejected()
            } else {
                domain_error
            }
        })?;
    let current = read_nni_config(state).map_err(|error| {
        NniSkillDomainError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "nni_config_read_failed",
            false,
            json!({"detail": error.to_string()}),
        )
    })?;
    Ok(json!({
        "device": nni_skill_device_projection(&device),
        "heartbeat": nni_skill_heartbeat_projection(&current),
    }))
}

async fn execute_internal_nni_action(
    state: &AppState,
    token_ctx: &InternalSkillTokenContext,
    request: &InternalNniActionRequest,
) -> Result<Value, NniSkillDomainError> {
    let user_key = token_ctx
        .user_key
        .clone()
        .unwrap_or_else(|| format!("nni-task:{}", token_ctx.task_id));
    match request.action {
        InternalNniAction::Status => {
            let device = nni_device_snapshot(state, false).await;
            let config = read_nni_config(state).map_err(|error| {
                NniSkillDomainError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "nni_config_read_failed",
                    false,
                    json!({"detail": error.to_string()}),
                )
            })?;
            Ok(json!({
                "device": nni_skill_device_projection(&device),
                "heartbeat": nni_skill_heartbeat_projection(&config),
            }))
        }
        InternalNniAction::DeviceStatus => {
            Ok(nni_skill_device_projection(&nni_device_snapshot(state, false).await))
        }
        InternalNniAction::HeartbeatStatus => {
            let config = read_nni_config(state).map_err(|error| {
                NniSkillDomainError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "nni_config_read_failed",
                    false,
                    json!({"detail": error.to_string()}),
                )
            })?;
            Ok(nni_skill_heartbeat_projection(&config))
        }
        InternalNniAction::HeartbeatEnable => nni_skill_heartbeat_enable(state).await,
        InternalNniAction::HeartbeatDisable => nni_skill_heartbeat_disable(state).await,
        InternalNniAction::HeartbeatNow => nni_skill_heartbeat_now(state).await,
        InternalNniAction::NetworkStats => nni_skill_network_stats_data(state).await,
        InternalNniAction::MyRewards => {
            let device = nni_device_snapshot(state, false).await;
            nni_skill_require_signer(&device)?;
            nni_skill_rewards_data(
                state,
                &user_key,
                nni_skill_limit(request.limit, 20, 100)?,
            )
            .await
        }
        InternalNniAction::BancorMarket => {
            let data = nni_skill_public_node_data(
                state,
                "bancor/market",
                None,
            )
            .await?;
            validate_bancor_market_response(&data).map_err(|error| {
                NniSkillDomainError::new(
                    StatusCode::BAD_GATEWAY,
                    error,
                    false,
                    json!({"response_status": data.get("status")}),
                )
            })?;
            Ok(data)
        }
        InternalNniAction::BancorAccount => {
            let device = nni_device_snapshot(state, false).await;
            nni_skill_require_signer(&device)?;
            nni_skill_bancor_account_data(
                state,
                &user_key,
                nni_skill_limit(request.limit, 20, 100)?,
            )
            .await
        }
        InternalNniAction::BancorMarketTrades => {
            let limit = nni_skill_limit(request.limit, 100, 100)?;
            let mut data = nni_skill_public_node_data(
                state,
                "bancor/trades",
                None,
            )
            .await?;
            normalize_bancor_market_trades(&mut data);
            if let Some(object) = data.as_object_mut() {
                let counts = object
                    .get_mut("trades")
                    .and_then(Value::as_array_mut)
                    .map(|trades| {
                    let total_available = trades.len();
                    trades.truncate(limit);
                        (trades.len(), total_available > trades.len())
                    });
                if let Some((returned_count, truncated)) = counts {
                    object.insert("returned_count".to_string(), json!(returned_count));
                    object.insert("truncated".to_string(), json!(truncated));
                }
            }
            Ok(data)
        }
        InternalNniAction::BancorCandles => {
            let interval_seconds = nni_skill_interval_seconds(request.interval.as_deref())?;
            let limit = nni_skill_limit(request.limit, 120, 300)?;
            if request.end_time_ts.is_some_and(|value| value < 0) {
                return Err(NniSkillDomainError::new(
                    StatusCode::BAD_REQUEST,
                    "nni_argument_invalid",
                    false,
                    json!({"field": "end_time_ts", "minimum": 0}),
                ));
            }
            let mut path = format!(
                "bancor/candles?interval_seconds={interval_seconds}&limit={limit}"
            );
            if let Some(end_time_ts) = request.end_time_ts {
                path.push_str(&format!("&end_time_unix={end_time_ts}"));
            }
            let data = nni_skill_public_node_data(state, &path, None).await?;
            validate_bancor_candles_response(&data, interval_seconds, limit).map_err(|error| {
                NniSkillDomainError::new(
                    StatusCode::BAD_GATEWAY,
                    error,
                    false,
                    json!({"interval_seconds": interval_seconds, "limit": limit}),
                )
            })?;
            Ok(data)
        }
        InternalNniAction::BancorQuote => {
            let side = request.side.as_deref().ok_or_else(|| {
                NniSkillDomainError::new(
                    StatusCode::BAD_REQUEST,
                    "nni_argument_invalid",
                    false,
                    json!({"missing_fields": ["side"]}),
                )
            })?;
            let side = normalize_bancor_side(side).map_err(|error| {
                NniSkillDomainError::new(StatusCode::BAD_REQUEST, error, false, json!({"field": "side"}))
            })?;
            let expected_asset = if side == "buy" { "USD" } else { "POINT" };
            if request
                .pay_asset
                .as_deref()
                .is_some_and(|asset| !asset.eq_ignore_ascii_case(expected_asset))
            {
                return Err(NniSkillDomainError::new(
                    StatusCode::BAD_REQUEST,
                    "nni_argument_invalid",
                    false,
                    json!({"field": "pay_asset", "expected": expected_asset}),
                ));
            }
            let amount = request.pay_amount.as_deref().ok_or_else(|| {
                NniSkillDomainError::new(
                    StatusCode::BAD_REQUEST,
                    "nni_argument_invalid",
                    false,
                    json!({"missing_fields": ["pay_amount"]}),
                )
            })?;
            let (amount, _) = normalize_bancor_amount(amount).map_err(|error| {
                NniSkillDomainError::new(
                    StatusCode::BAD_REQUEST,
                    error,
                    false,
                    json!({"field": "pay_amount"}),
                )
            })?;
            let slippage_bps = normalize_bancor_slippage_bps(request.slippage_bps).map_err(|error| {
                NniSkillDomainError::new(
                    StatusCode::BAD_REQUEST,
                    error,
                    false,
                    json!({"field": "slippage_bps"}),
                )
            })?;
            let body = serde_json::to_value(NniBancorQuoteRequest {
                side: side.to_string(),
                input_amount: amount,
                slippage_bps: Some(slippage_bps),
            })
            .map_err(|error| {
                NniSkillDomainError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "nni_argument_invalid",
                    false,
                    json!({"detail": error.to_string()}),
                )
            })?;
            nni_skill_public_node_data(
                state,
                "bancor/quote",
                Some(&body),
            )
            .await
        }
    }
}

async fn internal_nni_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<InternalNniActionRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let token_ctx = match redeem_internal_skill_token(&headers) {
        Ok(context) => context,
        Err(response) => return response,
    };
    if token_ctx.skill_name != "nni" {
        return nni_skill_error_response(
            request.action,
            NniSkillDomainError::new(
                StatusCode::FORBIDDEN,
                "nni_internal_gateway_unauthorized",
                false,
                json!({"token_skill": token_ctx.skill_name}),
            ),
        );
    }
    if let Err(error) = validate_internal_nni_action_request(&request) {
        return nni_skill_error_response(request.action, error);
    }

    let previous_heartbeat_desired = matches!(
        request.action,
        InternalNniAction::HeartbeatEnable
            | InternalNniAction::HeartbeatDisable
            | InternalNniAction::HeartbeatNow
    )
    .then(|| read_nni_config(&state).ok().map(|config| config.joined))
    .flatten();

    append_nni_log_event_best_effort(
        &state,
        "skill_action",
        json!({
            "action": request.action.as_str(),
            "task_id": token_ctx.task_id,
            "channel": token_ctx.channel,
        }),
    );
    let result = execute_internal_nni_action(&state, &token_ctx, &request).await;
    let current_heartbeat_desired = previous_heartbeat_desired
        .is_some()
        .then(|| read_nni_config(&state).ok().map(|config| config.joined))
        .flatten();
    append_nni_log_event_best_effort(
        &state,
        "skill_action_result",
        json!({
            "action": request.action.as_str(),
            "task_id": token_ctx.task_id,
            "user_id": token_ctx.user_id,
            "chat_id": token_ctx.chat_id,
            "channel": token_ctx.channel,
            "status": if result.is_ok() { "ok" } else { "error" },
            "error_code": result.as_ref().err().map(|error| error.error_code.as_str()),
            "previous_heartbeat_desired": previous_heartbeat_desired,
            "current_heartbeat_desired": current_heartbeat_desired,
        }),
    );
    match result {
        Ok(data) => nni_skill_ok_response(request.action, data),
        Err(error) => nni_skill_error_response(request.action, error),
    }
}
const NNI_SKILL_REMOTE_RESPONSE_MAX_BYTES: usize = 2 * 1024 * 1024;
