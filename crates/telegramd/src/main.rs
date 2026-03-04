use std::collections::HashSet;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, anyhow};
use claw_core::config::AppConfig;
use claw_core::hard_rules::types::VoiceModeIntentAliases;
use claw_core::hard_rules::voice_mode::{
    load_voice_mode_intent_aliases, parse_voice_mode_intent_decision,
};
use claw_core::types::{
    ApiResponse, ChannelKind, HealthResponse, SubmitTaskRequest, SubmitTaskResponse, TaskKind,
    TaskQueryResponse, TaskStatus,
};
use reqwest::Client;
use serde_json::{Value as JsonValue, json};
use teloxide::prelude::*;
use teloxide::types::{
    CallbackQuery, ChatAction, InlineKeyboardButton, InlineKeyboardMarkup, InputFile, MediaKind,
    MessageKind,
};
use tokio::sync::oneshot;
use toml::Value as TomlValue;
use tracing::{debug, info, warn};

#[derive(Clone)]
struct BotState {
    admins: Arc<HashSet<i64>>,
    allowlist: Arc<HashSet<i64>>,
    skills_list: Arc<Vec<String>>,
    agent_off_chats: Arc<Mutex<HashSet<i64>>>,
    clawd_base_url: String,
    client: Client,
    poll_interval_ms: u64,
    task_wait_seconds: u64,
    queue_limit: usize,
    auto_vision_on_image_only: bool,
    pending_image_by_chat: Arc<Mutex<HashMap<i64, String>>>,
    bot_token: String,
    image_inbox_dir: String,
    audio_inbox_dir: String,
    voice_reply_mode: String,
    voice_mode_nl_intent_enabled: bool,
    voice_reply_mode_by_chat: Arc<Mutex<HashMap<i64, String>>>,
    voice_mode_intent_aliases: Arc<VoiceModeIntentAliases>,
    max_audio_input_bytes: usize,
    sendfile_admin_only: bool,
    sendfile_full_access: bool,
    sendfile_allowed_dirs: Arc<Vec<String>>,
    ephemeral_image_saved_seconds: u64,
    voice_chat_prompt_template: String,
    voice_mode_intent_prompt_template: String,
    pending_resume_by_chat: Arc<Mutex<HashMap<i64, PendingResumeContext>>>,
    i18n: Arc<TextCatalog>,
}

#[derive(Debug, Clone)]
struct PendingResumeContext {
    user_id: i64,
    created_at_secs: u64,
}

#[derive(Debug, Clone)]
struct TextCatalog {
    current: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VoiceReplyMode {
    Voice,
    Text,
    Both,
}

const DEFAULT_VOICE_CHAT_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/voice_chat_prompt.md");
const DEFAULT_VOICE_MODE_INTENT_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/voice_mode_intent_prompt.md");
const RESUME_CONTEXT_TTL_SECONDS: u64 = 30 * 60;
const VOICE_MODE_INTENT_ALIASES_PATH: &str = "configs/command_intent/voice_mode_intent_aliases.toml";

fn parse_voice_reply_mode(raw: &str) -> VoiceReplyMode {
    match raw.trim().to_ascii_lowercase().as_str() {
        "text" => VoiceReplyMode::Text,
        "both" => VoiceReplyMode::Both,
        _ => VoiceReplyMode::Voice,
    }
}

fn normalize_voice_reply_mode(raw: &str) -> Option<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "voice" => Some("voice".to_string()),
        "text" => Some("text".to_string()),
        "both" => Some("both".to_string()),
        _ => None,
    }
}

fn can_change_voice_mode(state: &BotState, user_id: i64) -> bool {
    state.admins.contains(&user_id) || state.allowlist.contains(&user_id)
}

async fn detect_voice_mode_intent_with_llm(
    state: &BotState,
    user_id: i64,
    chat_id: i64,
    text: &str,
) -> Option<&'static str> {
    if text.trim().is_empty() {
        return None;
    }
    let prompt = render_voice_mode_intent_prompt(&state.voice_mode_intent_prompt_template, text);
    let task_id = match submit_task_only(
        state,
        user_id,
        chat_id,
        TaskKind::Ask,
        json!({ "text": prompt, "agent_mode": false, "source": "voice_mode_intent_detect" }),
    )
    .await
    {
        Ok(id) => id,
        Err(err) => {
            warn!("voice mode llm detect submit failed: {err}");
            return None;
        }
    };
    let out = match poll_task_result(state, &task_id, Some(12)).await {
        Ok(v) => v.into_iter().next().unwrap_or_default(),
        Err(err) => {
            warn!("voice mode llm detect poll failed: {err}");
            return None;
        }
    };
    let decision = parse_voice_mode_intent_decision(&out, state.voice_mode_intent_aliases.as_ref());
    if let Some(d) = decision {
        debug!(
            "voice mode llm detect parsed: chat_id={} user_id={} mode={} confidence={} parser_path={}",
            chat_id,
            user_id,
            d.mode,
            d.confidence.unwrap_or(-1.0),
            d.parser_path
        );
    } else {
        debug!(
            "voice mode llm detect parsed none: chat_id={} user_id={}",
            chat_id, user_id
        );
    }
    decision.map(|d| d.mode)
}

fn pending_resume_valid_for(
    pending: &PendingResumeContext,
    user_id: i64,
    now_secs: u64,
) -> bool {
    if pending.user_id != user_id {
        return false;
    }
    now_secs.saturating_sub(pending.created_at_secs) <= RESUME_CONTEXT_TTL_SECONDS
}

async fn maybe_handle_resume_continuation(
    _bot: &Bot,
    msg: &Message,
    state: &BotState,
    user_id: i64,
    _prompt: &str,
) -> anyhow::Result<bool> {
    let chat_id = msg.chat.id.0;
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let pending = {
        let guard = state
            .pending_resume_by_chat
            .lock()
            .map_err(|_| anyhow!("pending resume lock poisoned"))?;
        guard.get(&chat_id).cloned()
    };
    let Some(pending) = pending else {
        return Ok(false);
    };
    if !pending_resume_valid_for(&pending, user_id, now_secs) {
        if let Ok(mut guard) = state.pending_resume_by_chat.lock() {
            guard.remove(&chat_id);
        }
    }
    // Do not route by reply text in telegram transport layer.
    // Resume/continue decisions must be handled upstream (or by explicit button callbacks).
    Ok(false)
}

fn effective_voice_reply_mode_for_chat(state: &BotState, chat_id: i64) -> String {
    let fallback = normalize_voice_reply_mode(&state.voice_reply_mode).unwrap_or_else(|| "voice".to_string());
    if let Ok(map) = state.voice_reply_mode_by_chat.lock() {
        if let Some(mode) = map.get(&chat_id).and_then(|v| normalize_voice_reply_mode(v)) {
            return mode;
        }
    }
    fallback
}

impl TextCatalog {
    fn load(path: &str) -> anyhow::Result<Self> {
        let raw = fs::read_to_string(path)?;
        let value: TomlValue = toml::from_str(&raw)?;
        let dict = value
            .get("dict")
            .and_then(|v| v.as_table())
            .ok_or_else(|| anyhow!("missing [dict] table in i18n file: {path}"))?;

        let mut current = HashMap::new();
        for (k, v) in dict {
            if let Some(text) = v.as_str() {
                current.insert(k.to_string(), text.to_string());
            }
        }
        Ok(Self { current })
    }

    fn fallback() -> Self {
        let mut current = HashMap::new();
        current.insert(
            "common.unknown_error".to_string(),
            "Unknown error".to_string(),
        );
        Self { current }
    }

    fn t(&self, key: &str) -> String {
        self.current
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }

    fn t_with(&self, key: &str, vars: &[(&str, &str)]) -> String {
        let mut out = self.t(key);
        for (name, value) in vars {
            out = out.replace(&format!("{{{name}}}"), value);
        }
        out
    }
}

fn resolve_i18n_path(language: &str, configured_path: &str) -> String {
    let lang = language.trim();
    if !lang.is_empty() {
        let candidate = format!("configs/i18n/telegramd.{lang}.toml");
        if Path::new(&candidate).exists() {
            return candidate;
        }
    }
    configured_path.to_string()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        // 默认用 info 级别，若设置 RUST_LOG 则以环境变量为准。
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()))
        .with_target(false)
        .compact()
        .init();

    let config = AppConfig::load("configs/config.toml")?;
    let i18n_path = resolve_i18n_path(&config.telegram.language, &config.telegram.i18n_path);
    let i18n = match TextCatalog::load(&i18n_path) {
        Ok(v) => Arc::new(v),
        Err(err) => {
            warn!(
                "load i18n file failed: path={} err={}",
                i18n_path, err
            );
            Arc::new(TextCatalog::fallback())
        }
    };
    let bot = Bot::new(config.telegram.bot_token.clone());
    if let Err(err) =
        register_telegram_commands_and_menu(&config.telegram.bot_token, i18n.as_ref()).await
    {
        warn!("register Telegram menu failed: {err}");
    } else {
        info!("registered Telegram menu commands");
    }

    let mut allowlist = HashSet::new();
    for id in &config.telegram.allowlist {
        allowlist.insert(*id);
    }

    let mut admins = HashSet::new();
    for id in &config.telegram.admins {
        admins.insert(*id);
        allowlist.insert(*id);
    }

    let client = Client::builder()
        .timeout(Duration::from_secs(config.server.request_timeout_seconds))
        .build()
        .context("build reqwest client failed")?;
    let voice_mode_intent_aliases = load_voice_mode_intent_aliases(VOICE_MODE_INTENT_ALIASES_PATH);
    let mut voice_reply_mode_by_chat = HashMap::new();
    for (chat_id_raw, mode_raw) in &config.telegram.voice_reply_mode_by_chat {
        if let (Ok(chat_id), Some(mode)) = (chat_id_raw.parse::<i64>(), normalize_voice_reply_mode(mode_raw)) {
            voice_reply_mode_by_chat.insert(chat_id, mode);
        }
    }

    let state = BotState {
        admins: Arc::new(admins),
        allowlist: Arc::new(allowlist),
        skills_list: Arc::new(config.skills.skills_list.clone()),
        agent_off_chats: Arc::new(Mutex::new(HashSet::new())),
        clawd_base_url: format!("http://{}", config.server.listen),
        client,
        poll_interval_ms: config.worker.poll_interval_ms,
        task_wait_seconds: config.worker.task_timeout_seconds,
        queue_limit: config.worker.queue_limit,
        auto_vision_on_image_only: config.telegram.auto_vision_on_image_only,
        pending_image_by_chat: Arc::new(Mutex::new(HashMap::new())),
        pending_resume_by_chat: Arc::new(Mutex::new(HashMap::new())),
        bot_token: config.telegram.bot_token.clone(),
        image_inbox_dir: "image/upload".to_string(),
        audio_inbox_dir: config.telegram.audio_inbox_dir.clone(),
        voice_reply_mode: config.telegram.voice_reply_mode.clone(),
        voice_mode_nl_intent_enabled: config.telegram.voice_mode_nl_intent_enabled,
        voice_reply_mode_by_chat: Arc::new(Mutex::new(voice_reply_mode_by_chat)),
        voice_mode_intent_aliases: Arc::new(voice_mode_intent_aliases),
        max_audio_input_bytes: config.telegram.max_audio_input_bytes.max(1024),
        sendfile_admin_only: config.telegram.sendfile.admin_only,
        sendfile_full_access: config.telegram.sendfile.full_access,
        sendfile_allowed_dirs: Arc::new(config.telegram.sendfile.allowed_dirs.clone()),
        ephemeral_image_saved_seconds: config.telegram.ephemeral_image_saved_seconds,
        voice_chat_prompt_template: load_prompt_template(
            "prompts/voice_chat_prompt.md",
            DEFAULT_VOICE_CHAT_PROMPT_TEMPLATE,
        ),
        voice_mode_intent_prompt_template: load_prompt_template(
            "prompts/voice_mode_intent_prompt.md",
            DEFAULT_VOICE_MODE_INTENT_PROMPT_TEMPLATE,
        ),
        i18n,
    };

    let mut admins_list: Vec<i64> = state.admins.iter().copied().collect();
    admins_list.sort_unstable();
    let mut allowlist_list: Vec<i64> = state.allowlist.iter().copied().collect();
    allowlist_list.sort_unstable();

    info!(
        "{}",
        state.i18n.t_with(
            "telegram.log.started",
            &[
                ("admins", &format!("{admins_list:?}")),
                ("allowlist", &format!("{allowlist_list:?}")),
                ("skills", &state.skills_list.join(",")),
            ],
        )
    );
    info!(
        "{}",
        state.i18n.t_with(
            "telegram.log.startup_memory_rss",
            &[("bytes", &current_rss_bytes().unwrap_or(0).to_string())]
        )
    );

    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(handle_message))
        .branch(Update::filter_callback_query().endpoint(handle_callback_query));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state])
        .build()
        .dispatch()
        .await;

    Ok(())
}

