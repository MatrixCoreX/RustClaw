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
    if config.remote_nodes.is_empty() {
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
    for node_url in &config.remote_nodes {
        match query_nni_rewards_for_node(
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
    let request_endpoint = format!("{node_url}/v1/nni/server/rewards/request");
    let request_response = state
        .core
        .http_client
        .post(&request_endpoint)
        .timeout(Duration::from_secs(NNI_REMOTE_JOIN_TIMEOUT_SECONDS))
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
            })
        })?;
    if !request_status.is_success() || !request_body.ok {
        let error_code =
            nni_remote_api_error_code(&request_body, "nni_reward_request_failed");
        return Err(json!({
            "node_url": node_url,
            "http_status": request_status.as_u16(),
            "error_code": error_code,
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
        |_| json!({"node_url": node_url, "error_code": "nni_reward_signature_helper_failed"}),
    )?;
    if !sign_output.ok {
        return Err(json!({
            "node_url": node_url,
            "error_code": "nni_reward_signature_failed",
        }));
    }
    let signature = sign_output
        .payload
        .get("signature")
        .and_then(Value::as_str)
        .ok_or_else(
            || json!({"node_url": node_url, "error_code": "nni_reward_signature_missing"}),
        )?;

    let verify_endpoint = format!("{node_url}/v1/nni/server/rewards/verify");
    let verify_response = state
        .core
        .http_client
        .post(&verify_endpoint)
        .timeout(Duration::from_secs(NNI_REMOTE_JOIN_TIMEOUT_SECONDS))
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
            })
        })?;
    if !verify_status.is_success() || !verify_body.ok {
        let error_code =
            nni_remote_api_error_code(&verify_body, "nni_reward_verify_failed");
        return Err(json!({
            "node_url": node_url,
            "http_status": verify_status.as_u16(),
            "error_code": error_code,
        }));
    }
    verify_body.data.ok_or_else(
        || json!({"node_url": node_url, "error_code": "nni_reward_verify_data_missing"}),
    )
}
