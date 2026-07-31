mod binding_voice;
mod commands;
mod media_handlers;
mod message_handler;
mod task_delivery;
mod telegram_buttons;
mod telegram_formatting;

use binding_voice::*;
use commands::*;
use media_handlers::*;
use message_handler::*;
use task_delivery::*;
use telegram_formatting::*;

use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context};
use claw_core::channel_chunk::{chunk_text_for_channel, SEGMENT_PREFIX_MAX_CHARS};
use claw_core::channel_commands::{ChannelCommandCatalog, CoreCommandAction};
use claw_core::config::{AppConfig, ResolvedTelegramBotConfig};
use claw_core::types::{
    ApiResponse, AuthIdentity, BindChannelKeyRequest, ChannelKind, GatewayInstanceRuntimeStatus,
    HealthResponse, ResolveChannelBindingRequest, ResolveChannelBindingResponse, SubmitTaskRequest,
    SubmitTaskResponse, TaskKind, TaskQueryResponse, TaskStatus, TelegramBotRuntimeStatus,
};
use reqwest::Client;
use serde_json::{json, Value as JsonValue};
use teloxide::prelude::*;
use teloxide::types::{CallbackQuery, ChatAction, InputFile, MediaKind, MessageKind, ParseMode};
use tokio::sync::oneshot;
use toml::Value as TomlValue;
use tracing::{debug, info, warn};

use crate::telegram_buttons::{
    build_url_button_markup, extract_url_buttons_from_text, UrlButtonSpec,
};

#[derive(Clone)]
struct BotState {
    bot_name: String,
    allowlist: Arc<HashSet<i64>>,
    access_mode: String,
    allowed_usernames: Arc<HashSet<String>>,
    clawd_base_url: String,
    workspace_root: PathBuf,
    client: Client,
    poll_interval_ms: u64,
    task_wait_seconds: u64,
    language: String,
    bot_token: String,
    image_inbox_dir: String,
    video_inbox_dir: String,
    file_inbox_dir: String,
    audio_inbox_dir: String,
    voice_reply_mode: String,
    voice_reply_mode_by_chat: Arc<Mutex<HashMap<i64, String>>>,
    max_audio_input_bytes: usize,
    ephemeral_image_saved_seconds: u64,
    pending_resume_by_chat: Arc<Mutex<HashMap<i64, PendingResumeContext>>>,
    pending_key_bind_by_chat: Arc<Mutex<HashSet<i64>>>,
    bound_identity_by_chat: Arc<Mutex<HashMap<i64, AuthIdentity>>>,
    command_catalog: Arc<ChannelCommandCatalog>,
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

const RESUME_CONTEXT_TTL_SECONDS: u64 = 30 * 60;
const TELEGRAM_API_RETRY_BASE_SECONDS: u64 = 5;
const TELEGRAM_API_RETRY_MAX_SECONDS: u64 = 60;

fn unix_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
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
    let workspace_root = std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let mut telegram_runtime_bots = config.telegram_runtime_bots();
    if telegram_runtime_bots.is_empty() {
        return Err(anyhow!("no telegram bot configured"));
    }
    if telegram_runtime_bots.len() > 1 {
        warn!(
            "single-bot mode enabled: only the first configured bot will run; total_configured={}",
            telegram_runtime_bots.len()
        );
    }
    info!(
        "telegram runtimes configured: count={} names={:?}",
        telegram_runtime_bots.len(),
        telegram_runtime_bots
            .iter()
            .map(|bot| bot.name.clone())
            .collect::<Vec<_>>()
    );

    let selected_bot = telegram_runtime_bots.remove(0);
    let state = build_bot_state(&config, &selected_bot, client, &workspace_root);
    run_telegram_bot_runtime(state).await
}

