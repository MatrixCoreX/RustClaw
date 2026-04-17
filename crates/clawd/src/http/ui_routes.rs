use axum::extract::{Multipart, Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use rusqlite::OptionalExtension;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio as StdProcessStdio};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::process::Command;
use tokio::sync::Semaphore;

use super::super::{
    attach_pending_channel_bind_session_install_flow, bind_channel_identity,
    channel_gateway_process_stats, create_auth_key, create_pending_channel_bind_session,
    current_rss_bytes, daemon_process_pids_by_name, delete_auth_key_by_id,
    exchange_credential_status_for_user_key, feishud_process_stats,
    finalize_pending_channel_bind_session, get_auth_key_value_by_id,
    get_pending_channel_bind_session_by_id, get_pending_channel_bind_session_by_token,
    has_channel_binding_for_user_key, larkd_process_stats, list_auth_keys,
    mark_pending_channel_bind_session_detected, mark_pending_channel_bind_session_expired,
    mark_pending_channel_bind_session_failed, mask_secret, oldest_running_task_age_seconds,
    reload_skill_views, reset_channel_binding_state_for_user_key, resolve_auth_identity_by_key,
    resolve_channel_binding_identity, task_count_by_status, telegramd_process_stats,
    update_auth_key_by_id, upsert_exchange_credential_for_user_key, upsert_webd_login_account,
    verify_webd_password_login, wa_webd_process_stats, webd_process_stats, wechatd_process_stats,
    whatsappd_process_stats, ApiResponse, AppState, HealthResponse, LlmProviderRuntime,
    LocalInteractionContext, PendingChannelBindSession,
};
use claw_core::types::{
    AuthIdentity, BindChannelKeyRequest, DetectFeishuBindSessionRequest,
    DetectFeishuBindSessionResponse, ExchangeCredentialStatus, FeishuBindSessionStatusResponse,
    GatewayInstanceRuntimeStatus, ResolveChannelBindingRequest, ResolveChannelBindingResponse,
    StartFeishuBindSessionRequest, TelegramBotRuntimeStatus, UiKeyVerifyRequest,
    UpsertExchangeCredentialRequest,
};
use claw_core::{prompt_layers, skill_registry::SkillKind};

const UI_HIDDEN_SKILLS: &[&str] = &["chat"];
const TELEGRAM_BOT_HEARTBEAT_STALE_SECONDS: i64 = 45;
const FEISHU_BIND_SESSION_DEFAULT_TTL_SECONDS: u64 = 600;
const FEISHU_BIND_SESSION_MIN_TTL_SECONDS: u64 = 60;
const FEISHU_BIND_SESSION_MAX_TTL_SECONDS: u64 = 1800;
const FEISHU_OFFICIAL_ACCOUNTS_BASE_URL: &str = "https://accounts.feishu.cn";
const FEISHU_CONFIG_TEMPLATE: &str = r#"# Feishu（中国站）应用机器人通道配置 - 与 lark.toml（国际版）独立，勿混用
# 飞书中国站使用 open.feishu.cn；国际版 Lark 使用 open.larksuite.com，由 lark.toml 配置
# 支持文本与入站媒体（图片/文件/音视频）落盘后再提交 clawd ask
# 使用方式（二选一）：
#   - webhook：应用机器人 → 事件订阅 → 请求地址配置为本服务 callback URL（需公网可达）
#   - long_connection：应用机器人 → 事件订阅 → 使用长连接接收事件（无需公网，内网优先）

[feishu]
# 是否启用
enabled = true
# 入站模式：webhook | long_connection
mode = "long_connection"
# 本服务监听地址（webhook 模式时用于接收回调；long_connection 模式可选用于健康检查等）
listen = "0.0.0.0:8789"
# clawd 基地址，用于提交任务与轮询结果
clawd_base_url = "http://127.0.0.1:8787"
# 飞书中国站 API 根地址（与 lark.toml 的国际版分开，勿改为 open.larksuite.com）
api_base_url = "https://open.feishu.cn"
# i18n 语言（优先尝试 configs/i18n/feishud.<language>.toml）
language = "zh-CN"
# i18n 文件路径（当 language 对应文件不存在时回退到此路径）
i18n_path = "configs/i18n/feishud.zh-CN.toml"
# 入站媒体落盘根目录（相对 feishud 进程工作目录；实际路径为 <目录>/<chat_id>/<文件名>）
image_inbox_dir = "data/feishud/image"
video_inbox_dir = "data/feishud/video"
audio_inbox_dir = "data/feishud/audio"
file_inbox_dir = "data/feishud/file"
# 飞书应用 App ID
app_id = ""
# 飞书应用 App Secret（日志不打印）
app_secret = ""
# 事件订阅 Verification Token（webhook 时校验请求来自飞书）
verification_token = ""
# 事件加密密钥（webhook 时用于消息解密/签名校验）
encrypt_key = ""
# 单次 HTTP 请求超时秒数（submit / poll / 发消息等单次请求）
request_timeout_seconds = 30
# 任务投递软超时阈值（秒）：达到后会提示“任务仍在执行，完成后回复”，并继续轮询，不会中断投递
task_delivery_timeout_seconds = 600
# 长文本分段发送时每段最大字符数（按 UTF-8 安全截断）
text_chunk_chars = 4000
"#;
const LLM_CONNECTIVITY_TEST_PROMPT: &str = "Reply with OK only.";

fn hide_skill_in_ui(state: &AppState, name: &str) -> bool {
    let canonical = state.resolve_canonical_skill_name(name);
    UI_HIDDEN_SKILLS.iter().any(|s| *s == canonical)
}

fn read_telegram_bot_statuses(
    workspace_root: &Path,
    configured_names: &[String],
) -> Vec<TelegramBotRuntimeStatus> {
    let status_dir = workspace_root.join("run").join("telegram-bot-status");
    let mut by_name: HashMap<String, TelegramBotRuntimeStatus> = HashMap::new();
    if let Ok(entries) = fs::read_dir(&status_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(raw) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(mut status) = serde_json::from_str::<TelegramBotRuntimeStatus>(&raw) else {
                continue;
            };
            if let Some(last_ts) = status.last_heartbeat_ts {
                let age = current_unix_ts().saturating_sub(last_ts);
                if age > TELEGRAM_BOT_HEARTBEAT_STALE_SECONDS {
                    status.healthy = false;
                    if status.status == "running" {
                        status.status = "stale".to_string();
                    }
                }
            } else {
                status.healthy = false;
            }
            by_name.insert(status.name.clone(), status);
        }
    }

    configured_names
        .iter()
        .map(|name| {
            by_name
                .remove(name)
                .unwrap_or_else(|| TelegramBotRuntimeStatus {
                    name: name.clone(),
                    healthy: false,
                    status: "missing".to_string(),
                    last_heartbeat_ts: None,
                    last_error: None,
                })
        })
        .collect()
}

fn read_gateway_instance_statuses(
    workspace_root: &Path,
) -> HashMap<String, GatewayInstanceRuntimeStatus> {
    let status_dir = workspace_root.join("run").join("gateway-instance-status");
    let mut by_scope = HashMap::new();
    if let Ok(entries) = fs::read_dir(&status_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(raw) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(mut status) = serde_json::from_str::<GatewayInstanceRuntimeStatus>(&raw) else {
                continue;
            };
            if let Some(last_ts) = status.last_heartbeat_ts {
                let age = current_unix_ts().saturating_sub(last_ts);
                if age > TELEGRAM_BOT_HEARTBEAT_STALE_SECONDS {
                    status.healthy = false;
                    if status.status == "running" {
                        status.status = "stale".to_string();
                    }
                }
            } else {
                status.healthy = false;
            }
            by_scope.insert(status.scope.clone(), status);
        }
    }
    by_scope
}

fn current_unix_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn telegram_config_path(state: &AppState) -> PathBuf {
    state.workspace_root.join("configs/channels/telegram.toml")
}

fn wechat_config_path(state: &AppState) -> PathBuf {
    state.workspace_root.join("configs/channels/wechat.toml")
}

fn feishu_config_path(state: &AppState) -> PathBuf {
    state.workspace_root.join("configs/channels/feishu.toml")
}

fn read_telegram_config_value(state: &AppState) -> anyhow::Result<toml::Value> {
    let path = telegram_config_path(state);
    if !path.exists() {
        return Ok(toml::Value::Table(Default::default()));
    }
    let raw = std::fs::read_to_string(&path)?;
    if raw.trim().is_empty() {
        return Ok(toml::Value::Table(Default::default()));
    }
    Ok(toml::from_str(&raw)?)
}

fn read_wechat_config_value(state: &AppState) -> anyhow::Result<toml::Value> {
    let path = wechat_config_path(state);
    if !path.exists() {
        return Ok(toml::Value::Table(Default::default()));
    }
    let raw = std::fs::read_to_string(&path)?;
    if raw.trim().is_empty() {
        return Ok(toml::Value::Table(Default::default()));
    }
    Ok(toml::from_str(&raw)?)
}

fn read_feishu_config_value(state: &AppState) -> anyhow::Result<toml::Value> {
    let path = feishu_config_path(state);
    if !path.exists() {
        return Ok(toml::Value::Table(Default::default()));
    }
    let raw = std::fs::read_to_string(&path)?;
    if raw.trim().is_empty() {
        return Ok(toml::Value::Table(Default::default()));
    }
    Ok(toml::from_str(&raw)?)
}

fn read_feishu_config_raw(state: &AppState) -> anyhow::Result<String> {
    let path = feishu_config_path(state);
    if !path.exists() {
        return Ok(FEISHU_CONFIG_TEMPLATE.to_string());
    }
    let raw = std::fs::read_to_string(&path)?;
    if raw.trim().is_empty() {
        return Ok(FEISHU_CONFIG_TEMPLATE.to_string());
    }
    Ok(raw)
}

fn toml_string_literal(value: &str) -> String {
    toml::Value::String(value.to_string()).to_string()
}

fn upsert_section_key_line(raw: &str, section: &str, key: &str, rendered_value: &str) -> String {
    let mut lines: Vec<String> = raw.lines().map(|line| line.to_string()).collect();
    let section_header = format!("[{section}]");
    let mut section_start = lines.iter().position(|line| line.trim() == section_header);
    if section_start.is_none() {
        if !lines.is_empty() && !lines.last().is_some_and(|line| line.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(section_header);
        section_start = Some(lines.len() - 1);
    }
    let start = section_start.expect("section start must exist");
    let section_end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, line)| {
            let trimmed = line.trim();
            trimmed.starts_with('[') && trimmed.ends_with(']')
        })
        .map(|(idx, _)| idx)
        .unwrap_or(lines.len());
    let target_idx = (start + 1..section_end).find(|idx| {
        let trimmed = lines[*idx].trim_start();
        if trimmed.starts_with('#') {
            return false;
        }
        trimmed
            .strip_prefix(key)
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    });
    let new_line = format!("{key} = {rendered_value}");
    if let Some(idx) = target_idx {
        lines[idx] = new_line;
    } else {
        let mut insert_at = section_end;
        while insert_at > start + 1 && lines[insert_at - 1].trim().is_empty() {
            insert_at -= 1;
        }
        lines.insert(insert_at, new_line);
    }
    let mut output = lines.join("\n");
    output.push('\n');
    output
}

fn update_feishu_config_raw_preserving_format(raw: &str, app_id: &str, app_secret: &str) -> String {
    let enabled = !app_id.trim().is_empty() && !app_secret.trim().is_empty();
    let mut output = if raw.trim().is_empty() {
        FEISHU_CONFIG_TEMPLATE.to_string()
    } else {
        raw.to_string()
    };
    for (key, rendered_value) in [
        ("enabled", enabled.to_string()),
        ("app_id", toml_string_literal(app_id.trim())),
        ("app_secret", toml_string_literal(app_secret.trim())),
        ("mode", toml_string_literal("long_connection")),
        ("listen", toml_string_literal("0.0.0.0:8789")),
        (
            "clawd_base_url",
            toml_string_literal("http://127.0.0.1:8787"),
        ),
        (
            "api_base_url",
            toml_string_literal("https://open.feishu.cn"),
        ),
        ("language", toml_string_literal("zh-CN")),
        (
            "i18n_path",
            toml_string_literal("configs/i18n/feishud.zh-CN.toml"),
        ),
        ("image_inbox_dir", toml_string_literal("data/feishud/image")),
        ("video_inbox_dir", toml_string_literal("data/feishud/video")),
        ("audio_inbox_dir", toml_string_literal("data/feishud/audio")),
        ("file_inbox_dir", toml_string_literal("data/feishud/file")),
        ("verification_token", toml_string_literal("")),
        ("encrypt_key", toml_string_literal("")),
        ("request_timeout_seconds", "30".to_string()),
        ("task_delivery_timeout_seconds", "600".to_string()),
        ("text_chunk_chars", "4000".to_string()),
    ] {
        output = upsert_section_key_line(&output, "feishu", key, &rendered_value);
    }
    output
}

fn reset_feishu_config_raw_preserving_format(raw: &str) -> String {
    let mut output = if raw.trim().is_empty() {
        FEISHU_CONFIG_TEMPLATE.to_string()
    } else {
        raw.to_string()
    };
    for (key, rendered_value) in [
        ("enabled", "false".to_string()),
        ("app_id", toml_string_literal("")),
        ("app_secret", toml_string_literal("")),
        ("verification_token", toml_string_literal("")),
        ("encrypt_key", toml_string_literal("")),
    ] {
        output = upsert_section_key_line(&output, "feishu", key, &rendered_value);
    }
    output
}

fn ensure_toml_table<'a>(
    root: &'a mut toml::Value,
    path: &[&str],
) -> anyhow::Result<&'a mut toml::map::Map<String, toml::Value>> {
    let mut current = root
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("config root is not a TOML table"))?;
    for segment in path {
        let value = current
            .entry((*segment).to_string())
            .or_insert_with(|| toml::Value::Table(Default::default()));
        if !value.is_table() {
            *value = toml::Value::Table(Default::default());
        }
        current = value
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("config section {} is not a table", segment))?;
    }
    Ok(current)
}

fn telegram_bots_from_config(config: &claw_core::config::AppConfig) -> Vec<TelegramBotConfigItem> {
    let mut bots = Vec::new();
    if !config.telegram.bot_token.trim().is_empty() {
        bots.push(TelegramBotConfigItem {
            name: "primary".to_string(),
            bot_token: String::new(),
            bot_token_configured: true,
            bot_token_masked: Some(mask_secret(&config.telegram.bot_token)),
            agent_id: if config.telegram.agent_id.trim().is_empty() {
                "main".to_string()
            } else {
                config.telegram.agent_id.trim().to_string()
            },
            allowlist: config.telegram.allowlist.clone(),
            access_mode: normalize_telegram_access_mode(&config.telegram.access_mode),
            allowed_telegram_usernames: normalize_telegram_username_list(
                &config.telegram.allowed_usernames,
            ),
            is_primary: true,
        });
    }
    bots.extend(
        config
            .telegram
            .bots
            .iter()
            .map(|bot| TelegramBotConfigItem {
                name: bot.name.clone(),
                bot_token: String::new(),
                bot_token_configured: !bot.bot_token.trim().is_empty(),
                bot_token_masked: if bot.bot_token.trim().is_empty() {
                    None
                } else {
                    Some(mask_secret(&bot.bot_token))
                },
                agent_id: if bot.agent_id.trim().is_empty() {
                    "main".to_string()
                } else {
                    bot.agent_id.trim().to_string()
                },
                allowlist: bot.allowlist.clone(),
                access_mode: if bot.access_mode.trim().is_empty() {
                    normalize_telegram_access_mode(&config.telegram.access_mode)
                } else {
                    normalize_telegram_access_mode(&bot.access_mode)
                },
                allowed_telegram_usernames: if bot.allowed_usernames.is_empty() {
                    normalize_telegram_username_list(&config.telegram.allowed_usernames)
                } else {
                    normalize_telegram_username_list(&bot.allowed_usernames)
                },
                is_primary: false,
            }),
    );
    bots
}

fn telegram_bot_tokens_from_config(
    config: &claw_core::config::AppConfig,
) -> std::collections::HashMap<String, String> {
    let mut tokens = std::collections::HashMap::new();
    if !config.telegram.bot_token.trim().is_empty() {
        tokens.insert(
            "primary".to_string(),
            config.telegram.bot_token.trim().to_string(),
        );
    }
    for bot in &config.telegram.bots {
        let name = bot.name.trim();
        if name.is_empty() || bot.bot_token.trim().is_empty() {
            continue;
        }
        tokens.insert(name.to_string(), bot.bot_token.trim().to_string());
    }
    tokens
}

fn agents_from_config(config: &claw_core::config::AppConfig) -> Vec<AgentConfigItem> {
    config
        .normalized_agents()
        .into_iter()
        .map(|agent| AgentConfigItem {
            id: agent.id,
            name: agent.name,
            description: agent.description,
            persona_prompt: agent.persona_prompt,
            preferred_vendor: agent.preferred_vendor,
            preferred_model: agent.preferred_model,
            allowed_skills: agent.allowed_skills,
        })
        .collect()
}

fn normalize_agent_items(agents: &[AgentConfigItem]) -> anyhow::Result<Vec<AgentConfigItem>> {
    let mut normalized = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (index, agent) in agents.iter().enumerate() {
        let id = if agent.id.trim().is_empty() {
            if index == 0 {
                "main".to_string()
            } else {
                format!("agent-{}", index + 1)
            }
        } else {
            agent.id.trim().to_string()
        };
        if !seen.insert(id.clone()) {
            return Err(anyhow::anyhow!("duplicate agent id: {id}"));
        }
        normalized.push(AgentConfigItem {
            id: id.clone(),
            name: if agent.name.trim().is_empty() {
                if id == "main" {
                    "Main".to_string()
                } else {
                    id.clone()
                }
            } else {
                agent.name.trim().to_string()
            },
            description: agent.description.trim().to_string(),
            persona_prompt: agent.persona_prompt.trim().to_string(),
            preferred_vendor: agent
                .preferred_vendor
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string),
            preferred_model: agent
                .preferred_model
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string),
            allowed_skills: agent
                .allowed_skills
                .iter()
                .map(|skill| skill.trim())
                .filter(|skill| !skill.is_empty())
                .map(ToString::to_string)
                .collect(),
        });
    }
    if !seen.contains("main") {
        normalized.insert(
            0,
            AgentConfigItem {
                id: "main".to_string(),
                name: "Main".to_string(),
                description: String::new(),
                persona_prompt: String::new(),
                preferred_vendor: None,
                preferred_model: None,
                allowed_skills: Vec::new(),
            },
        );
    }
    Ok(normalized)
}

fn normalize_telegram_bot_items(
    bots: &[TelegramBotConfigItem],
    known_agent_ids: &std::collections::HashSet<String>,
) -> anyhow::Result<Vec<TelegramBotConfigItem>> {
    let mut normalized = Vec::new();
    let mut has_primary = false;
    let mut names = std::collections::HashSet::new();
    for (index, bot) in bots.iter().enumerate() {
        let is_primary = bot.is_primary || index == 0;
        let name = if is_primary {
            "primary".to_string()
        } else {
            bot.name.trim().to_string()
        };
        if !is_primary && name.is_empty() {
            return Err(anyhow::anyhow!("secondary telegram bot name is required"));
        }
        if !name.is_empty() && !names.insert(name.clone()) {
            return Err(anyhow::anyhow!("duplicate telegram bot name: {name}"));
        }
        if is_primary {
            if has_primary {
                return Err(anyhow::anyhow!("only one primary telegram bot is allowed"));
            }
            has_primary = true;
        }
        let agent_id = if bot.agent_id.trim().is_empty() {
            "main".to_string()
        } else {
            bot.agent_id.trim().to_string()
        };
        if !known_agent_ids.contains(&agent_id) {
            return Err(anyhow::anyhow!(
                "unknown agent id for telegram bot {name}: {agent_id}"
            ));
        }
        normalized.push(TelegramBotConfigItem {
            name,
            bot_token: bot.bot_token.trim().to_string(),
            bot_token_configured: !bot.bot_token.trim().is_empty(),
            bot_token_masked: None,
            agent_id,
            allowlist: bot.allowlist.clone(),
            access_mode: normalize_telegram_access_mode(&bot.access_mode),
            allowed_telegram_usernames: normalize_telegram_username_list(
                &bot.allowed_telegram_usernames,
            ),
            is_primary,
        });
    }
    Ok(normalized)
}

fn normalize_telegram_access_mode(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "specified" | "specified_accounts" | "restricted" | "private" => "specified".to_string(),
        _ => "public".to_string(),
    }
}

fn normalize_telegram_username(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches('@').trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

fn normalize_telegram_username_list(values: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut normalized = Vec::new();
    for value in values {
        if let Some(name) = normalize_telegram_username(value) {
            if seen.insert(name.clone()) {
                normalized.push(name);
            }
        }
    }
    normalized
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceAction {
    Start,
    Stop,
    Restart,
}

pub(crate) fn build_ui_router() -> Router<AppState> {
    Router::new()
        .route("/auth/ui-key/verify", post(verify_ui_key))
        .route("/auth/me", get(auth_me))
        .route("/auth/channel/resolve", post(resolve_channel_binding))
        .route("/auth/channel/bind", post(bind_channel_key))
        .route(
            "/auth/channel-binds/feishu/detect",
            post(detect_feishu_bind_session_handler),
        )
        .route(
            "/auth/crypto-credentials",
            get(get_crypto_credentials).post(upsert_crypto_credentials),
        )
        .route("/health", get(health))
        .route("/skills", get(list_skills))
        .route(
            "/skills/config",
            get(get_skills_config).post(update_skills_config),
        )
        .route(
            "/telegram/config",
            get(get_telegram_config).post(update_telegram_config),
        )
        .route(
            "/wechat/config",
            get(get_wechat_config).post(update_wechat_config),
        )
        .route(
            "/feishu/config",
            get(get_feishu_config).post(update_feishu_config),
        )
        .route("/admin/feishu/reset", post(reset_feishu_config_handler))
        .route("/skills/import", post(import_external_skill))
        .route("/skills/import/upload", post(import_external_skill_upload))
        .route("/skills/uninstall", post(uninstall_external_skill))
        .route("/llm/config", get(get_llm_config).post(update_llm_config))
        .route("/llm/test", post(test_llm_config))
        .route("/logs/latest", get(logs_latest))
        .route("/debug/tasks/:task_id", get(task_debug_detail))
        .route("/debug/recent-robot-tasks", get(recent_robot_tasks))
        .route("/debug/usage-records", get(usage_records))
        .route("/debug/usage-records/:record_id", get(usage_record_detail))
        .route("/wechat/login-status", get(wechat_login_status))
        .route("/wechat/login-qr/start", post(wechat_login_qr_start))
        .route("/wechat/login-qr/wait", post(wechat_login_qr_wait))
        .route("/whatsapp-web/login-status", get(whatsapp_web_login_status))
        .route("/whatsapp-web/logout", post(whatsapp_web_logout))
        .route("/services/:service/:action", post(control_service))
        .route("/system/restart", post(restart_system))
        .route("/local/interaction-context", get(local_interaction_context))
        .route(
            "/admin/model-config",
            get(get_model_config).post(update_model_config),
        )
        .route(
            "/admin/provider-keys",
            get(get_provider_keys).post(update_provider_keys),
        )
        .route("/admin/restart-clawd", post(restart_clawd))
        .route(
            "/admin/auth-keys",
            get(get_auth_keys).post(create_auth_key_handler),
        )
        .route(
            "/admin/auth-keys/:key_id/full",
            get(get_auth_key_full_handler),
        )
        .route(
            "/admin/channel-binds/feishu/start",
            post(start_feishu_bind_session_handler),
        )
        .route(
            "/admin/channel-binds/feishu/:session_id",
            get(get_feishu_bind_session_handler),
        )
        .route(
            "/admin/auth-keys/:key_id",
            put(update_auth_key_handler).delete(delete_auth_key_handler),
        )
        .route(
            "/internal/webd/verify-login",
            post(webd_internal_verify_login),
        )
        .route("/admin/webd-accounts", post(admin_upsert_webd_account))
}

#[derive(Debug, Deserialize)]
struct CreateAuthKeyRequest {
    #[serde(default)]
    role: String,
}

#[derive(Debug, Deserialize)]
struct UpdateAuthKeyRequest {
    role: Option<String>,
    enabled: Option<bool>,
}

async fn get_auth_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    match list_auth_keys(&state) {
        Ok(rows) => {
            let list: Vec<Value> = rows
                .into_iter()
                .filter(|row| {
                    identity.role.eq_ignore_ascii_case("admin") || row.user_key == identity.user_key
                })
                .map(|row| {
                    json!({
                        "key_id": row.key_id,
                        "user_key": row.user_key,
                        "user_key_masked": row.user_key_masked,
                        "role": row.role,
                        "enabled": row.enabled != 0,
                        "created_at": row.created_at,
                        "last_used_at": row.last_used_at,
                        "webd_username": row.webd_username,
                        "current_key": row.user_key == identity.user_key,
                    })
                })
                .collect();
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({ "keys": list })),
                    error: None,
                }),
            )
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("list auth keys failed: {err}")),
            }),
        ),
    }
}

