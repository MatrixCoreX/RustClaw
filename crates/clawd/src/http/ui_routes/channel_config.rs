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
    let config_path = state.skill_rt.workspace_root.join("configs/config.toml");
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
    let config_path = state.skill_rt.workspace_root.join("configs/config.toml");
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
        &state.skill_rt.workspace_root,
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
    let session_path = state
        .skill_rt
        .workspace_root
        .join("data/wechatd/session.json");
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
        &state.skill_rt.workspace_root,
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
        &state.skill_rt.workspace_root,
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
        &state.skill_rt.workspace_root,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    planner_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    adapter_category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    background_job_capable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    risk_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_invocable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    requires_confirmation: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    side_effect: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retryable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_available: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unavailable_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_os: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unsupported_os: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    missing_required_bins: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    missing_optional_bins: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supported_os: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    required_bins: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    optional_bins: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform_notes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    planner_capabilities: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    capabilities: Option<Vec<String>>,
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
