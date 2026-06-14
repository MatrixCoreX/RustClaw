use super::*;

#[test]
fn normalize_symbol_ok() {
    assert_eq!(normalize_symbol("btc/usdt"), "BTCUSDT");
    assert_eq!(normalize_symbol("eth-usd"), "ETHUSD");
    assert_eq!(normalize_symbol("btc"), "BTCUSDT");
    assert_eq!(normalize_symbol("eth"), "ETHUSDT");
    assert_eq!(normalize_symbol("sol"), "SOLUSDT");
    assert_eq!(normalize_symbol("bnb"), "BNBUSDT");
    assert_eq!(normalize_symbol("1000pepe"), "1000PEPEUSDT");
    assert_eq!(normalize_symbol("solbtc"), "SOLBTC");
}

#[test]
fn okx_inst_id_convert_ok() {
    assert_eq!(to_okx_inst_id("BTCUSDT"), "BTC-USDT");
    assert_eq!(to_okx_inst_id("ethusd"), "ETH-USD");
    assert_eq!(to_okx_inst_id("SOL-USDT"), "SOL-USDT");
}

#[test]
fn parse_trade_limit_requires_price() {
    let cfg = RootConfig::default();
    let mut m = serde_json::Map::new();
    m.insert("symbol".to_string(), Value::String("BTCUSDT".to_string()));
    m.insert("side".to_string(), Value::String("buy".to_string()));
    m.insert("order_type".to_string(), Value::String("limit".to_string()));
    m.insert("qty".to_string(), Value::from(0.1_f64));
    assert!(parse_trade_input(&m, &cfg).is_err());
}

#[test]
fn parse_trade_accepts_structured_quantity_and_order_type_aliases() {
    let cfg = RootConfig::default();
    let mut m = serde_json::Map::new();
    m.insert("exchange".to_string(), json!("binance"));
    m.insert("symbol".to_string(), json!("BTCUSDT"));
    m.insert("side".to_string(), json!("buy"));
    m.insert("type".to_string(), json!("market"));
    m.insert("quantity".to_string(), json!(0.01));

    let trade = parse_trade_input(&m, &cfg).unwrap();
    assert_eq!(trade.exchange, "binance");
    assert_eq!(trade.symbol, "BTCUSDT");
    assert_eq!(trade.order_type, "market");
    assert_eq!(trade.qty, 0.01);
}

#[test]
fn parse_trade_accepts_amount_as_base_quantity_alias() {
    let cfg = RootConfig::default();
    let mut m = serde_json::Map::new();
    m.insert("exchange".to_string(), json!("binance"));
    m.insert("symbol".to_string(), json!("DOGEUSDT"));
    m.insert("side".to_string(), json!("sell"));
    m.insert("amount".to_string(), json!(100));

    let trade = parse_trade_input(&m, &cfg).unwrap();
    assert_eq!(trade.side, "sell");
    assert_eq!(trade.order_type, "market");
    assert_eq!(trade.qty, 100.0);
    assert_eq!(trade.quote_qty_usd, None);
}

#[test]
fn parse_trade_quote_amount_alias_takes_priority_over_base_amount() {
    let cfg = RootConfig::default();
    let mut m = serde_json::Map::new();
    m.insert("exchange".to_string(), json!("binance"));
    m.insert("symbol".to_string(), json!("DOGEUSDT"));
    m.insert("side".to_string(), json!("buy"));
    m.insert("amount".to_string(), json!(100));
    m.insert("amount_usd".to_string(), json!(10));

    let trade = parse_trade_input(&m, &cfg).unwrap();
    assert_eq!(trade.qty, 0.0);
    assert_eq!(trade.quote_qty_usd, Some(10.0));
}

