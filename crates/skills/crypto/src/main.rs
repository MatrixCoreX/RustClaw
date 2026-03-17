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
static I18N: OnceLock<TextCatalog> = OnceLock::new();
const PRICE_ALERT_TRIGGERED_TAG: &str = "[PRICE_ALERT_TRIGGERED]";
const PRICE_ALERT_NOT_TRIGGERED_TAG: &str = "[PRICE_ALERT_NOT_TRIGGERED]";

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
    #[serde(default)]
    #[allow(dead_code)] // kept for config compatibility; confirmation is now planner-decided
    require_explicit_send: Option<bool>,
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

#[derive(Debug, Clone)]
struct TextCatalog {
    current: HashMap<String, String>,
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

fn tr(key: &str) -> String {
    I18N.get()
        .and_then(|c| c.current.get(key))
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

fn tr_with(key: &str, vars: &[(&str, &str)]) -> String {
    let mut out = tr(key);
    for (name, value) in vars {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

fn i18n_lang(cfg: &RootConfig) -> String {
    cfg.crypto
        .language
        .as_deref()
        .unwrap_or("zh-CN")
        .trim()
        .to_string()
}

fn default_crypto_catalog(lang: &str) -> TextCatalog {
    let mut current = HashMap::new();
    let _ = lang;
    current.insert("crypto.err.invalid_input".to_string(), "invalid input: {error}".to_string());
    current.insert("crypto.err.args_object".to_string(), "args must be object".to_string());
    current.insert(
        "crypto.err.action_blocked".to_string(),
        "action is blocked by config: {action}".to_string(),
    );
    current.insert(
        "crypto.err.build_http_client".to_string(),
        "build http client failed: {error}".to_string(),
    );
    current.insert("crypto.err.unsupported_action".to_string(), "unsupported action".to_string());
    current.insert("crypto.err.symbol_required".to_string(), "symbol is required".to_string());
    current.insert(
        "crypto.err.symbols_required".to_string(),
        "symbols or symbol is required".to_string(),
    );
    current.insert("crypto.err.symbols_empty".to_string(), "symbols is empty".to_string());
    current.insert("crypto.err.no_candles".to_string(), "no candles returned".to_string());
    current.insert(
        "crypto.err.indicator_requires_close_prices".to_string(),
        "indicator requires close_prices".to_string(),
    );
    current.insert(
        "crypto.err.unsupported_chain".to_string(),
        "unsupported chain; use bitcoin|ethereum".to_string(),
    );
    current.insert(
        "crypto.err.unsupported_execution_exchange".to_string(),
        "unsupported execution exchange: {exchange}".to_string(),
    );
    current.insert(
        "crypto.err.unsupported_exchange_for_order_status".to_string(),
        "unsupported exchange for order_status: {exchange}".to_string(),
    );
    current.insert(
        "crypto.err.unsupported_exchange_for_cancel_order".to_string(),
        "unsupported exchange for cancel_order: {exchange}".to_string(),
    );
    current.insert(
        "crypto.err.unsupported_exchange_for_positions".to_string(),
        "unsupported exchange for positions: {exchange}".to_string(),
    );
    current.insert("crypto.err.order_not_found".to_string(), "order not found: {order_id}".to_string());
    current.insert("crypto.err.order_id_required".to_string(), "order_id is required".to_string());
    current.insert(
        "crypto.err.order_cannot_cancel_from_status".to_string(),
        "order cannot be cancelled from status {status}".to_string(),
    );
    current.insert(
        "crypto.err.symbol_required_for_binance_order_status".to_string(),
        "symbol is required for binance order_status".to_string(),
    );
    current.insert(
        "crypto.err.order_or_client_order_id_required".to_string(),
        "order_id or client_order_id is required".to_string(),
    );
    current.insert(
        "crypto.err.symbol_required_for_binance_cancel_order".to_string(),
        "symbol is required for binance cancel_order".to_string(),
    );
    current.insert(
        "crypto.err.symbol_required_for_okx_order_status".to_string(),
        "symbol is required for okx order_status".to_string(),
    );
    current.insert(
        "crypto.err.symbol_required_for_okx_cancel_order".to_string(),
        "symbol is required for okx cancel_order".to_string(),
    );
    current.insert("crypto.err.side_invalid".to_string(), "side must be buy or sell".to_string());
    current.insert(
        "crypto.err.order_type_invalid".to_string(),
        "order_type must be market or limit".to_string(),
    );
    current.insert(
        "crypto.err.qty_required_number".to_string(),
        "qty is required and must be number".to_string(),
    );
    current.insert("crypto.err.qty_must_gt_zero".to_string(), "qty must be > 0".to_string());
    current.insert(
        "crypto.err.threshold_pct_must_gt_zero".to_string(),
        "threshold_pct must be > 0".to_string(),
    );
    current.insert(
        "crypto.err.symbol_not_on_binance".to_string(),
        "symbol is not available on Binance spot: {symbol}".to_string(),
    );
    current.insert(
        "crypto.err.price_required_for_limit".to_string(),
        "price is required for limit order".to_string(),
    );
    current.insert(
        "crypto.err.trade_submit_requires_confirm".to_string(),
        "trade_submit requires confirm=true".to_string(),
    );
    current.insert(
        "crypto.err.exchange_not_allowed".to_string(),
        "exchange is not allowed: {exchange}".to_string(),
    );
    current.insert(
        "crypto.err.symbol_not_allowed".to_string(),
        "symbol is not allowed: {symbol}".to_string(),
    );
    current.insert(
        "crypto.err.binance_not_bound".to_string(),
        "Binance API is not bound for this key. Configure first:\nTelegram: /cryptoapi set binance <api_key> <api_secret>"
            .to_string(),
    );
    current.insert(
        "crypto.err.binance_credentials_incomplete".to_string(),
        "Binance API credentials are incomplete for this key. Configure first:\nTelegram: /cryptoapi set binance <api_key> <api_secret>"
            .to_string(),
    );
    current.insert(
        "crypto.err.okx_not_bound".to_string(),
        "OKX API is not bound for this key. Configure first:\nTelegram: /cryptoapi set okx <api_key> <api_secret> <passphrase>"
            .to_string(),
    );
    current.insert(
        "crypto.err.okx_credentials_incomplete".to_string(),
        "OKX API credentials are incomplete for this key. Configure first:\nTelegram: /cryptoapi set okx <api_key> <api_secret> <passphrase>"
            .to_string(),
    );
    current.insert("crypto.msg.no_orders_yet".to_string(), "no orders yet".to_string());
    current.insert("crypto.msg.no_filled_positions".to_string(), "no filled positions".to_string());
    current.insert("crypto.msg.no_balances".to_string(), "no balances".to_string());
    current.insert(
        "crypto.msg.market_quote_line_gateio".to_string(),
        "- GATEIO ${price}".to_string(),
    );
    current.insert(
        "crypto.msg.market_quote_line_coinbase".to_string(),
        "- COINBASE ${price}".to_string(),
    );
    current.insert(
        "crypto.msg.market_quote_line_kraken".to_string(),
        "- KRAKEN ${price}".to_string(),
    );
    current.insert(
        "crypto.msg.price_alert_triggered".to_string(),
        "ALERT {symbol}: {window_minutes}m change {change_pct}% reached threshold {threshold_pct}% (start={start_price}, now={current_price}, direction={direction})".to_string(),
    );
    current.insert(
        "crypto.msg.price_alert_not_triggered".to_string(),
        "{symbol} monitor: {window_minutes}m change {change_pct}% has not reached threshold {threshold_pct}% (start={start_price}, now={current_price}, direction={direction})".to_string(),
    );
    current.insert(
        "crypto.msg.trade_submitted_pending_suffix".to_string(),
        " (order placed, awaiting fill)".to_string(),
    );
    current.insert("crypto.msg.trade_preview_summary".to_string(), "trade_preview {exchange} {symbol} {side} {qty_part} notional_usd={notional} checks={checks}".to_string());
    current.insert("crypto.msg.trade_submitted_filled".to_string(), "trade_submitted order_id={order_id} status=FILLED {exchange} {symbol} {side} qty_filled={qty_filled}{price_part} quote_spent={quote_spent} USDT".to_string());
    current.insert("crypto.msg.trade_submitted_partial".to_string(), "trade_submitted order_id={order_id} status=PARTIAL {exchange} {symbol} {side} filled={filled}/{total} notional_usd={notional}".to_string());
    current.insert("crypto.msg.trade_submitted_fallback".to_string(), "trade_submitted order_id={order_id} status={status} {exchange} {symbol} {side} qty={qty} notional_usd={notional}".to_string());
    current.insert("crypto.msg.open_orders_none".to_string(), "open_orders {exchange}{symbol_suffix}: none".to_string());
    current.insert("crypto.msg.open_orders_header".to_string(), "open_orders {exchange} count={count}\n{body}".to_string());
    current.insert("crypto.msg.open_orders_line_binance".to_string(), "{sym} {side} {otype} qty={orig_qty} filled={exec_qty} price={price} status={status} id={oid}".to_string());
    current.insert("crypto.msg.open_orders_line_okx".to_string(), "{inst} {side} {otype} sz={sz} price={px} state={state} id={oid}".to_string());
    current.insert("crypto.msg.cancel_all_orders_done".to_string(), "cancel_all_orders {exchange} {sym} cancelled={count}".to_string());
    current.insert("crypto.msg.cancel_all_orders_no_open_orders".to_string(), "cancel_all_orders {exchange} {sym_info}: no open orders".to_string());
    current.insert("crypto.msg.cancel_order_done".to_string(), "order_cancelled {id_text}".to_string());
    current.insert("crypto.msg.indicator_rsi_summary".to_string(), "{symbol} RSI{period}={rsi} last={last} signal={signal}".to_string());
    current.insert("crypto.msg.indicator_ema_summary".to_string(), "{symbol} EMA{period}={ema} last={last} signal={signal}".to_string());
    current.insert("crypto.msg.indicator_sma_summary".to_string(), "{symbol} SMA{period}={sma} last={last} signal={signal}".to_string());
    current.insert("crypto.msg.indicator_signal_neutral".to_string(), "neutral".to_string());
    current.insert("crypto.msg.indicator_signal_overbought".to_string(), "overbought".to_string());
    current.insert("crypto.msg.indicator_signal_oversold".to_string(), "oversold".to_string());
    current.insert("crypto.msg.indicator_signal_above_ema".to_string(), "above_ema".to_string());
    current.insert("crypto.msg.indicator_signal_below_ema".to_string(), "below_ema".to_string());
    current.insert("crypto.msg.indicator_signal_above_sma".to_string(), "above_sma".to_string());
    current.insert("crypto.msg.indicator_signal_below_sma".to_string(), "below_sma".to_string());
    current.insert("crypto.msg.trade_submitted_pending".to_string(), "Order placed (pending): order_id={order_id}, status=PENDING, {exchange} {symbol} {side} {order_type} qty={qty}{price_str}{stop_str} notional_usd={notional}{pending_suffix}".to_string());
    current.insert("crypto.msg.positions_balance_line".to_string(), "{asset} free={free} locked={locked}".to_string());
    current.insert("crypto.msg.positions_balance_line_okx".to_string(), "{ccy} eq={eq} avail={avail}".to_string());
    current.insert("crypto.msg.order_status_summary".to_string(), "Order status: {symbol} id={id_text} status={status}".to_string());
    current.insert("crypto.msg.order_status_skipped_missing_symbol".to_string(), "Order status skipped ({id_text}): missing symbol, {exchange} requires symbol for query".to_string());
    current.insert("crypto.msg.onchain_btc_fees".to_string(), "BTC fee(sat/vB): fastest={fastest}, half_hour={half_hour}, hour={hour}".to_string());
    current.insert("crypto.msg.onchain_eth_stats_summary".to_string(), "ETH onchain: tx_24h={tx_24h}, blocks_24h={blocks_24h}, market_price_usd={market_price_usd}".to_string());
    current.insert("crypto.msg.onchain_eth_native_summary".to_string(), "ETH address={address} token=ETH balance={balance} recent_txs={recent_txs}".to_string());
    current.insert("crypto.msg.onchain_eth_token_summary".to_string(), "ETH address={address} token={token} balance={balance} recent_txs={recent_txs}".to_string());
    TextCatalog { current }
}

fn flatten_toml_table(prefix: &str, table: &toml::map::Map<String, toml::Value>, out: &mut HashMap<String, String>) {
    for (k, v) in table {
        let key = if prefix.is_empty() {
            k.to_string()
        } else {
            format!("{prefix}.{k}")
        };
        match v {
            toml::Value::String(text) => {
                out.insert(key, text.to_string());
            }
            toml::Value::Table(child) => {
                flatten_toml_table(&key, child, out);
            }
            _ => {}
        }
    }
}

fn load_external_i18n(path: &Path) -> Option<HashMap<String, String>> {
    let raw = std::fs::read_to_string(path).ok()?;
    let value: toml::Value = toml::from_str(&raw).ok()?;
    let mut out = HashMap::new();
    if let Some(dict) = value.get("dict").and_then(|v| v.as_table()) {
        flatten_toml_table("", dict, &mut out);
        return Some(out);
    }
    if let Some(root) = value.as_table() {
        flatten_toml_table("", root, &mut out);
        if out.is_empty() {
            return None;
        }
        return Some(out);
    }
    None
}

fn init_i18n(cfg: &RootConfig, workspace_root: &Path) {
    let lang = i18n_lang(cfg);
    let mut catalog = default_crypto_catalog(&lang);
    let path = cfg
        .crypto
        .i18n_path
        .as_deref()
        .map(|p| workspace_root.join(p))
        .unwrap_or_else(|| workspace_root.join(format!("configs/i18n/crypto.{lang}.toml")));
    if let Some(override_dict) = load_external_i18n(&path) {
        for (k, v) in override_dict {
            catalog.current.insert(k, v);
        }
    }
    let _ = I18N.set(catalog);
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
                    extra: None,
                    error_text: Some(err),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: None,
                error_text: Some(tr_with("crypto.err.invalid_input", &[("error", &err.to_string())])),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn execute(cfg: &RootConfig, args: Value, context: Option<Value>) -> Result<(String, Value), String> {
    let context = context
        .and_then(|v| serde_json::from_value::<SkillContext>(v).ok())
        .unwrap_or_default();
    let cfg = apply_context_credentials(cfg, &context);
    let obj = args
        .as_object()
        .ok_or_else(|| tr("crypto.err.args_object"))?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("quote")
        .trim()
        .to_ascii_lowercase();
    let action = match action.as_str() {
        "get_price" => "quote".to_string(),
        "get_multi_price" => "multi_quote".to_string(),
        "price_monitor" | "monitor_price" | "price_alert" | "volatility_alert" => {
            "price_alert_check".to_string()
        }
        other => other.to_string(),
    };
    if cfg
        .crypto
        .blocked_actions
        .iter()
        .any(|v| v.trim().eq_ignore_ascii_case(&action))
    {
        return Err(tr_with("crypto.err.action_blocked", &[("action", &action)]));
    }
    let timeout_seconds = obj
        .get("timeout_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| cfg.crypto.timeout_seconds.unwrap_or(20))
        .clamp(3, 120);
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .build()
        .map_err(|err| tr_with("crypto.err.build_http_client", &[("error", &err.to_string())]))?;

    match action.as_str() {
        "quote" => handle_quote(&client, &cfg, obj),
        "multi_quote" => handle_multi_quote(&client, &cfg, obj),
        "get_book_ticker" | "book_ticker" => handle_book_ticker(&client, &cfg, obj),
        "binance_symbol_check" => handle_binance_symbol_check(&client, &cfg, obj),
        "normalize_symbol" => handle_normalize_symbol(obj),
        "healthcheck" => handle_healthcheck(&client, &cfg, obj),
        "candles" => handle_candles(&client, &cfg, obj),
        "indicator" => handle_indicator(&client, &cfg, obj),
        "price_alert_check" => handle_price_alert_check(&client, &cfg, obj),
        "onchain" => handle_onchain(&client, &cfg, obj),
        "trade_preview" => handle_trade_preview(&client, &cfg, obj),
        "trade_submit" => handle_trade_submit(&client, &cfg, obj),
        "order_status" => handle_order_status(&client, &cfg, obj),
        "cancel_order" => handle_cancel_order(&client, &cfg, obj),
        "cancel_all_orders" | "cancel_open_orders" => handle_cancel_all_orders(&client, &cfg, obj),
        "open_orders" | "get_open_orders" | "pending_orders" => handle_open_orders(&client, &cfg, obj),
        "trade_history" | "my_trades" | "recent_trades" => handle_trade_history(&client, &cfg, obj),
        "positions" => handle_positions(&client, &cfg, obj),
        _ => Err(tr("crypto.err.unsupported_action")),
    }
}

fn handle_quote(client: &Client, cfg: &RootConfig, obj: &serde_json::Map<String, Value>) -> Result<(String, Value), String> {
    let symbol = obj
        .get("symbol")
        .and_then(|v| v.as_str())
        .ok_or_else(|| tr("crypto.err.symbol_required"))?;
    let b = fetch_quote_from_binance(client, cfg, symbol);
    let o = fetch_quote_from_okx(client, cfg, symbol);
    let g = fetch_quote_from_gateio(client, cfg, symbol);
    let cb = fetch_quote_from_coinbase(client, cfg, symbol);
    let k = fetch_quote_from_kraken(client, cfg, symbol);
    let c = fetch_quote_from_coingecko(client, cfg, symbol);
    let mut errors = Vec::new();
    if let Err(err) = &b {
        errors.push(format!("binance={err}"));
    }
    if let Err(err) = &o {
        errors.push(format!("okx={err}"));
    }
    if let Err(err) = &g {
        errors.push(format!("gateio={err}"));
    }
    if let Err(err) = &cb {
        errors.push(format!("coinbase={err}"));
    }
    if let Err(err) = &k {
        errors.push(format!("kraken={err}"));
    }
    if let Err(err) = &c {
        errors.push(format!("coingecko={err}"));
    }
    let binance = b.ok();
    let okx = o.ok();
    let gateio = g.ok();
    let coinbase = cb.ok();
    let kraken = k.ok();
    let coingecko = c.ok();
    if binance.is_none()
        && okx.is_none()
        && gateio.is_none()
        && coinbase.is_none()
        && kraken.is_none()
        && coingecko.is_none()
    {
        return Err(format!("all market sources failed: {}", errors.join("; ")));
    }
    let pref = binance
        .clone()
        .or(okx.clone())
        .or(gateio.clone())
        .or(coinbase.clone())
        .or(kraken.clone())
        .or(coingecko.clone())
        .ok_or_else(|| "no quote available".to_string())?;
    let text = format_market_quote_line(
        &pref.symbol,
        binance.as_ref(),
        okx.as_ref(),
        gateio.as_ref(),
        coinbase.as_ref(),
        kraken.as_ref(),
        coingecko.as_ref(),
    );
    Ok((
        text,
        json!({
            "action": "quote",
            "quote": pref,
            "quotes_by_exchange": {
                "binance": binance,
                "okx": okx,
                "gateio": gateio,
                "coinbase": coinbase,
                "kraken": kraken,
                "coingecko": coingecko
            },
            "errors": errors
        }),
    ))
}

fn handle_multi_quote(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let symbols: Vec<String> = if let Some(arr) = obj.get("symbols").and_then(|v| v.as_array()) {
        arr.iter()
            .filter_map(|v| v.as_str())
            .map(|v| v.to_string())
            .take(20)
            .collect()
    } else {
        let single = obj
            .get("symbol")
            .and_then(|v| v.as_str())
            .ok_or_else(|| tr("crypto.err.symbols_required"))?;
        vec![single.to_string()]
    };
    if symbols.is_empty() {
        return Err(tr("crypto.err.symbols_empty"));
    }
    let mut quotes = Vec::new();
    let mut lines = Vec::new();
    let mut by_exchange_rows = Vec::new();
    for s in symbols {
        let b = fetch_quote_from_binance(client, cfg, &s).ok();
        let o = fetch_quote_from_okx(client, cfg, &s).ok();
        let g = fetch_quote_from_gateio(client, cfg, &s).ok();
        let cb = fetch_quote_from_coinbase(client, cfg, &s).ok();
        let k = fetch_quote_from_kraken(client, cfg, &s).ok();
        let c = fetch_quote_from_coingecko(client, cfg, &s).ok();
        if b.is_none() && o.is_none() && g.is_none() && cb.is_none() && k.is_none() && c.is_none() {
            return Err(format!("quote failed on all sources for symbol={}", normalize_symbol(&s)));
        }
        let chosen = b
            .clone()
            .or(o.clone())
            .or(g.clone())
            .or(cb.clone())
            .or(k.clone())
            .or(c.clone())
            .ok_or_else(|| "no quote available".to_string())?;
        lines.push(format_market_quote_line(
            &chosen.symbol,
            b.as_ref(),
            o.as_ref(),
            g.as_ref(),
            cb.as_ref(),
            k.as_ref(),
            c.as_ref(),
        ));
        quotes.push(chosen.clone());
        by_exchange_rows.push(json!({
            "symbol": chosen.symbol,
            "binance": b,
            "okx": o,
            "gateio": g,
            "coinbase": cb,
            "kraken": k,
            "coingecko": c
        }));
    }
    let mut extra = json!({ "action": "multi_quote", "quotes": quotes });
    extra["quotes_by_exchange"] = Value::Array(by_exchange_rows);
    Ok((lines.join("\n"), extra))
}

fn format_market_quote_line(
    symbol: &str,
    binance: Option<&Quote>,
    okx: Option<&Quote>,
    gateio: Option<&Quote>,
    coinbase: Option<&Quote>,
    kraken: Option<&Quote>,
    coingecko: Option<&Quote>,
) -> String {
    let mut lines = Vec::new();
    lines.push(tr_with("crypto.msg.market_quote_header", &[("symbol", symbol)]));
    if let Some(q) = binance {
        let price = format!("{:.6}", q.price_usd);
        lines.push(tr_with(
            "crypto.msg.market_quote_line_binance",
            &[("price", &price)],
        ));
    }
    if let Some(q) = okx {
        let price = format!("{:.6}", q.price_usd);
        lines.push(tr_with(
            "crypto.msg.market_quote_line_okx",
            &[("price", &price)],
        ));
    }
    if let Some(q) = gateio {
        let price = format!("{:.6}", q.price_usd);
        lines.push(tr_with(
            "crypto.msg.market_quote_line_gateio",
            &[("price", &price)],
        ));
    }
    if let Some(q) = coinbase {
        let price = format!("{:.6}", q.price_usd);
        lines.push(tr_with(
            "crypto.msg.market_quote_line_coinbase",
            &[("price", &price)],
        ));
    }
    if let Some(q) = kraken {
        let price = format!("{:.6}", q.price_usd);
        lines.push(tr_with(
            "crypto.msg.market_quote_line_kraken",
            &[("price", &price)],
        ));
    }
    if let Some(q) = coingecko {
        let price = format!("{:.6}", q.price_usd);
        lines.push(tr_with(
            "crypto.msg.market_quote_line_coingecko",
            &[("price", &price)],
        ));
    }
    if lines.len() == 1 {
        return tr_with("crypto.msg.market_quote_unavailable", &[("symbol", symbol)]);
    }
    lines.join("\n")
}

fn handle_book_ticker(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let symbol = obj
        .get("symbol")
        .and_then(|v| v.as_str())
        .ok_or_else(|| tr("crypto.err.symbol_required"))?;
    let exchange_input = obj.get("exchange").and_then(|v| v.as_str()).unwrap_or("dual");
    let dual_mode = matches!(
        exchange_input.trim().to_ascii_lowercase().as_str(),
        "" | "dual" | "both" | "all" | "auto"
    );
    if dual_mode {
        let b = fetch_book_ticker_from_binance(client, cfg, symbol).ok();
        let o = fetch_book_ticker_from_okx(client, cfg, symbol).ok();
        let g = fetch_book_ticker_from_gateio(client, cfg, symbol).ok();
        let cb = fetch_book_ticker_from_coinbase(client, cfg, symbol).ok();
        let k = fetch_book_ticker_from_kraken(client, cfg, symbol).ok();
        if b.is_none() && o.is_none() && g.is_none() && cb.is_none() && k.is_none() {
            return Err(format!(
                "book ticker failed on all exchanges for symbol={}",
                normalize_symbol(symbol)
            ));
        }
        let s = normalize_symbol(symbol);
        let mut lines = vec![format!("{s} | 盘口来源：")];
        if let Some(x) = b.as_ref() {
            lines.push(format!(
                "- BINANCE bid/ask={} / {}",
                fmt_num(x.bid_price),
                fmt_num(x.ask_price)
            ));
        }
        if let Some(x) = o.as_ref() {
            lines.push(format!(
                "- OKX bid/ask={} / {}",
                fmt_num(x.bid_price),
                fmt_num(x.ask_price)
            ));
        }
        if let Some(x) = g.as_ref() {
            lines.push(format!(
                "- GATEIO bid/ask={} / {}",
                fmt_num(x.bid_price),
                fmt_num(x.ask_price)
            ));
        }
        if let Some(x) = cb.as_ref() {
            lines.push(format!(
                "- COINBASE bid/ask={} / {}",
                fmt_num(x.bid_price),
                fmt_num(x.ask_price)
            ));
        }
        if let Some(x) = k.as_ref() {
            lines.push(format!(
                "- KRAKEN bid/ask={} / {}",
                fmt_num(x.bid_price),
                fmt_num(x.ask_price)
            ));
        }
        let text = lines.join("\n");
        return Ok((
            text,
            json!({
                "action":"book_ticker",
                "symbol": s,
                "book_ticker_by_exchange": {
                    "binance": b,
                    "okx": o,
                    "gateio": g,
                    "coinbase": cb,
                    "kraken": k
                }
            }),
        ));
    }
    let exchange = resolve_exchange(Some(exchange_input), cfg);
    let bt = match exchange.as_str() {
        "okx" => fetch_book_ticker_from_okx(client, cfg, symbol)?,
        "gateio" => fetch_book_ticker_from_gateio(client, cfg, symbol)?,
        "coinbase" => fetch_book_ticker_from_coinbase(client, cfg, symbol)?,
        "kraken" => fetch_book_ticker_from_kraken(client, cfg, symbol)?,
        _ => fetch_book_ticker_from_binance(client, cfg, symbol)?,
    };
    let text = format!(
        "{} {} bid/ask={} / {}",
        bt.symbol,
        bt.exchange.to_ascii_uppercase(),
        fmt_num(bt.bid_price),
        fmt_num(bt.ask_price)
    );
    Ok((text, json!({"action":"book_ticker","book_ticker":bt})))
}

fn handle_normalize_symbol(obj: &serde_json::Map<String, Value>) -> Result<(String, Value), String> {
    let symbol_raw = obj
        .get("symbol")
        .and_then(|v| v.as_str())
        .ok_or_else(|| tr("crypto.err.symbol_required"))?;
    let binance_symbol = normalize_symbol(symbol_raw);
    let okx_symbol = to_okx_inst_id(symbol_raw);
    let text = format!("symbol={} -> binance={} okx={}", symbol_raw, binance_symbol, okx_symbol);
    Ok((
        text,
        json!({
            "action":"normalize_symbol",
            "symbol_raw": symbol_raw,
            "binance_symbol": binance_symbol,
            "okx_inst_id": okx_symbol
        }),
    ))
}

fn handle_healthcheck(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let symbol = obj
        .get("symbol")
        .and_then(|v| v.as_str())
        .unwrap_or("BTCUSDT");
    let normalized = normalize_symbol(symbol);
    let okx_inst = to_okx_inst_id(symbol);
    let binance_url = build_exchange_url(
        cfg.binance.base_url.trim_end_matches('/'),
        cfg.crypto.binance_quote_price_api_path.trim(),
        &[("symbol", &normalized)],
    );
    let okx_url = build_exchange_url(
        cfg.okx.base_url.trim_end_matches('/'),
        cfg.crypto.okx_market_ticker_api_path.trim(),
        &[("inst_id", &okx_inst), ("instId", &okx_inst)],
    );
    let mut checks = Vec::new();
    for (exchange, url) in [("binance", binance_url), ("okx", okx_url)] {
        let started = Instant::now();
        let out = client.get(&url).send();
        let latency_ms = started.elapsed().as_millis() as u64;
        match out {
            Ok(resp) => {
                let status = resp.status().as_u16();
                checks.push(json!({
                    "exchange": exchange,
                    "ok": status >= 200 && status < 300,
                    "latency_ms": latency_ms,
                    "url": url,
                    "http_status": status
                }));
            }
            Err(err) => {
                checks.push(json!({
                    "exchange": exchange,
                    "ok": false,
                    "latency_ms": latency_ms,
                    "url": url,
                    "error": err.to_string()
                }));
            }
        }
    }
    let ok = checks.iter().all(|x| x.get("ok").and_then(|v| v.as_bool()) == Some(true));
    let text = if ok {
        "crypto healthcheck ok (binance+okx)".to_string()
    } else {
        "crypto healthcheck degraded".to_string()
    };
    Ok((text, json!({"action":"healthcheck","ok":ok,"checks":checks})))
}

fn handle_candles(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let symbol = normalize_symbol(
        obj.get("symbol")
            .and_then(|v| v.as_str())
            .ok_or_else(|| tr("crypto.err.symbol_required"))?,
    );
    let interval = obj
        .get("timeframe")
        .and_then(|v| v.as_str())
        .unwrap_or("1h");
    let limit = obj
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(30)
        .clamp(1, 500);
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg);
    let candles = if exchange == "okx" {
        fetch_candles_ohlcv_okx(client, cfg, &symbol, interval, limit)?
    } else {
        fetch_candles_ohlcv_binance(client, cfg, &symbol, interval, limit)?
    };
    if candles.is_empty() {
        return Err(tr("crypto.err.no_candles"));
    }
    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let last = closes.last().copied().unwrap_or(0.0);
    let first = closes.first().copied().unwrap_or(last);
    let delta = if first > 0.0 {
        (last - first) / first * 100.0
    } else {
        0.0
    };
    let high = candles.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
    let low = candles.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    let total_volume: f64 = candles.iter().map(|c| c.volume).sum();
    let ohlcv_arr: Vec<Value> = candles
        .iter()
        .map(|c| {
            json!({
                "open": c.open,
                "high": c.high,
                "low": c.low,
                "close": c.close,
                "volume": c.volume,
                "quote_volume": c.quote_volume
            })
        })
        .collect();
    Ok((
        format!(
            "{} {} close={} change={:+.2}% high={} low={} vol={:.4} candles={}",
            symbol, interval, last, delta, high, low, total_volume, candles.len()
        ),
        json!({
            "action":"candles",
            "symbol":symbol,
            "timeframe":interval,
            "exchange":exchange,
            "close_prices": closes,
            "candles": ohlcv_arr,
            "high": high,
            "low": low,
            "volume": total_volume
        }),
    ))
}

fn calc_sma(values: &[f64], period: usize) -> Option<f64> {
    if values.len() < period {
        return None;
    }
    let tail = &values[values.len() - period..];
    Some(tail.iter().sum::<f64>() / period as f64)
}

fn calc_ema(values: &[f64], period: usize) -> Option<f64> {
    if values.len() < period {
        return None;
    }
    let k = 2.0 / (period as f64 + 1.0);
    // seed with SMA of first `period` values
    let mut ema: f64 = values[..period].iter().sum::<f64>() / period as f64;
    for &price in &values[period..] {
        ema = price * k + ema * (1.0 - k);
    }
    Some(ema)
}

fn calc_rsi(values: &[f64], period: usize) -> Option<f64> {
    if values.len() <= period {
        return None;
    }
    let mut gains = 0.0_f64;
    let mut losses = 0.0_f64;
    for i in 1..=period {
        let diff = values[i] - values[i - 1];
        if diff > 0.0 {
            gains += diff;
        } else {
            losses += -diff;
        }
    }
    let mut avg_gain = gains / period as f64;
    let mut avg_loss = losses / period as f64;
    // Wilder smoothing
    for i in (period + 1)..values.len() {
        let diff = values[i] - values[i - 1];
        let gain = if diff > 0.0 { diff } else { 0.0 };
        let loss = if diff < 0.0 { -diff } else { 0.0 };
        avg_gain = (avg_gain * (period as f64 - 1.0) + gain) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + loss) / period as f64;
    }
    if avg_loss == 0.0 {
        return Some(100.0);
    }
    let rs = avg_gain / avg_loss;
    Some(100.0 - 100.0 / (1.0 + rs))
}

fn handle_indicator(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let indicator_type = obj
        .get("indicator")
        .and_then(|v| v.as_str())
        .unwrap_or("sma")
        .trim()
        .to_ascii_lowercase();
    let mut args = obj.clone();
    args.entry("action".to_string())
        .or_insert(Value::String("candles".to_string()));
    // For RSI/EMA we need more candles: at least period*3 for accuracy
    let period = obj
        .get("period")
        .and_then(|v| v.as_u64())
        .unwrap_or(14)
        .clamp(2, 200) as usize;
    let min_needed = match indicator_type.as_str() {
        "rsi" => (period * 3 + 1) as u64,
        "ema" => (period * 3) as u64,
        _ => period as u64,
    };
    let limit_from_args = obj.get("limit").and_then(|v| v.as_u64()).unwrap_or(0);
    if limit_from_args < min_needed {
        args.insert("limit".to_string(), Value::from(min_needed.max(100)));
    }
    let (_, extra) = handle_candles(client, cfg, &args)?;
    let closes = extra
        .get("close_prices")
        .and_then(|v| v.as_array())
        .ok_or_else(|| tr("crypto.err.indicator_requires_close_prices"))?;
    let values: Vec<f64> = closes.iter().filter_map(|v| v.as_f64()).collect();
    let symbol = obj
        .get("symbol")
        .and_then(|v| v.as_str())
        .map(normalize_symbol)
        .unwrap_or_else(|| "UNKNOWN".to_string());
    let last = values.last().copied().unwrap_or(0.0);
    match indicator_type.as_str() {
        "rsi" => {
            let rsi = calc_rsi(&values, period).ok_or_else(|| {
                format!("not enough candles for RSI{}: got={}", period, values.len())
            })?;
            let signal_key = if rsi >= 70.0 {
                "crypto.msg.indicator_signal_overbought"
            } else if rsi <= 30.0 {
                "crypto.msg.indicator_signal_oversold"
            } else {
                "crypto.msg.indicator_signal_neutral"
            };
            let signal_display = tr(signal_key);
            let raw_signal = if rsi >= 70.0 { "overbought" } else if rsi <= 30.0 { "oversold" } else { "neutral" };
            Ok((
                tr_with(
                    "crypto.msg.indicator_rsi_summary",
                    &[
                        ("symbol", symbol.as_str()),
                        ("period", &period.to_string()),
                        ("rsi", &format!("{rsi:.2}")),
                        ("last", &format!("{last:.6}")),
                        ("signal", signal_display.as_str()),
                    ],
                ),
                json!({
                    "action":"indicator",
                    "indicator":"rsi",
                    "period":period,
                    "symbol":symbol,
                    "rsi":rsi,
                    "last":last,
                    "signal":raw_signal
                }),
            ))
        }
        "ema" => {
            let ema = calc_ema(&values, period).ok_or_else(|| {
                format!("not enough candles for EMA{}: got={}", period, values.len())
            })?;
            let signal_key = if last >= ema {
                "crypto.msg.indicator_signal_above_ema"
            } else {
                "crypto.msg.indicator_signal_below_ema"
            };
            let signal_display = tr(signal_key);
            let raw_signal = if last >= ema { "above_ema" } else { "below_ema" };
            Ok((
                tr_with(
                    "crypto.msg.indicator_ema_summary",
                    &[
                        ("symbol", symbol.as_str()),
                        ("period", &period.to_string()),
                        ("ema", &format!("{ema:.6}")),
                        ("last", &format!("{last:.6}")),
                        ("signal", signal_display.as_str()),
                    ],
                ),
                json!({
                    "action":"indicator",
                    "indicator":"ema",
                    "period":period,
                    "symbol":symbol,
                    "ema":ema,
                    "last":last,
                    "signal":raw_signal
                }),
            ))
        }
        _ => {
            if values.len() < period {
                return Err(format!(
                    "not enough candles for SMA{}: got={}",
                    period,
                    values.len()
                ));
            }
            let sma = calc_sma(&values, period).unwrap_or(0.0);
            let signal_key = if last >= sma {
                "crypto.msg.indicator_signal_above_sma"
            } else {
                "crypto.msg.indicator_signal_below_sma"
            };
            let signal_display = tr(signal_key);
            let raw_signal = if last >= sma { "above_sma" } else { "below_sma" };
            Ok((
                tr_with(
                    "crypto.msg.indicator_sma_summary",
                    &[
                        ("symbol", symbol.as_str()),
                        ("period", &period.to_string()),
                        ("sma", &format!("{sma:.6}")),
                        ("last", &format!("{last:.6}")),
                        ("signal", signal_display.as_str()),
                    ],
                ),
                json!({
                    "action":"indicator",
                    "indicator":"sma",
                    "period":period,
                    "symbol":symbol,
                    "sma":sma,
                    "last":last,
                    "signal":raw_signal
                }),
            ))
        }
    }
}

fn handle_binance_symbol_check(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let symbol = normalize_symbol(
        obj.get("symbol")
            .and_then(|v| v.as_str())
            .ok_or_else(|| tr("crypto.err.symbol_required"))?,
    );
    ensure_symbol_supported_on_binance(client, cfg, &symbol)?;
    Ok((
        format!("binance symbol check ok: {symbol}"),
        json!({
            "action":"binance_symbol_check",
            "exchange":"binance",
            "symbol":symbol,
            "ok":true
        }),
    ))
}

fn handle_price_alert_check(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let symbol = normalize_symbol(
        obj.get("symbol")
            .and_then(|v| v.as_str())
            .ok_or_else(|| tr("crypto.err.symbol_required"))?,
    );
    let max_window = cfg.crypto.alert_max_window_minutes.unwrap_or(240).clamp(1, 1440);
    let window_minutes = obj
        .get("window_minutes")
        .or_else(|| obj.get("minutes"))
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| cfg.crypto.alert_default_window_minutes.unwrap_or(15))
        .clamp(1, max_window);
    let threshold_pct = obj
        .get("threshold_pct")
        .or_else(|| obj.get("pct"))
        .or_else(|| obj.get("percent"))
        .and_then(value_to_f64)
        .unwrap_or_else(|| cfg.crypto.alert_default_threshold_pct.unwrap_or(5.0));
    if threshold_pct <= 0.0 {
        return Err(tr("crypto.err.threshold_pct_must_gt_zero"));
    }
    let direction = obj
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("both")
        .trim()
        .to_ascii_lowercase();
    let direction = match direction.as_str() {
        "up" | "rise" | "pump" => "up",
        "down" | "drop" | "dump" => "down",
        _ => "both",
    };
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg);
    // Always validate symbol availability; OKX falls back to Binance check if okx not configured
    if exchange != "okx" {
        ensure_symbol_supported_on_binance(client, cfg, &symbol)?;
    }
    let closes = if exchange == "okx" {
        fetch_candles_okx(client, cfg, &symbol, "1m", window_minutes.saturating_add(1))?
    } else {
        fetch_candles_binance(client, cfg, &symbol, "1m", window_minutes.saturating_add(1))?
    };
    if closes.len() < 2 {
        return Err(tr("crypto.err.no_candles"));
    }
    let start_price = closes.first().copied().unwrap_or(0.0);
    let current_price = closes.last().copied().unwrap_or(start_price);
    let change_pct = if start_price > 0.0 {
        (current_price - start_price) / start_price * 100.0
    } else {
        0.0
    };
    let triggered = match direction {
        "up" => change_pct >= threshold_pct,
        "down" => change_pct <= -threshold_pct,
        _ => change_pct.abs() >= threshold_pct,
    };
    let trend = if change_pct > 0.0 {
        "up"
    } else if change_pct < 0.0 {
        "down"
    } else {
        "flat"
    };
    let change_text = format!("{:+.2}", change_pct);
    let threshold_text = format!("{:.2}", threshold_pct);
    let start_text = format!("{:.6}", start_price);
    let current_text = format!("{:.6}", current_price);
    let text_body = if triggered {
        tr_with(
            "crypto.msg.price_alert_triggered",
            &[
                ("symbol", &symbol),
                ("window_minutes", &window_minutes.to_string()),
                ("change_pct", &change_text),
                ("threshold_pct", &threshold_text),
                ("start_price", &start_text),
                ("current_price", &current_text),
                ("direction", direction),
            ],
        )
    } else {
        tr_with(
            "crypto.msg.price_alert_not_triggered",
            &[
                ("symbol", &symbol),
                ("window_minutes", &window_minutes.to_string()),
                ("change_pct", &change_text),
                ("threshold_pct", &threshold_text),
                ("start_price", &start_text),
                ("current_price", &current_text),
                ("direction", direction),
            ],
        )
    };
    let tagged_text = if triggered {
        format!("{PRICE_ALERT_TRIGGERED_TAG} {text_body}")
    } else {
        format!("{PRICE_ALERT_NOT_TRIGGERED_TAG} {text_body}")
    };
    Ok((
        tagged_text,
        json!({
            "action":"price_alert_check",
            "symbol":symbol,
            "exchange":exchange,
            "window_minutes":window_minutes,
            "threshold_pct":threshold_pct,
            "direction":direction,
            "triggered":triggered,
            "trend":trend,
            "start_price":start_price,
            "current_price":current_price,
            "change_pct":change_pct,
            "candles":closes.len()
        }),
    ))
}


fn handle_onchain(client: &Client, cfg: &RootConfig, obj: &serde_json::Map<String, Value>) -> Result<(String, Value), String> {
    let chain = obj
        .get("chain")
        .and_then(|v| v.as_str())
        .unwrap_or("bitcoin")
        .trim()
        .to_ascii_lowercase();
    match chain.as_str() {
        "bitcoin" | "btc" => {
            let fees_api = cfg.crypto.btc_onchain_fees_api_url.trim();
            let v: Value = client
                .get(fees_api)
                .send()
                .map_err(|err| format!("fetch bitcoin onchain failed: {err}"))?
                .json()
                .map_err(|err| format!("parse bitcoin onchain failed: {err}"))?;
            let fastest = v.get("fastestFee").and_then(|x| x.as_u64()).unwrap_or(0);
            let half_hour = v.get("halfHourFee").and_then(|x| x.as_u64()).unwrap_or(0);
            let hour = v.get("hourFee").and_then(|x| x.as_u64()).unwrap_or(0);
            let text = tr_with(
                "crypto.msg.onchain_btc_fees",
                &[
                    ("fastest", &fastest.to_string()),
                    ("half_hour", &half_hour.to_string()),
                    ("hour", &hour.to_string()),
                ],
            );
            Ok((text, json!({"action":"onchain","chain":"bitcoin","fees":v})))
        }
        "ethereum" | "eth" => {
            if let Some(address) = obj
                .get("address")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return handle_eth_address_onchain(client, cfg, obj, address);
            }
            let stats_api = cfg.crypto.eth_onchain_stats_api_url.trim();
            let v: Value = client
                .get(stats_api)
                .send()
                .map_err(|err| format!("fetch ethereum onchain failed: {err}"))?
                .json()
                .map_err(|err| format!("parse ethereum onchain failed: {err}"))?;
            let data = v.get("data").cloned().unwrap_or(Value::Null);
            let tx_24h = data
                .get("transactions_24h")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let blocks_24h = data.get("blocks_24h").and_then(|x| x.as_u64()).unwrap_or(0);
            let market = data
                .get("market_price_usd")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0);
            let market_price_usd = format!("{market:.4}");
            let text = tr_with(
                "crypto.msg.onchain_eth_stats_summary",
                &[
                    ("tx_24h", &tx_24h.to_string()),
                    ("blocks_24h", &blocks_24h.to_string()),
                    ("market_price_usd", market_price_usd.as_str()),
                ],
            );
            Ok((text, json!({"action":"onchain","chain":"ethereum","stats":data})))
        }
        _ => Err(tr("crypto.err.unsupported_chain")),
    }
}

