use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::blocking::Client;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use urlencoding::encode;

type HmacSha256 = Hmac<Sha256>;
mod exchange_api;
mod i18n;
mod market_data;
mod market_handlers;
mod onchain_handlers;
mod trade_execution;
mod trade_handlers;

use exchange_api::*;
use i18n::*;
use market_data::*;
use market_handlers::*;
use onchain_handlers::*;
use trade_execution::*;
use trade_handlers::*;

const CRYPTO_ACCOUNT_ACCESS_ERROR_PREFIX: &str = "__RC_CRYPTO_ACCOUNT_ACCESS_ERROR__:";
const CRYPTO_CONFIG_ERROR_PREFIX: &str = "__RC_CRYPTO_CONFIG_ERROR__:";
const CRYPTO_ACCOUNT_ACCESS_ERROR_KIND: &str = "account_access_failed";
const CRYPTO_ACCOUNT_ACCESS_MESSAGE_KEY: &str = "crypto.err.account_access_failed";
const CRYPTO_CREDENTIAL_NOT_BOUND_ERROR_KIND: &str = "credential_not_bound";
const CRYPTO_CREDENTIAL_INCOMPLETE_ERROR_KIND: &str = "credential_incomplete";
const SKILL_NAME: &str = "crypto";

fn crypto_account_access_error(exchange: &str, err: impl AsRef<str>) -> String {
    if err
        .as_ref()
        .trim()
        .starts_with(CRYPTO_ACCOUNT_ACCESS_ERROR_PREFIX)
    {
        return err.as_ref().trim().to_string();
    }
    let detail = sanitize_crypto_account_access_error_detail(err.as_ref());
    let payload = json!({
        "exchange": exchange.trim(),
        "detail": detail,
        "error_kind": CRYPTO_ACCOUNT_ACCESS_ERROR_KIND,
        "message_key": CRYPTO_ACCOUNT_ACCESS_MESSAGE_KEY,
        "recoverable": true,
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| {
        json!({
            "exchange": exchange.trim(),
            "detail": "private exchange account access failed"
        })
        .to_string()
    });
    format!("{CRYPTO_ACCOUNT_ACCESS_ERROR_PREFIX}{encoded}")
}

fn crypto_account_access_error_extra_from_text(error_text: &str) -> Option<Value> {
    let payload = error_text
        .trim()
        .strip_prefix(CRYPTO_ACCOUNT_ACCESS_ERROR_PREFIX)?;
    let parsed = serde_json::from_str::<Value>(payload).ok()?;
    Some(json!({
        "error_kind": parsed
            .get("error_kind")
            .and_then(|value| value.as_str())
            .unwrap_or(CRYPTO_ACCOUNT_ACCESS_ERROR_KIND),
        "message_key": parsed
            .get("message_key")
            .and_then(|value| value.as_str())
            .unwrap_or(CRYPTO_ACCOUNT_ACCESS_MESSAGE_KEY),
        "recoverable": parsed
            .get("recoverable")
            .and_then(|value| value.as_bool())
            .unwrap_or(true),
        "exchange": parsed.get("exchange").cloned().unwrap_or(Value::Null),
        "detail": parsed.get("detail").cloned().unwrap_or(Value::Null),
        "status_code": CRYPTO_ACCOUNT_ACCESS_ERROR_KIND,
    }))
}

fn crypto_config_error(
    exchange: &str,
    action: &str,
    error_kind: &str,
    message_key: &str,
) -> String {
    let payload = json!({
        "exchange": exchange.trim(),
        "action": action.trim(),
        "error_kind": error_kind,
        "message_key": message_key,
        "recoverable": true,
        "status_code": error_kind,
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| {
        json!({
            "exchange": exchange.trim(),
            "action": action.trim(),
            "error_kind": error_kind,
            "message_key": message_key,
            "recoverable": true,
        })
        .to_string()
    });
    format!("{CRYPTO_CONFIG_ERROR_PREFIX}{encoded}")
}

fn crypto_config_error_payload_from_text(error_text: &str) -> Option<Value> {
    let payload = error_text.trim().strip_prefix(CRYPTO_CONFIG_ERROR_PREFIX)?;
    serde_json::from_str::<Value>(payload).ok()
}

fn crypto_config_error_extra_from_text(error_text: &str) -> Option<Value> {
    let parsed = crypto_config_error_payload_from_text(error_text)?;
    Some(json!({
        "error_kind": parsed
            .get("error_kind")
            .and_then(|value| value.as_str())
            .unwrap_or("credential_error"),
        "message_key": parsed.get("message_key").cloned().unwrap_or(Value::Null),
        "recoverable": parsed
            .get("recoverable")
            .and_then(|value| value.as_bool())
            .unwrap_or(true),
        "exchange": parsed.get("exchange").cloned().unwrap_or(Value::Null),
        "action": parsed.get("action").cloned().unwrap_or(Value::Null),
        "status_code": parsed
            .get("status_code")
            .cloned()
            .or_else(|| parsed.get("error_kind").cloned())
            .unwrap_or(Value::Null),
    }))
}

fn crypto_error_extra_from_text(error_text: &str) -> Option<Value> {
    let details = crypto_account_access_error_extra_from_text(error_text)
        .or_else(|| crypto_config_error_extra_from_text(error_text))?;
    let error_kind = details
        .get("error_kind")
        .and_then(Value::as_str)
        .unwrap_or("execution_failed")
        .to_string();
    Some(crypto_error_extra_with_details(&error_kind, Some(details)))
}

fn crypto_error_extra(error_kind: &str) -> Value {
    crypto_error_extra_with_details(error_kind, None)
}

fn crypto_error_extra_with_details(error_kind: &str, details: Option<Value>) -> Value {
    let mut extra = json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_kind": error_kind,
        "message_key": format!("skill.{}.{}", SKILL_NAME, error_kind),
        "retryable": false,
    });
    if let Some(details) = details {
        if let (Some(base), Some(details_obj)) = (extra.as_object_mut(), details.as_object()) {
            for (key, value) in details_obj {
                base.entry(key.clone()).or_insert_with(|| value.clone());
            }
        } else if let Some(base) = extra.as_object_mut() {
            base.insert("details".to_string(), details);
        }
    }
    extra
}