fn build_bot_state(
    config: &AppConfig,
    bot_config: &ResolvedTelegramBotConfig,
    client: Client,
    workspace_root: &Path,
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

    let mut allowed_usernames = HashSet::new();
    for username in &bot_config.allowed_usernames {
        if let Some(normalized) = normalize_telegram_username(username) {
            allowed_usernames.insert(normalized);
        }
    }

    let command_catalog = Arc::new(ChannelCommandCatalog::load_or_default(
        &workspace_root.join("configs/channel_commands.toml"),
    ));

    BotState {
        bot_name: bot_config.name.clone(),
        allowlist: Arc::new(allowlist),
        access_mode: match bot_config.access_mode.trim().to_ascii_lowercase().as_str() {
            "specified" => "specified".to_string(),
            _ => "public".to_string(),
        },
        allowed_usernames: Arc::new(allowed_usernames),
        clawd_base_url: clawd_base_url_from_config(config),
        workspace_root: workspace_root.to_path_buf(),
        client,
        poll_interval_ms: config.worker.poll_interval_ms,
        task_wait_seconds: bot_config.task_delivery_timeout_seconds.max(1),
        language: bot_config.language.clone(),
        bot_token: bot_config.bot_token.clone(),
        image_inbox_dir: config.telegram.image_inbox_dir.clone(),
        video_inbox_dir: config.telegram.video_inbox_dir.clone(),
        file_inbox_dir: config.telegram.file_inbox_dir.clone(),
        audio_inbox_dir: config.telegram.audio_inbox_dir.clone(),
        voice_reply_mode: config.telegram.voice_reply_mode.clone(),
        voice_reply_mode_by_chat: Arc::new(Mutex::new(load_voice_reply_mode_by_chat(config))),
        max_audio_input_bytes: config.telegram.max_audio_input_bytes.max(1024),
        ephemeral_image_saved_seconds: config.telegram.ephemeral_image_saved_seconds,
        pending_resume_by_chat: Arc::new(Mutex::new(HashMap::new())),
        pending_key_bind_by_chat: Arc::new(Mutex::new(HashSet::new())),
        bound_identity_by_chat: Arc::new(Mutex::new(HashMap::new())),
        command_catalog,
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
    config
        .server
        .clawd_base_url
        .clone()
        .unwrap_or_else(|| claw_core::config::CLAWD_INTERNAL_BASE_URL.to_string())
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
        .join(format!(
            "{}.json",
            if safe_name.is_empty() {
                "bot"
            } else {
                &safe_name
            }
        ))
}

fn gateway_instance_status_file_path(workspace_root: &Path, kind: &str, name: &str) -> PathBuf {
    let safe_kind = sanitize_status_name(kind);
    let safe_name = sanitize_status_name(name);
    workspace_root
        .join("run")
        .join("gateway-instance-status")
        .join(format!(
            "{}__{}.json",
            if safe_kind.is_empty() {
                "instance"
            } else {
                &safe_kind
            },
            if safe_name.is_empty() {
                "primary"
            } else {
                &safe_name
            }
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

fn safe_telegram_storage_segment(raw: &str, fallback: &str) -> String {
    let sanitized = sanitize_status_name(raw);
    if sanitized.trim_matches('_').is_empty() {
        fallback.to_string()
    } else {
        sanitized
    }
}

fn build_telegram_inbox_rel_path(
    root_dir: &str,
    bot_name: &str,
    chat_id: i64,
    user_id: i64,
    ts: u64,
    ext: &str,
) -> String {
    let base = root_dir.trim().trim_end_matches('/');
    let safe_bot = safe_telegram_storage_segment(bot_name, "bot");
    let safe_ext = ext.trim().trim_start_matches('.');
    if base.is_empty() {
        format!("{safe_bot}/{chat_id}/{user_id}/{ts}.{safe_ext}")
    } else {
        format!("{base}/{safe_bot}/{chat_id}/{user_id}/{ts}.{safe_ext}")
    }
}

async fn write_bot_runtime_status(path: &Path, status: &TelegramBotRuntimeStatus) {
    let Some(parent) = path.parent() else {
        return;
    };
    if let Err(err) = tokio::fs::create_dir_all(parent).await {
        warn!(
            "create telegram bot status dir failed: path={} err={}",
            parent.display(),
            err
        );
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
    write_runtime_statuses(&state, false, "starting", Some(unix_ts() as i64), None).await;
    let mut startup_error: Option<String> = None;
    if let Err(err) = register_telegram_commands_and_menu(
        &state.bot_token,
        state.i18n.as_ref(),
        state.command_catalog.as_ref(),
    )
    .await
    {
        warn!(
            "register Telegram menu failed: bot_name={} err={}",
            state.bot_name, err
        );
        startup_error = Some(err.to_string());
    } else {
        info!(
            "registered Telegram menu commands: bot_name={}",
            state.bot_name
        );
    }

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
                ("allowlist", &format!("{allowlist_list:?}")),
                ("access_mode", &state.access_mode),
                ("allowed_usernames", &format!("{allowed_usernames_list:?}")),
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
    wait_for_telegram_api(&bot, &state).await;
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

fn telegram_api_retry_delay(attempt: u32) -> Duration {
    let multiplier = 1_u64.checked_shl(attempt.min(6)).unwrap_or(u64::MAX);
    Duration::from_secs(
        TELEGRAM_API_RETRY_BASE_SECONDS
            .saturating_mul(multiplier)
            .min(TELEGRAM_API_RETRY_MAX_SECONDS),
    )
}

async fn wait_for_telegram_api(bot: &Bot, state: &BotState) {
    let mut attempt = 0_u32;
    loop {
        match bot.get_me().send().await {
            Ok(_) => {
                if attempt > 0 {
                    info!(
                        bot_name = %state.bot_name,
                        attempts = attempt,
                        "telegram API connectivity restored"
                    );
                }
                return;
            }
            Err(_) => {
                attempt = attempt.saturating_add(1);
                let delay = telegram_api_retry_delay(attempt - 1);
                write_runtime_statuses(
                    state,
                    false,
                    "waiting_provider",
                    Some(unix_ts() as i64),
                    Some("telegram_api_unavailable".to_string()),
                )
                .await;
                warn!(
                    error_code = "telegram_api_unavailable",
                    bot_name = %state.bot_name,
                    attempt,
                    retry_seconds = delay.as_secs(),
                    "telegram API unavailable; retrying"
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
}

async fn register_telegram_commands_and_menu(
    bot_token: &str,
    i18n: &TextCatalog,
    command_catalog: &ChannelCommandCatalog,
) -> anyhow::Result<()> {
    let api_base = format!("https://api.telegram.org/bot{bot_token}");
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("build telegram menu client failed")?;

    let commands = command_catalog
        .menu_commands_for_channel("telegram")
        .into_iter()
        .map(|command| {
            json!({
                "command": command.name,
                "description": command
                    .description_key()
                    .map(|key| i18n.t(key))
                    .unwrap_or_else(|| command.name.clone()),
            })
        })
        .collect::<Vec<_>>();
    let commands_payload = json!({ "commands": commands });

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

#[cfg(target_os = "linux")]
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

#[cfg(not(target_os = "linux"))]
fn current_rss_bytes() -> Option<u64> {
    let pid = std::process::id().to_string();
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let rss_kb = String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(rss_kb.saturating_mul(1024))
}

#[cfg(test)]
#[path = "main_bind_gate_tests.rs"]
mod bind_gate_tests;
#[cfg(test)]
#[path = "main_runtime_tests.rs"]
mod runtime_tests;
#[cfg(test)]
#[path = "main_telegram_text_payload_tests.rs"]
mod telegram_text_payload_tests;
