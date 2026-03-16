use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::IsTerminal;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context};
use claw_core::channel_chunk::{chunk_text_for_channel, SEGMENT_PREFIX_MAX_CHARS};
use claw_core::config::{AppConfig, ResolvedTelegramBotConfig};
use claw_core::hard_rules::types::VoiceModeIntentAliases;
use claw_core::hard_rules::voice_mode::{
    load_voice_mode_intent_aliases, parse_voice_mode_intent_decision,
};
use claw_core::types::{
    ApiResponse, AuthIdentity, BindChannelKeyRequest, ChannelKind, ExchangeCredentialStatus,
    GatewayInstanceRuntimeStatus, HealthResponse, ResolveChannelBindingRequest,
    ResolveChannelBindingResponse, SubmitTaskRequest, SubmitTaskResponse, TaskKind,
    TaskQueryResponse, TaskStatus, TelegramBotRuntimeStatus, UpsertExchangeCredentialRequest,
};
use reqwest::Client;
use serde_json::{json, Value as JsonValue};
use teloxide::prelude::*;
use teloxide::types::{
    CallbackQuery, ChatAction, InlineKeyboardButton, InlineKeyboardMarkup, InputFile, MediaKind,
    MessageKind, ParseMode,
};
use tokio::sync::oneshot;
use toml::Value as TomlValue;
use tracing::{debug, info, warn};

#[derive(Clone)]
struct BotState {
    bot_name: String,
    agent_id: String,
    admins: Arc<HashSet<i64>>,
    allowlist: Arc<HashSet<i64>>,
    access_mode: String,
    allowed_usernames: Arc<HashSet<String>>,
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
    crypto_confirm_ttl_seconds: u64,
    crypto_confirm_expiry_cancels: Arc<Mutex<HashMap<(i64, i32), oneshot::Sender<()>>>>,
    voice_chat_prompt_template: String,
    voice_mode_intent_prompt_template: String,
    pending_resume_by_chat: Arc<Mutex<HashMap<i64, PendingResumeContext>>>,
    pending_key_bind_by_chat: Arc<Mutex<HashSet<i64>>>,
    bound_identity_by_chat: Arc<Mutex<HashMap<i64, AuthIdentity>>>,
    i18n: Arc<TextCatalog>,
    status_file_path: PathBuf,
    gateway_status_file_path: PathBuf,
}