async fn get_auth_key_full_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(key_id): AxumPath<i64>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can reveal auth keys".to_string()),
            }),
        );
    }

    match get_auth_key_value_by_id(&state, key_id) {
        Ok(Some(user_key)) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({ "user_key": user_key })),
                error: None,
            }),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("auth key not found".to_string()),
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("get auth key failed: {err}")),
            }),
        ),
    }
}

fn clamp_feishu_bind_ttl_seconds(raw: Option<u64>) -> u64 {
    raw.unwrap_or(FEISHU_BIND_SESSION_DEFAULT_TTL_SECONDS)
        .clamp(
            FEISHU_BIND_SESSION_MIN_TTL_SECONDS,
            FEISHU_BIND_SESSION_MAX_TTL_SECONDS,
        )
}

#[derive(Debug, Deserialize, Default)]
struct FeishuOfficialRegistrationInitResponse {
    #[serde(default)]
    supported_auth_methods: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
struct FeishuOfficialRegistrationBeginResponse {
    #[serde(default)]
    device_code: String,
    #[serde(default)]
    verification_uri_complete: String,
    #[serde(default)]
    interval: Option<u64>,
    #[serde(default)]
    expire_in: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
struct FeishuOfficialRegistrationUserInfo {
    #[serde(default)]
    open_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct FeishuOfficialRegistrationPollResponse {
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default)]
    user_info: Option<FeishuOfficialRegistrationUserInfo>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

fn feishu_accounts_base_url() -> String {
    std::env::var("RUSTCLAW_FEISHU_ACCOUNTS_BASE_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| FEISHU_OFFICIAL_ACCOUNTS_BASE_URL.to_string())
}

async fn call_feishu_official_registration<T: DeserializeOwned>(
    state: &AppState,
    params: &[(&str, &str)],
) -> anyhow::Result<T> {
    let url = format!("{}/oauth/v1/app/registration", feishu_accounts_base_url());
    let resp = state.http_client.post(url).form(params).send().await?;
    let status = resp.status();
    let body = resp.text().await?;
    serde_json::from_str::<T>(&body).map_err(|err| {
        anyhow::anyhow!(
            "decode feishu registration response failed: status={} body={} err={}",
            status,
            body,
            err
        )
    })
}

async fn begin_feishu_official_registration(
    state: &AppState,
) -> anyhow::Result<FeishuOfficialRegistrationBeginResponse> {
    let init = call_feishu_official_registration::<FeishuOfficialRegistrationInitResponse>(
        state,
        &[("action", "init")],
    )
    .await?;
    if !init
        .supported_auth_methods
        .iter()
        .any(|method| method == "client_secret")
    {
        anyhow::bail!("feishu registration does not support client_secret auth");
    }
    let begin = call_feishu_official_registration::<FeishuOfficialRegistrationBeginResponse>(
        state,
        &[
            ("action", "begin"),
            ("archetype", "PersonalAgent"),
            ("auth_method", "client_secret"),
            ("request_user_info", "open_id"),
        ],
    )
    .await?;
    if begin.device_code.trim().is_empty() || begin.verification_uri_complete.trim().is_empty() {
        anyhow::bail!("feishu registration did not return a device_code or verification url");
    }
    Ok(begin)
}

async fn poll_feishu_official_registration(
    state: &AppState,
    device_code: &str,
) -> anyhow::Result<FeishuOfficialRegistrationPollResponse> {
    call_feishu_official_registration::<FeishuOfficialRegistrationPollResponse>(
        state,
        &[("action", "poll"), ("device_code", device_code)],
    )
    .await
}

fn feishu_entry_url_for_app_id(app_id: &str) -> Option<String> {
    let trimmed = app_id.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(format!(
        "https://applink.feishu.cn/client/bot/open?appId={trimmed}"
    ))
}

fn feishu_bind_entry_url(
    state: &AppState,
    session: Option<&PendingChannelBindSession>,
) -> Option<String> {
    let config = load_feishu_config_response(state, None).ok()?;
    if config.bind_ready {
        if let Some(entry_url) = feishu_entry_url_for_app_id(&config.app_id) {
            return Some(entry_url);
        }
    }
    session
        .and_then(|session| session.install_verification_url.clone())
        .filter(|url| !url.trim().is_empty())
}

fn feishu_bind_session_response(
    state: &AppState,
    session: PendingChannelBindSession,
) -> FeishuBindSessionStatusResponse {
    let entry_url = feishu_bind_entry_url(state, Some(&session));
    FeishuBindSessionStatusResponse {
        session_id: session.id,
        channel: session.channel,
        bind_token: session.bind_token,
        status: session.status,
        external_user_id: session.external_user_id,
        external_chat_id: session.external_chat_id,
        error_text: session.error_text,
        created_at: session.created_at,
        updated_at: session.updated_at,
        expires_at: session.expires_at,
        entry_url,
    }
}

fn maybe_expire_feishu_bind_session(
    db: &mut rusqlite::Connection,
    session: PendingChannelBindSession,
) -> anyhow::Result<PendingChannelBindSession> {
    if matches!(session.status.as_str(), "pending" | "detected") {
        let expires_at = session.expires_at.parse::<i64>().unwrap_or_default();
        if expires_at > 0 && expires_at <= current_unix_ts() {
            return mark_pending_channel_bind_session_expired(db, session.id);
        }
    }
    Ok(session)
}

fn write_feishu_generated_credentials(
    state: &AppState,
    app_id: &str,
    app_secret: &str,
) -> anyhow::Result<()> {
    let raw = read_feishu_config_raw(state)?;
    let output = update_feishu_config_raw_preserving_format(&raw, app_id, app_secret);
    write_workspace_and_mounted_file(
        &state.workspace_root,
        "configs/channels/feishu.toml",
        &output,
    )?;
    Ok(())
}

async fn start_service_if_needed(state: &AppState, service: &str) -> anyhow::Result<()> {
    if service_is_running(service) {
        return Ok(());
    }
    let profile = std::env::var("RUSTCLAW_START_PROFILE")
        .ok()
        .filter(|v| matches!(v.as_str(), "debug" | "release"))
        .unwrap_or_else(|| runtime_profile_default().to_string());
    let script_name = service_start_script(service)
        .ok_or_else(|| anyhow::anyhow!("unsupported service: {service}"))?;
    validate_service_start_readiness(state, service).map_err(|err| anyhow::anyhow!(err))?;
    let workspace = state.workspace_root.to_string_lossy();
    let log_file = format!("logs/{}.log", service);
    let cmd = format!(
        "cd {} && mkdir -p logs .pids && nohup ./{} {} > {} 2>&1 &",
        shell_escape_arg(workspace.as_ref()),
        script_name,
        shell_escape_arg(profile.as_str()),
        shell_escape_arg(log_file.as_str())
    );
    spawn_background_shell(&cmd)?;
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    if !service_is_running(service) {
        anyhow::bail!(
            "service did not enter running state: {service}. check logs/{service}.log and channel config"
        );
    }
    Ok(())
}

async fn maybe_complete_feishu_official_scan(
    state: &AppState,
    session: PendingChannelBindSession,
) -> anyhow::Result<PendingChannelBindSession> {
    if !matches!(session.status.as_str(), "pending" | "detected") {
        return Ok(session);
    }
    let Some(device_code) = session
        .install_device_code
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(session);
    };

    let poll = poll_feishu_official_registration(state, device_code).await?;
    if let (Some(client_id), Some(client_secret), Some(_open_id)) = (
        poll.client_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        poll.client_secret
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        poll.user_info
            .as_ref()
            .and_then(|user| user.open_id.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    ) {
        write_feishu_generated_credentials(state, client_id, client_secret)?;
        if let Err(err) = start_service_if_needed(state, "feishud").await {
            let mut db = state
                .db
                .lock()
                .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
            return mark_pending_channel_bind_session_failed(&mut db, session.id, &err.to_string());
        }
        return Ok(session);
    }

    let Some(error_code) = poll
        .error
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(session);
    };
    let error_text = poll
        .error_description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|detail| format!("{error_code}: {detail}"))
        .unwrap_or_else(|| error_code.to_string());
    let mut db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    match error_code {
        "authorization_pending" | "slow_down" => Ok(session),
        "expired_token" => mark_pending_channel_bind_session_expired(&mut db, session.id),
        "access_denied" => {
            mark_pending_channel_bind_session_failed(&mut db, session.id, &error_text)
        }
        _ => mark_pending_channel_bind_session_failed(&mut db, session.id, &error_text),
    }
}

fn find_detectable_feishu_bind_session(
    db: &rusqlite::Connection,
    bind_token: Option<&str>,
) -> anyhow::Result<Option<PendingChannelBindSession>> {
    let Some(bind_token) = bind_token.map(str::trim).filter(|token| !token.is_empty()) else {
        return Ok(None);
    };
    get_pending_channel_bind_session_by_token(db, bind_token)
}

async fn start_feishu_bind_session_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<StartFeishuBindSessionRequest>,
) -> (
    StatusCode,
    Json<ApiResponse<FeishuBindSessionStatusResponse>>,
) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err((status, Json(resp))) => {
            return (
                status,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: resp.error,
                }),
            );
        }
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can start feishu binds".to_string()),
            }),
        );
    }

    let ttl_seconds = clamp_feishu_bind_ttl_seconds(req.expires_in_seconds);
    let default_expires_at = current_unix_ts()
        .saturating_add(ttl_seconds as i64)
        .to_string();
    let session = {
        let mut db = match state.db.lock() {
            Ok(db) => db,
            Err(_) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("db lock poisoned".to_string()),
                    }),
                );
            }
        };
        match create_pending_channel_bind_session(
            &mut db,
            "feishu",
            &identity.user_key,
            &default_expires_at,
        ) {
            Ok(session) => session,
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("create feishu bind session failed: {err}")),
                    }),
                );
            }
        }
    };

    let config = match load_feishu_config_response(&state, Some(&identity.user_key)) {
        Ok(config) => config,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read feishu config failed: {err}")),
                }),
            );
        }
    };
    if config.bind_ready {
        return (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(feishu_bind_session_response(&state, session)),
                error: None,
            }),
        );
    }

    let begin = match begin_feishu_official_registration(&state).await {
        Ok(begin) => begin,
        Err(err) => {
            let mut db = match state.db.lock() {
                Ok(db) => db,
                Err(_) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiResponse {
                            ok: false,
                            data: None,
                            error: Some("db lock poisoned".to_string()),
                        }),
                    );
                }
            };
            let _ = mark_pending_channel_bind_session_failed(&mut db, session.id, &err.to_string());
            return (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("start feishu official registration failed: {err}")),
                }),
            );
        }
    };
    let begin_expire_seconds = begin.expire_in.unwrap_or(ttl_seconds);
    let session_expires_at = current_unix_ts()
        .saturating_add(begin_expire_seconds.min(ttl_seconds) as i64)
        .to_string();
    let mut db = match state.db.lock() {
        Ok(db) => db,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("db lock poisoned".to_string()),
                }),
            );
        }
    };
    match attach_pending_channel_bind_session_install_flow(
        &mut db,
        session.id,
        &begin.device_code,
        &begin.verification_uri_complete,
        begin.interval.unwrap_or(5) as i64,
        &session_expires_at,
    ) {
        Ok(session) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(feishu_bind_session_response(&state, session)),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!(
                    "persist feishu official registration failed: {err}"
                )),
            }),
        ),
    }
}

async fn get_feishu_bind_session_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(session_id): AxumPath<i64>,
) -> (
    StatusCode,
    Json<ApiResponse<FeishuBindSessionStatusResponse>>,
) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err((status, Json(resp))) => {
            return (
                status,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: resp.error,
                }),
            );
        }
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can inspect feishu binds".to_string()),
            }),
        );
    }

    let session = {
        let mut db = match state.db.lock() {
            Ok(db) => db,
            Err(_) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("db lock poisoned".to_string()),
                    }),
                );
            }
        };
        match get_pending_channel_bind_session_by_id(&db, session_id) {
            Ok(Some(session)) => {
                if session.user_key != identity.user_key {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(ApiResponse {
                            ok: false,
                            data: None,
                            error: Some("feishu bind session not found".to_string()),
                        }),
                    );
                }
                match maybe_expire_feishu_bind_session(&mut db, session) {
                    Ok(session) => session,
                    Err(err) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ApiResponse {
                                ok: false,
                                data: None,
                                error: Some(format!("refresh feishu bind session failed: {err}")),
                            }),
                        );
                    }
                }
            }
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("feishu bind session not found".to_string()),
                    }),
                );
            }
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("get feishu bind session failed: {err}")),
                    }),
                );
            }
        }
    };

    match maybe_complete_feishu_official_scan(&state, session).await {
        Ok(session) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(feishu_bind_session_response(&state, session)),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("refresh feishu bind session failed: {err}")),
            }),
        ),
    }
}

async fn detect_feishu_bind_session_handler(
    State(state): State<AppState>,
    Json(req): Json<DetectFeishuBindSessionRequest>,
) -> (
    StatusCode,
    Json<ApiResponse<DetectFeishuBindSessionResponse>>,
) {
    let external_user_id = req.external_user_id.trim();
    let external_chat_id = req.external_chat_id.trim();
    let bind_token = req
        .bind_token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty());
    if external_user_id.is_empty() || external_chat_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("external_user_id and external_chat_id are required".to_string()),
            }),
        );
    }
    if bind_token.is_none() {
        return (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(DetectFeishuBindSessionResponse {
                    matched: false,
                    session: None,
                }),
                error: None,
            }),
        );
    }

    let mut db = match state.db.lock() {
        Ok(db) => db,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("db lock poisoned".to_string()),
                }),
            );
        }
    };
    let Some(session) = (match find_detectable_feishu_bind_session(&db, bind_token) {
        Ok(session) => session,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("load feishu bind session failed: {err}")),
                }),
            );
        }
    }) else {
        return (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(DetectFeishuBindSessionResponse {
                    matched: false,
                    session: None,
                }),
                error: None,
            }),
        );
    };

    let session = match maybe_expire_feishu_bind_session(&mut db, session) {
        Ok(session) => session,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("refresh feishu bind session failed: {err}")),
                }),
            );
        }
    };
    if session.status == "expired" {
        return (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(DetectFeishuBindSessionResponse {
                    matched: false,
                    session: Some(feishu_bind_session_response(&state, session)),
                }),
                error: None,
            }),
        );
    }

    let session = if session.status == "bound" {
        session
    } else {
        let detected = match mark_pending_channel_bind_session_detected(
            &mut db,
            session.id,
            external_user_id,
            external_chat_id,
        ) {
            Ok(session) => session,
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("detect feishu bind session failed: {err}")),
                    }),
                );
            }
        };
        match finalize_pending_channel_bind_session(&mut db, detected.id) {
            Ok(session) => session,
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("finalize feishu bind session failed: {err}")),
                    }),
                );
            }
        }
    };

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(DetectFeishuBindSessionResponse {
                matched: true,
                session: Some(feishu_bind_session_response(&state, session)),
            }),
            error: None,
        }),
    )
}

async fn update_auth_key_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(key_id): AxumPath<i64>,
    Json(req): Json<UpdateAuthKeyRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can update auth keys".to_string()),
            }),
        );
    }

    let role = req.role.as_deref();
    let role = role.map(str::trim).filter(|v| !v.is_empty());
    match update_auth_key_by_id(&state, key_id, role, req.enabled, &identity.user_key) {
        Ok(true) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({ "updated": true })),
                error: None,
            }),
        ),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("auth key not found".to_string()),
            }),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("update auth key failed: {err}")),
            }),
        ),
    }
}

async fn delete_auth_key_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(key_id): AxumPath<i64>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can delete auth keys".to_string()),
            }),
        );
    }

    match delete_auth_key_by_id(&state, key_id, &identity.user_key) {
        Ok(true) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({ "deleted": true })),
                error: None,
            }),
        ),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("auth key not found".to_string()),
            }),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("delete auth key failed: {err}")),
            }),
        ),
    }
}

async fn create_auth_key_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateAuthKeyRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can create auth keys".to_string()),
            }),
        );
    }
    match create_auth_key(&state, req.role.as_str()) {
        Ok(user_key) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({ "user_key": user_key })),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("create auth key failed: {err}")),
            }),
        ),
    }
}

fn ui_auth_error(message: &str) -> (StatusCode, Json<ApiResponse<Value>>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ApiResponse {
            ok: false,
            data: None,
            error: Some(message.to_string()),
        }),
    )
}

pub(crate) fn require_ui_identity(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthIdentity, (StatusCode, Json<ApiResponse<Value>>)> {
    let Some(raw_key) = headers
        .get("x-rustclaw-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return Err(ui_auth_error("Missing X-RustClaw-Key header"));
    };
    match resolve_auth_identity_by_key(state, raw_key) {
        Ok(Some(identity)) => Ok(identity),
        Ok(None) => Err(ui_auth_error("Invalid key")),
        Err(err) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("auth lookup failed: {err}")),
            }),
        )),
    }
}

#[derive(Debug, Deserialize)]
struct WebdInternalVerifyRequest {
    username: String,
    password: String,
}

#[derive(Debug, Deserialize)]
struct AdminWebdAccountRequest {
    username: String,
    password: String,
    #[serde(default)]
    key_id: Option<i64>,
    #[serde(default)]
    user_key: String,
}

async fn webd_internal_verify_login(
    State(state): State<AppState>,
    Json(req): Json<WebdInternalVerifyRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let db = match state.db.lock() {
        Ok(g) => g,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("db lock poisoned".to_string()),
                }),
            );
        }
    };
    match verify_webd_password_login(&db, &req.username, &req.password) {
        Ok(Some(user_key)) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({ "user_key": user_key })),
                error: None,
            }),
        ),
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("invalid username or password".to_string()),
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("login failed: {err}")),
            }),
        ),
    }
}

async fn admin_upsert_webd_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<AdminWebdAccountRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(id) => id,
        Err((status, Json(resp))) => {
            return (
                status,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: resp.error,
                }),
            );
        }
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can manage webd accounts".to_string()),
            }),
        );
    }
    let target_user_key = if let Some(key_id) = req.key_id {
        match get_auth_key_value_by_id(&state, key_id) {
            Ok(Some(user_key)) => user_key,
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("auth key not found".to_string()),
                    }),
                );
            }
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("load auth key failed: {err}")),
                    }),
                );
            }
        }
    } else {
        let user_key = req.user_key.trim().to_string();
        if user_key.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("key_id or user_key is required".to_string()),
                }),
            );
        }
        user_key
    };
    let db = match state.db.lock() {
        Ok(g) => g,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("db lock poisoned".to_string()),
                }),
            );
        }
    };
    match upsert_webd_login_account(&db, &req.username, &req.password, &target_user_key) {
        Ok(()) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({ "updated": true })),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(err.to_string()),
            }),
        ),
    }
}

async fn verify_ui_key(
    State(state): State<AppState>,
    Json(req): Json<UiKeyVerifyRequest>,
) -> (StatusCode, Json<ApiResponse<AuthIdentity>>) {
    match resolve_auth_identity_by_key(&state, &req.user_key) {
        Ok(Some(identity)) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(identity),
                error: None,
            }),
        ),
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("Invalid key".to_string()),
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("auth lookup failed: {err}")),
            }),
        ),
    }
}

async fn auth_me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<AuthIdentity>>) {
    match require_ui_identity(&state, &headers) {
        Ok(identity) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(identity),
                error: None,
            }),
        ),
        Err((status, Json(resp))) => (
            status,
            Json(ApiResponse {
                ok: resp.ok,
                data: None,
                error: resp.error,
            }),
        ),
    }
}

async fn resolve_channel_binding(
    State(state): State<AppState>,
    Json(req): Json<ResolveChannelBindingRequest>,
) -> (StatusCode, Json<ApiResponse<ResolveChannelBindingResponse>>) {
    match resolve_channel_binding_identity(
        &state,
        &scoped_channel_name(req.channel, req.telegram_bot_name.as_deref()),
        req.external_user_id.as_deref(),
        req.external_chat_id.as_deref(),
    ) {
        Ok(identity) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(ResolveChannelBindingResponse {
                    bound: identity.is_some(),
                    identity,
                }),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("resolve channel binding failed: {err}")),
            }),
        ),
    }
}

async fn bind_channel_key(
    State(state): State<AppState>,
    Json(req): Json<BindChannelKeyRequest>,
) -> (StatusCode, Json<ApiResponse<AuthIdentity>>) {
    match bind_channel_identity(
        &state,
        &scoped_channel_name(req.channel, req.telegram_bot_name.as_deref()),
        req.external_user_id.as_deref(),
        req.external_chat_id.as_deref(),
        &req.user_key,
    ) {
        Ok(Some(identity)) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(identity),
                error: None,
            }),
        ),
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("Invalid key".to_string()),
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("bind channel key failed: {err}")),
            }),
        ),
    }
}

async fn get_telegram_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<TelegramConfigResponse>>) {
    if let Err((status, Json(resp))) = require_ui_identity(&state, &headers) {
        return (
            status,
            Json(ApiResponse {
                ok: resp.ok,
                data: None,
                error: resp.error,
            }),
        );
    }
    let config_path = state.workspace_root.join("configs/config.toml");
    let config = match claw_core::config::AppConfig::load(&config_path.to_string_lossy()) {
        Ok(config) => config,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read telegram config failed: {err}")),
                }),
            );
        }
    };
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(TelegramConfigResponse {
                config_path: "configs/channels/telegram.toml".to_string(),
                bots: telegram_bots_from_config(&config),
                agents: agents_from_config(&config),
                restart_required: true,
            }),
            error: None,
        }),
    )
}

