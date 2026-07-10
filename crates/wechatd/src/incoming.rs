use super::*;

pub(super) async fn handle_incoming_message(state: State, msg: WeixinMessage) {
    let Some(from_user_id) = msg
        .from_user_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
    else {
        return;
    };
    if let Some(token) = msg
        .context_token
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        remember_context_token(&state, &from_user_id, token).await;
    }
    let prefetched_typing_ticket =
        resolve_typing_ticket_for_peer(&state, &from_user_id, msg.context_token.as_deref()).await;
    // Cover CDN download / decrypt / transcode latency before the clawd task heartbeat starts.
    let _media_typing_guard = if extract_text_message(&msg).is_none() {
        start_typing_heartbeat_for_peer(&state, &from_user_id, prefetched_typing_ticket.as_deref())
            .await
    } else {
        None
    };

    if extract_text_message(&msg).is_none() {
        let Some(identity) =
            ensure_bound_before_task(&state, &from_user_id, msg.context_token.as_deref(), None)
                .await
        else {
            return;
        };
        let bound_user_key = identity.user_key;
        if let Some((ep, key)) = inbound_image_decrypt_params(&msg) {
            let cdn = state.config.cdn_base_url.trim();
            match download_decrypted_media(&state.client, &ep, &key, cdn, "inbound-image").await {
                Ok(bytes) => {
                    if bytes.len() > 25 * 1024 * 1024 {
                        warn!("wechatd: inbound image too large ({} bytes)", bytes.len());
                        return;
                    }
                    let rel = build_wechat_inbox_rel_path(
                        &state.config.image_inbox_dir,
                        &from_user_id,
                        &format!("{}.jpg", current_ts_ms()),
                    );
                    let abs = state.workspace_root.join(&rel);
                    if let Some(parent) = abs.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    if tokio::fs::write(&abs, &bytes).await.is_err() {
                        warn!("wechatd: failed to write inbound image {}", rel);
                        return;
                    }
                    update_status(&state, |status| {
                        status.healthy = true;
                        status.status = "message_received".to_string();
                        status.last_event_ts = msg.create_time_ms.or(Some(current_ts_ms()));
                        status.last_peer = Some(from_user_id.clone());
                        status.last_error = None;
                    })
                    .await;
                    return spawn_inbound_skill_flow(
                        state,
                        from_user_id,
                        msg,
                        "image_vision",
                        json!({
                            "action": "describe",
                            "images": [{"path": rel}],
                            "detail_level": "normal"
                        }),
                        bound_user_key.clone(),
                        prefetched_typing_ticket.clone(),
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
                        return;
                    }
                    let rel = build_wechat_inbox_rel_path(
                        &state.config.video_inbox_dir,
                        &from_user_id,
                        &format!("{}.mp4", current_ts_ms()),
                    );
                    let abs = state.workspace_root.join(&rel);
                    if let Some(parent) = abs.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    if tokio::fs::write(&abs, &bytes).await.is_err() {
                        warn!("wechatd: failed to write inbound video {}", rel);
                        return;
                    }
                    update_status(&state, |status| {
                        status.healthy = true;
                        status.status = "message_received".to_string();
                        status.last_event_ts = msg.create_time_ms.or(Some(current_ts_ms()));
                        status.last_peer = Some(from_user_id.clone());
                        status.last_error = None;
                    })
                    .await;
                    let hint = wechat_media_agent_context("video", &rel, None);
                    return spawn_inbound_ask_flow(
                        state,
                        from_user_id,
                        msg,
                        hint,
                        bound_user_key.clone(),
                        prefetched_typing_ticket.clone(),
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
                        return;
                    }
                    let rel = build_wechat_inbox_rel_path(
                        &state.config.file_inbox_dir,
                        &from_user_id,
                        &format!("{}_{}", current_ts_ms(), safe_name),
                    );
                    let abs = state.workspace_root.join(&rel);
                    if let Some(parent) = abs.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    if tokio::fs::write(&abs, &bytes).await.is_err() {
                        warn!("wechatd: failed to write inbound file {}", rel);
                        return;
                    }
                    update_status(&state, |status| {
                        status.healthy = true;
                        status.status = "message_received".to_string();
                        status.last_event_ts = msg.create_time_ms.or(Some(current_ts_ms()));
                        status.last_peer = Some(from_user_id.clone());
                        status.last_error = None;
                    })
                    .await;
                    if inbox_rel_suits_doc_parse(&rel) {
                        return spawn_inbound_skill_flow(
                            state,
                            from_user_id,
                            msg,
                            "doc_parse",
                            json!({
                                "action": "parse_doc",
                                "path": rel,
                                "max_chars": 12000,
                                "include_metadata": true,
                                "table_mode": "basic"
                            }),
                            bound_user_key.clone(),
                            prefetched_typing_ticket.clone(),
                        )
                        .await;
                    }
                    let hint = wechat_media_agent_context("file", &rel, Some(&safe_name));
                    return spawn_inbound_ask_flow(
                        state,
                        from_user_id,
                        msg,
                        hint,
                        bound_user_key.clone(),
                        prefetched_typing_ticket.clone(),
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
                        return;
                    }
                    let ts = current_ts_ms();
                    let (rel, data_to_write) =
                        if let Some(wav) = wechat_silk_wav::try_silk_to_wav(&bytes) {
                            (
                                build_wechat_inbox_rel_path(
                                    &state.config.audio_inbox_dir,
                                    &from_user_id,
                                    &format!("v{}.wav", ts),
                                ),
                                wav,
                            )
                        } else {
                            (
                                build_wechat_inbox_rel_path(
                                    &state.config.audio_inbox_dir,
                                    &from_user_id,
                                    &format!("v{}.bin", ts),
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
                        return;
                    }
                    update_status(&state, |status| {
                        status.healthy = true;
                        status.status = "message_received".to_string();
                        status.last_event_ts = msg.create_time_ms.or(Some(current_ts_ms()));
                        status.last_peer = Some(from_user_id.clone());
                        status.last_error = None;
                    })
                    .await;
                    return spawn_inbound_skill_flow(
                        state,
                        from_user_id,
                        msg,
                        "audio_transcribe",
                        json!({ "audio": { "path": rel } }),
                        bound_user_key.clone(),
                        prefetched_typing_ticket.clone(),
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
            return;
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
        &from_user_id,
        msg.context_token.as_deref(),
        Some(text.as_str()),
    )
    .await
    else {
        return;
    };
    tokio::spawn(submit_wechat_task_and_reply(
        state,
        from_user_id,
        text,
        msg.context_token,
        Some(identity.user_key),
        prefetched_typing_ticket,
    ));
}