#[derive(Debug, Clone)]
struct PendingResumeContext {
    user_id: i64,
    created_at_secs: u64,
    resume_context: JsonValue,
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
    include_str!("../../../prompts/vendors/default/voice_chat_prompt.md");
const DEFAULT_VOICE_MODE_INTENT_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/vendors/default/voice_mode_intent_prompt.md");
const VOICE_CHAT_PROMPT_PATH: &str = "prompts/voice_chat_prompt.md";
const VOICE_MODE_INTENT_PROMPT_PATH: &str = "prompts/voice_mode_intent_prompt.md";
const RESUME_CONTEXT_TTL_SECONDS: u64 = 30 * 60;
const VOICE_MODE_INTENT_ALIASES_PATH: &str =
    "configs/command_intent/voice_mode_intent_aliases.toml";

fn log_color_enabled() -> bool {
    match std::env::var("RUSTCLAW_LOG_COLOR") {
        Ok(v) if v == "0" || v.eq_ignore_ascii_case("false") => false,
        Ok(v) if v == "1" || v.eq_ignore_ascii_case("true") => true,
        _ => std::io::stdout().is_terminal(),
    }
}

fn transport_highlight_tag(kind: &str) -> String {
    let upper = kind.to_ascii_uppercase();
    if !log_color_enabled() {
        return format!("[{upper}]");
    }
    let code = match kind {
        "transport_prompt" => "38;5;45",
        _ => "1",
    };
    format!("\x1b[{code}m[{upper}]\x1b[0m")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CryptoConfirmCallbackAction {
    Yes,
    No,
    DoneNoop,
    ExpiredNoop,
}

fn parse_crypto_confirm_callback(data: &str) -> Option<(CryptoConfirmCallbackAction, Option<u64>)> {
    if data == "crypto_confirm_done_noop" {
        return Some((CryptoConfirmCallbackAction::DoneNoop, None));
    }
    if data == "crypto_confirm_expired_noop" {
        return Some((CryptoConfirmCallbackAction::ExpiredNoop, None));
    }
    let (action, prefix) = if data.starts_with("crypto_confirm_yes") {
        (CryptoConfirmCallbackAction::Yes, "crypto_confirm_yes")
    } else if data.starts_with("crypto_confirm_no") {
        (CryptoConfirmCallbackAction::No, "crypto_confirm_no")
    } else {
        return None;
    };
    let expiry = data
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix(':'))
        .and_then(|v| v.parse::<u64>().ok());
    Some((action, expiry))
}

fn cancel_crypto_confirm_expiry(state: &BotState, chat_id: i64, message_id: i32) {
    if let Ok(mut pending) = state.crypto_confirm_expiry_cancels.lock() {
        if let Some(cancel_tx) = pending.remove(&(chat_id, message_id)) {
            let _ = cancel_tx.send(());
        }
    }
}

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

fn can_change_voice_mode(_state: &BotState, _user_id: i64) -> bool {
    true
}

fn should_expect_key_reply(state: &BotState, chat_id: i64) -> bool {
    state
        .pending_key_bind_by_chat
        .lock()
        .ok()
        .is_some_and(|set| set.contains(&chat_id))
}

fn set_expect_key_reply(state: &BotState, chat_id: i64, enabled: bool) {
    if let Ok(mut set) = state.pending_key_bind_by_chat.lock() {
        if enabled {
            set.insert(chat_id);
        } else {
            set.remove(&chat_id);
        }
    }
}

fn store_bound_identity(state: &BotState, chat_id: i64, identity: &AuthIdentity) {
    if let Ok(mut map) = state.bound_identity_by_chat.lock() {
        map.insert(chat_id, identity.clone());
    }
}

fn bound_user_key_for_chat(state: &BotState, chat_id: i64) -> Option<String> {
    state
        .bound_identity_by_chat
        .lock()
        .ok()
        .and_then(|map| map.get(&chat_id).map(|identity| identity.user_key.clone()))
}

fn normalize_telegram_username(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches('@').trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

fn telegram_user_allowed(state: &BotState, user_id: i64, username: Option<&str>) -> bool {
    if state.access_mode != "specified" {
        return true;
    }
    if state.admins.contains(&user_id) || state.allowlist.contains(&user_id) {
        return true;
    }
    username
        .and_then(normalize_telegram_username)
        .is_some_and(|name| state.allowed_usernames.contains(&name))
}

async fn resolve_telegram_identity(
    state: &BotState,
    platform_user_id: i64,
    platform_chat_id: i64,
) -> anyhow::Result<Option<AuthIdentity>> {
    let url = format!("{}/v1/auth/channel/resolve", state.clawd_base_url);
    let req = ResolveChannelBindingRequest {
        channel: ChannelKind::Telegram,
        telegram_bot_name: Some(state.bot_name.clone()),
        external_user_id: Some(platform_user_id.to_string()),
        external_chat_id: Some(platform_chat_id.to_string()),
    };
    let resp = state.client.post(&url).json(&req).send().await?;
    let status = resp.status();
    let body: ApiResponse<ResolveChannelBindingResponse> = resp.json().await?;
    if !status.is_success() || !body.ok {
        return Err(anyhow!(
            "resolve telegram identity failed: {}",
            body.error.unwrap_or_else(|| "unknown error".to_string())
        ));
    }
    Ok(body.data.and_then(|v| v.identity))
}

async fn bind_telegram_identity(
    state: &BotState,
    platform_user_id: i64,
    platform_chat_id: i64,
    user_key: &str,
) -> anyhow::Result<Option<AuthIdentity>> {
    let url = format!("{}/v1/auth/channel/bind", state.clawd_base_url);
    let req = BindChannelKeyRequest {
        channel: ChannelKind::Telegram,
        telegram_bot_name: Some(state.bot_name.clone()),
        external_user_id: Some(platform_user_id.to_string()),
        external_chat_id: Some(platform_chat_id.to_string()),
        user_key: user_key.trim().to_string(),
    };
    let resp = state.client.post(&url).json(&req).send().await?;
    let status = resp.status();
    let body: ApiResponse<AuthIdentity> = resp.json().await?;
    if !status.is_success() {
        if status.as_u16() == 401 {
            return Ok(None);
        }
        return Err(anyhow!(
            "bind telegram identity failed: {}",
            body.error.unwrap_or_else(|| "unknown error".to_string())
        ));
    }
    if !body.ok {
        return Ok(None);
    }
    Ok(body.data)
}

async fn fetch_crypto_credential_status(
    state: &BotState,
    identity: &AuthIdentity,
) -> anyhow::Result<Vec<ExchangeCredentialStatus>> {
    let url = format!("{}/v1/auth/crypto-credentials", state.clawd_base_url);
    let resp = state
        .client
        .get(&url)
        .header("X-RustClaw-Key", identity.user_key.as_str())
        .send()
        .await?;
    let status = resp.status();
    let body: ApiResponse<Vec<ExchangeCredentialStatus>> = resp.json().await?;
    if !status.is_success() || !body.ok {
        return Err(anyhow!(
            "read crypto credential status failed: {}",
            body.error.unwrap_or_else(|| "unknown error".to_string())
        ));
    }
    Ok(body.data.unwrap_or_default())
}

async fn upsert_crypto_credential(
    state: &BotState,
    identity: &AuthIdentity,
    exchange: &str,
    api_key: &str,
    api_secret: &str,
    passphrase: Option<&str>,
) -> anyhow::Result<ExchangeCredentialStatus> {
    let url = format!("{}/v1/auth/crypto-credentials", state.clawd_base_url);
    let resp = state
        .client
        .post(&url)
        .header("X-RustClaw-Key", identity.user_key.as_str())
        .json(&UpsertExchangeCredentialRequest {
            exchange: exchange.to_string(),
            api_key: api_key.to_string(),
            api_secret: api_secret.to_string(),
            passphrase: passphrase.map(|v| v.to_string()),
        })
        .send()
        .await?;
    let status = resp.status();
    let body: ApiResponse<ExchangeCredentialStatus> = resp.json().await?;
    if !status.is_success() || !body.ok {
        return Err(anyhow!(
            "upsert crypto credential failed: {}",
            body.error.unwrap_or_else(|| "unknown error".to_string())
        ));
    }
    body.data
        .ok_or_else(|| anyhow!("upsert crypto credential missing data"))
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
    info!(
        "{} transport_prompt_use flow=voice_mode_intent_detect prompt_name=voice_mode_intent_prompt chat_id={} user_id={} prompt_file={}",
        transport_highlight_tag("transport_prompt"),
        chat_id,
        user_id,
        VOICE_MODE_INTENT_PROMPT_PATH
    );
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
    let out = match poll_task_result(
        state,
        &task_id,
        bound_user_key_for_chat(state, chat_id).as_deref(),
        Some(12),
    )
    .await
    {
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

fn pending_resume_valid_for(pending: &PendingResumeContext, user_id: i64, now_secs: u64) -> bool {
    if pending.user_id != user_id {
        return false;
    }
    now_secs.saturating_sub(pending.created_at_secs) <= RESUME_CONTEXT_TTL_SECONDS
}

async fn maybe_handle_resume_continuation(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    user_id: i64,
    prompt: &str,
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
        return Ok(false);
    }
    let agent_enabled = state
        .agent_off_chats
        .lock()
        .map(|set| !set.contains(&chat_id))
        .unwrap_or(true);
    let payload = json!({
        "text": prompt,
        "agent_mode": agent_enabled,
        "source": "resume_continue_execute",
        "resume_user_text": prompt,
        "resume_context": pending.resume_context,
    });
    match submit_task_only(state, user_id, chat_id, TaskKind::Ask, payload).await {
        Ok(task_id) => {
            spawn_task_result_delivery(
                bot.clone(),
                state.clone(),
                msg.chat.id,
                user_id,
                task_id,
                None,
                state.i18n.t("telegram.msg.process_failed"),
            );
            Ok(true)
        }
        Err(err) => {
            bot.send_message(
                msg.chat.id,
                state.i18n.t_with(
                    "telegram.msg.process_failed",
                    &[("error", &err.to_string())],
                ),
            )
            .await
            .context("send resume submit error failed")?;
            Ok(true)
        }
    }
}

fn effective_voice_reply_mode_for_chat(state: &BotState, chat_id: i64) -> String {
    let fallback =
        normalize_voice_reply_mode(&state.voice_reply_mode).unwrap_or_else(|| "voice".to_string());
    if let Ok(map) = state.voice_reply_mode_by_chat.lock() {
        if let Some(mode) = map
            .get(&chat_id)
            .and_then(|v| normalize_voice_reply_mode(v))
        {
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
    let client = Client::builder()
        .timeout(Duration::from_secs(config.server.request_timeout_seconds))
        .build()
        .context("build reqwest client failed")?;
    let voice_mode_intent_aliases =
        Arc::new(load_voice_mode_intent_aliases(VOICE_MODE_INTENT_ALIASES_PATH));
    let workspace_root = workspace_root();
    let prompt_vendor =
        prompt_vendor_name_from_selected_vendor(config.llm.selected_vendor.as_deref());
    let voice_chat_prompt_template = load_prompt_template(
        &workspace_root,
        &prompt_vendor,
        VOICE_CHAT_PROMPT_PATH,
        DEFAULT_VOICE_CHAT_PROMPT_TEMPLATE,
    );
    let voice_mode_intent_prompt_template = load_prompt_template(
        &workspace_root,
        &prompt_vendor,
        VOICE_MODE_INTENT_PROMPT_PATH,
        DEFAULT_VOICE_MODE_INTENT_PROMPT_TEMPLATE,
    );
    let telegram_runtime_bots = config.telegram_runtime_bots();
    if telegram_runtime_bots.is_empty() {
        return Err(anyhow!("no telegram bot configured"));
    }
    info!(
        "telegram runtimes configured: count={} names={:?}",
        telegram_runtime_bots.len(),
        telegram_runtime_bots
            .iter()
            .map(|bot| bot.name.clone())
            .collect::<Vec<_>>()
    );

    let mut join_set = tokio::task::JoinSet::new();
    let mut remaining = 0usize;
    for bot_config in telegram_runtime_bots {
        let state = build_bot_state(
            &config,
            &bot_config,
            client.clone(),
            Arc::clone(&voice_mode_intent_aliases),
            &workspace_root,
            &voice_chat_prompt_template,
            &voice_mode_intent_prompt_template,
        );
        join_set.spawn(run_telegram_bot_runtime(state));
        remaining += 1;
    }
    let mut last_error: Option<String> = None;
    while let Some(result) = join_set.join_next().await {
        remaining = remaining.saturating_sub(1);
        match result {
            Ok(Ok(())) => {
                warn!(
                    "telegram bot runtime stopped: remaining_runtimes={}",
                    remaining
                );
                if last_error.is_none() {
                    last_error = Some("one telegram bot runtime stopped".to_string());
                }
            }
            Ok(Err(err)) => {
                warn!(
                    "telegram bot runtime failed: remaining_runtimes={} err={}",
                    remaining, err
                );
                last_error = Some(err.to_string());
            }
            Err(err) => {
                warn!(
                    "telegram bot runtime join failed: remaining_runtimes={} err={}",
                    remaining, err
                );
                last_error = Some(format!("telegram bot runtime join failed: {err}"));
            }
        }
        if remaining == 0 {
            break;
        }
    }
    Err(anyhow!(
        "{}",
        last_error.unwrap_or_else(|| "all telegram bot runtimes exited".to_string())
    ))
}

fn build_bot_state(
    config: &AppConfig,
    bot_config: &ResolvedTelegramBotConfig,
    client: Client,
    voice_mode_intent_aliases: Arc<VoiceModeIntentAliases>,
    workspace_root: &Path,
    voice_chat_prompt_template: &str,
    voice_mode_intent_prompt_template: &str,
) -> BotState {
    let i18n_path = resolve_i18n_path(&bot_config.language, &bot_config.i18n_path);
    let i18n = match TextCatalog::load(&i18n_path) {
        Ok(v) => Arc::new(v),
        Err(err) => {
            warn!(
                "load i18n file failed: bot_name={} path={} err={}",
                bot_config.name, i18n_path, err
            );
            Arc::new(TextCatalog::fallback())
        }
    };

    let mut allowlist = HashSet::new();
    for id in &bot_config.allowlist {
        allowlist.insert(*id);
    }

    let mut admins = HashSet::new();
    for id in &bot_config.admins {
        admins.insert(*id);
        allowlist.insert(*id);
    }
    let mut allowed_usernames = HashSet::new();
    for username in &bot_config.allowed_usernames {
        if let Some(normalized) = normalize_telegram_username(username) {
            allowed_usernames.insert(normalized);
        }
    }

    BotState {
        bot_name: bot_config.name.clone(),
        agent_id: bot_config.agent_id.clone(),
        admins: Arc::new(admins),
        allowlist: Arc::new(allowlist),
        access_mode: match bot_config.access_mode.trim().to_ascii_lowercase().as_str() {
            "specified" => "specified".to_string(),
            _ => "public".to_string(),
        },
        allowed_usernames: Arc::new(allowed_usernames),
        skills_list: Arc::new(config.skills.skills_list.clone()),
        agent_off_chats: Arc::new(Mutex::new(HashSet::new())),
        clawd_base_url: clawd_base_url_from_config(config),
        client,
        poll_interval_ms: config.worker.poll_interval_ms,
        task_wait_seconds: config.worker.task_timeout_seconds,
        queue_limit: config.worker.queue_limit,
        auto_vision_on_image_only: config.telegram.auto_vision_on_image_only,
        pending_image_by_chat: Arc::new(Mutex::new(HashMap::new())),
        bot_token: bot_config.bot_token.clone(),
        image_inbox_dir: "image/upload".to_string(),
        audio_inbox_dir: config.telegram.audio_inbox_dir.clone(),
        voice_reply_mode: config.telegram.voice_reply_mode.clone(),
        voice_mode_nl_intent_enabled: config.telegram.voice_mode_nl_intent_enabled,
        voice_reply_mode_by_chat: Arc::new(Mutex::new(load_voice_reply_mode_by_chat(config))),
        voice_mode_intent_aliases,
        max_audio_input_bytes: config.telegram.max_audio_input_bytes.max(1024),
        sendfile_admin_only: config.telegram.sendfile.admin_only,
        sendfile_full_access: config.telegram.sendfile.full_access,
        sendfile_allowed_dirs: Arc::new(config.telegram.sendfile.allowed_dirs.clone()),
        ephemeral_image_saved_seconds: config.telegram.ephemeral_image_saved_seconds,
        crypto_confirm_ttl_seconds: config.telegram.crypto_confirm_ttl_seconds.max(1),
        crypto_confirm_expiry_cancels: Arc::new(Mutex::new(HashMap::new())),
        voice_chat_prompt_template: voice_chat_prompt_template.to_string(),
        voice_mode_intent_prompt_template: voice_mode_intent_prompt_template.to_string(),
        pending_resume_by_chat: Arc::new(Mutex::new(HashMap::new())),
        pending_key_bind_by_chat: Arc::new(Mutex::new(HashSet::new())),
        bound_identity_by_chat: Arc::new(Mutex::new(HashMap::new())),
        i18n,
        status_file_path: telegram_bot_status_file_path(workspace_root, &bot_config.name),
        gateway_status_file_path: gateway_instance_status_file_path(
            workspace_root,
            "telegram",
            &bot_config.name,
        ),
    }
}

fn clawd_base_url_from_config(config: &AppConfig) -> String {
    config.server.clawd_base_url.clone().unwrap_or_else(|| {
        let listen = config.server.listen.as_str();
        let host = if listen.starts_with("0.0.0.0:") {
            listen.replacen("0.0.0.0", "127.0.0.1", 1)
        } else {
            listen.to_string()
        };
        format!("http://{}", host)
    })
}

fn load_voice_reply_mode_by_chat(config: &AppConfig) -> HashMap<i64, String> {
    let mut voice_reply_mode_by_chat = HashMap::new();
    for (chat_id_raw, mode_raw) in &config.telegram.voice_reply_mode_by_chat {
        if let (Ok(chat_id), Some(mode)) = (
            chat_id_raw.parse::<i64>(),
            normalize_voice_reply_mode(mode_raw),
        ) {
            voice_reply_mode_by_chat.insert(chat_id, mode);
        }
    }
    voice_reply_mode_by_chat
}

fn telegram_bot_status_file_path(workspace_root: &Path, bot_name: &str) -> PathBuf {
    let safe_name = sanitize_status_name(bot_name);
    workspace_root
        .join("run")
        .join("telegram-bot-status")
        .join(format!("{}.json", if safe_name.is_empty() { "bot" } else { &safe_name }))
}

fn gateway_instance_status_file_path(workspace_root: &Path, kind: &str, name: &str) -> PathBuf {
    let safe_kind = sanitize_status_name(kind);
    let safe_name = sanitize_status_name(name);
    workspace_root
        .join("run")
        .join("gateway-instance-status")
        .join(format!(
            "{}__{}.json",
            if safe_kind.is_empty() { "instance" } else { &safe_kind },
            if safe_name.is_empty() { "primary" } else { &safe_name }
        ))
}

fn sanitize_status_name(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

async fn write_bot_runtime_status(path: &Path, status: &TelegramBotRuntimeStatus) {
    let Some(parent) = path.parent() else {
        return;
    };
    if let Err(err) = tokio::fs::create_dir_all(parent).await {
        warn!("create telegram bot status dir failed: path={} err={}", parent.display(), err);
        return;
    }
    let bytes = match serde_json::to_vec(status) {
        Ok(bytes) => bytes,
        Err(err) => {
            warn!(
                "serialize telegram bot status failed: bot_name={} err={}",
                status.name, err
            );
            return;
        }
    };
    if let Err(err) = tokio::fs::write(path, bytes).await {
        warn!(
            "write telegram bot status failed: bot_name={} path={} err={}",
            status.name,
            path.display(),
            err
        );
    }
}

async fn write_gateway_instance_runtime_status(path: &Path, status: &GatewayInstanceRuntimeStatus) {
    let Some(parent) = path.parent() else {
        return;
    };
    if let Err(err) = tokio::fs::create_dir_all(parent).await {
        warn!(
            "create gateway instance status dir failed: path={} err={}",
            parent.display(),
            err
        );
        return;
    }
    let bytes = match serde_json::to_vec(status) {
        Ok(bytes) => bytes,
        Err(err) => {
            warn!(
                "serialize gateway instance status failed: scope={} err={}",
                status.scope, err
            );
            return;
        }
    };
    if let Err(err) = tokio::fs::write(path, bytes).await {
        warn!(
            "write gateway instance status failed: scope={} path={} err={}",
            status.scope,
            path.display(),
            err
        );
    }
}

async fn write_runtime_statuses(
    state: &BotState,
    healthy: bool,
    status: &str,
    last_heartbeat_ts: Option<i64>,
    last_error: Option<String>,
) {
    write_bot_runtime_status(
        &state.status_file_path,
        &TelegramBotRuntimeStatus {
            name: state.bot_name.clone(),
            healthy,
            status: status.to_string(),
            last_heartbeat_ts,
            last_error: last_error.clone(),
        },
    )
    .await;
    write_gateway_instance_runtime_status(
        &state.gateway_status_file_path,
        &GatewayInstanceRuntimeStatus {
            kind: "telegram".to_string(),
            name: state.bot_name.clone(),
            scope: format!("telegram:{}", state.bot_name),
            healthy,
            status: status.to_string(),
            last_heartbeat_ts,
            last_error,
        },
    )
    .await;
}

async fn run_telegram_bot_runtime(state: BotState) -> anyhow::Result<()> {
    let bot = Bot::new(state.bot_token.clone());
    write_runtime_statuses(
        &state,
        false,
        "starting",
        Some(unix_ts() as i64),
        None,
    )
    .await;
    let mut startup_error: Option<String> = None;
    if let Err(err) = register_telegram_commands_and_menu(&state.bot_token, state.i18n.as_ref()).await
    {
        warn!(
            "register Telegram menu failed: bot_name={} err={}",
            state.bot_name, err
        );
        startup_error = Some(err.to_string());
    } else {
        info!("registered Telegram menu commands: bot_name={}", state.bot_name);
    }

    let mut admins_list: Vec<i64> = state.admins.iter().copied().collect();
    admins_list.sort_unstable();
    let mut allowlist_list: Vec<i64> = state.allowlist.iter().copied().collect();
    allowlist_list.sort_unstable();
    let mut allowed_usernames_list: Vec<String> = state.allowed_usernames.iter().cloned().collect();
    allowed_usernames_list.sort_unstable();

    info!(
        "telegram bot [{}] {}",
        state.bot_name,
        state.i18n.t_with(
            "telegram.log.started",
            &[
                ("admins", &format!("{admins_list:?}")),
                ("allowlist", &format!("{allowlist_list:?}")),
                ("access_mode", &state.access_mode),
                ("allowed_usernames", &format!("{allowed_usernames_list:?}")),
                ("skills", &state.skills_list.join(",")),
            ],
        )
    );
    info!(
        "telegram bot [{}] {}",
        state.bot_name,
        state.i18n.t_with(
            "telegram.log.startup_memory_rss",
            &[("bytes", &current_rss_bytes().unwrap_or(0).to_string())]
        )
    );
    write_runtime_statuses(
        &state,
        true,
        "running",
        Some(unix_ts() as i64),
        startup_error.clone(),
    )
    .await;
    let heartbeat_state = state.clone();
    let heartbeat_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            write_runtime_statuses(
                &heartbeat_state,
                true,
                "running",
                Some(unix_ts() as i64),
                None,
            )
            .await;
        }
    });

    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(handle_message))
        .branch(Update::filter_callback_query().endpoint(handle_callback_query));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state.clone()])
        .build()
        .dispatch()
        .await;

