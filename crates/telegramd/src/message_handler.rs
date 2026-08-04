use super::*;

pub(super) async fn handle_message(bot: Bot, msg: Message, state: BotState) -> anyhow::Result<()> {
    let platform_user_id = msg
        .from()
        .map(|u| i64::try_from(u.id.0).unwrap_or_default())
        .unwrap_or_default();
    let platform_username = msg.from().and_then(|u| u.username.clone());
    let platform_chat_id = msg.chat.id.0;
    let text = msg.text().or_else(|| msg.caption()).unwrap_or_default();
    let slash_command = state.command_catalog.match_command(text, "telegram");
    let core_action = slash_command
        .as_ref()
        .and_then(|command| command.definition.core_action());
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

    if !msg.chat.is_private() && text.trim().starts_with("/key") {
        send_bind_key_required_prompt(&bot, &msg, &state).await?;
        return Ok(());
    }

    let explicit_bind_candidate = msg
        .chat
        .is_private()
        .then(|| {
            text.trim()
                .strip_prefix("/key")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        })
        .flatten();
    if let Some(candidate) = explicit_bind_candidate {
        if let Some(bind_result) =
            bind_telegram_identity(&state, platform_user_id, platform_chat_id, &candidate).await?
        {
            let identity = bind_result.identity;
            set_expect_key_reply(&state, platform_chat_id, false);
            store_bound_identity(&state, platform_chat_id, &identity);
            bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.bind_success"))
                .await
                .context("send key bind success failed")?;
            if let Some(resume) = bind_result.pending_resume {
                if let Some(task_id) = resume.task_id {
                    let delivery_chat = resume
                        .external_chat_id
                        .as_deref()
                        .and_then(|value| value.parse::<i64>().ok())
                        .map(ChatId)
                        .unwrap_or(msg.chat.id);
                    spawn_task_result_delivery(
                        bot.clone(),
                        state.clone(),
                        delivery_chat,
                        identity.user_id,
                        task_id.to_string(),
                        None,
                        state.i18n.t("telegram.msg.process_failed"),
                    );
                } else if resume.error_code.is_some() {
                    bot.send_message(
                        msg.chat.id,
                        state.i18n.t("telegram.msg.pending_resume_stopped"),
                    )
                    .await
                    .context("send pending resume stopped failed")?;
                }
            }
        } else {
            set_expect_key_reply(&state, platform_chat_id, true);
            bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.bind_invalid"))
                .await
                .context("send invalid key failed")?;
        }
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
            let maybe_candidate = msg
                .chat
                .is_private()
                .then(|| {
                    extract_bind_key_candidate(
                        text,
                        should_expect_key_reply(&state, platform_chat_id),
                    )
                })
                .flatten();
            if let Some(candidate) = maybe_candidate {
                if let Some(bind_result) =
                    bind_telegram_identity(&state, platform_user_id, platform_chat_id, &candidate)
                        .await?
                {
                    let identity = bind_result.identity;
                    set_expect_key_reply(&state, platform_chat_id, false);
                    store_bound_identity(&state, platform_chat_id, &identity);
                    bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.bind_success"))
                        .await
                        .context("send key bind success failed")?;
                    if let Some(resume) = bind_result.pending_resume {
                        if let Some(task_id) = resume.task_id {
                            let delivery_chat = resume
                                .external_chat_id
                                .as_deref()
                                .and_then(|value| value.parse::<i64>().ok())
                                .map(ChatId)
                                .unwrap_or(msg.chat.id);
                            spawn_task_result_delivery(
                                bot.clone(),
                                state.clone(),
                                delivery_chat,
                                identity.user_id,
                                task_id.to_string(),
                                None,
                                state.i18n.t("telegram.msg.process_failed"),
                            );
                        } else if resume.error_code.is_some() {
                            bot.send_message(
                                msg.chat.id,
                                state.i18n.t("telegram.msg.pending_resume_stopped"),
                            )
                            .await
                            .context("send pending resume stopped failed")?;
                        }
                    }
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
        let may_persist =
            msg.chat.is_private() || !should_expect_key_reply(&state, platform_chat_id);
        if may_persist {
            let pending_media =
                match store_pending_telegram_attachment(&bot, &msg, &state, platform_user_id, text)
                    .await
                {
                    Ok(stored) => stored,
                    Err(error) => {
                        warn!(
                            "telegramd: pending attachment persistence failed chat_id={} error={}",
                            platform_chat_id, error
                        );
                        false
                    }
                };
            if !pending_media {
                if let Err(error) = store_pending_telegram_request(
                    &state,
                    platform_user_id,
                    platform_chat_id,
                    msg.id.0.to_string(),
                    text,
                )
                .await
                {
                    warn!(
                        "telegramd: pending request persistence failed chat_id={} error={}",
                        platform_chat_id, error
                    );
                }
            }
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

    if slash_command
        .as_ref()
        .is_some_and(|command| command.definition.admin_only && !is_admin)
    {
        bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.admin_command_only"))
            .await
            .context("send admin command rejection failed")?;
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
                warn!(chat_id = msg.chat.id.0, error = %err, "task cancellation failed");
                bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.cancel_failed"))
                    .await
                    .context("send /cancel error failed")?;
            }
        }
        return Ok(());
    }

    // Unknown slash-prefixed text is an ordinary request. The agent loop owns its semantics.
    let prompt = text.trim();

    if let Some((file_id, ext)) = extract_image_attachment(&msg) {
        return handle_image_message(&bot, &msg, &state, user_id, file_id, &ext, prompt).await;
    }
    if let Some((file_id, ext)) = extract_audio_attachment(&msg) {
        return handle_audio_message(&bot, &msg, &state, user_id, file_id, &ext, prompt).await;
    }
    if let Some((file_id, ext)) = extract_video_attachment(&msg) {
        return handle_video_message(&bot, &msg, &state, user_id, file_id, &ext, prompt).await;
    }
    if let Some((file_id, ext)) = extract_file_attachment(&msg) {
        return handle_file_message(&bot, &msg, &state, user_id, file_id, &ext, prompt).await;
    }

    if prompt.is_empty() {
        bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.empty_prompt"))
            .await
            .context("send empty prompt reply failed")?;
        return Ok(());
    }

    if maybe_handle_resume_continuation(&bot, &msg, &state, user_id, prompt).await? {
        return Ok(());
    }

    match submit_task_only(
        &state,
        user_id,
        msg.chat.id.0,
        Some(msg.id.0.to_string()),
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
            warn!(chat_id = msg.chat.id.0, error = %err, "task submission failed");
            bot.send_message(
                msg.chat.id,
                state.i18n.t("telegram.msg.process_failed_with_error"),
            )
            .await
            .context("send ask error failed")?;
        }
    }

    Ok(())
}