async fn update_telegram_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpdateTelegramConfigRequest>,
) -> (StatusCode, Json<ApiResponse<TelegramConfigResponse>>) {
    if let Err((status, Json(resp))) = require_ui_identity(&state, &headers) {
        return (
            status,
            Json(ApiResponse {
                ok: resp.ok,
                data: None,
                error: resp.error,
            }),
        );
    }

    let normalized_agents = match normalize_agent_items(&req.agents) {
        Ok(items) => items,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(err.to_string()),
                }),
            );
        }
    };
    let known_agent_ids = normalized_agents
        .iter()
        .map(|agent| agent.id.clone())
        .collect::<std::collections::HashSet<_>>();
    let normalized = match normalize_telegram_bot_items(&req.bots, &known_agent_ids) {
        Ok(items) => items,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(err.to_string()),
                }),
            );
        }
    };
    let config_path = state.workspace_root.join("configs/config.toml");
    let existing_config = match claw_core::config::AppConfig::load(&config_path.to_string_lossy()) {
        Ok(config) => config,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read telegram config failed: {err}")),
                }),
            );
        }
    };
    let existing_bot_tokens = telegram_bot_tokens_from_config(&existing_config);
    let effective_bots = normalized
        .iter()
        .cloned()
        .map(|mut bot| {
            if bot.bot_token.trim().is_empty() {
                if let Some(existing) = existing_bot_tokens.get(&bot.name) {
                    bot.bot_token = existing.clone();
                }
            }
            bot.bot_token_configured = !bot.bot_token.trim().is_empty();
            bot
        })
        .collect::<Vec<_>>();

    let mut value = match read_telegram_config_value(&state) {
        Ok(value) => value,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read telegram config failed: {err}")),
                }),
            );
        }
    };
    let primary = effective_bots.iter().find(|bot| bot.is_primary).cloned();
    let primary_bot_token_enabled = primary
        .as_ref()
        .map(|bot| !bot.bot_token.trim().is_empty())
        .unwrap_or(false);

    let extra_bots = effective_bots
        .iter()
        .filter(|bot| !bot.is_primary)
        .map(|bot| {
            let mut table = toml::map::Map::new();
            table.insert("name".to_string(), toml::Value::String(bot.name.clone()));
            table.insert(
                "bot_token".to_string(),
                toml::Value::String(bot.bot_token.clone()),
            );
            table.insert(
                "agent_id".to_string(),
                toml::Value::String(bot.agent_id.clone()),
            );
            table.insert(
                "allowlist".to_string(),
                toml::Value::Array(
                    bot.allowlist
                        .iter()
                        .copied()
                        .map(|id| toml::Value::Integer(id))
                        .collect(),
                ),
            );
            table.insert(
                "access_mode".to_string(),
                toml::Value::String(bot.access_mode.clone()),
            );
            table.insert(
                "allowed_usernames".to_string(),
                toml::Value::Array(
                    bot.allowed_telegram_usernames
                        .iter()
                        .cloned()
                        .map(toml::Value::String)
                        .collect(),
                ),
            );
            toml::Value::Table(table)
        })
        .collect::<Vec<_>>();
    {
        let telegram_table = match ensure_toml_table(&mut value, &["telegram"]) {
            Ok(table) => table,
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("prepare telegram config failed: {err}")),
                    }),
                );
            }
        };

        telegram_table.insert(
            "bot_token".to_string(),
            toml::Value::String(
                primary
                    .as_ref()
                    .map(|bot| bot.bot_token.clone())
                    .unwrap_or_default(),
            ),
        );
        telegram_table.insert(
            "agent_id".to_string(),
            toml::Value::String(
                primary
                    .as_ref()
                    .map(|bot| bot.agent_id.clone())
                    .unwrap_or_else(|| "main".to_string()),
            ),
        );
        telegram_table.insert(
            "allowlist".to_string(),
            toml::Value::Array(
                primary
                    .as_ref()
                    .map(|bot| bot.allowlist.as_slice())
                    .unwrap_or(&[])
                    .iter()
                    .copied()
                    .map(|id| toml::Value::Integer(id))
                    .collect(),
            ),
        );
        telegram_table.insert(
            "access_mode".to_string(),
            toml::Value::String(
                primary
                    .as_ref()
                    .map(|bot| bot.access_mode.clone())
                    .unwrap_or_else(|| "public".to_string()),
            ),
        );
        telegram_table.insert(
            "allowed_usernames".to_string(),
            toml::Value::Array(
                primary
                    .as_ref()
                    .map(|bot| bot.allowed_telegram_usernames.as_slice())
                    .unwrap_or(&[])
                    .iter()
                    .cloned()
                    .map(toml::Value::String)
                    .collect(),
            ),
        );
        telegram_table.insert("bots".to_string(), toml::Value::Array(extra_bots));
    }

    let telegram_bot_table = match ensure_toml_table(&mut value, &["telegram_bot"]) {
        Ok(table) => table,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("prepare telegram compat config failed: {err}")),
                }),
            );
        }
    };
    telegram_bot_table.insert(
        "enabled".to_string(),
        toml::Value::Boolean(primary_bot_token_enabled),
    );
    if let Some(root_table) = value.as_table_mut() {
        root_table.insert(
            "agents".to_string(),
            toml::Value::Array(
                normalized_agents
                    .iter()
                    .map(|agent| {
                        let mut table = toml::map::Map::new();
                        table.insert("id".to_string(), toml::Value::String(agent.id.clone()));
                        table.insert("name".to_string(), toml::Value::String(agent.name.clone()));
                        if !agent.description.trim().is_empty() {
                            table.insert(
                                "description".to_string(),
                                toml::Value::String(agent.description.clone()),
                            );
                        }
                        table.insert(
                            "persona_prompt".to_string(),
                            toml::Value::String(agent.persona_prompt.clone()),
                        );
                        if let Some(vendor) = agent.preferred_vendor.as_ref() {
                            table.insert(
                                "preferred_vendor".to_string(),
                                toml::Value::String(vendor.clone()),
                            );
                        }
                        if let Some(model) = agent.preferred_model.as_ref() {
                            table.insert(
                                "preferred_model".to_string(),
                                toml::Value::String(model.clone()),
                            );
                        }
                        table.insert(
                            "allowed_skills".to_string(),
                            toml::Value::Array(
                                agent
                                    .allowed_skills
                                    .iter()
                                    .map(|skill| toml::Value::String(skill.clone()))
                                    .collect(),
                            ),
                        );
                        toml::Value::Table(table)
                    })
                    .collect(),
            ),
        );
    }

    let output = match toml::to_string_pretty(&value) {
        Ok(output) => output,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("serialize telegram config failed: {err}")),
                }),
            );
        }
    };
    if let Err(err) = write_workspace_and_mounted_file(
        &state.workspace_root,
        "configs/channels/telegram.toml",
        &output,
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write telegram config failed: {err}")),
            }),
        );
    }

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(TelegramConfigResponse {
                config_path: "configs/channels/telegram.toml".to_string(),
                bots: telegram_bots_from_config(&existing_config),
                agents: normalized_agents,
                restart_required: true,
            }),
            error: None,
        }),
    )
}

fn load_wechat_config_response(state: &AppState) -> anyhow::Result<WechatConfigResponse> {
    let value = read_wechat_config_value(state)?;
    let wechat = value
        .get("wechat")
        .and_then(|v| v.as_table())
        .cloned()
        .unwrap_or_default();
    let session_path = state.workspace_root.join("data/wechatd/session.json");
    let bot_token = wechat
        .get("bot_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    Ok(WechatConfigResponse {
        config_path: "configs/channels/wechat.toml".to_string(),
        enabled: wechat
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        listen: wechat
            .get("listen")
            .and_then(|v| v.as_str())
            .unwrap_or("0.0.0.0:8792")
            .to_string(),
        clawd_base_url: wechat
            .get("clawd_base_url")
            .and_then(|v| v.as_str())
            .unwrap_or("http://127.0.0.1:8787")
            .to_string(),
        api_base_url: wechat
            .get("api_base_url")
            .and_then(|v| v.as_str())
            .unwrap_or("https://ilinkai.weixin.qq.com")
            .to_string(),
        wechat_uin_base64: wechat
            .get("wechat_uin_base64")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        request_timeout_seconds: wechat
            .get("request_timeout_seconds")
            .and_then(|v| v.as_integer())
            .map(|v| v.max(5) as u64)
            .unwrap_or(30),
        longpoll_timeout_ms: wechat
            .get("longpoll_timeout_ms")
            .and_then(|v| v.as_integer())
            .map(|v| v.max(1_000) as u64)
            .unwrap_or(35_000),
        text_chunk_chars: wechat
            .get("text_chunk_chars")
            .and_then(|v| v.as_integer())
            .map(|v| v.max(1) as usize)
            .unwrap_or(1200),
        bot_token_configured: !bot_token.is_empty() && bot_token != "REPLACE_ME",
        saved_session_present: session_path.exists(),
        restart_required: true,
    })
}

fn load_feishu_config_response(
    state: &AppState,
    current_user_key: Option<&str>,
) -> anyhow::Result<FeishuConfigResponse> {
    let value = read_feishu_config_value(state)?;
    let feishu = value
        .get("feishu")
        .and_then(|v| v.as_table())
        .cloned()
        .unwrap_or_default();
    let app_id = feishu
        .get("app_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let app_secret = feishu
        .get("app_secret")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let verification_token = feishu
        .get("verification_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let encrypt_key = feishu
        .get("encrypt_key")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let mode = feishu
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("long_connection")
        .trim()
        .to_string();
    let enabled = feishu
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(!app_id.is_empty() && !app_secret.is_empty());
    let current_key_bound = match current_user_key {
        Some(user_key) => has_channel_binding_for_user_key(state, "feishu", user_key)?,
        None => false,
    };
    Ok(FeishuConfigResponse {
        config_path: "configs/channels/feishu.toml".to_string(),
        enabled,
        mode,
        listen: feishu
            .get("listen")
            .and_then(|v| v.as_str())
            .unwrap_or("0.0.0.0:8789")
            .to_string(),
        clawd_base_url: feishu
            .get("clawd_base_url")
            .and_then(|v| v.as_str())
            .unwrap_or("http://127.0.0.1:8787")
            .to_string(),
        api_base_url: feishu
            .get("api_base_url")
            .and_then(|v| v.as_str())
            .unwrap_or("https://open.feishu.cn")
            .to_string(),
        app_id: app_id.clone(),
        app_secret: app_secret.clone(),
        verification_token_configured: !verification_token.is_empty(),
        encrypt_key_configured: !encrypt_key.is_empty(),
        bind_ready: !app_id.is_empty() && !app_secret.is_empty(),
        current_key_bound,
        restart_required: true,
    })
}

async fn get_wechat_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<WechatConfigResponse>>) {
    if let Err((status, Json(resp))) = require_ui_identity(&state, &headers) {
        return (
            status,
            Json(ApiResponse {
                ok: resp.ok,
                data: None,
                error: resp.error,
            }),
        );
    }
    match load_wechat_config_response(&state) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("read wechat config failed: {err}")),
            }),
        ),
    }
}

async fn get_feishu_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<FeishuConfigResponse>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err((status, Json(resp))) => {
            return (
                status,
                Json(ApiResponse {
                    ok: resp.ok,
                    data: None,
                    error: resp.error,
                }),
            );
        }
    };
    match load_feishu_config_response(&state, Some(&identity.user_key)) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("read feishu config failed: {err}")),
            }),
        ),
    }
}

async fn update_wechat_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpdateWechatConfigRequest>,
) -> (StatusCode, Json<ApiResponse<WechatConfigResponse>>) {
    if let Err((status, Json(resp))) = require_ui_identity(&state, &headers) {
        return (
            status,
            Json(ApiResponse {
                ok: resp.ok,
                data: None,
                error: resp.error,
            }),
        );
    }

    if req.listen.trim().is_empty()
        || req.clawd_base_url.trim().is_empty()
        || req.api_base_url.trim().is_empty()
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("listen, clawd_base_url, and api_base_url are required".to_string()),
            }),
        );
    }

    let mut value = match read_wechat_config_value(&state) {
        Ok(value) => value,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read wechat config failed: {err}")),
                }),
            );
        }
    };
    let wechat_table = match ensure_toml_table(&mut value, &["wechat"]) {
        Ok(table) => table,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("prepare wechat config failed: {err}")),
                }),
            );
        }
    };
    wechat_table.insert("enabled".to_string(), toml::Value::Boolean(req.enabled));
    wechat_table.insert(
        "listen".to_string(),
        toml::Value::String(req.listen.trim().to_string()),
    );
    wechat_table.insert(
        "clawd_base_url".to_string(),
        toml::Value::String(req.clawd_base_url.trim().to_string()),
    );
    wechat_table.insert(
        "api_base_url".to_string(),
        toml::Value::String(req.api_base_url.trim().to_string()),
    );
    wechat_table.insert(
        "wechat_uin_base64".to_string(),
        toml::Value::String(req.wechat_uin_base64.trim().to_string()),
    );
    wechat_table.insert(
        "request_timeout_seconds".to_string(),
        toml::Value::Integer(req.request_timeout_seconds.max(5) as i64),
    );
    wechat_table.insert(
        "longpoll_timeout_ms".to_string(),
        toml::Value::Integer(req.longpoll_timeout_ms.max(1_000) as i64),
    );
    wechat_table.insert(
        "text_chunk_chars".to_string(),
        toml::Value::Integer(req.text_chunk_chars.max(1) as i64),
    );
    if !wechat_table.contains_key("bot_token") {
        wechat_table.insert("bot_token".to_string(), toml::Value::String(String::new()));
    }

    let output = match toml::to_string_pretty(&value) {
        Ok(output) => output,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("serialize wechat config failed: {err}")),
                }),
            );
        }
    };
    if let Err(err) = write_workspace_and_mounted_file(
        &state.workspace_root,
        "configs/channels/wechat.toml",
        &output,
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write wechat config failed: {err}")),
            }),
        );
    }

    match load_wechat_config_response(&state) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("reload wechat config failed: {err}")),
            }),
        ),
    }
}

async fn update_feishu_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpdateFeishuConfigRequest>,
) -> (StatusCode, Json<ApiResponse<FeishuConfigResponse>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err((status, Json(resp))) => {
            return (
                status,
                Json(ApiResponse {
                    ok: resp.ok,
                    data: None,
                    error: resp.error,
                }),
            );
        }
    };

    let raw = match read_feishu_config_raw(&state) {
        Ok(raw) => raw,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read feishu config failed: {err}")),
                }),
            );
        }
    };

    let app_id = req.app_id.trim().to_string();
    let app_secret = req.app_secret.trim().to_string();
    let output = update_feishu_config_raw_preserving_format(&raw, &app_id, &app_secret);
    if let Err(err) = write_workspace_and_mounted_file(
        &state.workspace_root,
        "configs/channels/feishu.toml",
        &output,
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write feishu config failed: {err}")),
            }),
        );
    }

    match load_feishu_config_response(&state, Some(&identity.user_key)) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("reload feishu config failed: {err}")),
            }),
        ),
    }
}

async fn reset_feishu_config_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<FeishuConfigResponse>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err((status, Json(resp))) => {
            return (
                status,
                Json(ApiResponse {
                    ok: resp.ok,
                    data: None,
                    error: resp.error,
                }),
            );
        }
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can reset feishu config".to_string()),
            }),
        );
    }

    let raw = match read_feishu_config_raw(&state) {
        Ok(raw) => raw,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read feishu config failed: {err}")),
                }),
            );
        }
    };
    let output = reset_feishu_config_raw_preserving_format(&raw);
    if let Err(err) = write_workspace_and_mounted_file(
        &state.workspace_root,
        "configs/channels/feishu.toml",
        &output,
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write feishu config failed: {err}")),
            }),
        );
    }
    if let Err(err) = reset_channel_binding_state_for_user_key(&state, "feishu", &identity.user_key)
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("reset feishu bindings failed: {err}")),
            }),
        );
    }

    match load_feishu_config_response(&state, Some(&identity.user_key)) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("reload feishu config failed: {err}")),
            }),
        ),
    }
}

fn scoped_channel_name(
    channel: claw_core::types::ChannelKind,
    telegram_bot_name: Option<&str>,
) -> String {
    match channel {
        claw_core::types::ChannelKind::Telegram => telegram_bot_name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(|name| format!("telegram:{name}"))
            .unwrap_or_else(|| "telegram".to_string()),
        claw_core::types::ChannelKind::Whatsapp => "whatsapp".to_string(),
        claw_core::types::ChannelKind::Ui => "ui".to_string(),
        claw_core::types::ChannelKind::Wechat => "wechat".to_string(),
        claw_core::types::ChannelKind::Feishu => "feishu".to_string(),
        claw_core::types::ChannelKind::Lark => "lark".to_string(),
    }
}

async fn get_crypto_credentials(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Vec<ExchangeCredentialStatus>>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err((status, Json(resp))) => {
            return (
                status,
                Json(ApiResponse {
                    ok: resp.ok,
                    data: None,
                    error: resp.error,
                }),
            );
        }
    };
    match exchange_credential_status_for_user_key(&state, &identity.user_key) {
        Ok(mut statuses) => {
            for status in &mut statuses {
                status.api_key_masked = status.api_key_masked.as_deref().map(mask_secret);
            }
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(statuses),
                    error: None,
                }),
            )
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("read crypto credentials failed: {err}")),
            }),
        ),
    }
}

async fn upsert_crypto_credentials(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpsertExchangeCredentialRequest>,
) -> (StatusCode, Json<ApiResponse<ExchangeCredentialStatus>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err((status, Json(resp))) => {
            return (
                status,
                Json(ApiResponse {
                    ok: resp.ok,
                    data: None,
                    error: resp.error,
                }),
            );
        }
    };
    match upsert_exchange_credential_for_user_key(
        &state,
        &identity.user_key,
        &req.exchange,
        &req.api_key,
        &req.api_secret,
        req.passphrase.as_deref(),
    ) {
        Ok(status) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(ExchangeCredentialStatus {
                    exchange: status.exchange,
                    configured: status.configured,
                    api_key_masked: status.api_key_masked.as_deref().map(mask_secret),
                    updated_at: status.updated_at,
                }),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(err.to_string()),
            }),
        ),
    }
}

#[derive(Debug, serde::Deserialize, Default)]
struct LogsLatestQuery {
    file: Option<String>,
    lines: Option<usize>,
}

#[derive(Debug, serde::Deserialize, Default)]
struct RecentRobotTasksQuery {
    limit: Option<usize>,
}

#[derive(Debug, serde::Deserialize, Default)]
struct UsageRecordsQuery {
    page: Option<usize>,
    page_size: Option<usize>,
    search: Option<String>,
    channel: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct RecentRobotTaskSummary {
    task_id: String,
    status: String,
    kind: String,
    channel: String,
    telegram_bot_name: Option<String>,
    external_user_id: Option<String>,
    external_chat_id: Option<String>,
    request_text: Option<String>,
    result_text: Option<String>,
    error_text: Option<String>,
    created_at: Option<u64>,
    updated_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
struct UsageHistoryStats {
    total_requests: usize,
    success_requests: usize,
    failed_requests: usize,
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

#[derive(Debug, Clone, Serialize)]
struct UsageHistoryRecordSummary {
    record_id: String,
    task_id: String,
    ts: Option<u64>,
    channel: Option<String>,
    kind: Option<String>,
    task_status: Option<String>,
    telegram_bot_name: Option<String>,
    external_user_id: Option<String>,
    external_chat_id: Option<String>,
    request_text: Option<String>,
    vendor: Option<String>,
    provider: Option<String>,
    provider_type: Option<String>,
    model: Option<String>,
    model_kind: Option<String>,
    prompt_file: Option<String>,
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    total_tokens: Option<u64>,
    llm_call_count: usize,
    status: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct UsageHistoryRecordDetail {
    #[serde(flatten)]
    summary: UsageHistoryRecordSummary,
    entries: Vec<UsageHistoryChainEntry>,
}

#[derive(Debug, Clone, Serialize)]
struct UsageHistoryChainEntry {
    ts: Option<u64>,
    vendor: Option<String>,
    provider: Option<String>,
    provider_type: Option<String>,
    model: Option<String>,
    model_kind: Option<String>,
    status: Option<String>,
    prompt_file: Option<String>,
    prompt: Option<String>,
    request_payload: Option<Value>,
    raw_response: Option<String>,
    clean_response: Option<String>,
    error: Option<String>,
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
struct UsageHistoryPage {
    page: usize,
    page_size: usize,
    total_records: usize,
    total_pages: usize,
}

#[derive(Debug, Clone, Serialize)]
struct SkillListItem {
    name: String,
    description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TaskDebugUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    total_tokens: Option<u64>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
    cached_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TaskDebugEntry {
    ts: Option<u64>,
    task_id: Option<String>,
    vendor: Option<String>,
    provider: Option<String>,
    provider_type: Option<String>,
    model: Option<String>,
    model_kind: Option<String>,
    status: Option<String>,
    prompt_file: Option<String>,
    prompt: Option<String>,
    request_payload: Option<Value>,
    response: Option<String>,
    raw_response: Option<String>,
    clean_response: Option<String>,
    sanitized: Option<bool>,
    error: Option<String>,
    usage: Option<TaskDebugUsage>,
}

#[derive(Debug, Clone)]
struct UsageTaskMeta {
    channel: String,
    kind: String,
    task_status: String,
    user_key: Option<String>,
    external_user_id: Option<String>,
    external_chat_id: Option<String>,
    telegram_bot_name: Option<String>,
    request_text: Option<String>,
}

fn normalize_log_file_name(raw: Option<&str>) -> String {
    let fallback = "agent_trace.log".to_string();
    let candidate = raw.unwrap_or("").trim();
    if candidate.is_empty() {
        return fallback;
    }
    let allowed = [
        "agent_trace.log",
        "model_io.log",
        "routing.log",
        "act_plan.log",
        "clawd.log",
        "channel-gateway.log",
        "telegramd.log",
        "whatsappd.log",
        "whatsapp_webd.log",
    ];
    if allowed.iter().any(|v| v.eq_ignore_ascii_case(candidate)) {
        return candidate.to_string();
    }
    fallback
}

fn read_last_lines(path: &std::path::Path, limit_lines: usize) -> anyhow::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let total_size = file.metadata()?.len();
    let max_tail_bytes: u64 = 512 * 1024;
    let read_from = total_size.saturating_sub(max_tail_bytes);
    if read_from > 0 {
        file.seek(SeekFrom::Start(read_from))?;
    }
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    let content = String::from_utf8_lossy(&buf);
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Ok(String::new());
    }
    let start = lines.len().saturating_sub(limit_lines);
    Ok(lines[start..].join("\n"))
}

fn canonical_bound_channel_name(raw: &str) -> String {
    let channel = raw.trim().to_ascii_lowercase();
    match channel.as_str() {
        "" => String::new(),
        "telegram_bot" => "telegram".to_string(),
        "whatsapp_cloud" | "whatsapp-cloud" | "whatsapp_web" | "whatsapp-web" | "wa_cloud"
        | "wa-cloud" | "wa_web" | "wa-web" => "whatsapp".to_string(),
        "wechat_bot" | "openclaw-weixin" | "weixin" => "wechat".to_string(),
        other => other.to_string(),
    }
}

fn auth_user_summary_counts(state: &AppState) -> anyhow::Result<(usize, usize, Vec<String>)> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    let user_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM auth_keys WHERE enabled = 1",
        [],
        |row| row.get(0),
    )?;
    let mut stmt = db.prepare(
        "SELECT DISTINCT channel FROM channel_bindings WHERE TRIM(COALESCE(channel, '')) != ''",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut bound_channels = Vec::new();
    for row in rows {
        let channel = canonical_bound_channel_name(&row?);
        if !channel.is_empty() && !bound_channels.iter().any(|existing| existing == &channel) {
            bound_channels.push(channel);
        }
    }
    let channel_order = |channel: &str| match channel {
        "telegram" => 0,
        "whatsapp" => 1,
        "wechat" => 2,
        "feishu" => 3,
        "lark" => 4,
        "ui" => 5,
        _ => 99,
    };
    bound_channels.sort_by(|a, b| {
        channel_order(a)
            .cmp(&channel_order(b))
            .then_with(|| a.cmp(b))
    });
    let bound_channel_count = bound_channels.len();
    Ok((
        user_count.max(0) as usize,
        bound_channel_count,
        bound_channels,
    ))
}

async fn logs_latest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<LogsLatestQuery>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let file_name = normalize_log_file_name(query.file.as_deref());
    let lines = query.lines.unwrap_or(200).clamp(20, 2000);
    let path = state.workspace_root.join("logs").join(&file_name);
    let raw = match read_last_lines(&path, lines) {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read log failed: {err}")),
                }),
            );
        }
    };
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "file": file_name,
                "lines": lines,
                "text": raw,
            })),
            error: None,
        }),
    )
}

fn channel_allows_shared_ui_task_access(channel: &str) -> bool {
    matches!(
        channel,
        "telegram" | "whatsapp" | "wechat" | "feishu" | "lark"
    )
}

fn task_access_meta_for_debug(
    state: &AppState,
    task_id: &str,
) -> anyhow::Result<Option<(Option<String>, String)>> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    db.query_row(
        "SELECT user_key, channel FROM tasks WHERE task_id = ?1 LIMIT 1",
        [task_id],
        |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
    )
    .optional()
    .map_err(Into::into)
}