#[test]
fn parse_trade_accepts_camel_case_trade_aliases() {
    let cfg = RootConfig::default();
    let mut m = serde_json::Map::new();
    m.insert("exchange".to_string(), json!("okx"));
    m.insert("symbol".to_string(), json!("BTCUSDT"));
    m.insert("side".to_string(), json!("buy"));
    m.insert("orderType".to_string(), json!("limit"));
    m.insert("base_quantity".to_string(), json!("0.02"));
    m.insert("price".to_string(), json!("65000"));
    m.insert("timeInForce".to_string(), json!("gtc"));

    let trade = parse_trade_input(&m, &cfg).unwrap();
    assert_eq!(trade.order_type, "limit");
    assert_eq!(trade.qty, 0.02);
    assert_eq!(trade.price, Some(65000.0));
    assert_eq!(trade.time_in_force.as_deref(), Some("GTC"));
}

#[test]
fn quote_symbol_mapping_for_new_exchanges() {
    assert_eq!(to_gateio_pair("BTCUSDT"), "BTC_USDT");
    assert_eq!(to_coinbase_product("BTCUSDT"), "BTC-USD");
    assert_eq!(to_kraken_pair("BTCUSDT"), "XBTUSDT");
}

#[test]
fn market_quote_extra_exposes_content_excerpt() {
    let extra = market_quote_extra(json!({"action": "quote"}), "BTCUSDT $69587.26");
    assert_eq!(
        extra.get("content_excerpt").and_then(Value::as_str),
        Some("BTCUSDT $69587.26")
    );
}

#[test]
fn price_alert_trigger_logic_up_down_both() {
    let up = 3.2_f64;
    let down = -3.2_f64;
    let threshold = 3.0_f64;
    assert!(up >= threshold);
    assert!(down <= -threshold);
    assert!(up.abs() >= threshold);
    assert!(down.abs() >= threshold);
}

#[test]
fn normalize_crypto_dispatch_action_maps_monitor_aliases() {
    let empty = serde_json::Map::new();
    assert_eq!(
        normalize_crypto_dispatch_action("price_monitor", &empty),
        "price_alert_check"
    );
    assert_eq!(
        normalize_crypto_dispatch_action("MONITOR_PRICE", &empty),
        "price_alert_check"
    );
    assert_eq!(
        normalize_crypto_dispatch_action("volatility_alert", &empty),
        "price_alert_check"
    );
    assert_eq!(
        normalize_crypto_dispatch_action("price_alert_check", &empty),
        "price_alert_check"
    );
    assert_eq!(normalize_crypto_dispatch_action("quote", &empty), "quote");
}

#[test]
fn normalize_crypto_dispatch_action_maps_price_alias_by_args_shape() {
    let empty = serde_json::Map::new();
    assert_eq!(normalize_crypto_dispatch_action("price", &empty), "quote");

    let mut multi = serde_json::Map::new();
    multi.insert("symbols".to_string(), json!(["BTC", "ETH", "DOGE"]));
    assert_eq!(
        normalize_crypto_dispatch_action("price", &multi),
        "multi_quote"
    );
}

#[test]
fn normalize_crypto_dispatch_action_maps_indicator_aliases() {
    let empty = serde_json::Map::new();
    for alias in [
        "technical_indicator",
        "technical_indicators",
        "ta_indicator",
        "TA",
    ] {
        assert_eq!(normalize_crypto_dispatch_action(alias, &empty), "indicator");
    }
}

#[test]
fn normalize_crypto_dispatch_action_maps_klines_by_args_shape() {
    let empty = serde_json::Map::new();
    assert_eq!(
        normalize_crypto_dispatch_action("klines", &empty),
        "candles"
    );

    let mut with_indicator = serde_json::Map::new();
    with_indicator.insert("indicator".to_string(), json!("sma"));
    assert_eq!(
        normalize_crypto_dispatch_action("klines", &with_indicator),
        "indicator"
    );
    assert_eq!(
        normalize_crypto_dispatch_action("OHLCV", &with_indicator),
        "indicator"
    );
}

