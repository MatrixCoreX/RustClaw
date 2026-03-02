use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
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
    require_explicit_send: Option<bool>,
    #[serde(default)]
    max_notional_usd: Option<f64>,
    #[serde(default)]
    allowed_exchanges: Vec<String>,
    #[serde(default)]
    allowed_symbols: Vec<String>,
    #[serde(default)]
    blocked_actions: Vec<String>,
    #[serde(default)]
    paper_state_path: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    i18n_path: Option<String>,
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

#[derive(Debug, Clone)]
struct TradeInput {
    exchange: String,
    symbol: String,
    side: String,
    order_type: String,
    qty: f64,
    price: Option<f64>,
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
}

#[derive(Debug, Clone, Serialize)]
struct OrderState {
    order_id: String,
    exchange: String,
    symbol: String,
    side: String,
    order_type: String,
    qty: f64,
    price: Option<f64>,
    notional_usd: f64,
    status: String,
    updated_ts: u64,
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
    current.insert("crypto.err.news_no_items".to_string(), "news feed has no items".to_string());
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
        "Binance API is not bound. Configure first:\nTelegram: /cryptoapi set binance <api_key> <api_secret>\nOr edit configs/crypto.toml [binance].enabled=true with api_key/api_secret."
            .to_string(),
    );
    current.insert(
        "crypto.err.binance_credentials_incomplete".to_string(),
        "Binance API credentials are incomplete. Configure first:\nTelegram: /cryptoapi set binance <api_key> <api_secret>\nOr edit configs/crypto.toml [binance].api_key/[binance].api_secret."
            .to_string(),
    );
    current.insert(
        "crypto.err.okx_not_bound".to_string(),
        "OKX API is not bound. Configure first:\nTelegram: /cryptoapi set okx <api_key> <api_secret> <passphrase>\nOr edit configs/crypto.toml [okx].enabled=true with api_key/api_secret/passphrase."
            .to_string(),
    );
    current.insert(
        "crypto.err.okx_credentials_incomplete".to_string(),
        "OKX API credentials are incomplete. Configure first:\nTelegram: /cryptoapi set okx <api_key> <api_secret> <passphrase>\nOr edit configs/crypto.toml [okx].api_key/[okx].api_secret/[okx].passphrase."
            .to_string(),
    );
    current.insert("crypto.msg.no_orders_yet".to_string(), "no orders yet".to_string());
    current.insert("crypto.msg.no_filled_positions".to_string(), "no filled positions".to_string());
    current.insert("crypto.msg.no_balances".to_string(), "no balances".to_string());
    TextCatalog { current }
}

