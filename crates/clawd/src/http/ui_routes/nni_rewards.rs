#[derive(Debug, Serialize)]
struct NniRemoteRewardQueryRequest {
    device_pubkey: String,
    client_user_key: String,
}

#[derive(Debug, Serialize)]
struct NniRemoteRewardQueryVerifyRequest {
    task_id: String,
    signature: String,
    page: usize,
    per_page: usize,
}

fn nni_network_stats_decimal(value: &Value) -> bool {
    let Some(value) = value.as_str() else {
        return false;
    };
    let Some((whole, fraction)) = value.split_once('.') else {
        return false;
    };
    !whole.is_empty()
        && whole.bytes().all(|byte| byte.is_ascii_digit())
        && fraction.len() == 8
        && fraction.bytes().all(|byte| byte.is_ascii_digit())
}

fn nni_network_stats_integer_string(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
}

fn nni_network_stats_optional_unix(value: Option<&Value>) -> bool {
    value.is_some_and(|value| value.is_null() || value.as_i64().is_some_and(|unix| unix >= 0))
}

fn validate_nni_network_stats_sections(
    root: &serde_json::Map<String, Value>,
) -> Result<(), &'static str> {
    let devices = root
        .get("network_devices")
        .and_then(Value::as_object)
        .ok_or("nni_network_stats_contract_invalid")?;
    let policy = root
        .get("reward_policy")
        .and_then(Value::as_object)
        .ok_or("nni_network_stats_contract_invalid")?;
    let rewards = root
        .get("network_rewards")
        .and_then(Value::as_object)
        .ok_or("nni_network_stats_contract_invalid")?;
    let phase_is_valid = policy
        .get("phase")
        .and_then(Value::as_str)
        .is_some_and(|phase| matches!(phase, "disabled" | "scheduled" | "active"));
    let current_reward_units_are_valid = policy
        .get("current_reward_pool_units")
        .is_some_and(|value| value.is_null() || nni_network_stats_integer_string(value));
    let halving_era_is_valid = policy
        .get("halving_era")
        .is_some_and(|value| value.is_null() || value.as_u64().is_some());

    if devices
        .get("registered_device_count")
        .and_then(Value::as_u64)
        .is_none()
        || devices
            .get("active_device_count")
            .and_then(Value::as_u64)
            .is_none()
        || devices
            .get("window_seconds")
            .and_then(Value::as_u64)
            .is_none_or(|seconds| seconds == 0)
        || !nni_network_stats_optional_unix(devices.get("active_period_start_unix"))
        || !nni_network_stats_optional_unix(devices.get("active_period_end_unix"))
        || !nni_network_stats_optional_unix(devices.get("first_heartbeat_unix"))
        || !phase_is_valid
        || policy
            .get("accepting_reward_heartbeats")
            .and_then(Value::as_bool)
            .is_none()
        || policy
            .get("reward_start_time_unix")
            .and_then(Value::as_i64)
            .is_none_or(|unix| unix < 0)
        || policy
            .get("starts_in_seconds")
            .and_then(Value::as_u64)
            .is_none()
        || !nni_network_stats_optional_unix(policy.get("first_settlement_at_unix"))
        || policy
            .get("interval_seconds")
            .and_then(Value::as_u64)
            .is_none_or(|seconds| seconds == 0)
        || policy
            .get("initial_reward_pool_aic")
            .and_then(Value::as_u64)
            .is_none_or(|aic| aic == 0)
        || !current_reward_units_are_valid
        || policy.get("distribution").and_then(Value::as_str)
            != Some("equal_per_eligible_device")
        || !policy
            .get("current_reward_pool_aic")
            .is_some_and(|value| value.is_null() || nni_network_stats_decimal(value))
        || !nni_network_stats_optional_unix(policy.get("halving_epoch_unix"))
        || policy
            .get("halving_interval_seconds")
            .and_then(Value::as_u64)
            .is_none_or(|seconds| seconds == 0)
        || !halving_era_is_valid
        || policy.get("rewards_ended").and_then(Value::as_bool).is_none()
        || !nni_network_stats_optional_unix(policy.get("next_halving_at_unix"))
        || !rewards
            .get("total_distributed_reward_units")
            .is_some_and(nni_network_stats_integer_string)
        || !rewards
            .get("total_distributed_reward_aic")
            .is_some_and(nni_network_stats_decimal)
        || rewards
            .get("settled_period_count")
            .and_then(Value::as_u64)
            .is_none()
        || !nni_network_stats_optional_unix(rewards.get("first_period_start_unix"))
        || !nni_network_stats_optional_unix(rewards.get("latest_period_end_unix"))
    {
        return Err("nni_network_stats_contract_invalid");
    }
    Ok(())
}