#[test]
fn account_access_errors_use_stable_prefix_and_safe_detail() {
    let err = crypto_account_access_error(
        "binance",
        "binance api error code=-2015: Invalid API-key, IP, or permissions for action",
    );

    assert!(err.starts_with(CRYPTO_ACCOUNT_ACCESS_ERROR_PREFIX));
    let payload = err
        .strip_prefix(CRYPTO_ACCOUNT_ACCESS_ERROR_PREFIX)
        .unwrap();
    let parsed: Value = serde_json::from_str(payload).unwrap();
    assert_eq!(
        parsed.get("exchange").and_then(|v| v.as_str()),
        Some("binance")
    );
    assert_eq!(
        parsed.get("error_kind").and_then(|v| v.as_str()),
        Some("account_access_failed")
    );
    assert_eq!(
        parsed.get("message_key").and_then(|v| v.as_str()),
        Some("crypto.err.account_access_failed")
    );
    assert!(parsed
        .get("detail")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .contains("Invalid API-key"));
    let extra = crypto_account_access_error_extra_from_text(&err).expect("structured extra");
    assert_eq!(
        extra.get("error_kind").and_then(|v| v.as_str()),
        Some("account_access_failed")
    );
    assert_eq!(
        extra.get("message_key").and_then(|v| v.as_str()),
        Some("crypto.err.account_access_failed")
    );
}

#[test]
fn account_access_error_sanitizes_signed_url_details() {
    let err = crypto_account_access_error(
        "binance",
        "request failed for https://example.invalid/api/v3/account?timestamp=1&signature=secret",
    );
    let payload: Value = serde_json::from_str(
        err.strip_prefix(CRYPTO_ACCOUNT_ACCESS_ERROR_PREFIX)
            .unwrap(),
    )
    .unwrap();
    let detail = payload.get("detail").and_then(|v| v.as_str()).unwrap_or("");
    assert!(!detail.contains("secret"));
    assert!(!detail.contains("signature="));
}

#[test]
fn candles_accept_interval_alias_for_timeframe() {
    let cfg = RootConfig::default();
    let args = json!({
        "action": "klines",
        "symbol": "BTCUSDT",
        "indicator": "SMA",
        "interval": "1d",
        "period": 14,
        "timeout_seconds": 3
    });
    let action = args
        .as_object()
        .and_then(|obj| obj.get("action").and_then(|v| v.as_str()).map(|a| (obj, a)))
        .map(|(obj, action)| normalize_crypto_dispatch_action(action, obj))
        .unwrap();
    assert_eq!(action, "indicator");

    let obj = args.as_object().unwrap();
    assert_eq!(
        obj.get("timeframe")
            .or_else(|| obj.get("interval"))
            .and_then(|v| v.as_str()),
        Some("1d")
    );

    let out = execute(&cfg, args, None);
    assert!(
        !matches!(&out, Err(err) if err == "unsupported action"),
        "technical_indicator should normalize before dispatch"
    );
}

#[test]
fn missing_action_defaults_to_multi_quote_when_symbols_present() {
    let cfg = RootConfig::default();
    let args = json!({
        "symbols": ["BTC", "ETH", "DOGE"],
        "timeout_seconds": 3
    });
    let out = execute(&cfg, args, None);
    assert!(
        !matches!(&out, Err(err) if err == "symbol is required"),
        "missing action with symbols should not fall back to single-symbol quote"
    );
}

#[test]
fn price_alert_defaults_when_args_omit_window_and_threshold() {
    let cfg = RootConfig::default();
    let m = serde_json::Map::new();
    assert_eq!(resolve_price_alert_window_minutes(&m, &cfg), 15);
    assert_eq!(resolve_price_alert_threshold_pct(&m, &cfg), 5.0);
}