fn load_external_i18n(path: &Path) -> Option<HashMap<String, String>> {
    let raw = std::fs::read_to_string(path).ok()?;
    let value: toml::Value = toml::from_str(&raw).ok()?;
    let dict = value.get("dict")?.as_table()?;
    let mut out = HashMap::new();
    for (k, v) in dict {
        if let Some(text) = v.as_str() {
            out.insert(k.to_string(), text.to_string());
        }
    }
    Some(out)
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
            Ok(req) => match execute(&cfg, &workspace_root, req.args) {
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

fn execute(cfg: &RootConfig, workspace_root: &Path, args: Value) -> Result<(String, Value), String> {
    let obj = args
        .as_object()
        .ok_or_else(|| tr("crypto.err.args_object"))?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("quote")
        .trim()
        .to_ascii_lowercase();
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
        "quote" => handle_quote(&client, cfg, obj),
        "multi_quote" => handle_multi_quote(&client, cfg, obj),
        "candles" => handle_candles(&client, cfg, obj),
        "indicator" => handle_indicator(&client, cfg, obj),
        "news" => handle_news(&client, obj),
        "onchain" => handle_onchain(&client, obj),
        "trade_preview" => handle_trade_preview(&client, cfg, obj),
        "trade_submit" => handle_trade_submit(&client, cfg, workspace_root, obj),
        "order_status" => handle_order_status(&client, cfg, workspace_root, obj),
        "cancel_order" => handle_cancel_order(&client, cfg, workspace_root, obj),
        "positions" => handle_positions(&client, cfg, workspace_root, obj),
        _ => Err(tr("crypto.err.unsupported_action")),
    }
}

fn handle_quote(client: &Client, cfg: &RootConfig, obj: &serde_json::Map<String, Value>) -> Result<(String, Value), String> {
    let symbol = obj
        .get("symbol")
        .and_then(|v| v.as_str())
        .ok_or_else(|| tr("crypto.err.symbol_required"))?;
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg);
    let quote = fetch_quote(client, cfg, symbol, &exchange)?;
    let text = format!(
        "{} ${:.6} ({})",
        quote.symbol,
        quote.price_usd,
        quote
            .change_24h_pct
            .map(|v| format!("{v:+.2}%"))
            .unwrap_or_else(|| "24h n/a".to_string())
    );
    Ok((text, json!({ "action": "quote", "quote": quote })))
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
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg);
    let mut quotes = Vec::new();
    let mut lines = Vec::new();
    for s in symbols {
        let q = fetch_quote(client, cfg, &s, &exchange)?;
        lines.push(format!(
            "{} ${:.6} ({})",
            q.symbol,
            q.price_usd,
            q.change_24h_pct
                .map(|v| format!("{v:+.2}%"))
                .unwrap_or_else(|| "24h n/a".to_string())
        ));
        quotes.push(q);
    }
    Ok((
        lines.join("\n"),
        json!({ "action": "multi_quote", "quotes": quotes }),
    ))
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
    let closes = if exchange == "okx" {
        fetch_candles_okx(client, cfg, &symbol, interval, limit)?
    } else {
        fetch_candles_binance(client, cfg, &symbol, interval, limit)?
    };
    if closes.is_empty() {
        return Err(tr("crypto.err.no_candles"));
    }
    let last = closes.last().copied().unwrap_or(0.0);
    let first = closes.first().copied().unwrap_or(last);
    let delta = if first > 0.0 {
        (last - first) / first * 100.0
    } else {
        0.0
    };
    Ok((
        format!(
            "{} {} close={} change={:+.2}% candles={}",
            symbol, interval, last, delta, closes.len()
        ),
        json!({
            "action":"candles",
            "symbol":symbol,
            "timeframe":interval,
            "exchange":exchange,
            "close_prices": closes
        }),
    ))
}

fn handle_indicator(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let mut args = obj.clone();
    args.entry("action".to_string())
        .or_insert(Value::String("candles".to_string()));
    let (_, extra) = handle_candles(client, cfg, &args)?;
    let closes = extra
        .get("close_prices")
        .and_then(|v| v.as_array())
        .ok_or_else(|| tr("crypto.err.indicator_requires_close_prices"))?;
    let values: Vec<f64> = closes.iter().filter_map(|v| v.as_f64()).collect();
    let period = obj
        .get("period")
        .and_then(|v| v.as_u64())
        .unwrap_or(14)
        .clamp(2, 200) as usize;
    if values.len() < period {
        return Err(format!(
            "not enough candles for period={}, got={}",
            period,
            values.len()
        ));
    }
    let tail = &values[values.len() - period..];
    let sma = tail.iter().sum::<f64>() / period as f64;
    let last = values.last().copied().unwrap_or(0.0);
    let signal = if last >= sma { "above_sma" } else { "below_sma" };
    let symbol = obj
        .get("symbol")
        .and_then(|v| v.as_str())
        .map(normalize_symbol)
        .unwrap_or_else(|| "UNKNOWN".to_string());
    Ok((
        format!("{symbol} SMA{period}={sma:.6} last={last:.6} signal={signal}"),
        json!({
            "action":"indicator",
            "indicator":"sma",
            "period":period,
            "symbol":symbol,
            "sma":sma,
            "last":last,
            "signal":signal
        }),
    ))
}

