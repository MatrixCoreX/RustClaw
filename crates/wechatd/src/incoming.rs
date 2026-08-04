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
    let Some(task_context) =
        pin_inbound_task_context(&state, &from_user_id, msg.context_token.as_deref()).await
    else {
        warn!("wechatd: inbound message skipped because task context could not be pinned");
        return;
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
        let pending_message_id = format!(
            "{}:{}:{}",
            from_user_id,
            msg.create_time_ms.unwrap_or_default(),
            pending_attachment_kind.unwrap_or("media")
        );
        let Some(identity) = ensure_bound_before_task(
            &state,
            &task_context,
            &from_user_id,
            None,
            Some(&pending_message_id),
            pending_attachment_kind,
        )
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
                    return spawn_inbound_attachment_flow(
                        state,
                        task_context.clone(),
                        "image",
                        rel,
                        "image/jpeg",
                        bytes.len() as u64,
                        bound_user_key.clone(),
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
                    return spawn_inbound_attachment_flow(
                        state,
                        task_context.clone(),
                        "video",
                        rel,
                        "video/mp4",
                        bytes.len() as u64,
                        bound_user_key.clone(),
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
                    return spawn_inbound_attachment_flow(
                        state,
                        task_context.clone(),
                        "file",
                        rel,
                        "application/octet-stream",
                        bytes.len() as u64,
                        bound_user_key.clone(),
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

    let pending_message_id = format!(
        "{}:{}:{}",
        from_user_id,
        msg.create_time_ms.unwrap_or_default(),
        stable_i64_from_string(&text)
    );
    let Some(identity) = ensure_bound_before_task(
        &state,
        &task_context,
        &from_user_id,
        Some(text.as_str()),
        Some(&pending_message_id),
        None,
    )
    .await
    else {
        return;
    };
    tokio::spawn(submit_wechat_task_and_reply(
        state,
        task_context,
        text,
        Some(identity.user_key),
    ));
}