fn handle_eth_address_onchain(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
    address: &str,
) -> Result<(String, Value), String> {
    let token = obj
        .get("token")
        .and_then(|v| v.as_str())
        .unwrap_or("eth")
        .trim()
        .to_ascii_lowercase();
    let tx_limit = obj
        .get("tx_limit")
        .and_then(|v| v.as_u64())
        .or_else(|| obj.get("limit").and_then(|v| v.as_u64()))
        .unwrap_or(5)
        .clamp(1, 30) as usize;
    let tx_url = render_url_template(
        &cfg.crypto.eth_address_tx_list_api_url,
        &[("address", address), ("limit", &tx_limit.to_string())],
    );
    let tx_resp: Value = client
        .get(&tx_url)
        .send()
        .map_err(|err| format!("fetch ethereum tx list failed: {err}"))?
        .json()
        .map_err(|err| format!("parse ethereum tx list failed: {err}"))?;
    let tx_items = parse_evm_tx_list(&tx_resp, address, tx_limit);

    if matches!(token.as_str(), "eth" | "native") {
        let bal_url = render_url_template(
            &cfg.crypto.eth_address_native_balance_api_url,
            &[("address", address)],
        );
        let bal_resp: Value = client
            .get(&bal_url)
            .send()
            .map_err(|err| format!("fetch ethereum native balance failed: {err}"))?
            .json()
            .map_err(|err| format!("parse ethereum native balance failed: {err}"))?;
        let raw = parse_evm_api_result_string(&bal_resp)
            .ok_or_else(|| "ethereum native balance response missing result".to_string())?;
        let amount = raw_to_decimal_string(&raw, 18);
        let recent_txs = tx_items.len().to_string();
        let text = tr_with(
            "crypto.msg.onchain_eth_native_summary",
            &[
                ("address", address),
                ("balance", amount.as_str()),
                ("recent_txs", recent_txs.as_str()),
            ],
        );
        return Ok((
            text,
            json!({
                "action":"onchain",
                "chain":"ethereum",
                "address":address,
                "token":"ETH",
                "balance": {
                    "raw": raw,
                    "decimals": 18,
                    "formatted": amount
                },
                "recent_txs": tx_items
            }),
        ));
    }

    let contract = cfg
        .crypto
        .eth_token_contracts
        .get(&token)
        .or_else(|| cfg.crypto.eth_token_contracts.get(&token.to_ascii_uppercase()))
        .ok_or_else(|| format!("token contract not configured for ethereum token: {token}"))?;
    let decimals = cfg
        .crypto
        .eth_token_decimals
        .get(&token)
        .or_else(|| cfg.crypto.eth_token_decimals.get(&token.to_ascii_uppercase()))
        .copied()
        .unwrap_or(6);
    let bal_url = render_url_template(
        &cfg.crypto.eth_address_token_balance_api_url,
        &[("address", address), ("contract", contract)],
    );
    let bal_resp: Value = client
        .get(&bal_url)
        .send()
        .map_err(|err| format!("fetch ethereum token balance failed: {err}"))?
        .json()
        .map_err(|err| format!("parse ethereum token balance failed: {err}"))?;
    let raw = parse_evm_api_result_string(&bal_resp)
        .ok_or_else(|| "ethereum token balance response missing result".to_string())?;
    let amount = raw_to_decimal_string(&raw, decimals);
    let token_upper = token.to_ascii_uppercase();
    let recent_txs = tx_items.len().to_string();
    let text = tr_with(
        "crypto.msg.onchain_eth_token_summary",
        &[
            ("address", address),
            ("token", token_upper.as_str()),
            ("balance", amount.as_str()),
            ("recent_txs", recent_txs.as_str()),
        ],
    );
    Ok((
        text,
        json!({
            "action":"onchain",
            "chain":"ethereum",
            "address":address,
            "token":token.to_ascii_uppercase(),
            "contract":contract,
            "balance": {
                "raw": raw,
                "decimals": decimals,
                "formatted": amount
            },
            "recent_txs": tx_items
        }),
    ))
}