fn handle_news(client: &Client, obj: &serde_json::Map<String, Value>) -> Result<(String, Value), String> {
    let limit = obj
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .clamp(1, 20) as usize;
    let feed_url = obj
        .get("feed_url")
        .and_then(|v| v.as_str())
        .unwrap_or("https://www.coindesk.com/arc/outboundfeeds/rss/");
    let xml = client
        .get(feed_url)
        .header("User-Agent", "RustClaw-Crypto-Skill/1.0")
        .send()
        .map_err(|err| format!("fetch news failed: {err}"))?
        .text()
        .map_err(|err| format!("read news response failed: {err}"))?;
    let mut items = Vec::new();
    for blk in extract_blocks(&xml, "item").into_iter().take(limit) {
        let title = extract_tag_text(blk, "title").unwrap_or_else(|| "(no title)".to_string());
        let link = extract_tag_text(blk, "link").unwrap_or_default();
        let date = extract_tag_text(blk, "pubDate").unwrap_or_default();
        items.push(json!({"title":title,"link":link,"date":date}));
    }
    if items.is_empty() {
        for blk in extract_blocks(&xml, "entry").into_iter().take(limit) {
            let title = extract_tag_text(blk, "title").unwrap_or_else(|| "(no title)".to_string());
            let link = extract_atom_link(blk).unwrap_or_default();
            let date = extract_tag_text(blk, "updated").unwrap_or_default();
            items.push(json!({"title":title,"link":link,"date":date}));
        }
    }
    if items.is_empty() {
        return Err(tr("crypto.err.news_no_items"));
    }
    let text = items
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            format!(
                "{}. {}",
                idx + 1,
                item.get("title").and_then(|v| v.as_str()).unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok((text, json!({"action":"news","items":items})))
}

fn handle_onchain(client: &Client, obj: &serde_json::Map<String, Value>) -> Result<(String, Value), String> {
    let chain = obj
        .get("chain")
        .and_then(|v| v.as_str())
        .unwrap_or("bitcoin")
        .trim()
        .to_ascii_lowercase();
    match chain.as_str() {
        "bitcoin" | "btc" => {
            let v: Value = client
                .get("https://mempool.space/api/v1/fees/recommended")
                .send()
                .map_err(|err| format!("fetch bitcoin onchain failed: {err}"))?
                .json()
                .map_err(|err| format!("parse bitcoin onchain failed: {err}"))?;
            let fast = v.get("fastestFee").and_then(|x| x.as_u64()).unwrap_or(0);
            let half = v.get("halfHourFee").and_then(|x| x.as_u64()).unwrap_or(0);
            let hour = v.get("hourFee").and_then(|x| x.as_u64()).unwrap_or(0);
            Ok((
                format!("BTC fee(sat/vB): fastest={fast}, half_hour={half}, hour={hour}"),
                json!({"action":"onchain","chain":"bitcoin","fees":v}),
            ))
        }
        "ethereum" | "eth" => {
            let v: Value = client
                .get("https://api.blockchair.com/ethereum/stats")
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
            Ok((
                format!(
                    "ETH onchain: tx_24h={tx_24h}, blocks_24h={blocks_24h}, market_price_usd={market:.4}"
                ),
                json!({"action":"onchain","chain":"ethereum","stats":data}),
            ))
        }
        _ => Err(tr("crypto.err.unsupported_chain")),
    }
}

fn handle_trade_preview(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let trade = parse_trade_input(obj, cfg)?;
    let checks = risk_checks(client, cfg, &trade, false)?;
    let notional = estimate_notional_usd(client, cfg, &trade)?;
    let text = format!(
        "trade_preview {} {} {} qty={} notional_usd={:.4} checks={}",
        trade.exchange,
        trade.symbol,
        trade.side,
        trade.qty,
        notional,
        checks.len()
    );
    Ok((
        text,
        json!({
            "action":"trade_preview",
            "order": trade_to_json(&trade),
            "notional_usd": notional,
            "risk_checks": checks,
            "decision":"preview_only"
        }),
    ))
}

fn handle_trade_submit(
    client: &Client,
    cfg: &RootConfig,
    workspace_root: &Path,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let trade = parse_trade_input(obj, cfg)?;
    let checks = risk_checks(client, cfg, &trade, true)?;
    let event = match trade.exchange.as_str() {
        "paper" => submit_paper_order(client, cfg, workspace_root, &trade)?,
        "binance" => submit_binance_order(client, cfg, &trade)?,
        "okx" => submit_okx_order(client, cfg, &trade)?,
        other => return Err(tr_with("crypto.err.unsupported_execution_exchange", &[("exchange", other)])),
    };
    let text = format!(
        "trade_submitted order_id={} status={} notional_usd={:.4}",
        event.order_id, event.status, event.notional_usd
    );
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

fn handle_order_status(
    client: &Client,
    cfg: &RootConfig,
    workspace_root: &Path,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg);
    match exchange.as_str() {
        "paper" => handle_order_status_paper(cfg, workspace_root, obj),
        "binance" => handle_order_status_binance(client, cfg, obj),
        "okx" => handle_order_status_okx(client, cfg, obj),
        _ => Err(tr_with("crypto.err.unsupported_exchange_for_order_status", &[("exchange", &exchange)])),
    }
}

fn handle_cancel_order(
    client: &Client,
    cfg: &RootConfig,
    workspace_root: &Path,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg);
    match exchange.as_str() {
        "paper" => handle_cancel_order_paper(cfg, workspace_root, obj),
        "binance" => handle_cancel_order_binance(client, cfg, obj),
        "okx" => handle_cancel_order_okx(client, cfg, obj),
        _ => Err(tr_with("crypto.err.unsupported_exchange_for_cancel_order", &[("exchange", &exchange)])),
    }
}