async fn register_telegram_commands_and_menu(
    bot_token: &str,
    i18n: &TextCatalog,
) -> anyhow::Result<()> {
    let api_base = format!("https://api.telegram.org/bot{bot_token}");
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("build telegram menu client failed")?;

    let commands_payload = json!({
        "commands": [
            { "command": "start", "description": i18n.t("telegram.menu.start_desc") },
            { "command": "help", "description": i18n.t("telegram.menu.help_desc") },
            { "command": "agent", "description": i18n.t("telegram.menu.agent_desc") },
            { "command": "status", "description": i18n.t("telegram.menu.status_desc") },
            { "command": "cancel", "description": i18n.t("telegram.menu.cancel_desc") },
            { "command": "skills", "description": i18n.t("telegram.menu.skills_desc") },
            { "command": "run", "description": i18n.t("telegram.menu.run_desc") },
            { "command": "sendfile", "description": i18n.t("telegram.menu.sendfile_desc") },
            { "command": "voicemode", "description": i18n.t("telegram.menu.voicemode_desc") },
            { "command": "rustclaw", "description": i18n.t("telegram.menu.openclaw_desc") },
            { "command": "crypto", "description": i18n.t("telegram.menu.crypto_desc") },
            { "command": "cryptoapi", "description": i18n.t("telegram.menu.cryptoapi_desc") }
        ]
    });

    let cmd_resp = client
        .post(format!("{api_base}/setMyCommands"))
        .json(&commands_payload)
        .send()
        .await
        .context("request setMyCommands failed")?;
    let cmd_status = cmd_resp.status();
    let cmd_body = cmd_resp
        .text()
        .await
        .context("read setMyCommands response failed")?;
    if !cmd_status.is_success() {
        return Err(anyhow!("setMyCommands http {}: {}", cmd_status, cmd_body));
    }
    let cmd_json: JsonValue =
        serde_json::from_str(&cmd_body).unwrap_or_else(|_| json!({"ok": false}));
    if !cmd_json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        return Err(anyhow!("setMyCommands returned not ok: {}", cmd_body));
    }

    let menu_payload = json!({
        "menu_button": {
            "type": "commands"
        }
    });
    let menu_resp = client
        .post(format!("{api_base}/setChatMenuButton"))
        .json(&menu_payload)
        .send()
        .await
        .context("request setChatMenuButton failed")?;
    let menu_status = menu_resp.status();
    let menu_body = menu_resp
        .text()
        .await
        .context("read setChatMenuButton response failed")?;
    if !menu_status.is_success() {
        return Err(anyhow!("setChatMenuButton http {}: {}", menu_status, menu_body));
    }
    let menu_json: JsonValue =
        serde_json::from_str(&menu_body).unwrap_or_else(|_| json!({"ok": false}));
    if !menu_json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        return Err(anyhow!("setChatMenuButton returned not ok: {}", menu_body));
    }

    Ok(())
}

fn current_rss_bytes() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok())?;
            return Some(kb * 1024);
        }
    }
    None
}