    heartbeat_task.abort();
    write_runtime_statuses(
        &state,
        false,
        "stopped",
        Some(unix_ts() as i64),
        startup_error,
    )
    .await;
    warn!("telegram bot runtime exited: bot_name={}", state.bot_name);
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
            { "command": "voicemode", "description": i18n.t("telegram.menu.voicemode_desc") },
            { "command": "crypto", "description": i18n.t("telegram.menu.crypto_desc") },
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
    if !cmd_json
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
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
        return Err(anyhow!(
            "setChatMenuButton http {}: {}",
            menu_status,
            menu_body
        ));
    }
    let menu_json: JsonValue =
        serde_json::from_str(&menu_body).unwrap_or_else(|_| json!({"ok": false}));
    if !menu_json
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
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
    let platform_user_id = msg
        .from()
        .map(|u| i64::try_from(u.id.0).unwrap_or_default())
        .unwrap_or_default();
    let platform_username = msg
        .from()
        .and_then(|u| u.username.clone());
    let platform_chat_id = msg.chat.id.0;
    let text = msg.text().unwrap_or_default();
    info!(
        "handle_message: chat_id={} user_id={} username={} text={}",
        platform_chat_id,
        platform_user_id,
        platform_username.as_deref().unwrap_or("-"),
        text
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
            let trimmed = text.trim();
            let maybe_candidate = trimmed
                .strip_prefix("/key")
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string)
                .or_else(|| {
                    if should_expect_key_reply(&state, platform_chat_id) && !trimmed.is_empty() {
                        Some(trimmed.to_string())
                    } else {
                        None
                    }
                });
            if let Some(candidate) = maybe_candidate {
                if let Some(identity) =
                    bind_telegram_identity(&state, platform_user_id, platform_chat_id, &candidate)
                        .await?
                {
                    set_expect_key_reply(&state, platform_chat_id, false);
                    store_bound_identity(&state, platform_chat_id, &identity);
                    bot.send_message(
                        msg.chat.id,
                        "Key 绑定成功，请重新发送刚才的消息。\nKey bound successfully. Please send your previous message again.",
                    )
                    .await
                    .context("send key bind success failed")?;
                    return Ok(());
                } else {
                    set_expect_key_reply(&state, platform_chat_id, true);
                    bot.send_message(
                        msg.chat.id,
                        "Key 无效，请重新输入。\nInvalid key. Please try again.",
                    )
                    .await
                    .context("send invalid key failed")?;
                    return Ok(());
                }
            }
            None
        }
    };
    let user_id = bound_identity
        .as_ref()
        .map(|identity| identity.user_id)
        .unwrap_or(platform_user_id);

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
        bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.console_only_command"))
            .await
            .context("send /rustclaw console-only failed")?;
        return Ok(());
    }

    if text.starts_with("/cryptoapi") {
        bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.console_only_command"))
            .await
            .context("send /cryptoapi console-only failed")?;
        return Ok(());
    }

    if text.starts_with("/voicemode") {
        if !can_change_voice_mode(&state, user_id) {
            bot.send_message(
                msg.chat.id,
                state.i18n.t("telegram.msg.voicemode_admin_only"),
            )
            .await
            .context("send /voicemode unauthorized failed")?;
            return Ok(());
        }
        let mode = text.strip_prefix("/voicemode").unwrap_or_default().trim();
        let reply = handle_voicemode_command(&state, msg.chat.id.0, text)?;
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

    if text.starts_with("/cancel") {
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
            bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.console_only_command"))
                .await
                .context("send /crypto add console-only failed")?;
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

    if text.starts_with("/sendfile") {
        bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.console_only_command"))
            .await
            .context("send /sendfile console-only failed")?;
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

async fn handle_callback_query(bot: Bot, q: CallbackQuery, state: BotState) -> anyhow::Result<()> {
    let Some(data) = q.data.as_deref() else {
        return Ok(());
    };
    let Some((callback_action, expires_at_secs)) = parse_crypto_confirm_callback(data) else {
        return Ok(());
    };
    debug!(
        "phase=callback callback_id={} from_user_id={} data={}",
        q.id, q.from.id.0, data
    );
    if callback_action == CryptoConfirmCallbackAction::DoneNoop {
        if let Err(err) = bot
            .answer_callback_query(q.id.clone())
            .text(
                state
                    .i18n
                    .t("telegram.msg.crypto_confirm_callback_done_ack"),
            )
            .await
        {
            warn!("answer done callback query failed: {}", err);
        }
        return Ok(());
    }
    if callback_action == CryptoConfirmCallbackAction::ExpiredNoop {
        if let Err(err) = bot
            .answer_callback_query(q.id.clone())
            .text(
                state
                    .i18n
                    .t("telegram.msg.crypto_confirm_callback_expired_ack"),
            )
            .await
        {
            warn!("answer expired callback query failed: {}", err);
        }
        return Ok(());
    }
    let is_yes = callback_action == CryptoConfirmCallbackAction::Yes;

    let Some(message) = q.message.as_ref() else {
        return Ok(());
    };
    let chat_id = message.chat.id;
    let message_id = message.id;
    cancel_crypto_confirm_expiry(&state, chat_id.0, message_id.0);
    debug!(
        "phase=callback_ack chat_id={} message_id={} data={}",
        chat_id.0, message_id.0, data
    );
    let now_secs = unix_ts();
    if expires_at_secs.map(|v| now_secs > v).unwrap_or(false) {
        let expired_hint = state.i18n.t("telegram.msg.crypto_confirm_hint_expired");
        let msg_text = message.text();
        if let Some(original_text) = msg_text {
            let expired_text = build_expired_trade_text(original_text, &expired_hint);
            if let Err(err) = bot
                .edit_message_text(chat_id, message_id, expired_text)
                .await
            {
                warn!("edit expired callback message text failed: {}", err);
            }
        }
        if let Err(err) = bot
            .edit_message_reply_markup(chat_id, message_id)
            .reply_markup(InlineKeyboardMarkup::new(
                Vec::<Vec<InlineKeyboardButton>>::new(),
            ))
            .await
        {
            warn!("edit expired callback message markup failed: {}", err);
        }
        if let Err(err) = bot
            .answer_callback_query(q.id.clone())
            .text(
                state
                    .i18n
                    .t("telegram.msg.crypto_confirm_callback_expired_ack"),
            )
            .await
        {
            warn!("answer expired callback query failed: {}", err);
        }
        return Ok(());
    }
    if let Err(err) = bot
        .answer_callback_query(q.id.clone())
        .text(if is_yes {
            state.i18n.t("telegram.msg.crypto_confirm_callback_yes_ack")
        } else {
            state.i18n.t("telegram.msg.crypto_confirm_callback_no_ack")
        })
        .await
    {
        warn!("answer callback query failed: {}", err);
    }

    if let Err(err) = bot
        .edit_message_reply_markup(chat_id, message_id)
        .reply_markup(InlineKeyboardMarkup::new(
            Vec::<Vec<InlineKeyboardButton>>::new(),
        ))
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
    let prompt = if is_yes { "yes" } else { "no" };
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
                state.i18n.t_with(
                    "telegram.msg.process_failed",
                    &[("error", &err.to_string())],
                ),
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
        state.image_inbox_dir, msg.chat.id.0, user_id, ts, normalized_ext
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
        state.image_inbox_dir, msg.chat.id.0, user_id, ts, normalized_ext
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
        state.audio_inbox_dir, msg.chat.id.0, user_id, ts, normalized_ext
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
    let transcript = poll_task_result(
        state,
        &transcribe_task_id,
        bound_user_key_for_chat(state, msg.chat.id.0).as_deref(),
        Some(120),
    )
    .await
    .context("poll audio_transcribe result failed")?;
    let transcript = transcript.join("\n").trim().to_string();
    let transcript = transcript.as_str();
    if transcript.is_empty() {
        bot.send_message(
            msg.chat.id,
            state.i18n.t("telegram.msg.audio_transcript_empty"),
        )
        .await
        .context("send empty transcript message failed")?;
        return Ok(());
    }

    let agent_enabled = state
        .agent_off_chats
        .lock()
        .map(|set| !set.contains(&msg.chat.id.0))
        .unwrap_or(true);
    info!(
        "{} transport_prompt_use flow=voice_chat prompt_name=voice_chat_prompt chat_id={} user_id={} prompt_file={}",
        transport_highlight_tag("transport_prompt"),
        msg.chat.id.0,
        user_id,
        VOICE_CHAT_PROMPT_PATH
    );
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
    let answers = poll_task_result(
        state,
        &ask_task_id,
        bound_user_key_for_chat(state, msg.chat.id.0).as_deref(),
        Some(state.task_wait_seconds.max(300)),
    )
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
            match submit_task_only(
                state,
                user_id,
                msg.chat.id.0,
                TaskKind::RunSkill,
                tts_payload,
            )
            .await
            {
                Ok(tts_task_id) => match poll_task_result(
                    state,
                    &tts_task_id,
                    bound_user_key_for_chat(state, msg.chat.id.0).as_deref(),
                    Some(90),
                )
                .await
                {
                    Ok(tts_answer) => {
                        for msg_text in tts_answer {
                            let _ =
                                send_text_or_image(bot, state, msg.chat.id, &msg_text, false).await;
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
    if matches!(
        e.as_str(),
        "ogg" | "mp3" | "wav" | "m4a" | "aac" | "flac" | "opus"
    ) {
        e
    } else {
        "ogg".to_string()
    }
}

fn normalize_prompt_vendor_name(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "anthropic" | "claude" => "claude".to_string(),
        "google" | "gemini" => "google".to_string(),
        "openai" => "openai".to_string(),
        "grok" | "xai" => "grok".to_string(),
        "deepseek" => "deepseek".to_string(),
        "qwen" => "qwen".to_string(),
        "minimax" => "minimax".to_string(),
        "custom" => "openai".to_string(),
        _ => "default".to_string(),
    }
}

fn prompt_vendor_name_from_selected_vendor(selected_vendor: Option<&str>) -> String {
    selected_vendor
        .map(normalize_prompt_vendor_name)
        .unwrap_or_else(|| "default".to_string())
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn resolve_prompt_rel_path_for_vendor(
    workspace_root: &Path,
    vendor: &str,
    rel_path: &str,
) -> String {
    let trimmed = rel_path.trim();
    if trimmed.is_empty() || !trimmed.starts_with("prompts/") {
        return trimmed.to_string();
    }
    let suffix = trimmed.trim_start_matches("prompts/");
    let vendor_candidate = format!("prompts/vendors/{vendor}/{suffix}");
    if workspace_root.join(&vendor_candidate).is_file() {
        return vendor_candidate;
    }
    let default_candidate = format!("prompts/vendors/default/{suffix}");
    if vendor != "default" && workspace_root.join(&default_candidate).is_file() {
        return default_candidate;
    }
    trimmed.to_string()
}

fn load_prompt_template(
    workspace_root: &Path,
    prompt_vendor: &str,
    rel_path: &str,
    default_template: &str,
) -> String {
    let resolved_path = resolve_prompt_rel_path_for_vendor(workspace_root, prompt_vendor, rel_path);
    match fs::read_to_string(workspace_root.join(resolved_path)) {
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
        // Telegram 的 typing 状态约 5 秒后过期，需在过期前重新发送以保持「正在输入」持续显示直到回复。
        const TYPING_REFRESH_INTERVAL_SECS: u64 = 4;
        let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            loop {
                let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(TYPING_REFRESH_INTERVAL_SECS)) => {}
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
    let explicit_file_tokens = dedupe_preserve_order(extract_prefixed_tokens(answer, FILE_PREFIX));
    let (explicit_file_paths, missing_explicit_file_tokens) =
        resolve_delivery_paths(&explicit_file_tokens);
    let mut file_paths = explicit_file_paths.clone();
    let voice_paths = dedupe_preserve_order(extract_prefixed_paths(answer, VOICE_PREFIX));
    let inferred_write_paths = if file_paths.is_empty() {
        dedupe_preserve_order(extract_written_file_paths(answer))
    } else {
        Vec::new()
    };
    if !inferred_write_paths.is_empty() {
        file_paths.extend(inferred_write_paths.clone());
        file_paths = dedupe_preserve_order(file_paths);
    }
    // If both IMAGE_FILE and FILE contain the same path, keep FILE only.
    let file_set = file_paths.iter().cloned().collect::<HashSet<_>>();
    image_paths.retain(|p| !file_set.contains(p));

    if !image_paths.is_empty()
        || !file_paths.is_empty()
        || !voice_paths.is_empty()
        || !missing_explicit_file_tokens.is_empty()
    {
        debug!(
            "phase=deliver_media chat_id={} answer_fp={} image_count={} file_count={} voice_count={} preface_preview={}",
            chat_id.0,
            text_fingerprint_hex(answer),
            image_paths.len(),
            file_paths.len(),
            voice_paths.len(),
            text_preview_for_log(answer, 120)
        );
        let ephemeral_image_saved_hint = answer.lines().any(|line| {
            line.trim()
                .eq_ignore_ascii_case(EPHEMERAL_IMAGE_SAVED_TOKEN)
        });
        let mut text_without_tokens = strip_prefixed_tokens(
            answer,
            &[PREFIX, FILE_PREFIX, VOICE_PREFIX, EPHEMERAL_PREFIX],
        )
        .trim()
        .to_string();
        if !inferred_write_paths.is_empty() {
            text_without_tokens = strip_written_file_confirmation_lines(&text_without_tokens)
                .trim()
                .to_string();
        }
        if !text_without_tokens.is_empty() {
            let sent = send_telegram_text(bot, chat_id, &text_without_tokens)
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
        } else if explicit_file_paths.is_empty()
            && missing_explicit_file_tokens.is_empty()
            && !inferred_write_paths.is_empty()
        {
            if let Some(inline_text) =
                inline_single_small_text_file(&file_paths, &image_paths, &voice_paths)
            {
                let sent = send_telegram_text(bot, chat_id, &inline_text)
                    .await
                    .context("send inline text file body failed")?;
                debug!(
                    "phase=deliver_inline_text_file chat_id={} answer_fp={} telegram_msg_id={} text_preview={}",
                    chat_id.0,
                    text_fingerprint_hex(&inline_text),
                    sent.id.0,
                    text_preview_for_log(&inline_text, 120)
                );
                return Ok(());
            }
        }
        if !missing_explicit_file_tokens.is_empty() {
            warn!(
                "phase=deliver_media_missing_file chat_id={} missing_paths={:?}",
                chat_id.0, missing_explicit_file_tokens
            );
            let missing_text = format!(
                "文件发送失败：找不到以下路径，请先写入文件后再用 FILE: 发送。\n{}",
                missing_explicit_file_tokens.join("\n")
            );
            let _ = send_telegram_text(bot, chat_id, &missing_text).await;
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
        let expires_at_secs = unix_ts().saturating_add(state.crypto_confirm_ttl_seconds);
        let yes_callback = format!("crypto_confirm_yes:{expires_at_secs}");
        let no_callback = format!("crypto_confirm_no:{expires_at_secs}");
        let keyboard = InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback(
                state.i18n.t("telegram.msg.crypto_confirm_button_yes"),
                yes_callback,
            ),
            InlineKeyboardButton::callback(
                state.i18n.t("telegram.msg.crypto_confirm_button_no"),
                no_callback,
            ),
        ]]);
        let sent = send_telegram_text_with_markup(bot, chat_id, answer, keyboard)
            .await
            .context("send text message with confirm keyboard failed")?;
        debug!(
            "phase=deliver_text_confirm chat_id={} answer_fp={} telegram_msg_id={} answer_preview={}",
            chat_id.0,
            text_fingerprint_hex(answer),
            sent.id.0,
            text_preview_for_log(answer, 120)
        );
        let bot_clone = bot.clone();
        let state_clone = state.clone();
        let sent_msg_id = sent.id;
        let confirm_msg_key = (chat_id.0, sent_msg_id.0);
        let original_text = answer.to_string();
        let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
        if let Ok(mut pending) = state.crypto_confirm_expiry_cancels.lock() {
            if let Some(prev) = pending.insert(confirm_msg_key, cancel_tx) {
                let _ = prev.send(());
            }
        }
        tokio::spawn(async move {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(state_clone.crypto_confirm_ttl_seconds)) => {
                    if let Ok(mut pending) = state_clone.crypto_confirm_expiry_cancels.lock() {
                        pending.remove(&confirm_msg_key);
                    }
                    let expired_hint = state_clone.i18n.t("telegram.msg.crypto_confirm_hint_expired");
                    let expired_text = build_expired_trade_text(&original_text, &expired_hint);
                    let _ = bot_clone
                        .edit_message_text(chat_id, sent_msg_id, expired_text)
                        .await;
                    let _ = bot_clone
                        .edit_message_reply_markup(chat_id, sent_msg_id)
                        .reply_markup(InlineKeyboardMarkup::new(Vec::<Vec<InlineKeyboardButton>>::new()))
                        .await;
                }
                _ = &mut cancel_rx => {}
            }
        });
    } else {
        let sent = send_telegram_text(bot, chat_id, answer)
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

fn inline_single_small_text_file(
    file_paths: &[String],
    image_paths: &[String],
    voice_paths: &[String],
) -> Option<String> {
    if !image_paths.is_empty() || !voice_paths.is_empty() || file_paths.len() != 1 {
        return None;
    }
    let path = file_paths.first()?;
    if !is_inline_text_file(path) {
        return None;
    }
    let raw = fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.len() > TELEGRAM_INLINE_TEXT_FILE_MAX_CHARS
        || trimmed.lines().count() > TELEGRAM_INLINE_TEXT_FILE_MAX_LINES
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn is_inline_text_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".txt")
        || lower.ends_with(".md")
        || lower.ends_with(".markdown")
        || lower.ends_with(".json")
        || lower.ends_with(".csv")
        || lower.ends_with(".log")
}

/// Max characters per Telegram message (conservative; platform limit ~4096).
const TELEGRAM_TEXT_CHUNK_CHARS: usize = 3500;
const TELEGRAM_INLINE_TEXT_FILE_MAX_CHARS: usize = 3000;
const TELEGRAM_INLINE_TEXT_FILE_MAX_LINES: usize = 120;

fn telegram_text_payload(text: &str) -> (String, Option<ParseMode>) {
    let trimmed = text.trim();
    if let Some(code_body) = code_or_command_block_body(trimmed) {
        return (
            format!(
                "<pre><code>{}</code></pre>",
                escape_telegram_html(&code_body)
            ),
            Some(ParseMode::Html),
        );
    }
    if let Some(inline_html) = render_inline_code_html(text) {
        return (inline_html, Some(ParseMode::Html));
    }
    (text.to_string(), None)
}

async fn send_telegram_text(bot: &Bot, chat_id: ChatId, text: &str) -> anyhow::Result<Message> {
    let chunks = chunk_text_for_channel(
        text,
        TELEGRAM_TEXT_CHUNK_CHARS.saturating_sub(SEGMENT_PREFIX_MAX_CHARS),
    );
    if chunks.is_empty() {
        return Err(anyhow::anyhow!("empty text"));
    }
    if chunks.len() == 1 {
        let (body, parse_mode) = telegram_text_payload(&chunks[0]);
        let req = bot.send_message(chat_id, body);
        let req = if let Some(mode) = parse_mode {
            req.parse_mode(mode)
        } else {
            req
        };
        return Ok(req.await?);
    }
    let n = chunks.len();
    info!(
        "send_chunks channel=telegram chat_id={:?} original_len={} chunk_count={}",
        chat_id,
        text.len(),
        n
    );
    // Long text: send each chunk as plain text (no HTML/code) with segment hint.
    let mut last = None;
    for (i, chunk) in chunks.into_iter().enumerate() {
        let body = format!("（{}/{}）\n{}", i + 1, n, chunk);
        info!(
            "send_chunk channel=telegram chat_id={:?} index={} total={}",
            chat_id,
            i + 1,
            n
        );
        let msg = bot.send_message(chat_id, body).await?;
        last = Some(msg);
    }
    Ok(last.expect("chunks non-empty"))
}

async fn send_telegram_text_with_markup(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    reply_markup: InlineKeyboardMarkup,
) -> anyhow::Result<Message> {
    let chunks = chunk_text_for_channel(
        text,
        TELEGRAM_TEXT_CHUNK_CHARS.saturating_sub(SEGMENT_PREFIX_MAX_CHARS),
    );
    if chunks.is_empty() {
        return Err(anyhow::anyhow!("empty text"));
    }
    if chunks.len() == 1 {
        let (body, parse_mode) = telegram_text_payload(&chunks[0]);
        let req = bot.send_message(chat_id, body);
        let req = if let Some(mode) = parse_mode {
            req.parse_mode(mode)
        } else {
            req
        };
        return Ok(req.reply_markup(reply_markup).await?);
    }
    let n = chunks.len();
    info!(
        "send_chunks channel=telegram chat_id={:?} original_len={} chunk_count={} with_markup=true",
        chat_id,
        text.len(),
        n
    );
    // Long text: send each chunk as plain text with segment hint; attach markup only to the last message.
    let mut last = None;
    for (i, chunk) in chunks.into_iter().enumerate() {
        let body = format!("（{}/{}）\n{}", i + 1, n, chunk);
        info!(
            "send_chunk channel=telegram chat_id={:?} index={} total={}",
            chat_id,
            i + 1,
            n
        );
        let req = bot.send_message(chat_id, body);
        let req = if i == n - 1 {
            req.reply_markup(reply_markup.clone())
        } else {
            req
        };
        let msg = req.await?;
        last = Some(msg);
    }
    Ok(last.expect("chunks non-empty"))
}

fn escape_telegram_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn render_inline_code_html(text: &str) -> Option<String> {
    if !text.contains('`') || text.contains("```") || has_delivery_prefix(text.trim()) {
        return None;
    }
    let mut out = String::new();
    let mut buf = String::new();
    let mut in_code = false;
    let mut saw_code = false;
    for ch in text.chars() {
        if ch == '`' {
            if in_code {
                if buf.is_empty() {
                    out.push('`');
                } else {
                    out.push_str("<code>");
                    out.push_str(&escape_telegram_html(&buf));
                    out.push_str("</code>");
                    saw_code = true;
                }
                buf.clear();
                in_code = false;
            } else {
                out.push_str(&escape_telegram_html(&buf));
                buf.clear();
                in_code = true;
            }
        } else {
            buf.push(ch);
        }
    }
    if in_code {
        out.push('`');
    }
    out.push_str(&escape_telegram_html(&buf));
    if saw_code {
        Some(out)
    } else {
        None
    }
}

fn code_or_command_block_body(text: &str) -> Option<String> {
    if text.is_empty()
        || text.len() > 3600
        || has_delivery_prefix(text)
        || text.starts_with("<pre>")
    {
        return None;
    }
    if let Some((lang, unfenced)) = strip_markdown_code_fence(text) {
        let trimmed = unfenced.trim();
        if !trimmed.is_empty() {
            if language_is_shell(&lang) || looks_like_shell_command_block(trimmed) {
                return Some(add_shell_prompt_prefix(trimmed));
            }
            return Some(trimmed.to_string());
        }
    }
    if looks_like_shell_command_line(text) {
        return Some(add_shell_prompt_prefix(text.trim()));
    }
    if looks_like_shell_command_block(text) {
        return Some(add_shell_prompt_prefix(text.trim()));
    }
    if looks_like_single_line_code(text) || looks_like_multiline_code(text) {
        return Some(text.trim().to_string());
    }
    None
}

fn has_delivery_prefix(text: &str) -> bool {
    text.starts_with("FILE:")
        || text.starts_with("IMAGE_FILE:")
        || text.starts_with("VOICE_FILE:")
        || text.starts_with("EPHEMERAL:")
}

fn strip_markdown_code_fence(text: &str) -> Option<(String, String)> {
    let trimmed = text.trim();
    if !trimmed.starts_with("```") {
        return None;
    }
    let lines = trimmed.lines().collect::<Vec<_>>();
    if lines.len() < 2 {
        return None;
    }
    let first = lines.first()?.trim_start();
    let last = lines.last()?.trim();
    if !first.starts_with("```") || !last.starts_with("```") {
        return None;
    }
    let lang = first.trim_start_matches("```").trim().to_string();
    Some((lang, lines[1..lines.len().saturating_sub(1)].join("\n")))
}

fn language_is_shell(lang: &str) -> bool {
    matches!(
        lang.trim().to_ascii_lowercase().as_str(),
        "bash" | "sh" | "zsh" | "shell" | "console" | "terminal"
    )
}

fn add_shell_prompt_prefix(text: &str) -> String {
    text.lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                String::new()
            } else if trimmed.starts_with('$') {
                trimmed.to_string()
            } else {
                format!("$ {trimmed}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn looks_like_shell_command_line(text: &str) -> bool {
    if text.is_empty() || text.len() > 320 || text.contains('\n') {
        return false;
    }
    let first = text
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c| matches!(c, '"' | '\'' | '`'));
    if first.is_empty() {
        return false;
    }
    let command_heads = [
        "bash",
        "sh",
        "zsh",
        "python",
        "python3",
        "pip",
        "pip3",
        "uv",
        "node",
        "npm",
        "pnpm",
        "yarn",
        "cargo",
        "rustclaw",
        "openclaw",
        "git",
        "curl",
        "wget",
        "ssh",
        "scp",
        "rsync",
        "ls",
        "pwd",
        "cd",
        "cat",
        "cp",
        "mv",
        "rm",
        "mkdir",
        "chmod",
        "chown",
        "touch",
        "head",
        "tail",
        "grep",
        "rg",
        "sed",
        "awk",
        "find",
        "echo",
        "printf",
        "export",
        "env",
        "source",
        "sudo",
        "systemctl",
        "service",
        "journalctl",
        "docker",
        "docker-compose",
        "kubectl",
        "sqlite3",
        "mysql",
        "psql",
        "ps",
        "pgrep",
        "pkill",
        "kill",
        "killall",
        "uname",
        "df",
        "du",
        "top",
        "htop",
        "free",
        "mount",
        "umount",
        "ip",
        "ifconfig",
        "ss",
        "netstat",
        "lsof",
        "tar",
        "zip",
        "unzip",
        "make",
        "cmake",
        "go",
        "java",
        "javac",
        "perl",
        "ruby",
        "php",
        "lua",
        "deno",
        "npx",
        "brew",
        "apt",
        "apt-get",
        "yum",
        "dnf",
        "pacman",
    ];
    if command_heads
        .iter()
        .any(|cmd| first.eq_ignore_ascii_case(cmd))
    {
        return true;
    }
    first.starts_with("./")
        || first.starts_with("../")
        || first.starts_with("~/")
        || (first.starts_with('/') && first.contains('/'))
}

fn looks_like_shell_command_block(text: &str) -> bool {
    if text.is_empty() || !text.contains('\n') || text.len() > 3600 {
        return false;
    }
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() < 2 {
        return false;
    }
    let shell_like = lines
        .iter()
        .filter(|line| looks_like_shell_command_line(line) || line.starts_with('$'))
        .count();
    shell_like >= 2 && shell_like * 2 >= lines.len()
}

fn looks_like_single_line_code(text: &str) -> bool {
    if text.is_empty() || text.len() > 320 || text.contains('\n') {
        return false;
    }
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    let starters = [
        "fn ",
        "pub fn ",
        "async fn ",
        "let ",
        "const ",
        "var ",
        "val ",
        "def ",
        "class ",
        "import ",
        "from ",
        "export ",
        "#include ",
        "package ",
        "interface ",
        "type ",
        "enum ",
        "impl ",
        "SELECT ",
        "INSERT ",
        "UPDATE ",
        "DELETE ",
        "CREATE ",
        "ALTER ",
        "DROP ",
        "{",
        "[",
        "<?php",
        "#!/usr/bin/env ",
        "#!/bin/bash",
        "#!/bin/sh",
    ];
    if starters
        .iter()
        .any(|s| trimmed.starts_with(s) || lower.starts_with(&s.to_ascii_lowercase()))
    {
        return true;
    }
    (trimmed.contains("=>") && (trimmed.contains('{') || trimmed.contains('(')))
        || (trimmed.contains("::") && (trimmed.contains("fn") || trimmed.contains("impl")))
        || (trimmed.ends_with(';') && (trimmed.contains('=') || trimmed.contains('(')))
        || (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
}

fn looks_like_multiline_code(text: &str) -> bool {
    if text.is_empty() || !text.contains('\n') || text.len() > 3600 {
        return false;
    }
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() < 2 {
        return false;
    }
    let mut score = 0usize;
    for line in &lines {
        let lower = line.to_ascii_lowercase();
        if line.starts_with("#!") {
            score += 2;
        }
        if line.starts_with('$') || line.starts_with("sudo ") || line.starts_with("./") {
            score += 2;
        }
        if looks_like_shell_command_line(line) || looks_like_single_line_code(line) {
            score += 1;
        }
        if line.ends_with('{')
            || line.ends_with('}')
            || line.ends_with(';')
            || line.ends_with(':')
            || line.starts_with("```")
        {
            score += 1;
        }
        if lower.starts_with("if ")
            || lower.starts_with("for ")
            || lower.starts_with("while ")
            || lower.starts_with("return ")
            || lower.starts_with("match ")
            || lower.starts_with("case ")
            || lower.starts_with("try:")
            || lower.starts_with("except")
            || lower.starts_with("finally:")
            || lower.starts_with("with ")
            || lower.starts_with("echo ")
        {
            score += 1;
        }
    }
    score >= 3
}

/// Replace the confirm-hint line at the end of a trade preview message with the expired hint.
fn build_expired_trade_text(original: &str, expired_hint: &str) -> String {
    if let Some(idx) = original.rfind('\n') {
        format!("{}\n{}", &original[..idx], expired_hint)
    } else {
        format!("{}\n{}", original, expired_hint)
    }
}

/// No longer used: confirmation UI for crypto trade_preview has been removed; planner decides flow.
fn is_crypto_trade_confirm_prompt(_text: &str, structured_hint: bool) -> bool {
    if structured_hint {
        return true;
    }
    // Do not attach confirm keyboard for trade_preview; confirmation is planner-decided.
    let decision = false;
    debug!(
        "phase=confirm_detect structured_hint={} decision={} (confirm UI disabled)",
        structured_hint, decision
    );
    decision
}

fn extract_prefixed_paths(answer: &str, prefix: &str) -> Vec<String> {
    let tokens = extract_prefixed_tokens(answer, prefix);
    let (resolved, _) = resolve_delivery_paths(&tokens);
    resolved
}

fn extract_prefixed_tokens(answer: &str, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in answer.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let cleaned = normalize_path_token(rest.trim());
            if !cleaned.is_empty() {
                out.push(cleaned.to_string());
            }
        }
    }
    out
}

fn resolve_delivery_paths(tokens: &[String]) -> (Vec<String>, Vec<String>) {
    let mut found = Vec::new();
    let mut missing = Vec::new();
    for token in tokens {
        if let Some(path) = resolve_delivery_token_path(token) {
            found.push(path);
        } else {
            missing.push(token.clone());
        }
    }
    (dedupe_preserve_order(found), dedupe_preserve_order(missing))
}

fn resolve_delivery_token_path(token: &str) -> Option<String> {
    let cleaned = normalize_path_token(token);
    if cleaned.is_empty() {
        return None;
    }
    let candidate = if Path::new(cleaned).is_absolute() {
        PathBuf::from(cleaned)
    } else {
        let cwd = std::env::current_dir().ok()?;
        cwd.join(cleaned)
    };
    if candidate.is_file() {
        return Some(candidate.to_string_lossy().to_string());
    }
    if Path::new(cleaned).is_file() {
        return Some(cleaned.to_string());
    }
    None
}

fn is_written_file_confirmation_line(line: &str) -> bool {
    let trimmed = line.trim();
    let Some(rest) = trimmed.strip_prefix("written ") else {
        return false;
    };
    let Some((bytes_text, path_text)) = rest.split_once(" bytes to ") else {
        return false;
    };
    if bytes_text.trim().parse::<u64>().is_err() {
        return false;
    }
    let cleaned = normalize_path_token(path_text.trim());
    !cleaned.is_empty() && Path::new(cleaned).is_file()
}

fn extract_written_file_paths(answer: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in answer.lines() {
        if !is_written_file_confirmation_line(line) {
            continue;
        }
        let Some(rest) = line.trim().strip_prefix("written ") else {
            continue;
        };
        let Some((_, path_text)) = rest.split_once(" bytes to ") else {
            continue;
        };
        let cleaned = normalize_path_token(path_text.trim());
        out.push(cleaned.to_string());
    }
    out
}

fn strip_written_file_confirmation_lines(answer: &str) -> String {
    answer
        .lines()
        .filter(|line| !is_written_file_confirmation_line(line))
        .collect::<Vec<_>>()
        .join("\n")
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
        .filter(|line| {
            !prefixes
                .iter()
                .any(|prefix| line.trim_start().starts_with(prefix))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_delivery_tokens_for_tts(answer: &str) -> String {
    strip_prefixed_tokens(
        answer,
        &["IMAGE_FILE:", "FILE:", "VOICE_FILE:", "EPHEMERAL:"],
    )
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
        // 0 表示不发送“任务已运行超过 X 秒”的提示
        let soft_notice_seconds = soft_notice_override_seconds.unwrap_or(state.task_wait_seconds);
        let hard_notice_seconds = state.task_wait_seconds;
        let started_at = tokio::time::Instant::now();
        let mut soft_notice_sent = false;
        let mut hard_notice_sent = false;
        let mut sent_progress_count = 0usize;

        loop {
            match query_task_status(
                &state,
                &task_id,
                bound_user_key_for_chat(&state, chat_id.0).as_deref(),
            )
            .await
            {
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
                            debug!(
                                "phase=skip_progress_delivery task_id={} chat_id={} skipped_count={}",
                                task_id,
                                chat_id.0,
                                progress_messages.len() - sent_progress_count
                            );
                            sent_progress_count = progress_messages.len();
                        }
                        if soft_notice_seconds > 0
                            && !soft_notice_sent
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
                        if hard_notice_seconds > 0
                            && !hard_notice_sent
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
                        let answers = task_success_messages(&state, &task);
                        let resume_followup_decision = task
                            .result_json
                            .as_ref()
                            .and_then(|v| v.get("resume_followup_decision"))
                            .and_then(|v| v.get("decision"))
                            .and_then(|v| v.as_str());
                        let has_structured_messages = task
                            .result_json
                            .as_ref()
                            .and_then(|v| v.get("messages"))
                            .and_then(|v| v.as_array())
                            .map(|arr| !arr.is_empty())
                            .unwrap_or(false);
                        if resume_followup_decision == Some("abandon") {
                            clear_pending_resume_for_chat(&state, chat_id.0);
                        } else if sent_progress_count > 0 || has_structured_messages {
                            clear_pending_resume_for_chat(&state, chat_id.0);
                        }
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
                            let _ = send_success_message_for_telegram(
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
                        if let Some(resume_context) = task
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
                                resume_context,
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

async fn send_success_message_for_telegram(
    bot: &Bot,
    state: &BotState,
    chat_id: ChatId,
    answer: &str,
    requires_confirmation: bool,
) -> anyhow::Result<()> {
    if let Some(blocks) = split_subtask_success_messages(answer) {
        for (header, body) in blocks {
            if body.is_empty() {
                send_telegram_text(bot, chat_id, &header)
                    .await
                    .context("send subtask header failed")?;
                continue;
            }
            if should_send_subtask_body_as_file(&header, &body) {
                let file_path = write_subtask_body_to_temp_file(&header, &body)?;
                let answer_with_file = format!("{header}\nFILE:{file_path}");
                send_text_or_image(bot, state, chat_id, &answer_with_file, false).await?;
                continue;
            }
            let html = format!(
                "{}\n<pre><code>{}</code></pre>",
                escape_telegram_html(&header),
                escape_telegram_html(&body)
            );
            bot.send_message(chat_id, html)
                .parse_mode(ParseMode::Html)
                .await
                .context("send subtask code block failed")?;
        }
        return Ok(());
    }
    send_text_or_image(bot, state, chat_id, answer, requires_confirmation).await
}

fn split_subtask_success_messages(text: &str) -> Option<Vec<(String, String)>> {
    let trimmed = text.trim();
    if !trimmed.starts_with("subtask#") {
        return None;
    }

    let mut raw_blocks = Vec::new();
    let mut current = String::new();

    for line in trimmed.lines() {
        let line = line.trim_end();
        if line.starts_with("subtask#") {
            if !current.trim().is_empty() {
                raw_blocks.push(current.trim().to_string());
                current.clear();
            }
            current.push_str(line);
        } else {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(line);
        }
    }

    if !current.trim().is_empty() {
        raw_blocks.push(current.trim().to_string());
    }

    let blocks = raw_blocks
        .into_iter()
        .map(|block| split_single_subtask_block(&block))
        .collect::<Vec<_>>();
    Some(blocks)
}

fn split_single_subtask_block(block: &str) -> (String, String) {
    let trimmed = block.trim();
    let (first_line, rest) = match trimmed.split_once('\n') {
        Some((head, tail)) => (head.trim(), tail.trim()),
        None => (trimmed, ""),
    };

    if let Some((header, inline_body)) = first_line.split_once(" | ") {
        let mut body = inline_body.trim().to_string();
        if !rest.is_empty() {
            if !body.is_empty() {
                body.push('\n');
            }
            body.push_str(rest);
        }
        return (header.trim().to_string(), body);
    }

    (first_line.to_string(), rest.to_string())
}

fn should_send_subtask_body_as_file(header: &str, body: &str) -> bool {
    let html_len = escape_telegram_html(header).len() + escape_telegram_html(body).len() + 32;
    html_len > 3000 || body.lines().count() > 120
}

fn write_subtask_body_to_temp_file(header: &str, body: &str) -> anyhow::Result<String> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let sanitized = sanitize_filename_fragment(header);
    let path = std::env::temp_dir().join(format!("rustclaw-{sanitized}-{millis}.txt"));
    fs::write(&path, body).with_context(|| format!("write subtask temp file failed: {}", path.display()))?;
    Ok(path.to_string_lossy().to_string())
}

fn sanitize_filename_fragment(text: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in text.chars() {
        let keep = ch.is_ascii_alphanumeric();
        if keep {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "subtask".to_string()
    } else {
        trimmed
    }
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
        let mut out = dedupe_preserve_order(out);
        if !out.is_empty() {
            let has_explicit_delivery = out.iter().any(|msg| has_delivery_prefix(msg));
            if has_explicit_delivery {
                out.retain(|msg| !is_written_file_confirmation_line(msg));
            }
            debug!(
                "phase=success_source task_id={} source=messages offset={} messages_len={} explicit_delivery={}",
                task_id,
                offset,
                out.len(),
                has_explicit_delivery
            );
            if offset >= out.len() {
                // Progress delivery already consumed all message items.
                // Do not fallback to result_json.text here, otherwise the
                // last item is sent again (duplicate delivery).
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
    let text = wrap_single_step_skill_message(task, &text).unwrap_or(text);
    debug!(
        "phase=success_source task_id={} source=text_only offset={} text_fp={} text_len={}",
        task_id,
        offset,
        text_fingerprint_hex(&text),
        text.len()
    );
    vec![text]
}

fn wrap_single_step_skill_message(task: &TaskQueryResponse, text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("subtask#")
        || contains_delivery_tokens(trimmed)
        || trimmed.starts_with("<pre>")
    {
        return None;
    }
    let meta = task.result_json.as_ref()?.get("delivery_meta")?;
    if meta.get("mode").and_then(|v| v.as_str()) != Some("single_step_skill") {
        return None;
    }
    let label = meta
        .get("label")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("step");
    let skill_name = meta
        .get("skill_name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let header = if let Some(skill_name) = skill_name {
        format!("{label} · skill({skill_name})")
    } else {
        label.to_string()
    };
    Some(format!("subtask#1 {header}: success\n{trimmed}"))
}

fn contains_delivery_tokens(text: &str) -> bool {
    text.lines().map(str::trim).any(has_delivery_prefix)
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

async fn query_task_status(
    state: &BotState,
    task_id: &str,
    user_key: Option<&str>,
) -> anyhow::Result<TaskQueryResponse> {
    let url = format!("{}/v1/tasks/{task_id}", state.clawd_base_url);
    let mut req = state.client.get(&url);
    if let Some(user_key) = user_key.map(str::trim).filter(|v| !v.is_empty()) {
        req = req.header("X-RustClaw-Key", user_key);
    }
    let resp = req.send().await.context("query task status failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let msg = if (body.contains("<!doctype") || body.contains("<html")) && body.len() > 100 {
            state.i18n.t("telegram.error.query_task_wrong_host")
        } else {
            let body_preview = if body.len() > 300 {
                format!("{}...", &body[..300])
            } else {
                body.clone()
            };
            state.i18n.t_with(
                "telegram.error.query_task_failed_http",
                &[("status", &status.to_string()), ("body", &body_preview)],
            )
        };
        return Err(anyhow!("{}", msg));
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
                    &body
                        .error
                        .unwrap_or_else(|| state.i18n.t("common.unknown_error"))
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
    mut payload: serde_json::Value,
) -> anyhow::Result<String> {
    let user_key = state
        .bound_identity_by_chat
        .lock()
        .ok()
        .and_then(|map| map.get(&chat_id).map(|identity| identity.user_key.clone()));
    if let Some(obj) = payload.as_object_mut() {
        obj.insert(
            "telegram_bot_name".to_string(),
            JsonValue::String(state.bot_name.clone()),
        );
        obj.insert("agent_id".to_string(), JsonValue::String(state.agent_id.clone()));
    }
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
        user_id: Some(user_id),
        chat_id: Some(chat_id),
        user_key,
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
        .ok_or_else(|| {
            anyhow!(
                "{}",
                state.i18n.t("telegram.error.submit_task_missing_task_id")
            )
        })?
        .task_id;

    debug!(
        "phase=submit_done user_id={} chat_id={} kind={:?} task_id={} payload_fp={}",
        user_id, chat_id, kind, task_id, payload_fp
    );
    Ok(task_id.to_string())
}

async fn poll_task_result(
    state: &BotState,
    task_id: &str,
    user_key: Option<&str>,
    wait_override_seconds: Option<u64>,
) -> anyhow::Result<Vec<String>> {
    let poll_interval_ms = state.poll_interval_ms.max(1);
    let wait_seconds = wait_override_seconds
        .unwrap_or(state.task_wait_seconds)
        .max(1);
    let max_rounds = ((wait_seconds * 1000) / poll_interval_ms).max(1);

    for _ in 0..max_rounds {
        let task = query_task_status(state, task_id, user_key).await?;
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
        return Err(anyhow!("cancel http {status}: {body}",));
    }

    let body: ApiResponse<JsonValue> =
        resp.json().await.context("decode cancel response failed")?;

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

    let body: ApiResponse<HealthResponse> =
        resp.json().await.context("decode health response failed")?;

    if !body.ok {
        return Err(anyhow!(
            "{}",
            state.i18n.t_with(
                "telegram.error.health_failed",
                &[(
                    "error",
                    &body
                        .error
                        .unwrap_or_else(|| state.i18n.t("common.unknown_error"))
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
    let body: ApiResponse<HealthResponse> =
        resp.json().await.context("decode health response failed")?;
    if !body.ok {
        return Err(anyhow!(
            "{}",
            state.i18n.t_with(
                "telegram.error.health_failed",
                &[(
                    "error",
                    &body
                        .error
                        .unwrap_or_else(|| state.i18n.t("common.unknown_error"))
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

    let output = toml::to_string_pretty(&value)
        .context(state.i18n.t("telegram.error.serialize_config_failed"))?;
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
                return Err(anyhow!(
                    "{}",
                    state.i18n.t("telegram.msg.openclaw_set_usage")
                ));
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
    let upper = raw.trim().to_ascii_uppercase().replace(['-', '/', ' '], "");
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

async fn handle_cryptoapi_command(
    state: &BotState,
    identity: &AuthIdentity,
    raw: &str,
) -> anyhow::Result<String> {
    let cmd = raw.trim();
    if cmd.is_empty() {
        return Ok(cryptoapi_usage_text(state));
    }
    let mut parts = cmd.split_whitespace();
    let action = parts.next().unwrap_or_default().to_ascii_lowercase();
    match action.as_str() {
        "show" => show_cryptoapi_status(state, identity).await,
        "add" => {
            if identity.role != "admin" {
                return Ok(state.i18n.t("telegram.msg.cryptoapi_admin_only"));
            }
            let target = parts.next().unwrap_or_default().to_ascii_lowercase();
            match target.as_str() {
                "allowed_symbols" | "allowedsymbols" | "symbols" => {
                    let symbols_raw = parts.collect::<Vec<_>>().join(" ");
                    let parsed = parse_symbols_csv(&symbols_raw)
                        .into_iter()
                        .filter_map(|s| normalize_trade_symbol_for_config(&s))
                        .collect::<Vec<_>>();
                    if parsed.is_empty() {
                        return Ok(state
                            .i18n
                            .t("telegram.msg.cryptoapi_add_allowed_symbols_invalid"));
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
                    let status = upsert_crypto_credential(
                        state, identity, "binance", api_key, api_secret, None,
                    )
                    .await?;
                    let api_key_masked = status
                        .api_key_masked
                        .unwrap_or_else(|| mask_secret(api_key));
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
                    let status = upsert_crypto_credential(
                        state,
                        identity,
                        "okx",
                        api_key,
                        api_secret,
                        Some(passphrase),
                    )
                    .await?;
                    let api_key_masked = status
                        .api_key_masked
                        .unwrap_or_else(|| mask_secret(api_key));
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

async fn show_cryptoapi_status(
    state: &BotState,
    identity: &AuthIdentity,
) -> anyhow::Result<String> {
    let path = "configs/crypto.toml";
    let raw = fs::read_to_string(path).context("failed to read configs/crypto.toml")?;
    let value: TomlValue = toml::from_str(&raw).context("failed to parse configs/crypto.toml")?;
    let crypto = value.get("crypto").and_then(|v| v.as_table());
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
    let statuses = fetch_crypto_credential_status(state, identity).await?;
    let binance = statuses.iter().find(|v| v.exchange == "binance");
    let okx = statuses.iter().find(|v| v.exchange == "okx");
    let binance_enabled_text = binance.is_some_and(|v| v.configured).to_string();
    let okx_enabled_text = okx.is_some_and(|v| v.configured).to_string();
    let binance_key_masked = binance
        .and_then(|v| v.api_key_masked.clone())
        .unwrap_or_else(|| "-".to_string());
    let okx_key_masked = okx
        .and_then(|v| v.api_key_masked.clone())
        .unwrap_or_else(|| "-".to_string());
    let allowed_symbols_text = if allowed_symbols.is_empty() {
        "-".to_string()
    } else {
        allowed_symbols.join(", ")
    };
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

fn persist_crypto_allowed_symbols_add(symbols: &[String]) -> anyhow::Result<Vec<String>> {
    let path = "configs/crypto.toml";
    let raw = fs::read_to_string(path).context("failed to read configs/crypto.toml")?;
    let mut value: TomlValue =
        toml::from_str(&raw).context("failed to parse configs/crypto.toml")?;
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
        TomlValue::Array(
            merged
                .iter()
                .map(|s| TomlValue::String(s.clone()))
                .collect(),
        ),
    );
    let output =
        toml::to_string_pretty(&value).context("failed to serialize configs/crypto.toml")?;
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

    let vendors = [
        "openai",
        "google",
        "anthropic",
        "grok",
        "qwen",
        "minimax",
        "custom",
    ];
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
            lines.push(state.i18n.t_with(
                "telegram.msg.openclaw_vendor_line",
                &[("vendor", vendor), ("model", model), ("models", &models)],
            ));
        }
    }

    lines.push("".to_string());
    lines.push(state.i18n.t("telegram.msg.openclaw_restart_hint"));
    Ok(lines.join("\n"))
}

fn is_supported_model_vendor(vendor: &str) -> bool {
    matches!(
        vendor,
        "openai" | "google" | "anthropic" | "grok" | "qwen" | "minimax" | "custom"
    )
}

fn default_base_url_for_vendor(vendor: &str) -> &'static str {
    match vendor {
        "openai" => "https://api.openai.com/v1",
        "google" => "https://generativelanguage.googleapis.com/v1beta",
        "anthropic" => "https://api.anthropic.com/v1",
        "grok" => "https://api.x.ai/v1",
        "qwen" => "https://dashscope.aliyuncs.com/compatible-mode/v1",
        "minimax" => "https://api.minimax.io/v1",
        "custom" => "https://api.example.com/v1",
        _ => "https://api.example.com/v1",
    }
}

fn apply_model_config_value(
    value: &mut TomlValue,
    vendor: &str,
    model: &str,
) -> anyhow::Result<()> {
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
    llm_tbl.insert(
        "selected_vendor".to_string(),
        TomlValue::String(vendor.to_string()),
    );
    llm_tbl.insert(
        "selected_model".to_string(),
        TomlValue::String(model.to_string()),
    );

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
            TomlValue::String(format!(
                "REPLACE_ME_{}_API_KEY",
                vendor.to_ascii_uppercase()
            )),
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

    Ok(())
}

fn set_model_config(state: &BotState, vendor: &str, model: &str) -> anyhow::Result<String> {
    if !is_supported_model_vendor(vendor) {
        return Err(anyhow!(
            "{}",
            state.i18n.t_with(
                "telegram.msg.openclaw_unsupported_vendor",
                &[("vendor", vendor)]
            )
        ));
    }

    let raw = fs::read_to_string("configs/config.toml")
        .context(state.i18n.t("telegram.error.read_config_failed"))?;
    let mut value: TomlValue =
        toml::from_str(&raw).context(state.i18n.t("telegram.error.parse_config_failed"))?;
    apply_model_config_value(&mut value, vendor, model)
        .context(state.i18n.t("telegram.error.config_not_table"))?;

    let output = toml::to_string_pretty(&value)
        .context(state.i18n.t("telegram.error.serialize_config_failed"))?;
    fs::write("configs/config.toml", output)
        .context(state.i18n.t("telegram.error.write_config_failed"))?;

    Ok(state.i18n.t_with(
        "telegram.msg.openclaw_set_ok",
        &[("vendor", vendor), ("model", model)],
    ))
}

#[cfg(test)]
mod model_config_tests {
    use super::*;

    #[test]
    fn apply_model_config_populates_custom_vendor_only_in_llm_section() {
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
        let custom = llm
            .get("custom")
            .and_then(|x| x.as_table())
            .expect("custom");
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
        assert!(v.get("audio_synthesize").is_none());
        assert!(v.get("audio_transcribe").is_none());
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

    #[test]
    fn apply_model_config_minimax_uses_expected_default_base_url() {
        let mut v: TomlValue = toml::from_str("[llm]\n").expect("parse");
        apply_model_config_value(&mut v, "minimax", "MiniMax-M2.5").expect("apply");
        let minimax = v
            .get("llm")
            .and_then(|x| x.get("minimax"))
            .and_then(|x| x.as_table())
            .expect("minimax");
        assert_eq!(
            minimax.get("base_url").and_then(|x| x.as_str()),
            Some("https://api.minimax.io/v1")
        );
    }
}
