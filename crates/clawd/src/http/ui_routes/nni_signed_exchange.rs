// Shared signed HTTP exchange; work requests use a distinct challenge domain.
async fn nni_signed_exchange(
    state: &AppState,
    node_url: &str,
    operation: &str,
    heartbeat_request: &NniRemoteHeartbeatRequest,
) -> Result<Value, NniHeartbeatError> {
    let request_endpoint = nni_remote_api_endpoint(node_url, &format!("{operation}/request"));
    let request_resp = state
        .core
        .public_http_client
        .post(&request_endpoint)
        .timeout(nni_remote_api_timeout())
        .json(&heartbeat_request)
        .send()
        .await
        .map_err(|err| {
            NniHeartbeatError::network(
                format!("{operation}_request_network_failed"),
                err.to_string(),
            )
        })?;
    let request_status = request_resp.status();
    let request_body = request_resp
        .json::<ApiResponse<Value>>()
        .await
        .map_err(|err| {
            NniHeartbeatError::non_network(
                format!("{operation}_request_body_failed"),
                err.to_string(),
            )
        })?;
    if !request_status.is_success() || !request_body.ok {
        let error_code =
            nni_remote_api_error_code(&request_body, &format!("{operation}_request_failed"));
        return Err(NniHeartbeatError::non_network(
            error_code,
            format!("status={} data={:?}", request_status, request_body.data),
        ));
    }
    let request_data = request_body.data.ok_or_else(|| {
        NniHeartbeatError::non_network(
            format!("{operation}_request_missing_data"),
            format!("{operation}_request_missing_data"),
        )
    })?;
    let task_id = request_data
        .get("task_id")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            NniHeartbeatError::non_network(
                format!("{operation}_task_id_missing"),
                format!("{operation}_task_id_missing"),
            )
        })?;
    let expected_task_id = task_id.clone();
    let challenge = request_data
        .get("challenge")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            NniHeartbeatError::non_network(
                format!("{operation}_challenge_missing"),
                format!("{operation}_challenge_missing"),
            )
        })?;
    if operation == "work" {
        nni_verify_work_poll_challenge(
            &challenge,
            &expected_task_id,
            request_data
                .get("work_challenge_nonce")
                .and_then(Value::as_str),
            heartbeat_request,
        )
        .map_err(|code| NniHeartbeatError::non_network(code, code))?;
    }

    let sign_output = run_nni_signature_helper(state, &[String::from("sign_challenge"), challenge])
        .await
        .map_err(|err| {
            NniHeartbeatError::non_network(format!("{operation}_signature_helper_failed"), err)
        })?;
    if !sign_output.ok {
        return Err(NniHeartbeatError::non_network(
            format!("{operation}_signature_failed"),
            sign_output
                .error
                .or_else(|| {
                    (!sign_output.stderr_tail.is_empty()).then_some(sign_output.stderr_tail)
                })
                .unwrap_or_else(|| format!("{operation}_signature_failed")),
        ));
    }
    let signature = sign_output
        .payload
        .get("signature")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            NniHeartbeatError::non_network(
                format!("{operation}_signature_missing"),
                format!("{operation}_signature_missing"),
            )
        })?;

    let verify_endpoint = nni_remote_api_endpoint(node_url, &format!("{operation}/verify"));
    let verify_resp = state
        .core
        .public_http_client
        .post(&verify_endpoint)
        .timeout(nni_remote_api_timeout())
        .json(&NniRemoteHeartbeatVerifyRequest { task_id, signature })
        .send()
        .await
        .map_err(|err| {
            NniHeartbeatError::network(
                format!("{operation}_verify_network_failed"),
                err.to_string(),
            )
        })?;
    let verify_status = verify_resp.status();
    let verify_body = verify_resp
        .json::<ApiResponse<Value>>()
        .await
        .map_err(|err| {
            NniHeartbeatError::non_network(
                format!("{operation}_verify_body_failed"),
                err.to_string(),
            )
        })?;
    if !verify_status.is_success() || !verify_body.ok {
        let error_code =
            nni_remote_api_error_code(&verify_body, &format!("{operation}_verify_failed"));
        return Err(NniHeartbeatError::non_network(
            error_code,
            format!("status={} data={:?}", verify_status, verify_body.data),
        ));
    }
    let data = verify_body.data.ok_or_else(|| {
        NniHeartbeatError::non_network(
            format!("{operation}_verify_missing_data"),
            format!("{operation}_verify_missing_data"),
        )
    })?;
    if data.get("task_id").and_then(Value::as_str) != Some(expected_task_id.as_str()) {
        return Err(NniHeartbeatError::non_network(
            format!("{operation}_verify_task_id_mismatch"),
            format!("{operation}_verify_task_id_mismatch"),
        ));
    }
    Ok(data)
}