async fn handle_message(bot: Bot, msg: Message, state: BotState) -> anyhow::Result<()> {
    let user_id = msg
        .from
        .as_ref()
        .map(|u| i64::try_from(u.id.0).unwrap_or_default())
        .unwrap_or_default();

    if !state.allowlist.contains(&user_id) {
        warn!(
            "{}",
            state.i18n.t_with(
                "telegram.log.unauthorized_user",
                &[("user_id", &user_id.to_string())]
            )
        );
        bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.unauthorized"))
            .await
            .context("send unauthorized message failed")?;
        return Ok(());
    }

    let text = msg.text().unwrap_or_default();
    info!(
        "handle_message: chat_id={} user_id={} text={}",
        msg.chat.id.0, user_id, text
    );

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
        if let Some((file_id, ext)) = extract_audio_attachment(&msg) {
            return handle_audio_message(&bot, &msg, &state, user_id, file_id, &ext).await;
        }
    }
    if text.starts_with("/start") {
        let reply = state.i18n.t("telegram.msg.start");
        bot.send_message(msg.chat.id, reply)
            .await
            .context("send /start reply failed")?;
        return Ok(());
    }

    if text.starts_with("/help") {
        let reply = state.i18n.t("telegram.msg.help");
        bot.send_message(msg.chat.id, reply)
            .await
            .context("send /help reply failed")?;
        return Ok(());
    }

    if text.starts_with("/rustclaw") || text.starts_with("/openclaw") {
        let is_admin = state.admins.contains(&user_id);
        if !is_admin {
            bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.openclaw_admin_only"))
                .await
                .context("send /rustclaw unauthorized failed")?;
            return Ok(());
        }

        let state_for_cmd = state.clone();
        let text_owned = text.to_string();
        let openclaw_result =
            tokio::task::spawn_blocking(move || handle_openclaw_config_command(&state_for_cmd, &text_owned))
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

    if text.starts_with("/cryptoapi") {
        let is_admin = state.admins.contains(&user_id);
        if !is_admin {
            bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.openclaw_admin_only"))
                .await
                .context("send /cryptoapi unauthorized failed")?;
            return Ok(());
        }
        let raw = text.strip_prefix("/cryptoapi").unwrap_or_default().trim();
        match handle_cryptoapi_command(&state, raw) {
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

    if text.starts_with("/voicemode") {
        if !can_change_voice_mode(&state, user_id) {
            bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.voicemode_admin_only"))
                .await
                .context("send /voicemode unauthorized failed")?;
            return Ok(());
        }
        let mode = text.strip_prefix("/voicemode").unwrap_or_default().trim();
        let reply = handle_voicemode_command(&state, msg.chat.id.0, text)?;
        info!(
            "voice mode command: source=slash chat_id={} user_id={} command={}",
            msg.chat.id.0,
            user_id,
            mode
        );
        bot.send_message(msg.chat.id, reply)
            .await
            .context("send /voicemode reply failed")?;
        return Ok(());
    }

    if state.voice_mode_nl_intent_enabled {
        if let Some(mode) = detect_voice_mode_intent_with_llm(&state, user_id, msg.chat.id.0, text).await {
        if mode == "none" {
            // no-op, fall through to normal ask flow
        } else {
        if !can_change_voice_mode(&state, user_id) {
            bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.voicemode_admin_only"))
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
                state.i18n
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

    if text.starts_with("/agent") {
        let mode = text.strip_prefix("/agent").unwrap_or_default().trim();
        let reply = {
            let mut set = state
                .agent_off_chats
                .lock()
                .map_err(|_| anyhow!("agent mode lock poisoned"))?;
            match mode {
                "on" => {
                    set.remove(&msg.chat.id.0);
                    state.i18n.t("telegram.msg.agent_on")
                }
                "off" => {
                    set.insert(msg.chat.id.0);
                    state.i18n.t("telegram.msg.agent_off")
                }
                _ => {
                    let enabled = !set.contains(&msg.chat.id.0);
                    state.i18n.t_with(
                        "telegram.msg.agent_usage_status",
                        &[("status", if enabled { "on" } else { "off" })],
                    )
                }
            }
        };

        bot.send_message(msg.chat.id, reply)
            .await
            .context("send /agent reply failed")?;
        return Ok(());
    }

    if text.starts_with("/status") {
        match fetch_status_text(&state).await {
            Ok(status_text) => {
                bot.send_message(msg.chat.id, status_text)
                    .await
                    .context("send /status reply failed")?;
            }
            Err(err) => {
                bot.send_message(
                    msg.chat.id,
                    state
                        .i18n
                        .t_with("telegram.msg.read_status_failed", &[("error", &err.to_string())]),
                )
                    .await
                    .context("send /status error failed")?;
            }
        }
        return Ok(());
    }

    if text.starts_with("/cancel") {
        match cancel_tasks_for_chat(&state, user_id, msg.chat.id.0).await {
            Ok(canceled) => {
                let reply = if canceled > 0 {
                    state
                        .i18n
                        .t_with("telegram.msg.cancel_ok", &[("count", &canceled.to_string())])
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

    if text.starts_with("/skills") {
        let skills = if state.skills_list.is_empty() {
            state.i18n.t("telegram.msg.no_skills")
        } else {
            state.i18n.t_with(
                "telegram.msg.skills_list",
                &[("skills", &state.skills_list.join(", "))],
            )
        };
        bot.send_message(msg.chat.id, skills)
            .await
            .context("send /skills reply failed")?;
        return Ok(());
    }

    if text.starts_with("/crypto") {
        let raw = text.strip_prefix("/crypto").unwrap_or_default().trim();
        if raw.to_ascii_lowercase().starts_with("add ") {
            let is_admin = state.admins.contains(&user_id);
            if !is_admin {
                bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.cryptoapi_admin_only"))
                    .await
                    .context("send /crypto add unauthorized failed")?;
                return Ok(());
            }
            match handle_cryptoapi_command(&state, raw) {
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

        let queue_len = match fetch_queue_length(&state).await {
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
                    state
                        .i18n
                        .t_with("telegram.msg.skill_exec_failed_with_error", &[("error", &err.to_string())]),
                )
                .await
                .context("send /crypto error failed")?;
            }
        }
        return Ok(());
    }

    if text.starts_with("/run") {
        let rest = text.strip_prefix("/run").unwrap_or_default().trim();
        if rest.is_empty() {
            bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.run_usage"))
                .await
                .context("send /run usage failed")?;
            return Ok(());
        }

        let mut parts = rest.splitn(2, ' ');
        let skill_name = parts.next().unwrap_or_default().trim();
        let args = parts.next().unwrap_or_default().trim();

        if skill_name.is_empty() {
            bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.run_usage"))
                .await
                .context("send /run usage2 failed")?;
            return Ok(());
        }

        let queue_len = match fetch_queue_length(&state).await {
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
                    state
                        .i18n
                        .t_with("telegram.msg.skill_exec_failed_with_error", &[("error", &err.to_string())]),
                )
                    .await
                    .context("send /run error failed")?;
            }
        }
        return Ok(());
    }

    if text.starts_with("/sendfile") {
        let raw = text.strip_prefix("/sendfile").unwrap_or_default().trim();
        if raw.is_empty() {
            bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.sendfile_usage"))
                .await
                .context("send /sendfile usage failed")?;
            return Ok(());
        }

        if state.sendfile_admin_only && !state.admins.contains(&user_id) {
            bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.sendfile_admin_only"))
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
                state
                    .i18n
                    .t_with("telegram.msg.file_not_found", &[("path", &p.display().to_string())]),
            )
                .await
                .context("send /sendfile not found failed")?;
            return Ok(());
        }
        if !p.is_file() {
            bot.send_message(
                msg.chat.id,
                state
                    .i18n
                    .t_with("telegram.msg.not_a_file", &[("path", &p.display().to_string())]),
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

    let prompt = if text.starts_with("/ask") {
        text.strip_prefix("/ask").unwrap_or_default().trim()
    } else if text.starts_with('/') {
        bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.unknown_command"))
            .await
            .context("send unknown command reply failed")?;
        return Ok(());
    } else {
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
            let queue_len = match fetch_queue_length(&state).await {
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
            match submit_task_only(&state, user_id, msg.chat.id.0, TaskKind::RunSkill, payload).await {
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

    let queue_len = match fetch_queue_length(&state).await {
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
    let agent_enabled = state
        .agent_off_chats
        .lock()
        .map(|set| !set.contains(&msg.chat.id.0))
        .unwrap_or(true);

    match submit_task_only(
        &state,
        user_id,
        msg.chat.id.0,
        TaskKind::Ask,
        json!({ "text": prompt, "agent_mode": agent_enabled }),
    )
    .await
    {
        Ok(task_id) => {
            info!(
                "telegramd: submitted ask task_id={} user_id={} chat_id={} agent_mode={}",
                task_id, user_id, msg.chat.id.0, agent_enabled
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
                state
                    .i18n
                    .t_with("telegram.msg.process_failed_with_error", &[("error", &err.to_string())]),
            )
                .await
                .context("send ask error failed")?;
        }
    }

    Ok(())
}

async fn handle_callback_query(bot: Bot, q: CallbackQuery, state: BotState) -> anyhow::Result<()> {
    let Some(data) = q.data.as_deref() else {
        return Ok(());
    };
    debug!(
        "phase=callback callback_id={} from_user_id={} data={}",
        q.id,
        q.from.id.0,
        data
    );
    if data == "crypto_confirm_done_noop" {
        if let Err(err) = bot
            .answer_callback_query(q.id.clone())
            .text(state.i18n.t("telegram.msg.crypto_confirm_callback_done_ack"))
            .await
        {
            warn!("answer done callback query failed: {}", err);
        }
        return Ok(());
    }
    if data != "crypto_confirm_yes" && data != "crypto_confirm_no" {
        return Ok(());
    }

    if let Err(err) = bot
        .answer_callback_query(q.id.clone())
        .text(if data == "crypto_confirm_yes" {
            state.i18n.t("telegram.msg.crypto_confirm_callback_yes_ack")
        } else {
            state.i18n.t("telegram.msg.crypto_confirm_callback_no_ack")
        })
        .await
    {
        warn!("answer callback query failed: {}", err);
    }

    let Some(message) = q.message.as_ref() else {
        return Ok(());
    };
    let chat_id = message.chat().id;
    let message_id = message.id();
    debug!(
        "phase=callback_ack chat_id={} message_id={} data={}",
        chat_id.0,
        message_id.0,
        data
    );
    let done_keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        state.i18n.t("telegram.msg.crypto_confirm_button_done"),
        "crypto_confirm_done_noop",
    )]]);
    if let Err(err) = bot
        .edit_message_reply_markup(chat_id, message_id)
        .reply_markup(done_keyboard)
        .await
    {
        warn!("edit callback message markup failed: {}", err);
    }
    let user_id = match i64::try_from(q.from.id.0) {
        Ok(v) => v,
        Err(_) => {
            warn!("callback user id out of range: {}", q.from.id.0);
            return Ok(());
        }
    };

    // Use stable semantic tokens for downstream confirmation parsing.
    let prompt = if data == "crypto_confirm_yes" { "yes" } else { "no" };
    let agent_enabled = state
        .agent_off_chats
        .lock()
        .map(|set| !set.contains(&chat_id.0))
        .unwrap_or(true);
    let payload = json!({
        "text": prompt,
        "agent_mode": agent_enabled
    });

    match submit_task_only(&state, user_id, chat_id.0, TaskKind::Ask, payload).await {
        Ok(task_id) => {
            spawn_task_result_delivery(
                bot.clone(),
                state.clone(),
                chat_id,
                user_id,
                task_id,
                None,
                state.i18n.t("telegram.msg.process_failed"),
            );
        }
        Err(err) => {
            bot.send_message(
                chat_id,
                state
                    .i18n
                    .t_with("telegram.msg.process_failed", &[("error", &err.to_string())]),
            )
            .await
            .context("send callback submit error failed")?;
        }
    }
    Ok(())
}

async fn handle_image_only_message(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    user_id: i64,
    file_id: String,
    ext: &str,
) -> anyhow::Result<()> {
    let queue_len = match fetch_queue_length(state).await {
        Ok(v) => v,
        Err(_) => 0,
    };
    if queue_len >= state.queue_limit {
        bot.send_message(
            msg.chat.id,
            state.i18n.t_with(
                "telegram.msg.queue_full",
                &[
                    ("queued", &queue_len.to_string()),
                    ("limit", &state.queue_limit.to_string()),
                ],
            ),
        )
        .await
        .context("send queue full image message failed")?;
        return Ok(());
    }

    let ts = unix_ts();
    let normalized_ext = normalize_image_ext(ext);
    let rel_path = format!(
        "{}/tg_{}_{}_{}.{}",
        state.image_inbox_dir,
        msg.chat.id.0,
        user_id,
        ts,
        normalized_ext
    );
    let abs_path = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(&rel_path);

    download_telegram_file(state, bot, file_id, &abs_path).await?;

    let args = json!({
        "action": "describe",
        "images": [{"path": rel_path}],
        "detail_level": "normal"
    });
    let payload = json!({
        "skill_name": "image_vision",
        "args": args
    });

    match submit_task_only(state, user_id, msg.chat.id.0, TaskKind::RunSkill, payload).await {
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
            .context("send image vision submit error failed")?;
        }
    }

    Ok(())
}

async fn handle_image_only_save_only(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    user_id: i64,
    file_id: String,
    ext: &str,
) -> anyhow::Result<()> {
    let ts = unix_ts();
    let normalized_ext = normalize_image_ext(ext);
    let rel_path = format!(
        "{}/tg_{}_{}_{}.{}",
        state.image_inbox_dir,
        msg.chat.id.0,
        user_id,
        ts,
        normalized_ext
    );
    let abs_path = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(&rel_path);
    download_telegram_file(state, bot, file_id, &abs_path).await?;
    if let Ok(mut m) = state.pending_image_by_chat.lock() {
        m.insert(msg.chat.id.0, rel_path.clone());
    }
    bot.send_message(
        msg.chat.id,
        state.i18n.t("telegram.msg.image_received_wait_prompt"),
    )
    .await
    .context("send image saved path message failed")?;
    Ok(())
}

async fn handle_audio_message(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    user_id: i64,
    file_id: String,
    ext: &str,
) -> anyhow::Result<()> {
    let ts = unix_ts();
    let normalized_ext = normalize_audio_ext(ext);
    let rel_path = format!(
        "{}/tg_{}_{}_{}.{}",
        state.audio_inbox_dir,
        msg.chat.id.0,
        user_id,
        ts,
        normalized_ext
    );
    let abs_path = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(&rel_path);
    download_telegram_file(state, bot, file_id, &abs_path).await?;
    if let Ok(meta) = tokio::fs::metadata(&abs_path).await {
        if meta.len() as usize > state.max_audio_input_bytes {
            bot.send_message(
                msg.chat.id,
                state.i18n.t_with(
                    "telegram.msg.audio_too_large",
                    &[
                        ("size", &meta.len().to_string()),
                        ("limit", &state.max_audio_input_bytes.to_string()),
                    ],
                ),
            )
            .await
            .context("send audio too large message failed")?;
            return Ok(());
        }
    }

    let _typing_guard = TypingHeartbeatGuard::start(bot.clone(), msg.chat.id);
    let transcribe_payload = json!({
        "skill_name": "audio_transcribe",
        "args": {
            "audio": { "path": rel_path }
        }
    });
    let transcribe_task_id = submit_task_only(
        state,
        user_id,
        msg.chat.id.0,
        TaskKind::RunSkill,
        transcribe_payload,
    )
    .await
    .context("submit audio_transcribe task failed")?;
    let transcript = poll_task_result(state, &transcribe_task_id, Some(120))
        .await
        .context("poll audio_transcribe result failed")?;
    let transcript = transcript.join("\n").trim().to_string();
    let transcript = transcript.as_str();
    if transcript.is_empty() {
        bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.audio_transcript_empty"))
            .await
            .context("send empty transcript message failed")?;
        return Ok(());
    }

    let agent_enabled = state
        .agent_off_chats
        .lock()
        .map(|set| !set.contains(&msg.chat.id.0))
        .unwrap_or(true);
    let ask_task_id = submit_task_only(
        state,
        user_id,
        msg.chat.id.0,
        TaskKind::Ask,
        json!({
            "text": render_voice_chat_prompt(&state.voice_chat_prompt_template, transcript),
            "agent_mode": agent_enabled,
            "source": "voice"
        }),
    )
    .await
    .context("submit ask task for transcript failed")?;
    let answers = poll_task_result(state, &ask_task_id, Some(state.task_wait_seconds.max(300)))
        .await
        .context("poll ask result for transcript failed")?;
    let answer_joined = answers.join("\n\n");
    let mode = parse_voice_reply_mode(&effective_voice_reply_mode_for_chat(state, msg.chat.id.0));
    if matches!(mode, VoiceReplyMode::Text | VoiceReplyMode::Both) {
        for answer in &answers {
            send_text_or_image(bot, state, msg.chat.id, answer, false).await?;
        }
    }

    if matches!(mode, VoiceReplyMode::Voice | VoiceReplyMode::Both) {
        let tts_input = strip_delivery_tokens_for_tts(&answer_joined);
        if !tts_input.is_empty() {
            let tts_payload = json!({
                "skill_name": "audio_synthesize",
                "args": {
                    "text": tts_input,
                    "response_format": "opus"
                }
            });
            match submit_task_only(state, user_id, msg.chat.id.0, TaskKind::RunSkill, tts_payload).await {
                Ok(tts_task_id) => match poll_task_result(state, &tts_task_id, Some(90)).await {
                    Ok(tts_answer) => {
                        for msg_text in tts_answer {
                            let _ = send_text_or_image(bot, state, msg.chat.id, &msg_text, false).await;
                        }
                    }
                    Err(err) => {
                        warn!("audio_synthesize poll failed: {err}");
                    }
                },
                Err(err) => {
                    warn!("submit audio_synthesize failed: {err}");
                }
            }
        } else if matches!(mode, VoiceReplyMode::Voice) {
            // Voice-only mode but no speakable text: fallback to original answer.
            for answer in &answers {
                send_text_or_image(bot, state, msg.chat.id, answer, false).await?;
            }
        }
    }
    Ok(())
}

fn extract_image_attachment(msg: &Message) -> Option<(String, String)> {
    let MessageKind::Common(common) = &msg.kind else {
        return None;
    };
    match &common.media_kind {
        MediaKind::Photo(media) => media
            .photo
            .last()
            .map(|photo| (photo.file.id.to_string(), "jpg".to_string())),
        MediaKind::Document(media) => {
            let file_name_ext = media
                .document
                .file_name
                .as_deref()
                .and_then(extension_from_filename)
                .unwrap_or_default();
            let mime_is_image = media
                .document
                .mime_type
                .as_ref()
                .map(|m| m.type_().as_str() == "image")
                .unwrap_or(false);
            if mime_is_image || is_image_ext(&file_name_ext) {
                let ext = if file_name_ext.is_empty() {
                    "png".to_string()
                } else {
                    file_name_ext
                };
                Some((media.document.file.id.to_string(), ext))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn extract_audio_attachment(msg: &Message) -> Option<(String, String)> {
    let MessageKind::Common(common) = &msg.kind else {
        return None;
    };
    match &common.media_kind {
        MediaKind::Voice(media) => Some((media.voice.file.id.to_string(), "ogg".to_string())),
        MediaKind::Audio(media) => {
            let ext = media
                .audio
                .file_name
                .as_deref()
                .and_then(extension_from_filename)
                .unwrap_or_else(|| "mp3".to_string());
            Some((media.audio.file.id.to_string(), ext))
        }
        _ => None,
    }
}

async fn download_telegram_file(
    state: &BotState,
    bot: &Bot,
    file_id: String,
    local_path: &Path,
) -> anyhow::Result<()> {
    let file = bot
        .get_file(file_id)
        .await
        .context("telegram get_file failed")?;
    let file_url = format!(
        "https://api.telegram.org/file/bot{}/{}",
        state.bot_token, file.path
    );
    let bytes = state
        .client
        .get(file_url)
        .send()
        .await
        .context("download telegram file request failed")?
        .bytes()
        .await
        .context("read telegram file bytes failed")?;
    if let Some(parent) = local_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .context("create image inbox dir failed")?;
    }
    tokio::fs::write(local_path, &bytes)
        .await
        .context("write downloaded file failed")?;
    Ok(())
}

fn extension_from_filename(name: &str) -> Option<String> {
    let ext = Path::new(name).extension()?.to_string_lossy().to_string();
    if ext.is_empty() {
        None
    } else {
        Some(ext.to_ascii_lowercase())
    }
}

fn normalize_image_ext(ext: &str) -> String {
    let e = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    if is_image_ext(&e) {
        e
    } else {
        "png".to_string()
    }
}

fn normalize_audio_ext(ext: &str) -> String {
    let e = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    if matches!(e.as_str(), "ogg" | "mp3" | "wav" | "m4a" | "aac" | "flac" | "opus") {
        e
    } else {
        "ogg".to_string()
    }
}

fn load_prompt_template(path: &str, default_template: &str) -> String {
    match fs::read_to_string(path) {
        Ok(s) if !s.trim().is_empty() => s,
        _ => default_template.to_string(),
    }
}

fn render_voice_chat_prompt(template: &str, transcript: &str) -> String {
    template.replace("__TRANSCRIPT__", transcript.trim())
}

fn render_voice_mode_intent_prompt(template: &str, user_text: &str) -> String {
    template.replace("__USER_TEXT__", user_text.trim())
}

fn is_image_ext(ext: &str) -> bool {
    matches!(
        ext,
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "heic" | "heif"
    )
}

fn unix_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

struct TypingHeartbeatGuard {
    stop_tx: Option<oneshot::Sender<()>>,
}

impl TypingHeartbeatGuard {
    fn start(bot: Bot, chat_id: ChatId) -> Self {
        // Telegram typing status naturally expires after a few seconds.
        // We intentionally refresh slower than that to create a "pulse"
        // effect (shows for a while, then disappears, then shows again).
        const TYPING_PULSE_INTERVAL_SECS: u64 = 12;
        let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            loop {
                let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(TYPING_PULSE_INTERVAL_SECS)) => {}
                    _ = &mut stop_rx => break,
                }
            }
        });
        Self {
            stop_tx: Some(stop_tx),
        }
    }

    fn stop(&mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
    }
}

impl Drop for TypingHeartbeatGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

fn is_image_saved_preface(text: &str) -> bool {
    let t = text.trim().to_ascii_lowercase();
    t.starts_with("image saved")
        || t.starts_with("images saved")
        || t.starts_with("generated successfully and saved:")
        || t.starts_with("edited successfully and saved:")
        || text.trim().starts_with("图片已保存：")
        || text.trim().starts_with("图片生成成功并已保存：")
        || text.trim().starts_with("图片编辑成功并已保存：")
}

async fn send_text_or_image(
    bot: &Bot,
    state: &BotState,
    chat_id: ChatId,
    answer: &str,
    requires_confirmation: bool,
) -> anyhow::Result<()> {
    const PREFIX: &str = "IMAGE_FILE:";
    const FILE_PREFIX: &str = "FILE:";
    const VOICE_PREFIX: &str = "VOICE_FILE:";
    const EPHEMERAL_PREFIX: &str = "EPHEMERAL:";
    const EPHEMERAL_IMAGE_SAVED_TOKEN: &str = "EPHEMERAL:IMAGE_SAVED";

    let mut image_paths = dedupe_preserve_order(extract_prefixed_paths(answer, PREFIX));
    let file_paths = dedupe_preserve_order(extract_prefixed_paths(answer, FILE_PREFIX));
    let voice_paths = dedupe_preserve_order(extract_prefixed_paths(answer, VOICE_PREFIX));
    // If both IMAGE_FILE and FILE contain the same path, keep FILE only.
    let file_set = file_paths.iter().cloned().collect::<HashSet<_>>();
    image_paths.retain(|p| !file_set.contains(p));

    if !image_paths.is_empty() || !file_paths.is_empty() || !voice_paths.is_empty() {
        debug!(
            "phase=deliver_media chat_id={} answer_fp={} image_count={} file_count={} voice_count={} preface_preview={}",
            chat_id.0,
            text_fingerprint_hex(answer),
            image_paths.len(),
            file_paths.len(),
            voice_paths.len(),
            text_preview_for_log(answer, 120)
        );
        let ephemeral_image_saved_hint = answer
            .lines()
            .any(|line| line.trim().eq_ignore_ascii_case(EPHEMERAL_IMAGE_SAVED_TOKEN));
        let text_without_tokens =
            strip_prefixed_tokens(answer, &[PREFIX, FILE_PREFIX, VOICE_PREFIX, EPHEMERAL_PREFIX])
                .trim()
                .to_string();
        if !text_without_tokens.is_empty() {
            let sent = bot
                .send_message(chat_id, &text_without_tokens)
                .await
                .context("send file preface text failed")?;
            debug!(
                "phase=deliver_media_preface chat_id={} answer_fp={} telegram_msg_id={} text_preview={}",
                chat_id.0,
                text_fingerprint_hex(&text_without_tokens),
                sent.id.0,
                text_preview_for_log(&text_without_tokens, 120)
            );
            if state.ephemeral_image_saved_seconds > 0
                && (ephemeral_image_saved_hint || is_image_saved_preface(&text_without_tokens))
            {
                let bot_clone = bot.clone();
                let msg_id = sent.id;
                let secs = state.ephemeral_image_saved_seconds;
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(secs)).await;
                    let _ = bot_clone.delete_message(chat_id, msg_id).await;
                });
            }
        }

        for path in image_paths {
            bot.send_photo(chat_id, InputFile::file(path))
                .await
                .context("send image file failed")?;
        }

        for path in file_paths {
            // FILE: always means "send as document/file", even for image extensions.
            bot.send_document(chat_id, InputFile::file(path))
                .await
                .context("send document file failed")?;
        }

        for path in voice_paths {
            if let Err(err) = bot.send_voice(chat_id, InputFile::file(path.clone())).await {
                warn!("send_voice failed for {}: {}", path, err);
                bot.send_document(chat_id, InputFile::file(path))
                    .await
                    .context("fallback send voice as document failed")?;
            }
        }
        return Ok(());
    }

    if is_crypto_trade_confirm_prompt(answer, requires_confirmation) {
        let keyboard = InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback(
                state.i18n.t("telegram.msg.crypto_confirm_button_yes"),
                "crypto_confirm_yes",
            ),
            InlineKeyboardButton::callback(
                state.i18n.t("telegram.msg.crypto_confirm_button_no"),
                "crypto_confirm_no",
            ),
        ]]);
        let sent = bot.send_message(chat_id, answer.to_string())
            .reply_markup(keyboard)
            .await
            .context("send text message with confirm keyboard failed")?;
        debug!(
            "phase=deliver_text_confirm chat_id={} answer_fp={} telegram_msg_id={} answer_preview={}",
            chat_id.0,
            text_fingerprint_hex(answer),
            sent.id.0,
            text_preview_for_log(answer, 120)
        );
    } else {
        let sent = bot.send_message(chat_id, answer.to_string())
            .await
            .context("send text message failed")?;
        debug!(
            "phase=deliver_text chat_id={} answer_fp={} telegram_msg_id={} answer_preview={}",
            chat_id.0,
            text_fingerprint_hex(answer),
            sent.id.0,
            text_preview_for_log(answer, 120)
        );
    }
    Ok(())
}

fn is_crypto_trade_confirm_prompt(text: &str, structured_hint: bool) -> bool {
    if structured_hint {
        return true;
    }
    let t = text.to_ascii_lowercase();
    let decision = if t.contains("trade_preview") {
        true
    } else {
        (t.contains("confirm") || t.contains("trade_preview"))
        && t.contains("yes")
            && t.contains("no")
    };
    debug!(
        "phase=confirm_detect structured_hint={} decision={} text_fp={} text_preview={}",
        structured_hint,
        decision,
        text_fingerprint_hex(text),
        text_preview_for_log(text, 120)
    );
    decision
}

fn extract_prefixed_paths(answer: &str, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in answer.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let cleaned = normalize_path_token(rest.trim());
            if !cleaned.is_empty() && Path::new(cleaned).exists() && Path::new(cleaned).is_file() {
                out.push(cleaned.to_string());
            }
        }
    }
    out
}