fn crypto_error_text_for_response(error_text: &str) -> String {
    if let Some(payload) = crypto_config_error_payload_from_text(error_text) {
        if let Some(message_key) = payload.get("message_key").and_then(|value| value.as_str()) {
            return tr(message_key);
        }
    }
    error_text.to_string()
}

fn sanitize_crypto_account_access_error_detail(raw: &str) -> String {
    let one_line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.trim().is_empty() {
        return "private exchange account access failed".to_string();
    }
    let lower = one_line.to_ascii_lowercase();
    if lower.contains("signature=")
        || lower.contains("x-mbx-apikey")
        || lower.contains("ok-access-key")
    {
        return "private exchange API request failed before a safe response could be read"
            .to_string();
    }
    truncate(&one_line, 500)
}

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
    #[serde(default)]
    context: Option<Value>,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    extra: Option<Value>,
    error_text: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RootConfig {
    #[serde(default)]
    crypto: CryptoConfig,
    #[serde(default)]
    binance: BinanceConfig,
    #[serde(default)]
    okx: OkxConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct SkillContext {
    #[serde(default)]
    user_key: Option<String>,
    #[serde(default)]
    exchange_credentials: HashMap<String, ExchangeCredentialInput>,
    /// Present when the skill is invoked from a scheduled job (injected by `clawd` into `context`).
    #[serde(default)]
    schedule_job_id: Option<String>,
    #[serde(default)]
    invocation_source: Option<String>,
    #[serde(default)]
    scheduled: Option<bool>,
    #[serde(default)]
    schedule_triggered: Option<bool>,
    #[serde(default)]
    locale: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    lang: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ExchangeCredentialInput {
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    api_secret: String,
    #[serde(default)]
    passphrase: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct LegacyRootConfig {
    #[serde(default)]
    crypto: CryptoConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct CryptoConfig {
    #[serde(default)]
    timeout_seconds: Option<u64>,
    #[serde(default)]
    default_exchange: Option<String>,
    #[serde(default)]
    execution_mode: Option<String>,
    // Kept for config compatibility; confirmation is now planner-decided.
    #[serde(default, rename = "require_explicit_send")]
    _require_explicit_send: Option<bool>,
    #[serde(default)]
    max_notional_usd: Option<f64>,
    #[serde(default)]
    min_notional_usd: Option<f64>,
    #[serde(default)]
    allowed_exchanges: Vec<String>,
    #[serde(default)]
    allowed_symbols: Vec<String>,
    #[serde(default)]
    blocked_actions: Vec<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    i18n_path: Option<String>,
    #[serde(default = "default_btc_onchain_fees_api_url")]
    btc_onchain_fees_api_url: String,
    #[serde(default = "default_eth_onchain_stats_api_url")]
    eth_onchain_stats_api_url: String,
    #[serde(default = "default_coingecko_simple_price_api_url")]
    coingecko_simple_price_api_url: String,
    #[serde(default = "default_gateio_quote_ticker_api_path")]
    gateio_quote_ticker_api_path: String,
    #[serde(default = "default_coinbase_quote_ticker_api_path")]
    coinbase_quote_ticker_api_path: String,
    #[serde(default = "default_kraken_quote_ticker_api_path")]
    kraken_quote_ticker_api_path: String,
    #[serde(default = "default_gateio_book_ticker_api_path")]
    gateio_book_ticker_api_path: String,
    #[serde(default = "default_coinbase_book_ticker_api_path")]
    coinbase_book_ticker_api_path: String,
    #[serde(default = "default_kraken_book_ticker_api_path")]
    kraken_book_ticker_api_path: String,
    #[serde(default = "default_binance_quote_24hr_api_path")]
    binance_quote_24hr_api_path: String,
    #[serde(default = "default_binance_quote_price_api_path")]
    binance_quote_price_api_path: String,
    #[serde(default = "default_binance_book_ticker_api_path")]
    binance_book_ticker_api_path: String,
    #[serde(default = "default_okx_market_ticker_api_path")]
    okx_market_ticker_api_path: String,
    #[serde(default = "default_eth_address_native_balance_api_url")]
    eth_address_native_balance_api_url: String,
    #[serde(default = "default_eth_address_token_balance_api_url")]
    eth_address_token_balance_api_url: String,
    #[serde(default = "default_eth_address_tx_list_api_url")]
    eth_address_tx_list_api_url: String,
    #[serde(default)]
    eth_token_contracts: HashMap<String, String>,
    #[serde(default)]
    eth_token_decimals: HashMap<String, u32>,
    #[serde(default)]
    alert_default_window_minutes: Option<u64>,
    #[serde(default)]
    alert_default_threshold_pct: Option<f64>,
    #[serde(default)]
    alert_max_window_minutes: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct BinanceConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default = "default_binance_base_url")]
    base_url: String,
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    api_secret: String,
    #[serde(default = "default_recv_window")]
    recv_window: u64,
}

impl Default for BinanceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: default_binance_base_url(),
            api_key: String::new(),
            api_secret: String::new(),
            recv_window: default_recv_window(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct OkxConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default = "default_okx_base_url")]
    base_url: String,
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    api_secret: String,
    #[serde(default)]
    passphrase: String,
    #[serde(default = "default_okx_simulated")]
    simulated: bool,
}