fn handle_positions(
    client: &Client,
    cfg: &RootConfig,
    workspace_root: &Path,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg);
    match exchange.as_str() {
        "paper" => handle_positions_paper(cfg, workspace_root, obj),
        "binance" => handle_positions_binance(client, cfg),
        "okx" => handle_positions_okx(client, cfg),
        _ => Err(tr_with("crypto.err.unsupported_exchange_for_positions", &[("exchange", &exchange)])),
    }
}

fn handle_order_status_paper(
    cfg: &RootConfig,
    workspace_root: &Path,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let order_id = obj.get("order_id").and_then(|v| v.as_str()).map(str::to_string);
    let orders = paper_order_states(cfg, workspace_root)?;
    if let Some(id) = order_id {
        let state = orders
            .into_iter()
            .find(|o| o.order_id == id)
            .ok_or_else(|| tr_with("crypto.err.order_not_found", &[("order_id", &id)]))?;
        let text = format!(
            "order_status {} {} {} qty={} status={}",
            state.order_id, state.symbol, state.side, state.qty, state.status
        );
        return Ok((text, json!({"action":"order_status","order":state})));
    }
    let latest = orders.iter().max_by_key(|x| x.updated_ts).cloned();
    let text = if let Some(v) = &latest {
        format!("latest_order {} {} status={}", v.order_id, v.symbol, v.status)
    } else {
        tr("crypto.msg.no_orders_yet")
    };
    Ok((text, json!({"action":"order_status","orders":orders})))
}

