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

pub(super) fn persist_chat_voice_mode_to_config(
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

pub(super) fn handle_openclaw_config_command(
    state: &BotState,
    command_tail: &str,
) -> anyhow::Result<String> {
    let cmd = command_tail.trim();
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

pub(super) fn cryptoapi_usage_text(state: &BotState) -> String {
    state.i18n.t("telegram.msg.cryptoapi_usage")
}

pub(super) fn crypto_command_usage_text(state: &BotState) -> String {
    state.i18n.t("telegram.msg.crypto_usage")
}

pub(super) fn run_skill_help_text(state: &BotState) -> String {
    let usage = state.i18n.t("telegram.msg.run_usage");
    let skills = if state.skills_list.is_empty() {
        state.i18n.t("telegram.msg.no_skills")
    } else {
        state.i18n.t_with(
            "telegram.msg.skills_list",
            &[("skills", &state.skills_list.join(", "))],
        )
    };
    format!("{usage}\n{skills}")
}

pub(super) fn parse_symbols_csv(raw: &str) -> Vec<String> {
    raw.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .collect()
}

pub(super) fn normalize_trade_symbol_for_config(raw: &str) -> Option<String> {
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

pub(super) fn maybe_exchange_token(raw: &str) -> bool {
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

pub(super) fn build_crypto_skill_payload(raw: &str) -> anyhow::Result<Option<JsonValue>> {
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

pub(super) fn mask_secret(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return "<empty>".to_string();
    }
    if s.len() <= 8 {
        return "***".to_string();
    }
    format!("{}***{}", &s[..4], &s[s.len() - 4..])
}

pub(super) fn sanitize_message_text_for_log(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.starts_with("/key ") {
        return "/key <redacted>".to_string();
    }
    if let Some(rest) = trimmed.strip_prefix("/cryptoapi") {
        let mut parts = rest.split_whitespace();
        let action = parts.next().unwrap_or_default();
        if action.eq_ignore_ascii_case("set") {
            let exchange = parts.next().unwrap_or_default();
            return if exchange.is_empty() {
                "/cryptoapi set <redacted>".to_string()
            } else {
                format!("/cryptoapi set {} <redacted>", exchange)
            };
        }
    }
    text.to_string()
}

pub(super) fn clear_pending_resume_for_chat(state: &BotState, chat_id: i64) {
    if let Ok(mut guard) = state.pending_resume_by_chat.lock() {
        guard.remove(&chat_id);
    }
}

pub(super) async fn handle_cryptoapi_command(
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

pub(super) async fn show_cryptoapi_status(
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

pub(super) fn persist_crypto_allowed_symbols_add(
    symbols: &[String],
) -> anyhow::Result<Vec<String>> {
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

pub(super) fn openclaw_usage_text(state: &BotState) -> String {
    state.i18n.t("telegram.msg.openclaw_usage")
}

pub(super) fn supported_types_text(state: &BotState) -> String {
    state.i18n.t("telegram.msg.openclaw_supported_vendors")
}

pub(super) fn show_model_config(state: &BotState) -> anyhow::Result<String> {
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
        "mimo",
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

pub(super) fn is_supported_model_vendor(vendor: &str) -> bool {
    matches!(
        vendor,
        "openai"
            | "google"
            | "anthropic"
            | "grok"
            | "deepseek"
            | "qwen"
            | "minimax"
            | "mimo"
            | "custom"
    )
}

pub(super) fn default_base_url_for_vendor(vendor: &str) -> &'static str {
    match vendor {
        "openai" => "https://api.openai.com/v1",
        "google" => "https://generativelanguage.googleapis.com/v1beta",
        "anthropic" => "https://api.anthropic.com/v1",
        "grok" => "https://api.x.ai/v1",
        "qwen" => "https://dashscope.aliyuncs.com/compatible-mode/v1",
        "minimax" => "https://api.minimaxi.com/v1",
        "mimo" => "https://token-plan-sgp.xiaomimimo.com/v1",
        "custom" => "https://api.example.com/v1",
        _ => "https://api.example.com/v1",
    }
}

pub(super) fn apply_model_config_value(
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

pub(super) fn set_model_config(
    state: &BotState,
    vendor: &str,
    model: &str,
) -> anyhow::Result<String> {
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