#[test]
fn price_alert_window_minutes_clamps_1_through_4_to_5() {
    let cfg = RootConfig::default();
    for w in [1u64, 2, 3, 4] {
        let mut m = serde_json::Map::new();
        m.insert("window_minutes".to_string(), json!(w));
        assert_eq!(
            resolve_price_alert_window_minutes(&m, &cfg),
            5,
            "window_minutes={w} should clamp to 5"
        );
    }
}

#[test]
fn price_alert_window_minutes_30_unchanged() {
    let cfg = RootConfig::default();
    let mut m = serde_json::Map::new();
    m.insert("window_minutes".to_string(), json!(30));
    assert_eq!(resolve_price_alert_window_minutes(&m, &cfg), 30);
}

#[test]
fn price_alert_window_accepts_i64_and_float_json() {
    let cfg = RootConfig::default();
    let mut m = serde_json::Map::new();
    m.insert("window_minutes".to_string(), json!(3));
    assert_eq!(resolve_price_alert_window_minutes(&m, &cfg), 5);
    let mut m2 = serde_json::Map::new();
    m2.insert("minutes".to_string(), json!(3.7));
    assert_eq!(resolve_price_alert_window_minutes(&m2, &cfg), 5);
}

#[test]
fn price_alert_config_default_below_5_clamps_to_5() {
    let mut cfg = RootConfig::default();
    cfg.crypto.alert_default_window_minutes = Some(2);
    let m = serde_json::Map::new();
    assert_eq!(resolve_price_alert_window_minutes(&m, &cfg), 5);
}

#[test]
fn price_alert_direction_defaults_to_both_and_normalizes_aliases() {
    let empty = serde_json::Map::new();
    assert_eq!(resolve_price_alert_direction_normalized(&empty), "both");
    let mut up = serde_json::Map::new();
    up.insert("direction".to_string(), json!("RISE"));
    assert_eq!(resolve_price_alert_direction_normalized(&up), "up");
    let mut down = serde_json::Map::new();
    down.insert("direction".to_string(), json!("dump"));
    assert_eq!(resolve_price_alert_direction_normalized(&down), "down");
}

#[test]
fn price_alert_binance_listing_precheck_skipped_for_okx_only() {
    assert!(price_alert_needs_binance_listing_precheck("binance"));
    assert!(price_alert_needs_binance_listing_precheck("BINANCE"));
    assert!(!price_alert_needs_binance_listing_precheck("okx"));
    assert!(!price_alert_needs_binance_listing_precheck(" OKX "));
}

#[test]
fn preflight_price_alert_symbol_listing_okx_skips_binance_listing_call() {
    let cfg = RootConfig::default();
    let client = Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .unwrap();
    assert!(preflight_price_alert_symbol_listing(&client, &cfg, "okx", "BTCUSDT").is_ok());
}

#[test]
fn resolve_exchange_uses_configured_default_exchange() {
    let mut cfg = RootConfig::default();
    cfg.crypto.execution_mode = Some("okx".to_string());
    assert_eq!(resolve_exchange(None, &cfg).unwrap(), "okx");
}

#[test]
fn resolve_exchange_requires_explicit_exchange_when_no_default_configured() {
    let cfg = RootConfig::default();
    assert_eq!(
        resolve_exchange(None, &cfg).unwrap_err(),
        tr("crypto.err.exchange_required_when_no_default")
    );
}

#[test]
fn skill_context_parses_schedule_metadata() {
    let raw = json!({
        "exchange_credentials": {},
        "schedule_job_id": "job_x",
        "invocation_source": "schedule",
        "scheduled": true,
        "schedule_triggered": true,
        "locale": "en-US"
    });
    let ctx: SkillContext = serde_json::from_value(raw).unwrap();
    assert_eq!(ctx.schedule_job_id.as_deref(), Some("job_x"));
    assert_eq!(ctx.invocation_source.as_deref(), Some("schedule"));
    assert_eq!(ctx.scheduled, Some(true));
    assert_eq!(ctx.schedule_triggered, Some(true));
    assert_eq!(ctx.locale.as_deref(), Some("en-US"));
}

