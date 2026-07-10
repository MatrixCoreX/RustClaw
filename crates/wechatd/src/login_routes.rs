use super::*;

pub(super) fn build_login_status_response(
    status: &WechatRuntimeStatus,
    active_login: Option<&ActiveLogin>,
) -> LoginStatusResponse {
    LoginStatusResponse {
        connected: runtime_status_is_connected(&status.status),
        qr_ready: active_login.is_some(),
        session_key: active_login.map(|login| login.session_key.clone()),
        qr_status: active_login.map(|login| login.status.clone()),
        qrcode_url: active_login.map(|login| login.qrcode_url.clone()),
        message: active_login.map(|login| login.message.clone()),
        last_update_ts: status.last_event_ts,
        last_error: status.last_error.clone(),
        account_label: status.account_label.clone(),
        status: status.status.clone(),
    }
}

pub(super) async fn login_status(AxumState(state): AxumState<State>) -> Json<LoginStatusResponse> {
    let status = state.status.read().await.clone();
    let active_login = {
        let logins = state.active_logins.read().await;
        logins
            .values()
            .find(|login| active_login_is_fresh(login))
            .cloned()
    };
    Json(build_login_status_response(&status, active_login.as_ref()))
}

pub(super) async fn login_qr_start(
    AxumState(state): AxumState<State>,
    Json(req): Json<LoginStartRequest>,
) -> Result<Json<LoginStartResponse>, (axum::http::StatusCode, String)> {
    let session_key = "primary".to_string();
    let mut active = state.active_logins.write().await;
    if !req.force {
        if let Some(existing) = active.get(&session_key) {
            if active_login_is_fresh(existing) {
                return Ok(Json(LoginStartResponse {
                    session_key,
                    qrcode_url: existing.qrcode_url.clone(),
                    message: wechat_t(&state.config, "wechat.msg.login.qr_ready_scan"),
                }));
            }
        }
    }
    let response = fetch_qrcode(
        &state.client,
        &state.config,
        req.bot_type.as_deref().unwrap_or("3"),
    )
    .await
    .map_err(|e| (axum::http::StatusCode::BAD_GATEWAY, e))?;
    let qrcode_url = qr_svg_data_url(qr_render_content(&response))
        .map_err(|e| (axum::http::StatusCode::BAD_GATEWAY, e))?;
    active.insert(
        session_key.clone(),
        ActiveLogin {
            session_key: session_key.clone(),
            qrcode: response.qrcode.clone(),
            qrcode_url: qrcode_url.clone(),
            started_at_ms: current_ts_ms(),
            status: "wait".to_string(),
            message: wechat_t(&state.config, "wechat.msg.login.qr_waiting_scan"),
        },
    );
    update_status(&state, |status| {
        status.healthy = true;
        status.status = "qr_ready".to_string();
        status.last_event_ts = Some(current_ts_ms());
        status.last_error = None;
    })
    .await;
    Ok(Json(LoginStartResponse {
        session_key,
        qrcode_url,
        message: wechat_t(&state.config, "wechat.msg.login.scan_to_connect"),
    }))
}

