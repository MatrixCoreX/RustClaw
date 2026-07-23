use super::*;

pub(super) static I18N: OnceLock<HashMap<String, TextCatalog>> = OnceLock::new();
thread_local! {
    static CURRENT_LANG: RefCell<String> = RefCell::new("zh-CN".to_string());
}

#[derive(Debug, Clone)]
pub(super) struct TextCatalog {
    current: HashMap<String, String>,
}

pub(super) fn normalize_lang_tag(raw: &str) -> String {
    let lower = raw.trim().to_ascii_lowercase();
    match lower.as_str() {
        "en" | "en-us" | "en_us" | "english" => "en-US".to_string(),
        "zh" | "zh-cn" | "zh_cn" | "cn" | "zh-hans" | "chinese" => "zh-CN".to_string(),
        _ => {
            if lower.starts_with("en") {
                "en-US".to_string()
            } else {
                "zh-CN".to_string()
            }
        }
    }
}

pub(super) fn current_lang() -> String {
    CURRENT_LANG.with(|lang| lang.borrow().clone())
}

pub(super) fn set_current_lang(lang: &str) {
    let normalized = normalize_lang_tag(lang);
    CURRENT_LANG.with(|slot| {
        *slot.borrow_mut() = normalized;
    });
}

pub(super) fn tr(key: &str) -> String {
    let lang = current_lang();
    I18N.get()
        .and_then(|catalogs| {
            catalogs
                .get(&lang)
                .or_else(|| catalogs.get("zh-CN"))
                .or_else(|| catalogs.get("en-US"))
        })
        .and_then(|catalog| catalog.current.get(key))
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

pub(super) fn tr_with(key: &str, vars: &[(&str, &str)]) -> String {
    let mut out = tr(key);
    for (name, value) in vars {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

pub(super) fn i18n_lang(cfg: &RootConfig) -> String {
    normalize_lang_tag(cfg.crypto.language.as_deref().unwrap_or("zh-CN"))
}

pub(super) fn resolve_i18n_lang(
    args: &serde_json::Map<String, Value>,
    context: &SkillContext,
    cfg: &RootConfig,
) -> String {
    for key in ["locale", "language", "lang"] {
        if let Some(lang) = args
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            return normalize_lang_tag(lang);
        }
    }
    for lang in [
        context.locale.as_deref(),
        context.language.as_deref(),
        context.lang.as_deref(),
    ] {
        if let Some(value) = lang.map(str::trim).filter(|v| !v.is_empty()) {
            return normalize_lang_tag(value);
        }
    }
    i18n_lang(cfg)
}

pub(super) fn default_crypto_catalog(lang: &str) -> TextCatalog {
    let mut current = HashMap::new();
    let _ = lang;
    current.insert(
        "crypto.err.invalid_input".to_string(),
        "invalid input: {error}".to_string(),
    );
    current.insert(
        "crypto.err.args_object".to_string(),
        "args must be object".to_string(),
    );
    current.insert(
        "crypto.err.action_blocked".to_string(),
        "action is blocked by config: {action}".to_string(),
    );
    current.insert(
        "crypto.err.build_http_client".to_string(),
        "build http client failed: {error}".to_string(),
    );
    current.insert(
        "crypto.err.unsupported_action".to_string(),
        "unsupported action".to_string(),
    );
    current.insert(
        "crypto.err.symbol_required".to_string(),
        "symbol is required".to_string(),
    );
    current.insert(
        "crypto.err.symbols_required".to_string(),
        "symbols or symbol is required".to_string(),
    );
    current.insert(
        "crypto.err.symbols_empty".to_string(),
        "symbols is empty".to_string(),
    );
    current.insert(
        "crypto.err.no_candles".to_string(),
        "no candles returned".to_string(),
    );
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
    current.insert(
        "crypto.err.exchange_required_when_no_default".to_string(),
        "exchange is required because no default exchange is configured".to_string(),
    );
    current.insert(
        "crypto.err.order_not_found".to_string(),
        "order not found: {order_id}".to_string(),
    );
    current.insert(
        "crypto.err.order_id_required".to_string(),
        "order_id is required".to_string(),
    );
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
    current.insert(
        "crypto.err.side_invalid".to_string(),
        "side must be buy or sell".to_string(),
    );
    current.insert(
        "crypto.err.order_type_invalid".to_string(),
        "order_type must be market or limit".to_string(),
    );
    current.insert(
        "crypto.err.qty_required_number".to_string(),
        "qty is required and must be number".to_string(),
    );
    current.insert(
        "crypto.err.qty_must_gt_zero".to_string(),
        "qty must be > 0".to_string(),
    );
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
        "Binance API is not bound for the current key yet. Exchange credentials are stored per user_key in the local database, not by directly editing configs/crypto.toml.\nTo avoid sending secrets through normal chat, use Telegram: /cryptoapi set binance <api_key> <api_secret>\nor POST /v1/auth/crypto-credentials.\nIf you want, I can format the exact command for you."
            .to_string(),
    );
    current.insert(
        "crypto.err.binance_credentials_incomplete".to_string(),
        "Binance API credentials are incomplete for the current key. Exchange credentials are stored per user_key in the local database, not by directly editing configs/crypto.toml.\nTo avoid sending secrets through normal chat, use Telegram: /cryptoapi set binance <api_key> <api_secret>\nor POST /v1/auth/crypto-credentials.\nIf you want, I can format the exact command for you."
            .to_string(),
    );
    current.insert(
        "crypto.err.okx_not_bound".to_string(),
        "OKX API is not bound for the current key yet. Exchange credentials are stored per user_key in the local database, not by directly editing configs/crypto.toml.\nTo avoid sending secrets through normal chat, use Telegram: /cryptoapi set okx <api_key> <api_secret> <passphrase>\nor POST /v1/auth/crypto-credentials.\nIf you want, I can format the exact command for you."
            .to_string(),
    );
    current.insert(
        "crypto.err.okx_credentials_incomplete".to_string(),
        "OKX API credentials are incomplete for the current key. Exchange credentials are stored per user_key in the local database, not by directly editing configs/crypto.toml.\nTo avoid sending secrets through normal chat, use Telegram: /cryptoapi set okx <api_key> <api_secret> <passphrase>\nor POST /v1/auth/crypto-credentials.\nIf you want, I can format the exact command for you."
            .to_string(),
    );
    current.insert(
        "crypto.msg.no_orders_yet".to_string(),
        "no orders yet".to_string(),
    );
    current.insert(
        "crypto.msg.no_filled_positions".to_string(),
        "no filled positions".to_string(),
    );
    current.insert(
        "crypto.msg.no_balances".to_string(),
        "no balances".to_string(),
    );
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
        "ALERT {symbol}: {window_minutes}m lookback change {change_pct}% reached threshold {threshold_pct}%. Reference/base price: {reference_price}. Current price: {current_price}. Direction: {direction}.".to_string(),
    );
    current.insert(
        "crypto.msg.price_alert_not_triggered".to_string(),
        "{symbol} monitor: {window_minutes}m lookback change {change_pct}% below threshold {threshold_pct}%. Reference/base price: {reference_price}. Current price: {current_price}. Direction: {direction}.".to_string(),
    );
    current.insert(
        "crypto.msg.trade_submitted_pending_suffix".to_string(),
        " (order placed, awaiting fill)".to_string(),
    );
    current.insert("crypto.msg.trade_preview_summary".to_string(), "trade_preview {exchange} {symbol} {side} {qty_part} notional_usd={notional} checks={checks}".to_string());
    current.insert("crypto.msg.trade_submitted_filled".to_string(), "trade_submitted order_id={order_id} status=FILLED {exchange} {symbol} {side} qty_filled={qty_filled}{price_part} quote_spent={quote_spent} USDT".to_string());
    current.insert("crypto.msg.trade_submitted_partial".to_string(), "trade_submitted order_id={order_id} status=PARTIAL {exchange} {symbol} {side} filled={filled}/{total} notional_usd={notional}".to_string());
    current.insert("crypto.msg.trade_submitted_fallback".to_string(), "trade_submitted order_id={order_id} status={status} {exchange} {symbol} {side} qty={qty} notional_usd={notional}".to_string());
    current.insert(
        "crypto.msg.open_orders_none".to_string(),
        "open_orders {exchange}{symbol_suffix}: none".to_string(),
    );
    current.insert(
        "crypto.msg.open_orders_header".to_string(),
        "open_orders {exchange} count={count}\n{body}".to_string(),
    );
    current.insert("crypto.msg.open_orders_line_binance".to_string(), "{sym} {side} {otype} qty={orig_qty} filled={exec_qty} price={price} status={status} id={oid}".to_string());
    current.insert(
        "crypto.msg.open_orders_line_okx".to_string(),
        "{inst} {side} {otype} sz={sz} price={px} state={state} id={oid}".to_string(),
    );
    current.insert(
        "crypto.msg.cancel_all_orders_done".to_string(),
        "cancel_all_orders {exchange} {sym} cancelled={count}".to_string(),
    );
    current.insert(
        "crypto.msg.cancel_all_orders_no_open_orders".to_string(),
        "cancel_all_orders {exchange} {sym_info}: no open orders".to_string(),
    );
    current.insert(
        "crypto.msg.cancel_order_done".to_string(),
        "order_cancelled {id_text}".to_string(),
    );
    current.insert(
        "crypto.msg.indicator_rsi_summary".to_string(),
        "{symbol} RSI{period}={rsi} last={last} signal={signal}".to_string(),
    );
    current.insert(
        "crypto.msg.indicator_ema_summary".to_string(),
        "{symbol} EMA{period}={ema} last={last} signal={signal}".to_string(),
    );
    current.insert(
        "crypto.msg.indicator_sma_summary".to_string(),
        "{symbol} SMA{period}={sma} last={last} signal={signal}".to_string(),
    );
    current.insert(
        "crypto.msg.indicator_signal_neutral".to_string(),
        "neutral".to_string(),
    );
    current.insert(
        "crypto.msg.indicator_signal_overbought".to_string(),
        "overbought".to_string(),
    );
    current.insert(
        "crypto.msg.indicator_signal_oversold".to_string(),
        "oversold".to_string(),
    );
    current.insert(
        "crypto.msg.indicator_signal_above_ema".to_string(),
        "above_ema".to_string(),
    );
    current.insert(
        "crypto.msg.indicator_signal_below_ema".to_string(),
        "below_ema".to_string(),
    );
    current.insert(
        "crypto.msg.indicator_signal_above_sma".to_string(),
        "above_sma".to_string(),
    );
    current.insert(
        "crypto.msg.indicator_signal_below_sma".to_string(),
        "below_sma".to_string(),
    );
    current.insert("crypto.msg.trade_submitted_pending".to_string(), "Order placed (pending): order_id={order_id}, status=PENDING, {exchange} {symbol} {side} {order_type} qty={qty}{price_str}{stop_str} notional_usd={notional}{pending_suffix}".to_string());
    current.insert(
        "crypto.msg.positions_balance_line".to_string(),
        "{asset} free={free} locked={locked}".to_string(),
    );
    current.insert(
        "crypto.msg.positions_balance_line_okx".to_string(),
        "{ccy} eq={eq} avail={avail}".to_string(),
    );
    current.insert(
        "crypto.msg.order_status_summary".to_string(),
        "Order status: {symbol} id={id_text} status={status}".to_string(),
    );
    current.insert(
        "crypto.msg.order_status_skipped_missing_symbol".to_string(),
        "Order status skipped ({id_text}): missing symbol, {exchange} requires symbol for query"
            .to_string(),
    );
    current.insert(
        "crypto.msg.onchain_btc_fees".to_string(),
        "BTC fee(sat/vB): fastest={fastest}, half_hour={half_hour}, hour={hour}".to_string(),
    );
    current.insert("crypto.msg.onchain_eth_stats_summary".to_string(), "ETH onchain: tx_24h={tx_24h}, blocks_24h={blocks_24h}, market_price_usd={market_price_usd}".to_string());
    current.insert(
        "crypto.msg.onchain_eth_native_summary".to_string(),
        "ETH address={address} token=ETH balance={balance} recent_txs={recent_txs}".to_string(),
    );
    current.insert(
        "crypto.msg.onchain_eth_token_summary".to_string(),
        "ETH address={address} token={token} balance={balance} recent_txs={recent_txs}".to_string(),
    );
    TextCatalog { current }
}

