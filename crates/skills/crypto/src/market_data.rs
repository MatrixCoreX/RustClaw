use super::*;

pub(super) fn fetch_quote(
    client: &Client,
    cfg: &RootConfig,
    symbol_input: &str,
    exchange_input: &str,
) -> Result<Quote, String> {
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

pub(super) fn value_to_f64(v: &Value) -> Option<f64> {
    if let Some(n) = v.as_f64() {
        return Some(n);
    }
    v.as_str().and_then(|s| s.parse::<f64>().ok())
}

pub(super) fn number_field(obj: &Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        if let Some(v) = obj.get(*key).and_then(value_to_f64) {
            return Some(v);
        }
    }
    None
}

pub(super) fn render_url_template(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        let encoded = encode(v).into_owned();
        out = out.replace(&format!("{{{k}}}"), &encoded);
    }
    out
}

pub(super) fn build_exchange_url(base: &str, path_or_url: &str, vars: &[(&str, &str)]) -> String {
    let rendered = render_url_template(path_or_url, vars);
    if rendered.starts_with("http://") || rendered.starts_with("https://") {
        return rendered;
    }
    let b = base.trim_end_matches('/');
    let p = rendered.trim_start_matches('/');
    format!("{b}/{p}")
}

pub(super) fn parse_evm_api_result_string(v: &Value) -> Option<String> {
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

pub(super) fn parse_evm_tx_list(v: &Value, address: &str, limit: usize) -> Vec<Value> {
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
        let from = it
            .get("from")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let to = it
            .get("to")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let hash = it
            .get("hash")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
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

pub(super) fn raw_to_decimal_string(raw: &str, decimals: u32) -> String {
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
        let frac = format!("{:0>width$}", digits, width = d)
            .trim_end_matches('0')
            .to_string();
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

pub(super) fn fetch_quote_from_binance(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
) -> Result<Quote, String> {
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
    let change = v.get("priceChangePercent").and_then(value_to_f64);
    Ok(Quote {
        symbol: normalized_symbol,
        price_usd: price,
        change_24h_pct: change,
        exchange: "binance".to_string(),
        source: "binance_api".to_string(),
    })
}

pub(super) fn ensure_symbol_supported_on_binance(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
) -> Result<(), String> {
    let base = cfg.binance.base_url.trim_end_matches('/');
    let normalized_symbol = normalize_symbol(symbol);
    let url = format!(
        "{base}/api/v3/exchangeInfo?symbol={}",
        encode(&normalized_symbol)
    );
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

pub(super) fn fetch_quote_from_okx(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
) -> Result<Quote, String> {
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

pub(super) fn fetch_quote_from_coingecko(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
) -> Result<Quote, String> {
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

pub(super) fn fetch_quote_from_gateio(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
) -> Result<Quote, String> {
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

pub(super) fn fetch_quote_from_coinbase(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
) -> Result<Quote, String> {
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

pub(super) fn fetch_quote_from_kraken(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
) -> Result<Quote, String> {
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

pub(super) fn fetch_book_ticker_from_binance(
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

pub(super) fn fetch_book_ticker_from_okx(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
) -> Result<BookTicker, String> {
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

pub(super) fn fetch_book_ticker_from_gateio(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
) -> Result<BookTicker, String> {
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
    let ask_qty = row.get("lowest_size").and_then(value_to_f64).unwrap_or(0.0);
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

pub(super) fn fetch_book_ticker_from_coinbase(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
) -> Result<BookTicker, String> {
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

pub(super) fn fetch_book_ticker_from_kraken(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
) -> Result<BookTicker, String> {
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
pub(super) struct CandleOhlcv {
    pub(super) open: f64,
    pub(super) high: f64,
    pub(super) low: f64,
    pub(super) close: f64,
    pub(super) volume: f64,
    pub(super) quote_volume: f64,
}

pub(super) fn fetch_candles_binance(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
    interval: &str,
    limit: u64,
) -> Result<Vec<f64>, String> {
    Ok(
        fetch_candles_ohlcv_binance(client, cfg, symbol, interval, limit)?
            .into_iter()
            .map(|c| c.close)
            .collect(),
    )
}

pub(super) fn fetch_candles_ohlcv_binance(
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

pub(super) fn fetch_candles_okx(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
    interval: &str,
    limit: u64,
) -> Result<Vec<f64>, String> {
    Ok(
        fetch_candles_ohlcv_okx(client, cfg, symbol, interval, limit)?
            .into_iter()
            .map(|c| c.close)
            .collect(),
    )
}

pub(super) fn fetch_candles_ohlcv_okx(
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