fn handle_trade_preview(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let trade = parse_trade_input(obj, cfg)?;
    let checks = risk_checks(client, cfg, &trade, false)?;
    let preview_qty = effective_order_qty_for_preview(client, cfg, &trade)?;
    let notional = estimate_notional_usd(client, cfg, &TradeInput {
        qty: preview_qty,
        ..trade.clone()
    })?;
    // When user specified a USDT spend amount, Binance uses quoteOrderQty and fills
    // the actual coin qty at market; the displayed qty is only an estimate.
    let (qty_label, quote_part) = if let Some(q) = trade.quote_qty_usd {
        ("est_qty", format!(" quote_usd={:.4}", q))
    } else {
        ("qty", String::new())
    };
    // Include order_type and price so the confirm-route can reconstruct the exact order.
    // For market orders we omit order_type (defaults to market in parse).
    let order_type_part = if trade.order_type != "market" {
        format!(" order_type={}", trade.order_type)
    } else {
        String::new()
    };
    let price_part = if let Some(p) = trade.price {
        format!(" price={}", fmt_num(p))
    } else {
        String::new()
    };
    let stop_price_part = if let Some(sp) = trade.stop_price {
        format!(" stop_price={}", fmt_num(sp))
    } else {
        String::new()
    };
    let tif_part = if let Some(tif) = &trade.time_in_force {
        if tif != "GTC" {
            format!(" tif={tif}")
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    let qty_part = format!(
        "{}={:.6}{}{}{}{}{}",
        qty_label, preview_qty, quote_part, order_type_part, price_part, stop_price_part, tif_part
    );
    let text = tr_with(
        "crypto.msg.trade_preview_summary",
        &[
            ("exchange", trade.exchange.as_str()),
            ("symbol", trade.symbol.as_str()),
            ("side", trade.side.as_str()),
            ("qty_part", qty_part.as_str()),
            ("notional", &format!("{:.4}", notional)),
            ("checks", &checks.len().to_string()),
        ],
    );
    Ok((
        text,
        json!({
            "action":"trade_preview",
            "order": trade_to_json(&trade),
            "effective_qty": preview_qty,
            "notional_usd": notional,
            "risk_checks": checks,
            "decision":"preview_only"
        }),
    ))
}

fn handle_trade_submit(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let trade = parse_trade_input(obj, cfg)?;
    let checks = risk_checks(client, cfg, &trade, true)?;
    let event = match trade.exchange.as_str() {
        "binance" => submit_binance_order(client, cfg, &trade)?,
        "okx" => submit_okx_order(client, cfg, &trade)?,
        other => return Err(tr_with("crypto.err.unsupported_execution_exchange", &[("exchange", other)])),
    };

    // Build a human-friendly result text based on order status
    let text = build_trade_submitted_text(&event);

    Ok((
        text,
        json!({
            "action":"trade_submit",
            "order": event,
            "risk_checks": checks,
            "decision":"submitted"
        }),
    ))
}

fn build_trade_submitted_text(event: &OrderEvent) -> String {
    let status_upper = event.status.to_ascii_uppercase();
    match status_upper.as_str() {
        "FILLED" => {
            let qty_filled = if let Some(eq) = event.executed_qty {
                format!("{}", fmt_num(eq))
            } else {
                format!("{}", fmt_num(event.qty))
            };
            let quote_spent = if let Some(qv) = event.executed_quote_qty {
                format!("{:.4}", qv)
            } else {
                format!("{:.4}", event.notional_usd)
            };
            let price_part = event.avg_fill_price
                .map(|p| format!(" avg_price={}", fmt_num(p)))
                .unwrap_or_default();
            tr_with(
                "crypto.msg.trade_submitted_filled",
                &[
                    ("order_id", event.order_id.as_str()),
                    ("exchange", event.exchange.as_str()),
                    ("symbol", event.symbol.as_str()),
                    ("side", event.side.as_str()),
                    ("qty_filled", qty_filled.as_str()),
                    ("price_part", price_part.as_str()),
                    ("quote_spent", quote_spent.as_str()),
                ],
            )
        }
        "NEW" | "LIVE" => {
            let price_str = event.price
                .map(|p| format!(" price={}", fmt_num(p)))
                .unwrap_or_default();
            let stop_str = event.reason
                .as_deref()
                .filter(|r| !r.is_empty())
                .map(|r| format!(" info={r}"))
                .unwrap_or_default();
            let pending_suffix = tr("crypto.msg.trade_submitted_pending_suffix");
            tr_with(
                "crypto.msg.trade_submitted_pending",
                &[
                    ("order_id", event.order_id.as_str()),
                    ("exchange", event.exchange.as_str()),
                    ("symbol", event.symbol.as_str()),
                    ("side", event.side.as_str()),
                    ("order_type", event.order_type.as_str()),
                    ("qty", &format!("{:.6}", event.qty)),
                    ("price_str", price_str.as_str()),
                    ("stop_str", stop_str.as_str()),
                    ("notional", &format!("{:.4}", event.notional_usd)),
                    ("pending_suffix", pending_suffix.as_str()),
                ],
            )
        }
        "PARTIALLY_FILLED" => {
            let filled_str = event.executed_qty
                .map(|q| format!("{:.6}", q))
                .unwrap_or_else(|| "?".to_string());
            let total_str = format!("{:.6}", event.qty);
            tr_with(
                "crypto.msg.trade_submitted_partial",
                &[
                    ("order_id", event.order_id.as_str()),
                    ("exchange", event.exchange.as_str()),
                    ("symbol", event.symbol.as_str()),
                    ("side", event.side.as_str()),
                    ("filled", filled_str.as_str()),
                    ("total", total_str.as_str()),
                    ("notional", &format!("{:.4}", event.notional_usd)),
                ],
            )
        }
        _ => tr_with(
            "crypto.msg.trade_submitted_fallback",
            &[
                ("order_id", event.order_id.as_str()),
                ("status", event.status.as_str()),
                ("exchange", event.exchange.as_str()),
                ("symbol", event.symbol.as_str()),
                ("side", event.side.as_str()),
                ("qty", &format!("{:.6}", event.qty)),
                ("notional", &format!("{:.4}", event.notional_usd)),
            ],
        )
    }
}

fn handle_order_status(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg);
    match exchange.as_str() {
        "binance" => handle_order_status_binance(client, cfg, obj),
        "okx" => handle_order_status_okx(client, cfg, obj),
        _ => Err(tr_with("crypto.err.unsupported_exchange_for_order_status", &[("exchange", &exchange)])),
    }
}

fn handle_cancel_order(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg);
    match exchange.as_str() {
        "binance" => handle_cancel_order_binance(client, cfg, obj),
        "okx" => handle_cancel_order_okx(client, cfg, obj),
        _ => Err(tr_with("crypto.err.unsupported_exchange_for_cancel_order", &[("exchange", &exchange)])),
    }
}