fn dedupe_preserve_order(items: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        if seen.insert(item.clone()) {
            out.push(item);
        }
    }
    out
}

fn text_fingerprint_hex(text: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn text_preview_for_log(text: &str, max_chars: usize) -> String {
    let normalized = text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    normalized.chars().take(max_chars).collect::<String>() + "...(truncated)"
}

fn strip_prefixed_tokens(answer: &str, prefixes: &[&str]) -> String {
    answer
        .lines()
        .filter(|line| !prefixes.iter().any(|prefix| line.trim_start().starts_with(prefix)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_delivery_tokens_for_tts(answer: &str) -> String {
    strip_prefixed_tokens(answer, &["IMAGE_FILE:", "FILE:", "VOICE_FILE:", "EPHEMERAL:"])
        .trim()
        .to_string()
}

fn normalize_path_token(token: &str) -> &str {
    token.trim_matches(|c: char| {
        matches!(
            c,
            '"' | '\'' | '`' | '，' | ',' | ':' | '：' | ';' | '。' | ')' | '(' | '）' | '（'
        )
    })
}

fn resolve_sendfile_path(
    raw: &str,
    full_access: bool,
    allowed_dirs: &[String],
) -> Result<PathBuf, String> {
    let token = normalize_path_token(raw);
    if token.is_empty() {
        return Err("empty path".to_string());
    }

    let cwd = std::env::current_dir().map_err(|err| format!("read current_dir failed: {err}"))?;
    let candidate = if Path::new(token).is_absolute() {
        PathBuf::from(token)
    } else {
        cwd.join(token)
    };
    if candidate
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err("path with '..' is not allowed".to_string());
    }
    if full_access {
        return Ok(candidate);
    }

    for dir in allowed_dirs {
        if dir == "*" {
            return Ok(candidate);
        }
        let base = if Path::new(dir).is_absolute() {
            PathBuf::from(dir)
        } else {
            cwd.join(dir)
        };
        if candidate.starts_with(&base) {
            return Ok(candidate);
        }
    }

    Err(format!(
        "path is outside allowed dirs: {}",
        allowed_dirs.join(", ")
    ))
}

fn is_image_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".webp")
        || lower.ends_with(".gif")
}

fn spawn_task_result_delivery(
    bot: Bot,
    state: BotState,
    chat_id: ChatId,
    user_id: i64,
    task_id: String,
    soft_notice_override_seconds: Option<u64>,
    fail_prefix: String,
) {
    tokio::spawn(async move {
        let _typing_guard = TypingHeartbeatGuard::start(bot.clone(), chat_id);
        let poll_interval_ms = state.poll_interval_ms.max(1);
        let soft_notice_seconds = soft_notice_override_seconds
            .unwrap_or(state.task_wait_seconds)
            .max(1);
        let hard_notice_seconds = state.task_wait_seconds.max(1);
        let started_at = tokio::time::Instant::now();
        let mut soft_notice_sent = false;
        let mut hard_notice_sent = false;
        let mut sent_progress_count = 0usize;

        loop {
            match query_task_status(&state, &task_id).await {
                Ok(task) => match task.status {
                    TaskStatus::Queued | TaskStatus::Running => {
                        let progress_messages = task_progress_messages(&task);
                        debug!(
                            "phase=poll task_id={} chat_id={} status={:?} elapsed_ms={} sent_progress_count={} progress_len={}",
                            task_id,
                            chat_id.0,
                            task.status,
                            started_at.elapsed().as_millis(),
                            sent_progress_count,
                            progress_messages.len()
                        );
                        if sent_progress_count < progress_messages.len() {
                            for answer in progress_messages.iter().skip(sent_progress_count) {
                                let requires_confirmation =
                                    is_crypto_trade_confirm_prompt(answer, false);
                                debug!(
                                    "phase=deliver_progress task_id={} chat_id={} msg_fp={} msg_len={} requires_confirmation={} msg_preview={}",
                                    task_id,
                                    chat_id.0,
                                    text_fingerprint_hex(answer),
                                    answer.len(),
                                    requires_confirmation,
                                    text_preview_for_log(answer, 160)
                                );
                                let _ = send_text_or_image(
                                    &bot,
                                    &state,
                                    chat_id,
                                    answer,
                                    requires_confirmation,
                                )
                                .await;
                            }
                            sent_progress_count = progress_messages.len();
                        }
                        if !soft_notice_sent
                            && started_at.elapsed() >= Duration::from_secs(soft_notice_seconds)
                        {
                            info!(
                                "task still running notice: phase=quick task_id={} chat_id={} elapsed_seconds={}",
                                task_id,
                                chat_id.0,
                                soft_notice_seconds
                            );
                            let msg = state.i18n.t_with(
                                "telegram.msg.task_still_running_background",
                                &[("seconds", &soft_notice_seconds.to_string())],
                            );
                            let _ = bot.send_message(chat_id, msg).await;
                            soft_notice_sent = true;
                        }
                        if !hard_notice_sent
                            && hard_notice_seconds > soft_notice_seconds
                            && started_at.elapsed() >= Duration::from_secs(hard_notice_seconds)
                        {
                            info!(
                                "task still running notice: phase=worker_timeout task_id={} chat_id={} elapsed_seconds={}",
                                task_id,
                                chat_id.0,
                                hard_notice_seconds
                            );
                            let msg = state.i18n.t_with(
                                "telegram.msg.task_still_running_worker_timeout",
                                &[("seconds", &hard_notice_seconds.to_string())],
                            );
                            let _ = bot.send_message(chat_id, msg).await;
                            hard_notice_sent = true;
                        }
                        tokio::time::sleep(Duration::from_millis(poll_interval_ms)).await;
                    }
                    TaskStatus::Succeeded => {
                        let answers = task_success_messages_from_offset(&state, &task, sent_progress_count);
                        let requires_confirmation = task_requires_crypto_confirmation(&task);
                        debug!(
                            "phase=deliver_success task_id={} chat_id={} sent_progress_count={} success_count={} requires_confirmation={}",
                            task_id,
                            chat_id.0,
                            sent_progress_count,
                            answers.len(),
                            requires_confirmation
                        );
                        for answer in answers {
                            debug!(
                                "phase=deliver_success_item task_id={} chat_id={} msg_fp={} msg_len={} requires_confirmation={} msg_preview={}",
                                task_id,
                                chat_id.0,
                                text_fingerprint_hex(&answer),
                                answer.len(),
                                requires_confirmation,
                                text_preview_for_log(&answer, 160)
                            );
                            let _ = send_text_or_image(
                                &bot,
                                &state,
                                chat_id,
                                &answer,
                                requires_confirmation,
                            )
                            .await;
                        }
                        break;
                    }
                    TaskStatus::Failed | TaskStatus::Canceled | TaskStatus::Timeout => {
                        let detail = task_terminal_error_text(&state, &task);
                        if let Some(_resume_context) = task
                            .result_json
                            .as_ref()
                            .and_then(|v| v.get("resume_context"))
                            .cloned()
                        {
                            let pending = PendingResumeContext {
                                user_id,
                                created_at_secs: SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs(),
                            };
                            if let Ok(mut guard) = state.pending_resume_by_chat.lock() {
                                guard.insert(chat_id.0, pending);
                            }
                            let fail_msg = format!(
                                "{}",
                                state.i18n.t_with(
                                    "telegram.msg.resume_interrupted_hint",
                                    &[("prefix", &fail_prefix), ("detail", &detail)],
                                )
                            );
                            let _ = bot.send_message(chat_id, fail_msg).await;
                            break;
                        }
                        let _ = bot
                            .send_message(chat_id, format!("{fail_prefix}：{detail}"))
                            .await;
                        break;
                    }
                },
                Err(err) => {
                    let _ = bot
                        .send_message(chat_id, format!("{fail_prefix}：{}", err))
                        .await;
                    break;
                }
            }
        }
    });
}

fn task_success_messages(state: &BotState, task: &TaskQueryResponse) -> Vec<String> {
    task_success_messages_from_offset(state, task, 0)
}

fn task_success_messages_from_offset(
    state: &BotState,
    task: &TaskQueryResponse,
    offset: usize,
) -> Vec<String> {
    let task_id = &task.task_id;
    if let Some(messages) = task
        .result_json
        .as_ref()
        .and_then(|v| v.get("messages"))
        .and_then(|v| v.as_array())
    {
        let out = messages
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        let out = dedupe_preserve_order(out);
        if !out.is_empty() {
            debug!(
                "phase=success_source task_id={} source=messages offset={} messages_len={}",
                task_id,
                offset,
                out.len()
            );
            if offset >= out.len() {
                let text = task
                    .result_json
                    .as_ref()
                    .and_then(|v| v.get("text"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                if let Some(text) = text {
                    debug!(
                        "phase=success_source task_id={} source=text_fallback offset={} text_fp={} text_len={}",
                        task_id,
                        offset,
                        text_fingerprint_hex(&text),
                        text.len()
                    );
                    return vec![text];
                }
                return Vec::new();
            }
            return out.into_iter().skip(offset).collect();
        }
    }
    let text = task
        .result_json
        .as_ref()
        .and_then(|v| v.get("text"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| state.i18n.t("telegram.msg.task_done_no_text"));
    debug!(
        "phase=success_source task_id={} source=text_only offset={} text_fp={} text_len={}",
        task_id,
        offset,
        text_fingerprint_hex(&text),
        text.len()
    );
    vec![text]
}

fn task_progress_messages(task: &TaskQueryResponse) -> Vec<String> {
    let out = task
        .result_json
        .as_ref()
        .and_then(|v| v.get("progress_messages"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    dedupe_preserve_order(out)
}

fn task_requires_crypto_confirmation(task: &TaskQueryResponse) -> bool {
    task.result_json
        .as_ref()
        .and_then(|v| v.get("requires_confirmation"))
        .and_then(|v| v.as_str())
        .map(|v| v.eq_ignore_ascii_case("crypto_trade"))
        .unwrap_or(false)
}

fn task_terminal_error_text(state: &BotState, task: &TaskQueryResponse) -> String {
    if let Some(raw_detail) = task.error_text.as_deref() {
        let detail = raw_detail.trim();
        if !detail.is_empty() {
            return detail.to_string();
        }
    }
    state.i18n.t_with(
        "telegram.error.task_finished_with_detail",
        &[
            ("status", &format!("{:?}", task.status)),
            (
                "detail",
                &task
                    .error_text
                    .clone()
                    .unwrap_or_else(|| state.i18n.t("telegram.msg.no_error_text")),
            ),
        ],
    )
}

async fn query_task_status(state: &BotState, task_id: &str) -> anyhow::Result<TaskQueryResponse> {
    let url = format!("{}/v1/tasks/{task_id}", state.clawd_base_url);
    let resp = state
        .client
        .get(&url)
        .send()
        .await
        .context("query task status failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "{}",
            state.i18n.t_with(
                "telegram.error.query_task_failed_http",
                &[("status", &status.to_string()), ("body", &body)],
            )
        ));
    }

    let body: ApiResponse<TaskQueryResponse> = resp
        .json()
        .await
        .context("decode query task response failed")?;

    if !body.ok {
        return Err(anyhow!(
            "{}",
            state.i18n.t_with(
                "telegram.error.query_task_failed",
                &[(
                    "error",
                    &body.error.unwrap_or_else(|| state.i18n.t("common.unknown_error"))
                )],
            )
        ));
    }

    body.data
        .ok_or_else(|| anyhow!("{}", state.i18n.t("telegram.error.query_task_missing_data")))
}

async fn submit_task_only(
    state: &BotState,
    user_id: i64,
    chat_id: i64,
    kind: TaskKind,
    payload: serde_json::Value,
) -> anyhow::Result<String> {
    let payload_compact = payload.to_string();
    let payload_fp = text_fingerprint_hex(&payload_compact);
    let payload_preview = text_preview_for_log(&payload_compact, 180);
    debug!(
        "phase=submit user_id={} chat_id={} kind={:?} payload_fp={} payload_len={} payload_preview={}",
        user_id,
        chat_id,
        kind,
        payload_fp,
        payload_compact.len(),
        payload_preview
    );
    let submit_req = SubmitTaskRequest {
        user_id,
        chat_id,
        channel: Some(ChannelKind::Telegram),
        external_user_id: Some(user_id.to_string()),
        external_chat_id: Some(chat_id.to_string()),
        kind: kind.clone(),
        payload,
    };

    let submit_url = format!("{}/v1/tasks", state.clawd_base_url);
    debug!(
        "submit_task_only: url={} user_id={} chat_id={} kind={:?}",
        submit_url, user_id, chat_id, submit_req.kind
    );
    let submit_resp = state
        .client
        .post(&submit_url)
        .json(&submit_req)
        .send()
        .await
        .context("submit task request failed")?;

    if !submit_resp.status().is_success() {
        let status = submit_resp.status();
        let body = submit_resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "{}",
            state.i18n.t_with(
                "telegram.error.submit_task_failed_http",
                &[("status", &status.to_string()), ("body", &body)],
            )
        ));
    }

    let submit_body: ApiResponse<SubmitTaskResponse> = submit_resp
        .json()
        .await
        .context("decode submit task response failed")?;

    if !submit_body.ok {
        return Err(anyhow!(
            "{}",
            state.i18n.t_with(
                "telegram.error.submit_task_rejected",
                &[(
                    "error",
                    &submit_body
                        .error
                        .unwrap_or_else(|| state.i18n.t("common.unknown_error"))
                )],
            )
        ));
    }

    let task_id = submit_body
        .data
        .ok_or_else(|| anyhow!("{}", state.i18n.t("telegram.error.submit_task_missing_task_id")))?
        .task_id;

    debug!(
        "phase=submit_done user_id={} chat_id={} kind={:?} task_id={} payload_fp={}",
        user_id,
        chat_id,
        kind,
        task_id,
        payload_fp
    );
    Ok(task_id.to_string())
}

async fn poll_task_result(
    state: &BotState,
    task_id: &str,
    wait_override_seconds: Option<u64>,
) -> anyhow::Result<Vec<String>> {
    let poll_interval_ms = state.poll_interval_ms.max(1);
    let wait_seconds = wait_override_seconds.unwrap_or(state.task_wait_seconds).max(1);
    let max_rounds = ((wait_seconds * 1000) / poll_interval_ms).max(1);

    for _ in 0..max_rounds {
        let task = query_task_status(state, task_id).await?;
        match task.status {
            TaskStatus::Queued | TaskStatus::Running => {
                tokio::time::sleep(Duration::from_millis(poll_interval_ms)).await;
            }
            TaskStatus::Succeeded => {
                return Ok(task_success_messages(state, &task));
            }
            TaskStatus::Failed | TaskStatus::Canceled | TaskStatus::Timeout => {
                return Err(anyhow!("{}", task_terminal_error_text(state, &task)));
            }
        }
    }

    Err(anyhow!("task_result_wait_timeout"))
}

async fn cancel_tasks_for_chat(
    state: &BotState,
    user_id: i64,
    chat_id: i64,
) -> anyhow::Result<i64> {
    let url = format!("{}/v1/tasks/cancel", state.clawd_base_url);
    let payload = json!({
        "user_id": user_id,
        "chat_id": chat_id,
    });
    let resp = state
        .client
        .post(&url)
        .json(&payload)
        .send()
        .await
        .context("request cancel tasks failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "cancel http {status}: {body}",
        ));
    }

    let body: ApiResponse<JsonValue> = resp
        .json()
        .await
        .context("decode cancel response failed")?;

    if !body.ok {
        return Err(anyhow!(
            "cancel failed: {}",
            body.error.unwrap_or_else(|| "unknown error".to_string())
        ));
    }

    let canceled = body
        .data
        .and_then(|v| v.get("canceled").and_then(|n| n.as_i64()))
        .unwrap_or(0);
    Ok(canceled)
}
async fn fetch_status_text(state: &BotState) -> anyhow::Result<String> {
    let url = format!("{}/v1/health", state.clawd_base_url);
    let resp = state
        .client
        .get(&url)
        .send()
        .await
        .context("request health failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "{}",
            state.i18n.t_with(
                "telegram.error.health_http_failed",
                &[("status", &status.to_string()), ("body", &body)],
            )
        ));
    }

    let body: ApiResponse<HealthResponse> = resp
        .json()
        .await
        .context("decode health response failed")?;

    if !body.ok {
        return Err(anyhow!(
            "{}",
            state.i18n.t_with(
                "telegram.error.health_failed",
                &[(
                    "error",
                    &body.error.unwrap_or_else(|| state.i18n.t("common.unknown_error"))
                )],
            )
        ));
    }

    let data = body
        .data
        .ok_or_else(|| anyhow!("{}", state.i18n.t("telegram.error.health_missing_data")))?;
    Ok(state.i18n.t_with(
        "telegram.msg.status_text",
        &[
            ("worker_state", &data.worker_state),
            ("queue_length", &data.queue_length.to_string()),
            ("running_length", &data.running_length.to_string()),
            (
                "running_oldest_age_seconds",
                &data.running_oldest_age_seconds.to_string(),
            ),
            (
                "task_timeout_seconds",
                &data.task_timeout_seconds.to_string(),
            ),
            ("uptime_seconds", &data.uptime_seconds.to_string()),
            ("version", &data.version),
        ],
    ))
}