fn validate_nni_network_stats_response(data: &Value) -> Result<(), &'static str> {
    let root = data
        .as_object()
        .ok_or("nni_network_stats_contract_invalid")?;
    if root.get("schema_version").and_then(Value::as_u64) != Some(1)
        || root.get("status").and_then(Value::as_str) != Some("heartbeat_network_stats")
        || root.contains_key("device_pubkey")
        || root.contains_key("records")
    {
        return Err("nni_network_stats_contract_invalid");
    }
    validate_nni_network_stats_sections(root)
}

fn validate_nni_rewards_response(data: &Value) -> Result<(), &'static str> {
    let root = data
        .as_object()
        .ok_or("nni_rewards_contract_invalid")?;
    validate_nni_network_stats_sections(root).map_err(|_| "nni_rewards_contract_invalid")?;
    let per_page = root
        .get("per_page")
        .and_then(Value::as_u64)
        .filter(|value| (1..=100).contains(value))
        .ok_or("nni_rewards_contract_invalid")?;
    let records = root
        .get("records")
        .and_then(Value::as_array)
        .ok_or("nni_rewards_contract_invalid")?;

    if root.get("schema_version").and_then(Value::as_u64) != Some(1)
        || root.get("status").and_then(Value::as_str) != Some("heartbeat_rewards")
        || root
            .get("device_pubkey")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        || root
            .get("reward_aic_scale")
            .and_then(Value::as_u64)
            .is_none_or(|scale| scale == 0)
        || root.get("reward_decimal_places").and_then(Value::as_u64) != Some(8)
        || !root
            .get("total_reward_units")
            .is_some_and(nni_network_stats_integer_string)
        || !root
            .get("total_reward_aic")
            .is_some_and(nni_network_stats_decimal)
        || root
            .get("reward_grant_count")
            .and_then(Value::as_u64)
            .is_none()
        || !nni_network_stats_optional_unix(root.get("first_period_start_unix"))
        || !nni_network_stats_optional_unix(root.get("latest_period_end_unix"))
        || root.get("page").and_then(Value::as_u64).is_none_or(|page| page == 0)
        || root.get("total").and_then(Value::as_u64).is_none()
        || root
            .get("total_pages")
            .and_then(Value::as_u64)
            .is_none_or(|pages| pages == 0)
        || root
            .get("history_limit")
            .and_then(Value::as_u64)
            .is_none_or(|limit| !(1..=100).contains(&limit))
        || root.get("history_truncated").and_then(Value::as_bool).is_none()
        || records.len() as u64 > per_page
        || records.iter().any(|record| !record.is_object())
    {
        return Err("nni_rewards_contract_invalid");
    }
    Ok(())
}

async fn nni_network_stats(
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
            );
        }
    };
    if nni_selected_remote_node(&config).is_none() {
        return nni_join_error(
            StatusCode::BAD_REQUEST,
            "nni_remote_node_required",
            json!({"status": "remote_node_required"}),
        );
    }

    let mut attempts = Vec::new();
    for node_url in nni_selected_remote_nodes(&config) {
        match nni_remote_read_with_retry(|| query_nni_network_stats_for_node(&state, node_url))
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
        "nni_network_stats_nodes_unavailable",
        json!({"status": "network_stats_nodes_unavailable", "attempts": attempts}),
    )
}

async fn query_nni_network_stats_for_node(
    state: &AppState,
    node_url: &str,
) -> Result<Value, Value> {
    let endpoint = nni_remote_api_endpoint(node_url, "network-stats");
    let response = state
        .core
        .http_client
        .get(&endpoint)
        .timeout(nni_remote_api_timeout())
        .send()
        .await
        .map_err(|err| {
            json!({
                "node_url": node_url,
                "error_code": "nni_network_stats_network_failed",
                "detail": err.to_string(),
                "retryable": true,
            })
        })?;
    let status = response.status();
    let body = response.json::<ApiResponse<Value>>().await.map_err(|err| {
        json!({
            "node_url": node_url,
            "http_status": status.as_u16(),
            "error_code": "nni_network_stats_body_invalid",
            "detail": err.to_string(),
            "retryable": nni_remote_http_status_retryable(status.as_u16()),
        })
    })?;
    if !status.is_success() || !body.ok {
        let error_code = nni_remote_api_error_code(&body, "nni_network_stats_failed");
        return Err(json!({
            "node_url": node_url,
            "http_status": status.as_u16(),
            "error_code": error_code,
            "retryable": nni_remote_http_status_retryable(status.as_u16()),
        }));
    }
    let data = body.data.ok_or_else(
        || json!({"node_url": node_url, "error_code": "nni_network_stats_data_missing"}),
    )?;
    validate_nni_network_stats_response(&data)
        .map_err(|error_code| json!({"node_url": node_url, "error_code": error_code}))?;
    Ok(data)
}