impl Default for OkxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: default_okx_base_url(),
            api_key: String::new(),
            api_secret: String::new(),
            passphrase: String::new(),
            simulated: default_okx_simulated(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct Quote {
    symbol: String,
    price_usd: f64,
    change_24h_pct: Option<f64>,
    exchange: String,
    source: String,
}

#[derive(Debug, Clone, Serialize)]
struct BookTicker {
    symbol: String,
    bid_price: f64,
    bid_qty: f64,
    ask_price: f64,
    ask_qty: f64,
    exchange: String,
    source: String,
}

#[derive(Debug, Clone)]
struct TradeInput {
    exchange: String,
    symbol: String,
    side: String,
    order_type: String,
    qty: f64,
    qty_all: bool,
    quote_qty_usd: Option<f64>,
    price: Option<f64>,
    stop_price: Option<f64>,
    time_in_force: Option<String>,
    client_order_id: Option<String>,
    confirm: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OrderEvent {
    event: String,
    order_id: String,
    ts: u64,
    exchange: String,
    symbol: String,
    side: String,
    order_type: String,
    qty: f64,
    price: Option<f64>,
    notional_usd: f64,
    status: String,
    client_order_id: Option<String>,
    reason: Option<String>,
    /// Actual filled base-asset qty (from exchange response)
    executed_qty: Option<f64>,
    /// Actual filled quote-asset qty, e.g. USDT spent/received
    executed_quote_qty: Option<f64>,
    /// Average fill price (executed_quote / executed_qty)
    avg_fill_price: Option<f64>,
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let cfg = load_root_config();
    let workspace_root = workspace_root();
    init_i18n(&cfg, &workspace_root);

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match execute(&cfg, req.args, req.context) {
                Ok((text, extra)) => Resp {
                    request_id: req.request_id,
                    status: "ok".to_string(),
                    text,
                    extra: Some(extra),
                    error_text: None,
                },
                Err(err) => Resp {
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    extra: crypto_error_extra_from_text(&err),
                    error_text: Some(crypto_error_text_for_response(&err)),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: Some(crypto_error_extra("invalid_input")),
                error_text: Some(tr_with(
                    "crypto.err.invalid_input",
                    &[("error", &err.to_string())],
                )),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

/// Normalizes legacy / alias action names to the dispatch name used in `match` below.
fn normalize_crypto_dispatch_action(raw: &str, obj: &serde_json::Map<String, Value>) -> String {
    let action = raw.trim().to_ascii_lowercase();
    match action.as_str() {
        "price" => {
            if obj.contains_key("symbols") {
                "multi_quote".to_string()
            } else {
                "quote".to_string()
            }
        }
        "get_price" => "quote".to_string(),
        "get_multi_price" => "multi_quote".to_string(),
        "technical_indicator" | "technical_indicators" | "ta_indicator" | "ta" => {
            "indicator".to_string()
        }
        "kline" | "klines" | "candlestick" | "candlesticks" | "ohlcv" => {
            if obj.contains_key("indicator") {
                "indicator".to_string()
            } else {
                "candles".to_string()
            }
        }
        "price_monitor" | "monitor_price" | "price_alert" | "volatility_alert" => {
            "price_alert_check".to_string()
        }
        other => other.to_string(),
    }
}

fn action_requires_exchange_credentials(action: &str) -> bool {
    matches!(
        action,
        "trade_preview"
            | "trade_submit"
            | "order_status"
            | "cancel_order"
            | "cancel_all_orders"
            | "cancel_open_orders"
            | "open_orders"
            | "get_open_orders"
            | "pending_orders"
            | "trade_history"
            | "my_trades"
            | "recent_trades"
            | "positions"
    )
}

fn ensure_action_exchange_credentials(
    cfg: &RootConfig,
    action: &str,
    obj: &serde_json::Map<String, Value>,
) -> Result<(), String> {
    if !action_requires_exchange_credentials(action) {
        return Ok(());
    }
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg)?;
    match exchange.as_str() {
        "binance" => ensure_binance_config_for_action(cfg, action),
        "okx" => ensure_okx_config_for_action(cfg, action),
        _ => Ok(()),
    }
}

/// Minimum lookback window for `price_alert_check` (minutes). Values below this are clamped up.
const PRICE_ALERT_MIN_WINDOW_MINUTES: u64 = 5;

fn value_to_u64_non_negative_window(v: &Value) -> Option<u64> {
    if let Some(u) = v.as_u64() {
        return Some(u);
    }
    if let Some(i) = v.as_i64() {
        return (i >= 0).then_some(i as u64);
    }
    if let Some(f) = v.as_f64() {
        if f.is_finite() && f >= 0.0 {
            return Some(f.round() as u64);
        }
    }
    v.as_str()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|f| f.is_finite() && *f >= 0.0)
        .map(|f| f.round() as u64)
}

fn resolve_price_alert_window_minutes(
    obj: &serde_json::Map<String, Value>,
    cfg: &RootConfig,
) -> u64 {
    let max_window = cfg
        .crypto
        .alert_max_window_minutes
        .unwrap_or(240)
        .clamp(1, 1440);
    let raw = obj
        .get("window_minutes")
        .or_else(|| obj.get("minutes"))
        .and_then(value_to_u64_non_negative_window)
        .unwrap_or_else(|| cfg.crypto.alert_default_window_minutes.unwrap_or(15));
    let floor = PRICE_ALERT_MIN_WINDOW_MINUTES.min(max_window);
    raw.max(floor).min(max_window)
}

fn resolve_price_alert_threshold_pct(
    obj: &serde_json::Map<String, Value>,
    cfg: &RootConfig,
) -> f64 {
    obj.get("threshold_pct")
        .or_else(|| obj.get("pct"))
        .or_else(|| obj.get("percent"))
        .and_then(value_to_f64)
        .unwrap_or_else(|| cfg.crypto.alert_default_threshold_pct.unwrap_or(5.0))
}

/// `true` when `price_alert_check` should validate the symbol against Binance listings before candles (non-OKX path).
fn price_alert_needs_binance_listing_precheck(exchange: &str) -> bool {
    !exchange.trim().eq_ignore_ascii_case("okx")
}

/// Listing validation for `price_alert_check` (same Binance filter path as action `binance_symbol_check`).
/// Schedule / other layers must not pre-call `binance_symbol_check`; only this skill path runs it when needed.
fn preflight_price_alert_symbol_listing(
    client: &Client,
    cfg: &RootConfig,
    exchange: &str,
    symbol: &str,
) -> Result<(), String> {
    if price_alert_needs_binance_listing_precheck(exchange) {
        ensure_symbol_supported_on_binance(client, cfg, symbol)
    } else {
        Ok(())
    }
}

fn resolve_price_alert_direction_normalized(obj: &serde_json::Map<String, Value>) -> &'static str {
    let raw = obj
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("both")
        .trim()
        .to_ascii_lowercase();
    match raw.as_str() {
        "up" | "rise" | "pump" => "up",
        "down" | "drop" | "dump" => "down",
        _ => "both",
    }
}

fn execute(
    cfg: &RootConfig,
    args: Value,
    context: Option<Value>,
) -> Result<(String, Value), String> {
    let context = context
        .and_then(|v| serde_json::from_value::<SkillContext>(v).ok())
        .unwrap_or_default();
    let cfg = apply_context_credentials(cfg, &context);
    let obj = args
        .as_object()
        .ok_or_else(|| tr("crypto.err.args_object"))?;
    let resolved_lang = resolve_i18n_lang(obj, &context, &cfg);
    set_current_lang(&resolved_lang);
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(if obj.contains_key("symbols") {
            "price"
        } else {
            "quote"
        });
    let action = normalize_crypto_dispatch_action(action, obj);
    if cfg
        .crypto
        .blocked_actions
        .iter()
        .any(|v| v.trim().eq_ignore_ascii_case(&action))
    {
        return Err(tr_with("crypto.err.action_blocked", &[("action", &action)]));
    }
    ensure_action_exchange_credentials(&cfg, &action, obj)?;
    let timeout_seconds = obj
        .get("timeout_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| cfg.crypto.timeout_seconds.unwrap_or(20))
        .clamp(3, 120);
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .build()
        .map_err(|err| {
            tr_with(
                "crypto.err.build_http_client",
                &[("error", &err.to_string())],
            )
        })?;

    match action.as_str() {
        "quote" => handle_quote(&client, &cfg, obj),
        "multi_quote" => handle_multi_quote(&client, &cfg, obj),
        "get_book_ticker" | "book_ticker" => handle_book_ticker(&client, &cfg, obj),
        "binance_symbol_check" => handle_binance_symbol_check(&client, &cfg, obj),
        "normalize_symbol" => handle_normalize_symbol(obj),
        "healthcheck" => handle_healthcheck(&client, &cfg, obj),
        "candles" => handle_candles(&client, &cfg, obj),
        "indicator" => handle_indicator(&client, &cfg, obj),
        "price_alert_check" => handle_price_alert_check(&client, &cfg, obj, &context),
        "onchain" => handle_onchain(&client, &cfg, obj),
        "trade_preview" => handle_trade_preview(&client, &cfg, obj),
        "trade_submit" => handle_trade_submit(&client, &cfg, obj),
        "order_status" => handle_order_status(&client, &cfg, obj),
        "cancel_order" => handle_cancel_order(&client, &cfg, obj),
        "cancel_all_orders" | "cancel_open_orders" => handle_cancel_all_orders(&client, &cfg, obj),
        "open_orders" | "get_open_orders" | "pending_orders" => {
            handle_open_orders(&client, &cfg, obj)
        }
        "trade_history" | "my_trades" | "recent_trades" => handle_trade_history(&client, &cfg, obj),
        "positions" => handle_positions(&client, &cfg, obj),
        _ => Err(tr("crypto.err.unsupported_action")),
    }
}

fn is_placeholder(v: &str) -> bool {
    let t = v.trim();
    t.is_empty() || t.starts_with("REPLACE_ME_") || t == "__REDACTED__"
}

fn configured_default_exchange(cfg: &RootConfig) -> Option<String> {
    cfg.crypto
        .execution_mode
        .as_deref()
        .or(cfg.crypto.default_exchange.as_deref())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_ascii_lowercase())
}

fn resolve_exchange(input: Option<&str>, cfg: &RootConfig) -> Result<String, String> {
    if let Some(raw) = input.map(str::trim).filter(|v| !v.is_empty()) {
        return Ok(raw.to_ascii_lowercase());
    }
    if let Some(exchange) = configured_default_exchange(cfg) {
        return Ok(exchange);
    }
    Err(tr("crypto.err.exchange_required_when_no_default"))
}

fn symbol_to_coingecko_id(symbol: &str) -> Option<&'static str> {
    match normalize_symbol(symbol).as_str() {
        "BTCUSDT" | "BTCUSD" => Some("bitcoin"),
        "ETHUSDT" | "ETHUSD" => Some("ethereum"),
        "BNBUSDT" | "BNBUSD" => Some("binancecoin"),
        "SOLUSDT" | "SOLUSD" => Some("solana"),
        "XRPUSDT" | "XRPUSD" => Some("ripple"),
        "DOGEUSDT" | "DOGEUSD" => Some("dogecoin"),
        "ADAUSDT" | "ADAUSD" => Some("cardano"),
        _ => None,
    }
}

fn normalize_symbol(input: &str) -> String {
    let s = input
        .trim()
        .to_ascii_uppercase()
        .replace('/', "")
        .replace('-', "")
        .replace('_', "");
    if s.is_empty() {
        return s;
    }
    if has_known_quote_suffix(&s) {
        return s;
    }
    // For coin symbols without explicit quote, default quote is USDT.
    if s.chars().all(|c| c.is_ascii_alphanumeric()) {
        return format!("{s}USDT");
    }
    s
}

fn has_known_quote_suffix(symbol: &str) -> bool {
    [
        "USDT", "USD", "USDC", "BUSD", "FDUSD", "USDE", "BTC", "ETH", "BNB", "EUR", "TRY", "BRL",
        "DAI",
    ]
    .iter()
    .any(|q| symbol.len() > q.len() && symbol.ends_with(q))
}

fn to_okx_inst_id(symbol: &str) -> String {
    let raw = symbol.trim().to_ascii_uppercase();
    if raw.contains('-') {
        return raw;
    }
    let s = normalize_symbol(&raw);
    if let Some(base) = s.strip_suffix("USDT") {
        return format!("{base}-USDT");
    }
    if let Some(base) = s.strip_suffix("USD") {
        return format!("{base}-USD");
    }
    format!("{s}-USDT")
}

fn split_symbol_base_quote(symbol: &str) -> (String, String) {
    let normalized = normalize_symbol(symbol);
    for q in [
        "USDT", "USD", "USDC", "BUSD", "FDUSD", "USDE", "BTC", "ETH", "BNB", "EUR", "TRY", "BRL",
        "DAI",
    ] {
        if let Some(base) = normalized.strip_suffix(q) {
            if !base.is_empty() {
                return (base.to_string(), q.to_string());
            }
        }
    }
    (normalized, "USDT".to_string())
}

fn to_gateio_pair(symbol: &str) -> String {
    let (base, quote) = split_symbol_base_quote(symbol);
    format!("{base}_{quote}")
}

fn to_coinbase_product(symbol: &str) -> String {
    let (base, _quote) = split_symbol_base_quote(symbol);
    format!("{base}-USD")
}

fn to_kraken_pair(symbol: &str) -> String {
    let (base_raw, quote) = split_symbol_base_quote(symbol);
    let base = if base_raw == "BTC" { "XBT" } else { &base_raw };
    format!("{base}{quote}")
}

fn map_interval_binance(input: &str) -> &'static str {
    match input.trim().to_ascii_lowercase().as_str() {
        "1m" => "1m",
        "3m" => "3m",
        "5m" => "5m",
        "15m" => "15m",
        "30m" => "30m",
        "1h" => "1h",
        "2h" => "2h",
        "4h" => "4h",
        "6h" => "6h",
        "8h" => "8h",
        "12h" => "12h",
        "1d" | "24h" | "daily" => "1d",
        "3d" => "3d",
        "1w" | "7d" | "weekly" => "1w",
        "1M" | "1mo" | "monthly" => "1M",
        _ => "1h",
    }
}

