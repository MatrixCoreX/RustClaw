use std::collections::HashSet;
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, anyhow};
use claw_core::config::AppConfig;
use claw_core::types::{
    ApiResponse, SubmitTaskRequest, SubmitTaskResponse, HealthResponse, TaskKind, TaskQueryResponse, TaskStatus,
};
use reqwest::Client;
use serde_json::{Value as JsonValue, json};
use teloxide::prelude::*;
use teloxide::types::InputFile;
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
    quick_result_wait_seconds: u64,
    auto_vision_on_image_only: bool,
    pending_image_by_chat: Arc<Mutex<HashMap<i64, String>>>,
    bot_token: String,
    image_inbox_dir: String,
    audio_inbox_dir: String,
    voice_reply_mode: String,
    voice_reply_mode_by_chat: Arc<Mutex<HashMap<i64, String>>>,
    max_audio_input_bytes: usize,
    sendfile_admin_only: bool,
    sendfile_full_access: bool,
    sendfile_allowed_dirs: Arc<Vec<String>>,
    voice_chat_prompt_template: String,
    i18n: Arc<TextCatalog>,
}

#[derive(Debug, Clone)]
struct TextCatalog {
    current: HashMap<&'static str, &'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VoiceReplyMode {
    Voice,
    Text,
    Both,
}

const DEFAULT_VOICE_CHAT_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/voice_chat_prompt.md");

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
    fn zh_cn() -> Self {
        Self {
            current: HashMap::from([
                ("common.unknown_error", "Unknown error"), // zh: 未知错误
                ("telegram.log.started", "telegramd started, admins={admins}, allowlist={allowlist}, skills={skills}, quick_result_wait_seconds={quick_result_wait_seconds}"), // zh: telegramd 已启动, admins={admins}, allowlist={allowlist}, skills={skills}, quick_result_wait_seconds={quick_result_wait_seconds}
                ("telegram.log.startup_memory_rss", "Startup memory RSS bytes={bytes}"), // zh: 启动内存 RSS bytes={bytes}
                ("telegram.log.unauthorized_user", "Unauthorized Telegram user: {user_id}"), // zh: 未授权 Telegram 用户: {user_id}
                ("telegram.msg.unauthorized", "Unauthorized: contact admin to be added to allowlist."), // zh: 未授权：请联系管理员加入 allowlist。
                ("telegram.msg.start", "RustClaw is running.\nYou can ask directly in chat or use skill commands.\nAvailable commands: /start /help /agent on|off /status /cancel /skills /run <skill> <args> /sendfile <path> /voicemode show|voice|text|both|reset /openclaw config ..."), // zh: RustClaw 已启动。\n支持直接聊天提问，也支持技能命令。\n可用命令：/start /help /agent on|off /status /cancel /skills /run <skill> <args> /sendfile <path> /voicemode show|voice|text|both|reset /openclaw config ...
                ("telegram.msg.help", "Send messages directly to ask.\n/agent on|off toggles agent tool mode\n/status shows queue and runtime status\n/cancel stops queued/running tasks for this chat\n/skills lists skills\n/run <skill> <args> runs a skill\n/sendfile <path> sends a local file to this chat\n/voicemode show|voice|text|both|reset (admin)\n/openclaw config show|vendors|set <vendor> <model>"), // zh: 直接发消息即可提问。\n/agent on|off 开关代理工具模式\n/status 查看队列与运行状态\n/cancel 结束当前聊天的排队/执行任务\n/skills 查看技能\n/run <skill> <args> 运行技能\n/sendfile <path> 发送本地文件到当前聊天\n/voicemode show|voice|text|both|reset（管理员）\n/openclaw config show|vendors|set <vendor> <model>
                ("telegram.msg.openclaw_admin_only", "Only admins can use /openclaw config commands."), // zh: 仅管理员可使用 /openclaw 配置命令。
                ("telegram.msg.voicemode_admin_only", "Only admins can use /voicemode."), // zh: 仅管理员可使用 /voicemode。
                ("telegram.msg.voicemode_usage", "Usage: /voicemode show|voice|text|both|reset"), // zh: 用法：/voicemode show|voice|text|both|reset
                ("telegram.msg.voicemode_show", "Current chat voice mode: {chat_mode}\nGlobal default: {global_mode}"), // zh: 当前聊天语音模式：{chat_mode}\n全局默认：{global_mode}
                ("telegram.msg.voicemode_set_ok", "Voice mode updated for this chat: {mode}"), // zh: 当前聊天语音模式已更新：{mode}
                ("telegram.msg.voicemode_reset_ok", "Voice mode reset for this chat. Fallback to global: {global_mode}"), // zh: 当前聊天语音模式已重置，回退到全局：{global_mode}
                ("telegram.msg.config_failed", "Config update failed: {error}"), // zh: 配置失败：{error}
                ("telegram.msg.agent_on", "Agent mode is ON (default): normal messages may use tools/skills."), // zh: Agent 模式已开启（默认）：普通消息会允许工具/技能决策执行。
                ("telegram.msg.agent_off", "Agent mode is OFF: normal messages use plain Q&A only."), // zh: Agent 模式已关闭：普通消息仅走普通问答。
                ("telegram.msg.agent_usage_status", "Usage: /agent on|off\nCurrent status: {status}"), // zh: 用法：/agent on|off\n当前状态：{status}
                ("telegram.msg.read_status_failed", "Read status failed: {error}"), // zh: 读取状态失败：{error}
                ("telegram.msg.no_skills", "No available skills"), // zh: 暂无可用技能
                ("telegram.msg.skills_list", "Available skills: {skills}"), // zh: 可用技能：{skills}
                ("telegram.msg.run_usage", "Usage: /run <skill_name> <args>"), // zh: 用法：/run <skill_name> <args>
                ("telegram.msg.queue_full", "Too many queued tasks ({queued}/{limit}). Please try again later."), // zh: 当前任务过多（排队 {queued} / 上限 {limit}），请稍后再试。
                ("telegram.msg.skill_exec_failed", "Skill execution failed"), // zh: 技能执行失败
                ("telegram.msg.skill_exec_failed_with_error", "Skill execution failed: {error}"), // zh: 技能执行失败：{error}
                ("telegram.msg.accepted_processing", "Received, processing... (queued: {queued})"), // legacy, no longer used
                ("telegram.msg.sendfile_usage", "Usage: /sendfile <local_file_path>"), // zh: 用法：/sendfile <本地文件路径>
                ("telegram.msg.file_not_found", "File not found: {path}"), // zh: 文件不存在：{path}
                ("telegram.msg.not_a_file", "Not a file: {path}"), // zh: 不是文件：{path}
                ("telegram.msg.unknown_command", "Unknown command. Send a message directly or use /help."), // zh: 未知命令。直接发消息即可提问，或使用 /help。
                ("telegram.msg.empty_prompt", "Please provide a prompt."), // zh: 请输入问题内容。
                ("telegram.msg.image_saved_path", "Image saved: {path}"), // zh: 图片已保存：{path}
                ("telegram.msg.image_received_wait_prompt", "Image received. Please send your edit prompt."), // zh: 已收到图片，请发送编辑提示词。
                ("telegram.msg.process_failed", "Processing failed"), // zh: 处理失败
                ("telegram.msg.process_failed_with_error", "Processing failed: {error}"), // zh: 处理失败：{error}
                ("telegram.msg.wait_task_timeout", "Waiting for task result timed out"), // zh: 等待任务结果超时
                ("telegram.msg.task_still_running_background", "Task is still running after {seconds}s. It will continue in background and I will send the result when finished."), // zh: 任务已运行超过 {seconds} 秒，将继续在后台执行，完成后自动发送结果。
                ("telegram.msg.task_done_no_text", "Task finished, but there is no displayable text result."), // zh: 任务完成，但没有可显示的文本结果。
                ("telegram.msg.no_error_text", "No error details"), // zh: 无错误详情
                ("telegram.msg.status_text", "State: {worker_state}\nQueue length: {queue_length}\nRunning: {running_length}\nOldest running age: {running_oldest_age_seconds}s\nTask timeout: {task_timeout_seconds}s\nUptime: {uptime_seconds}s\nVersion: {version}"), // zh: 状态: {worker_state}\n队列长度: {queue_length}\n运行中: {running_length}\n最久运行时长: {running_oldest_age_seconds}s\n任务超时: {task_timeout_seconds}s\n运行时长: {uptime_seconds}s\n版本: {version}
                ("telegram.msg.openclaw_set_usage", "Usage: /openclaw config set <vendor> <model>"), // zh: 用法：/openclaw config set <vendor> <model>
                ("telegram.msg.openclaw_usage", "OpenClaw config commands:\n/openclaw config show\n/openclaw config vendors\n/openclaw config set <vendor> <model>\nExample: /openclaw config set openai gpt-4o-mini\nExample: /openclaw config set google gemini-2.5-pro\nExample: /openclaw config set anthropic claude-3-7-sonnet-latest\nExample: /openclaw config set grok grok-2-latest"), // zh: OpenClaw 配置命令：\n/openclaw config show\n/openclaw config vendors\n/openclaw config set <vendor> <model>\n示例：/openclaw config set openai gpt-4o-mini\n示例：/openclaw config set google gemini-2.5-pro\n示例：/openclaw config set anthropic claude-3-7-sonnet-latest
                ("telegram.msg.openclaw_supported_vendors", "Supported model vendors:\n- openai\n- google\n- anthropic\n- grok"), // zh: 支持的模型厂商(vendor)：\n- openai\n- google\n- anthropic
                ("telegram.msg.openclaw_current_selection", "Current selection: vendor={vendor}, model={model}"), // zh: 当前选择：vendor={vendor}, model={model}
                ("telegram.msg.openclaw_preset_vendors", "Preset vendor configs:"), // zh: 预设厂商配置：
                ("telegram.msg.openclaw_vendor_line", "- {vendor}: model={model}; models=[{models}]"), // zh: - {vendor}: model={model}; models=[{models}]
                ("telegram.msg.openclaw_restart_hint", "Restart clawd for changes to take effect."), // zh: 修改后请重启 clawd 生效。
                ("telegram.msg.openclaw_unsupported_vendor", "Unsupported vendor: {vendor}"), // zh: 不支持的 vendor: {vendor}
                ("telegram.msg.openclaw_set_ok", "Primary model updated: vendor={vendor}, model={model}\nRestart clawd for changes to take effect."), // zh: 已更新主模型：vendor={vendor}, model={model}\n请重启 clawd 生效。
                ("telegram.error.submit_task_failed_http", "Submit task failed ({status}): {body}"), // zh: 提交任务失败({status}): {body}
                ("telegram.error.submit_task_rejected", "Submit task rejected: {error}"), // zh: 提交任务被拒绝: {error}
                ("telegram.error.submit_task_missing_task_id", "Submit task response missing task_id"), // zh: 提交任务返回缺少 task_id
                ("telegram.error.query_task_failed_http", "Query task failed ({status}): {body}"), // zh: 查询任务失败({status}): {body}
                ("telegram.error.query_task_failed", "Task query returned failure: {error}"), // zh: 任务查询返回失败: {error}
                ("telegram.error.query_task_missing_data", "Task query missing data"), // zh: 任务查询缺少 data
                ("telegram.error.task_finished_with_detail", "Task finished with status={status}, detail={detail}"), // zh: 任务结束状态={status}, 详情={detail}
                ("telegram.error.health_http_failed", "Health endpoint failed ({status}): {body}"), // zh: health 接口失败({status}): {body}
                ("telegram.error.health_failed", "Health returned failure: {error}"), // zh: health 返回失败: {error}
                ("telegram.error.health_missing_data", "Health response missing data"), // zh: health 缺少 data
                ("telegram.error.read_config_failed", "Failed to read configs/config.toml"), // zh: 读取 configs/config.toml 失败
                ("telegram.error.parse_config_failed", "Failed to parse config.toml"), // zh: 解析 config.toml 失败
                ("telegram.error.llm_config_missing", "Missing llm config"), // zh: llm 配置缺失
                ("telegram.error.config_not_table", "Config is not a valid TOML table"), // zh: config 不是有效 toml 表
                ("telegram.error.llm_struct_invalid", "Invalid llm structure"), // zh: llm 结构无效
                ("telegram.error.vendor_struct_invalid", "Invalid vendor structure"), // zh: vendor 结构无效
                ("telegram.error.models_struct_invalid", "Invalid models structure"), // zh: models 结构无效
                ("telegram.error.serialize_config_failed", "Failed to serialize TOML text"), // zh: 写回 toml 文本失败
                ("telegram.error.write_config_failed", "Failed to write configs/config.toml"), // zh: 写入 configs/config.toml 失败
            ]),
        }
    }