fn preview_text(raw: &str, limit: usize) -> Option<String> {
    let normalized = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut preview = String::new();
    let mut count = 0usize;
    for ch in trimmed.chars() {
        if count >= limit {
            break;
        }
        preview.push(ch);
        count += 1;
    }
    if trimmed.chars().count() > limit {
        preview.push_str("...");
    }
    Some(preview)
}

fn preview_text_from_json(raw: Option<&str>, preferred_keys: &[&str]) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let value = serde_json::from_str::<Value>(raw).ok()?;
    for key in preferred_keys {
        if let Some(text) = value.get(*key).and_then(|v| v.as_str()) {
            if let Some(preview) = preview_text(text, 180) {
                return Some(preview);
            }
        }
    }
    if let Some(text) = value.as_str() {
        return preview_text(text, 180);
    }
    None
}

fn payload_telegram_bot_name(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let value = serde_json::from_str::<Value>(raw).ok()?;
    value
        .get("telegram_bot_name")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn payload_request_text(raw: Option<&str>) -> Option<String> {
    preview_text_from_json(raw, &["text"])
}

fn usage_record_visible_to_identity(identity: &AuthIdentity, meta: &UsageTaskMeta) -> bool {
    if meta.channel == "ui" {
        let expected_key = meta
            .user_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        return expected_key == Some(identity.user_key.trim());
    }
    channel_allows_shared_ui_task_access(&meta.channel)
}

fn usage_chain_entry_from_entry(entry: &TaskDebugEntry) -> UsageHistoryChainEntry {
    let prompt_tokens = entry
        .usage
        .as_ref()
        .and_then(|usage| usage.prompt_tokens.or(usage.input_tokens));
    let completion_tokens = entry
        .usage
        .as_ref()
        .and_then(|usage| usage.completion_tokens.or(usage.output_tokens));
    let total_tokens = entry.usage.as_ref().and_then(|usage| usage.total_tokens);
    UsageHistoryChainEntry {
        ts: entry.ts,
        vendor: entry.vendor.clone(),
        provider: entry.provider.clone(),
        provider_type: entry.provider_type.clone(),
        model: entry.model.clone(),
        model_kind: entry.model_kind.clone(),
        prompt_file: entry.prompt_file.clone(),
        prompt_tokens,
        completion_tokens,
        total_tokens,
        status: entry.status.clone(),
        error: entry.error.clone(),
        prompt: entry.prompt.clone(),
        request_payload: entry.request_payload.clone(),
        raw_response: entry.raw_response.clone(),
        clean_response: entry.clean_response.clone().or(entry.response.clone()),
    }
}

fn summarize_usage_task(
    task_id: String,
    meta: UsageTaskMeta,
    entries: &[TaskDebugEntry],
) -> UsageHistoryRecordSummary {
    let mut prompt_tokens = 0u64;
    let mut completion_tokens = 0u64;
    let mut total_tokens = 0u64;
    let mut latest_entry: Option<&TaskDebugEntry> = None;
    for entry in entries {
        let chain_entry = usage_chain_entry_from_entry(entry);
        prompt_tokens += chain_entry.prompt_tokens.unwrap_or(0);
        completion_tokens += chain_entry.completion_tokens.unwrap_or(0);
        total_tokens += chain_entry.total_tokens.unwrap_or_else(|| {
            chain_entry.prompt_tokens.unwrap_or(0) + chain_entry.completion_tokens.unwrap_or(0)
        });
        let replace = latest_entry
            .map(|current| entry.ts.unwrap_or(0) >= current.ts.unwrap_or(0))
            .unwrap_or(true);
        if replace {
            latest_entry = Some(entry);
        }
    }
    let latest = latest_entry.cloned().unwrap_or(TaskDebugEntry {
        ts: None,
        task_id: Some(task_id.clone()),
        vendor: None,
        provider: None,
        provider_type: None,
        model: None,
        model_kind: None,
        status: None,
        prompt_file: None,
        prompt: None,
        request_payload: None,
        response: None,
        raw_response: None,
        clean_response: None,
        sanitized: None,
        error: None,
        usage: None,
    });
    UsageHistoryRecordSummary {
        record_id: task_id.clone(),
        task_id,
        ts: latest.ts,
        channel: Some(meta.channel),
        kind: Some(meta.kind),
        task_status: Some(meta.task_status),
        telegram_bot_name: meta.telegram_bot_name,
        external_user_id: meta.external_user_id,
        external_chat_id: meta.external_chat_id,
        request_text: meta.request_text,
        vendor: latest.vendor,
        provider: latest.provider,
        provider_type: latest.provider_type,
        model: latest.model,
        model_kind: latest.model_kind,
        prompt_file: latest.prompt_file,
        prompt_tokens: Some(prompt_tokens),
        completion_tokens: Some(completion_tokens),
        total_tokens: Some(total_tokens),
        llm_call_count: entries.len(),
        status: latest.status,
        error: latest.error,
    }
}

fn usage_stats_add(stats: &mut UsageHistoryStats, record: &UsageHistoryRecordSummary) {
    stats.total_requests += 1;
    if record.status.as_deref() == Some("ok") {
        stats.success_requests += 1;
    } else {
        stats.failed_requests += 1;
    }
    stats.prompt_tokens += record.prompt_tokens.unwrap_or(0);
    stats.completion_tokens += record.completion_tokens.unwrap_or(0);
    stats.total_tokens += record.total_tokens.unwrap_or_else(|| {
        record.prompt_tokens.unwrap_or(0) + record.completion_tokens.unwrap_or(0)
    });
}

fn usage_channel_matches(query_channel: Option<&str>, record: &UsageHistoryRecordSummary) -> bool {
    let Some(query_channel) = query_channel
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return true;
    };
    record.channel.as_deref().unwrap_or_default() == query_channel
}

fn usage_status_matches(query_status: Option<&str>, record: &UsageHistoryRecordSummary) -> bool {
    let Some(query_status) = query_status
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return true;
    };
    match query_status {
        "success" => record.status.as_deref() == Some("ok"),
        "failed" => record.status.as_deref() != Some("ok"),
        _ => true,
    }
}

fn usage_search_matches(query: Option<&str>, record: &UsageHistoryRecordSummary) -> bool {
    let Some(query) = query.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    let query = query.to_lowercase();
    let haystack = [
        Some(record.task_id.as_str()),
        record.request_text.as_deref(),
        record.model.as_deref(),
        record.vendor.as_deref(),
        record.provider.as_deref(),
        record.telegram_bot_name.as_deref(),
        record.external_user_id.as_deref(),
        record.external_chat_id.as_deref(),
        record.prompt_file.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
    .to_lowercase();
    haystack.contains(&query)
}

fn task_usage_meta(state: &AppState, task_id: &str) -> anyhow::Result<Option<UsageTaskMeta>> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    db.query_row(
        "SELECT channel, kind, status, user_key, external_user_id, external_chat_id, payload_json
         FROM tasks
         WHERE task_id = ?1
         LIMIT 1",
        [task_id],
        |row| {
            let payload_json: Option<String> = row.get(6)?;
            Ok(UsageTaskMeta {
                channel: row.get(0)?,
                kind: row.get(1)?,
                task_status: row.get(2)?,
                user_key: row.get(3)?,
                external_user_id: row.get(4)?,
                external_chat_id: row.get(5)?,
                telegram_bot_name: payload_telegram_bot_name(payload_json.as_deref()),
                request_text: payload_request_text(payload_json.as_deref()),
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

async fn recent_robot_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<RecentRobotTasksQuery>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let limit = query.limit.unwrap_or(12).clamp(1, 50);

    let read_result = (|| -> anyhow::Result<Vec<RecentRobotTaskSummary>> {
        let db = state
            .db
            .lock()
            .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
        let mut stmt = db.prepare(
            "SELECT task_id, status, kind, channel, external_user_id, external_chat_id, payload_json, result_json, error_text,
                    CAST(NULLIF(created_at, '') AS INTEGER) AS created_ts,
                    CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) AS updated_ts
             FROM tasks
             WHERE channel IN ('telegram', 'whatsapp', 'wechat', 'feishu', 'lark')
             ORDER BY updated_ts DESC
             LIMIT ?1",
        )?;

        let rows = stmt.query_map(rusqlite::params![limit as i64], |row| {
            let payload_json: Option<String> = row.get(6)?;
            let result_json: Option<String> = row.get(7)?;
            Ok(RecentRobotTaskSummary {
                task_id: row.get(0)?,
                status: row.get(1)?,
                kind: row.get(2)?,
                channel: row.get(3)?,
                external_user_id: row.get(4)?,
                external_chat_id: row.get(5)?,
                telegram_bot_name: payload_telegram_bot_name(payload_json.as_deref()),
                request_text: preview_text_from_json(payload_json.as_deref(), &["text"]),
                result_text: preview_text_from_json(result_json.as_deref(), &["text"]),
                error_text: row.get(8)?,
                created_at: row.get::<_, Option<i64>>(9)?.map(|v| v.max(0) as u64),
                updated_at: row.get::<_, Option<i64>>(10)?.map(|v| v.max(0) as u64),
            })
        })?;

        let mut items = Vec::new();
        for row in rows {
            items.push(row?);
        }
        Ok(items)
    })();

    match read_result {
        Ok(tasks) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({ "tasks": tasks })),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("read recent robot tasks failed: {err}")),
            }),
        ),
    }
}

async fn usage_records(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<UsageRecordsQuery>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    let page_size = query.page_size.unwrap_or(20).clamp(10, 100);
    let page = query.page.unwrap_or(1).max(1);
    let search = query.search.as_deref();
    let channel = query.channel.as_deref().filter(|value| *value != "all");
    let status = query.status.as_deref().filter(|value| *value != "all");
    let log_path = state.workspace_root.join("logs").join("model_io.log");
    if !log_path.exists() {
        return (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({
                    "stats": UsageHistoryStats {
                        total_requests: 0,
                        success_requests: 0,
                        failed_requests: 0,
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        total_tokens: 0,
                    },
                    "records": Vec::<UsageHistoryRecordSummary>::new(),
                    "pagination": UsageHistoryPage {
                        page,
                        page_size,
                        total_records: 0,
                        total_pages: 0,
                    },
                })),
                error: None,
            }),
        );
    }

    let file = match std::fs::File::open(&log_path) {
        Ok(file) => file,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("open usage log failed: {err}")),
                }),
            );
        }
    };
    let reader = std::io::BufReader::new(file);
    let mut meta_cache: HashMap<String, Option<UsageTaskMeta>> = HashMap::new();
    let mut tasks_by_id: HashMap<String, (UsageTaskMeta, Vec<TaskDebugEntry>)> = HashMap::new();
    let mut stats = UsageHistoryStats {
        total_requests: 0,
        success_requests: 0,
        failed_requests: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
    };
    for line in reader.lines() {
        let Ok(line) = line else {
            continue;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<TaskDebugEntry>(trimmed) else {
            continue;
        };
        let Some(task_id) = entry
            .task_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string)
        else {
            continue;
        };
        let meta = if let Some(existing) = meta_cache.get(&task_id) {
            existing.clone()
        } else {
            let loaded = match task_usage_meta(&state, &task_id) {
                Ok(value) => value,
                Err(err) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiResponse {
                            ok: false,
                            data: None,
                            error: Some(format!("load usage task meta failed: {err}")),
                        }),
                    );
                }
            };
            meta_cache.insert(task_id.clone(), loaded.clone());
            loaded
        };
        let Some(meta) = meta else {
            continue;
        };
        if !usage_record_visible_to_identity(&identity, &meta) {
            continue;
        }
        tasks_by_id
            .entry(task_id)
            .and_modify(|(_, entries)| entries.push(entry.clone()))
            .or_insert_with(|| (meta, vec![entry]));
    }
    let mut matched_records = Vec::new();
    for (task_id, (meta, mut entries)) in tasks_by_id {
        entries.sort_by(|a, b| (a.ts.unwrap_or(0)).cmp(&b.ts.unwrap_or(0)));
        let summary = summarize_usage_task(task_id, meta, &entries);
        if !usage_channel_matches(channel, &summary) {
            continue;
        }
        if !usage_status_matches(status, &summary) {
            continue;
        }
        if !usage_search_matches(search, &summary) {
            continue;
        }
        usage_stats_add(&mut stats, &summary);
        matched_records.push(summary);
    }
    matched_records.sort_by(|a, b| (b.ts.unwrap_or(0)).cmp(&a.ts.unwrap_or(0)));
    let total_records = matched_records.len();
    let total_pages = if total_records == 0 {
        0
    } else {
        total_records.div_ceil(page_size)
    };
    let safe_page = if total_pages == 0 {
        1
    } else {
        page.min(total_pages)
    };
    let start = (safe_page.saturating_sub(1)) * page_size;
    let records = matched_records
        .into_iter()
        .skip(start)
        .take(page_size)
        .collect::<Vec<_>>();

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "stats": stats,
                "records": records,
                "pagination": UsageHistoryPage {
                    page: safe_page,
                    page_size,
                    total_records,
                    total_pages,
                },
            })),
            error: None,
        }),
    )
}

async fn usage_record_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(task_id): AxumPath<String>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    let task_id = task_id.trim();
    if task_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("invalid task id".to_string()),
            }),
        );
    }
    let meta = match task_usage_meta(&state, task_id) {
        Ok(Some(meta)) => meta,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("usage record not found".to_string()),
                }),
            );
        }
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("load usage task meta failed: {err}")),
                }),
            );
        }
    };
    if !usage_record_visible_to_identity(&identity, &meta) {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("usage record access denied".to_string()),
            }),
        );
    }

    let mut entries = match read_task_debug_entries(&state, task_id) {
        Ok(entries) => entries,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read usage chain failed: {err}")),
                }),
            );
        }
    };
    if entries.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("usage record detail not found".to_string()),
            }),
        );
    }
    entries.sort_by(|a, b| (a.ts.unwrap_or(0)).cmp(&b.ts.unwrap_or(0)));
    let summary = summarize_usage_task(task_id.to_string(), meta, &entries);
    let record = UsageHistoryRecordDetail {
        summary,
        entries: entries.iter().map(usage_chain_entry_from_entry).collect(),
    };

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!(record)),
            error: None,
        }),
    )
}

fn read_task_debug_entries(state: &AppState, task_id: &str) -> anyhow::Result<Vec<TaskDebugEntry>> {
    let path = state.workspace_root.join("logs").join("model_io.log");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let mut entries = Vec::new();
    for line in reader.lines() {
        let Ok(line) = line else {
            continue;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<TaskDebugEntry>(trimmed) else {
            continue;
        };
        if entry.task_id.as_deref() == Some(task_id) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

async fn task_debug_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(task_id): AxumPath<String>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    let normalized_task_id = task_id.trim();
    if normalized_task_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("task_id is required".to_string()),
            }),
        );
    }
    let Some((task_user_key, channel)) =
        (match task_access_meta_for_debug(&state, normalized_task_id) {
            Ok(value) => value,
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("read task owner failed: {err}")),
                    }),
                );
            }
        })
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("Task not found".to_string()),
            }),
        );
    };
    let expected_key = task_user_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    if !channel_allows_shared_ui_task_access(&channel)
        && expected_key.is_some()
        && identity.user_key.trim() != expected_key.unwrap_or_default()
    {
        return ui_auth_error("Task owner mismatch");
    }
    let entries = match read_task_debug_entries(&state, normalized_task_id) {
        Ok(entries) => entries,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read task debug failed: {err}")),
                }),
            );
        }
    };
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "task_id": normalized_task_id,
                "entries": entries,
            })),
            error: None,
        }),
    )
}

fn shell_escape_arg(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

fn parse_service_action(raw: &str) -> Option<ServiceAction> {
    match raw {
        "start" => Some(ServiceAction::Start),
        "stop" => Some(ServiceAction::Stop),
        "restart" => Some(ServiceAction::Restart),
        _ => None,
    }
}

fn service_start_script(service: &str) -> Option<&'static str> {
    match service {
        "channel-gateway" | "channel_gateway" => Some("start-channel-gateway.sh"),
        "telegramd" => Some("start-telegramd.sh"),
        "whatsappd" => Some("start-whatsappd.sh"),
        "whatsapp_webd" => Some("start-whatsapp-webd.sh"),
        "wechatd" => Some("start-wechatd.sh"),
        "feishud" => Some("start-feishud.sh"),
        "larkd" => Some("start-larkd.sh"),
        _ => None,
    }
}

fn service_process_name(service: &str) -> Option<&'static str> {
    match service {
        "channel-gateway" | "channel_gateway" => Some("channel-gateway"),
        "telegramd" => Some("telegramd"),
        "whatsappd" => Some("whatsappd"),
        "whatsapp_webd" => Some("whatsapp_webd"),
        "wechatd" => Some("wechatd"),
        "feishud" => Some("feishud"),
        "larkd" => Some("larkd"),
        _ => None,
    }
}

fn service_pid_file(service: &str) -> Option<&'static str> {
    match service {
        "channel-gateway" | "channel_gateway" => Some("channel-gateway.pid"),
        "telegramd" => Some("telegramd.pid"),
        "whatsappd" => Some("whatsappd.pid"),
        "whatsapp_webd" => Some("whatsapp_webd.pid"),
        "wechatd" => Some("wechatd.pid"),
        "feishud" => Some("feishud.pid"),
        "larkd" => Some("larkd.pid"),
        _ => None,
    }
}

fn service_direct_process_count(service: &str) -> Option<usize> {
    match service {
        "channel-gateway" | "channel_gateway" => {
            channel_gateway_process_stats().map(|(count, _)| count)
        }
        "telegramd" => telegramd_process_stats().map(|(count, _)| count),
        "whatsappd" => whatsappd_process_stats().map(|(count, _)| count),
        "whatsapp_webd" => wa_webd_process_stats().map(|(count, _)| count),
        "wechatd" => wechatd_process_stats().map(|(count, _)| count),
        "feishud" => feishud_process_stats().map(|(count, _)| count),
        "larkd" => larkd_process_stats().map(|(count, _)| count),
        _ => None,
    }
}

fn service_is_gateway_managed(service: &str) -> bool {
    matches!(
        service,
        "telegramd" | "whatsappd" | "whatsapp_webd" | "feishud" | "larkd"
    ) && matches!(service_direct_process_count(service), Some(0) | None)
        && matches!(channel_gateway_process_stats(), Some((count, _)) if count > 0)
}

fn service_extra_process_names_on_stop(service: &str) -> &'static [&'static str] {
    match service {
        "whatsapp_webd" => &["services/wa-web-bridge/index.js", "wa-web-bridge/index.js"],
        _ => &[],
    }
}

fn service_is_running(service: &str) -> bool {
    match service {
        "channel-gateway" | "channel_gateway" => channel_gateway_process_stats()
            .map(|(count, _)| count > 0)
            .unwrap_or(false),
        "telegramd" => {
            let channel_gateway_running = channel_gateway_process_stats()
                .map(|(count, _)| count > 0)
                .unwrap_or(false);
            let legacy_telegramd_running = telegramd_process_stats()
                .map(|(count, _)| count > 0)
                .unwrap_or(false);
            channel_gateway_running || legacy_telegramd_running
        }
        "whatsappd" => whatsappd_process_stats()
            .map(|(count, _)| count > 0)
            .unwrap_or(false),
        "whatsapp_webd" => wa_webd_process_stats()
            .map(|(count, _)| count > 0)
            .unwrap_or(false),
        "wechatd" => wechatd_process_stats()
            .map(|(count, _)| count > 0)
            .unwrap_or(false),
        "feishud" => feishud_process_stats()
            .map(|(count, _)| count > 0)
            .unwrap_or(false),
        "larkd" => larkd_process_stats()
            .map(|(count, _)| count > 0)
            .unwrap_or(false),
        _ => false,
    }
}

fn runtime_profile_default() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

fn spawn_background_shell(cmd: &str) -> std::io::Result<()> {
    Command::new("bash")
        .arg("-lc")
        .arg(cmd)
        .stdin(StdProcessStdio::null())
        .stdout(StdProcessStdio::null())
        .stderr(StdProcessStdio::null())
        .spawn()?;
    Ok(())
}

fn validate_service_start_readiness(state: &AppState, service: &str) -> Result<(), String> {
    match service {
        "feishud" => {
            let config = load_feishu_config_response(state, None)
                .map_err(|err| format!("read feishu config failed: {err}"))?;
            if !config.enabled {
                return Err("service disabled".to_string());
            }
            if config.app_id.trim().is_empty() || config.app_secret.trim().is_empty() {
                return Err("feishu app_id/app_secret are required".to_string());
            }
            if config.mode.eq_ignore_ascii_case("webhook")
                && !config.verification_token_configured
                && !config.encrypt_key_configured
            {
                return Err(
                    "feishu webhook mode requires verification_token or encrypt_key".to_string(),
                );
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

async fn control_service(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((service, action)): AxumPath<(String, String)>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let action = match parse_service_action(action.trim()) {
        Some(v) => v,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("action must be start, stop, or restart".to_string()),
                }),
            );
        }
    };

    if service_start_script(service.as_str()).is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("unsupported service".to_string()),
            }),
        );
    }

    match action {
        ServiceAction::Start => {
            if let Err(err) = validate_service_start_readiness(&state, service.as_str()) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(err),
                    }),
                );
            }
            if service_is_running(service.as_str()) {
                return (
                    StatusCode::OK,
                    Json(ApiResponse {
                        ok: true,
                        data: Some(json!({
                            "service": service,
                            "action": "start",
                            "status": "already_running"
                        })),
                        error: None,
                    }),
                );
            }
            let profile = std::env::var("RUSTCLAW_START_PROFILE")
                .ok()
                .filter(|v| matches!(v.as_str(), "debug" | "release"))
                .unwrap_or_else(|| runtime_profile_default().to_string());
            let Some(script_name) = service_start_script(service.as_str()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("unsupported service".to_string()),
                    }),
                );
            };
            let workspace = state.workspace_root.to_string_lossy();
            let log_file = format!("logs/{}.log", service);
            let cmd = format!(
                "cd {} && mkdir -p logs .pids && nohup ./{} {} > {} 2>&1 &",
                shell_escape_arg(workspace.as_ref()),
                script_name,
                shell_escape_arg(profile.as_str()),
                shell_escape_arg(log_file.as_str())
            );
            if let Err(err) = spawn_background_shell(&cmd) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("failed to start service process: {err}")),
                    }),
                );
            }
            // The start command may return success even if script preflight exits quickly
            // (for example, service disabled or missing required config). Verify process is up.
            tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
            if !service_is_running(service.as_str()) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!(
                            "service did not enter running state: {service}. check logs/{service}.log and channel config"
                        )),
                    }),
                );
            }
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({
                        "service": service,
                        "action": "start",
                        "status": "starting",
                        "profile": profile
                    })),
                    error: None,
                }),
            )
        }
        ServiceAction::Stop => {
            if service_is_gateway_managed(service.as_str()) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!(
                            "{service} is currently managed by channel-gateway and cannot be stopped from the per-service button"
                        )),
                    }),
                );
            }
            let Some(process_name) = service_process_name(service.as_str()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("unsupported service".to_string()),
                    }),
                );
            };
            let mut killed = 0usize;
            if let Some(pids) = daemon_process_pids_by_name(process_name) {
                for pid in pids {
                    let cmd = format!("kill -TERM {} >/dev/null 2>&1 || true", pid);
                    let _ = Command::new("bash").arg("-lc").arg(cmd).output().await;
                    killed += 1;
                }
            }
            for extra_name in service_extra_process_names_on_stop(service.as_str()) {
                if let Some(pids) = daemon_process_pids_by_name(extra_name) {
                    for pid in pids {
                        let cmd = format!("kill -TERM {} >/dev/null 2>&1 || true", pid);
                        let _ = Command::new("bash").arg("-lc").arg(cmd).output().await;
                        killed += 1;
                    }
                }
            }
            if killed == 0 && !service_is_running(service.as_str()) {
                return (
                    StatusCode::OK,
                    Json(ApiResponse {
                        ok: true,
                        data: Some(json!({
                            "service": service,
                            "action": "stop",
                            "status": "already_stopped"
                        })),
                        error: None,
                    }),
                );
            }
            let Some(pid_file) = service_pid_file(service.as_str()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("unsupported service".to_string()),
                    }),
                );
            };
            let workspace = state.workspace_root.to_string_lossy();
            let cmd = format!(
                "cd {} && rm -f .pids/{}",
                shell_escape_arg(workspace.as_ref()),
                shell_escape_arg(pid_file)
            );
            let output = match Command::new("bash").arg("-lc").arg(cmd).output().await {
                Ok(v) => v,
                Err(err) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiResponse {
                            ok: false,
                            data: None,
                            error: Some(format!("failed to stop service process: {err}")),
                        }),
                    );
                }
            };
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let detail = if !stderr.is_empty() { stderr } else { stdout };
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("service stop command failed: {detail}")),
                    }),
                );
            }
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({
                        "service": service,
                        "action": "stop",
                        "status": "stopped"
                    })),
                    error: None,
                }),
            )
        }
        ServiceAction::Restart => {
            if service_is_gateway_managed(service.as_str()) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!(
                            "{service} is currently managed by channel-gateway and cannot be restarted from the per-service button"
                        )),
                    }),
                );
            }
            if let Err(err) = validate_service_start_readiness(&state, service.as_str()) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(err),
                    }),
                );
            }
            let Some(process_name) = service_process_name(service.as_str()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("unsupported service".to_string()),
                    }),
                );
            };
            if let Some(pids) = daemon_process_pids_by_name(process_name) {
                for pid in pids {
                    let cmd = format!("kill -TERM {} >/dev/null 2>&1 || true", pid);
                    let _ = Command::new("bash").arg("-lc").arg(cmd).output().await;
                }
            }
            for extra_name in service_extra_process_names_on_stop(service.as_str()) {
                if let Some(pids) = daemon_process_pids_by_name(extra_name) {
                    for pid in pids {
                        let cmd = format!("kill -TERM {} >/dev/null 2>&1 || true", pid);
                        let _ = Command::new("bash").arg("-lc").arg(cmd).output().await;
                    }
                }
            }
            if let Some(pid_file) = service_pid_file(service.as_str()) {
                let workspace = state.workspace_root.to_string_lossy();
                let cmd = format!(
                    "cd {} && rm -f .pids/{}",
                    shell_escape_arg(workspace.as_ref()),
                    shell_escape_arg(pid_file)
                );
                let _ = Command::new("bash").arg("-lc").arg(cmd).output().await;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let profile = std::env::var("RUSTCLAW_START_PROFILE")
                .ok()
                .filter(|v| matches!(v.as_str(), "debug" | "release"))
                .unwrap_or_else(|| runtime_profile_default().to_string());
            let Some(script_name) = service_start_script(service.as_str()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("unsupported service".to_string()),
                    }),
                );
            };
            let workspace = state.workspace_root.to_string_lossy();
            let log_file = format!("logs/{}.log", service);
            let cmd = format!(
                "cd {} && mkdir -p logs .pids && nohup ./{} {} > {} 2>&1 &",
                shell_escape_arg(workspace.as_ref()),
                script_name,
                shell_escape_arg(profile.as_str()),
                shell_escape_arg(log_file.as_str())
            );
            if let Err(err) = spawn_background_shell(&cmd) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("failed to start service process: {err}")),
                    }),
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
            if !service_is_running(service.as_str()) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!(
                            "service did not enter running state after restart: {service}. check logs/{service}.log"
                        )),
                    }),
                );
            }
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({
                        "service": service,
                        "action": "restart",
                        "status": "restarted",
                        "profile": profile
                    })),
                    error: None,
                }),
            )
        }
    }
}