async fn fetch_queue_length(state: &BotState) -> anyhow::Result<usize> {
    let url = format!("{}/v1/health", state.clawd_base_url);
    let resp = state
        .client
        .get(&url)
        .send()
        .await
        .context("request health failed")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "{}",
            state.i18n.t_with(
                "telegram.error.health_http_failed",
                &[("status", &status.to_string()), ("body", &body)],
            )
        ));
    }
    let body: ApiResponse<HealthResponse> = resp
        .json()
        .await
        .context("decode health response failed")?;
    if !body.ok {
        return Err(anyhow!(
            "{}",
            state.i18n.t_with(
                "telegram.error.health_failed",
                &[(
                    "error",
                    &body.error.unwrap_or_else(|| state.i18n.t("common.unknown_error"))
                )],
            )
        ));
    }
    let data = body
        .data
        .ok_or_else(|| anyhow!("{}", state.i18n.t("telegram.error.health_missing_data")))?;
    Ok(data.queue_length)
}

fn handle_voicemode_command(state: &BotState, chat_id: i64, text: &str) -> anyhow::Result<String> {
    let rest = text.strip_prefix("/voicemode").unwrap_or_default().trim();
    if rest.is_empty() {
        return Ok(state.i18n.t("telegram.msg.voicemode_usage"));
    }
    match rest {
        "show" => {
            let chat_mode = effective_voice_reply_mode_for_chat(state, chat_id);
            let global_mode =
                normalize_voice_reply_mode(&state.voice_reply_mode).unwrap_or_else(|| "voice".to_string());
            Ok(state.i18n.t_with(
                "telegram.msg.voicemode_show",
                &[("chat_mode", &chat_mode), ("global_mode", &global_mode)],
            ))
        }
        "voice" | "text" | "both" => {
            set_chat_voice_mode(state, chat_id, Some(rest))?;
            Ok(state.i18n.t_with("telegram.msg.voicemode_set_ok", &[("mode", rest)]))
        }
        "reset" => {
            set_chat_voice_mode(state, chat_id, None)?;
            let global_mode =
                normalize_voice_reply_mode(&state.voice_reply_mode).unwrap_or_else(|| "voice".to_string());
            Ok(state.i18n.t_with(
                "telegram.msg.voicemode_reset_ok",
                &[("global_mode", &global_mode)],
            ))
        }
        _ => Ok(state.i18n.t("telegram.msg.voicemode_usage")),
    }
}