fn handle_cancel_order_paper(
    cfg: &RootConfig,
    workspace_root: &Path,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let order_id = obj
        .get("order_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| tr("crypto.err.order_id_required"))?;
    let state = paper_order_states(cfg, workspace_root)?
        .into_iter()
        .find(|v| v.order_id == order_id)
        .ok_or_else(|| tr_with("crypto.err.order_not_found", &[("order_id", order_id)]))?;
    if state.status != "NEW" {
        return Err(tr_with(
            "crypto.err.order_cannot_cancel_from_status",
            &[("status", &state.status)],
        ));
    }
    let cancel_event = OrderEvent {
        event: "cancel".to_string(),
        order_id: state.order_id.clone(),
        ts: now_ts(),
        exchange: state.exchange.clone(),
        symbol: state.symbol.clone(),
        side: state.side.clone(),
        order_type: state.order_type.clone(),
        qty: state.qty,
        price: state.price,
        notional_usd: state.notional_usd,
        status: "CANCELED".to_string(),
        client_order_id: None,
        reason: Some("user_cancel".to_string()),
    };
    append_paper_event(cfg, workspace_root, &cancel_event)?;
    Ok((
        format!("order_cancelled {}", state.order_id),
        json!({"action":"cancel_order","order":cancel_event}),
    ))
}

fn handle_positions_paper(
    cfg: &RootConfig,
    workspace_root: &Path,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let exchange_filter = obj
        .get("exchange")
        .and_then(|v| v.as_str())
        .map(|v| v.to_ascii_lowercase());
    let mut net: HashMap<String, f64> = HashMap::new();
    for o in paper_order_states(cfg, workspace_root)? {
        if o.status != "FILLED" {
            continue;
        }
        if let Some(ex) = &exchange_filter {
            if o.exchange.to_ascii_lowercase() != *ex {
                continue;
            }
        }
        let e = net.entry(o.symbol).or_insert(0.0);
        if o.side.eq_ignore_ascii_case("buy") {
            *e += o.qty;
        } else {
            *e -= o.qty;
        }
    }
    let mut positions = Vec::new();
    let mut lines = Vec::new();
    for (symbol, qty) in net {
        lines.push(format!("{symbol} net_qty={qty:.8}"));
        positions.push(json!({"symbol":symbol,"net_qty":qty}));
    }
    if lines.is_empty() {
        lines.push(tr("crypto.msg.no_filled_positions"));
    }
    Ok((
        lines.join("\n"),
        json!({"action":"positions","positions":positions}),
    ))
}

fn handle_order_status_binance(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let symbol = obj
        .get("symbol")
        .and_then(|v| v.as_str())
        .ok_or_else(|| tr("crypto.err.symbol_required_for_binance_order_status"))?;
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
    let v = binance_signed_request(client, cfg, Method::GET, "/api/v3/order", &mut params)?;
    let status = v.get("status").and_then(|x| x.as_str()).unwrap_or("UNKNOWN");
    let id_text = order_id.or(client_order_id).unwrap_or("unknown");
    let text = format!(
        "order_status {} {} {}",
        normalize_symbol(symbol),
        id_text,
        status
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
    let text = format!("order_cancelled {}", id_text);
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
            lines.push(format!("{asset} free={} locked={}", fmt_num(free), fmt_num(locked)));
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
    let symbol = obj
        .get("symbol")
        .and_then(|v| v.as_str())
        .ok_or_else(|| tr("crypto.err.symbol_required_for_okx_order_status"))?;
    let order_id = obj.get("order_id").and_then(|v| v.as_str());
    let client_order_id = obj.get("client_order_id").and_then(|v| v.as_str());
    if order_id.is_none() && client_order_id.is_none() {
        return Err(tr("crypto.err.order_or_client_order_id_required"));
    }
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
    let text = format!("order_status {} {} {}", normalize_symbol(symbol), id_text, state);
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
    let text = format!("order_cancelled {}", id_text);
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
            lines.push(format!("{ccy} eq={} avail={}", fmt_num(eq), fmt_num(avail)));
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
    if !matches!(order_type.as_str(), "market" | "limit") {
        return Err(tr("crypto.err.order_type_invalid"));
    }
    let qty = obj
        .get("qty")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| tr("crypto.err.qty_required_number"))?;
    if qty <= 0.0 {
        return Err(tr("crypto.err.qty_must_gt_zero"));
    }
    let price = obj.get("price").and_then(|v| v.as_f64());
    if order_type == "limit" && price.unwrap_or(0.0) <= 0.0 {
        return Err(tr("crypto.err.price_required_for_limit"));
    }
    Ok(TradeInput {
        exchange,
        symbol,
        side,
        order_type,
        qty,
        price,
        client_order_id: obj
            .get("client_order_id")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        confirm: obj.get("confirm").and_then(|v| v.as_bool()).unwrap_or(false),
    })
}

fn risk_checks(client: &Client, cfg: &RootConfig, trade: &TradeInput, for_submit: bool) -> Result<Vec<Value>, String> {
    let mut checks = Vec::new();
    if cfg.crypto.require_explicit_send.unwrap_or(true) && for_submit && !trade.confirm {
        return Err(tr("crypto.err.trade_submit_requires_confirm"));
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
    let notional = estimate_notional_usd(client, cfg, trade)?;
    let max_notional = cfg.crypto.max_notional_usd.unwrap_or(0.0);
    if max_notional > 0.0 && notional > max_notional {
        return Err(format!(
            "notional exceeds max_notional_usd: {notional:.4} > {max_notional:.4}"
        ));
    }
    checks.push(json!({"check":"max_notional_usd","ok":true,"actual":notional,"limit":max_notional}));
    Ok(checks)
}

fn estimate_notional_usd(client: &Client, cfg: &RootConfig, trade: &TradeInput) -> Result<f64, String> {
    let price = if let Some(p) = trade.price {
        p
    } else {
        fetch_quote(client, cfg, &trade.symbol, &trade.exchange)?.price_usd
    };
    Ok((trade.qty * price).max(0.0))
}

fn submit_paper_order(
    client: &Client,
    cfg: &RootConfig,
    workspace_root: &Path,
    trade: &TradeInput,
) -> Result<OrderEvent, String> {
    let notional = estimate_notional_usd(client, cfg, trade)?;
    let order_id = format!("paper-{}", now_ts_ms());
    let status = if trade.order_type == "market" {
        "FILLED"
    } else {
        "NEW"
    };
    let event = OrderEvent {
        event: "submit".to_string(),
        order_id,
        ts: now_ts(),
        exchange: trade.exchange.clone(),
        symbol: trade.symbol.clone(),
        side: trade.side.clone(),
        order_type: trade.order_type.clone(),
        qty: trade.qty,
        price: trade.price,
        notional_usd: notional,
        status: status.to_string(),
        client_order_id: trade.client_order_id.clone(),
        reason: None,
    };
    append_paper_event(cfg, workspace_root, &event)?;
    Ok(event)
}

fn submit_binance_order(client: &Client, cfg: &RootConfig, trade: &TradeInput) -> Result<OrderEvent, String> {
    ensure_binance_config(cfg)?;
    let mut params = vec![
        ("symbol", trade.symbol.clone()),
        ("side", trade.side.to_ascii_uppercase()),
        ("type", trade.order_type.to_ascii_uppercase()),
        ("quantity", fmt_num(trade.qty)),
        ("newOrderRespType", "RESULT".to_string()),
    ];
    if trade.order_type == "limit" {
        params.push(("timeInForce", "GTC".to_string()));
        let limit_price = trade
            .price
            .ok_or_else(|| tr("crypto.err.price_required_for_limit"))?;
        params.push(("price", fmt_num(limit_price)));
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
    let notional = estimate_notional_usd(client, cfg, trade)?;
    Ok(OrderEvent {
        event: "submit".to_string(),
        order_id,
        ts: now_ts(),
        exchange: "binance".to_string(),
        symbol: trade.symbol.clone(),
        side: trade.side.clone(),
        order_type: trade.order_type.clone(),
        qty: trade.qty,
        price: trade.price,
        notional_usd: notional,
        status,
        client_order_id: trade.client_order_id.clone(),
        reason: None,
    })
}

fn submit_okx_order(client: &Client, cfg: &RootConfig, trade: &TradeInput) -> Result<OrderEvent, String> {
    ensure_okx_config(cfg)?;
    let mut body = json!({
        "instId": to_okx_inst_id(&trade.symbol),
        "tdMode": "cash",
        "side": trade.side,
        "ordType": trade.order_type,
        "sz": fmt_num(trade.qty)
    });
    if trade.order_type == "limit" {
        let limit_price = trade
            .price
            .ok_or_else(|| tr("crypto.err.price_required_for_limit"))?;
        body["px"] = Value::String(fmt_num(limit_price));
    } else if trade.order_type == "market" {
        body["tgtCcy"] = Value::String("base_ccy".to_string());
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
        qty: trade.qty,
        price: trade.price,
        notional_usd: notional,
        status,
        client_order_id: trade.client_order_id.clone(),
        reason: data
            .get("sMsg")
            .and_then(|x| x.as_str())
            .filter(|v| !v.is_empty())
            .map(str::to_string),
    })
}

fn append_paper_event(cfg: &RootConfig, workspace_root: &Path, event: &OrderEvent) -> Result<(), String> {
    let path = paper_state_path(cfg, workspace_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("create paper dir failed: {err}"))?;
    }
    let line = serde_json::to_string(event).map_err(|err| format!("serialize event failed: {err}"))?;
    let mut fp = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| format!("open paper state failed: {err}"))?;
    fp.write_all(format!("{line}\n").as_bytes())
        .map_err(|err| format!("write paper state failed: {err}"))?;
    Ok(())
}