    fn t(&self, key: &str) -> String {
        self.current.get(key).copied().unwrap_or(key).to_string()
    }

    fn t_with(&self, key: &str, vars: &[(&str, &str)]) -> String {
        let mut out = self.t(key);
        for (name, value) in vars {
            out = out.replace(&format!("{{{name}}}"), value);
        }
        out
    }
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
    let i18n = Arc::new(TextCatalog::zh_cn());
    let bot = Bot::new(config.telegram.bot_token.clone());
    if let Err(err) = register_telegram_commands_and_menu(&config.telegram.bot_token).await {
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
        quick_result_wait_seconds: config.telegram.quick_result_wait_seconds.max(1),
        auto_vision_on_image_only: config.telegram.auto_vision_on_image_only,
        pending_image_by_chat: Arc::new(Mutex::new(HashMap::new())),
        bot_token: config.telegram.bot_token.clone(),
        image_inbox_dir: "image/upload".to_string(),
        audio_inbox_dir: config.telegram.audio_inbox_dir.clone(),
        voice_reply_mode: config.telegram.voice_reply_mode.clone(),
        voice_reply_mode_by_chat: Arc::new(Mutex::new(voice_reply_mode_by_chat)),
        max_audio_input_bytes: config.telegram.max_audio_input_bytes.max(1024),
        sendfile_admin_only: config.telegram.sendfile.admin_only,
        sendfile_full_access: config.telegram.sendfile.full_access,
        sendfile_allowed_dirs: Arc::new(config.telegram.sendfile.allowed_dirs.clone()),
        voice_chat_prompt_template: load_prompt_template(
            "prompts/voice_chat_prompt.md",
            DEFAULT_VOICE_CHAT_PROMPT_TEMPLATE,
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
                (
                    "quick_result_wait_seconds",
                    &state.quick_result_wait_seconds.to_string(),
                ),
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

    let handler = Update::filter_message().endpoint(handle_message);

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state])
        .build()
        .dispatch()
        .await;