fn map_interval_okx(input: &str) -> &'static str {
    match input.trim().to_ascii_lowercase().as_str() {
        "1m" => "1m",
        "3m" => "3m",
        "5m" => "5m",
        "15m" => "15m",
        "30m" => "30m",
        "1h" => "1H",
        "2h" => "2H",
        "4h" => "4H",
        "6h" => "6H",
        "12h" => "12H",
        "1d" | "24h" | "daily" => "1D",
        "3d" => "3D",
        "1w" | "7d" | "weekly" => "1W",
        "1M" | "1mo" | "monthly" => "1M",
        _ => "1H",
    }
}

fn trade_to_json(t: &TradeInput) -> Value {
    json!({
        "exchange": t.exchange,
        "symbol": t.symbol,
        "side": t.side,
        "order_type": t.order_type,
        "qty": t.qty,
        "qty_all": t.qty_all,
        "quote_qty_usd": t.quote_qty_usd,
        "price": t.price,
        "stop_price": t.stop_price,
        "time_in_force": t.time_in_force,
        "client_order_id": t.client_order_id,
        "confirm": t.confirm
    })
}

fn load_root_config() -> RootConfig {
    let root = workspace_root();
    let cfg_path = root.join("configs/crypto.toml");
    if let Ok(raw) = std::fs::read_to_string(&cfg_path) {
        if let Ok(parsed) = toml::from_str::<RootConfig>(&raw) {
            return parsed;
        }
    }

    let legacy_path = root.join("configs/config.toml");
    if let Ok(raw) = std::fs::read_to_string(legacy_path) {
        if let Ok(parsed) = toml::from_str::<LegacyRootConfig>(&raw) {
            return RootConfig {
                crypto: parsed.crypto,
                ..RootConfig::default()
            };
        }
    }
    RootConfig::default()
}