fn handle_positions(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg);
    match exchange.as_str() {
        "binance" => handle_positions_binance(client, cfg),
        "okx" => handle_positions_okx(client, cfg),
        _ => Err(tr_with("crypto.err.unsupported_exchange_for_positions", &[("exchange", &exchange)])),
    }
}

fn handle_open_orders(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg);
    match exchange.as_str() {
        "binance" => {
            ensure_binance_config(cfg)?;
            let symbol = obj
                .get("symbol")
                .and_then(|v| v.as_str())
                .map(normalize_symbol);
            let mut params = Vec::<(&str, String)>::new();
            if let Some(s) = &symbol {
                params.push(("symbol", s.clone()));
            }
            let v = binance_signed_request(client, cfg, Method::GET, "/api/v3/openOrders", &mut params)?;
            let arr = v.as_array().cloned().unwrap_or_default();
            let mut lines = Vec::new();
            for order in &arr {
                let sym = order.get("symbol").and_then(|x| x.as_str()).unwrap_or("");
                let oid = order.get("orderId").map(|x| x.to_string()).unwrap_or_default();
                let side = order.get("side").and_then(|x| x.as_str()).unwrap_or("");
                let otype = order.get("type").and_then(|x| x.as_str()).unwrap_or("");
                let price = order.get("price").and_then(|x| x.as_str()).unwrap_or("0");
                let orig_qty = order.get("origQty").and_then(|x| x.as_str()).unwrap_or("0");
                let exec_qty = order.get("executedQty").and_then(|x| x.as_str()).unwrap_or("0");
                let status = order.get("status").and_then(|x| x.as_str()).unwrap_or("");
                lines.push(tr_with(
                    "crypto.msg.open_orders_line_binance",
                    &[
                        ("sym", sym),
                        ("side", side),
                        ("otype", otype),
                        ("orig_qty", orig_qty),
                        ("exec_qty", exec_qty),
                        ("price", price),
                        ("status", status),
                        ("oid", &oid),
                    ],
                ));
            }
            let symbol_suffix = symbol.as_ref().map(|s| format!(" {s}")).unwrap_or_default();
            let summary = if lines.is_empty() {
                tr_with(
                    "crypto.msg.open_orders_none",
                    &[("exchange", "binance"), ("symbol_suffix", symbol_suffix.as_str())],
                )
            } else {
                tr_with(
                    "crypto.msg.open_orders_header",
                    &[
                        ("exchange", "binance"),
                        ("count", &arr.len().to_string()),
                        ("body", &lines.join("\n")),
                    ],
                )
            };
            Ok((summary, json!({"action":"open_orders","exchange":"binance","orders":arr})))
        }
        "okx" => {
            ensure_okx_config(cfg)?;
            let symbol = obj
                .get("symbol")
                .and_then(|v| v.as_str())
                .map(normalize_symbol);
            let mut q_parts = Vec::new();
            if let Some(s) = &symbol {
                q_parts.push(format!("instId={}", encode(&to_okx_inst_id(s))));
            }
            q_parts.push("instType=SPOT".to_string());
            let q = q_parts.join("&");
            let v = okx_request(client, cfg, Method::GET, "/api/v5/trade/orders-pending", Some(&q), None)?;
            let arr = v.get("data").and_then(|x| x.as_array()).cloned().unwrap_or_default();
            let mut lines = Vec::new();
            for order in &arr {
                let inst = order.get("instId").and_then(|x| x.as_str()).unwrap_or("");
                let oid = order.get("ordId").and_then(|x| x.as_str()).unwrap_or("");
                let side = order.get("side").and_then(|x| x.as_str()).unwrap_or("");
                let otype = order.get("ordType").and_then(|x| x.as_str()).unwrap_or("");
                let sz = order.get("sz").and_then(|x| x.as_str()).unwrap_or("0");
                let px = order.get("px").and_then(|x| x.as_str()).unwrap_or("0");
                let state = order.get("state").and_then(|x| x.as_str()).unwrap_or("");
                lines.push(tr_with(
                    "crypto.msg.open_orders_line_okx",
                    &[
                        ("inst", inst),
                        ("side", side),
                        ("otype", otype),
                        ("sz", sz),
                        ("px", px),
                        ("state", state),
                        ("oid", oid),
                    ],
                ));
            }
            let symbol_suffix = symbol.as_ref().map(|s| format!(" {s}")).unwrap_or_default();
            let summary = if lines.is_empty() {
                tr_with(
                    "crypto.msg.open_orders_none",
                    &[("exchange", "okx"), ("symbol_suffix", symbol_suffix.as_str())],
                )
            } else {
                tr_with(
                    "crypto.msg.open_orders_header",
                    &[
                        ("exchange", "okx"),
                        ("count", &arr.len().to_string()),
                        ("body", &lines.join("\n")),
                    ],
                )
            };
            Ok((summary, json!({"action":"open_orders","exchange":"okx","orders":arr})))
        }
        _ => Err(format!("open_orders: unsupported exchange={exchange}")),
    }
}

fn handle_cancel_all_orders(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg);
    match exchange.as_str() {
        "binance" => {
            ensure_binance_config(cfg)?;
            let symbol = obj
                .get("symbol")
                .and_then(|v| v.as_str())
                .ok_or("cancel_all_orders on binance requires symbol")?;
            let sym = normalize_symbol(symbol);
            let mut params = vec![("symbol", sym.clone())];
            let v = binance_signed_request(client, cfg, Method::DELETE, "/api/v3/openOrders", &mut params)?;
            let arr = v.as_array().cloned().unwrap_or_default();
            let text = tr_with(
                "crypto.msg.cancel_all_orders_done",
                &[("exchange", "binance"), ("sym", sym.as_str()), ("count", &arr.len().to_string())],
            );
            Ok((text, json!({"action":"cancel_all_orders","exchange":"binance","symbol":sym,"cancelled":arr})))
        }
        "okx" => {
            ensure_okx_config(cfg)?;
            let symbol = obj.get("symbol").and_then(|v| v.as_str()).map(normalize_symbol);
            // First fetch open orders, then cancel them in batch
            let mut q_parts = vec!["instType=SPOT".to_string()];
            if let Some(s) = &symbol {
                q_parts.push(format!("instId={}", encode(&to_okx_inst_id(s))));
            }
            let q = q_parts.join("&");
            let pending = okx_request(client, cfg, Method::GET, "/api/v5/trade/orders-pending", Some(&q), None)?;
            let arr = pending.get("data").and_then(|x| x.as_array()).cloned().unwrap_or_default();
            if arr.is_empty() {
                let sym_info = symbol.as_deref().unwrap_or("all");
                return Ok((
                    tr_with(
                        "crypto.msg.cancel_all_orders_no_open_orders",
                        &[("exchange", "okx"), ("sym_info", sym_info)],
                    ),
                    json!({"action":"cancel_all_orders","exchange":"okx","cancelled":[]}),
                ));
            }
            let cancel_list: Vec<Value> = arr
                .iter()
                .filter_map(|o| {
                    let inst = o.get("instId").and_then(|x| x.as_str())?;
                    let oid = o.get("ordId").and_then(|x| x.as_str())?;
                    Some(json!({"instId": inst, "ordId": oid}))
                })
                .collect();
            let body = Value::Array(cancel_list);
            let v = okx_request(client, cfg, Method::POST, "/api/v5/trade/cancel-batch-orders", None, Some(body))?;
            let cancelled = v.get("data").and_then(|x| x.as_array()).cloned().unwrap_or_default();
            let sym_info = symbol.as_deref().unwrap_or("all");
            let text = tr_with(
                "crypto.msg.cancel_all_orders_done",
                &[("exchange", "okx"), ("sym", sym_info), ("count", &cancelled.len().to_string())],
            );
            Ok((text, json!({"action":"cancel_all_orders","exchange":"okx","cancelled":cancelled})))
        }
        _ => Err(format!("cancel_all_orders: unsupported exchange={exchange}")),
    }
}

fn handle_trade_history(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg);
    let limit = obj
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20)
        .clamp(1, 500);
    match exchange.as_str() {
        "binance" => {
            ensure_binance_config(cfg)?;
            let symbol = obj
                .get("symbol")
                .and_then(|v| v.as_str())
                .ok_or("trade_history on binance requires symbol")?;
            let sym = normalize_symbol(symbol);
            let mut params = vec![
                ("symbol", sym.clone()),
                ("limit", limit.to_string()),
            ];
            let v = binance_signed_request(client, cfg, Method::GET, "/api/v3/myTrades", &mut params)?;
            let arr = v.as_array().cloned().unwrap_or_default();
            let mut lines = Vec::new();
            for trade in &arr {
                let side = if trade.get("isBuyer").and_then(|x| x.as_bool()).unwrap_or(false) { "buy" } else { "sell" };
                let price = trade.get("price").and_then(|x| x.as_str()).unwrap_or("0");
                let qty = trade.get("qty").and_then(|x| x.as_str()).unwrap_or("0");
                let quote_qty = trade.get("quoteQty").and_then(|x| x.as_str()).unwrap_or("0");
                let commission = trade.get("commission").and_then(|x| x.as_str()).unwrap_or("0");
                let comm_asset = trade.get("commissionAsset").and_then(|x| x.as_str()).unwrap_or("");
                let tid = trade.get("id").map(|x| x.to_string()).unwrap_or_default();
                lines.push(format!(
                    "{sym} {side} qty={qty} quoteQty={quote_qty} price={price} fee={commission}{comm_asset} id={tid}"
                ));
            }
            let summary = if lines.is_empty() {
                format!("trade_history binance {sym}: none")
            } else {
                format!("trade_history binance {sym} count={}\n{}", arr.len(), lines.join("\n"))
            };
            Ok((summary, json!({"action":"trade_history","exchange":"binance","symbol":sym,"trades":arr})))
        }
        "okx" => {
            ensure_okx_config(cfg)?;
            let symbol = obj.get("symbol").and_then(|v| v.as_str()).map(normalize_symbol);
            let mut q_parts = vec![
                "instType=SPOT".to_string(),
                format!("limit={limit}"),
            ];
            if let Some(s) = &symbol {
                q_parts.push(format!("instId={}", encode(&to_okx_inst_id(s))));
            }
            let q = q_parts.join("&");
            let v = okx_request(client, cfg, Method::GET, "/api/v5/trade/fills", Some(&q), None)?;
            let arr = v.get("data").and_then(|x| x.as_array()).cloned().unwrap_or_default();
            let mut lines = Vec::new();
            for fill in &arr {
                let inst = fill.get("instId").and_then(|x| x.as_str()).unwrap_or("");
                let side = fill.get("side").and_then(|x| x.as_str()).unwrap_or("");
                let px = fill.get("fillPx").and_then(|x| x.as_str()).unwrap_or("0");
                let sz = fill.get("fillSz").and_then(|x| x.as_str()).unwrap_or("0");
                let fee = fill.get("fee").and_then(|x| x.as_str()).unwrap_or("0");
                let fee_ccy = fill.get("feeCcy").and_then(|x| x.as_str()).unwrap_or("");
                let tid = fill.get("tradeId").and_then(|x| x.as_str()).unwrap_or("");
                lines.push(format!(
                    "{inst} {side} sz={sz} price={px} fee={fee}{fee_ccy} id={tid}"
                ));
            }
            let sym_info = symbol.as_deref().unwrap_or("all");
            let summary = if lines.is_empty() {
                format!("trade_history okx {sym_info}: none")
            } else {
                format!("trade_history okx {sym_info} count={}\n{}", arr.len(), lines.join("\n"))
            };
            Ok((summary, json!({"action":"trade_history","exchange":"okx","trades":arr})))
        }
        _ => Err(format!("trade_history: unsupported exchange={exchange}")),
    }
}