fn paper_order_states(cfg: &RootConfig, workspace_root: &Path) -> Result<Vec<OrderState>, String> {
    let path = paper_state_path(cfg, workspace_root);
    let raw = match std::fs::read_to_string(path) {
        Ok(v) => v,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(format!("read paper state failed: {err}")),
    };
    let mut map: HashMap<String, OrderState> = HashMap::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let evt: OrderEvent = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let entry = map.entry(evt.order_id.clone()).or_insert(OrderState {
            order_id: evt.order_id.clone(),
            exchange: evt.exchange.clone(),
            symbol: evt.symbol.clone(),
            side: evt.side.clone(),
            order_type: evt.order_type.clone(),
            qty: evt.qty,
            price: evt.price,
            notional_usd: evt.notional_usd,
            status: evt.status.clone(),
            updated_ts: evt.ts,
        });
        entry.status = evt.status.clone();
        entry.updated_ts = evt.ts;
    }
    Ok(map.into_values().collect())
}

fn paper_state_path(cfg: &RootConfig, workspace_root: &Path) -> PathBuf {
    if let Some(path) = cfg.crypto.paper_state_path.as_deref() {
        let p = Path::new(path);
        if p.is_absolute() {
            return p.to_path_buf();
        }
        return workspace_root.join(p);
    }
    workspace_root.join("data/crypto-paper-orders.jsonl")
}

