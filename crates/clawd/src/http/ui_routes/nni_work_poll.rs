fn nni_work_poll_notify() -> &'static Notify {
    static NOTIFY: OnceLock<Notify> = OnceLock::new();
    NOTIFY.get_or_init(Notify::new)
}

fn nni_work_poll_retry_delay(failures: u32) -> Duration {
    Duration::from_secs(1_u64 << failures.saturating_sub(1).min(5))
}

fn nni_verify_work_poll_challenge(
    challenge: &str,
    task_id: &str,
    nonce: Option<&str>,
    request: &NniRemoteHeartbeatRequest,
) -> Result<(), &'static str> {
    let id = task_id
        .strip_prefix("nni-work-poll-")
        .ok_or("work_poll_task_id_invalid")?;
    let nonce = nonce.ok_or("work_poll_nonce_missing")?;
    if id.len() != 32
        || !id.bytes().all(|v| v.is_ascii_hexdigit())
        || nonce.len() != 64
        || !nonce.bytes().all(|v| v.is_ascii_hexdigit())
    {
        return Err("work_poll_challenge_invalid");
    }
    let capabilities = serde_json::to_string(&request.worker_capabilities)
        .map_err(|_| "work_poll_capabilities_encode_failed")?;
    let report = serde_json::to_string(&request.work_report)
        .map_err(|_| "work_poll_report_encode_failed")?;
    let expected = format!(
        "nni-work-poll-v1\0{task_id}\0{nonce}\0{}\0{}",
        nni_work_sha256(&capabilities),
        nni_work_sha256(&report)
    );
    if challenge != nni_work_sha256(&expected) {
        return Err("work_poll_challenge_binding_invalid");
    }
    Ok(())
}

fn spawn_nni_work_poll_worker(state: AppState) {
    tokio::spawn(async move {
        nni_resume_work_worker(state.clone());
        let mut failures = 0_u32;
        loop {
            // Do not abandon an issued signing challenge when a local job
            // finishes. Active-lease exchanges return immediately; the local
            // notification interrupts only the wait between renewals.
            let result = nni_work_poll_tick(&state).await;
            let delay = match result {
                Ok(true) => {
                    failures = 0;
                    read_nni_local_work_state(&state)
                        .ok()
                        .flatten()
                        .filter(|work| matches!(work.status.as_str(), "assigned" | "running"))
                        .map(|_| Duration::from_secs(20))
                }
                Ok(false) => {
                    failures = 0;
                    Some(Duration::from_secs(5))
                }
                Err(error) => {
                    failures = failures.saturating_add(1);
                    append_nni_log_event_best_effort(
                        &state,
                        "distributed_work_transport_error",
                        json!({"error_code": error.code, "network": error.network}),
                    );
                    let jitter = u64::from(uuid::Uuid::new_v4().as_bytes()[0]);
                    Some(nni_work_poll_retry_delay(failures) + Duration::from_millis(jitter))
                }
            };
            if let Some(delay) = delay {
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = nni_work_poll_notify().notified() => {}
                }
            }
        }
    });
}

async fn nni_work_poll_tick(state: &AppState) -> Result<bool, NniHeartbeatError> {
    if let Some(local) = read_nni_local_work_state(state)
        .map_err(|_| {
            NniHeartbeatError::non_network(
                "nni_work_state_read_failed",
                "nni_work_state_read_failed",
            )
        })?
        .filter(|work| work.status == "cleanup_pending")
    {
        nni_finish_acknowledged_cleanup(state, &local).map_err(|_| {
            NniHeartbeatError::non_network("work_cleanup_failed", "work_cleanup_failed")
        })?;
    }
    let config = read_nni_config(state).map_err(|_| {
        NniHeartbeatError::non_network("nni_work_config_read_failed", "nni_work_config_read_failed")
    })?;
    let local = read_nni_local_work_state(state).map_err(|_| {
        NniHeartbeatError::non_network("nni_work_state_read_failed", "nni_work_state_read_failed")
    })?;
    if (!config.joined || !nni_work_pull_enabled()) && local.is_none() {
        return Ok(false);
    }
    let keys = nni_work_assignment_verifying_keys()
        .map_err(|code| NniHeartbeatError::non_network(code, code))?;
    // In-flight work stays pinned to the coordinator that issued its lease.
    let node_url = local
        .as_ref()
        .map(|work| work.node_url.as_str())
        .or_else(|| nni_selected_remote_node(&config).map(String::as_str));
    let Some(node_url) = node_url else {
        return Ok(false);
    };
    let device = nni_device_pubkey(state)
        .await
        .map_err(|(_, code, _)| NniHeartbeatError::non_network(code, code))?;
    let request = NniRemoteHeartbeatRequest {
        device_pubkey: device.clone(),
        client_user_key: "clawd-nni-work".to_string(),
        worker_capabilities: nni_current_worker_capabilities(state),
        work_report: nni_pending_work_report_for_node(state, node_url, &device),
    };
    if request.worker_capabilities.is_none() && request.work_report.is_none() {
        return Ok(false);
    }
    let mut data = nni_signed_exchange(state, node_url, "work", &request).await?;
    let control: NniWorkAssignmentEnvelope = serde_json::from_value(
        data.get("work_control").cloned().unwrap_or(Value::Null),
    )
    .map_err(|_| NniHeartbeatError::non_network("work_control_missing", "work_control_missing"))?;
    let request_id = data
        .get("task_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let verified = nni_verify_work_control_envelope(&control, request_id, &device, &keys)
        .map_err(|code| NniHeartbeatError::non_network(code, code))?;
    data["node_url"] = Value::String(node_url.to_string());
    data["local_device_pubkey"] = Value::String(device);
    nni_handle_work_response(state, &data, request.work_report.as_ref());
    if verified.work_poll_status == "error" || verified.work_report_status == "error" {
        let code = verified
            .work_poll_error_code
            .or(verified.work_report_error_code)
            .unwrap_or_else(|| "nni_work_poll_failed".to_string());
        return Err(NniHeartbeatError::non_network(code.clone(), code));
    }
    Ok(true)
}

#[cfg(test)]
#[path = "nni_work_poll_tests.rs"]
mod nni_work_poll_tests;