async fn restart_system(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can restart RustClaw".to_string()),
            }),
        );
    }

    if !std::path::Path::new("/.dockerenv").exists() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("frontend restart is only available in Docker deployment".to_string()),
            }),
        );
    }

    let mut cmd = Command::new("bash");
    cmd.arg("-lc")
        .arg("sleep 1 && kill -TERM 1 >/dev/null 2>&1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    if let Err(err) = cmd.spawn() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("failed to schedule restart: {err}")),
            }),
        );
    }

    (
        StatusCode::ACCEPTED,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "status": "restarting",
                "mode": "docker",
            })),
            error: None,
        }),
    )
}

async fn health(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<HealthResponse>>) {
    if let Err((status, Json(resp))) = require_ui_identity(&state, &headers) {
        return (
            status,
            Json(ApiResponse {
                ok: resp.ok,
                data: None,
                error: resp.error,
            }),
        );
    }
    let queue_length = task_count_by_status(&state, "queued").unwrap_or_default();
    let running_length = task_count_by_status(&state, "running").unwrap_or_default();
    let running_oldest_age_seconds = oldest_running_task_age_seconds(&state).unwrap_or(0);
    let legacy_telegramd_stats = telegramd_process_stats();
    let channel_gateway_stats = channel_gateway_process_stats();
    let whatsappd_stats = whatsappd_process_stats();
    let wa_webd_stats = wa_webd_process_stats();
    let webd_stats = webd_process_stats();
    let wechatd_stats = wechatd_process_stats();
    let channel_gateway_process_count = channel_gateway_stats.map(|(count, _)| count);
    let channel_gateway_memory_rss_bytes = channel_gateway_stats.map(|(_, rss_bytes)| rss_bytes);
    let channel_gateway_healthy = channel_gateway_process_count.map(|count| count > 0);
    // Telegram 健康状态优先看 channel-gateway（新架构），
    // 但仅在其进程数 > 0 时才覆盖 legacy telegramd；否则回退到 legacy 进程统计。
    let telegram_uses_gateway_stats =
        matches!(channel_gateway_stats, Some((count, _)) if count > 0);
    let telegramd_stats = match (channel_gateway_stats, legacy_telegramd_stats) {
        (Some((count, rss_bytes)), _) if count > 0 => Some((count, rss_bytes)),
        (_, legacy) => legacy,
    };
    let telegramd_process_count = telegramd_stats.map(|(count, _)| count);
    let telegramd_memory_rss_bytes = if telegram_uses_gateway_stats {
        // channel-gateway RSS is shared by multiple adapters and cannot be attributed safely here.
        None
    } else {
        telegramd_stats.map(|(_, rss_bytes)| rss_bytes)
    };
    let telegramd_healthy = telegramd_process_count.map(|count| count > 0);
    let whatsappd_process_count_raw = whatsappd_stats.map(|(count, _)| count);
    let whatsappd_memory_rss_bytes_raw = whatsappd_stats.map(|(_, rss_bytes)| rss_bytes);
    let wa_webd_process_count_raw = wa_webd_stats.map(|(count, _)| count);
    let wa_webd_memory_rss_bytes_raw = wa_webd_stats.map(|(_, rss_bytes)| rss_bytes);
    let webd_process_count = webd_stats.map(|(count, _)| count);
    let webd_memory_rss_bytes = webd_stats.map(|(_, rss_bytes)| rss_bytes);
    let webd_healthy = webd_process_count.map(|count| count > 0);
    let wechatd_process_count = wechatd_stats.map(|(count, _)| count);
    let wechatd_memory_rss_bytes = wechatd_stats.map(|(_, rss_bytes)| rss_bytes);
    let wechatd_healthy = wechatd_process_count.map(|count| count > 0);
    let feishud_stats = feishud_process_stats();
    let feishud_process_count_raw = feishud_stats.map(|(count, _)| count);
    let feishud_memory_rss_bytes_raw = feishud_stats.map(|(_, rss_bytes)| rss_bytes);
    let larkd_stats = larkd_process_stats();
    let larkd_process_count_raw = larkd_stats.map(|(count, _)| count);
    let larkd_memory_rss_bytes_raw = larkd_stats.map(|(_, rss_bytes)| rss_bytes);
    let (user_count, bound_channel_count, bound_channels) =
        auth_user_summary_counts(&state).unwrap_or_default();
    let telegram_configured_bot_names = state.telegram_configured_bot_names.as_ref().clone();
    let telegram_bot_statuses =
        read_telegram_bot_statuses(&state.workspace_root, &telegram_configured_bot_names);
    let mut gateway_instance_statuses_by_scope =
        read_gateway_instance_statuses(&state.workspace_root);
    let whatsapp_cloud_gateway_healthy = gateway_instance_statuses_by_scope
        .get("whatsapp_cloud:primary")
        .map(|s| s.healthy);
    let whatsapp_web_gateway_healthy = gateway_instance_statuses_by_scope
        .get("whatsapp_web:primary")
        .map(|s| s.healthy);
    let feishu_gateway_healthy = gateway_instance_statuses_by_scope
        .get("feishu:primary")
        .map(|s| s.healthy);
    let lark_gateway_healthy = gateway_instance_statuses_by_scope
        .get("lark:primary")
        .map(|s| s.healthy);

    // 其他通信端也增加“网关状态回退”，防止独立进程未启用时 UI 误判未启动。
    let whatsappd_process_count = match whatsappd_process_count_raw {
        Some(count) if count > 0 => Some(count),
        _ if whatsapp_cloud_gateway_healthy == Some(true) => channel_gateway_process_count,
        _ => whatsappd_process_count_raw,
    };
    let whatsappd_memory_rss_bytes = match whatsappd_process_count_raw {
        Some(count) if count > 0 => whatsappd_memory_rss_bytes_raw,
        // channel-gateway RSS is shared by multiple adapters and cannot be attributed safely here.
        _ if whatsapp_cloud_gateway_healthy == Some(true) => None,
        _ => whatsappd_memory_rss_bytes_raw,
    };
    let whatsappd_healthy = match whatsappd_process_count_raw {
        Some(count) if count > 0 => Some(true),
        _ => whatsapp_cloud_gateway_healthy
            .or_else(|| whatsappd_process_count_raw.map(|count| count > 0)),
    };

    let wa_webd_process_count = match wa_webd_process_count_raw {
        Some(count) if count > 0 => Some(count),
        _ if whatsapp_web_gateway_healthy == Some(true) => channel_gateway_process_count,
        _ => wa_webd_process_count_raw,
    };
    let wa_webd_memory_rss_bytes = match wa_webd_process_count_raw {
        Some(count) if count > 0 => wa_webd_memory_rss_bytes_raw,
        // channel-gateway RSS is shared by multiple adapters and cannot be attributed safely here.
        _ if whatsapp_web_gateway_healthy == Some(true) => None,
        _ => wa_webd_memory_rss_bytes_raw,
    };
    let wa_webd_healthy = match wa_webd_process_count_raw {
        Some(count) if count > 0 => Some(true),
        _ => whatsapp_web_gateway_healthy
            .or_else(|| wa_webd_process_count_raw.map(|count| count > 0)),
    };

    let feishud_process_count = match feishud_process_count_raw {
        Some(count) if count > 0 => Some(count),
        _ if feishu_gateway_healthy == Some(true) => channel_gateway_process_count,
        _ => feishud_process_count_raw,
    };
    let feishud_memory_rss_bytes = match feishud_process_count_raw {
        Some(count) if count > 0 => feishud_memory_rss_bytes_raw,
        // channel-gateway RSS is shared by multiple adapters and cannot be attributed safely here.
        _ if feishu_gateway_healthy == Some(true) => None,
        _ => feishud_memory_rss_bytes_raw,
    };
    let feishud_healthy = match feishud_process_count_raw {
        Some(count) if count > 0 => Some(true),
        _ => feishu_gateway_healthy.or_else(|| feishud_process_count_raw.map(|count| count > 0)),
    };

    let larkd_process_count = match larkd_process_count_raw {
        Some(count) if count > 0 => Some(count),
        _ if lark_gateway_healthy == Some(true) => channel_gateway_process_count,
        _ => larkd_process_count_raw,
    };
    let larkd_memory_rss_bytes = match larkd_process_count_raw {
        Some(count) if count > 0 => larkd_memory_rss_bytes_raw,
        // channel-gateway RSS is shared by multiple adapters and cannot be attributed safely here.
        _ if lark_gateway_healthy == Some(true) => None,
        _ => larkd_memory_rss_bytes_raw,
    };
    let larkd_healthy = match larkd_process_count_raw {
        Some(count) if count > 0 => Some(true),
        _ => lark_gateway_healthy.or_else(|| larkd_process_count_raw.map(|count| count > 0)),
    };
    let mut gateway_instance_statuses = Vec::new();
    for bot_status in &telegram_bot_statuses {
        let scope = format!("telegram:{}", bot_status.name);
        gateway_instance_statuses.push(
            gateway_instance_statuses_by_scope
                .remove(&scope)
                .unwrap_or_else(|| GatewayInstanceRuntimeStatus {
                    kind: "telegram".to_string(),
                    name: bot_status.name.clone(),
                    scope,
                    healthy: bot_status.healthy,
                    status: bot_status.status.clone(),
                    last_heartbeat_ts: bot_status.last_heartbeat_ts,
                    last_error: bot_status.last_error.clone(),
                }),
        );
    }
    if state.whatsapp_cloud_enabled {
        let scope = "whatsapp_cloud:primary".to_string();
        gateway_instance_statuses.push(
            gateway_instance_statuses_by_scope
                .remove(&scope)
                .unwrap_or_else(|| GatewayInstanceRuntimeStatus {
                    kind: "whatsapp_cloud".to_string(),
                    name: "primary".to_string(),
                    scope,
                    healthy: whatsappd_healthy.unwrap_or(false),
                    status: if whatsappd_healthy.unwrap_or(false) {
                        "running".to_string()
                    } else {
                        "stopped".to_string()
                    },
                    last_heartbeat_ts: None,
                    last_error: None,
                }),
        );
    }
    if state.whatsapp_web_enabled {
        let scope = "whatsapp_web:primary".to_string();
        gateway_instance_statuses.push(
            gateway_instance_statuses_by_scope
                .remove(&scope)
                .unwrap_or_else(|| GatewayInstanceRuntimeStatus {
                    kind: "whatsapp_web".to_string(),
                    name: "primary".to_string(),
                    scope,
                    healthy: wa_webd_healthy.unwrap_or(false),
                    status: if wa_webd_healthy.unwrap_or(false) {
                        "running".to_string()
                    } else {
                        "stopped".to_string()
                    },
                    last_heartbeat_ts: None,
                    last_error: None,
                }),
        );
    }
    if state.wechat_send_config.is_some() {
        let scope = "wechat:primary".to_string();
        gateway_instance_statuses.push(
            gateway_instance_statuses_by_scope
                .remove(&scope)
                .unwrap_or_else(|| GatewayInstanceRuntimeStatus {
                    kind: "wechat".to_string(),
                    name: "primary".to_string(),
                    scope,
                    healthy: wechatd_healthy.unwrap_or(false),
                    status: if wechatd_healthy.unwrap_or(false) {
                        "running".to_string()
                    } else {
                        "stopped".to_string()
                    },
                    last_heartbeat_ts: None,
                    last_error: None,
                }),
        );
    }
    if state.feishu_send_config.is_some() {
        let scope = "feishu:primary".to_string();
        gateway_instance_statuses.push(
            gateway_instance_statuses_by_scope
                .remove(&scope)
                .unwrap_or_else(|| GatewayInstanceRuntimeStatus {
                    kind: "feishu".to_string(),
                    name: "primary".to_string(),
                    scope,
                    healthy: feishud_healthy.unwrap_or(false),
                    status: if feishud_healthy.unwrap_or(false) {
                        "running".to_string()
                    } else {
                        "stopped".to_string()
                    },
                    last_heartbeat_ts: None,
                    last_error: None,
                }),
        );
    }
    if state.lark_send_config.is_some() {
        let scope = "lark:primary".to_string();
        gateway_instance_statuses.push(
            gateway_instance_statuses_by_scope
                .remove(&scope)
                .unwrap_or_else(|| GatewayInstanceRuntimeStatus {
                    kind: "lark".to_string(),
                    name: "primary".to_string(),
                    scope,
                    healthy: larkd_healthy.unwrap_or(false),
                    status: if larkd_healthy.unwrap_or(false) {
                        "running".to_string()
                    } else {
                        "stopped".to_string()
                    },
                    last_heartbeat_ts: None,
                    last_error: None,
                }),
        );
    }
    gateway_instance_statuses.extend(gateway_instance_statuses_by_scope.into_values());
    let data = HealthResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        queue_length,
        worker_state: "running".to_string(),
        uptime_seconds: state.started_at.elapsed().as_secs(),
        memory_rss_bytes: current_rss_bytes(),
        running_length,
        task_timeout_seconds: state.worker_task_timeout_seconds,
        running_oldest_age_seconds,
        telegramd_healthy,
        telegramd_process_count,
        telegramd_memory_rss_bytes,
        channel_gateway_healthy,
        channel_gateway_process_count,
        channel_gateway_memory_rss_bytes,
        whatsappd_healthy,
        whatsappd_process_count,
        whatsappd_memory_rss_bytes,
        telegram_bot_healthy: telegramd_healthy,
        telegram_bot_process_count: telegramd_process_count,
        telegram_bot_memory_rss_bytes: telegramd_memory_rss_bytes,
        telegram_configured_bot_count: telegram_configured_bot_names.len(),
        telegram_configured_bot_names,
        telegram_bot_statuses,
        gateway_instance_statuses,
        whatsapp_cloud_healthy: whatsappd_healthy,
        whatsapp_cloud_process_count: whatsappd_process_count,
        whatsapp_cloud_memory_rss_bytes: whatsappd_memory_rss_bytes,
        whatsapp_web_healthy: wa_webd_healthy,
        whatsapp_web_process_count: wa_webd_process_count,
        whatsapp_web_memory_rss_bytes: wa_webd_memory_rss_bytes,
        webd_healthy,
        webd_process_count,
        webd_memory_rss_bytes,
        wechatd_healthy,
        wechatd_process_count,
        wechatd_memory_rss_bytes,
        feishud_healthy,
        feishud_process_count,
        feishud_memory_rss_bytes,
        larkd_healthy,
        larkd_process_count,
        larkd_memory_rss_bytes,
        user_count,
        bound_channel_count,
        bound_channels,
        future_adapters_enabled: state.future_adapters_enabled.as_ref().clone(),
    };

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(data),
            error: None,
        }),
    )
}

async fn list_skills(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let mut skills: Vec<String> = state.get_skills_list().iter().cloned().collect();
    skills.retain(|s| !hide_skill_in_ui(&state, s));
    skills.sort_unstable();
    let skill_items = skills
        .iter()
        .map(|name| SkillListItem {
            name: name.clone(),
            description: ui_skill_description(&state, name),
        })
        .collect::<Vec<_>>();
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "skills": skills,
                "skill_items": skill_items,
                "skill_runner_path": state.skill_runner_path.display().to_string(),
            })),
            error: None,
        }),
    )
}

fn ui_skill_description(state: &AppState, skill_name: &str) -> Option<String> {
    let registry_prompt_rel_path = state.skill_registry_prompt_rel_path(skill_name)?;
    let vendor = crate::bootstrap::prompts::active_prompt_vendor_name(state);
    let (raw, _) = prompt_layers::load_prompt_template_for_vendor(
        &state.workspace_root,
        &vendor,
        &registry_prompt_rel_path,
        "",
    );
    extract_skill_description_from_prompt(&raw)
}

fn extract_skill_description_from_prompt(raw: &str) -> Option<String> {
    let frontmatter = parse_skill_frontmatter(raw);
    if !frontmatter.description.trim().is_empty() {
        return Some(frontmatter.description.trim().to_string());
    }

    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("- Description:") {
            let value = rest.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
        if let Some(rest) = trimmed.strip_prefix("description:") {
            let value = rest.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }

    None
}

async fn import_external_skill(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ImportSkillRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let source = req.source.trim();
    if source.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("source is required".to_string()),
            }),
        );
    }
    let enabled = req.enabled.unwrap_or(true);

    let raw_name = guess_bundle_name_from_path_or_source(source, "external-skill");
    let canonical_name = slugify_skill_name(&raw_name);
    let bundle_rel_dir = format!("third_party/clawhub/{canonical_name}");
    let bundle_dir = state.workspace_root.join(&bundle_rel_dir);
    if bundle_dir.exists() {
        if let Err(err) = std::fs::remove_dir_all(&bundle_dir) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("remove old imported bundle failed: {err}")),
                }),
            );
        }
    }

    let skill_md = match materialize_import_source(&state, source, &bundle_dir).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(err),
                }),
            );
        }
    };
    finalize_imported_bundle(
        &state,
        &bundle_dir,
        &bundle_rel_dir,
        source,
        enabled,
        &skill_md,
    )
}

async fn import_external_skill_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }

    let mut bundle_name = String::new();
    let mut enabled = true;
    let mut uploaded_files: Vec<(PathBuf, Vec<u8>)> = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let field_name = field.name().unwrap_or("").to_string();
        match field_name.as_str() {
            "bundle_name" => {
                if let Ok(text) = field.text().await {
                    bundle_name = text.trim().to_string();
                }
            }
            "enabled" => {
                if let Ok(text) = field.text().await {
                    enabled = text.trim() != "false";
                }
            }
            "files" => {
                let raw_path = field
                    .file_name()
                    .map(str::to_string)
                    .unwrap_or_else(|| "SKILL.md".to_string());
                let Some(rel_path) = sanitize_upload_relative_path(&raw_path) else {
                    continue;
                };
                let Ok(bytes) = field.bytes().await else {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ApiResponse {
                            ok: false,
                            data: None,
                            error: Some("read uploaded file failed".to_string()),
                        }),
                    );
                };
                uploaded_files.push((rel_path, bytes.to_vec()));
            }
            _ => {}
        }
    }

    if uploaded_files.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("no uploaded files found".to_string()),
            }),
        );
    }

    let guessed_name = if !bundle_name.trim().is_empty() {
        bundle_name.trim().to_string()
    } else {
        uploaded_files
            .first()
            .and_then(|(path, _)| path.components().next())
            .and_then(|part| match part {
                std::path::Component::Normal(v) => v.to_str(),
                _ => None,
            })
            .unwrap_or("uploaded-skill")
            .to_string()
    };
    let canonical_name = slugify_skill_name(&guessed_name);
    let bundle_rel_dir = format!("third_party/clawhub/{canonical_name}");
    let bundle_dir = state.workspace_root.join(&bundle_rel_dir);
    if bundle_dir.exists() {
        if let Err(err) = std::fs::remove_dir_all(&bundle_dir) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("remove old uploaded bundle failed: {err}")),
                }),
            );
        }
    }
    if let Err(err) = std::fs::create_dir_all(&bundle_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("create upload bundle dir failed: {err}")),
            }),
        );
    }

    let mut skill_md_path = None;
    for (rel_path, bytes) in uploaded_files {
        let normalized = rel_path
            .strip_prefix(&guessed_name)
            .ok()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or(rel_path);
        let target_path = bundle_dir.join(&normalized);
        if let Some(parent) = target_path.parent() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("create uploaded subdirectory failed: {err}")),
                    }),
                );
            }
        }
        if let Err(err) = std::fs::write(&target_path, bytes) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("write uploaded file failed: {err}")),
                }),
            );
        }
        if normalized
            .file_name()
            .and_then(|v| v.to_str())
            .map(|name| name.eq_ignore_ascii_case("SKILL.md"))
            .unwrap_or(false)
        {
            skill_md_path = Some(target_path);
        }
    }

    let skill_md_path = skill_md_path.unwrap_or_else(|| bundle_dir.join("SKILL.md"));
    let skill_md = match std::fs::read_to_string(&skill_md_path) {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!(
                        "uploaded bundle is missing readable SKILL.md: {err}"
                    )),
                }),
            );
        }
    };

    finalize_imported_bundle(
        &state,
        &bundle_dir,
        &bundle_rel_dir,
        &format!("upload:{guessed_name}"),
        enabled,
        &skill_md,
    )
}

#[derive(Debug, Deserialize)]
struct UpdateSkillsConfigRequest {
    #[serde(default)]
    skill_switches: HashMap<String, bool>,
}