fn handle_order_status_binance(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let order_id = obj.get("order_id").and_then(|v| v.as_str());
    let client_order_id = obj.get("client_order_id").and_then(|v| v.as_str());
    if order_id.is_none() && client_order_id.is_none() {
        return Err(tr("crypto.err.order_or_client_order_id_required"));
    }
    let Some(symbol) = obj.get("symbol").and_then(|v| v.as_str()) else {
        let id_text = order_id.or(client_order_id).unwrap_or("unknown");
        let text = tr_with(
            "crypto.msg.order_status_skipped_missing_symbol",
            &[("id_text", id_text), ("exchange", "binance")],
        );
        return Ok((
            text,
            json!({
                "action":"order_status",
                "exchange":"binance",
                "skipped": true,
                "reason": "symbol_required",
                "order_id": order_id,
                "client_order_id": client_order_id
            }),
        ));
    };
    let mut params = vec![("symbol", normalize_symbol(symbol))];
    if let Some(v) = order_id {
        params.push(("orderId", v.to_string()));
    }
    if let Some(v) = client_order_id {
        params.push(("origClientOrderId", v.to_string()));
    }
    let v = binance_signed_request(client, cfg, Method::GET, "/api/v3/order", &mut params)?;
    let status = v.get("status").and_then(|x| x.as_str()).unwrap_or("UNKNOWN");
    let id_text = order_id.or(client_order_id).unwrap_or("unknown");
    let sym = normalize_symbol(symbol);
    let text = tr_with(
        "crypto.msg.order_status_summary",
        &[("symbol", sym.as_str()), ("id_text", id_text), ("status", status)],
    );
    Ok((text, json!({"action":"order_status","exchange":"binance","order":v})))
}

fn handle_cancel_order_binance(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let symbol = obj
        .get("symbol")
        .and_then(|v| v.as_str())
        .ok_or_else(|| tr("crypto.err.symbol_required_for_binance_cancel_order"))?;
    let order_id = obj.get("order_id").and_then(|v| v.as_str());
    let client_order_id = obj.get("client_order_id").and_then(|v| v.as_str());
    if order_id.is_none() && client_order_id.is_none() {
        return Err(tr("crypto.err.order_or_client_order_id_required"));
    }
    let mut params = vec![("symbol", normalize_symbol(symbol))];
    if let Some(v) = order_id {
        params.push(("orderId", v.to_string()));
    }
    if let Some(v) = client_order_id {
        params.push(("origClientOrderId", v.to_string()));
    }
    let v = binance_signed_request(client, cfg, Method::DELETE, "/api/v3/order", &mut params)?;
    let id_text = order_id.or(client_order_id).unwrap_or("unknown");
    let text = tr_with("crypto.msg.cancel_order_done", &[("id_text", id_text)]);
    Ok((text, json!({"action":"cancel_order","exchange":"binance","order":v})))
}

fn handle_positions_binance(client: &Client, cfg: &RootConfig) -> Result<(String, Value), String> {
    let mut params = Vec::<(&str, String)>::new();
    let v = binance_signed_request(client, cfg, Method::GET, "/api/v3/account", &mut params)?;
    let mut items = Vec::new();
    let mut lines = Vec::new();
    if let Some(arr) = v.get("balances").and_then(|x| x.as_array()) {
        for bal in arr {
            let asset = bal.get("asset").and_then(|x| x.as_str()).unwrap_or("");
            let free = bal
                .get("free")
                .and_then(|x| x.as_str())
                .and_then(|x| x.parse::<f64>().ok())
                .unwrap_or(0.0);
            let locked = bal
                .get("locked")
                .and_then(|x| x.as_str())
                .and_then(|x| x.parse::<f64>().ok())
                .unwrap_or(0.0);
            if free + locked <= 0.0 {
                continue;
            }
            let free_s = fmt_num(free);
            let locked_s = fmt_num(locked);
            lines.push(tr_with(
                "crypto.msg.positions_balance_line",
                &[("asset", asset), ("free", free_s.as_str()), ("locked", locked_s.as_str())],
            ));
            items.push(json!({"asset":asset,"free":free,"locked":locked}));
        }
    }
    if lines.is_empty() {
        lines.push(tr("crypto.msg.no_balances"));
    }
    Ok((
        lines.join("\n"),
        json!({"action":"positions","exchange":"binance","positions":items}),
    ))
}

fn handle_order_status_okx(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let order_id = obj.get("order_id").and_then(|v| v.as_str());
    let client_order_id = obj.get("client_order_id").and_then(|v| v.as_str());
    if order_id.is_none() && client_order_id.is_none() {
        return Err(tr("crypto.err.order_or_client_order_id_required"));
    }
    let Some(symbol) = obj.get("symbol").and_then(|v| v.as_str()) else {
        let id_text = order_id.or(client_order_id).unwrap_or("unknown");
        let text = tr_with(
            "crypto.msg.order_status_skipped_missing_symbol",
            &[("id_text", id_text), ("exchange", "okx")],
        );
        return Ok((
            text,
            json!({
                "action":"order_status",
                "exchange":"okx",
                "skipped": true,
                "reason": "symbol_required",
                "order_id": order_id,
                "client_order_id": client_order_id
            }),
        ));
    };
    let mut q_parts = vec![format!("instId={}", encode(&to_okx_inst_id(symbol)))];
    if let Some(v) = order_id {
        q_parts.push(format!("ordId={}", encode(v)));
    }
    if let Some(v) = client_order_id {
        q_parts.push(format!("clOrdId={}", encode(v)));
    }
    let q = q_parts.join("&");
    let v = okx_request(client, cfg, Method::GET, "/api/v5/trade/order", Some(&q), None)?;
    let data = v
        .get("data")
        .and_then(|x| x.as_array())
        .and_then(|x| x.first())
        .cloned()
        .unwrap_or(Value::Null);
    let state = data.get("state").and_then(|x| x.as_str()).unwrap_or("unknown");
    let id_text = order_id.or(client_order_id).unwrap_or("unknown");
    let sym = normalize_symbol(symbol);
    let text = tr_with(
        "crypto.msg.order_status_summary",
        &[("symbol", sym.as_str()), ("id_text", id_text), ("status", state)],
    );
    Ok((text, json!({"action":"order_status","exchange":"okx","order":data})))
}

fn handle_cancel_order_okx(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let symbol = obj
        .get("symbol")
        .and_then(|v| v.as_str())
        .ok_or_else(|| tr("crypto.err.symbol_required_for_okx_cancel_order"))?;
    let order_id = obj.get("order_id").and_then(|v| v.as_str());
    let client_order_id = obj.get("client_order_id").and_then(|v| v.as_str());
    if order_id.is_none() && client_order_id.is_none() {
        return Err(tr("crypto.err.order_or_client_order_id_required"));
    }
    let mut body = json!({"instId": to_okx_inst_id(symbol)});
    if let Some(v) = order_id {
        body["ordId"] = Value::String(v.to_string());
    }
    if let Some(v) = client_order_id {
        body["clOrdId"] = Value::String(v.to_string());
    }
    let v = okx_request(
        client,
        cfg,
        Method::POST,
        "/api/v5/trade/cancel-order",
        None,
        Some(body),
    )?;
    let id_text = order_id.or(client_order_id).unwrap_or("unknown");
    let text = tr_with("crypto.msg.cancel_order_done", &[("id_text", id_text)]);
    Ok((text, json!({"action":"cancel_order","exchange":"okx","order":v})))
}

fn handle_positions_okx(client: &Client, cfg: &RootConfig) -> Result<(String, Value), String> {
    let v = okx_request(client, cfg, Method::GET, "/api/v5/account/balance", None, None)?;
    let mut lines = Vec::new();
    let mut items = Vec::new();
    if let Some(details) = v
        .get("data")
        .and_then(|x| x.as_array())
        .and_then(|x| x.first())
        .and_then(|x| x.get("details"))
        .and_then(|x| x.as_array())
    {
        for it in details {
            let ccy = it.get("ccy").and_then(|x| x.as_str()).unwrap_or("");
            let eq = it
                .get("eq")
                .and_then(|x| x.as_str())
                .and_then(|x| x.parse::<f64>().ok())
                .unwrap_or(0.0);
            let avail = it
                .get("availBal")
                .and_then(|x| x.as_str())
                .and_then(|x| x.parse::<f64>().ok())
                .unwrap_or(0.0);
            if eq <= 0.0 {
                continue;
            }
            let eq_s = fmt_num(eq);
            let avail_s = fmt_num(avail);
            lines.push(tr_with(
                "crypto.msg.positions_balance_line_okx",
                &[("ccy", ccy), ("eq", eq_s.as_str()), ("avail", avail_s.as_str())],
            ));
            items.push(json!({"ccy":ccy,"eq":eq,"avail":avail}));
        }
    }
    if lines.is_empty() {
        lines.push(tr("crypto.msg.no_balances"));
    }
    Ok((
        lines.join("\n"),
        json!({"action":"positions","exchange":"okx","positions":items}),
    ))
}

fn parse_trade_input(obj: &serde_json::Map<String, Value>, cfg: &RootConfig) -> Result<TradeInput, String> {
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg);
    let symbol = normalize_symbol(
        obj.get("symbol")
            .and_then(|v| v.as_str())
            .ok_or_else(|| tr("crypto.err.symbol_required"))?,
    );
    let side = obj
        .get("side")
        .and_then(|v| v.as_str())
        .unwrap_or("buy")
        .trim()
        .to_ascii_lowercase();
    if !matches!(side.as_str(), "buy" | "sell") {
        return Err(tr("crypto.err.side_invalid"));
    }
    let order_type = obj
        .get("order_type")
        .and_then(|v| v.as_str())
        .unwrap_or("market")
        .trim()
        .to_ascii_lowercase();
    if !matches!(
        order_type.as_str(),
        "market" | "limit" | "stop_loss_limit" | "take_profit_limit" | "limit_maker"
    ) {
        return Err(tr("crypto.err.order_type_invalid"));
    }
    let quote_qty_usd = obj
        .get("quote_qty_usd")
        .and_then(value_to_f64)
        .or_else(|| obj.get("quote_qty").and_then(value_to_f64))
        .or_else(|| obj.get("amount_usd").and_then(value_to_f64))
        .or_else(|| obj.get("notional_usd").and_then(value_to_f64));
    let qty_all = obj
        .get("qty")
        .and_then(|v| v.as_str())
        .map(|s| {
            let n = s.trim().to_ascii_lowercase();
            matches!(n.as_str(), "all" | "max" | "全部" | "全仓")
        })
        .unwrap_or(false);
    let mut qty = obj.get("qty").and_then(value_to_f64).unwrap_or(0.0);
    if let Some(v) = quote_qty_usd {
        if v <= 0.0 {
            return Err(tr("crypto.err.qty_must_gt_zero"));
        }
        qty = 0.0;
    } else if qty_all {
        if side != "sell" {
            return Err("qty=all is only supported for sell side".to_string());
        }
        qty = 0.0;
    } else if qty <= 0.0 {
        return Err(tr("crypto.err.qty_required_number"));
    }
    let price = obj.get("price").and_then(|v| v.as_f64());
    let stop_price = obj
        .get("stop_price")
        .and_then(value_to_f64)
        .or_else(|| obj.get("stopPrice").and_then(value_to_f64));
    let time_in_force = obj
        .get("time_in_force")
        .and_then(|v| v.as_str())
        .map(|s| s.to_ascii_uppercase());
    if matches!(order_type.as_str(), "limit" | "limit_maker") && price.unwrap_or(0.0) <= 0.0 {
        return Err(tr("crypto.err.price_required_for_limit"));
    }
    if matches!(order_type.as_str(), "stop_loss_limit" | "take_profit_limit") {
        if price.unwrap_or(0.0) <= 0.0 {
            return Err("stop_loss_limit/take_profit_limit requires price (limit price)".to_string());
        }
        if stop_price.unwrap_or(0.0) <= 0.0 {
            return Err("stop_loss_limit/take_profit_limit requires stop_price (trigger price)".to_string());
        }
    }
    Ok(TradeInput {
        exchange,
        symbol,
        side,
        order_type,
        qty,
        qty_all,
        quote_qty_usd,
        price,
        stop_price,
        time_in_force,
        client_order_id: obj
            .get("client_order_id")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        confirm: obj.get("confirm").and_then(|v| v.as_bool()).unwrap_or(false),
    })
}

fn risk_checks(client: &Client, cfg: &RootConfig, trade: &TradeInput, _for_submit: bool) -> Result<Vec<Value>, String> {
    let mut checks = Vec::new();
    // Whether to require user confirmation is left to the planner/LLM; no runtime guard.
    match trade.exchange.as_str() {
        "binance" => {
            ensure_binance_config(cfg)?;
            checks.push(json!({"check":"exchange_api_config","ok":true,"exchange":"binance"}));
        }
        "okx" => {
            ensure_okx_config(cfg)?;
            checks.push(json!({"check":"exchange_api_config","ok":true,"exchange":"okx"}));
        }
        _ => {}
    }
    if !cfg.crypto.allowed_exchanges.is_empty()
        && !cfg
            .crypto
            .allowed_exchanges
            .iter()
            .any(|x| x.eq_ignore_ascii_case(&trade.exchange))
    {
        return Err(tr_with(
            "crypto.err.exchange_not_allowed",
            &[("exchange", &trade.exchange)],
        ));
    }
    checks.push(json!({"check":"allowed_exchanges","ok":true}));
    if !cfg.crypto.allowed_symbols.is_empty()
        && !cfg
            .crypto
            .allowed_symbols
            .iter()
            .any(|x| normalize_symbol(x) == trade.symbol)
    {
        return Err(tr_with(
            "crypto.err.symbol_not_allowed",
            &[("symbol", &trade.symbol)],
        ));
    }
    checks.push(json!({"check":"allowed_symbols","ok":true}));
    let notional_input = if trade.qty_all {
        let resolved_qty = resolve_base_qty(client, cfg, trade)?;
        TradeInput {
            qty: resolved_qty,
            qty_all: false,
            ..trade.clone()
        }
    } else {
        trade.clone()
    };
    let notional = estimate_notional_usd(client, cfg, &notional_input)?;
    let max_notional = cfg.crypto.max_notional_usd.unwrap_or(0.0);
    if max_notional > 0.0 && notional > max_notional {
        return Err(format!(
            "notional exceeds max_notional_usd: {notional:.4} > {max_notional:.4}"
        ));
    }
    checks.push(json!({"check":"max_notional_usd","ok":true,"actual":notional,"limit":max_notional}));
    // Binance spot minimum notional is typically 5~10 USDT; warn if below 1 USDT
    let min_notional = cfg.crypto.min_notional_usd.unwrap_or(1.0);
    if trade.exchange == "binance" && notional < min_notional && notional > 0.0 {
        return Err(format!(
            "notional too small: {notional:.4} < min_notional_usd={min_notional:.2} (Binance spot requires at least ~10 USDT)"
        ));
    }
    if min_notional > 0.0 && notional > 0.0 && notional < min_notional {
        checks.push(json!({"check":"min_notional_usd","ok":false,"actual":notional,"limit":min_notional}));
    } else {
        checks.push(json!({"check":"min_notional_usd","ok":true,"actual":notional,"limit":min_notional}));
    }
    Ok(checks)
}

fn estimate_notional_usd(client: &Client, cfg: &RootConfig, trade: &TradeInput) -> Result<f64, String> {
    if let Some(v) = trade.quote_qty_usd {
        return Ok(v.max(0.0));
    }
    let price = if let Some(p) = trade.price {
        p
    } else {
        fetch_quote(client, cfg, &trade.symbol, &trade.exchange)?.price_usd
    };
    Ok((trade.qty * price).max(0.0))
}