    Ok(())
}

async fn register_telegram_commands_and_menu(bot_token: &str) -> anyhow::Result<()> {
    let api_base = format!("https://api.telegram.org/bot{bot_token}");
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("build telegram menu client failed")?;

    let commands_payload = json!({
        "commands": [
            { "command": "start", "description": "Start bot and show basic usage" },
            { "command": "help", "description": "Show help and command examples" },
            { "command": "agent", "description": "Toggle agent mode on or off" },
            { "command": "status", "description": "Show queue and worker status" },
            { "command": "cancel", "description": "Cancel queued/running tasks for this chat" },
            { "command": "skills", "description": "List available skills" },
            { "command": "run", "description": "Run a skill: /run <skill> <args>" },
            { "command": "sendfile", "description": "Send local file: /sendfile <path>" },
            { "command": "voicemode", "description": "Set voice reply mode for this chat (admin)" },
            { "command": "openclaw", "description": "Model config commands for admins" }
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

    if text.starts_with("/openclaw") {
        let is_admin = state.admins.contains(&user_id);
        if !is_admin {
            bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.openclaw_admin_only"))
                .await
                .context("send /openclaw unauthorized failed")?;
            return Ok(());
        }

        let state_for_cmd = state.clone();
        let text_owned = text.to_string();
        let openclaw_result =
            tokio::task::spawn_blocking(move || handle_openclaw_config_command(&state_for_cmd, &text_owned))
                .await
                .map_err(|err| anyhow!("join openclaw config task failed: {err}"))?;

        match openclaw_result {
            Ok(reply) => {
                bot.send_message(msg.chat.id, reply)
                    .await
                    .context("send /openclaw reply failed")?;
            }
            Err(err) => {
                bot.send_message(
                    msg.chat.id,
                    state
                        .i18n
                        .t_with("telegram.msg.config_failed", &[("error", &err.to_string())]),
                )
                    .await
                    .context("send /openclaw error failed")?;
            }
        }
        return Ok(());
    }

    if text.starts_with("/voicemode") {
        let is_admin = state.admins.contains(&user_id);
        if !is_admin {
            bot.send_message(msg.chat.id, state.i18n.t("telegram.msg.voicemode_admin_only"))
                .await
                .context("send /voicemode unauthorized failed")?;
            return Ok(());
        }
        let reply = handle_voicemode_command(&state, msg.chat.id.0, text)?;
        bot.send_message(msg.chat.id, reply)
            .await
            .context("send /voicemode reply failed")?;
        return Ok(());
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
                    format!("Canceled {} queued/running task(s) for this chat.", canceled)
                } else {
                    "No queued or running tasks to cancel for this chat.".to_string()
                };
                bot.send_message(msg.chat.id, reply)
                    .await
                    .context("send /cancel reply failed")?;
            }
            Err(err) => {
                bot.send_message(
                    msg.chat.id,
                    format!("Cancel failed: {}", err),
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
                let delivered = try_deliver_quick_result(
                    &bot,
                    &state,
                    msg.chat.id,
                    &task_id,
                    Some(state.quick_result_wait_seconds),
                    &state.i18n.t("telegram.msg.skill_exec_failed"),
                )
                .await
                .context("try quick delivery for /run failed")?;
                if !delivered {
                    spawn_task_result_delivery(
                    bot.clone(),
                    state.clone(),
                    msg.chat.id,
                    task_id,
                    None,
                    state.i18n.t("telegram.msg.skill_exec_failed"),
                    );
                }
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
            bot.send_message(msg.chat.id, "Only admins can use /sendfile.")
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
                bot.send_message(msg.chat.id, format!("Invalid sendfile path: {err}"))
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
                    let delivered = try_deliver_quick_result(
                        &bot,
                        &state,
                        msg.chat.id,
                        &task_id,
                        Some(state.quick_result_wait_seconds),
                        &state.i18n.t("telegram.msg.skill_exec_failed"),
                    )
                    .await
                    .context("try quick delivery for pending image edit failed")?;
                    if !delivered {
                        spawn_task_result_delivery(
                            bot.clone(),
                            state.clone(),
                            msg.chat.id,
                            task_id,
                            None,
                            state.i18n.t("telegram.msg.skill_exec_failed"),
                        );
                    }
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
            let delivered = try_deliver_quick_result(
                &bot,
                &state,
                msg.chat.id,
                &task_id,
                Some(state.quick_result_wait_seconds),
                &state.i18n.t("telegram.msg.process_failed"),
            )
            .await
            .context("try quick delivery for ask failed")?;
            if !delivered {
                spawn_task_result_delivery(
                    bot.clone(),
                    state.clone(),
                    msg.chat.id,
                    task_id,
                    None,
                    state.i18n.t("telegram.msg.process_failed"),
                );
            }
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
            let delivered = try_deliver_quick_result(
                bot,
                state,
                msg.chat.id,
                &task_id,
                Some(state.quick_result_wait_seconds),
                &state.i18n.t("telegram.msg.skill_exec_failed"),
            )
            .await
            .context("try quick delivery for image vision failed")?;
            if !delivered {
                spawn_task_result_delivery(
                    bot.clone(),
                    state.clone(),
                    msg.chat.id,
                    task_id,
                    None,
                    state.i18n.t("telegram.msg.skill_exec_failed"),
                );
            }
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
                format!(
                    "语音文件过大（{} bytes），当前上限 {} bytes。",
                    meta.len(),
                    state.max_audio_input_bytes
                ),
            )
            .await
            .context("send audio too large message failed")?;
            return Ok(());
        }
    }

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
    let transcript = transcript.trim();
    if transcript.is_empty() {
        bot.send_message(msg.chat.id, "语音转写结果为空，请重试。")
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
    let answer = poll_task_result(state, &ask_task_id, Some(state.task_wait_seconds.max(300)))
        .await
        .context("poll ask result for transcript failed")?;
    let mode = parse_voice_reply_mode(&effective_voice_reply_mode_for_chat(state, msg.chat.id.0));
    if matches!(mode, VoiceReplyMode::Text | VoiceReplyMode::Both) {
        send_text_or_image(bot, msg.chat.id, &answer).await?;
    }

    if matches!(mode, VoiceReplyMode::Voice | VoiceReplyMode::Both) {
        let tts_input = strip_delivery_tokens_for_tts(&answer);
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
                        let _ = send_text_or_image(bot, msg.chat.id, &tts_answer).await;
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
            send_text_or_image(bot, msg.chat.id, &answer).await?;
        }
    }
    Ok(())
}

fn extract_image_attachment(msg: &Message) -> Option<(String, String)> {
    if let Some(photos) = msg.photo() {
        if let Some(photo) = photos.last() {
            return Some((photo.file.id.to_string(), "jpg".to_string()));
        }
    }
    if let Some(doc) = msg.document() {
        let file_name_ext = doc
            .file_name
            .as_deref()
            .and_then(extension_from_filename)
            .unwrap_or_default();
        let mime_is_image = doc
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
            return Some((doc.file.id.to_string(), ext));
        }
    }
    None
}