#[derive(Debug, Deserialize)]
struct ImportSkillRequest {
    source: String,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct UpdateLlmConfigRequest {
    selected_vendor: String,
    selected_model: String,
    #[serde(default)]
    vendor_base_url: Option<String>,
    #[serde(default)]
    vendor_api_key: Option<String>,
    #[serde(default)]
    vendor_api_format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TelegramBotConfigItem {
    name: String,
    #[serde(default)]
    bot_token: String,
    #[serde(default)]
    bot_token_configured: bool,
    #[serde(default)]
    bot_token_masked: Option<String>,
    #[serde(default = "default_agent_id")]
    agent_id: String,
    #[serde(default)]
    allowlist: Vec<i64>,
    #[serde(default = "default_telegram_access_mode")]
    access_mode: String,
    #[serde(default)]
    allowed_telegram_usernames: Vec<String>,
    #[serde(default)]
    is_primary: bool,
}

#[derive(Debug, Serialize)]
struct TelegramConfigResponse {
    config_path: String,
    bots: Vec<TelegramBotConfigItem>,
    agents: Vec<AgentConfigItem>,
    restart_required: bool,
}

#[derive(Debug, Deserialize)]
struct UpdateTelegramConfigRequest {
    #[serde(default)]
    bots: Vec<TelegramBotConfigItem>,
    #[serde(default)]
    agents: Vec<AgentConfigItem>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WechatConfigResponse {
    config_path: String,
    enabled: bool,
    listen: String,
    clawd_base_url: String,
    api_base_url: String,
    wechat_uin_base64: String,
    request_timeout_seconds: u64,
    longpoll_timeout_ms: u64,
    text_chunk_chars: usize,
    bot_token_configured: bool,
    saved_session_present: bool,
    restart_required: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct FeishuConfigResponse {
    config_path: String,
    enabled: bool,
    mode: String,
    listen: String,
    clawd_base_url: String,
    api_base_url: String,
    app_id: String,
    app_secret: String,
    verification_token_configured: bool,
    encrypt_key_configured: bool,
    bind_ready: bool,
    current_key_bound: bool,
    restart_required: bool,
}

#[derive(Debug, Deserialize)]
struct UpdateWechatConfigRequest {
    enabled: bool,
    listen: String,
    clawd_base_url: String,
    api_base_url: String,
    #[serde(default)]
    wechat_uin_base64: String,
    request_timeout_seconds: u64,
    longpoll_timeout_ms: u64,
    text_chunk_chars: usize,
}

#[derive(Debug, Deserialize)]
struct UpdateFeishuConfigRequest {
    #[serde(default)]
    app_id: String,
    #[serde(default)]
    app_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentConfigItem {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    persona_prompt: String,
    #[serde(default)]
    preferred_vendor: Option<String>,
    #[serde(default)]
    preferred_model: Option<String>,
    #[serde(default)]
    allowed_skills: Vec<String>,
}

fn default_agent_id() -> String {
    "main".to_string()
}

fn default_telegram_access_mode() -> String {
    "public".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelConfigItem {
    vendor: String,
    model: String,
}

#[derive(Debug, Serialize)]
struct ModelConfigResponse {
    llm: ModelConfigItem,
    image_edit: ModelConfigItem,
    image_generation: ModelConfigItem,
    image_vision: ModelConfigItem,
    audio_transcribe: ModelConfigItem,
    audio_synthesize: ModelConfigItem,
    restart_required: bool,
}

#[derive(Debug, Deserialize)]
struct ModelConfigUpdateRequest {
    llm: Option<ModelConfigItem>,
    image_edit: Option<ModelConfigItem>,
    image_generation: Option<ModelConfigItem>,
    image_vision: Option<ModelConfigItem>,
    audio_transcribe: Option<ModelConfigItem>,
    audio_synthesize: Option<ModelConfigItem>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ProviderKeysResponse {
    #[serde(default)]
    llm: HashMap<String, String>,
    #[serde(default)]
    image: HashMap<String, HashMap<String, String>>,
    #[serde(default)]
    audio: HashMap<String, HashMap<String, String>>,
}

fn default_model_item() -> ModelConfigItem {
    ModelConfigItem {
        vendor: String::new(),
        model: String::new(),
    }
}

fn read_model_config(state: &AppState) -> anyhow::Result<ModelConfigResponse> {
    let root = &state.workspace_root;

    let config_path = root.join("configs/config.toml");
    let config_raw = std::fs::read_to_string(&config_path).unwrap_or_else(|_| String::new());
    let config: toml::Value =
        toml::from_str(&config_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let llm = config
        .get("llm")
        .and_then(|t| t.as_table())
        .map(|t| ModelConfigItem {
            vendor: t
                .get("selected_vendor")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            model: t
                .get("selected_model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
        .unwrap_or_else(default_model_item);

    let image_path = root.join("configs/image.toml");
    let image_raw = std::fs::read_to_string(&image_path).unwrap_or_else(|_| String::new());
    let image: toml::Value =
        toml::from_str(&image_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));

    let read_image_section = |section: &str| -> ModelConfigItem {
        image
            .get(section)
            .and_then(|t| t.as_table())
            .map(|t| ModelConfigItem {
                vendor: t
                    .get("default_vendor")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                model: t
                    .get("default_model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
            .unwrap_or_else(default_model_item)
    };
    let image_edit = read_image_section("image_edit");
    let image_generation = read_image_section("image_generation");
    let image_vision = read_image_section("image_vision");

    let audio_path = root.join("configs/audio.toml");
    let audio_raw = std::fs::read_to_string(&audio_path).unwrap_or_else(|_| String::new());
    let audio: toml::Value =
        toml::from_str(&audio_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));

    let audio_transcribe = audio
        .get("audio_transcribe")
        .and_then(|t| t.as_table())
        .map(|t| ModelConfigItem {
            vendor: t
                .get("default_vendor")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            model: t
                .get("default_model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
        .unwrap_or_else(default_model_item);

    let audio_synthesize = audio
        .get("audio_synthesize")
        .and_then(|t| t.as_table())
        .map(|t| ModelConfigItem {
            vendor: t
                .get("default_vendor")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            model: t
                .get("default_model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
        .unwrap_or_else(default_model_item);

    Ok(ModelConfigResponse {
        llm,
        image_edit,
        image_generation,
        image_vision,
        audio_transcribe,
        audio_synthesize,
        restart_required: true,
    })
}

fn write_model_config(state: &AppState, req: &ModelConfigUpdateRequest) -> anyhow::Result<()> {
    let root = &state.workspace_root;

    if let Some(ref llm) = req.llm {
        let path = root.join("configs/config.toml");
        let raw = std::fs::read_to_string(&path)?;
        let mut value: toml::Value = toml::from_str(&raw)?;
        if let Some(t) = value.get_mut("llm").and_then(|v| v.as_table_mut()) {
            t.insert(
                "selected_vendor".to_string(),
                toml::Value::String(llm.vendor.clone()),
            );
            t.insert(
                "selected_model".to_string(),
                toml::Value::String(llm.model.clone()),
            );
        } else {
            let mut tbl = toml::map::Map::new();
            tbl.insert(
                "selected_vendor".to_string(),
                toml::Value::String(llm.vendor.clone()),
            );
            tbl.insert(
                "selected_model".to_string(),
                toml::Value::String(llm.model.clone()),
            );
            value
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("config.toml root is not a table"))?
                .insert("llm".to_string(), toml::Value::Table(tbl));
        }
        std::fs::write(&path, toml::to_string_pretty(&value)?)?;
    }

    let mut image_modified = false;
    let image_path = root.join("configs/image.toml");
    let image_raw = std::fs::read_to_string(&image_path).unwrap_or_else(|_| String::new());
    let mut image: toml::Value =
        toml::from_str(&image_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));

    for (section, item) in [
        ("image_edit", req.image_edit.as_ref()),
        ("image_generation", req.image_generation.as_ref()),
        ("image_vision", req.image_vision.as_ref()),
    ] {
        if let Some(it) = item {
            image_modified = true;
            let tbl = image
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("image.toml root is not a table"))?
                .entry(section.to_string())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            if let Some(t) = tbl.as_table_mut() {
                t.insert(
                    "default_vendor".to_string(),
                    toml::Value::String(it.vendor.clone()),
                );
                t.insert(
                    "default_model".to_string(),
                    toml::Value::String(it.model.clone()),
                );
            }
        }
    }
    if image_modified {
        std::fs::write(&image_path, toml::to_string_pretty(&image)?)?;
    }

    let mut audio_modified = false;
    let audio_path = root.join("configs/audio.toml");
    let audio_raw = std::fs::read_to_string(&audio_path).unwrap_or_else(|_| String::new());
    let mut audio: toml::Value =
        toml::from_str(&audio_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));

    if let Some(ref it) = req.audio_transcribe {
        audio_modified = true;
        let tbl = audio
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("audio.toml root is not a table"))?
            .entry("audio_transcribe".to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        if let Some(t) = tbl.as_table_mut() {
            t.insert(
                "default_vendor".to_string(),
                toml::Value::String(it.vendor.clone()),
            );
            t.insert(
                "default_model".to_string(),
                toml::Value::String(it.model.clone()),
            );
        }
    }
    if let Some(ref it) = req.audio_synthesize {
        audio_modified = true;
        let tbl = audio
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("audio.toml root is not a table"))?
            .entry("audio_synthesize".to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        if let Some(t) = tbl.as_table_mut() {
            t.insert(
                "default_vendor".to_string(),
                toml::Value::String(it.vendor.clone()),
            );
            t.insert(
                "default_model".to_string(),
                toml::Value::String(it.model.clone()),
            );
        }
    }
    if audio_modified {
        std::fs::write(&audio_path, toml::to_string_pretty(&audio)?)?;
    }

    Ok(())
}

fn read_llm_provider_keys(config: &toml::Value) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(llm) = config.get("llm").and_then(|v| v.as_table()) else {
        return out;
    };
    for (k, v) in llm {
        if let Some(tbl) = v.as_table() {
            if let Some(ak) = tbl.get("api_key").and_then(|a| a.as_str()) {
                out.insert(k.clone(), mask_secret(ak));
            }
        }
    }
    out
}

fn read_image_provider_keys(image: &toml::Value) -> HashMap<String, HashMap<String, String>> {
    let mut out = HashMap::new();
    for section in ["image_edit", "image_generation", "image_vision"] {
        let mut vendors = HashMap::new();
        if let Some(providers) = image
            .get(section)
            .and_then(|v| v.get("providers"))
            .and_then(|v| v.as_table())
        {
            for (vendor, tbl) in providers {
                if let Some(t) = tbl.as_table() {
                    if let Some(ak) = t.get("api_key").and_then(|a| a.as_str()) {
                        vendors.insert(vendor.clone(), mask_secret(ak));
                    }
                }
            }
        }
        out.insert(section.to_string(), vendors);
    }
    out
}

fn read_audio_provider_keys(audio: &toml::Value) -> HashMap<String, HashMap<String, String>> {
    let mut out = HashMap::new();
    for section in ["audio_synthesize", "audio_transcribe"] {
        let mut vendors = HashMap::new();
        if let Some(providers) = audio
            .get(section)
            .and_then(|v| v.get("providers"))
            .and_then(|v| v.as_table())
        {
            for (vendor, tbl) in providers {
                if let Some(t) = tbl.as_table() {
                    if let Some(ak) = t.get("api_key").and_then(|a| a.as_str()) {
                        vendors.insert(vendor.clone(), mask_secret(ak));
                    }
                }
            }
        }
        out.insert(section.to_string(), vendors);
    }
    out
}

fn read_provider_keys(state: &AppState) -> anyhow::Result<ProviderKeysResponse> {
    let root = &state.workspace_root;

    let config_path = root.join("configs/config.toml");
    let config_raw = std::fs::read_to_string(&config_path).unwrap_or_else(|_| String::new());
    let config: toml::Value =
        toml::from_str(&config_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let llm = read_llm_provider_keys(&config);

    let image_path = root.join("configs/image.toml");
    let image_raw = std::fs::read_to_string(&image_path).unwrap_or_else(|_| String::new());
    let image: toml::Value =
        toml::from_str(&image_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let image_keys = read_image_provider_keys(&image);

    let audio_path = root.join("configs/audio.toml");
    let audio_raw = std::fs::read_to_string(&audio_path).unwrap_or_else(|_| String::new());
    let audio: toml::Value =
        toml::from_str(&audio_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let audio_keys = read_audio_provider_keys(&audio);

    Ok(ProviderKeysResponse {
        llm,
        image: image_keys,
        audio: audio_keys,
    })
}

fn write_provider_keys(state: &AppState, req: &ProviderKeysResponse) -> anyhow::Result<()> {
    let root = &state.workspace_root;

    if !req.llm.is_empty() {
        let path = root.join("configs/config.toml");
        let raw = std::fs::read_to_string(&path)?;
        let mut config: toml::Value = toml::from_str(&raw)?;
        let llm = config
            .get_mut("llm")
            .and_then(|v| v.as_table_mut())
            .ok_or_else(|| anyhow::anyhow!("config.toml has no [llm] table"))?;
        for (vendor, new_key) in &req.llm {
            if new_key.is_empty() {
                continue;
            }
            let entry = llm
                .entry(vendor.clone())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            if let Some(t) = entry.as_table_mut() {
                t.insert("api_key".to_string(), toml::Value::String(new_key.clone()));
            }
        }
        std::fs::write(&path, toml::to_string_pretty(&config)?)?;
    }

    if !req.image.is_empty() {
        let path = root.join("configs/image.toml");
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| String::new());
        let mut image: toml::Value =
            toml::from_str(&raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
        let root_t = image
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("image.toml root not a table"))?;
        for (section, vendors) in &req.image {
            if vendors.is_empty() {
                continue;
            }
            let section_t = root_t
                .entry(section.clone())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            let providers = section_t
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("image.toml [{}] not a table", section))?
                .entry("providers".to_string())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            let prov_t = providers
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("providers not a table"))?;
            for (vendor, new_key) in vendors {
                if new_key.is_empty() {
                    continue;
                }
                let entry = prov_t
                    .entry(vendor.clone())
                    .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
                if let Some(t) = entry.as_table_mut() {
                    t.insert("api_key".to_string(), toml::Value::String(new_key.clone()));
                }
            }
        }
        std::fs::write(&path, toml::to_string_pretty(&image)?)?;
    }

    if !req.audio.is_empty() {
        let path = root.join("configs/audio.toml");
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| String::new());
        let mut audio: toml::Value =
            toml::from_str(&raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
        let root_t = audio
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("audio.toml root not a table"))?;
        for (section, vendors) in &req.audio {
            if vendors.is_empty() {
                continue;
            }
            let section_t = root_t
                .entry(section.clone())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            let providers = section_t
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("audio.toml [{}] not a table", section))?
                .entry("providers".to_string())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            let prov_t = providers
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("providers not a table"))?;
            for (vendor, new_key) in vendors {
                if new_key.is_empty() {
                    continue;
                }
                let entry = prov_t
                    .entry(vendor.clone())
                    .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
                if let Some(t) = entry.as_table_mut() {
                    t.insert("api_key".to_string(), toml::Value::String(new_key.clone()));
                }
            }
        }
        std::fs::write(&path, toml::to_string_pretty(&audio)?)?;
    }

    Ok(())
}

async fn get_model_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<ModelConfigResponse>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return (
            resp.0,
            Json(ApiResponse {
                ok: resp.1.ok,
                data: None,
                error: resp.1.error.clone(),
            }),
        );
    }
    match read_model_config(&state) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("read model config failed: {err}")),
            }),
        ),
    }
}

async fn update_model_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ModelConfigUpdateRequest>,
) -> (StatusCode, Json<ApiResponse<ModelConfigResponse>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return (
            resp.0,
            Json(ApiResponse {
                ok: resp.1.ok,
                data: None,
                error: resp.1.error.clone(),
            }),
        );
    }
    if let Err(err) = write_model_config(&state, &req) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write model config failed: {err}")),
            }),
        );
    }
    match read_model_config(&state) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: None,
                error: Some(format!("saved but re-read failed: {err}")),
            }),
        ),
    }
}

async fn get_provider_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<ProviderKeysResponse>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return (
            resp.0,
            Json(ApiResponse {
                ok: resp.1.ok,
                data: None,
                error: resp.1.error.clone(),
            }),
        );
    }
    match read_provider_keys(&state) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("read provider keys failed: {err}")),
            }),
        ),
    }
}

async fn update_provider_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ProviderKeysResponse>,
) -> (StatusCode, Json<ApiResponse<ProviderKeysResponse>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return (
            resp.0,
            Json(ApiResponse {
                ok: resp.1.ok,
                data: None,
                error: resp.1.error.clone(),
            }),
        );
    }
    if let Err(err) = write_provider_keys(&state, &req) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write provider keys failed: {err}")),
            }),
        );
    }
    match read_provider_keys(&state) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: None,
                error: Some(format!("saved but re-read failed: {err}")),
            }),
        ),
    }
}

async fn restart_clawd(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let workspace = state.workspace_root.to_string_lossy();
    let pid = std::process::id();
    let script =
        format!("sleep 2; kill {pid} 2>/dev/null; sleep 1; cd {workspace} && ./start-clawd.sh");
    let mut cmd = StdCommand::new("nohup");
    cmd.arg("bash")
        .arg("-c")
        .arg(&script)
        .current_dir(&state.workspace_root)
        .stdin(StdProcessStdio::null())
        .stdout(StdProcessStdio::null())
        .stderr(StdProcessStdio::null());
    match cmd.spawn() {
        Ok(_) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({
                    "message": "restart triggered; clawd will restart in a few seconds",
                    "restart_triggered": true
                })),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("failed to spawn restart process: {err}")),
            }),
        ),
    }
}

fn read_skill_config_file(state: &AppState) -> anyhow::Result<(String, toml::Value)> {
    let path = state.workspace_root.join("configs/config.toml");
    let raw = std::fs::read_to_string(&path)?;
    let parsed = toml::from_str::<toml::Value>(&raw)?;
    Ok((raw, parsed))
}

fn write_workspace_and_mounted_file(
    workspace_root: &Path,
    relative_path: &str,
    raw: &str,
) -> std::io::Result<()> {
    let active_path = workspace_root.join(relative_path);
    if let Some(parent) = active_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&active_path, raw)?;

    let mounted_relative = relative_path
        .strip_prefix("configs/")
        .unwrap_or(relative_path);
    let mounted_path = workspace_root.join("docker/config").join(mounted_relative);
    if let Some(parent) = mounted_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&mounted_path, raw)?;
    Ok(())
}

fn write_runtime_config_file(state: &AppState, raw: &str) -> std::io::Result<()> {
    write_workspace_and_mounted_file(&state.workspace_root, "configs/config.toml", raw)
}

fn read_skills_registry_file(state: &AppState) -> std::io::Result<String> {
    let path = state.workspace_root.join("configs/skills_registry.toml");
    match std::fs::read_to_string(path) {
        Ok(raw) => Ok(raw),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err),
    }
}

fn write_skills_registry_file(state: &AppState, raw: &str) -> std::io::Result<()> {
    let active_path = state.workspace_root.join("configs/skills_registry.toml");
    if let Some(parent) = active_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&active_path, raw)?;

    let mounted_path = state
        .workspace_root
        .join("docker/config/skills_registry.toml");
    if let Some(parent) = mounted_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&mounted_path, raw)?;
    Ok(())
}

#[derive(Debug, Default)]
struct ParsedSkillFrontmatter {
    name: String,
    description: String,
    metadata: Option<Value>,
}

#[derive(Debug)]
struct ImportedSkillPlan {
    canonical_name: String,
    display_name: String,
    description: String,
    external_kind: String,
    aliases: Vec<String>,
    registry_prompt_rel_path: String,
    prompt_body_rel_path: String,
    bundle_rel_dir: String,
    entry_file: String,
    runtime: Option<String>,
    require_bins: Vec<String>,
    require_py_modules: Vec<String>,
    source_url: String,
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct UninstallExternalSkillRequest {
    skill_name: String,
}

fn normalize_remote_skill_source(source: &str) -> String {
    let trimmed = source.trim();
    if let Some(rest) = trimmed.strip_prefix("https://github.com/") {
        if let Some((repo_part, path_part)) = rest.split_once("/blob/") {
            if let Some((branch, file_path)) = path_part.split_once('/') {
                return format!(
                    "https://raw.githubusercontent.com/{repo_part}/{branch}/{file_path}"
                );
            }
        }
    }
    trimmed.to_string()
}

fn slugify_skill_name(input: &str) -> String {
    let mut out = String::new();
    let mut last_was_sep = false;
    for ch in input.trim().chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else {
            '_'
        };
        if mapped == '_' {
            if out.is_empty() || last_was_sep {
                continue;
            }
            last_was_sep = true;
            out.push('_');
        } else {
            last_was_sep = false;
            out.push(mapped);
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        "external_skill".to_string()
    } else if out.chars().next().unwrap_or('a').is_ascii_digit() {
        format!("ext_{out}")
    } else {
        out
    }
}

fn parse_skill_frontmatter(skill_md: &str) -> ParsedSkillFrontmatter {
    let mut parsed = ParsedSkillFrontmatter::default();
    let mut lines = skill_md.lines();
    if lines.next().map(str::trim) != Some("---") {
        return parsed;
    }
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if trimmed.is_empty() {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"').trim_matches('\'');
        match key {
            "name" => parsed.name = value.to_string(),
            "description" => parsed.description = value.to_string(),
            "metadata" => {
                if let Ok(meta) = serde_json::from_str::<Value>(value) {
                    parsed.metadata = Some(meta);
                }
            }
            _ => {}
        }
    }
    parsed
}

fn scan_bundle_files(root: &Path, current: &Path, acc: &mut Vec<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            scan_bundle_files(root, &path, acc)?;
            continue;
        }
        if file_type.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            acc.push(rel);
        }
    }
    Ok(())
}

fn extract_required_bins(metadata: Option<&Value>) -> Vec<String> {
    let mut bins = Vec::new();
    let sources = [
        metadata,
        metadata.and_then(|m| m.get("openclaw")),
        metadata
            .and_then(|m| m.get("openclaw"))
            .and_then(|m| m.get("requires")),
    ];
    for source in sources.into_iter().flatten() {
        if let Some(arr) = source.get("bins").and_then(|v| v.as_array()) {
            for item in arr.iter().filter_map(|v| v.as_str()) {
                let item = item.trim();
                if !item.is_empty() && !bins.iter().any(|existing| existing == item) {
                    bins.push(item.to_string());
                }
            }
        }
    }
    bins
}

fn infer_python_modules(script_path: &Path) -> Vec<String> {
    let mut modules = Vec::new();
    let Ok(raw) = std::fs::read_to_string(script_path) else {
        return modules;
    };
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("import ") {
            for item in rest.split(',') {
                let name = item
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .split('.')
                    .next()
                    .unwrap_or("")
                    .trim();
                if name == "akshare" && !modules.iter().any(|m| m == name) {
                    modules.push(name.to_string());
                }
            }
        } else if let Some(rest) = trimmed.strip_prefix("from ") {
            let name = rest
                .split_whitespace()
                .next()
                .unwrap_or("")
                .split('.')
                .next()
                .unwrap_or("")
                .trim();
            if name == "akshare" && !modules.iter().any(|m| m == name) {
                modules.push(name.to_string());
            }
        }
    }
    modules
}