fn apply_context_credentials(base: &RootConfig, context: &SkillContext) -> RootConfig {
    let mut cfg = base.clone();
    let _ = context
        .user_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    cfg.binance.enabled = false;
    cfg.binance.api_key.clear();
    cfg.binance.api_secret.clear();
    cfg.okx.enabled = false;
    cfg.okx.api_key.clear();
    cfg.okx.api_secret.clear();
    cfg.okx.passphrase.clear();
    if let Some(binance) = context.exchange_credentials.get("binance") {
        let api_key = binance.api_key.trim();
        let api_secret = binance.api_secret.trim();
        if !api_key.is_empty() && !api_secret.is_empty() {
            cfg.binance.enabled = true;
            cfg.binance.api_key = api_key.to_string();
            cfg.binance.api_secret = api_secret.to_string();
        }
    }
    if let Some(okx) = context.exchange_credentials.get("okx") {
        let api_key = okx.api_key.trim();
        let api_secret = okx.api_secret.trim();
        let passphrase = okx.passphrase.as_deref().unwrap_or("").trim();
        if !api_key.is_empty() && !api_secret.is_empty() && !passphrase.is_empty() {
            cfg.okx.enabled = true;
            cfg.okx.api_key = api_key.to_string();
            cfg.okx.api_secret = api_secret.to_string();
            cfg.okx.passphrase = passphrase.to_string();
        }
    }
    cfg
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_ts_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn now_iso_ts() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

fn to_query(params: &[(&str, String)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn hmac_sha256_bytes(secret: &str, message: &str) -> Result<Vec<u8>, String> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|err| format!("build hmac failed: {err}"))?;
    mac.update(message.as_bytes());
    Ok(mac.finalize().into_bytes().to_vec())
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

fn fmt_num(v: f64) -> String {
    let s = format!("{:.8}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect::<String>() + "..."
}

fn default_binance_base_url() -> String {
    "https://api.binance.com".to_string()
}

fn default_okx_base_url() -> String {
    "https://www.okx.com".to_string()
}

fn default_btc_onchain_fees_api_url() -> String {
    "https://mempool.space/api/v1/fees/recommended".to_string()
}

fn default_eth_onchain_stats_api_url() -> String {
    "https://api.blockchair.com/ethereum/stats".to_string()
}

fn default_coingecko_simple_price_api_url() -> String {
    "https://api.coingecko.com/api/v3/simple/price?ids={ids}&vs_currencies=usd&include_24hr_change=true"
        .to_string()
}

fn default_gateio_quote_ticker_api_path() -> String {
    "/api/v4/spot/tickers?currency_pair={currency_pair}".to_string()
}

fn default_coinbase_quote_ticker_api_path() -> String {
    "/products/{product_id}/ticker".to_string()
}

fn default_kraken_quote_ticker_api_path() -> String {
    "/0/public/Ticker?pair={pair}".to_string()
}

fn default_gateio_book_ticker_api_path() -> String {
    "/api/v4/spot/tickers?currency_pair={currency_pair}".to_string()
}

fn default_coinbase_book_ticker_api_path() -> String {
    "/products/{product_id}/ticker".to_string()
}

fn default_kraken_book_ticker_api_path() -> String {
    "/0/public/Ticker?pair={pair}".to_string()
}

fn default_binance_quote_24hr_api_path() -> String {
    "/api/v3/ticker/24hr?symbol={symbol}".to_string()
}

fn default_binance_quote_price_api_path() -> String {
    "/api/v3/ticker/price?symbol={symbol}".to_string()
}

fn default_binance_book_ticker_api_path() -> String {
    "/api/v3/ticker/bookTicker?symbol={symbol}".to_string()
}

fn default_okx_market_ticker_api_path() -> String {
    "/api/v5/market/ticker?instId={inst_id}".to_string()
}

fn default_eth_address_native_balance_api_url() -> String {
    "https://eth.blockscout.com/api?module=account&action=balance&address={address}".to_string()
}

fn default_eth_address_token_balance_api_url() -> String {
    "https://eth.blockscout.com/api?module=account&action=tokenbalance&contractaddress={contract}&address={address}"
        .to_string()
}

fn default_eth_address_tx_list_api_url() -> String {
    "https://eth.blockscout.com/api?module=account&action=txlist&address={address}&sort=desc&offset={limit}"
        .to_string()
}

fn default_recv_window() -> u64 {
    5000
}

fn default_okx_simulated() -> bool {
    true
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
