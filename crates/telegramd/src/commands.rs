use super::*;

pub(super) fn handle_voicemode_command(
    state: &BotState,
    chat_id: i64,
    command_tail: &str,
) -> anyhow::Result<String> {
    let rest = command_tail.trim();
    if rest.is_empty() {
        return Ok(state.i18n.t("telegram.msg.voicemode_usage"));
    }
    match rest {
        "show" => {
            let chat_mode = effective_voice_reply_mode_for_chat(state, chat_id);
            let global_mode = normalize_voice_reply_mode(&state.voice_reply_mode)
                .unwrap_or_else(|| "voice".to_string());
            Ok(state.i18n.t_with(
                "telegram.msg.voicemode_show",
                &[("chat_mode", &chat_mode), ("global_mode", &global_mode)],
            ))
        }
        "voice" | "text" | "both" => {
            set_chat_voice_mode(state, chat_id, Some(rest))?;
            Ok(state
                .i18n
                .t_with("telegram.msg.voicemode_set_ok", &[("mode", rest)]))
        }
        "reset" => {
            set_chat_voice_mode(state, chat_id, None)?;
            let global_mode = normalize_voice_reply_mode(&state.voice_reply_mode)
                .unwrap_or_else(|| "voice".to_string());
            Ok(state.i18n.t_with(
                "telegram.msg.voicemode_reset_ok",
                &[("global_mode", &global_mode)],
            ))
        }
        _ => Ok(state.i18n.t("telegram.msg.voicemode_usage")),
    }
}

pub(super) fn set_chat_voice_mode(
    state: &BotState,
    chat_id: i64,
    mode: Option<&str>,
) -> anyhow::Result<()> {
    let normalized = mode.and_then(normalize_voice_reply_mode);
    let previous = {
        let mut map = state
            .voice_reply_mode_by_chat
            .lock()
            .map_err(|_| anyhow!("voice mode map lock poisoned"))?;
        let old = map.get(&chat_id).cloned();
        if let Some(new_mode) = &normalized {
            map.insert(chat_id, new_mode.clone());
        } else {
            map.remove(&chat_id);
        }
        old
    };
    if let Err(err) = persist_chat_voice_mode_to_config(state, chat_id, normalized.as_deref()) {
        let mut map = state
            .voice_reply_mode_by_chat
            .lock()
            .map_err(|_| anyhow!("voice mode map lock poisoned"))?;
        if let Some(old_mode) = previous {
            map.insert(chat_id, old_mode);
        } else {
            map.remove(&chat_id);
        }
        return Err(err);
    }
    Ok(())
}

fn persist_chat_voice_mode_to_config(
    state: &BotState,
    chat_id: i64,
    mode: Option<&str>,
) -> anyhow::Result<()> {
    let cfg_path = if Path::new("configs/channels/telegram.toml").exists() {
        Path::new("configs/channels/telegram.toml")
    } else {
        Path::new("configs/config.toml")
    };
    let raw =
        fs::read_to_string(cfg_path).context(state.i18n.t("telegram.error.read_config_failed"))?;
    let mut value: TomlValue =
        toml::from_str(&raw).context(state.i18n.t("telegram.error.parse_config_failed"))?;

    let root = value
        .as_table_mut()
        .ok_or_else(|| anyhow!("{}", state.i18n.t("telegram.error.config_not_table")))?;
    let telegram = root
        .entry("telegram")
        .or_insert(TomlValue::Table(toml::map::Map::new()));
    if !telegram.is_table() {
        *telegram = TomlValue::Table(toml::map::Map::new());
    }
    let telegram = telegram
        .as_table_mut()
        .ok_or_else(|| anyhow!("{}", state.i18n.t("telegram.error.config_not_table")))?;
    let by_chat = telegram
        .entry("voice_reply_mode_by_chat")
        .or_insert(TomlValue::Table(toml::map::Map::new()));
    if !by_chat.is_table() {
        *by_chat = TomlValue::Table(toml::map::Map::new());
    }
    let by_chat = by_chat
        .as_table_mut()
        .ok_or_else(|| anyhow!("{}", state.i18n.t("telegram.error.config_not_table")))?;
    let key = chat_id.to_string();
    if let Some(value) = mode {
        by_chat.insert(key, TomlValue::String(value.to_string()));
    } else {
        by_chat.remove(&key);
    }

    let output = toml::to_string_pretty(&value)
        .context(state.i18n.t("telegram.error.serialize_config_failed"))?;
    let temp_path = cfg_path.with_extension("toml.tmp");
    fs::write(&temp_path, output).context(state.i18n.t("telegram.error.write_config_failed"))?;
    fs::rename(&temp_path, cfg_path).context(state.i18n.t("telegram.error.write_config_failed"))?;
    Ok(())
}

pub(super) fn sanitize_message_text_for_log(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.starts_with("/key ") {
        "/key <redacted>".to_string()
    } else {
        text.to_string()
    }
}

pub(super) fn clear_pending_resume_for_chat(state: &BotState, chat_id: i64) {
    if let Ok(mut guard) = state.pending_resume_by_chat.lock() {
        guard.remove(&chat_id);
    }
}