fn fetch_quote(client: &Client, cfg: &RootConfig, symbol_input: &str, exchange_input: &str) -> Result<Quote, String> {
    let exchange = exchange_input.trim().to_ascii_lowercase();
    let symbol = normalize_symbol(symbol_input);
    match exchange.as_str() {
        "coingecko" => fetch_quote_from_coingecko(client, &symbol),
        "okx" => fetch_quote_from_okx(client, cfg, &symbol),
        "binance" | "paper" => fetch_quote_from_binance(client, cfg, &symbol),
        _ => fetch_quote_from_binance(client, cfg, &symbol)
            .or_else(|_| fetch_quote_from_okx(client, cfg, &symbol))
            .or_else(|_| fetch_quote_from_coingecko(client, &symbol)),
    }
}

fn fetch_quote_from_binance(client: &Client, cfg: &RootConfig, symbol: &str) -> Result<Quote, String> {
    let base = cfg.binance.base_url.trim_end_matches('/');
    let url = format!("{base}/api/v3/ticker/24hr?symbol={}", normalize_symbol(symbol));
    let v: Value = client
        .get(url)
        .send()
        .map_err(|err| format!("binance quote request failed: {err}"))?
        .json()
        .map_err(|err| format!("binance quote parse failed: {err}"))?;
    let price = v
        .get("lastPrice")
        .and_then(|x| x.as_str())
        .and_then(|x| x.parse::<f64>().ok())
        .ok_or_else(|| "binance quote missing lastPrice".to_string())?;
    let change = v
        .get("priceChangePercent")
        .and_then(|x| x.as_str())
        .and_then(|x| x.parse::<f64>().ok());
    Ok(Quote {
        symbol: normalize_symbol(symbol),
        price_usd: price,
        change_24h_pct: change,
        exchange: "binance".to_string(),
        source: "binance_api".to_string(),
    })
}

