fn workspace_update_state() -> Arc<Mutex<WorkspaceUpdateStatus>> {
    WORKSPACE_UPDATE_STATE
        .get_or_init(|| Arc::new(Mutex::new(WorkspaceUpdateStatus::default())))
        .clone()
}

fn workspace_update_control() -> Arc<Mutex<WorkspaceUpdateControl>> {
    WORKSPACE_UPDATE_CONTROL
        .get_or_init(|| Arc::new(Mutex::new(WorkspaceUpdateControl::default())))
        .clone()
}

fn hide_skill_in_ui(_state: &AppState, _name: &str) -> bool {
    false
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
    state
        .skill_rt
        .workspace_root
        .join("configs/channels/telegram.toml")
}

fn wechat_config_path(state: &AppState) -> PathBuf {
    state
        .skill_rt
        .workspace_root
        .join("configs/channels/wechat.toml")
}

fn feishu_config_path(state: &AppState) -> PathBuf {
    state
        .skill_rt
        .workspace_root
        .join("configs/channels/feishu.toml")
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