fn extract_audio_attachment(msg: &Message) -> Option<(String, String)> {
    if let Some(voice) = msg.voice() {
        return Some((voice.file.id.to_string(), "ogg".to_string()));
    }
    if let Some(audio) = msg.audio() {
        let ext = audio
            .file_name
            .as_deref()
            .and_then(extension_from_filename)
            .unwrap_or_else(|| "mp3".to_string());
        return Some((audio.file.id.to_string(), ext));
    }
    None
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

async fn try_deliver_quick_result(
    bot: &Bot,
    state: &BotState,
    chat_id: ChatId,
    task_id: &str,
    wait_override_seconds: Option<u64>,
    fail_prefix: &str,
) -> anyhow::Result<bool> {
    match poll_task_result(state, task_id, wait_override_seconds).await {
        Ok(answer) => {
            send_text_or_image(bot, chat_id, &answer).await?;
            Ok(true)
        }
        Err(err) => {
            let msg = err.to_string();
            if msg == "task_result_wait_timeout" {
                return Ok(false);
            }
            bot.send_message(chat_id, format!("{fail_prefix}：{msg}"))
                .await
                .context("send quick error message failed")?;
            Ok(true)
        }
    }
}

async fn send_text_or_image(bot: &Bot, chat_id: ChatId, answer: &str) -> anyhow::Result<()> {
    const PREFIX: &str = "IMAGE_FILE:";
    const FILE_PREFIX: &str = "FILE:";
    const VOICE_PREFIX: &str = "VOICE_FILE:";

    let image_paths = extract_prefixed_paths(answer, PREFIX);
    let file_paths = extract_prefixed_paths(answer, FILE_PREFIX);
    let voice_paths = extract_prefixed_paths(answer, VOICE_PREFIX);

    if !image_paths.is_empty() || !file_paths.is_empty() || !voice_paths.is_empty() {
        let text_without_tokens =
            strip_prefixed_tokens(answer, &[PREFIX, FILE_PREFIX, VOICE_PREFIX]).trim().to_string();
        if !text_without_tokens.is_empty() {
            bot.send_message(chat_id, text_without_tokens)
                .await
                .context("send file preface text failed")?;
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

    bot.send_message(chat_id, answer.to_string())
        .await
        .context("send text message failed")?;
    Ok(())
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

fn strip_prefixed_tokens(answer: &str, prefixes: &[&str]) -> String {
    answer
        .lines()
        .filter(|line| !prefixes.iter().any(|prefix| line.trim_start().starts_with(prefix)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_delivery_tokens_for_tts(answer: &str) -> String {
    strip_prefixed_tokens(answer, &["IMAGE_FILE:", "FILE:", "VOICE_FILE:"])
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
    task_id: String,
    soft_notice_override_seconds: Option<u64>,
    fail_prefix: String,
) {
    tokio::spawn(async move {
        let poll_interval_ms = state.poll_interval_ms.max(1);
        let soft_notice_seconds = soft_notice_override_seconds
            .unwrap_or(state.task_wait_seconds)
            .max(1);
        let started_at = tokio::time::Instant::now();
        let mut soft_notice_sent = false;

        loop {
            match query_task_status(&state, &task_id).await {
                Ok(task) => match task.status {
                    TaskStatus::Queued | TaskStatus::Running => {
                        if !soft_notice_sent
                            && started_at.elapsed() >= Duration::from_secs(soft_notice_seconds)
                        {
                            let msg = state.i18n.t_with(
                                "telegram.msg.task_still_running_background",
                                &[("seconds", &soft_notice_seconds.to_string())],
                            );
                            let _ = bot.send_message(chat_id, msg).await;
                            soft_notice_sent = true;
                        }
                        tokio::time::sleep(Duration::from_millis(poll_interval_ms)).await;
                    }
                    TaskStatus::Succeeded => {
                        let answer = task_success_text(&state, &task);
                        let _ = send_text_or_image(&bot, chat_id, &answer).await;
                        break;
                    }
                    TaskStatus::Failed | TaskStatus::Canceled | TaskStatus::Timeout => {
                        let detail = task_terminal_error_text(&state, &task);
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

fn task_success_text(state: &BotState, task: &TaskQueryResponse) -> String {
    task.result_json
        .as_ref()
        .and_then(|v| v.get("text"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| state.i18n.t("telegram.msg.task_done_no_text"))
}

fn task_terminal_error_text(state: &BotState, task: &TaskQueryResponse) -> String {
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
    let submit_req = SubmitTaskRequest {
        user_id,
        chat_id,
        kind,
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

    Ok(task_id.to_string())
}

async fn poll_task_result(
    state: &BotState,
    task_id: &str,
    wait_override_seconds: Option<u64>,
) -> anyhow::Result<String> {
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
                return Ok(task_success_text(state, &task));
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
    let raw = fs::read_to_string("configs/config.toml")
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
    fs::write("configs/config.toml", output).context(state.i18n.t("telegram.error.write_config_failed"))?;
    Ok(())
}

fn handle_openclaw_config_command(state: &BotState, text: &str) -> anyhow::Result<String> {
    let cmd = text.strip_prefix("/openclaw").unwrap_or_default().trim();
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

    let vendors = ["openai", "google", "anthropic", "grok"];
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

fn set_model_config(state: &BotState, vendor: &str, model: &str) -> anyhow::Result<String> {
    if !matches!(vendor, "openai" | "google" | "anthropic" | "grok") {
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

    let llm = value
        .as_table_mut()
        .ok_or_else(|| anyhow!("{}", state.i18n.t("telegram.error.config_not_table")))?
        .entry("llm")
        .or_insert(TomlValue::Table(toml::map::Map::new()));

    if !llm.is_table() {
        *llm = TomlValue::Table(toml::map::Map::new());
    }

    let llm_tbl = llm
        .as_table_mut()
        .ok_or_else(|| anyhow!("{}", state.i18n.t("telegram.error.llm_struct_invalid")))?;
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
        .ok_or_else(|| anyhow!("{}", state.i18n.t("telegram.error.vendor_struct_invalid")))?;
    vendor_tbl.insert("model".to_string(), TomlValue::String(model.to_string()));

    let models_value = vendor_tbl
        .entry("models".to_string())
        .or_insert(TomlValue::Array(vec![]));
    if !models_value.is_array() {
        *models_value = TomlValue::Array(vec![]);
    }
    let models = models_value
        .as_array_mut()
        .ok_or_else(|| anyhow!("{}", state.i18n.t("telegram.error.models_struct_invalid")))?;
    let exists = models.iter().any(|v| v.as_str() == Some(model));
    if !exists {
        models.push(TomlValue::String(model.to_string()));
    }

    let output =
        toml::to_string_pretty(&value).context(state.i18n.t("telegram.error.serialize_config_failed"))?;
    fs::write("configs/config.toml", output).context(state.i18n.t("telegram.error.write_config_failed"))?;

    Ok(state.i18n.t_with(
        "telegram.msg.openclaw_set_ok",
        &[("vendor", vendor), ("model", model)],
    ))
}