#[test]
fn schedule_invocation_extra_prefers_context_over_args() {
    let ctx = SkillContext {
        schedule_job_id: Some("from_ctx".to_string()),
        invocation_source: Some("schedule".to_string()),
        scheduled: Some(true),
        schedule_triggered: Some(true),
        ..Default::default()
    };
    let mut obj = serde_json::Map::new();
    obj.insert("schedule_job_id".to_string(), json!("from_args"));
    let m = schedule_invocation_extra_fields(&ctx, &obj);
    assert_eq!(
        m.get("schedule_job_id").and_then(|v| v.as_str()),
        Some("from_ctx")
    );
}

fn ensure_test_i18n_catalogs() {
    I18N.get_or_init(|| {
        let mut catalogs = HashMap::new();
        catalogs.insert("zh-CN".to_string(), default_crypto_catalog("zh-CN"));
        catalogs.insert("en-US".to_string(), default_crypto_catalog("en-US"));
        catalogs
    });
    set_current_lang("zh-CN");
}

#[test]
fn resolve_i18n_lang_prefers_context_locale_over_config_default() {
    let cfg = RootConfig::default();
    let args = serde_json::Map::new();
    let ctx = SkillContext {
        locale: Some("en-US".to_string()),
        ..Default::default()
    };
    assert_eq!(resolve_i18n_lang(&args, &ctx, &cfg), "en-US");
}

#[test]
fn execute_uses_context_locale_for_binance_not_bound_message() {
    ensure_test_i18n_catalogs();
    set_current_lang("zh-CN");
    let cfg = RootConfig::default();
    let err = execute(
        &cfg,
        json!({
            "action": "positions",
            "exchange": "binance"
        }),
        Some(json!({
            "locale": "en-US",
            "exchange_credentials": {}
        })),
    )
    .unwrap_err();
    let extra = crypto_config_error_extra_from_text(&err).expect("config error extra");
    assert_eq!(
        extra.get("message_key").and_then(|v| v.as_str()),
        Some("crypto.err.binance_not_bound")
    );
    assert_eq!(
        crypto_error_text_for_response(&err),
        tr("crypto.err.binance_not_bound")
    );
    set_current_lang("zh-CN");
}

#[test]
fn private_exchange_action_checks_binding_before_trade_params() {
    ensure_test_i18n_catalogs();
    let mut cfg = RootConfig::default();
    cfg.crypto.default_exchange = Some("binance".to_string());
    let err = execute(
        &cfg,
        json!({
            "action": "trade_preview"
        }),
        Some(json!({
            "locale": "zh-CN",
            "exchange_credentials": {}
        })),
    )
    .unwrap_err();
    let extra = crypto_config_error_extra_from_text(&err).expect("config error extra");
    assert_eq!(
        extra.get("message_key").and_then(|v| v.as_str()),
        Some("crypto.err.binance_not_bound")
    );
    assert_eq!(
        extra.get("action").and_then(|v| v.as_str()),
        Some("trade_preview")
    );
}

#[test]
fn private_exchange_action_alias_checks_binding_first() {
    ensure_test_i18n_catalogs();
    let mut cfg = RootConfig::default();
    cfg.crypto.default_exchange = Some("okx".to_string());
    let err = execute(
        &cfg,
        json!({
            "action": "pending_orders"
        }),
        Some(json!({
            "locale": "en-US",
            "exchange_credentials": {}
        })),
    )
    .unwrap_err();
    let extra = crypto_config_error_extra_from_text(&err).expect("config error extra");
    assert_eq!(
        extra.get("message_key").and_then(|v| v.as_str()),
        Some("crypto.err.okx_not_bound")
    );
    assert_eq!(
        extra.get("action").and_then(|v| v.as_str()),
        Some("pending_orders")
    );
    set_current_lang("zh-CN");
}