fn set_chat_voice_mode(state: &BotState, chat_id: i64, mode: Option<&str>) -> anyhow::Result<()> {
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
    let cfg_path = if std::path::Path::new("configs/channels/telegram.toml").exists() {
        "configs/channels/telegram.toml"
    } else {
        "configs/config.toml"
    };
    let raw = fs::read_to_string(cfg_path)
        .context(state.i18n.t("telegram.error.read_config_failed"))?;
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
    let telegram_tbl = telegram
        .as_table_mut()
        .ok_or_else(|| anyhow!("{}", state.i18n.t("telegram.error.config_not_table")))?;
    let by_chat = telegram_tbl
        .entry("voice_reply_mode_by_chat")
        .or_insert(TomlValue::Table(toml::map::Map::new()));
    if !by_chat.is_table() {
        *by_chat = TomlValue::Table(toml::map::Map::new());
    }
    let by_chat_tbl = by_chat
        .as_table_mut()
        .ok_or_else(|| anyhow!("{}", state.i18n.t("telegram.error.config_not_table")))?;
    let key = chat_id.to_string();
    if let Some(v) = mode {
        by_chat_tbl.insert(key, TomlValue::String(v.to_string()));
    } else {
        by_chat_tbl.remove(&key);
    }

    let output =
        toml::to_string_pretty(&value).context(state.i18n.t("telegram.error.serialize_config_failed"))?;
    fs::write(cfg_path, output).context(state.i18n.t("telegram.error.write_config_failed"))?;
    Ok(())
}

fn handle_openclaw_config_command(state: &BotState, text: &str) -> anyhow::Result<String> {
    let cmd = text
        .strip_prefix("/rustclaw")
        .or_else(|| text.strip_prefix("/openclaw"))
        .unwrap_or_default()
        .trim();
    if cmd.is_empty() {
        return Ok(openclaw_usage_text(state));
    }

    let mut parts = cmd.split_whitespace();
    let section = parts.next().unwrap_or_default();
    if section != "config" {
        return Ok(openclaw_usage_text(state));
    }

    let sub = parts.next().unwrap_or_default();
    match sub {
        "show" => show_model_config(state),
        "types" | "vendors" => Ok(supported_types_text(state)),
        "set" => {
            let provider_type = parts.next().unwrap_or_default();
            let model = parts.next().unwrap_or_default();
            if provider_type.is_empty() || model.is_empty() {
                return Err(anyhow!("{}", state.i18n.t("telegram.msg.openclaw_set_usage")));
            }
            set_model_config(state, provider_type, model)
        }
        _ => Ok(openclaw_usage_text(state)),
    }
}

