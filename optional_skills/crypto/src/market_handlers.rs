use super::*;

pub(super) fn handle_quote(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
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
    let extra = market_quote_extra(
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
        &text,
    );
    Ok((text, extra))
}

pub(super) fn handle_multi_quote(
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
            return Err(format!(
                "quote failed on all sources for symbol={}",
                normalize_symbol(&s)
            ));
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
    let text = lines.join("\n");
    Ok((text.clone(), market_quote_extra(extra, &text)))
}

pub(super) fn market_quote_extra(mut extra: Value, text: &str) -> Value {
    if let Some(obj) = extra.as_object_mut() {
        obj.insert("content_excerpt".to_string(), json!(text));
    }
    extra
}

pub(super) fn format_market_quote_line(
    symbol: &str,
    binance: Option<&Quote>,
    okx: Option<&Quote>,
    gateio: Option<&Quote>,
    coinbase: Option<&Quote>,
    kraken: Option<&Quote>,
    coingecko: Option<&Quote>,
) -> String {
    let mut lines = Vec::new();
    lines.push(tr_with(
        "crypto.msg.market_quote_header",
        &[("symbol", symbol)],
    ));
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

pub(super) fn handle_book_ticker(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let symbol = obj
        .get("symbol")
        .and_then(|v| v.as_str())
        .ok_or_else(|| tr("crypto.err.symbol_required"))?;
    let exchange_input = obj
        .get("exchange")
        .and_then(|v| v.as_str())
        .unwrap_or("dual");
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
        let text = format_book_ticker_sources(
            &s,
            &[
                ("binance", b.as_ref()),
                ("okx", o.as_ref()),
                ("gateio", g.as_ref()),
                ("coinbase", cb.as_ref()),
                ("kraken", k.as_ref()),
            ],
        );
        return Ok((
            text.clone(),
            json!({
                "action":"book_ticker",
                "message_key":"crypto.msg.book_ticker_sources",
                "content_excerpt": text,
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
    let exchange = resolve_exchange(Some(exchange_input), cfg)?;
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

pub(super) fn format_book_ticker_sources(
    symbol: &str,
    entries: &[(&str, Option<&BookTicker>)],
) -> String {
    let mut lines = vec![format!("book_ticker_sources symbol={symbol}")];
    for (source, ticker) in entries {
        let Some(ticker) = ticker else {
            continue;
        };
        lines.push(format!(
            "source={} bid={} ask={}",
            source,
            fmt_num(ticker.bid_price),
            fmt_num(ticker.ask_price)
        ));
    }
    lines.join("\n")
}

pub(super) fn handle_normalize_symbol(
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let symbol_raw = obj
        .get("symbol")
        .and_then(|v| v.as_str())
        .ok_or_else(|| tr("crypto.err.symbol_required"))?;
    let binance_symbol = normalize_symbol(symbol_raw);
    let okx_symbol = to_okx_inst_id(symbol_raw);
    let text = format!(
        "symbol={} -> binance={} okx={}",
        symbol_raw, binance_symbol, okx_symbol
    );
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

pub(super) fn handle_healthcheck(
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
    let ok = checks
        .iter()
        .all(|x| x.get("ok").and_then(|v| v.as_bool()) == Some(true));
    let text = if ok {
        "crypto healthcheck ok (binance+okx)".to_string()
    } else {
        "crypto healthcheck degraded".to_string()
    };
    Ok((
        text,
        json!({"action":"healthcheck","ok":ok,"checks":checks}),
    ))
}

pub(super) fn handle_candles(
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
        .or_else(|| obj.get("interval"))
        .and_then(|v| v.as_str())
        .unwrap_or("1h");
    let limit = obj
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(30)
        .clamp(1, 500);
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg)?;
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
    let high = candles
        .iter()
        .map(|c| c.high)
        .fold(f64::NEG_INFINITY, f64::max);
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
            symbol,
            interval,
            last,
            delta,
            high,
            low,
            total_volume,
            candles.len()
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

pub(super) fn calc_sma(values: &[f64], period: usize) -> Option<f64> {
    if values.len() < period {
        return None;
    }
    let tail = &values[values.len() - period..];
    Some(tail.iter().sum::<f64>() / period as f64)
}

pub(super) fn calc_ema(values: &[f64], period: usize) -> Option<f64> {
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

pub(super) fn calc_rsi(values: &[f64], period: usize) -> Option<f64> {
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

pub(super) fn handle_indicator(
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
            let raw_signal = if rsi >= 70.0 {
                "overbought"
            } else if rsi <= 30.0 {
                "oversold"
            } else {
                "neutral"
            };
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
            let raw_signal = if last >= ema {
                "above_ema"
            } else {
                "below_ema"
            };
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
            let raw_signal = if last >= sma {
                "above_sma"
            } else {
                "below_sma"
            };
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

pub(super) fn handle_binance_symbol_check(
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

pub(super) fn schedule_invocation_extra_fields(
    ctx: &SkillContext,
    obj: &serde_json::Map<String, Value>,
) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    let job_id = ctx
        .schedule_job_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            obj.get("schedule_job_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        });
    if let Some(s) = job_id {
        m.insert("schedule_job_id".to_string(), json!(s));
    }
    let src = ctx
        .invocation_source
        .as_deref()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            obj.get("invocation_source")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        });
    if let Some(s) = src {
        m.insert("invocation_source".to_string(), json!(s));
    }
    if let Some(b) = ctx
        .scheduled
        .or_else(|| obj.get("scheduled").and_then(|v| v.as_bool()))
    {
        m.insert("scheduled".to_string(), json!(b));
    }
    if let Some(b) = ctx
        .schedule_triggered
        .or_else(|| obj.get("schedule_triggered").and_then(|v| v.as_bool()))
    {
        m.insert("schedule_triggered".to_string(), json!(b));
    }
    m
}

pub(super) fn handle_price_alert_check(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
    ctx: &SkillContext,
) -> Result<(String, Value), String> {
    let symbol = normalize_symbol(
        obj.get("symbol")
            .and_then(|v| v.as_str())
            .ok_or_else(|| tr("crypto.err.symbol_required"))?,
    );
    let window_minutes = resolve_price_alert_window_minutes(obj, cfg);
    let threshold_pct = resolve_price_alert_threshold_pct(obj, cfg);
    if threshold_pct <= 0.0 {
        return Err(tr("crypto.err.threshold_pct_must_gt_zero"));
    }
    let direction = resolve_price_alert_direction_normalized(obj);
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg)?;
    preflight_price_alert_symbol_listing(client, cfg, &exchange, &symbol)?;
    // Lookback window: `window_minutes` of 1m candles; first close ≈ price at window start, last close = latest.
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
    let reference_text = format!("{:.6}", start_price);
    let current_text = format!("{:.6}", current_price);
    let text_body = if triggered {
        tr_with(
            "crypto.msg.price_alert_triggered",
            &[
                ("symbol", &symbol),
                ("window_minutes", &window_minutes.to_string()),
                ("change_pct", &change_text),
                ("threshold_pct", &threshold_text),
                ("reference_price", &reference_text),
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
                ("reference_price", &reference_text),
                ("current_price", &current_text),
                ("direction", direction),
            ],
        )
    };
    let mut extra = serde_json::Map::new();
    extra.insert("action".to_string(), json!("price_alert_check"));
    extra.insert("symbol".to_string(), json!(symbol));
    extra.insert("exchange".to_string(), json!(exchange));
    extra.insert("window_minutes".to_string(), json!(window_minutes));
    extra.insert("threshold_pct".to_string(), json!(threshold_pct));
    extra.insert("direction".to_string(), json!(direction));
    extra.insert("triggered".to_string(), json!(triggered));
    extra.insert("trend".to_string(), json!(trend));
    extra.insert("start_price".to_string(), json!(start_price));
    extra.insert("reference_price".to_string(), json!(start_price));
    extra.insert("current_price".to_string(), json!(current_price));
    extra.insert("change_pct".to_string(), json!(change_pct));
    extra.insert("candles".to_string(), json!(closes.len()));
    extra.insert("notify".to_string(), json!(triggered));
    for (k, v) in schedule_invocation_extra_fields(ctx, obj) {
        extra.insert(k, v);
    }
    Ok((text_body, Value::Object(extra)))
}