fn detect_import_plan(
    skill_md: &str,
    bundle_dir: &Path,
    bundle_rel_dir: &str,
    source: &str,
    enabled: bool,
) -> anyhow::Result<ImportedSkillPlan> {
    let frontmatter = parse_skill_frontmatter(skill_md);
    let mut files = Vec::new();
    scan_bundle_files(bundle_dir, bundle_dir, &mut files)?;
    files.sort();

    let display_name = if !frontmatter.name.trim().is_empty() {
        frontmatter.name.trim().to_string()
    } else {
        bundle_dir
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("external-skill")
            .to_string()
    };
    let canonical_name = slugify_skill_name(&display_name);
    let mut aliases = Vec::new();
    let alias = display_name.trim().to_ascii_lowercase();
    if !alias.is_empty() && alias != canonical_name {
        aliases.push(alias);
    }

    let mut require_bins = extract_required_bins(frontmatter.metadata.as_ref());
    let mut require_py_modules = Vec::new();
    let mut external_kind = "prompt_bundle".to_string();
    let mut entry_file = "SKILL.md".to_string();
    let mut runtime = None;

    let first_python = files.iter().find(|path| path.ends_with(".py")).cloned();
    let first_node = files
        .iter()
        .find(|path| path.ends_with(".js") || path.ends_with(".mjs") || path.ends_with(".cjs"))
        .cloned();
    if let Some(py_entry) = first_python {
        external_kind = "local_script".to_string();
        entry_file = py_entry.clone();
        runtime = Some("python3".to_string());
        if !require_bins.iter().any(|item| item == "python3") {
            require_bins.push("python3".to_string());
        }
        require_py_modules = infer_python_modules(&bundle_dir.join(&py_entry));
    } else if let Some(node_entry) = first_node {
        external_kind = "local_script".to_string();
        entry_file = node_entry;
        runtime = Some("node".to_string());
        if !require_bins.iter().any(|item| item == "node") {
            require_bins.push("node".to_string());
        }
    } else if skill_md.contains("```bash")
        || skill_md.contains("```sh")
        || !require_bins.is_empty()
        || skill_md.contains("curl ")
        || skill_md.contains("jq ")
    {
        external_kind = "local_shell_recipe".to_string();
    }

    let description = if !frontmatter.description.trim().is_empty() {
        frontmatter.description.trim().to_string()
    } else {
        "Imported external skill".to_string()
    };
    let registry_prompt_rel_path = format!("prompts/skills/{canonical_name}.md");
    let prompt_body_rel_path = format!("prompts/layers/generated/skills/{canonical_name}.md");
    Ok(ImportedSkillPlan {
        canonical_name,
        display_name,
        description,
        external_kind,
        aliases,
        registry_prompt_rel_path,
        prompt_body_rel_path,
        bundle_rel_dir: bundle_rel_dir.to_string(),
        entry_file,
        runtime,
        require_bins,
        require_py_modules,
        source_url: source.to_string(),
        enabled,
    })
}

fn render_string_array(items: &[String]) -> String {
    if items.is_empty() {
        "[]".to_string()
    } else {
        let body = items
            .iter()
            .map(|item| format!("{item:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("[{body}]")
    }
}

fn render_imported_skill_registry_block(plan: &ImportedSkillPlan) -> String {
    let mut lines = Vec::new();
    lines.push("[[skills]]".to_string());
    lines.push(format!("name = {:?}", plan.canonical_name));
    lines.push(format!("enabled = {}", plan.enabled));
    lines.push("kind = \"external\"".to_string());
    lines.push(format!("aliases = {}", render_string_array(&plan.aliases)));
    lines.push("timeout_seconds = 60".to_string());
    lines.push(format!("prompt_file = {:?}", plan.registry_prompt_rel_path));
    lines.push("output_kind = \"text\"".to_string());
    lines.push(format!("external_kind = {:?}", plan.external_kind));
    lines.push(format!("external_bundle_dir = {:?}", plan.bundle_rel_dir));
    lines.push(format!("external_entry_file = {:?}", plan.entry_file));
    if let Some(runtime) = &plan.runtime {
        lines.push(format!("external_runtime = {:?}", runtime));
    }
    lines.push(format!(
        "external_require_bins = {}",
        render_string_array(&plan.require_bins)
    ));
    lines.push(format!(
        "external_require_py_modules = {}",
        render_string_array(&plan.require_py_modules)
    ));
    lines.push(format!("external_source_url = {:?}", plan.source_url));
    lines.join("\n")
}

fn render_imported_skill_prompt(plan: &ImportedSkillPlan, skill_md: &str) -> String {
    let normalized_skill_md = skill_md.trim();
    let mut out = String::new();
    out.push_str("<!-- AUTO-GENERATED: external skill importer -->\n");
    out.push_str(&format!("# {}\n\n", plan.display_name));
    out.push_str("RustClaw imported external skill wrapper.\n\n");
    out.push_str("## RustClaw Wrapper\n");
    out.push_str(&format!(
        "- This is an imported external skill: `{}`.\n",
        plan.display_name
    ));
    out.push_str(&format!("- Description: {}\n", plan.description));
    out.push_str(&format!("- Runtime mode: `{}`\n", plan.external_kind));
    out.push_str(&format!("- Bundle directory: `{}`\n", plan.bundle_rel_dir));
    out.push_str(&format!("- Entry file: `{}`\n", plan.entry_file));
    if let Some(runtime) = &plan.runtime {
        out.push_str(&format!("- Runtime binary: `{runtime}`\n"));
    }
    if !plan.require_bins.is_empty() {
        out.push_str(&format!(
            "- Required local commands: {}\n",
            plan.require_bins.join(", ")
        ));
    }
    if !plan.require_py_modules.is_empty() {
        out.push_str(&format!(
            "- Required Python packages: {}\n",
            plan.require_py_modules.join(", ")
        ));
    }
    out.push_str(&format!("- Source: `{}`\n", plan.source_url));
    out.push_str("\n## Calling Rules\n");
    out.push_str("- Prefer the original `SKILL.md` below over your own guesses.\n");
    out.push_str(
        "- Follow the documented commands, options, examples, and parameter names from the original `SKILL.md` exactly.\n",
    );
    out.push_str(
        "- Do not invent unsupported CLI flags, JSON fields, shell fragments, or action names that are not grounded in the original `SKILL.md` or the entry file.\n",
    );
    match plan.external_kind.as_str() {
        "local_script" => {
            out.push_str(
                "- This skill runs a local script. Stay close to the script's real supported options and examples from the original `SKILL.md`.\n",
            );
            out.push_str(
                "- If the original `SKILL.md` shows a concrete command example, mirror that option shape instead of inventing a higher-level parameter.\n",
            );
        }
        "local_shell_recipe" => {
            out.push_str("- This skill runs shell recipes inside its bundle directory.\n");
            out.push_str(
                "- Keep the command close to the examples shown in the original `SKILL.md`.\n",
            );
            out.push_str(
                "- Prefer short, explicit commands. Reuse the documented recipes instead of inventing unrelated shell pipelines.\n",
            );
        }
        _ => {
            out.push_str(
                "- This prompt file lets the imported skill appear in RustClaw immediately.\n",
            );
            out.push_str(
                "- Runtime execution may still require a dedicated executor for this external kind.\n",
            );
        }
    }
    out.push_str(
        "- Avoid adding internal metadata fields yourself; RustClaw will inject its own runtime context.\n",
    );
    if !normalized_skill_md.is_empty() {
        out.push_str("\n## Original SKILL.md\n\n");
        out.push_str(normalized_skill_md);
        out.push('\n');
    }
    out
}

fn parse_registry_block_name(block: &[&str]) -> Option<String> {
    for line in block {
        let trimmed = line.trim();
        if !trimmed.starts_with("name") {
            continue;
        }
        let Some((lhs, rhs)) = trimmed.split_once('=') else {
            continue;
        };
        if lhs.trim() != "name" {
            continue;
        }
        let rhs = rhs.trim();
        let parsed = toml::from_str::<toml::Value>(&format!("value = {rhs}")).ok()?;
        let value = parsed.get("value")?.as_str()?.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn remove_skill_registry_block(raw: &str, skill_name: &str) -> (String, bool) {
    let mut out: Vec<String> = Vec::new();
    let lines: Vec<&str> = raw.lines().collect();
    let mut idx = 0usize;
    let mut removed = false;
    while idx < lines.len() {
        if lines[idx].trim() != "[[skills]]" {
            out.push(lines[idx].to_string());
            idx += 1;
            continue;
        }
        let start = idx;
        idx += 1;
        while idx < lines.len() && lines[idx].trim() != "[[skills]]" {
            idx += 1;
        }
        let block = &lines[start..idx];
        let block_name = parse_registry_block_name(block)
            .map(|name| name.to_ascii_lowercase())
            .unwrap_or_default();
        if block_name == skill_name {
            removed = true;
            continue;
        }
        out.extend(block.iter().map(|line| (*line).to_string()));
    }
    let mut rendered = out.join("\n");
    if raw.ends_with('\n') {
        rendered.push('\n');
    }
    (rendered, removed)
}

fn remove_managed_prompt_file(path: &Path) -> std::io::Result<bool> {
    let raw = match std::fs::read_to_string(path) {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };
    if raw.contains("<!-- AUTO-GENERATED: external skill importer -->") {
        std::fs::remove_file(path)?;
        return Ok(true);
    }
    Ok(false)
}

fn remove_runtime_skill_switch(raw: &str, state: &AppState, skill_name: &str) -> String {
    let parsed = toml::from_str::<toml::Value>(raw)
        .unwrap_or_else(|_| toml::Value::Table(Default::default()));
    let mut switches = collect_skill_switches(&parsed, state);
    switches.remove(skill_name);
    let rendered = render_switches_inline_table(&switches);
    upsert_skill_switches_line(raw, &rendered)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dst.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else if file_type.is_file() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

fn sanitize_upload_relative_path(input: &str) -> Option<PathBuf> {
    let trimmed = input.trim().replace('\\', "/");
    if trimmed.is_empty() {
        return None;
    }
    let path = Path::new(&trimmed);
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => out.push(part),
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

fn guess_bundle_name_from_path_or_source(source: &str, fallback: &str) -> String {
    let source_hint = Path::new(source);
    let mut raw_name = source_hint
        .file_name()
        .and_then(|v| v.to_str())
        .map(|v| v.trim_end_matches(".md"))
        .map(|v| v.trim_end_matches(".git"))
        .filter(|v| !v.is_empty())
        .unwrap_or(fallback)
        .to_string();
    if raw_name.eq_ignore_ascii_case("skill") {
        if let Some(parent_name) = source_hint
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|v| v.to_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            raw_name = parent_name.to_string();
        }
    }
    raw_name
}

fn finalize_imported_bundle(
    state: &AppState,
    bundle_dir: &Path,
    bundle_rel_dir: &str,
    source: &str,
    enabled: bool,
    skill_md: &str,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let plan = match detect_import_plan(skill_md, bundle_dir, bundle_rel_dir, source, enabled) {
        Ok(plan) => plan,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("analyze imported skill failed: {err}")),
                }),
            );
        }
    };

    let prompt_body_path = state.workspace_root.join(&plan.prompt_body_rel_path);
    if let Some(parent) = prompt_body_path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("create prompt directory failed: {err}")),
                }),
            );
        }
    }
    if let Err(err) = std::fs::write(
        &prompt_body_path,
        render_imported_skill_prompt(&plan, skill_md),
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write prompt file failed: {err}")),
            }),
        );
    }

    let mut registry_raw = match read_skills_registry_file(state) {
        Ok(raw) => raw,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read skills registry failed: {err}")),
                }),
            );
        }
    };
    if !registry_raw.ends_with('\n') && !registry_raw.is_empty() {
        registry_raw.push('\n');
    }
    registry_raw.push('\n');
    registry_raw.push_str(&render_imported_skill_registry_block(&plan));
    registry_raw.push('\n');
    if let Err(err) = write_skills_registry_file(state, &registry_raw) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write skills registry failed: {err}")),
            }),
        );
    }

    let reload = match reload_skill_views(state) {
        Ok(result) => result,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("reload skill views failed: {err}")),
                }),
            );
        }
    };

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "skill_name": plan.canonical_name,
                "display_name": plan.display_name,
                "description": plan.description,
                "external_kind": plan.external_kind,
                "bundle_dir": plan.bundle_rel_dir,
                "entry_file": plan.entry_file,
                "runtime": plan.runtime,
                "require_bins": plan.require_bins,
                "require_py_modules": plan.require_py_modules,
                "prompt_file": plan.registry_prompt_rel_path,
                "source": plan.source_url,
                "reload": reload
            })),
            error: None,
        }),
    )
}

async fn materialize_import_source(
    state: &AppState,
    source: &str,
    dest_dir: &Path,
) -> Result<String, String> {
    let normalized = normalize_remote_skill_source(source);
    let src_path = Path::new(&normalized);
    if src_path.exists() {
        if src_path.is_dir() {
            copy_dir_recursive(src_path, dest_dir)
                .map_err(|err| format!("copy local bundle failed: {err}"))?;
            let skill_md = dest_dir.join("SKILL.md");
            return std::fs::read_to_string(&skill_md)
                .map_err(|err| format!("read copied SKILL.md failed: {err}"));
        }
        if src_path.is_file() {
            std::fs::create_dir_all(dest_dir)
                .map_err(|err| format!("create import dir failed: {err}"))?;
            std::fs::copy(src_path, dest_dir.join("SKILL.md"))
                .map_err(|err| format!("copy local SKILL.md failed: {err}"))?;
            return std::fs::read_to_string(dest_dir.join("SKILL.md"))
                .map_err(|err| format!("read copied SKILL.md failed: {err}"));
        }
    }

    let res = state
        .http_client
        .get(&normalized)
        .send()
        .await
        .map_err(|err| format!("download skill source failed: {err}"))?;
    let status = res.status();
    let body = res
        .text()
        .await
        .map_err(|err| format!("read skill source body failed: {err}"))?;
    if !status.is_success() {
        return Err(format!(
            "download skill source returned {status}: {}",
            body.chars().take(200).collect::<String>()
        ));
    }
    std::fs::create_dir_all(dest_dir).map_err(|err| format!("create import dir failed: {err}"))?;
    std::fs::write(dest_dir.join("SKILL.md"), &body)
        .map_err(|err| format!("write downloaded SKILL.md failed: {err}"))?;
    Ok(body)
}

fn upsert_string_key_in_section(
    raw: &str,
    section_name: &str,
    key: &str,
    rendered_line: &str,
) -> String {
    let mut lines: Vec<String> = raw.lines().map(|s| s.to_string()).collect();
    let section_header = format!("[{section_name}]");
    let mut in_section = false;
    let mut section_seen = false;
    let mut inserted_or_replaced = false;
    let mut insert_index_in_section: Option<usize> = None;
    let mut section_end: Option<usize> = None;

    for idx in 0..lines.len() {
        let trimmed = lines[idx].trim();
        if trimmed == section_header {
            in_section = true;
            section_seen = true;
            insert_index_in_section = Some(idx + 1);
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed != section_header {
            if in_section {
                section_end = Some(idx);
                break;
            }
            continue;
        }
        if in_section && trimmed.starts_with(key) && trimmed.contains('=') {
            lines[idx] = rendered_line.to_string();
            inserted_or_replaced = true;
            break;
        }
    }

    if !inserted_or_replaced && section_seen {
        let idx = insert_index_in_section
            .or(section_end)
            .unwrap_or(lines.len());
        lines.insert(idx, rendered_line.to_string());
    }

    let mut out = lines.join("\n");
    if raw.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn llm_vendor_names() -> [&'static str; 8] {
    [
        "openai",
        "google",
        "anthropic",
        "grok",
        "deepseek",
        "qwen",
        "minimax",
        "custom",
    ]
}

fn collect_llm_vendor_info(value: &toml::Value) -> Vec<Value> {
    let mut vendors = Vec::new();
    let Some(llm) = value.get("llm").and_then(|v| v.as_table()) else {
        return vendors;
    };
    for vendor_name in llm_vendor_names() {
        let Some(vendor) = llm.get(vendor_name).and_then(|v| v.as_table()) else {
            continue;
        };
        let base_url = vendor
            .get("base_url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let default_model = vendor
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let api_key_configured = vendor
            .get("api_key")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let api_key_masked = vendor
            .get("api_key")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(mask_secret);
        let api_key = vendor
            .get("api_key")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        let api_format = if vendor_name == "minimax" {
            normalize_minimax_api_format(vendor.get("api_format").and_then(|v| v.as_str()))
        } else {
            String::new()
        };
        let mut models = vendor
            .get("models")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !default_model.is_empty() && !models.iter().any(|m| m == &default_model) {
            models.insert(0, default_model.clone());
        }
        vendors.push(json!({
            "name": vendor_name,
            "default_model": default_model,
            "models": models,
            "base_url": base_url,
            "api_format": api_format,
            "api_key": api_key,
            "api_key_configured": api_key_configured,
            "api_key_masked": api_key_masked
        }));
    }
    vendors
}

fn normalize_minimax_api_format(raw: Option<&str>) -> String {
    let fmt = raw.unwrap_or("").trim();
    if fmt.eq_ignore_ascii_case("anthropic") || fmt.eq_ignore_ascii_case("anthropic_claude") {
        "anthropic_claude".to_string()
    } else {
        "openai_compat".to_string()
    }
}

fn current_runtime_llm_info(state: &AppState) -> Value {
    if let Some(provider) = state.llm_providers.first() {
        let vendor = provider
            .config
            .name
            .strip_prefix("vendor-")
            .unwrap_or(provider.config.name.as_str())
            .to_string();
        return json!({
            "vendor": vendor,
            "model": provider.config.model,
            "provider_name": provider.config.name,
            "provider_type": provider.config.provider_type
        });
    }
    json!(null)
}

fn saved_llm_vendor_runtime_fields(
    parsed: &toml::Value,
    selected_vendor: &str,
) -> (String, String, String) {
    let section_key = format!("llm.{selected_vendor}");
    let vendor = parsed
        .get("llm")
        .and_then(|llm| llm.get(selected_vendor))
        .or_else(|| parsed.get(&section_key));
    let base_url = vendor
        .and_then(|v| v.get("base_url"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let api_key = vendor
        .and_then(|v| v.get("api_key"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let provider_type = if selected_vendor.trim().eq_ignore_ascii_case("minimax") {
        normalize_minimax_api_format(
            vendor
                .and_then(|v| v.get("api_format"))
                .and_then(|v| v.as_str()),
        )
    } else {
        String::new()
    };
    (base_url, api_key, provider_type)
}

fn llm_provider_type_for_vendor(selected_vendor: &str, vendor_api_format: Option<&str>) -> String {
    if selected_vendor.trim().eq_ignore_ascii_case("minimax") {
        normalize_minimax_api_format(vendor_api_format)
    } else if selected_vendor.trim().eq_ignore_ascii_case("google") {
        "google_gemini".to_string()
    } else if selected_vendor.trim().eq_ignore_ascii_case("anthropic") {
        "anthropic_claude".to_string()
    } else {
        "openai_compat".to_string()
    }
}

fn build_llm_test_runtime(
    selected_vendor: &str,
    selected_model: &str,
    vendor_base_url: &str,
    vendor_api_key: &str,
    vendor_api_format: Option<&str>,
) -> Result<Arc<LlmProviderRuntime>, String> {
    let provider_type = llm_provider_type_for_vendor(selected_vendor, vendor_api_format);
    let config = claw_core::config::LlmProviderConfig {
        name: format!("vendor-{}", selected_vendor.trim().to_ascii_lowercase()),
        provider_type,
        base_url: vendor_base_url.trim().to_string(),
        api_key: vendor_api_key.trim().to_string(),
        model: selected_model.trim().to_string(),
        priority: 1,
        timeout_seconds: 20,
        max_concurrency: 1,
        params: claw_core::config::LlmProviderParams::default(),
    };
    let client = crate::providers::build_llm_http_client(config.timeout_seconds)
        .map_err(|err| format!("build llm test client failed: {err}"))?;
    Ok(Arc::new(LlmProviderRuntime {
        config,
        client,
        semaphore: Arc::new(Semaphore::new(1)),
        breaker: Arc::new(crate::providers::CircuitBreaker::new()),
    }))
}

fn llm_runtime_differs(
    runtime_vendor: &str,
    runtime_model: &str,
    runtime_provider_type: &str,
    runtime_base_url: &str,
    runtime_api_key: &str,
    selected_vendor: &str,
    selected_model: &str,
    saved_provider_type: &str,
    saved_base_url: &str,
    saved_api_key: &str,
) -> bool {
    runtime_vendor.trim() != selected_vendor.trim()
        || runtime_model.trim() != selected_model.trim()
        || (selected_vendor.trim().eq_ignore_ascii_case("minimax")
            && runtime_provider_type.trim() != saved_provider_type.trim())
        || runtime_base_url.trim() != saved_base_url.trim()
        || runtime_api_key.trim() != saved_api_key.trim()
}

fn llm_restart_required(
    state: &AppState,
    parsed: &toml::Value,
    selected_vendor: &str,
    selected_model: &str,
) -> bool {
    let Some(provider) = state.llm_providers.first() else {
        return true;
    };
    let runtime_vendor = provider
        .config
        .name
        .strip_prefix("vendor-")
        .unwrap_or(provider.config.name.as_str());
    let (saved_base_url, saved_api_key, saved_provider_type) =
        saved_llm_vendor_runtime_fields(parsed, selected_vendor.trim());
    llm_runtime_differs(
        runtime_vendor,
        &provider.config.model,
        &provider.config.provider_type,
        &provider.config.base_url,
        &provider.config.api_key,
        selected_vendor,
        selected_model,
        &saved_provider_type,
        &saved_base_url,
        &saved_api_key,
    )
}

fn skills_restart_required(runtime_visible: &[String], effective_visible: &[String]) -> bool {
    let mut runtime_sorted = runtime_visible.to_vec();
    runtime_sorted.sort_unstable();
    let mut effective_sorted = effective_visible.to_vec();
    effective_sorted.sort_unstable();
    runtime_sorted != effective_sorted
}

fn collect_skills_baseline(value: &toml::Value, state: &AppState) -> Vec<String> {
    value
        .get("skills")
        .and_then(|v| v.get("skills_list"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| state.resolve_canonical_skill_name(s))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn collect_skill_switches(value: &toml::Value, state: &AppState) -> BTreeMap<String, bool> {
    let mut out = BTreeMap::new();
    let Some(tbl) = value
        .get("skills")
        .and_then(|v| v.get("skill_switches"))
        .and_then(|v| v.as_table())
    else {
        return out;
    };
    for (k, v) in tbl {
        let canonical = state.resolve_canonical_skill_name(k);
        if hide_skill_in_ui(state, &canonical) {
            continue;
        }
        if let Some(b) = v.as_bool() {
            out.insert(canonical, b);
        }
    }
    out
}

fn compute_effective_enabled(
    baseline: &[String],
    switches: &BTreeMap<String, bool>,
    state: &AppState,
) -> Vec<String> {
    let mut set: BTreeMap<String, bool> = BTreeMap::new();
    for skill in baseline {
        set.insert(state.resolve_canonical_skill_name(skill), true);
    }
    if let Some(registry) = state.get_skills_registry() {
        for skill in registry.enabled_names() {
            set.insert(state.resolve_canonical_skill_name(&skill), true);
        }
    }
    for (k, v) in switches {
        if *v {
            set.insert(state.resolve_canonical_skill_name(k), true);
        } else {
            set.remove(&state.resolve_canonical_skill_name(k));
        }
    }
    set.into_keys().collect()
}

fn render_switches_inline_table(switches: &BTreeMap<String, bool>) -> String {
    if switches.is_empty() {
        return "skill_switches = {}".to_string();
    }
    let pairs = switches
        .iter()
        .map(|(k, v)| format!("{k} = {v}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("skill_switches = {{ {pairs} }}")
}

fn upsert_skill_switches_line(raw: &str, rendered_line: &str) -> String {
    let mut lines: Vec<String> = raw.lines().map(|s| s.to_string()).collect();
    let mut in_skills = false;
    let mut inserted_or_replaced = false;
    let mut skills_section_seen = false;
    let mut insert_index_in_skills: Option<usize> = None;
    let mut skills_section_end: Option<usize> = None;

    for idx in 0..lines.len() {
        let trimmed = lines[idx].trim();
        if trimmed == "[skills]" {
            in_skills = true;
            skills_section_seen = true;
            insert_index_in_skills = Some(idx + 1);
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed != "[skills]" {
            if in_skills {
                skills_section_end = Some(idx);
                break;
            }
            continue;
        }
        if in_skills && trimmed.starts_with("skill_switches") && trimmed.contains('=') {
            lines[idx] = rendered_line.to_string();
            inserted_or_replaced = true;
            break;
        }
        if in_skills && insert_index_in_skills.is_none() && !trimmed.is_empty() {
            insert_index_in_skills = Some(idx);
        }
        if in_skills && trimmed.starts_with("skills_list") && insert_index_in_skills.is_none() {
            insert_index_in_skills = Some(idx);
        }
    }

    if !inserted_or_replaced && skills_section_seen {
        let idx = insert_index_in_skills
            .or(skills_section_end)
            .unwrap_or(lines.len());
        lines.insert(idx, rendered_line.to_string());
    }

    let mut out = lines.join("\n");
    if raw.ends_with('\n') {
        out.push('\n');
    }
    out
}

async fn get_skills_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let parsed = match read_skill_config_file(&state) {
        Ok((_, v)) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read skills config failed: {err}")),
                }),
            );
        }
    };
    let baseline = collect_skills_baseline(&parsed, &state);
    let switches = collect_skill_switches(&parsed, &state);
    let mut baseline_visible = baseline
        .iter()
        .filter(|s| !hide_skill_in_ui(&state, s))
        .cloned()
        .collect::<Vec<_>>();
    baseline_visible.sort_unstable();
    let mut runtime_visible = state
        .get_skills_list()
        .iter()
        .filter(|s| !hide_skill_in_ui(&state, s))
        .cloned()
        .collect::<Vec<_>>();
    runtime_visible.sort_unstable();
    let managed = {
        let mut set: BTreeMap<String, bool> = BTreeMap::new();
        for s in &baseline_visible {
            set.insert(s.clone(), true);
        }
        for s in switches.keys() {
            set.insert(s.clone(), true);
        }
        for s in runtime_visible.iter() {
            set.insert(s.clone(), true);
        }
        set.into_keys().collect::<Vec<_>>()
    };
    let mut effective = compute_effective_enabled(&baseline, &switches, &state);
    effective.retain(|s| !hide_skill_in_ui(&state, s));
    let restart_required = skills_restart_required(&runtime_visible, &effective);
    let base_skill_names: Vec<String> = claw_core::config::base_skill_names()
        .iter()
        .map(|s| s.to_string())
        .collect();
    let external_skill_names = state
        .get_skills_registry()
        .as_ref()
        .map(|registry| {
            registry
                .all_names()
                .into_iter()
                .filter(|name| {
                    !hide_skill_in_ui(&state, name)
                        && registry
                            .get(name)
                            .map(|entry| entry.kind == SkillKind::External)
                            .unwrap_or(false)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "config_path": "configs/config.toml",
                "skills_list": baseline_visible,
                "skill_switches": switches,
                "managed_skills": managed,
                "base_skill_names": base_skill_names,
                "external_skill_names": external_skill_names,
                "effective_enabled_skills_preview": effective,
                "runtime_enabled_skills": runtime_visible,
                "restart_required": restart_required
            })),
            error: None,
        }),
    )
}

async fn get_llm_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let parsed = match read_skill_config_file(&state) {
        Ok((_, v)) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read llm config failed: {err}")),
                }),
            );
        }
    };
    let llm = parsed.get("llm").and_then(|v| v.as_table());
    let selected_vendor = llm
        .and_then(|tbl| tbl.get("selected_vendor"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let selected_model = llm
        .and_then(|tbl| tbl.get("selected_model"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let vendors = collect_llm_vendor_info(&parsed);
    let restart_required = llm_restart_required(&state, &parsed, &selected_vendor, &selected_model);
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "config_path": "configs/config.toml",
                "selected_vendor": selected_vendor,
                "selected_model": selected_model,
                "vendors": vendors,
                "runtime": current_runtime_llm_info(&state),
                "restart_required": restart_required
            })),
            error: None,
        }),
    )
}