fn fetch_quote_from_okx(client: &Client, cfg: &RootConfig, symbol: &str) -> Result<Quote, String> {
    let base = cfg.okx.base_url.trim_end_matches('/');
    let inst_id = to_okx_inst_id(symbol);
    let url = format!("{base}/api/v5/market/ticker?instId={}", encode(&inst_id));
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

fn fetch_quote_from_coingecko(client: &Client, symbol: &str) -> Result<Quote, String> {
    let coin_id = symbol_to_coingecko_id(symbol).ok_or_else(|| {
        "coingecko mapping missing for symbol; try exchange=binance or map this symbol".to_string()
    })?;
    let url = format!(
        "https://api.coingecko.com/api/v3/simple/price?ids={}&vs_currencies=usd&include_24hr_change=true",
        coin_id
    );
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

fn fetch_candles_binance(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
    interval: &str,
    limit: u64,
) -> Result<Vec<f64>, String> {
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
    let mut closes = Vec::new();
    for item in arr {
        if let Some(k) = item.as_array() {
            if let Some(close) = k
                .get(4)
                .and_then(|x| x.as_str())
                .and_then(|x| x.parse::<f64>().ok())
            {
                closes.push(close);
            }
        }
    }
    Ok(closes)
}

fn fetch_candles_okx(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
    interval: &str,
    limit: u64,
) -> Result<Vec<f64>, String> {
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
    let mut closes = Vec::new();
    for item in arr {
        if let Some(k) = item.as_array() {
            if let Some(close) = k
                .get(4)
                .and_then(|x| x.as_str())
                .and_then(|x| x.parse::<f64>().ok())
            {
                closes.push(close);
            }
        }
    }
    closes.reverse();
    Ok(closes)
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
    input
        .or(cfg.crypto.execution_mode.as_deref())
        .or(cfg.crypto.default_exchange.as_deref())
        .unwrap_or("paper")
        .trim()
        .to_ascii_lowercase()
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
    input
        .trim()
        .to_ascii_uppercase()
        .replace('/', "")
        .replace('-', "")
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

fn map_interval_binance(input: &str) -> &'static str {
    match input.trim().to_ascii_lowercase().as_str() {
        "1m" => "1m",
        "5m" => "5m",
        "15m" => "15m",
        "30m" => "30m",
        "1h" => "1h",
        "4h" => "4h",
        "1d" | "24h" => "1d",
        _ => "1h",
    }
}

fn map_interval_okx(input: &str) -> &'static str {
    match input.trim().to_ascii_lowercase().as_str() {
        "1m" => "1m",
        "5m" => "5m",
        "15m" => "15m",
        "30m" => "30m",
        "1h" => "1H",
        "4h" => "4H",
        "1d" | "24h" => "1D",
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
        "price": t.price,
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

fn default_recv_window() -> u64 {
    5000
}

fn default_okx_simulated() -> bool {
    true
}

fn extract_blocks<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut pos = 0usize;
    while let Some(start_rel) = xml[pos..].find(&open) {
        let start = pos + start_rel;
        let gt_rel = match xml[start..].find('>') {
            Some(v) => v,
            None => break,
        };
        let body_start = start + gt_rel + 1;
        let end_rel = match xml[body_start..].find(&close) {
            Some(v) => v,
            None => break,
        };
        let end = body_start + end_rel;
        out.push(&xml[body_start..end]);
        pos = end + close.len();
    }
    out
}

fn extract_tag_text(xml: &str, tag: &str) -> Option<String> {
    extract_blocks(xml, tag)
        .into_iter()
        .next()
        .map(|v| xml_unescape(v.trim()))
}

fn extract_atom_link(xml: &str) -> Option<String> {
    let mut pos = 0usize;
    while let Some(start_rel) = xml[pos..].find("<link") {
        let start = pos + start_rel;
        let end_rel = xml[start..].find('>')?;
        let head = &xml[start..start + end_rel + 1];
        if let Some(href) = extract_attr(head, "href") {
            if !href.is_empty() {
                return Some(xml_unescape(&href));
            }
        }
        pos = start + end_rel + 1;
    }
    None
}

fn extract_attr(tag_text: &str, attr: &str) -> Option<String> {
    let key = format!("{attr}=\"");
    let start = tag_text.find(&key)? + key.len();
    let rem = &tag_text[start..];
    let end = rem.find('"')?;
    Some(rem[..end].to_string())
}

fn xml_unescape(input: &str) -> String {
    input
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_symbol_ok() {
        assert_eq!(normalize_symbol("btc/usdt"), "BTCUSDT");
        assert_eq!(normalize_symbol("eth-usd"), "ETHUSD");
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
}