pub(super) async fn login_qr_wait(
    AxumState(state): AxumState<State>,
    Json(req): Json<LoginWaitRequest>,
) -> Result<Json<LoginWaitResponse>, (axum::http::StatusCode, String)> {
    let timeout_ms = req.timeout_ms.unwrap_or(480_000).max(1_000);
    let deadline = current_ts_ms().saturating_add(timeout_ms);
    let mut refresh_count = 1usize;
    loop {
        let active_login = {
            let logins = state.active_logins.read().await;
            logins.get(&req.session_key).cloned()
        }
        .ok_or_else(|| {
            (
                axum::http::StatusCode::BAD_REQUEST,
                wechat_t(&state.config, "wechat.msg.login.no_active_login"),
            )
        })?;

        if !active_login_is_fresh(&active_login) {
            state.active_logins.write().await.remove(&req.session_key);
            return Ok(Json(LoginWaitResponse {
                connected: false,
                qr_status: "expired".to_string(),
                message: wechat_t(&state.config, "wechat.msg.login.qr_expired"),
                account_id: None,
                user_id: None,
            }));
        }

        let status = poll_qr_status(&state.client, &state.config, &active_login.qrcode)
            .await
            .map_err(|e| (axum::http::StatusCode::BAD_GATEWAY, e))?;
        match status.status.as_str() {
            "wait" | "scaned" => {
                let qr_message = if status.status == "scaned" {
                    wechat_t(&state.config, "wechat.msg.login.qr_scanned_confirm")
                } else {
                    wechat_t(&state.config, "wechat.msg.login.qr_waiting_scan")
                };
                if let Some(login) = state.active_logins.write().await.get_mut(&req.session_key) {
                    login.status = status.status.clone();
                    login.message = qr_message.clone();
                }
                if status.status == "scaned" {
                    update_status(&state, |runtime| {
                        runtime.healthy = true;
                        runtime.status = "qr_scanned".to_string();
                        runtime.last_event_ts = Some(current_ts_ms());
                        runtime.last_error = None;
                    })
                    .await;
                }
                if current_ts_ms().saturating_add(1_000) >= deadline {
                    return Ok(Json(LoginWaitResponse {
                        connected: false,
                        qr_status: status.status.clone(),
                        message: qr_message,
                        account_id: None,
                        user_id: None,
                    }));
                }
            }
            "expired" => {
                refresh_count = refresh_count.saturating_add(1);
                if refresh_count > 3 {
                    state.active_logins.write().await.remove(&req.session_key);
                    return Ok(Json(LoginWaitResponse {
                        connected: false,
                        qr_status: "expired".to_string(),
                        message: wechat_t(&state.config, "wechat.msg.login.qr_expired_too_many"),
                        account_id: None,
                        user_id: None,
                    }));
                }
                let refreshed = fetch_qrcode(
                    &state.client,
                    &state.config,
                    req.bot_type.as_deref().unwrap_or("3"),
                )
                .await
                .map_err(|e| (axum::http::StatusCode::BAD_GATEWAY, e))?;
                let qrcode_url = qr_svg_data_url(qr_render_content(&refreshed))
                    .map_err(|e| (axum::http::StatusCode::BAD_GATEWAY, e))?;
                state.active_logins.write().await.insert(
                    req.session_key.clone(),
                    ActiveLogin {
                        session_key: req.session_key.clone(),
                        qrcode: refreshed.qrcode,
                        qrcode_url,
                        started_at_ms: current_ts_ms(),
                        status: "wait".to_string(),
                        message: wechat_t(&state.config, "wechat.msg.login.qr_refreshed_waiting"),
                    },
                );
            }
            "confirmed" => {
                let bot_token = status.bot_token.clone().unwrap_or_default();
                let account_id = status.ilink_bot_id.clone();
                if bot_token.trim().is_empty() || account_id.is_none() {
                    return Ok(Json(LoginWaitResponse {
                        connected: false,
                        qr_status: "confirmed".to_string(),
                        message: wechat_t(
                            &state.config,
                            "wechat.msg.login.confirmed_missing_token",
                        ),
                        account_id: None,
                        user_id: None,
                    }));
                }
                let session = PersistedSession {
                    bot_token,
                    account_id: account_id.clone(),
                    base_url: status.baseurl.clone(),
                    user_id: status.ilink_user_id.clone(),
                    saved_at: Some(current_ts_secs().to_string()),
                };
                *state.session.write().await = Some(session.clone());
                write_json_file(&state.session_path, &session).await;
                state.active_logins.write().await.remove(&req.session_key);
                update_status(&state, |runtime| {
                    runtime.healthy = true;
                    runtime.status = "connected".to_string();
                    runtime.last_event_ts = Some(current_ts_ms());
                    runtime.last_error = None;
                    runtime.account_label = account_id.clone();
                })
                .await;
                return Ok(Json(LoginWaitResponse {
                    connected: true,
                    qr_status: "confirmed".to_string(),
                    message: wechat_t(&state.config, "wechat.msg.login.connected"),
                    account_id,
                    user_id: status.ilink_user_id,
                }));
            }
            other => {
                warn!("wechatd: unexpected qr status={}", other);
            }
        }
        if current_ts_ms() >= deadline {
            return Ok(Json(LoginWaitResponse {
                connected: false,
                qr_status: "wait".to_string(),
                message: wechat_t(&state.config, "wechat.msg.login.wait_timeout"),
                account_id: None,
                user_id: None,
            }));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