async fn nni_rewards(
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
            );
        }
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
    if nni_selected_remote_node(&config).is_none() {
        return nni_join_error(
            StatusCode::BAD_REQUEST,
            "nni_remote_node_required",
            json!({"status": "remote_node_required"}),
        );
    }
    let device_pubkey = match nni_device_pubkey(&state).await {
        Ok(pubkey) => pubkey,
        Err((status, error, data)) => return nni_join_error(status, error, data),
    };
    let page = query.page.unwrap_or(1).clamp(1, 1_000_000);
    let per_page = query.per_page.unwrap_or(10).clamp(1, 100);
    let mut attempts = Vec::new();
    for node_url in nni_selected_remote_nodes(&config) {
        match nni_remote_read_with_retry(|| {
            query_nni_rewards_for_node(
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
        "nni_reward_nodes_unavailable",
        json!({
            "status": "reward_nodes_unavailable",
            "attempts": attempts,
        }),
    )
}

async fn query_nni_rewards_for_node(
    state: &AppState,
    node_url: &str,
    device_pubkey: &str,
    user_key: &str,
    page: usize,
    per_page: usize,
) -> Result<Value, Value> {
    let request_endpoint = nni_remote_api_endpoint(node_url, "rewards/request");
    let request_response = state
        .core
        .http_client
        .post(&request_endpoint)
        .timeout(nni_remote_api_timeout())
        .json(&NniRemoteRewardQueryRequest {
            device_pubkey: device_pubkey.to_string(),
            client_user_key: user_key.to_string(),
        })
        .send()
        .await
        .map_err(|err| {
            json!({
                "node_url": node_url,
                "error_code": "nni_reward_request_network_failed",
                "detail": err.to_string(),
                "retryable": true,
            })
        })?;
    let request_status = request_response.status();
    let request_body = request_response
        .json::<ApiResponse<Value>>()
        .await
        .map_err(|err| {
            json!({
                "node_url": node_url,
                "http_status": request_status.as_u16(),
                "error_code": "nni_reward_request_body_invalid",
                "detail": err.to_string(),
                "retryable": true,
            })
        })?;
    if !request_status.is_success() || !request_body.ok {
        let error_code = nni_remote_api_error_code(&request_body, "nni_reward_request_failed");
        return Err(json!({
            "node_url": node_url,
            "http_status": request_status.as_u16(),
            "error_code": error_code,
            "retryable": nni_remote_http_status_retryable(request_status.as_u16()),
        }));
    }
    let request_data = request_body.data.ok_or_else(
        || json!({"node_url": node_url, "error_code": "nni_reward_request_data_missing"}),
    )?;
    let task_id = request_data
        .get("task_id")
        .and_then(Value::as_str)
        .ok_or_else(|| json!({"node_url": node_url, "error_code": "nni_reward_task_id_missing"}))?;
    let challenge = request_data
        .get("challenge")
        .and_then(Value::as_str)
        .ok_or_else(
            || json!({"node_url": node_url, "error_code": "nni_reward_challenge_missing"}),
        )?;

    let sign_output = run_nni_signature_helper(
        state,
        &[String::from("sign_challenge"), challenge.to_string()],
    )
    .await
    .map_err(
        |_| json!({"node_url": node_url, "error_code": "nni_reward_signature_helper_failed", "retryable": true}),
    )?;
    if !sign_output.ok {
        return Err(json!({
            "node_url": node_url,
            "error_code": "nni_reward_signature_failed",
            "retryable": true,
        }));
    }
    let signature = sign_output
        .payload
        .get("signature")
        .and_then(Value::as_str)
        .ok_or_else(
            || json!({"node_url": node_url, "error_code": "nni_reward_signature_missing"}),
        )?;

    let verify_endpoint = nni_remote_api_endpoint(node_url, "rewards/verify");
    let verify_response = state
        .core
        .http_client
        .post(&verify_endpoint)
        .timeout(nni_remote_api_timeout())
        .json(&NniRemoteRewardQueryVerifyRequest {
            task_id: task_id.to_string(),
            signature: signature.to_string(),
            page,
            per_page,
        })
        .send()
        .await
        .map_err(|err| {
            json!({
                "node_url": node_url,
                "error_code": "nni_reward_verify_network_failed",
                "detail": err.to_string(),
                "retryable": true,
            })
        })?;
    let verify_status = verify_response.status();
    let verify_body = verify_response
        .json::<ApiResponse<Value>>()
        .await
        .map_err(|err| {
            json!({
                "node_url": node_url,
                "http_status": verify_status.as_u16(),
                "error_code": "nni_reward_verify_body_invalid",
                "detail": err.to_string(),
                "retryable": true,
            })
        })?;
    if !verify_status.is_success() || !verify_body.ok {
        let error_code = nni_remote_api_error_code(&verify_body, "nni_reward_verify_failed");
        return Err(json!({
            "node_url": node_url,
            "http_status": verify_status.as_u16(),
            "error_code": error_code,
            "retryable": nni_remote_http_status_retryable(verify_status.as_u16()),
        }));
    }
    let data = verify_body.data.ok_or_else(
        || json!({"node_url": node_url, "error_code": "nni_reward_verify_data_missing"}),
    )?;
    validate_nni_rewards_response(&data).map_err(
        |error_code| json!({"node_url": node_url, "error_code": error_code}),
    )?;
    Ok(data)
}

#[cfg(test)]
mod nni_network_stats_unit_tests {
    use super::*;

    fn valid_network_stats() -> Value {
        json!({
            "schema_version": 1,
            "status": "heartbeat_network_stats",
            "network_devices": {
                "registered_device_count": 12,
                "active_device_count": 8,
                "active_period_start_unix": null,
                "active_period_end_unix": null,
                "first_heartbeat_unix": null,
                "window_seconds": 600
            },
            "reward_policy": {
                "phase": "scheduled",
                "accepting_reward_heartbeats": false,
                "reward_start_time_unix": 1_800_000_000,
                "starts_in_seconds": 300,
                "first_settlement_at_unix": 1_800_000_600,
                "interval_seconds": 600,
                "initial_reward_pool_aic": 5000,
                "current_reward_pool_units": "500000000000",
                "current_reward_pool_aic": "5000.00000000",
                "distribution": "equal_per_eligible_device",
                "halving_epoch_unix": null,
                "halving_interval_seconds": 126_144_000,
                "halving_era": null,
                "rewards_ended": false,
                "next_halving_at_unix": null
            },
            "network_rewards": {
                "total_distributed_reward_units": "0",
                "total_distributed_reward_aic": "0.00000000",
                "settled_period_count": 0,
                "first_period_start_unix": null,
                "latest_period_end_unix": null
            }
        })
    }

    fn valid_rewards() -> Value {
        let network_stats = valid_network_stats();
        json!({
            "schema_version": 1,
            "status": "heartbeat_rewards",
            "device_pubkey": "test-device-public-key",
            "asset_owner_pubkey": null,
            "authorization_epoch": null,
            "reward_aic_scale": 100_000_000,
            "reward_decimal_places": 8,
            "total_reward_units": "500000000000",
            "total_reward_aic": "5000.00000000",
            "reward_grant_count": 1,
            "first_period_start_unix": null,
            "latest_period_end_unix": null,
            "network_devices": network_stats["network_devices"].clone(),
            "reward_policy": network_stats["reward_policy"].clone(),
            "network_rewards": network_stats["network_rewards"].clone(),
            "page": 1,
            "per_page": 20,
            "total": 1,
            "total_pages": 1,
            "history_limit": 100,
            "history_truncated": false,
            "records": [{}]
        })
    }

    #[test]
    fn accepts_aggregate_only_network_stats_before_first_heartbeat() {
        assert_eq!(
            validate_nni_network_stats_response(&valid_network_stats()),
            Ok(())
        );
    }

    #[test]
    fn rejects_private_device_data_and_malformed_aggregate_values() {
        let mut private = valid_network_stats();
        private["device_pubkey"] = json!("private-key");
        assert_eq!(
            validate_nni_network_stats_response(&private),
            Err("nni_network_stats_contract_invalid")
        );

        let mut malformed = valid_network_stats();
        malformed["network_rewards"]["total_distributed_reward_aic"] = json!("0");
        assert_eq!(
            validate_nni_network_stats_response(&malformed),
            Err("nni_network_stats_contract_invalid")
        );

        let mut malformed_pool = valid_network_stats();
        malformed_pool["reward_policy"]["current_reward_pool_units"] = json!(5000);
        assert_eq!(
            validate_nni_network_stats_response(&malformed_pool),
            Err("nni_network_stats_contract_invalid")
        );
    }

    #[test]
    fn validates_private_reward_envelope_and_history_metadata() {
        assert_eq!(validate_nni_rewards_response(&valid_rewards()), Ok(()));

        let mut malformed = valid_rewards();
        malformed["history_truncated"] = json!("false");
        assert_eq!(
            validate_nni_rewards_response(&malformed),
            Err("nni_rewards_contract_invalid")
        );

        let mut oversized = valid_rewards();
        oversized["per_page"] = json!(1);
        oversized["records"] = json!([{}, {}]);
        assert_eq!(
            validate_nni_rewards_response(&oversized),
            Err("nni_rewards_contract_invalid")
        );
    }
}
