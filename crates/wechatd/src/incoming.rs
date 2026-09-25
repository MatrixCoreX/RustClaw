use super::*;

pub(super) fn inbound_message_is_explicit_control(msg: &WeixinMessage) -> bool {
    extract_text_message(msg)
        .as_deref()
        .is_some_and(|text| cancel_expected_task_id(text).is_some())
}

pub(super) async fn reserve_inbound_order(
    order: &Arc<Mutex<HashMap<String, InboundPeerOrder>>>,
    msg: &WeixinMessage,
) -> InboundOrderTicket {
    let peer_id = msg
        .from_user_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .to_string();
    let mut order = order.lock().await;
    let entry = order
        .entry(peer_id.clone())
        .or_insert_with(|| InboundPeerOrder {
            next_ticket: 0,
            serving_ticket: 0,
            notify: Arc::new(Notify::new()),
        });
    let ticket = entry.next_ticket;
    entry.next_ticket = entry.next_ticket.saturating_add(1);
    InboundOrderTicket {
        peer_id,
        ticket,
        notify: entry.notify.clone(),
    }
}

pub(super) async fn wait_for_inbound_order(
    order: &Arc<Mutex<HashMap<String, InboundPeerOrder>>>,
    ticket: &InboundOrderTicket,
) {
    loop {
        let notified = ticket.notify.notified();
        let ready = order
            .lock()
            .await
            .get(&ticket.peer_id)
            .is_some_and(|entry| entry.serving_ticket == ticket.ticket);
        if ready {
            return;
        }
        notified.await;
    }
}

pub(super) async fn complete_inbound_order(
    order: &Arc<Mutex<HashMap<String, InboundPeerOrder>>>,
    ticket: &InboundOrderTicket,
) {
    let mut order = order.lock().await;
    let Some(entry) = order.get_mut(&ticket.peer_id) else {
        return;
    };
    if entry.serving_ticket != ticket.ticket {
        return;
    }
    entry.serving_ticket = entry.serving_ticket.saturating_add(1);
    let complete = entry.serving_ticket == entry.next_ticket;
    entry.notify.notify_waiters();
    if complete {
        order.remove(&ticket.peer_id);
    }
}

pub(super) fn inbound_finish_outcome(
    durable_handoff: bool,
) -> claw_core::channel_event_admission::ChannelEventFinishOutcome {
    if durable_handoff {
        claw_core::channel_event_admission::ChannelEventFinishOutcome::Completed
    } else {
        claw_core::channel_event_admission::ChannelEventFinishOutcome::RetryableFailure
    }
}

pub(super) async fn handle_incoming_message(state: State, msg: WeixinMessage) {
    let Some(provider_message_id) = inbound_provider_message_id(&msg) else {
        warn!("wechatd: inbound message skipped because provider identity is missing");
        return;
    };
    let (account_id, admission_secret) = {
        let session = state.session.read().await;
        (
            session_account_id(session.as_ref()),
            session_token(&state.config, session.as_ref()),
        )
    };
    let Some(admission_secret) = admission_secret else {
        warn!("wechatd: inbound event admission secret unavailable");
        return;
    };
    let payload = match serde_json::to_vec(&msg) {
        Ok(payload) => payload,
        Err(error) => {
            warn!("wechatd: inbound event serialization failed error={error}");
            return;
        }
    };
    let claim = claw_core::channel_event_admission::ChannelEventClaimRequest::new(
        ChannelKind::Wechat,
        account_id.clone(),
        provider_message_id.clone(),
        &payload,
    );
    let claim_response = match claw_core::channel_event_admission::claim_channel_event(
        &state.client,
        &state.config.clawd_base_url,
        &admission_secret,
        &claim,
    )
    .await
    {
        Ok(response) => response,
        Err(error) => {
            warn!("wechatd: inbound event admission failed error={error}");
            return;
        }
    };
    if claim_response.status
        != claw_core::channel_event_admission::ChannelEventClaimStatus::Acquired
    {
        tracing::debug!(
            provider_message_id = %provider_message_id,
            status = ?claim_response.status,
            "wechat duplicate event suppressed"
        );
        return;
    }
    let Some(lease_token) = claim_response.lease_token else {
        warn!("wechatd: inbound event admission lease missing");
        return;
    };
    let durable_handoff =
        handle_claimed_incoming_message(state.clone(), msg, provider_message_id.clone()).await;
    let finish = claw_core::channel_event_admission::ChannelEventFinishRequest {
        schema_version: claw_core::channel_event_admission::CHANNEL_EVENT_ADMISSION_SCHEMA_VERSION,
        channel: ChannelKind::Wechat,
        account_id,
        provider_event_id: provider_message_id,
        payload_sha256: claim.payload_sha256,
        lease_token,
        outcome: inbound_finish_outcome(durable_handoff),
    };
    if let Err(error) = claw_core::channel_event_admission::finish_channel_event(
        &state.client,
        &state.config.clawd_base_url,
        &admission_secret,
        &finish,
    )
    .await
    {
        warn!("wechatd: inbound event admission finish failed error={error}");
    }
}

