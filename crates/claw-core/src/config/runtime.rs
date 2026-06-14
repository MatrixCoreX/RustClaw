use super::*;
use std::env;
use std::path::Path;

impl AppConfig {
    pub fn load(path: &str) -> Result<Self, config::ConfigError> {
        let base_path = Path::new(path);
        let base_dir = base_path.parent().unwrap_or_else(|| Path::new("."));
        let cfg = config::Config::builder()
            .add_source(config::File::with_name(path))
            // Optional split channel configs.
            .add_source(config::File::from(base_dir.join("channels/telegram.toml")).required(false))
            // Legacy mixed WhatsApp config (kept for backward compatibility).
            .add_source(config::File::from(base_dir.join("channels/whatsapp.toml")).required(false))
            // Split WhatsApp configs.
            .add_source(
                config::File::from(base_dir.join("channels/whatsapp-cloud.toml")).required(false),
            )
            .add_source(
                config::File::from(base_dir.join("channels/whatsapp-web.toml")).required(false),
            )
            .add_source(config::File::from(base_dir.join("channels/webd.toml")).required(false))
            .build()?;
        let mut app: AppConfig = cfg.try_deserialize()?;

        // Image skill config must come only from configs/image.toml, never from configs/config.toml.
        app.image_vision = ImageSkillConfig::default();
        app.image_generation = ImageSkillConfig::default();
        app.image_edit = ImageSkillConfig::default();

        let image_cfg: SplitImageConfig = config::Config::builder()
            .add_source(config::File::from(base_dir.join("image.toml")).required(false))
            .build()?
            .try_deserialize()?;
        app.image_vision = image_cfg.image_vision;
        app.image_generation = image_cfg.image_generation;
        app.image_edit = image_cfg.image_edit;
        apply_env_overrides(&mut app);

        Ok(app)
    }

    pub fn telegram_runtime_bots(&self) -> Vec<ResolvedTelegramBotConfig> {
        let mut bots = Vec::new();

        if !self.telegram.bot_token.trim().is_empty() {
            bots.push(ResolvedTelegramBotConfig {
                name: "primary".to_string(),
                bot_token: self.telegram.bot_token.trim().to_string(),
                agent_id: if self.telegram.agent_id.trim().is_empty() {
                    default_agent_id().to_string()
                } else {
                    self.telegram.agent_id.trim().to_string()
                },
                allowlist: self.telegram.allowlist.clone(),
                access_mode: if self.telegram.access_mode.trim().is_empty() {
                    default_telegram_access_mode()
                } else {
                    self.telegram.access_mode.trim().to_string()
                },
                allowed_usernames: self.telegram.allowed_usernames.clone(),
                language: self.telegram.language.clone(),
                i18n_path: self.telegram.i18n_path.clone(),
                quick_result_wait_seconds: self.telegram.quick_result_wait_seconds,
                task_delivery_timeout_seconds: self.telegram.task_delivery_timeout_seconds,
            });
        }

        for (index, bot) in self.telegram.bots.iter().enumerate() {
            let token = bot.bot_token.trim();
            if token.is_empty() {
                continue;
            }
            let preferred_name = if bot.name.trim().is_empty() {
                format!("bot-{}", index + 1)
            } else {
                bot.name.trim().to_string()
            };
            let name = unique_telegram_bot_name(&bots, &preferred_name, index + 1);
            let preferred_agent_id = bot.agent_id.trim();
            bots.push(ResolvedTelegramBotConfig {
                name,
                bot_token: token.to_string(),
                agent_id: if preferred_agent_id.is_empty() {
                    default_agent_id().to_string()
                } else {
                    preferred_agent_id.to_string()
                },
                allowlist: if bot.allowlist.is_empty() {
                    self.telegram.allowlist.clone()
                } else {
                    bot.allowlist.clone()
                },
                access_mode: if bot.access_mode.trim().is_empty() {
                    if self.telegram.access_mode.trim().is_empty() {
                        default_telegram_access_mode()
                    } else {
                        self.telegram.access_mode.trim().to_string()
                    }
                } else {
                    bot.access_mode.trim().to_string()
                },
                allowed_usernames: if bot.allowed_usernames.is_empty() {
                    self.telegram.allowed_usernames.clone()
                } else {
                    bot.allowed_usernames.clone()
                },
                language: if bot.language.trim().is_empty() {
                    self.telegram.language.clone()
                } else {
                    bot.language.trim().to_string()
                },
                i18n_path: if bot.i18n_path.trim().is_empty() {
                    self.telegram.i18n_path.clone()
                } else {
                    bot.i18n_path.trim().to_string()
                },
                quick_result_wait_seconds: bot.quick_result_wait_seconds,
                task_delivery_timeout_seconds: bot.task_delivery_timeout_seconds,
            });
        }

        let compat_token = self.telegram_bot.bot_token.trim();
        if self.telegram_bot.enabled
            && !compat_token.is_empty()
            && !bots.iter().any(|bot| bot.bot_token == compat_token)
        {
            bots.push(ResolvedTelegramBotConfig {
                name: unique_telegram_bot_name(&bots, "telegram-bot", bots.len() + 1),
                bot_token: compat_token.to_string(),
                agent_id: default_agent_id().to_string(),
                allowlist: if self.telegram_bot.allowlist.is_empty() {
                    self.telegram.allowlist.clone()
                } else {
                    self.telegram_bot.allowlist.clone()
                },
                access_mode: if self.telegram_bot.access_mode.trim().is_empty() {
                    if self.telegram.access_mode.trim().is_empty() {
                        default_telegram_access_mode()
                    } else {
                        self.telegram.access_mode.trim().to_string()
                    }
                } else {
                    self.telegram_bot.access_mode.trim().to_string()
                },
                allowed_usernames: if self.telegram_bot.allowed_usernames.is_empty() {
                    self.telegram.allowed_usernames.clone()
                } else {
                    self.telegram_bot.allowed_usernames.clone()
                },
                language: if self.telegram_bot.language.trim().is_empty() {
                    self.telegram.language.clone()
                } else {
                    self.telegram_bot.language.trim().to_string()
                },
                i18n_path: if self.telegram_bot.i18n_path.trim().is_empty() {
                    self.telegram.i18n_path.clone()
                } else {
                    self.telegram_bot.i18n_path.trim().to_string()
                },
                quick_result_wait_seconds: self.telegram_bot.quick_result_wait_seconds,
                task_delivery_timeout_seconds: self.telegram_bot.task_delivery_timeout_seconds,
            });
        }

        bots
    }