async fn update_llm_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpdateLlmConfigRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let selected_vendor = req.selected_vendor.trim().to_ascii_lowercase();
    let selected_model = req.selected_model.trim().to_string();
    if selected_vendor.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("selected_vendor is required".to_string()),
            }),
        );
    }
    if selected_model.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("selected_model is required".to_string()),
            }),
        );
    }

    let (raw, parsed) = match read_skill_config_file(&state) {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read llm config failed: {err}")),
                }),
            );
        }
    };
    let vendors = collect_llm_vendor_info(&parsed);
    let Some(vendor_info) = vendors.iter().find(|item| {
        item.get("name")
            .and_then(|v| v.as_str())
            .map(|name| name.eq_ignore_ascii_case(&selected_vendor))
            .unwrap_or(false)
    }) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("unsupported vendor: {selected_vendor}")),
            }),
        );
    };

    let allowed_models = vendor_info
        .get("models")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if selected_vendor != "custom"
        && !allowed_models.is_empty()
        && !allowed_models.iter().any(|m| m == &selected_model)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!(
                    "model is not in the configured pool for vendor {selected_vendor}: {selected_model}"
                )),
            }),
        );
    }

    let vendor_base_url = req.vendor_base_url.as_deref().map(str::trim).unwrap_or("");
    if vendor_base_url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("vendor_base_url is required".to_string()),
            }),
        );
    }

    let updated_vendor = upsert_string_key_in_section(
        &raw,
        "llm",
        "selected_vendor",
        &format!("selected_vendor = {:?}", selected_vendor),
    );
    let updated_raw = upsert_string_key_in_section(
        &updated_vendor,
        "llm",
        "selected_model",
        &format!("selected_model = {:?}", selected_model),
    );
    let updated_vendor_base_url = upsert_string_key_in_section(
        &updated_raw,
        &format!("llm.{selected_vendor}"),
        "base_url",
        &format!("base_url = {:?}", vendor_base_url),
    );
    let updated_vendor_model = upsert_string_key_in_section(
        &updated_vendor_base_url,
        &format!("llm.{selected_vendor}"),
        "model",
        &format!("model = {:?}", selected_model),
    );
    let vendor_api_key = req.vendor_api_key.as_deref().map(str::trim).unwrap_or("");
    let updated_api_key = upsert_string_key_in_section(
        &updated_vendor_model,
        &format!("llm.{selected_vendor}"),
        "api_key",
        &format!("api_key = {:?}", vendor_api_key),
    );
    let final_updated = if selected_vendor == "minimax" {
        let vendor_api_format = normalize_minimax_api_format(req.vendor_api_format.as_deref());
        upsert_string_key_in_section(
            &updated_api_key,
            "llm.minimax",
            "api_format",
            &format!("api_format = {:?}", vendor_api_format),
        )
    } else {
        updated_api_key
    };
    if let Err(err) = write_runtime_config_file(&state, &final_updated) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write llm config failed: {err}")),
            }),
        );
    }
    let updated_parsed = toml::from_str::<toml::Value>(&final_updated).unwrap_or(parsed);
    let restart_required =
        llm_restart_required(&state, &updated_parsed, &selected_vendor, &selected_model);

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "config_path": "configs/config.toml",
                "selected_vendor": selected_vendor,
                "selected_model": selected_model,
                "runtime": current_runtime_llm_info(&state),
                "restart_required": restart_required
            })),
            error: None,
        }),
    )
}

async fn test_llm_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpdateLlmConfigRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let selected_vendor = req.selected_vendor.trim().to_ascii_lowercase();
    let selected_model = req.selected_model.trim().to_string();
    if selected_vendor.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("selected_vendor is required".to_string()),
            }),
        );
    }
    if selected_model.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("selected_model is required".to_string()),
            }),
        );
    }

    let parsed = match read_skill_config_file(&state) {
        Ok((_, parsed)) => parsed,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read llm config failed: {err}")),
                }),
            );
        }
    };
    let vendors = collect_llm_vendor_info(&parsed);
    let Some(vendor_info) = vendors.iter().find(|item| {
        item.get("name")
            .and_then(|v| v.as_str())
            .map(|name| name.eq_ignore_ascii_case(&selected_vendor))
            .unwrap_or(false)
    }) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("unsupported vendor: {selected_vendor}")),
            }),
        );
    };

    let allowed_models = vendor_info
        .get("models")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if selected_vendor != "custom"
        && !allowed_models.is_empty()
        && !allowed_models.iter().any(|m| m == &selected_model)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!(
                    "model is not in the configured pool for vendor {selected_vendor}: {selected_model}"
                )),
            }),
        );
    }

    let vendor_base_url = req.vendor_base_url.as_deref().map(str::trim).unwrap_or("");
    if vendor_base_url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("vendor_base_url is required".to_string()),
            }),
        );
    }
    let vendor_api_key = req.vendor_api_key.as_deref().map(str::trim).unwrap_or("");
    let provider = match build_llm_test_runtime(
        &selected_vendor,
        &selected_model,
        vendor_base_url,
        vendor_api_key,
        req.vendor_api_format.as_deref(),
    ) {
        Ok(provider) => provider,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(err),
                }),
            );
        }
    };

    match crate::call_provider_with_retry(provider.clone(), LLM_CONNECTIVITY_TEST_PROMPT).await {
        Ok(resp) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({
                    "success": true,
                    "vendor": selected_vendor,
                    "model": selected_model,
                    "provider_type": provider.config.provider_type,
                    "message": crate::i18n_t_with_default_vars(
                        &state,
                        "clawd.msg.provider_connection_test_ok",
                        "连接测试通过：{provider_name} 可正常响应。",
                        &[("provider_name", &provider.config.name)],
                    ),
                    "response_text": resp.text,
                })),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("llm connectivity test failed: {err}")),
            }),
        ),
    }
}

async fn update_skills_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpdateSkillsConfigRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let (raw, parsed) = match read_skill_config_file(&state) {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read skills config failed: {err}")),
                }),
            );
        }
    };
    let baseline = collect_skills_baseline(&parsed, &state);
    let core_skills = claw_core::config::core_skills_always_enabled();
    let mut switches = BTreeMap::new();
    for (k, v) in req.skill_switches {
        let skill = state.resolve_canonical_skill_name(k.trim());
        if skill.is_empty() || hide_skill_in_ui(&state, &skill) {
            continue;
        }
        let is_core = core_skills.iter().any(|s| *s == skill);
        switches.insert(skill, if is_core { true } else { v });
    }
    let rendered = render_switches_inline_table(&switches);
    let updated = upsert_skill_switches_line(&raw, &rendered);
    if let Err(err) = write_runtime_config_file(&state, &updated) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write skills config failed: {err}")),
            }),
        );
    }
    let effective = compute_effective_enabled(&baseline, &switches, &state);
    let mut effective_visible = effective
        .iter()
        .filter(|s| !hide_skill_in_ui(&state, s))
        .cloned()
        .collect::<Vec<_>>();
    effective_visible.sort_unstable();
    let mut runtime_visible = state
        .get_skills_list()
        .iter()
        .filter(|s| !hide_skill_in_ui(&state, s))
        .cloned()
        .collect::<Vec<_>>();
    runtime_visible.sort_unstable();
    let restart_required = skills_restart_required(&runtime_visible, &effective_visible);
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "config_path": "configs/config.toml",
                "skill_switches": switches,
                "effective_enabled_skills_preview": effective,
                "restart_required": restart_required
            })),
            error: None,
        }),
    )
}

async fn uninstall_external_skill(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UninstallExternalSkillRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let skill_name = state.resolve_canonical_skill_name(req.skill_name.trim());
    if skill_name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("skill_name is required".to_string()),
            }),
        );
    }

    let Some(registry) = state.get_skills_registry() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("skills registry is not available".to_string()),
            }),
        );
    };
    let Some(entry) = registry.get(&skill_name).cloned() else {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("unknown skill: {skill_name}")),
            }),
        );
    };
    if entry.kind != SkillKind::External {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only imported external skills can be uninstalled here".to_string()),
            }),
        );
    }

    let registry_raw = match read_skills_registry_file(&state) {
        Ok(raw) => raw,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read skills registry failed: {err}")),
                }),
            );
        }
    };
    let (updated_registry, removed_from_registry) =
        remove_skill_registry_block(&registry_raw, &skill_name);
    if !removed_from_registry {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("skill registry block not found for {skill_name}")),
            }),
        );
    }
    if let Err(err) = write_skills_registry_file(&state, &updated_registry) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write skills registry failed: {err}")),
            }),
        );
    }

    let mut removed_bundle = false;
    if let Some(bundle_rel) = entry.external_bundle_dir.as_deref() {
        let bundle_path = if Path::new(bundle_rel).is_absolute() {
            PathBuf::from(bundle_rel)
        } else {
            state.workspace_root.join(bundle_rel)
        };
        let allowed_root = state.workspace_root.join("third_party");
        if bundle_path.starts_with(&allowed_root) && bundle_path.exists() {
            match std::fs::remove_dir_all(&bundle_path) {
                Ok(_) => removed_bundle = true,
                Err(err) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiResponse {
                            ok: false,
                            data: None,
                            error: Some(format!("remove imported bundle failed: {err}")),
                        }),
                    );
                }
            }
        }
    }

    let mut removed_prompt = false;
    let registry_prompt_rel_path = entry.prompt_file.trim();
    if !registry_prompt_rel_path.is_empty() {
        let prompt_body_path = if let Some(prompt_body_rel) =
            prompt_layers::canonical_skill_prompt_body_rel_path(registry_prompt_rel_path)
        {
            state.workspace_root.join(prompt_body_rel)
        } else if Path::new(registry_prompt_rel_path).is_absolute() {
            PathBuf::from(registry_prompt_rel_path)
        } else {
            state.workspace_root.join(registry_prompt_rel_path)
        };
        match remove_managed_prompt_file(&prompt_body_path) {
            Ok(value) => removed_prompt = value,
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("remove prompt file failed: {err}")),
                    }),
                );
            }
        }
    }

    let (runtime_raw, _) = match read_skill_config_file(&state) {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read skills config failed: {err}")),
                }),
            );
        }
    };
    let updated_runtime = remove_runtime_skill_switch(&runtime_raw, &state, &skill_name);
    if let Err(err) = write_runtime_config_file(&state, &updated_runtime) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write skills config failed: {err}")),
            }),
        );
    }

    let reload = match reload_skill_views(&state) {
        Ok(result) => result,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("reload skill views failed: {err}")),
                }),
            );
        }
    };

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "skill_name": skill_name,
                "removed_bundle": removed_bundle,
                "removed_prompt": removed_prompt,
                "reload": reload,
            })),
            error: None,
        }),
    )
}

async fn whatsapp_web_login_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let base = state
        .whatsapp_web_bridge_base_url
        .trim()
        .trim_end_matches('/');
    if base.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("whatsapp_web.bridge_base_url is empty".to_string()),
            }),
        );
    }
    let url = format!("{base}/v1/login-status");
    let resp = match state.http_client.get(&url).send().await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("request bridge login status failed: {err}")),
                }),
            );
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return (
            StatusCode::BAD_GATEWAY,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!(
                    "bridge login status failed: status={status} body={body}"
                )),
            }),
        );
    }
    let data = match resp.json::<Value>().await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("decode bridge login status failed: {err}")),
                }),
            );
        }
    };
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(data),
            error: None,
        }),
    )
}

#[derive(Debug, Deserialize, Default)]
struct WechatQrStartRequest {
    #[serde(default)]
    force: bool,
}

#[derive(Debug, Deserialize)]
struct WechatQrWaitRequest {
    session_key: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

fn wechatd_base_url(state: &AppState) -> Result<String, (StatusCode, Json<ApiResponse<Value>>)> {
    let config = load_wechat_config_response(state).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("read wechat config failed: {err}")),
            }),
        )
    })?;
    let listen = config.listen.trim();
    if !config.enabled || listen.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("wechat daemon is not configured".to_string()),
            }),
        ));
    }
    let host_port = if let Some(rest) = listen.strip_prefix("0.0.0.0:") {
        format!("127.0.0.1:{rest}")
    } else if let Some(rest) = listen.strip_prefix("[::]:") {
        format!("127.0.0.1:{rest}")
    } else {
        listen.to_string()
    };
    Ok(format!("http://{host_port}"))
}

async fn wechat_login_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let Ok(base) = wechatd_base_url(&state) else {
        return wechatd_base_url(&state).err().unwrap();
    };
    let url = format!("{}/login/status", base.trim_end_matches('/'));
    let resp = match state.http_client.get(&url).send().await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("request wechat login status failed: {err}")),
                }),
            );
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return (
            StatusCode::BAD_GATEWAY,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!(
                    "wechat login status failed: status={status} body={body}"
                )),
            }),
        );
    }
    let data = match resp.json::<Value>().await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("decode wechat login status failed: {err}")),
                }),
            );
        }
    };
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(data),
            error: None,
        }),
    )
}

async fn wechat_login_qr_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<WechatQrStartRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let Ok(base) = wechatd_base_url(&state) else {
        return wechatd_base_url(&state).err().unwrap();
    };
    let url = format!("{}/login/qr/start", base.trim_end_matches('/'));
    let resp = match state
        .http_client
        .post(&url)
        .json(&json!({ "force": req.force }))
        .send()
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("request wechat QR start failed: {err}")),
                }),
            );
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return (
            StatusCode::BAD_GATEWAY,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!(
                    "wechat QR start failed: status={status} body={body}"
                )),
            }),
        );
    }
    let data = match resp.json::<Value>().await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("decode wechat QR start failed: {err}")),
                }),
            );
        }
    };
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(data),
            error: None,
        }),
    )
}

async fn wechat_login_qr_wait(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<WechatQrWaitRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let Ok(base) = wechatd_base_url(&state) else {
        return wechatd_base_url(&state).err().unwrap();
    };
    let url = format!("{}/login/qr/wait", base.trim_end_matches('/'));
    let resp = match state
        .http_client
        .post(&url)
        .json(&json!({
            "session_key": req.session_key,
            "timeout_ms": req.timeout_ms.unwrap_or(1_500)
        }))
        .send()
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("request wechat QR wait failed: {err}")),
                }),
            );
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return (
            StatusCode::BAD_GATEWAY,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!(
                    "wechat QR wait failed: status={status} body={body}"
                )),
            }),
        );
    }
    let data = match resp.json::<Value>().await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("decode wechat QR wait failed: {err}")),
                }),
            );
        }
    };
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(data),
            error: None,
        }),
    )
}

async fn whatsapp_web_logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let base = state
        .whatsapp_web_bridge_base_url
        .trim()
        .trim_end_matches('/');
    if base.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("whatsapp_web.bridge_base_url is empty".to_string()),
            }),
        );
    }
    let url = format!("{base}/v1/logout");
    let resp = match state.http_client.post(&url).send().await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("request bridge logout failed: {err}")),
                }),
            );
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return (
            StatusCode::BAD_GATEWAY,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("bridge logout failed: status={status} body={body}")),
            }),
        );
    }
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({ "ok": true })),
            error: None,
        }),
    )
}

async fn local_interaction_context(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<LocalInteractionContext>>) {
    match require_ui_identity(&state, &headers) {
        Ok(identity) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(LocalInteractionContext {
                    user_id: identity.user_id,
                    chat_id: identity.chat_id,
                    role: identity.role,
                }),
                error: None,
            }),
        ),
        Err((status, Json(resp))) => (
            status,
            Json(ApiResponse {
                ok: resp.ok,
                data: None,
                error: resp.error,
            }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_workspace_root() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustclaw-ui-routes-{unique}"));
        std::fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn write_workspace_and_mounted_file_writes_both_copies() {
        let root = temp_workspace_root();
        let relative = "configs/config.toml";
        let raw = "[llm]\nprovider = \"minimax\"\n";

        write_workspace_and_mounted_file(&root, relative, raw).expect("write config");

        let active = std::fs::read_to_string(root.join(relative)).expect("read active");
        let mounted =
            std::fs::read_to_string(root.join("docker/config/config.toml")).expect("read mounted");
        assert_eq!(active, raw);
        assert_eq!(mounted, raw);
    }

    #[test]
    fn write_workspace_and_mounted_file_writes_channel_copy_to_mounted_channels_dir() {
        let root = temp_workspace_root();
        let relative = "configs/channels/wechat.toml";
        let raw = "[wechat]\nenabled = true\n";

        write_workspace_and_mounted_file(&root, relative, raw).expect("write config");

        let active = std::fs::read_to_string(root.join(relative)).expect("read active");
        let mounted = std::fs::read_to_string(root.join("docker/config/channels/wechat.toml"))
            .expect("read mounted");
        assert_eq!(active, raw);
        assert_eq!(mounted, raw);
    }

    #[test]
    fn update_feishu_config_raw_preserves_template_comments_and_updates_only_keys() {
        let output = update_feishu_config_raw_preserving_format(
            FEISHU_CONFIG_TEMPLATE,
            "cli_test_app",
            "secret_test",
        );
        assert!(output.contains("# Feishu（中国站）应用机器人通道配置"));
        assert!(output.contains("# 入站模式：webhook | long_connection"));
        assert!(output.contains("enabled = true"));
        assert!(output.contains("app_id = \"cli_test_app\""));
        assert!(output.contains("app_secret = \"secret_test\""));
        assert!(output.contains("image_inbox_dir = \"data/feishud/image\""));
    }

    #[test]
    fn update_feishu_config_raw_keeps_unrelated_lines_when_updating_existing_file() {
        let raw = r#"# header
[feishu]
# before
app_id = ""
app_secret = ""
enabled = false
custom_keep = "yes"
"#;
        let output = update_feishu_config_raw_preserving_format(
            raw,
            "cli_keep_format",
            "secret_keep_format",
        );
        assert!(output.contains("# before"));
        assert!(output.contains("custom_keep = \"yes\""));
        assert!(output.contains("app_id = \"cli_keep_format\""));
        assert!(output.contains("app_secret = \"secret_keep_format\""));
        assert!(output.contains("enabled = true"));
    }

    #[test]
    fn llm_runtime_differs_when_only_api_key_changes() {
        assert!(llm_runtime_differs(
            "minimax",
            "MiniMax-M2.7",
            "openai_compat",
            "https://api.minimax.io/v1",
            "old-key",
            "minimax",
            "MiniMax-M2.7",
            "openai_compat",
            "https://api.minimax.io/v1",
            "new-key",
        ));
    }

    #[test]
    fn llm_runtime_differs_when_only_base_url_changes() {
        assert!(llm_runtime_differs(
            "minimax",
            "MiniMax-M2.7",
            "openai_compat",
            "https://api.minimax.io/v1",
            "same-key",
            "minimax",
            "MiniMax-M2.7",
            "openai_compat",
            "https://api.minimax.cn/v1",
            "same-key",
        ));
    }

    #[test]
    fn llm_runtime_differs_is_false_when_runtime_matches_saved_config() {
        assert!(!llm_runtime_differs(
            "minimax",
            "MiniMax-M2.7",
            "openai_compat",
            "https://api.minimax.io/v1",
            "same-key",
            "minimax",
            "MiniMax-M2.7",
            "openai_compat",
            "https://api.minimax.io/v1",
            "same-key",
        ));
    }

    #[test]
    fn llm_runtime_differs_when_only_minimax_provider_type_changes() {
        assert!(llm_runtime_differs(
            "minimax",
            "MiniMax-M2.7",
            "anthropic_claude",
            "https://api.minimax.io/v1",
            "same-key",
            "minimax",
            "MiniMax-M2.7",
            "openai_compat",
            "https://api.minimax.io/v1",
            "same-key",
        ));
    }

    #[test]
    fn collect_llm_vendor_info_defaults_minimax_api_format_to_openai() {
        let parsed = toml::from_str::<toml::Value>(
            r#"
[llm]
selected_vendor = "minimax"
selected_model = "MiniMax-M2.7"

[llm.minimax]
api_key = ""
base_url = "https://api.minimax.io/v1"
model = "MiniMax-M2.7"
models = ["MiniMax-M2.7"]
"#,
        )
        .expect("parse");

        let vendors = collect_llm_vendor_info(&parsed);
        let minimax = vendors
            .iter()
            .find(|vendor| vendor.get("name").and_then(|v| v.as_str()) == Some("minimax"))
            .expect("minimax vendor");

        assert_eq!(
            minimax.get("api_format").and_then(|v| v.as_str()),
            Some("openai_compat")
        );
    }
}