async fn handle_claimed_incoming_message(
    state: State,
    msg: WeixinMessage,
    provider_message_id: String,
) -> bool {
    let Some(from_user_id) = msg
        .from_user_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
    else {
        return true;
    };
    let Some(task_context) =
        pin_inbound_task_context(&state, &from_user_id, msg.context_token.as_deref()).await
    else {
        warn!("wechatd: inbound message skipped because task context could not be pinned");
        return false;
    };
    // Cover CDN download / decrypt / transcode latency before the clawd task heartbeat starts.
    let _media_typing_guard = if extract_text_message(&msg).is_none() {
        start_typing_heartbeat_for_peer(&state, &task_context).await
    } else {
        None
    };

    if extract_text_message(&msg).is_none() {
        let pending_attachment_kind = if inbound_image_decrypt_params(&msg).is_some() {
            Some("image")
        } else if inbound_video_decrypt_params(&msg).is_some() {
            Some("video")
        } else if inbound_file_decrypt_params(&msg).is_some() {
            Some("file")
        } else if inbound_voice_decrypt_params(&msg).is_some() {
            Some("audio")
        } else {
            None
        };
        let Some(identity) = ensure_bound_before_task(
            &state,
            &task_context,
            &from_user_id,
            None,
            Some(&provider_message_id),
            pending_attachment_kind,
        )
        .await
        else {
            return true;
        };
        let bound_user_key = identity.user_key;
        if let Some((ep, key)) = inbound_image_decrypt_params(&msg) {
            let cdn = state.config.cdn_base_url.trim();
            match download_decrypted_media(&state.client, &ep, &key, cdn, "inbound-image").await {
                Ok(bytes) => {
                    if bytes.len() > 25 * 1024 * 1024 {
                        warn!("wechatd: inbound image too large ({} bytes)", bytes.len());
                        return true;
                    }
                    let rel = build_wechat_inbox_rel_path(
                        &state.config.image_inbox_dir,
                        &from_user_id,
                        &format!("{}.jpg", provider_message_id),
                    );
                    let abs = state.workspace_root.join(&rel);
                    if let Some(parent) = abs.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    if tokio::fs::write(&abs, &bytes).await.is_err() {
                        warn!("wechatd: failed to write inbound image {}", rel);
                        return false;
                    }
                    update_status(&state, |status| {
                        status.healthy = true;
                        status.status = "message_received".to_string();
                        status.last_event_ts = msg.create_time_ms.or(Some(current_ts_ms()));
                        status.last_peer = Some(from_user_id.clone());
                        status.last_error = None;
                    })
                    .await;
                    return spawn_inbound_attachment_flow(
                        state,
                        task_context.clone(),
                        "image",
                        rel,
                        "image/jpeg",
                        bytes.len() as u64,
                        bound_user_key.clone(),
                        provider_message_id.clone(),
                    )
                    .await;
                }
                Err(err) => {
                    warn!("wechatd: inbound image decrypt/download failed: {}", err);
                }
            }
        }
        if let Some((ep, key)) = inbound_video_decrypt_params(&msg) {
            let cdn = state.config.cdn_base_url.trim();
            match download_decrypted_media(&state.client, &ep, &key, cdn, "inbound-video").await {
                Ok(bytes) => {
                    if bytes.len() > 100 * 1024 * 1024 {
                        warn!("wechatd: inbound video too large");
                        return true;
                    }
                    let rel = build_wechat_inbox_rel_path(
                        &state.config.video_inbox_dir,
                        &from_user_id,
                        &format!("{}.mp4", provider_message_id),
                    );
                    let abs = state.workspace_root.join(&rel);
                    if let Some(parent) = abs.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    if tokio::fs::write(&abs, &bytes).await.is_err() {
                        warn!("wechatd: failed to write inbound video {}", rel);
                        return false;
                    }
                    update_status(&state, |status| {
                        status.healthy = true;
                        status.status = "message_received".to_string();
                        status.last_event_ts = msg.create_time_ms.or(Some(current_ts_ms()));
                        status.last_peer = Some(from_user_id.clone());
                        status.last_error = None;
                    })
                    .await;
                    return spawn_inbound_attachment_flow(
                        state,
                        task_context.clone(),
                        "video",
                        rel,
                        "video/mp4",
                        bytes.len() as u64,
                        bound_user_key.clone(),
                        provider_message_id.clone(),
                    )
                    .await;
                }
                Err(err) => {
                    warn!("wechatd: inbound video decrypt/download failed: {}", err);
                }
            }
        }
        if let Some((ep, key, safe_name)) = inbound_file_decrypt_params(&msg) {
            let cdn = state.config.cdn_base_url.trim();
            match download_decrypted_media(&state.client, &ep, &key, cdn, "inbound-file").await {
                Ok(bytes) => {
                    if bytes.len() > 100 * 1024 * 1024 {
                        warn!("wechatd: inbound file too large");
                        return true;
                    }
                    let rel = build_wechat_inbox_rel_path(
                        &state.config.file_inbox_dir,
                        &from_user_id,
                        &format!("{}_{}", provider_message_id, safe_name),
                    );
                    let abs = state.workspace_root.join(&rel);
                    if let Some(parent) = abs.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    if tokio::fs::write(&abs, &bytes).await.is_err() {
                        warn!("wechatd: failed to write inbound file {}", rel);
                        return false;
                    }
                    update_status(&state, |status| {
                        status.healthy = true;
                        status.status = "message_received".to_string();
                        status.last_event_ts = msg.create_time_ms.or(Some(current_ts_ms()));
                        status.last_peer = Some(from_user_id.clone());
                        status.last_error = None;
                    })
                    .await;
                    return spawn_inbound_attachment_flow(
                        state,
                        task_context.clone(),
                        "file",
                        rel,
                        "application/octet-stream",
                        bytes.len() as u64,
                        bound_user_key.clone(),
                        provider_message_id.clone(),
                    )
                    .await;
                }
                Err(err) => {
                    warn!("wechatd: inbound file decrypt/download failed: {}", err);
                }
            }
        }
        if let Some((ep, key)) = inbound_voice_decrypt_params(&msg) {
            let cdn = state.config.cdn_base_url.trim();
            match download_decrypted_media(&state.client, &ep, &key, cdn, "inbound-voice").await {
                Ok(bytes) => {
                    if bytes.len() > 20 * 1024 * 1024 {
                        warn!("wechatd: inbound voice too large");
                        return true;
                    }
                    let (rel, data_to_write) =
                        if let Some(wav) = wechat_silk_wav::try_silk_to_wav(&bytes) {
                            (
                                build_wechat_inbox_rel_path(
                                    &state.config.audio_inbox_dir,
                                    &from_user_id,
                                    &format!("v{}.wav", provider_message_id),
                                ),
                                wav,
                            )
                        } else {
                            (
                                build_wechat_inbox_rel_path(
                                    &state.config.audio_inbox_dir,
                                    &from_user_id,
                                    &format!("v{}.bin", provider_message_id),
                                ),
                                bytes,
                            )
                        };
                    let abs = state.workspace_root.join(&rel);
                    if let Some(parent) = abs.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    if tokio::fs::write(&abs, &data_to_write).await.is_err() {
                        warn!("wechatd: failed to write inbound voice {}", rel);
                        return false;
                    }
                    update_status(&state, |status| {
                        status.healthy = true;
                        status.status = "message_received".to_string();
                        status.last_event_ts = msg.create_time_ms.or(Some(current_ts_ms()));
                        status.last_peer = Some(from_user_id.clone());
                        status.last_error = None;
                    })
                    .await;
                    let mime_type = if rel.ends_with(".wav") {
                        "audio/wav"
                    } else {
                        "application/octet-stream"
                    };
                    return spawn_inbound_attachment_flow(
                        state,
                        task_context.clone(),
                        "audio",
                        rel,
                        mime_type,
                        data_to_write.len() as u64,
                        bound_user_key.clone(),
                        provider_message_id.clone(),
                    )
                    .await;
                }
                Err(err) => {
                    warn!("wechatd: inbound voice decrypt/download failed: {}", err);
                }
            }
        }
    }

    let text = match extract_text_message(&msg) {
        Some(t) => t,
        None => {
            if has_non_text_media_items(&msg) {
                let reply = wechat_t(&state.config, "wechat.msg.media_decode_or_unsupported");
                send_text_reply_via_session(
                    &state,
                    &from_user_id,
                    msg.context_token.as_deref(),
                    &reply,
                )
                .await;
            }
            return true;
        }
    };
    update_status(&state, |status| {
        status.healthy = true;
        status.status = "message_received".to_string();
        status.last_event_ts = msg.create_time_ms.or(Some(current_ts_ms()));
        status.last_peer = Some(from_user_id.clone());
        status.last_error = None;
    })
    .await;

    let Some(identity) = ensure_bound_before_task(
        &state,
        &task_context,
        &from_user_id,
        Some(text.as_str()),
        Some(&provider_message_id),
        None,
    )
    .await
    else {
        return true;
    };
    if let Some(expected_task_id) = cancel_expected_task_id(&text) {
        let request = claw_core::conversation_control::CancelCurrentConversationTaskRequest {
            schema_version: claw_core::conversation_control::CONVERSATION_CONTROL_SCHEMA_VERSION,
            client_request_id: format!(
                "wechat_cancel:{}:{}",
                task_context.account.account_id, provider_message_id
            ),
            scope: claw_core::conversation_input::ConversationInputScopeRef {
                conversation_id: task_context.scope.storage_key(),
                agent_id: "main".to_string(),
                channel: "wechat".to_string(),
                channel_account_id: task_context.account.account_id.clone(),
            },
            expected_task_id: expected_task_id
                .as_deref()
                .and_then(|task_id| task_id.parse().ok()),
        };
        let message_key = match claw_core::conversation_control::cancel_current_conversation_task(
            &state.client,
            &state.config.clawd_base_url,
            &identity.user_key,
            &request,
        )
        .await
        {
            Ok(receipt) => claw_core::conversation_control::cancel_receipt_message_key(&receipt),
            Err(error) => {
                warn!(error = %error, "wechat current task cancellation failed");
                claw_core::conversation_control::CANCEL_FAILED_MESSAGE_KEY
            }
        };
        let reply =
            claw_core::channel_i18n::common_text_for_locale(&state.config.language, message_key);
        send_text_reply_via_session(&state, &from_user_id, msg.context_token.as_deref(), &reply)
            .await;
        return true;
    }
    submit_wechat_task_and_reply(
        state,
        task_context,
        text,
        Some(identity.user_key),
        provider_message_id,
    )
    .await
}