    pub fn normalized_agents(&self) -> Vec<AgentConfig> {
        let mut agents = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (index, agent) in self.agents.iter().enumerate() {
            let preferred_id = if agent.id.trim().is_empty() {
                if index == 0 {
                    default_agent_id().to_string()
                } else {
                    format!("agent-{}", index + 1)
                }
            } else {
                agent.id.trim().to_string()
            };
            if !seen.insert(preferred_id.clone()) {
                continue;
            }
            agents.push(AgentConfig {
                id: preferred_id.clone(),
                name: if agent.name.trim().is_empty() {
                    if preferred_id == default_agent_id() {
                        "Main".to_string()
                    } else {
                        preferred_id.clone()
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

        if !seen.contains(&default_agent_id()) {
            agents.insert(
                0,
                AgentConfig {
                    id: default_agent_id().to_string(),
                    name: "Main".to_string(),
                    ..AgentConfig::default()
                },
            );
        }

        agents
    }
}

fn env_non_empty(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn apply_string_env(target: &mut String, key: &str) {
    if let Some(value) = env_non_empty(key) {
        *target = value;
    }
}

fn apply_llm_vendor_api_key_env(target: &mut Option<LlmVendorConfig>, key: &str) {
    if let (Some(value), Some(cfg)) = (env_non_empty(key), target.as_mut()) {
        cfg.api_key = value;
    }
}

fn apply_env_overrides(app: &mut AppConfig) {
    apply_llm_vendor_api_key_env(&mut app.llm.openai, "OPENAI_API_KEY");
    apply_llm_vendor_api_key_env(&mut app.llm.google, "GOOGLE_API_KEY");
    apply_llm_vendor_api_key_env(&mut app.llm.anthropic, "ANTHROPIC_API_KEY");
    apply_llm_vendor_api_key_env(&mut app.llm.grok, "GROK_API_KEY");
    apply_llm_vendor_api_key_env(&mut app.llm.deepseek, "DEEPSEEK_API_KEY");
    apply_llm_vendor_api_key_env(&mut app.llm.qwen, "QWEN_API_KEY");
    apply_llm_vendor_api_key_env(&mut app.llm.minimax, "MINIMAX_API_KEY");
    apply_llm_vendor_api_key_env(&mut app.llm.mimo, "XIAOMI_API_KEY");
    apply_llm_vendor_api_key_env(&mut app.llm.mimo, "MIMO_API_KEY");
    apply_llm_vendor_api_key_env(&mut app.llm.custom, "CUSTOM_API_KEY");

    apply_string_env(&mut app.telegram.bot_token, "TELEGRAM_BOT_TOKEN");
    apply_string_env(&mut app.telegram_bot.bot_token, "TELEGRAM_BOT_TOKEN");

    apply_string_env(&mut app.whatsapp.access_token, "WHATSAPP_ACCESS_TOKEN");
    apply_string_env(&mut app.whatsapp.app_secret, "WHATSAPP_APP_SECRET");
    apply_string_env(&mut app.whatsapp.verify_token, "WHATSAPP_VERIFY_TOKEN");
    apply_string_env(
        &mut app.whatsapp.phone_number_id,
        "WHATSAPP_PHONE_NUMBER_ID",
    );

    apply_string_env(
        &mut app.whatsapp_cloud.access_token,
        "WHATSAPP_CLOUD_ACCESS_TOKEN",
    );
    apply_string_env(
        &mut app.whatsapp_cloud.app_secret,
        "WHATSAPP_CLOUD_APP_SECRET",
    );
    apply_string_env(
        &mut app.whatsapp_cloud.verify_token,
        "WHATSAPP_CLOUD_VERIFY_TOKEN",
    );
    apply_string_env(
        &mut app.whatsapp_cloud.phone_number_id,
        "WHATSAPP_CLOUD_PHONE_NUMBER_ID",
    );
}

fn unique_telegram_bot_name(
    existing: &[ResolvedTelegramBotConfig],
    preferred: &str,
    index_hint: usize,
) -> String {
    let trimmed = preferred.trim();
    if !trimmed.is_empty() && !existing.iter().any(|bot| bot.name == trimmed) {
        return trimmed.to_string();
    }
    let base = if trimmed.is_empty() {
        "bot".to_string()
    } else {
        trimmed.to_string()
    };
    let mut suffix = index_hint.max(1);
    loop {
        let candidate = format!("{base}-{suffix}");
        if !existing.iter().any(|bot| bot.name == candidate) {
            return candidate;
        }
        suffix += 1;
    }
}