pub(super) fn flatten_toml_table(
    prefix: &str,
    table: &toml::map::Map<String, toml::Value>,
    out: &mut HashMap<String, String>,
) {
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

pub(super) fn load_external_i18n(path: &Path) -> Option<HashMap<String, String>> {
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

pub(super) fn catalog_path_for_lang(
    cfg: &RootConfig,
    workspace_root: &Path,
    lang: &str,
) -> PathBuf {
    let configured_lang = i18n_lang(cfg);
    if configured_lang == lang {
        if let Some(path) = cfg.crypto.i18n_path.as_deref() {
            return workspace_root.join(path);
        }
    }
    workspace_root.join(format!("configs/i18n/crypto.{lang}.toml"))
}

pub(super) fn build_catalog_for_lang(
    cfg: &RootConfig,
    workspace_root: &Path,
    lang: &str,
) -> TextCatalog {
    let mut catalog = default_crypto_catalog(lang);
    let path = catalog_path_for_lang(cfg, workspace_root, lang);
    if let Some(override_dict) = load_external_i18n(&path) {
        for (k, v) in override_dict {
            catalog.current.insert(k, v);
        }
    }
    catalog
}

pub(super) fn init_i18n(cfg: &RootConfig, workspace_root: &Path) {
    let default_lang = i18n_lang(cfg);
    let mut catalogs = HashMap::new();
    for lang in [
        default_lang.clone(),
        "zh-CN".to_string(),
        "en-US".to_string(),
    ] {
        catalogs
            .entry(lang.clone())
            .or_insert_with(|| build_catalog_for_lang(cfg, workspace_root, &lang));
    }
    let _ = I18N.set(catalogs);
    set_current_lang(&default_lang);
}