fn cryptoapi_usage_text(state: &BotState) -> String {
    state.i18n.t("telegram.msg.cryptoapi_usage")
}

fn crypto_command_usage_text(state: &BotState) -> String {
    state.i18n.t("telegram.msg.crypto_usage")
}

fn parse_symbols_csv(raw: &str) -> Vec<String> {
    raw.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .collect()
}

fn normalize_trade_symbol_for_config(raw: &str) -> Option<String> {
    let upper = raw
        .trim()
        .to_ascii_uppercase()
        .replace(['-', '/', ' '], "");
    if upper.is_empty() {
        return None;
    }
    let ok = upper
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    if !ok || upper.len() < 5 || upper.len() > 20 {
        return None;
    }
    Some(upper)
}

fn maybe_exchange_token(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "auto"
            | "dual"
            | "both"
            | "all"
            | "binance"
            | "okx"
            | "gateio"
            | "coinbase"
            | "kraken"
            | "coingecko"
    )
}

fn build_crypto_skill_payload(raw: &str) -> anyhow::Result<Option<JsonValue>> {
    let raw = raw.trim();
    if raw.is_empty() || matches!(raw, "help" | "-h" | "--help") {
        return Ok(None);
    }
    let parts = raw.split_whitespace().collect::<Vec<_>>();
    if parts.is_empty() {
        return Ok(None);
    }
    let cmd = parts[0].to_ascii_lowercase();
    let args = match cmd.as_str() {
        "price" => {
            let symbol = parts.get(1).ok_or_else(|| anyhow!("missing symbol"))?;
            let exchange = parts.get(2).copied().unwrap_or("auto");
            json!({"action":"get_price","symbol":symbol,"exchange":exchange})
        }
        "prices" => {
            if parts.len() < 2 {
                return Err(anyhow!("missing symbols"));
            }
            let mut exchange = "auto";
            let mut symbols_tokens = parts[1..].to_vec();
            if let Some(last) = symbols_tokens.last().copied() {
                if maybe_exchange_token(last) {
                    exchange = last;
                    let _ = symbols_tokens.pop();
                }
            }
            let symbols_raw = symbols_tokens.join(" ");
            let symbols = parse_symbols_csv(&symbols_raw);
            if symbols.is_empty() {
                return Err(anyhow!("symbols is empty"));
            }
            json!({"action":"get_multi_price","symbols":symbols,"exchange":exchange})
        }
        "book" => {
            let symbol = parts.get(1).ok_or_else(|| anyhow!("missing symbol"))?;
            let exchange = parts.get(2).copied().unwrap_or("auto");
            json!({"action":"get_book_ticker","symbol":symbol,"exchange":exchange})
        }
        "normalize" => {
            let symbol = parts.get(1).ok_or_else(|| anyhow!("missing symbol"))?;
            json!({"action":"normalize_symbol","symbol":symbol})
        }
        "health" => {
            let symbol = parts.get(1).copied().unwrap_or("BTCUSDT");
            json!({"action":"healthcheck","symbol":symbol})
        }
        "address" => {
            let address = parts.get(1).ok_or_else(|| anyhow!("missing eth address"))?;
            let token = parts.get(2).copied().unwrap_or("eth");
            let tx_limit = parts
                .get(3)
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(5);
            json!({
                "action":"onchain",
                "chain":"ethereum",
                "address":address,
                "token":token,
                "tx_limit":tx_limit
            })
        }
        _ => return Ok(None),
    };
    Ok(Some(json!({
        "skill_name": "crypto",
        "args": args
    })))
}

fn mask_secret(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return "<empty>".to_string();
    }
    if s.len() <= 8 {
        return "***".to_string();
    }
    format!("{}***{}", &s[..4], &s[s.len() - 4..])
}

fn clear_pending_resume_for_chat(state: &BotState, chat_id: i64) {
    if let Ok(mut guard) = state.pending_resume_by_chat.lock() {
        guard.remove(&chat_id);
    }
}

fn handle_cryptoapi_command(state: &BotState, raw: &str) -> anyhow::Result<String> {
    let cmd = raw.trim();
    if cmd.is_empty() {
        return Ok(cryptoapi_usage_text(state));
    }
    let mut parts = cmd.split_whitespace();
    let action = parts.next().unwrap_or_default().to_ascii_lowercase();
    match action.as_str() {
        "show" => show_cryptoapi_status(state),
        "add" => {
            let target = parts.next().unwrap_or_default().to_ascii_lowercase();
            match target.as_str() {
                "allowed_symbols" | "allowedsymbols" | "symbols" => {
                    let symbols_raw = parts.collect::<Vec<_>>().join(" ");
                    let parsed = parse_symbols_csv(&symbols_raw)
                        .into_iter()
                        .filter_map(|s| normalize_trade_symbol_for_config(&s))
                        .collect::<Vec<_>>();
                    if parsed.is_empty() {
                        return Ok(state.i18n.t("telegram.msg.cryptoapi_add_allowed_symbols_invalid"));
                    }
                    let updated = persist_crypto_allowed_symbols_add(&parsed)?;
                    Ok(state.i18n.t_with(
                        "telegram.msg.cryptoapi_add_allowed_symbols_ok",
                        &[("symbols", &updated.join(", "))],
                    ))
                }
                _ => Ok(cryptoapi_usage_text(state)),
            }
        }
        "set" => {
            let exchange = parts.next().unwrap_or_default().to_ascii_lowercase();
            match exchange.as_str() {
                "binance" => {
                    let api_key = parts.next().unwrap_or_default();
                    let api_secret = parts.next().unwrap_or_default();
                    if api_key.is_empty() || api_secret.is_empty() {
                        return Ok(cryptoapi_usage_text(state));
                    }
                    persist_crypto_api_config_binance(api_key, api_secret)?;
                    let api_key_masked = mask_secret(api_key);
                    let api_secret_masked = mask_secret(api_secret);
                    Ok(state.i18n.t_with(
                        "telegram.msg.cryptoapi_set_binance_ok",
                        &[
                            ("api_key", &api_key_masked),
                            ("api_secret", &api_secret_masked),
                        ],
                    ))
                }
                "okx" => {
                    let api_key = parts.next().unwrap_or_default();
                    let api_secret = parts.next().unwrap_or_default();
                    let passphrase = parts.next().unwrap_or_default();
                    if api_key.is_empty() || api_secret.is_empty() || passphrase.is_empty() {
                        return Ok(cryptoapi_usage_text(state));
                    }
                    persist_crypto_api_config_okx(api_key, api_secret, passphrase)?;
                    let api_key_masked = mask_secret(api_key);
                    let api_secret_masked = mask_secret(api_secret);
                    let passphrase_masked = mask_secret(passphrase);
                    Ok(state.i18n.t_with(
                        "telegram.msg.cryptoapi_set_okx_ok",
                        &[
                            ("api_key", &api_key_masked),
                            ("api_secret", &api_secret_masked),
                            ("passphrase", &passphrase_masked),
                        ],
                    ))
                }
                _ => Ok(cryptoapi_usage_text(state)),
            }
        }
        _ => Ok(cryptoapi_usage_text(state)),
    }
}

