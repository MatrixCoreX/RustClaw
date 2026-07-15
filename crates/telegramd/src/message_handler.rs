use super::*;

pub(super) async fn handle_message(bot: Bot, msg: Message, state: BotState) -> anyhow::Result<()> {
    let platform_user_id = msg
        .from()
        .map(|u| i64::try_from(u.id.0).unwrap_or_default())
        .unwrap_or_default();
    let platform_username = msg.from().and_then(|u| u.username.clone());
    let platform_chat_id = msg.chat.id.0;
    let text = msg.text().unwrap_or_default();
    let slash_command = state.command_catalog.match_command(text, "telegram");
    let core_action = slash_command
        .as_ref()
        .and_then(|command| command.definition.core_action());
    let skill_command = slash_command
        .as_ref()
        .and_then(|command| command.definition.skill_name());
    let logged_text = sanitize_message_text_for_log(text);
    info!(
        "handle_message: chat_id={} user_id={} username={} text={}",
        platform_chat_id,
        platform_user_id,
        platform_username.as_deref().unwrap_or("-"),
        logged_text
    );

    if !telegram_user_allowed(&state, platform_user_id, platform_username.as_deref()) {
        info!(
            "telegram access denied: bot_name={} chat_id={} user_id={} username={} access_mode={}",
            state.bot_name,
            platform_chat_id,
            platform_user_id,
            platform_username.as_deref().unwrap_or("-"),
            state.access_mode
        );
        return Ok(());
    }

    let bound_identity = match resolve_telegram_identity(&state, platform_user_id, platform_chat_id)
        .await?
    {
        Some(identity) => {
            set_expect_key_reply(&state, platform_chat_id, false);
            store_bound_identity(&state, platform_chat_id, &identity);
            Some(identity)
        }
        None => {
            let maybe_candidate =
                extract_bind_key_candidate(text, should_expect_key_reply(&state, platform_chat_id));
            if let Some(candidate) = maybe_candidate {
                if let Some(identity) =
                    bind_telegram_identity(&state, platform_user_id, platform_chat_id, &candidate)
                        .await?
                {
                    set_expect_key_reply(&state, platform_chat_id, false);
                    store_bound_identity(&state, platform_chat_id, &identity);
                    bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.bind_success"))
                        .await
                        .context("send key bind success failed")?;
                    return Ok(());
                } else {
                    set_expect_key_reply(&state, platform_chat_id, true);
                    bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.bind_invalid"))
                        .await
                        .context("send invalid key failed")?;
                    return Ok(());
                }
            }
            None
        }
    };
    if bound_identity.is_none() {
        if is_unbound_allowed_command(state.command_catalog.as_ref(), "telegram", text) {
            send_bind_key_required_prompt(&bot, &msg, &state).await?;
            return Ok(());
        }
        send_bind_key_required_prompt(&bot, &msg, &state).await?;
        return Ok(());
    }
    let user_id = bound_identity
        .as_ref()
        .map(|identity| identity.user_id)
        .unwrap_or(platform_user_id);
    // 管理员仅由绑定 key 的 role 决定，不再使用 config 中的 admins 列表
    let is_admin = bound_identity
        .as_ref()
        .is_some_and(|identity| identity.role.eq_ignore_ascii_case("admin"));

    // If user sends an image without text:
    // - auto_vision_on_image_only=true: save + auto-run image_vision
    // - auto_vision_on_image_only=false: save only and reply saved path
    if text.trim().is_empty() {
        if let Some((file_id, ext)) = extract_image_attachment(&msg) {
            if state.auto_vision_on_image_only {
                return handle_image_only_message(&bot, &msg, &state, user_id, file_id, &ext).await;
            }
            return handle_image_only_save_only(&bot, &msg, &state, user_id, file_id, &ext).await;
        }
        if let Some((file_id, ext)) = extract_video_attachment(&msg) {
            return handle_video_message(&bot, &msg, &state, user_id, file_id, &ext).await;
        }
        if let Some((file_id, ext)) = extract_file_attachment(&msg) {
            return handle_file_message(&bot, &msg, &state, user_id, file_id, &ext).await;
        }
        if let Some((file_id, ext)) = extract_audio_attachment(&msg) {
            return handle_audio_message(&bot, &msg, &state, user_id, file_id, &ext).await;
        }
    }
    if matches!(core_action, Some(CoreCommandAction::Start)) {
        let reply = if slash_command
            .as_ref()
            .is_some_and(|command| command.invoked_name_matches("start"))
        {
            state.i18n.t("telegram.msg.start")
        } else {
            state.i18n.t("telegram.msg.help")
        };
        bot.send_message(msg.chat.id, reply)
            .await
            .context("send /start or /help reply failed")?;
        return Ok(());
    }

    if matches!(core_action, Some(CoreCommandAction::RustclawConfig)) {
        if !is_admin {
            bot.send_message(
                msg.chat.id,
                state.i18n.t("telegram.msg.openclaw_admin_only"),
            )
            .await
            .context("send /rustclaw unauthorized failed")?;
            return Ok(());
        }
        let state_for_cmd = state.clone();
        let command_tail = slash_command
            .as_ref()
            .map(|command| command.tail.clone())
            .unwrap_or_default();
        let openclaw_result = tokio::task::spawn_blocking(move || {
            handle_openclaw_config_command(&state_for_cmd, &command_tail)
        })
        .await
        .map_err(|err| anyhow!("join rustclaw config task failed: {err}"))?;
        match openclaw_result {
            Ok(reply) => {
                bot.send_message(msg.chat.id, reply)
                    .await
                    .context("send /rustclaw reply failed")?;
            }
            Err(err) => {
                bot.send_message(
                    msg.chat.id,
                    state
                        .i18n
                        .t_with("telegram.msg.config_failed", &[("error", &err.to_string())]),
                )
                .await
                .context("send /rustclaw error failed")?;
            }
        }
        return Ok(());
    }

    if matches!(core_action, Some(CoreCommandAction::CryptoApi)) {
        let Some(identity) = bound_identity.as_ref() else {
            send_bind_key_required_prompt(&bot, &msg, &state)
                .await
                .context("send key prompt for /cryptoapi failed")?;
            return Ok(());
        };
        let raw = slash_command
            .as_ref()
            .map(|command| command.tail.as_str())
            .unwrap_or_default();
        match handle_cryptoapi_command(&state, identity, raw).await {
            Ok(reply) => {
                if raw.to_ascii_lowercase().starts_with("set ") {
                    clear_pending_resume_for_chat(&state, msg.chat.id.0);
                }
                bot.send_message(msg.chat.id, reply)
                    .await
                    .context("send /cryptoapi reply failed")?;
            }
            Err(err) => {
                bot.send_message(
                    msg.chat.id,
                    state.i18n.t_with(
                        "telegram.msg.cryptoapi_config_failed",
                        &[("error", &err.to_string())],
                    ),
                )
                .await
                .context("send /cryptoapi error failed")?;
            }
        }
        return Ok(());
    }

    if matches!(core_action, Some(CoreCommandAction::VoiceMode)) {
        if !can_change_voice_mode(&state, user_id) {
            bot.send_message(
                msg.chat.id,
                state.i18n.t("telegram.msg.voicemode_admin_only"),
            )
            .await
            .context("send /voicemode unauthorized failed")?;
            return Ok(());
        }
        let mode = slash_command
            .as_ref()
            .map(|command| command.tail.as_str())
            .unwrap_or_default();
        let reply = handle_voicemode_command(&state, msg.chat.id.0, mode)?;
        info!(
            "voice mode command: source=slash chat_id={} user_id={} command={}",
            msg.chat.id.0, user_id, mode
        );
        bot.send_message(msg.chat.id, reply)
            .await
            .context("send /voicemode reply failed")?;
        return Ok(());
    }

    if state.voice_mode_nl_intent_enabled {
        if let Some(mode) =
            detect_voice_mode_intent_with_llm(&state, user_id, msg.chat.id.0, text).await
        {
            if mode == "none" {
                // no-op, fall through to normal ask flow
            } else {
                if !can_change_voice_mode(&state, user_id) {
                    bot.send_message(
                        msg.chat.id,
                        state.i18n.t("telegram.msg.voicemode_admin_only"),
                    )
                    .await
                    .context("send nl voicemode unauthorized failed")?;
                    return Ok(());
                }
                let reply = match mode {
                    "reset" => {
                        set_chat_voice_mode(&state, msg.chat.id.0, None)?;
                        let global_mode = normalize_voice_reply_mode(&state.voice_reply_mode)
                            .unwrap_or_else(|| "voice".to_string());
                        state.i18n.t_with(
                            "telegram.msg.voicemode_reset_ok",
                            &[("global_mode", &global_mode)],
                        )
                    }
                    "show" => {
                        let chat_mode = effective_voice_reply_mode_for_chat(&state, msg.chat.id.0);
                        let global_mode = normalize_voice_reply_mode(&state.voice_reply_mode)
                            .unwrap_or_else(|| "voice".to_string());
                        state.i18n.t_with(
                            "telegram.msg.voicemode_show",
                            &[("chat_mode", &chat_mode), ("global_mode", &global_mode)],
                        )
                    }
                    _ => {
                        set_chat_voice_mode(&state, msg.chat.id.0, Some(mode))?;
                        state
                            .i18n
                            .t_with("telegram.msg.voicemode_set_ok_nl", &[("mode", mode)])
                    }
                };
                info!(
                    "voice mode command: source=nl_llm chat_id={} user_id={} mode={}",
                    msg.chat.id.0, user_id, mode
                );
                bot.send_message(msg.chat.id, reply)
                    .await
                    .context("send nl voicemode reply failed")?;
                return Ok(());
            }
        }
    }

    if matches!(core_action, Some(CoreCommandAction::Status)) {
        match fetch_status_text(&state, msg.chat.id.0).await {
            Ok(status_text) => {
                bot.send_message(msg.chat.id, status_text)
                    .await
                    .context("send /status reply failed")?;
            }
            Err(err) => {
                bot.send_message(
                    msg.chat.id,
                    state.i18n.t_with(
                        "telegram.msg.read_status_failed",
                        &[("error", &err.to_string())],
                    ),
                )
                .await
                .context("send /status error failed")?;
            }
        }
        return Ok(());
    }

    if matches!(core_action, Some(CoreCommandAction::Cancel)) {
        match cancel_tasks_for_chat(&state, user_id, msg.chat.id.0).await {
            Ok(canceled) => {
                let reply = if canceled > 0 {
                    state.i18n.t_with(
                        "telegram.msg.cancel_ok",
                        &[("count", &canceled.to_string())],
                    )
                } else {
                    state.i18n.t("telegram.msg.cancel_none")
                };
                bot.send_message(msg.chat.id, reply)
                    .await
                    .context("send /cancel reply failed")?;
            }
            Err(err) => {
                bot.send_message(
                    msg.chat.id,
                    state
                        .i18n
                        .t_with("telegram.msg.cancel_failed", &[("error", &err.to_string())]),
                )
                .await
                .context("send /cancel error failed")?;
            }
        }
        return Ok(());
    }

    if matches!(core_action, Some(CoreCommandAction::Skills)) {
        bot.send_message(msg.chat.id, run_skill_help_text(&state))
            .await
            .context("send /skills reply failed")?;
        return Ok(());
    }

    if matches!(skill_command, Some("crypto")) {
        let raw = slash_command
            .as_ref()
            .map(|command| command.tail.as_str())
            .unwrap_or_default();
        if raw.to_ascii_lowercase().starts_with("add ") {
            if !is_admin {
                bot.send_message(
                    msg.chat.id,
                    state.i18n.t("telegram.msg.cryptoapi_admin_only"),
                )
                .await
                .context("send /crypto add unauthorized failed")?;
                return Ok(());
            }
            let Some(identity) = bound_identity.as_ref() else {
                send_bind_key_required_prompt(&bot, &msg, &state)
                    .await
                    .context("send key prompt for /crypto add failed")?;
                return Ok(());
            };
            match handle_cryptoapi_command(&state, identity, raw).await {
                Ok(reply) => {
                    bot.send_message(msg.chat.id, reply)
                        .await
                        .context("send /crypto add reply failed")?;
                }
                Err(err) => {
                    bot.send_message(
                        msg.chat.id,
                        state.i18n.t_with(
                            "telegram.msg.cryptoapi_config_failed",
                            &[("error", &err.to_string())],
                        ),
                    )
                    .await
                    .context("send /crypto add error failed")?;
                }
            }
            return Ok(());
        }
        let payload = match build_crypto_skill_payload(raw) {
            Ok(Some(v)) => v,
            Ok(None) => {
                bot.send_message(msg.chat.id, crypto_command_usage_text(&state))
                    .await
                    .context("send /crypto usage failed")?;
                return Ok(());
            }
            Err(err) => {
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "{}\n\n{}",
                        state.i18n.t_with(
                            "telegram.msg.crypto_parse_failed",
                            &[("error", &err.to_string())],
                        ),
                        crypto_command_usage_text(&state)
                    ),
                )
                .await
                .context("send /crypto parse error failed")?;
                return Ok(());
            }
        };

        let queue_len = match fetch_queue_length(&state, msg.chat.id.0).await {
            Ok(v) => v,
            Err(_) => 0,
        };
        if queue_len >= state.queue_limit {
            bot.send_message(
                msg.chat.id,
                format!(
                    "{}",
                    state.i18n.t_with(
                        "telegram.msg.queue_full",
                        &[
                            ("queued", &queue_len.to_string()),
                            ("limit", &state.queue_limit.to_string()),
                        ],
                    )
                ),
            )
            .await
            .context("send queue full /crypto failed")?;
            return Ok(());
        }

        match submit_task_only(&state, user_id, msg.chat.id.0, TaskKind::RunSkill, payload).await {
            Ok(task_id) => {
                spawn_task_result_delivery(
                    bot.clone(),
                    state.clone(),
                    msg.chat.id,
                    user_id,
                    task_id,
                    None,
                    state.i18n.t("telegram.msg.skill_exec_failed"),
                );
            }
            Err(err) => {
                bot.send_message(
                    msg.chat.id,
                    state.i18n.t_with(
                        "telegram.msg.skill_exec_failed_with_error",
                        &[("error", &err.to_string())],
                    ),
                )
                .await
                .context("send /crypto error failed")?;
            }
        }
        return Ok(());
    }

    if matches!(core_action, Some(CoreCommandAction::RunSkill)) {
        let rest = slash_command
            .as_ref()
            .map(|command| command.tail.as_str())
            .unwrap_or_default();
        if rest.is_empty() {
            bot.send_message(msg.chat.id, run_skill_help_text(&state))
                .await
                .context("send /run usage failed")?;
            return Ok(());
        }

        let mut parts = rest.splitn(2, ' ');
        let skill_name = parts.next().unwrap_or_default().trim();
        let args = parts.next().unwrap_or_default().trim();

        if skill_name.is_empty() {
            bot.send_message(msg.chat.id, run_skill_help_text(&state))
                .await
                .context("send /run usage2 failed")?;
            return Ok(());
        }

        let queue_len = match fetch_queue_length(&state, msg.chat.id.0).await {
            Ok(v) => v,
            Err(_) => 0,
        };
        if queue_len >= state.queue_limit {
            bot.send_message(
                msg.chat.id,
                format!(
                    "{}",
                    state.i18n.t_with(
                        "telegram.msg.queue_full",
                        &[
                            ("queued", &queue_len.to_string()),
                            ("limit", &state.queue_limit.to_string()),
                        ],
                    )
                ),
            )
            .await
            .context("send queue full message failed")?;
            return Ok(());
        }

        let payload = json!({
            "skill_name": skill_name,
            "args": args,
        });

        match submit_task_only(&state, user_id, msg.chat.id.0, TaskKind::RunSkill, payload).await {
            Ok(task_id) => {
                spawn_task_result_delivery(
                    bot.clone(),
                    state.clone(),
                    msg.chat.id,
                    user_id,
                    task_id,
                    None,
                    state.i18n.t("telegram.msg.skill_exec_failed"),
                );
            }
            Err(err) => {
                bot.send_message(
                    msg.chat.id,
                    state.i18n.t_with(
                        "telegram.msg.skill_exec_failed_with_error",
                        &[("error", &err.to_string())],
                    ),
                )
                .await
                .context("send /run error failed")?;
            }
        }
        return Ok(());
    }

    if matches!(core_action, Some(CoreCommandAction::SendFile)) {
        let raw = slash_command
            .as_ref()
            .map(|command| command.tail.as_str())
            .unwrap_or_default();
        if raw.is_empty() {
            bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.sendfile_usage"))
                .await
                .context("send /sendfile usage failed")?;
            return Ok(());
        }
        if state.sendfile_admin_only && !is_admin {
            bot.send_message(
                msg.chat.id,
                state.i18n.t("telegram.msg.sendfile_admin_only"),
            )
            .await
            .context("send /sendfile admin-only rejection failed")?;
            return Ok(());
        }
        let path = normalize_path_token(raw);
        let p = match resolve_sendfile_path(
            path,
            state.sendfile_full_access,
            state.sendfile_allowed_dirs.as_ref(),
        ) {
            Ok(v) => v,
            Err(err) => {
                bot.send_message(
                    msg.chat.id,
                    state
                        .i18n
                        .t_with("telegram.msg.sendfile_invalid_path", &[("error", &err)]),
                )
                .await
                .context("send /sendfile path rejection failed")?;
                return Ok(());
            }
        };
        if !p.exists() {
            bot.send_message(
                msg.chat.id,
                state.i18n.t_with(
                    "telegram.msg.file_not_found",
                    &[("path", &p.display().to_string())],
                ),
            )
            .await
            .context("send /sendfile not found failed")?;
            return Ok(());
        }
        if !p.is_file() {
            bot.send_message(
                msg.chat.id,
                state.i18n.t_with(
                    "telegram.msg.not_a_file",
                    &[("path", &p.display().to_string())],
                ),
            )
            .await
            .context("send /sendfile not file failed")?;
            return Ok(());
        }
        let path_s = p.display().to_string();
        if is_image_file(&path_s) {
            bot.send_photo(msg.chat.id, InputFile::file(path_s))
                .await
                .context("send /sendfile image failed")?;
        } else {
            bot.send_document(msg.chat.id, InputFile::file(path_s))
                .await
                .context("send /sendfile document failed")?;
        }
        return Ok(());
    }

    let prompt = if matches!(core_action, Some(CoreCommandAction::Ask)) {
        slash_command
            .as_ref()
            .map(|command| command.tail.as_str())
            .unwrap_or_default()
    } else {
        // Paths like /home/user/file and other non-command /-prefixed text go to ask unchanged.
        text.trim()
    };

    if prompt.is_empty() {
        bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.empty_prompt"))
            .await
            .context("send empty prompt reply failed")?;
        return Ok(());
    }

    if maybe_handle_resume_continuation(&bot, &msg, &state, user_id, prompt).await? {
        return Ok(());
    }

    // Two-step image edit flow when auto vision is disabled:
    // 1) user sends image only -> saved as pending image for this chat
    // 2) user sends prompt text -> run image_edit directly using pending image
    if !state.auto_vision_on_image_only {
        let pending_image = state
            .pending_image_by_chat
            .lock()
            .ok()
            .and_then(|m| m.get(&msg.chat.id.0).cloned());
        if let Some(image_path) = pending_image {
            let queue_len = match fetch_queue_length(&state, msg.chat.id.0).await {
                Ok(v) => v,
                Err(_) => 0,
            };
            if queue_len >= state.queue_limit {
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "{}",
                        state.i18n.t_with(
                            "telegram.msg.queue_full",
                            &[
                                ("queued", &queue_len.to_string()),
                                ("limit", &state.queue_limit.to_string()),
                            ],
                        )
                    ),
                )
                .await
                .context("send queue full image-edit message failed")?;
                return Ok(());
            }
            let payload = json!({
                "skill_name": "image_edit",
                "args": {
                    "action": "edit",
                    "image": {"path": image_path},
                    "instruction": prompt
                }
            });
            match submit_task_only(&state, user_id, msg.chat.id.0, TaskKind::RunSkill, payload)
                .await
            {
                Ok(task_id) => {
                    if let Ok(mut m) = state.pending_image_by_chat.lock() {
                        m.remove(&msg.chat.id.0);
                    }
                    spawn_task_result_delivery(
                        bot.clone(),
                        state.clone(),
                        msg.chat.id,
                        user_id,
                        task_id,
                        None,
                        state.i18n.t("telegram.msg.skill_exec_failed"),
                    );
                }
                Err(err) => {
                    bot.send_message(
                        msg.chat.id,
                        state.i18n.t_with(
                            "telegram.msg.skill_exec_failed_with_error",
                            &[("error", &err.to_string())],
                        ),
                    )
                    .await
                    .context("send pending image edit submit error failed")?;
                }
            }
            return Ok(());
        }
    }

    let queue_len = match fetch_queue_length(&state, msg.chat.id.0).await {
        Ok(v) => v,
        Err(_) => 0,
    };
    if queue_len >= state.queue_limit {
        bot.send_message(
            msg.chat.id,
            format!(
                "{}",
                state.i18n.t_with(
                    "telegram.msg.queue_full",
                    &[
                        ("queued", &queue_len.to_string()),
                        ("limit", &state.queue_limit.to_string()),
                    ],
                )
            ),
        )
        .await
        .context("send queue full ask message failed")?;
        return Ok(());
    }
    match submit_task_only(
        &state,
        user_id,
        msg.chat.id.0,
        TaskKind::Ask,
        json!({ "text": prompt }),
    )
    .await
    {
        Ok(task_id) => {
            info!(
                "telegramd: submitted ask task_id={} user_id={} chat_id={}",
                task_id, user_id, msg.chat.id.0
            );
            spawn_task_result_delivery(
                bot.clone(),
                state.clone(),
                msg.chat.id,
                user_id,
                task_id,
                None,
                state.i18n.t("telegram.msg.process_failed"),
            );
        }
        Err(err) => {
            bot.send_message(
                msg.chat.id,
                state.i18n.t_with(
                    "telegram.msg.process_failed_with_error",
                    &[("error", &err.to_string())],
                ),
            )
            .await
            .context("send ask error failed")?;
        }
    }

    Ok(())
}