fn resolve_base_qty(client: &Client, cfg: &RootConfig, trade: &TradeInput) -> Result<f64, String> {
    if trade.qty_all {
        return resolve_all_sell_qty(client, cfg, trade);
    }
    if trade.qty > 0.0 {
        return Ok(trade.qty);
    }
    let quote = trade
        .quote_qty_usd
        .ok_or_else(|| tr("crypto.err.qty_required_number"))?;
    let price = if let Some(p) = trade.price {
        p
    } else {
        fetch_quote(client, cfg, &trade.symbol, &trade.exchange)?.price_usd
    };
    if price <= 0.0 {
        return Err("invalid price for quote_qty_usd conversion".to_string());
    }
    Ok((quote / price).max(0.0))
}

fn resolve_all_sell_qty(client: &Client, cfg: &RootConfig, trade: &TradeInput) -> Result<f64, String> {
    if trade.side != "sell" {
        return Err("qty=all requires sell side".to_string());
    }
    let (base_asset, _) = split_symbol_base_quote(&trade.symbol);
    if base_asset.is_empty() {
        return Err("cannot resolve base asset for qty=all".to_string());
    }
    match trade.exchange.as_str() {
        "binance" => {
            let mut params = Vec::<(&str, String)>::new();
            let v = binance_signed_request(client, cfg, Method::GET, "/api/v3/account", &mut params)?;
            let free = v
                .get("balances")
                .and_then(|x| x.as_array())
                .and_then(|arr| {
                    arr.iter().find_map(|bal| {
                        let asset = bal.get("asset").and_then(|x| x.as_str()).unwrap_or("");
                        if asset.eq_ignore_ascii_case(&base_asset) {
                            bal.get("free")
                                .and_then(|x| x.as_str())
                                .and_then(|x| x.parse::<f64>().ok())
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or(0.0);
            if free <= 0.0 {
                return Err(format!("no available balance for {} on binance", base_asset));
            }
            Ok(free)
        }
        "okx" => {
            let v = okx_request(client, cfg, Method::GET, "/api/v5/account/balance", None, None)?;
            let avail = v
                .get("data")
                .and_then(|x| x.as_array())
                .and_then(|x| x.first())
                .and_then(|x| x.get("details"))
                .and_then(|x| x.as_array())
                .and_then(|arr| {
                    arr.iter().find_map(|it| {
                        let ccy = it.get("ccy").and_then(|x| x.as_str()).unwrap_or("");
                        if ccy.eq_ignore_ascii_case(&base_asset) {
                            it.get("availBal")
                                .and_then(|x| x.as_str())
                                .and_then(|x| x.parse::<f64>().ok())
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or(0.0);
            if avail <= 0.0 {
                return Err(format!("no available balance for {} on okx", base_asset));
            }
            Ok(avail)
        }
        _ => Err(format!("qty=all is unsupported exchange: {}", trade.exchange)),
    }
}

fn adjust_qty_to_step_floor(qty: f64, step: f64) -> f64 {
    if qty <= 0.0 || step <= 0.0 {
        return qty.max(0.0);
    }
    let units = (qty / step).floor();
    (units * step).max(0.0)
}

fn fetch_binance_lot_size_filter(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
) -> Result<(f64, f64, f64), String> {
    let base = cfg.binance.base_url.trim_end_matches('/');
    let normalized_symbol = normalize_symbol(symbol);
    let url = format!("{base}/api/v3/exchangeInfo?symbol={}", encode(&normalized_symbol));
    let v: Value = client
        .get(url)
        .send()
        .map_err(|err| format!("binance exchangeInfo request failed: {err}"))?
        .json()
        .map_err(|err| format!("binance exchangeInfo parse failed: {err}"))?;
    if let Some(code) = v.get("code").and_then(|x| x.as_i64()) {
        if code != 0 {
            let msg = v.get("msg").and_then(|x| x.as_str()).unwrap_or("unknown");
            return Err(format!("binance exchangeInfo api error code={code}: {msg}"));
        }
    }
    let symbol_obj = v
        .get("symbols")
        .and_then(|x| x.as_array())
        .and_then(|arr| arr.first())
        .ok_or_else(|| "binance exchangeInfo missing symbols".to_string())?;
    let lot_filter = symbol_obj
        .get("filters")
        .and_then(|x| x.as_array())
        .and_then(|arr| {
            arr.iter().find(|f| {
                f.get("filterType")
                    .and_then(|x| x.as_str())
                    .map(|s| s.eq_ignore_ascii_case("LOT_SIZE"))
                    .unwrap_or(false)
            })
        })
        .ok_or_else(|| "binance exchangeInfo missing LOT_SIZE filter".to_string())?;
    let min_qty = lot_filter
        .get("minQty")
        .and_then(value_to_f64)
        .ok_or_else(|| "binance LOT_SIZE minQty missing".to_string())?;
    let max_qty = lot_filter
        .get("maxQty")
        .and_then(value_to_f64)
        .ok_or_else(|| "binance LOT_SIZE maxQty missing".to_string())?;
    let step_size = lot_filter
        .get("stepSize")
        .and_then(value_to_f64)
        .ok_or_else(|| "binance LOT_SIZE stepSize missing".to_string())?;
    Ok((min_qty, max_qty, step_size))
}

/// Fetch PRICE_FILTER (tickSize, minPrice, maxPrice) for a symbol from Binance exchangeInfo.
fn fetch_binance_price_filter(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
) -> Result<(f64, f64, f64), String> {
    let base = cfg.binance.base_url.trim_end_matches('/');
    let normalized_symbol = normalize_symbol(symbol);
    let url = format!("{base}/api/v3/exchangeInfo?symbol={}", encode(&normalized_symbol));
    let v: Value = client
        .get(url)
        .send()
        .map_err(|err| format!("binance exchangeInfo request failed: {err}"))?
        .json()
        .map_err(|err| format!("binance exchangeInfo parse failed: {err}"))?;
    if let Some(code) = v.get("code").and_then(|x| x.as_i64()) {
        if code != 0 {
            let msg = v.get("msg").and_then(|x| x.as_str()).unwrap_or("unknown");
            return Err(format!("binance exchangeInfo api error code={code}: {msg}"));
        }
    }
    let symbol_obj = v
        .get("symbols")
        .and_then(|x| x.as_array())
        .and_then(|arr| arr.first())
        .ok_or_else(|| "binance exchangeInfo missing symbols".to_string())?;
    let price_filter = symbol_obj
        .get("filters")
        .and_then(|x| x.as_array())
        .and_then(|arr| {
            arr.iter().find(|f| {
                f.get("filterType")
                    .and_then(|x| x.as_str())
                    .map(|s| s.eq_ignore_ascii_case("PRICE_FILTER"))
                    .unwrap_or(false)
            })
        })
        .ok_or_else(|| "binance exchangeInfo missing PRICE_FILTER".to_string())?;
    let tick_size = price_filter
        .get("tickSize")
        .and_then(value_to_f64)
        .ok_or_else(|| "binance PRICE_FILTER tickSize missing".to_string())?;
    let min_price = price_filter.get("minPrice").and_then(value_to_f64).unwrap_or(0.0);
    let max_price = price_filter.get("maxPrice").and_then(value_to_f64).unwrap_or(0.0);
    Ok((tick_size, min_price, max_price))
}

/// Round price to Binance tickSize (price must satisfy price % tickSize == 0).
fn adjust_price_to_tick(price: f64, tick_size: f64) -> f64 {
    if tick_size <= 0.0 {
        return price;
    }
    (price / tick_size).round() * tick_size
}

/// Format price for Binance API with decimal places matching tickSize (avoids PRICE_FILTER/PERCENT_PRICE format issues).
fn fmt_price_for_binance(price: f64, tick_size: f64) -> String {
    if tick_size <= 0.0 {
        return fmt_num(price);
    }
    let tick_str = format!("{:.10}", tick_size).trim_end_matches('0').to_string();
    let decimals = tick_str
        .find('.')
        .map(|i| (tick_str.len().saturating_sub(i).saturating_sub(1)).min(8))
        .unwrap_or(0);
    format!("{:.prec$}", price, prec = decimals)
}

fn normalize_binance_order_qty(client: &Client, cfg: &RootConfig, symbol: &str, raw_qty: f64) -> Result<f64, String> {
    let (min_qty, max_qty, step_size) = fetch_binance_lot_size_filter(client, cfg, symbol)?;
    let adjusted = adjust_qty_to_step_floor(raw_qty, step_size);
    if adjusted <= 0.0 {
        return Err(format!(
            "binance LOT_SIZE invalid adjusted quantity: raw={} adjusted={} stepSize={}",
            raw_qty, adjusted, step_size
        ));
    }
    if adjusted + 1e-12 < min_qty {
        return Err(format!(
            "binance LOT_SIZE quantity below minQty: raw={} adjusted={} minQty={} stepSize={}",
            raw_qty, adjusted, min_qty, step_size
        ));
    }
    if max_qty > 0.0 && adjusted - 1e-12 > max_qty {
        return Err(format!(
            "binance LOT_SIZE quantity above maxQty: raw={} adjusted={} maxQty={} stepSize={}",
            raw_qty, adjusted, max_qty, step_size
        ));
    }
    Ok(adjusted)
}

fn effective_order_qty_for_preview(client: &Client, cfg: &RootConfig, trade: &TradeInput) -> Result<f64, String> {
    let base_qty = resolve_base_qty(client, cfg, trade)?;
    if trade.exchange == "binance" {
        let use_quote_order_qty = trade.order_type == "market" && trade.quote_qty_usd.is_some();
        if !use_quote_order_qty {
            return normalize_binance_order_qty(client, cfg, &trade.symbol, base_qty);
        }
    }
    Ok(base_qty)
}

fn submit_binance_order(client: &Client, cfg: &RootConfig, trade: &TradeInput) -> Result<OrderEvent, String> {
    ensure_binance_config(cfg)?;
    // Map internal order_type names to Binance API type strings
    let binance_type = match trade.order_type.as_str() {
        "market" => "MARKET",
        "limit" => "LIMIT",
        "stop_loss_limit" => "STOP_LOSS_LIMIT",
        "take_profit_limit" => "TAKE_PROFIT_LIMIT",
        "limit_maker" => "LIMIT_MAKER",
        other => other,
    };
    let mut params = vec![
        ("symbol", trade.symbol.clone()),
        ("side", trade.side.to_ascii_uppercase()),
        ("type", binance_type.to_string()),
        ("newOrderRespType", "RESULT".to_string()),
    ];
    let base_qty = resolve_base_qty(client, cfg, trade)?;
    // Binance supports quoteOrderQty for MARKET orders on both BUY and SELL sides:
    // BUY:  spend exactly quote_qty_usd worth of quote asset
    // SELL: sell enough base to receive exactly quote_qty_usd of quote asset
    let use_quote_order_qty = trade.order_type == "market" && trade.quote_qty_usd.is_some();
    let final_qty = if use_quote_order_qty {
        base_qty
    } else {
        normalize_binance_order_qty(client, cfg, &trade.symbol, base_qty)?
    };
    if use_quote_order_qty {
        if let Some(quote_qty) = trade.quote_qty_usd {
            params.push(("quoteOrderQty", fmt_num(quote_qty)));
        } else {
            params.push(("quantity", fmt_num(final_qty)));
        }
    } else {
        params.push(("quantity", fmt_num(final_qty)));
    }
    let price_filter_opt = (matches!(
        trade.order_type.as_str(),
        "limit" | "stop_loss_limit" | "take_profit_limit" | "limit_maker"
    ) || trade.stop_price.is_some())
    .then(|| fetch_binance_price_filter(client, cfg, &trade.symbol));
    if matches!(trade.order_type.as_str(), "limit" | "stop_loss_limit" | "take_profit_limit") {
        let tif = trade
            .time_in_force
            .as_deref()
            .filter(|s| matches!(*s, "GTC" | "IOC" | "FOK"))
            .unwrap_or("GTC");
        params.push(("timeInForce", tif.to_string()));
        let limit_price = trade
            .price
            .ok_or_else(|| tr("crypto.err.price_required_for_limit"))?;
        let price_str = match &price_filter_opt {
            Some(Ok((tick, _, _))) => fmt_price_for_binance(adjust_price_to_tick(limit_price, *tick), *tick),
            _ => fmt_num(limit_price),
        };
        params.push(("price", price_str));
    }
    if trade.order_type == "limit_maker" {
        let limit_price = trade
            .price
            .ok_or_else(|| tr("crypto.err.price_required_for_limit"))?;
        let price_str = match &price_filter_opt {
            Some(Ok((tick, _, _))) => fmt_price_for_binance(adjust_price_to_tick(limit_price, *tick), *tick),
            _ => fmt_num(limit_price),
        };
        params.push(("price", price_str));
    }
    if let Some(sp) = trade.stop_price {
        let stop_str = match &price_filter_opt {
            Some(Ok((tick, _, _))) => fmt_price_for_binance(adjust_price_to_tick(sp, *tick), *tick),
            _ => fmt_num(sp),
        };
        params.push(("stopPrice", stop_str));
    }
    if let Some(cid) = &trade.client_order_id {
        params.push(("newClientOrderId", cid.clone()));
    }
    let v = binance_signed_request(client, cfg, Method::POST, "/api/v3/order", &mut params)?;
    let order_id = v
        .get("orderId")
        .and_then(|x| x.as_i64())
        .map(|x| x.to_string())
        .or_else(|| v.get("orderId").and_then(|x| x.as_str()).map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string());
    let status = v
        .get("status")
        .and_then(|x| x.as_str())
        .unwrap_or("NEW")
        .to_string();
    // Extract actual fill amounts from RESULT-type response
    let executed_qty = v
        .get("executedQty")
        .and_then(|x| x.as_str())
        .and_then(|x| x.parse::<f64>().ok())
        .filter(|&q| q > 0.0);
    let executed_quote_qty = v
        .get("cummulativeQuoteQty")
        .and_then(|x| x.as_str())
        .and_then(|x| x.parse::<f64>().ok())
        .filter(|&q| q > 0.0);
    let avg_fill_price = match (executed_qty, executed_quote_qty) {
        (Some(base), Some(quote)) if base > 0.0 => Some(quote / base),
        _ => None,
    };
    let notional = executed_quote_qty
        .unwrap_or_else(|| estimate_notional_usd(client, cfg, trade).unwrap_or(0.0));
    Ok(OrderEvent {
        event: "submit".to_string(),
        order_id,
        ts: now_ts(),
        exchange: "binance".to_string(),
        symbol: trade.symbol.clone(),
        side: trade.side.clone(),
        order_type: trade.order_type.clone(),
        qty: final_qty,
        price: trade.price,
        notional_usd: notional,
        status,
        client_order_id: trade.client_order_id.clone(),
        reason: None,
        executed_qty,
        executed_quote_qty,
        avg_fill_price,
    })
}

fn submit_okx_order(client: &Client, cfg: &RootConfig, trade: &TradeInput) -> Result<OrderEvent, String> {
    ensure_okx_config(cfg)?;
    let base_qty = resolve_base_qty(client, cfg, trade)?;
    let mut body = json!({
        "instId": to_okx_inst_id(&trade.symbol),
        "tdMode": "cash",
        "side": trade.side,
        "ordType": trade.order_type,
        "sz": fmt_num(base_qty)
    });
    if trade.order_type == "limit" {
        let limit_price = trade
            .price
            .ok_or_else(|| tr("crypto.err.price_required_for_limit"))?;
        body["px"] = Value::String(fmt_num(limit_price));
    } else if trade.order_type == "market" {
        // For OKX market orders:
        // - BUY with quote_qty_usd: use quote_ccy so sz represents quote asset (e.g. USDT amount)
        // - BUY with base qty or SELL: use base_ccy so sz represents base asset
        if trade.side == "buy" && trade.quote_qty_usd.is_some() {
            body["sz"] = Value::String(fmt_num(trade.quote_qty_usd.unwrap_or(base_qty)));
            body["tgtCcy"] = Value::String("quote_ccy".to_string());
        } else {
            body["tgtCcy"] = Value::String("base_ccy".to_string());
        }
    }
    if let Some(cid) = &trade.client_order_id {
        body["clOrdId"] = Value::String(cid.clone());
    }
    let v = okx_request(client, cfg, Method::POST, "/api/v5/trade/order", None, Some(body))?;
    let data = v
        .get("data")
        .and_then(|x| x.as_array())
        .and_then(|x| x.first())
        .cloned()
        .unwrap_or(Value::Null);
    let order_id = data
        .get("ordId")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown")
        .to_string();
    let status_code = data.get("sCode").and_then(|x| x.as_str()).unwrap_or("0");
    if status_code != "0" {
        return Err(format!(
            "okx order rejected sCode={} sMsg={}",
            status_code,
            data.get("sMsg").and_then(|x| x.as_str()).unwrap_or("unknown")
        ));
    }
    // OKX POST /api/v5/trade/order only returns ordId+sCode; fill details require separate query
    let status = "live".to_string();
    let notional = estimate_notional_usd(client, cfg, trade)?;
    Ok(OrderEvent {
        event: "submit".to_string(),
        order_id,
        ts: now_ts(),
        exchange: "okx".to_string(),
        symbol: trade.symbol.clone(),
        side: trade.side.clone(),
        order_type: trade.order_type.clone(),
        qty: base_qty,
        price: trade.price,
        notional_usd: notional,
        status,
        client_order_id: trade.client_order_id.clone(),
        reason: data
            .get("sMsg")
            .and_then(|x| x.as_str())
            .filter(|v| !v.is_empty())
            .map(str::to_string),
        executed_qty: None,
        executed_quote_qty: None,
        avg_fill_price: None,
    })
}

fn fetch_quote(client: &Client, cfg: &RootConfig, symbol_input: &str, exchange_input: &str) -> Result<Quote, String> {
    let exchange = exchange_input.trim().to_ascii_lowercase();
    let symbol = normalize_symbol(symbol_input);
    match exchange.as_str() {
        "coingecko" => fetch_quote_from_coingecko(client, cfg, &symbol),
        "okx" => fetch_quote_from_okx(client, cfg, &symbol),
        "binance" => fetch_quote_from_binance(client, cfg, &symbol),
        _ => fetch_quote_from_binance(client, cfg, &symbol)
            .or_else(|_| fetch_quote_from_okx(client, cfg, &symbol))
            .or_else(|_| fetch_quote_from_coingecko(client, cfg, &symbol)),
    }
}

fn value_to_f64(v: &Value) -> Option<f64> {
    if let Some(n) = v.as_f64() {
        return Some(n);
    }
    v.as_str().and_then(|s| s.parse::<f64>().ok())
}

fn number_field(obj: &Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        if let Some(v) = obj.get(*key).and_then(value_to_f64) {
            return Some(v);
        }
    }
    None
}

fn render_url_template(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        let encoded = encode(v).into_owned();
        out = out.replace(&format!("{{{k}}}"), &encoded);
    }
    out
}

fn build_exchange_url(base: &str, path_or_url: &str, vars: &[(&str, &str)]) -> String {
    let rendered = render_url_template(path_or_url, vars);
    if rendered.starts_with("http://") || rendered.starts_with("https://") {
        return rendered;
    }
    let b = base.trim_end_matches('/');
    let p = rendered.trim_start_matches('/');
    format!("{b}/{p}")
}

fn parse_evm_api_result_string(v: &Value) -> Option<String> {
    v.get("result")
        .and_then(|x| x.as_str())
        .map(str::to_string)
        .or_else(|| {
            v.get("data")
                .and_then(|x| x.get("result"))
                .and_then(|x| x.as_str())
                .map(str::to_string)
        })
}

fn parse_evm_tx_list(v: &Value, address: &str, limit: usize) -> Vec<Value> {
    let addr_lc = address.to_ascii_lowercase();
    let mut items = Vec::new();
    let arr_opt = v
        .get("result")
        .and_then(|x| x.as_array())
        .or_else(|| v.get("data").and_then(|x| x.as_array()));
    let Some(arr) = arr_opt else {
        return items;
    };
    for it in arr.iter().take(limit) {
        let from = it.get("from").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let to = it.get("to").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let hash = it.get("hash").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let ts = it
            .get("timeStamp")
            .or_else(|| it.get("timestamp"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let value_raw = it.get("value").and_then(|x| x.as_str()).unwrap_or("0");
        let direction = if to.eq_ignore_ascii_case(&addr_lc) {
            "in"
        } else if from.eq_ignore_ascii_case(&addr_lc) {
            "out"
        } else {
            "other"
        };
        items.push(json!({
            "hash": hash,
            "from": from,
            "to": to,
            "direction": direction,
            "value_raw": value_raw,
            "timestamp": ts
        }));
    }
    items
}

fn raw_to_decimal_string(raw: &str, decimals: u32) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return "0".to_string();
    }
    let sign = if raw.starts_with('-') { "-" } else { "" };
    let digits = raw.trim_start_matches('-').trim_start_matches('0');
    if digits.is_empty() {
        return "0".to_string();
    }
    if decimals == 0 {
        return format!("{sign}{digits}");
    }
    let d = decimals as usize;
    if digits.len() <= d {
        let frac = format!("{:0>width$}", digits, width = d).trim_end_matches('0').to_string();
        if frac.is_empty() {
            "0".to_string()
        } else {
            format!("{sign}0.{frac}")
        }
    } else {
        let int_part = &digits[..digits.len() - d];
        let frac_part = digits[digits.len() - d..].trim_end_matches('0').to_string();
        if frac_part.is_empty() {
            format!("{sign}{int_part}")
        } else {
            format!("{sign}{int_part}.{frac_part}")
        }
    }
}

fn fetch_quote_from_binance(client: &Client, cfg: &RootConfig, symbol: &str) -> Result<Quote, String> {
    let base = cfg.binance.base_url.trim_end_matches('/');
    let normalized_symbol = normalize_symbol(symbol);
    let url = build_exchange_url(
        base,
        cfg.crypto.binance_quote_24hr_api_path.trim(),
        &[("symbol", &normalized_symbol)],
    );
    let v: Value = client
        .get(url)
        .send()
        .map_err(|err| format!("binance quote request failed: {err}"))?
        .json()
        .map_err(|err| format!("binance quote parse failed: {err}"))?;
    if v.get("lastPrice").is_none() && v.get("price").is_none() {
        let err_code = v.get("code").and_then(|x| x.as_i64()).unwrap_or(0);
        if err_code != 0 {
            let msg = v.get("msg").and_then(|x| x.as_str()).unwrap_or("unknown");
            return Err(format!("binance quote api error code={err_code}: {msg}"));
        }
    }
    let mut price = number_field(&v, &["lastPrice", "price", "last", "close"]);
    if price.is_none() {
        let fallback_url = build_exchange_url(
            base,
            cfg.crypto.binance_quote_price_api_path.trim(),
            &[("symbol", &normalized_symbol)],
        );
        let fallback_v: Value = client
            .get(fallback_url)
            .send()
            .map_err(|err| format!("binance quote fallback request failed: {err}"))?
            .json()
            .map_err(|err| format!("binance quote fallback parse failed: {err}"))?;
        price = number_field(&fallback_v, &["price", "lastPrice", "last", "close"]);
    }
    let price = price.ok_or_else(|| "binance quote missing price field".to_string())?;
    let change = v
        .get("priceChangePercent")
        .and_then(value_to_f64);
    Ok(Quote {
        symbol: normalized_symbol,
        price_usd: price,
        change_24h_pct: change,
        exchange: "binance".to_string(),
        source: "binance_api".to_string(),
    })
}

fn ensure_symbol_supported_on_binance(client: &Client, cfg: &RootConfig, symbol: &str) -> Result<(), String> {
    let base = cfg.binance.base_url.trim_end_matches('/');
    let normalized_symbol = normalize_symbol(symbol);
    let url = format!("{base}/api/v3/exchangeInfo?symbol={}", encode(&normalized_symbol));
    let v: Value = client
        .get(url)
        .send()
        .map_err(|err| format!("binance exchangeInfo request failed: {err}"))?
        .json()
        .map_err(|err| format!("binance exchangeInfo parse failed: {err}"))?;
    if let Some(code) = v.get("code").and_then(|x| x.as_i64()) {
        if code != 0 {
            return Err(tr_with(
                "crypto.err.symbol_not_on_binance",
                &[("symbol", &normalized_symbol)],
            ));
        }
    }
    let symbols = v.get("symbols").and_then(|x| x.as_array());
    let exists = symbols
        .map(|arr| {
            arr.iter().any(|it| {
                it.get("symbol")
                    .and_then(|x| x.as_str())
                    .map(|s| s.eq_ignore_ascii_case(&normalized_symbol))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    if !exists {
        return Err(tr_with(
            "crypto.err.symbol_not_on_binance",
            &[("symbol", &normalized_symbol)],
        ));
    }
    Ok(())
}

fn fetch_quote_from_okx(client: &Client, cfg: &RootConfig, symbol: &str) -> Result<Quote, String> {
    let base = cfg.okx.base_url.trim_end_matches('/');
    let inst_id = to_okx_inst_id(symbol);
    let url = build_exchange_url(
        base,
        cfg.crypto.okx_market_ticker_api_path.trim(),
        &[("inst_id", &inst_id), ("instId", &inst_id)],
    );
    let v: Value = client
        .get(url)
        .send()
        .map_err(|err| format!("okx quote request failed: {err}"))?
        .json()
        .map_err(|err| format!("okx quote parse failed: {err}"))?;
    if v.get("code").and_then(|x| x.as_str()).unwrap_or("0") != "0" {
        return Err(format!(
            "okx quote error: {}",
            v.get("msg").and_then(|x| x.as_str()).unwrap_or("unknown")
        ));
    }
    let data = v
        .get("data")
        .and_then(|x| x.as_array())
        .and_then(|x| x.first())
        .ok_or_else(|| "okx quote missing data".to_string())?;
    let last = data
        .get("last")
        .and_then(|x| x.as_str())
        .and_then(|x| x.parse::<f64>().ok())
        .ok_or_else(|| "okx quote missing last".to_string())?;
    let open = data
        .get("open24h")
        .and_then(|x| x.as_str())
        .and_then(|x| x.parse::<f64>().ok())
        .unwrap_or(0.0);
    let change = if open > 0.0 {
        Some((last - open) / open * 100.0)
    } else {
        None
    };
    Ok(Quote {
        symbol: normalize_symbol(symbol),
        price_usd: last,
        change_24h_pct: change,
        exchange: "okx".to_string(),
        source: "okx_api".to_string(),
    })
}

fn fetch_quote_from_coingecko(client: &Client, cfg: &RootConfig, symbol: &str) -> Result<Quote, String> {
    let coin_id = symbol_to_coingecko_id(symbol).ok_or_else(|| {
        "coingecko mapping missing for symbol; try exchange=binance or map this symbol".to_string()
    })?;
    let ids = encode(coin_id).into_owned();
    let template = cfg.crypto.coingecko_simple_price_api_url.trim();
    let url = if template.contains("{ids}") {
        template.replace("{ids}", &ids)
    } else if template.contains('?') {
        format!("{template}&ids={ids}&vs_currencies=usd&include_24hr_change=true")
    } else {
        format!("{template}?ids={ids}&vs_currencies=usd&include_24hr_change=true")
    };
    let v: Value = client
        .get(url)
        .send()
        .map_err(|err| format!("coingecko quote request failed: {err}"))?
        .json()
        .map_err(|err| format!("coingecko quote parse failed: {err}"))?;
    let node = v
        .get(coin_id)
        .ok_or_else(|| "coingecko quote missing symbol node".to_string())?;
    let price = node
        .get("usd")
        .and_then(|x| x.as_f64())
        .ok_or_else(|| "coingecko quote missing usd".to_string())?;
    let change = node.get("usd_24h_change").and_then(|x| x.as_f64());
    Ok(Quote {
        symbol: normalize_symbol(symbol),
        price_usd: price,
        change_24h_pct: change,
        exchange: "coingecko".to_string(),
        source: "coingecko_api".to_string(),
    })
}

fn fetch_quote_from_gateio(client: &Client, cfg: &RootConfig, symbol: &str) -> Result<Quote, String> {
    let pair = to_gateio_pair(symbol);
    let path_or_url = cfg.crypto.gateio_quote_ticker_api_path.trim();
    let url = build_exchange_url(
        "https://api.gateio.ws",
        path_or_url,
        &[("currency_pair", &pair)],
    );
    let v: Value = client
        .get(url)
        .send()
        .map_err(|err| format!("gateio quote request failed: {err}"))?
        .json()
        .map_err(|err| format!("gateio quote parse failed: {err}"))?;
    let row = v
        .as_array()
        .and_then(|arr| arr.first())
        .ok_or_else(|| "gateio quote missing data".to_string())?;
    let price = row
        .get("last")
        .and_then(value_to_f64)
        .ok_or_else(|| "gateio quote missing last".to_string())?;
    Ok(Quote {
        symbol: normalize_symbol(symbol),
        price_usd: price,
        change_24h_pct: row.get("change_percentage").and_then(value_to_f64),
        exchange: "gateio".to_string(),
        source: "gateio_api".to_string(),
    })
}

fn fetch_quote_from_coinbase(client: &Client, cfg: &RootConfig, symbol: &str) -> Result<Quote, String> {
    let product = to_coinbase_product(symbol);
    let path_or_url = cfg.crypto.coinbase_quote_ticker_api_path.trim();
    let url = build_exchange_url(
        "https://api.exchange.coinbase.com",
        path_or_url,
        &[("product_id", &product), ("product", &product)],
    );
    let v: Value = client
        .get(url)
        .header("User-Agent", "RustClaw-Crypto-Skill/1.0")
        .send()
        .map_err(|err| format!("coinbase quote request failed: {err}"))?
        .json()
        .map_err(|err| format!("coinbase quote parse failed: {err}"))?;
    let price = number_field(&v, &["price", "ask", "bid"])
        .ok_or_else(|| "coinbase quote missing price".to_string())?;
    Ok(Quote {
        symbol: normalize_symbol(symbol),
        price_usd: price,
        change_24h_pct: None,
        exchange: "coinbase".to_string(),
        source: "coinbase_api".to_string(),
    })
}

fn fetch_quote_from_kraken(client: &Client, cfg: &RootConfig, symbol: &str) -> Result<Quote, String> {
    let pair = to_kraken_pair(symbol);
    let path_or_url = cfg.crypto.kraken_quote_ticker_api_path.trim();
    let url = build_exchange_url("https://api.kraken.com", path_or_url, &[("pair", &pair)]);
    let v: Value = client
        .get(url)
        .send()
        .map_err(|err| format!("kraken quote request failed: {err}"))?
        .json()
        .map_err(|err| format!("kraken quote parse failed: {err}"))?;
    let error = v
        .get("error")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !error.is_empty() {
        return Err(format!("kraken quote error: {}", error.join("; ")));
    }
    let result = v
        .get("result")
        .and_then(|x| x.as_object())
        .ok_or_else(|| "kraken quote missing result".to_string())?;
    let first = result
        .values()
        .next()
        .ok_or_else(|| "kraken quote missing ticker node".to_string())?;
    let price = first
        .get("c")
        .and_then(|x| x.as_array())
        .and_then(|x| x.first())
        .and_then(value_to_f64)
        .ok_or_else(|| "kraken quote missing c[0]".to_string())?;
    Ok(Quote {
        symbol: normalize_symbol(symbol),
        price_usd: price,
        change_24h_pct: None,
        exchange: "kraken".to_string(),
        source: "kraken_api".to_string(),
    })
}

fn fetch_book_ticker_from_binance(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
) -> Result<BookTicker, String> {
    let base = cfg.binance.base_url.trim_end_matches('/');
    let normalized_symbol = normalize_symbol(symbol);
    let url = build_exchange_url(
        base,
        cfg.crypto.binance_book_ticker_api_path.trim(),
        &[("symbol", &normalized_symbol)],
    );
    let v: Value = client
        .get(url)
        .send()
        .map_err(|err| format!("binance bookTicker request failed: {err}"))?
        .json()
        .map_err(|err| format!("binance bookTicker parse failed: {err}"))?;
    let bid_price = v
        .get("bidPrice")
        .and_then(value_to_f64)
        .ok_or_else(|| "binance bookTicker missing bidPrice".to_string())?;
    let bid_qty = v
        .get("bidQty")
        .and_then(value_to_f64)
        .ok_or_else(|| "binance bookTicker missing bidQty".to_string())?;
    let ask_price = v
        .get("askPrice")
        .and_then(value_to_f64)
        .ok_or_else(|| "binance bookTicker missing askPrice".to_string())?;
    let ask_qty = v
        .get("askQty")
        .and_then(value_to_f64)
        .ok_or_else(|| "binance bookTicker missing askQty".to_string())?;
    Ok(BookTicker {
        symbol: normalized_symbol,
        bid_price,
        bid_qty,
        ask_price,
        ask_qty,
        exchange: "binance".to_string(),
        source: "binance_api".to_string(),
    })
}

fn fetch_book_ticker_from_okx(client: &Client, cfg: &RootConfig, symbol: &str) -> Result<BookTicker, String> {
    let base = cfg.okx.base_url.trim_end_matches('/');
    let inst_id = to_okx_inst_id(symbol);
    let url = build_exchange_url(
        base,
        cfg.crypto.okx_market_ticker_api_path.trim(),
        &[("inst_id", &inst_id), ("instId", &inst_id)],
    );
    let v: Value = client
        .get(url)
        .send()
        .map_err(|err| format!("okx bookTicker request failed: {err}"))?
        .json()
        .map_err(|err| format!("okx bookTicker parse failed: {err}"))?;
    if v.get("code").and_then(|x| x.as_str()).unwrap_or("0") != "0" {
        return Err(format!(
            "okx bookTicker error: {}",
            v.get("msg").and_then(|x| x.as_str()).unwrap_or("unknown")
        ));
    }
    let data = v
        .get("data")
        .and_then(|x| x.as_array())
        .and_then(|x| x.first())
        .ok_or_else(|| "okx bookTicker missing data".to_string())?;
    let bid_price = data
        .get("bidPx")
        .and_then(value_to_f64)
        .ok_or_else(|| "okx bookTicker missing bidPx".to_string())?;
    let bid_qty = data
        .get("bidSz")
        .and_then(value_to_f64)
        .ok_or_else(|| "okx bookTicker missing bidSz".to_string())?;
    let ask_price = data
        .get("askPx")
        .and_then(value_to_f64)
        .ok_or_else(|| "okx bookTicker missing askPx".to_string())?;
    let ask_qty = data
        .get("askSz")
        .and_then(value_to_f64)
        .ok_or_else(|| "okx bookTicker missing askSz".to_string())?;
    Ok(BookTicker {
        symbol: normalize_symbol(symbol),
        bid_price,
        bid_qty,
        ask_price,
        ask_qty,
        exchange: "okx".to_string(),
        source: "okx_api".to_string(),
    })
}

fn fetch_book_ticker_from_gateio(client: &Client, cfg: &RootConfig, symbol: &str) -> Result<BookTicker, String> {
    let pair = to_gateio_pair(symbol);
    let url = build_exchange_url(
        "https://api.gateio.ws",
        cfg.crypto.gateio_book_ticker_api_path.trim(),
        &[("currency_pair", &pair)],
    );
    let v: Value = client
        .get(url)
        .send()
        .map_err(|err| format!("gateio bookTicker request failed: {err}"))?
        .json()
        .map_err(|err| format!("gateio bookTicker parse failed: {err}"))?;
    let row = v
        .as_array()
        .and_then(|arr| arr.first())
        .ok_or_else(|| "gateio bookTicker missing data".to_string())?;
    let bid_price = row
        .get("highest_bid")
        .and_then(value_to_f64)
        .ok_or_else(|| "gateio bookTicker missing highest_bid".to_string())?;
    let bid_qty = row
        .get("highest_size")
        .and_then(value_to_f64)
        .unwrap_or(0.0);
    let ask_price = row
        .get("lowest_ask")
        .and_then(value_to_f64)
        .ok_or_else(|| "gateio bookTicker missing lowest_ask".to_string())?;
    let ask_qty = row
        .get("lowest_size")
        .and_then(value_to_f64)
        .unwrap_or(0.0);
    Ok(BookTicker {
        symbol: normalize_symbol(symbol),
        bid_price,
        bid_qty,
        ask_price,
        ask_qty,
        exchange: "gateio".to_string(),
        source: "gateio_api".to_string(),
    })
}

fn fetch_book_ticker_from_coinbase(client: &Client, cfg: &RootConfig, symbol: &str) -> Result<BookTicker, String> {
    let product = to_coinbase_product(symbol);
    let url = build_exchange_url(
        "https://api.exchange.coinbase.com",
        cfg.crypto.coinbase_book_ticker_api_path.trim(),
        &[("product_id", &product), ("product", &product)],
    );
    let v: Value = client
        .get(url)
        .header("User-Agent", "RustClaw-Crypto-Skill/1.0")
        .send()
        .map_err(|err| format!("coinbase bookTicker request failed: {err}"))?
        .json()
        .map_err(|err| format!("coinbase bookTicker parse failed: {err}"))?;
    let bid_price = v
        .get("bid")
        .and_then(value_to_f64)
        .ok_or_else(|| "coinbase bookTicker missing bid".to_string())?;
    let ask_price = v
        .get("ask")
        .and_then(value_to_f64)
        .ok_or_else(|| "coinbase bookTicker missing ask".to_string())?;
    Ok(BookTicker {
        symbol: normalize_symbol(symbol),
        bid_price,
        bid_qty: 0.0,
        ask_price,
        ask_qty: 0.0,
        exchange: "coinbase".to_string(),
        source: "coinbase_api".to_string(),
    })
}

fn fetch_book_ticker_from_kraken(client: &Client, cfg: &RootConfig, symbol: &str) -> Result<BookTicker, String> {
    let pair = to_kraken_pair(symbol);
    let url = build_exchange_url(
        "https://api.kraken.com",
        cfg.crypto.kraken_book_ticker_api_path.trim(),
        &[("pair", &pair)],
    );
    let v: Value = client
        .get(url)
        .send()
        .map_err(|err| format!("kraken bookTicker request failed: {err}"))?
        .json()
        .map_err(|err| format!("kraken bookTicker parse failed: {err}"))?;
    let error = v
        .get("error")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !error.is_empty() {
        return Err(format!("kraken bookTicker error: {}", error.join("; ")));
    }
    let result = v
        .get("result")
        .and_then(|x| x.as_object())
        .ok_or_else(|| "kraken bookTicker missing result".to_string())?;
    let first = result
        .values()
        .next()
        .ok_or_else(|| "kraken bookTicker missing ticker node".to_string())?;
    let bid_price = first
        .get("b")
        .and_then(|x| x.as_array())
        .and_then(|x| x.first())
        .and_then(value_to_f64)
        .ok_or_else(|| "kraken bookTicker missing b[0]".to_string())?;
    let bid_qty = first
        .get("b")
        .and_then(|x| x.as_array())
        .and_then(|x| x.get(1))
        .and_then(value_to_f64)
        .unwrap_or(0.0);
    let ask_price = first
        .get("a")
        .and_then(|x| x.as_array())
        .and_then(|x| x.first())
        .and_then(value_to_f64)
        .ok_or_else(|| "kraken bookTicker missing a[0]".to_string())?;
    let ask_qty = first
        .get("a")
        .and_then(|x| x.as_array())
        .and_then(|x| x.get(1))
        .and_then(value_to_f64)
        .unwrap_or(0.0);
    Ok(BookTicker {
        symbol: normalize_symbol(symbol),
        bid_price,
        bid_qty,
        ask_price,
        ask_qty,
        exchange: "kraken".to_string(),
        source: "kraken_api".to_string(),
    })
}

/// Candle OHLCV data: (open, high, low, close, volume_base, volume_quote)
#[derive(Debug, Clone)]
struct CandleOhlcv {
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    quote_volume: f64,
}

fn fetch_candles_binance(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
    interval: &str,
    limit: u64,
) -> Result<Vec<f64>, String> {
    Ok(fetch_candles_ohlcv_binance(client, cfg, symbol, interval, limit)?
        .into_iter()
        .map(|c| c.close)
        .collect())
}

fn fetch_candles_ohlcv_binance(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
    interval: &str,
    limit: u64,
) -> Result<Vec<CandleOhlcv>, String> {
    let base = cfg.binance.base_url.trim_end_matches('/');
    let url = format!(
        "{base}/api/v3/klines?symbol={}&interval={}&limit={}",
        symbol,
        map_interval_binance(interval),
        limit
    );
    let v: Value = client
        .get(url)
        .send()
        .map_err(|err| format!("fetch binance candles failed: {err}"))?
        .json()
        .map_err(|err| format!("parse binance candles failed: {err}"))?;
    let arr = v
        .as_array()
        .ok_or_else(|| "binance candles response is invalid".to_string())?;
    // Binance kline format: [open_time, open, high, low, close, volume, close_time, quote_volume, ...]
    let mut candles = Vec::new();
    for item in arr {
        if let Some(k) = item.as_array() {
            let parse_str = |idx: usize| -> f64 {
                k.get(idx)
                    .and_then(|x| x.as_str())
                    .and_then(|x| x.parse::<f64>().ok())
                    .unwrap_or(0.0)
            };
            candles.push(CandleOhlcv {
                open: parse_str(1),
                high: parse_str(2),
                low: parse_str(3),
                close: parse_str(4),
                volume: parse_str(5),
                quote_volume: parse_str(7),
            });
        }
    }
    Ok(candles)
}

fn fetch_candles_okx(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
    interval: &str,
    limit: u64,
) -> Result<Vec<f64>, String> {
    Ok(fetch_candles_ohlcv_okx(client, cfg, symbol, interval, limit)?
        .into_iter()
        .map(|c| c.close)
        .collect())
}

fn fetch_candles_ohlcv_okx(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
    interval: &str,
    limit: u64,
) -> Result<Vec<CandleOhlcv>, String> {
    let base = cfg.okx.base_url.trim_end_matches('/');
    let inst_id = to_okx_inst_id(symbol);
    let bar = map_interval_okx(interval);
    let url = format!(
        "{base}/api/v5/market/candles?instId={}&bar={}&limit={}",
        encode(&inst_id),
        encode(bar),
        limit
    );
    let v: Value = client
        .get(url)
        .send()
        .map_err(|err| format!("fetch okx candles failed: {err}"))?
        .json()
        .map_err(|err| format!("parse okx candles failed: {err}"))?;
    if v.get("code").and_then(|x| x.as_str()).unwrap_or("0") != "0" {
        return Err(format!(
            "okx candles error: {}",
            v.get("msg").and_then(|x| x.as_str()).unwrap_or("unknown")
        ));
    }
    let arr = v
        .get("data")
        .and_then(|x| x.as_array())
        .ok_or_else(|| "okx candles response is invalid".to_string())?;
    // OKX candle format: [ts, open, high, low, close, vol_base, vol_ccy, vol_ccy_quote, confirmed]
    let mut candles: Vec<CandleOhlcv> = arr
        .iter()
        .filter_map(|item| {
            let k = item.as_array()?;
            let parse_str = |idx: usize| -> f64 {
                k.get(idx)
                    .and_then(|x| x.as_str())
                    .and_then(|x| x.parse::<f64>().ok())
                    .unwrap_or(0.0)
            };
            Some(CandleOhlcv {
                open: parse_str(1),
                high: parse_str(2),
                low: parse_str(3),
                close: parse_str(4),
                volume: parse_str(5),
                quote_volume: parse_str(6),
            })
        })
        .collect();
    candles.reverse(); // OKX returns newest first
    Ok(candles)
}

fn binance_signed_request(
    client: &Client,
    cfg: &RootConfig,
    method: Method,
    path: &str,
    params: &mut Vec<(&str, String)>,
) -> Result<Value, String> {
    ensure_binance_config(cfg)?;
    params.push(("timestamp", now_ts_ms().to_string()));
    let recv_window = cfg.binance.recv_window.clamp(1, 60_000);
    params.push(("recvWindow", recv_window.to_string()));
    let query = to_query(params);
    let signature = bytes_to_hex(&hmac_sha256_bytes(&cfg.binance.api_secret, &query)?);
    let full_q = format!("{query}&signature={signature}");
    let base = cfg.binance.base_url.trim_end_matches('/');
    let url = format!("{base}{path}?{full_q}");

    let req = client
        .request(method, url)
        .header("X-MBX-APIKEY", cfg.binance.api_key.trim());
    let resp = req
        .send()
        .map_err(|err| format!("binance request failed: {err}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .map_err(|err| format!("parse binance response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "binance error status={status}: {}",
            truncate(&v.to_string(), 500)
        ));
    }
    if v.get("code").and_then(|x| x.as_i64()).is_some() && v.get("msg").is_some() {
        let code = v.get("code").and_then(|x| x.as_i64()).unwrap_or(0);
        if code < 0 {
            return Err(format!(
                "binance api error code={code}: {}",
                v.get("msg").and_then(|x| x.as_str()).unwrap_or("unknown")
            ));
        }
    }
    Ok(v)
}

fn okx_request(
    client: &Client,
    cfg: &RootConfig,
    method: Method,
    path: &str,
    query: Option<&str>,
    body: Option<Value>,
) -> Result<Value, String> {
    ensure_okx_config(cfg)?;
    let query = query.unwrap_or("");
    let req_path = if query.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{query}")
    };
    let body_text = body
        .as_ref()
        .map(|v| v.to_string())
        .unwrap_or_else(String::new);
    let ts = now_iso_ts();
    let prehash = format!("{}{}{}{}", ts, method.as_str().to_ascii_uppercase(), req_path, body_text);
    let sign = STANDARD.encode(hmac_sha256_bytes(&cfg.okx.api_secret, &prehash)?);
    let base = cfg.okx.base_url.trim_end_matches('/');
    let url = format!("{base}{req_path}");
    let mut req = client
        .request(method, url)
        .header("OK-ACCESS-KEY", cfg.okx.api_key.trim())
        .header("OK-ACCESS-SIGN", sign)
        .header("OK-ACCESS-TIMESTAMP", ts)
        .header("OK-ACCESS-PASSPHRASE", cfg.okx.passphrase.trim())
        .header("Content-Type", "application/json");
    if cfg.okx.simulated {
        req = req.header("x-simulated-trading", "1");
    }
    if let Some(v) = body {
        req = req.body(v.to_string());
    }
    let resp = req.send().map_err(|err| format!("okx request failed: {err}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .map_err(|err| format!("parse okx response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "okx error status={status}: {}",
            truncate(&v.to_string(), 500)
        ));
    }
    if v.get("code").and_then(|x| x.as_str()).unwrap_or("0") != "0" {
        return Err(format!(
            "okx api error code={}: {}",
            v.get("code").and_then(|x| x.as_str()).unwrap_or(""),
            v.get("msg").and_then(|x| x.as_str()).unwrap_or("unknown")
        ));
    }
    Ok(v)
}

fn ensure_binance_config(cfg: &RootConfig) -> Result<(), String> {
    if !cfg.binance.enabled {
        return Err(tr("crypto.err.binance_not_bound"));
    }
    if is_placeholder(&cfg.binance.api_key) || is_placeholder(&cfg.binance.api_secret) {
        return Err(tr("crypto.err.binance_credentials_incomplete"));
    }
    Ok(())
}

fn ensure_okx_config(cfg: &RootConfig) -> Result<(), String> {
    if !cfg.okx.enabled {
        return Err(tr("crypto.err.okx_not_bound"));
    }
    if is_placeholder(&cfg.okx.api_key)
        || is_placeholder(&cfg.okx.api_secret)
        || is_placeholder(&cfg.okx.passphrase)
    {
        return Err(tr("crypto.err.okx_credentials_incomplete"));
    }
    Ok(())
}

fn is_placeholder(v: &str) -> bool {
    let t = v.trim();
    t.is_empty() || t.starts_with("REPLACE_ME_") || t == "__REDACTED__"
}

fn resolve_exchange(input: Option<&str>, cfg: &RootConfig) -> String {
    let raw = input
        .or(cfg.crypto.execution_mode.as_deref())
        .or(cfg.crypto.default_exchange.as_deref())
        .unwrap_or("binance")
        .trim()
        .to_ascii_lowercase();
    raw
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
        "USDT", "USD", "USDC", "BUSD", "FDUSD", "USDE", "BTC", "ETH", "BNB", "EUR", "TRY",
        "BRL", "DAI",
    ]
        .iter()
        .any(|q| symbol.ends_with(q))
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
mod tests {
    use super::*;

    #[test]
    fn normalize_symbol_ok() {
        assert_eq!(normalize_symbol("btc/usdt"), "BTCUSDT");
        assert_eq!(normalize_symbol("eth-usd"), "ETHUSD");
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
    fn quote_symbol_mapping_for_new_exchanges() {
        assert_eq!(to_gateio_pair("BTCUSDT"), "BTC_USDT");
        assert_eq!(to_coinbase_product("BTCUSDT"), "BTC-USD");
        assert_eq!(to_kraken_pair("BTCUSDT"), "XBTUSDT");
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
}