fn show_cryptoapi_status(state: &BotState) -> anyhow::Result<String> {
    let path = "configs/crypto.toml";
    let raw = fs::read_to_string(path).context("failed to read configs/crypto.toml")?;
    let value: TomlValue = toml::from_str(&raw).context("failed to parse configs/crypto.toml")?;
    let bin = value.get("binance").and_then(|v| v.as_table());
    let okx = value.get("okx").and_then(|v| v.as_table());
    let crypto = value.get("crypto").and_then(|v| v.as_table());
    let bin_enabled = bin
        .and_then(|t| t.get("enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let okx_enabled = okx
        .and_then(|t| t.get("enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let bin_key = bin
        .and_then(|t| t.get("api_key"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let okx_key = okx
        .and_then(|t| t.get("api_key"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let allowed_symbols = crypto
        .and_then(|t| t.get("allowed_symbols"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let binance_key_masked = mask_secret(bin_key);
    let okx_key_masked = mask_secret(okx_key);
    let allowed_symbols_text = if allowed_symbols.is_empty() {
        "-".to_string()
    } else {
        allowed_symbols.join(", ")
    };
    let binance_enabled_text = bin_enabled.to_string();
    let okx_enabled_text = okx_enabled.to_string();
    Ok(state.i18n.t_with(
        "telegram.msg.cryptoapi_status",
        &[
            ("binance_enabled", &binance_enabled_text),
            ("binance_key", &binance_key_masked),
            ("okx_enabled", &okx_enabled_text),
            ("okx_key", &okx_key_masked),
            ("allowed_symbols", &allowed_symbols_text),
        ],
    ))
}

fn persist_crypto_api_config_binance(api_key: &str, api_secret: &str) -> anyhow::Result<()> {
    let path = "configs/crypto.toml";
    let raw = fs::read_to_string(path).context("failed to read configs/crypto.toml")?;
    let mut value: TomlValue = toml::from_str(&raw).context("failed to parse configs/crypto.toml")?;
    let root = value
        .as_table_mut()
        .ok_or_else(|| anyhow!("crypto config root is not a table"))?;
    let entry = root
        .entry("binance")
        .or_insert(TomlValue::Table(toml::map::Map::new()));
    if !entry.is_table() {
        *entry = TomlValue::Table(toml::map::Map::new());
    }
    let table = entry
        .as_table_mut()
        .ok_or_else(|| anyhow!("binance node is not a table"))?;
    table.insert("enabled".to_string(), TomlValue::Boolean(true));
    table.insert("api_key".to_string(), TomlValue::String(api_key.to_string()));
    table.insert("api_secret".to_string(), TomlValue::String(api_secret.to_string()));
    let output = toml::to_string_pretty(&value).context("failed to serialize configs/crypto.toml")?;
    fs::write(path, output).context("failed to write configs/crypto.toml")?;
    Ok(())
}

fn persist_crypto_api_config_okx(api_key: &str, api_secret: &str, passphrase: &str) -> anyhow::Result<()> {
    let path = "configs/crypto.toml";
    let raw = fs::read_to_string(path).context("failed to read configs/crypto.toml")?;
    let mut value: TomlValue = toml::from_str(&raw).context("failed to parse configs/crypto.toml")?;
    let root = value
        .as_table_mut()
        .ok_or_else(|| anyhow!("crypto config root is not a table"))?;
    let entry = root
        .entry("okx")
        .or_insert(TomlValue::Table(toml::map::Map::new()));
    if !entry.is_table() {
        *entry = TomlValue::Table(toml::map::Map::new());
    }
    let table = entry
        .as_table_mut()
        .ok_or_else(|| anyhow!("okx node is not a table"))?;
    table.insert("enabled".to_string(), TomlValue::Boolean(true));
    table.insert("api_key".to_string(), TomlValue::String(api_key.to_string()));
    table.insert("api_secret".to_string(), TomlValue::String(api_secret.to_string()));
    table.insert("passphrase".to_string(), TomlValue::String(passphrase.to_string()));
    let output = toml::to_string_pretty(&value).context("failed to serialize configs/crypto.toml")?;
    fs::write(path, output).context("failed to write configs/crypto.toml")?;
    Ok(())
}

fn persist_crypto_allowed_symbols_add(symbols: &[String]) -> anyhow::Result<Vec<String>> {
    let path = "configs/crypto.toml";
    let raw = fs::read_to_string(path).context("failed to read configs/crypto.toml")?;
    let mut value: TomlValue = toml::from_str(&raw).context("failed to parse configs/crypto.toml")?;
    let root = value
        .as_table_mut()
        .ok_or_else(|| anyhow!("crypto config root is not a table"))?;

    let crypto_entry = root
        .entry("crypto")
        .or_insert(TomlValue::Table(toml::map::Map::new()));
    if !crypto_entry.is_table() {
        *crypto_entry = TomlValue::Table(toml::map::Map::new());
    }
    let crypto_table = crypto_entry
        .as_table_mut()
        .ok_or_else(|| anyhow!("crypto node is not a table"))?;

    let existing = crypto_table
        .get("allowed_symbols")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .filter_map(normalize_trade_symbol_for_config)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut merged = existing;
    for s in symbols {
        if !merged.iter().any(|x| x == s) {
            merged.push(s.clone());
        }
    }
    if merged.is_empty() {
        merged.push("BTCUSDT".to_string());
    }
    crypto_table.insert(
        "allowed_symbols".to_string(),
        TomlValue::Array(merged.iter().map(|s| TomlValue::String(s.clone())).collect()),
    );
    let output = toml::to_string_pretty(&value).context("failed to serialize configs/crypto.toml")?;
    fs::write(path, output).context("failed to write configs/crypto.toml")?;
    Ok(merged)
}

fn openclaw_usage_text(state: &BotState) -> String {
    state.i18n.t("telegram.msg.openclaw_usage")
}

fn supported_types_text(state: &BotState) -> String {
    state.i18n.t("telegram.msg.openclaw_supported_vendors")
}

fn show_model_config(state: &BotState) -> anyhow::Result<String> {
    let raw = fs::read_to_string("configs/config.toml")
        .context(state.i18n.t("telegram.error.read_config_failed"))?;
    let value: TomlValue =
        toml::from_str(&raw).context(state.i18n.t("telegram.error.parse_config_failed"))?;
    let llm = value
        .get("llm")
        .and_then(|v| v.as_table())
        .ok_or_else(|| anyhow!("{}", state.i18n.t("telegram.error.llm_config_missing")))?;

    let selected_vendor = llm
        .get("selected_vendor")
        .and_then(|v| v.as_str())
        .unwrap_or("-");
    let selected_model = llm
        .get("selected_model")
        .and_then(|v| v.as_str())
        .unwrap_or("-");

    let vendors = ["openai", "google", "anthropic", "grok", "qwen", "custom"];
    let mut lines = vec![
        state.i18n.t_with(
            "telegram.msg.openclaw_current_selection",
            &[("vendor", selected_vendor), ("model", selected_model)],
        ),
        "".to_string(),
        state.i18n.t("telegram.msg.openclaw_preset_vendors"),
    ];

    for vendor in vendors {
        if let Some(tbl) = llm.get(vendor).and_then(|v| v.as_table()) {
            let model = tbl.get("model").and_then(|v| v.as_str()).unwrap_or("-");
            let models = tbl
                .get("models")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_else(|| "-".to_string());
            lines.push(
                state.i18n.t_with(
                    "telegram.msg.openclaw_vendor_line",
                    &[("vendor", vendor), ("model", model), ("models", &models)],
                ),
            );
        }
    }

    lines.push("".to_string());
    lines.push(state.i18n.t("telegram.msg.openclaw_restart_hint"));
    Ok(lines.join("\n"))
}

fn is_supported_model_vendor(vendor: &str) -> bool {
    matches!(
        vendor,
        "openai" | "google" | "anthropic" | "grok" | "qwen" | "custom"
    )
}

fn default_base_url_for_vendor(vendor: &str) -> &'static str {
    match vendor {
        "openai" => "https://api.openai.com/v1",
        "google" => "https://generativelanguage.googleapis.com/v1beta",
        "anthropic" => "https://api.anthropic.com/v1",
        "grok" => "https://api.x.ai/v1",
        "qwen" => "https://dashscope.aliyuncs.com/compatible-mode/v1",
        "custom" => "https://api.example.com/v1",
        _ => "https://api.example.com/v1",
    }
}

fn apply_model_config_value(value: &mut TomlValue, vendor: &str, model: &str) -> anyhow::Result<()> {
    if !is_supported_model_vendor(vendor) {
        return Err(anyhow!("unsupported vendor: {vendor}"));
    }

    let root = value
        .as_table_mut()
        .ok_or_else(|| anyhow!("config root is not a table"))?;
    let llm = root
        .entry("llm")
        .or_insert(TomlValue::Table(toml::map::Map::new()));
    if !llm.is_table() {
        *llm = TomlValue::Table(toml::map::Map::new());
    }

    let llm_tbl = llm
        .as_table_mut()
        .ok_or_else(|| anyhow!("llm is not a table"))?;
    llm_tbl.insert("selected_vendor".to_string(), TomlValue::String(vendor.to_string()));
    llm_tbl.insert("selected_model".to_string(), TomlValue::String(model.to_string()));

    let vendor_value = llm_tbl
        .entry(vendor.to_string())
        .or_insert(TomlValue::Table(toml::map::Map::new()));
    if !vendor_value.is_table() {
        *vendor_value = TomlValue::Table(toml::map::Map::new());
    }
    let vendor_tbl = vendor_value
        .as_table_mut()
        .ok_or_else(|| anyhow!("vendor section is not a table"))?;
    if !vendor_tbl.contains_key("base_url") {
        vendor_tbl.insert(
            "base_url".to_string(),
            TomlValue::String(default_base_url_for_vendor(vendor).to_string()),
        );
    }
    if !vendor_tbl.contains_key("api_key") {
        vendor_tbl.insert(
            "api_key".to_string(),
            TomlValue::String(format!("REPLACE_ME_{}_API_KEY", vendor.to_ascii_uppercase())),
        );
    }
    if !vendor_tbl.contains_key("max_concurrency") {
        vendor_tbl.insert("max_concurrency".to_string(), TomlValue::Integer(1));
    }
    if !vendor_tbl.contains_key("timeout_seconds") {
        vendor_tbl.insert("timeout_seconds".to_string(), TomlValue::Integer(60));
    }
    vendor_tbl.insert("model".to_string(), TomlValue::String(model.to_string()));

    let models_value = vendor_tbl
        .entry("models".to_string())
        .or_insert(TomlValue::Array(vec![]));
    if !models_value.is_array() {
        *models_value = TomlValue::Array(vec![]);
    }
    let models = models_value
        .as_array_mut()
        .ok_or_else(|| anyhow!("models is not an array"))?;
    let exists = models.iter().any(|v| v.as_str() == Some(model));
    if !exists {
        models.push(TomlValue::String(model.to_string()));
    }

    // Keep voice skills aligned with the selected primary vendor/model.
    for section in ["audio_synthesize", "audio_transcribe"] {
        let section_value = root
            .entry(section.to_string())
            .or_insert(TomlValue::Table(toml::map::Map::new()));
        if !section_value.is_table() {
            *section_value = TomlValue::Table(toml::map::Map::new());
        }
        let section_tbl = section_value
            .as_table_mut()
            .ok_or_else(|| anyhow!("{section} is not a table"))?;
        section_tbl.insert("default_vendor".to_string(), TomlValue::String(vendor.to_string()));
        section_tbl.insert("default_model".to_string(), TomlValue::String(model.to_string()));
    }

    Ok(())
}

fn set_model_config(state: &BotState, vendor: &str, model: &str) -> anyhow::Result<String> {
    if !is_supported_model_vendor(vendor) {
        return Err(anyhow!(
            "{}",
            state
                .i18n
                .t_with("telegram.msg.openclaw_unsupported_vendor", &[("vendor", vendor)])
        ));
    }

    let raw = fs::read_to_string("configs/config.toml")
        .context(state.i18n.t("telegram.error.read_config_failed"))?;
    let mut value: TomlValue =
        toml::from_str(&raw).context(state.i18n.t("telegram.error.parse_config_failed"))?;
    apply_model_config_value(&mut value, vendor, model)
        .context(state.i18n.t("telegram.error.config_not_table"))?;

    let output =
        toml::to_string_pretty(&value).context(state.i18n.t("telegram.error.serialize_config_failed"))?;
    fs::write("configs/config.toml", output).context(state.i18n.t("telegram.error.write_config_failed"))?;

    Ok(state.i18n.t_with(
        "telegram.msg.openclaw_set_ok",
        &[("vendor", vendor), ("model", model)],
    ))
}

#[cfg(test)]
mod model_config_tests {
    use super::*;

    #[test]
    fn apply_model_config_populates_custom_vendor_and_voice_defaults() {
        let mut v: TomlValue = toml::from_str(
            r#"
[llm]
selected_vendor = "openai"
selected_model = "gpt-4o-mini"
"#,
        )
        .expect("parse");
        apply_model_config_value(&mut v, "custom", "my-custom-model").expect("apply");

        let llm = v.get("llm").and_then(|x| x.as_table()).expect("llm");
        assert_eq!(
            llm.get("selected_vendor").and_then(|x| x.as_str()),
            Some("custom")
        );
        assert_eq!(
            llm.get("selected_model").and_then(|x| x.as_str()),
            Some("my-custom-model")
        );
        let custom = llm.get("custom").and_then(|x| x.as_table()).expect("custom");
        assert_eq!(
            custom.get("base_url").and_then(|x| x.as_str()),
            Some("https://api.example.com/v1")
        );
        assert_eq!(
            custom.get("api_key").and_then(|x| x.as_str()),
            Some("REPLACE_ME_CUSTOM_API_KEY")
        );
        assert_eq!(
            custom.get("model").and_then(|x| x.as_str()),
            Some("my-custom-model")
        );

        let synth = v
            .get("audio_synthesize")
            .and_then(|x| x.as_table())
            .expect("audio_synthesize");
        assert_eq!(
            synth.get("default_vendor").and_then(|x| x.as_str()),
            Some("custom")
        );
        assert_eq!(
            synth.get("default_model").and_then(|x| x.as_str()),
            Some("my-custom-model")
        );
        let trans = v
            .get("audio_transcribe")
            .and_then(|x| x.as_table())
            .expect("audio_transcribe");
        assert_eq!(
            trans.get("default_vendor").and_then(|x| x.as_str()),
            Some("custom")
        );
        assert_eq!(
            trans.get("default_model").and_then(|x| x.as_str()),
            Some("my-custom-model")
        );
    }

    #[test]
    fn apply_model_config_qwen_uses_expected_default_base_url() {
        let mut v: TomlValue = toml::from_str("[llm]\n").expect("parse");
        apply_model_config_value(&mut v, "qwen", "qwen-max-latest").expect("apply");
        let qwen = v
            .get("llm")
            .and_then(|x| x.get("qwen"))
            .and_then(|x| x.as_table())
            .expect("qwen");
        assert_eq!(
            qwen.get("base_url").and_then(|x| x.as_str()),
            Some("https://dashscope.aliyuncs.com/compatible-mode/v1")
        );
    }
}
