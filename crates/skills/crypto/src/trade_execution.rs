use super::*;

pub(super) fn parse_trade_input(
    obj: &serde_json::Map<String, Value>,
    cfg: &RootConfig,
) -> Result<TradeInput, String> {
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg)?;
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
    let order_type = trade_order_type_value(obj)
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
    let qty_value = trade_qty_value(obj);
    let qty_all = qty_value
        .and_then(|v| v.as_str())
        .map(|s| {
            let n = s.trim().to_ascii_lowercase();
            matches!(n.as_str(), "all" | "max" | "全部" | "全仓")
        })
        .unwrap_or(false);
    let mut qty = qty_value.and_then(value_to_f64).unwrap_or(0.0);
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
    let price = obj.get("price").and_then(value_to_f64);
    let stop_price = obj
        .get("stop_price")
        .and_then(value_to_f64)
        .or_else(|| obj.get("stopPrice").and_then(value_to_f64));
    let time_in_force = obj
        .get("time_in_force")
        .or_else(|| obj.get("timeInForce"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_ascii_uppercase());
    if matches!(order_type.as_str(), "limit" | "limit_maker") && price.unwrap_or(0.0) <= 0.0 {
        return Err(tr("crypto.err.price_required_for_limit"));
    }
    if matches!(order_type.as_str(), "stop_loss_limit" | "take_profit_limit") {
        if price.unwrap_or(0.0) <= 0.0 {
            return Err(
                "stop_loss_limit/take_profit_limit requires price (limit price)".to_string(),
            );
        }
        if stop_price.unwrap_or(0.0) <= 0.0 {
            return Err(
                "stop_loss_limit/take_profit_limit requires stop_price (trigger price)".to_string(),
            );
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
        confirm: obj
            .get("confirm")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

pub(super) fn trade_order_type_value<'a>(
    obj: &'a serde_json::Map<String, Value>,
) -> Option<&'a Value> {
    obj.get("order_type")
        .or_else(|| obj.get("orderType"))
        .or_else(|| obj.get("type"))
}

pub(super) fn trade_qty_value<'a>(obj: &'a serde_json::Map<String, Value>) -> Option<&'a Value> {
    obj.get("qty")
        .or_else(|| obj.get("quantity"))
        .or_else(|| obj.get("amount"))
        .or_else(|| obj.get("base_qty"))
        .or_else(|| obj.get("base_quantity"))
}

pub(super) fn risk_checks(
    client: &Client,
    cfg: &RootConfig,
    trade: &TradeInput,
    _for_submit: bool,
) -> Result<Vec<Value>, String> {
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
    checks
        .push(json!({"check":"max_notional_usd","ok":true,"actual":notional,"limit":max_notional}));
    // Binance spot minimum notional is typically 5~10 USDT; warn if below 1 USDT
    let min_notional = cfg.crypto.min_notional_usd.unwrap_or(1.0);
    if trade.exchange == "binance" && notional < min_notional && notional > 0.0 {
        return Err(format!(
            "notional too small: {notional:.4} < min_notional_usd={min_notional:.2} (Binance spot requires at least ~10 USDT)"
        ));
    }
    if min_notional > 0.0 && notional > 0.0 && notional < min_notional {
        checks.push(
            json!({"check":"min_notional_usd","ok":false,"actual":notional,"limit":min_notional}),
        );
    } else {
        checks.push(
            json!({"check":"min_notional_usd","ok":true,"actual":notional,"limit":min_notional}),
        );
    }
    Ok(checks)
}

pub(super) fn estimate_notional_usd(
    client: &Client,
    cfg: &RootConfig,
    trade: &TradeInput,
) -> Result<f64, String> {
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

pub(super) fn resolve_base_qty(
    client: &Client,
    cfg: &RootConfig,
    trade: &TradeInput,
) -> Result<f64, String> {
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

pub(super) fn resolve_all_sell_qty(
    client: &Client,
    cfg: &RootConfig,
    trade: &TradeInput,
) -> Result<f64, String> {
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
            let v =
                binance_signed_request(client, cfg, Method::GET, "/api/v3/account", &mut params)?;
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
                return Err(format!(
                    "no available balance for {} on binance",
                    base_asset
                ));
            }
            Ok(free)
        }
        "okx" => {
            let v = okx_request(
                client,
                cfg,
                Method::GET,
                "/api/v5/account/balance",
                None,
                None,
            )?;
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
        _ => Err(format!(
            "qty=all is unsupported exchange: {}",
            trade.exchange
        )),
    }
}

pub(super) fn adjust_qty_to_step_floor(qty: f64, step: f64) -> f64 {
    if qty <= 0.0 || step <= 0.0 {
        return qty.max(0.0);
    }
    let units = (qty / step).floor();
    (units * step).max(0.0)
}

pub(super) fn fetch_binance_lot_size_filter(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
) -> Result<(f64, f64, f64), String> {
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
pub(super) fn fetch_binance_price_filter(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
) -> Result<(f64, f64, f64), String> {
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
    let min_price = price_filter
        .get("minPrice")
        .and_then(value_to_f64)
        .unwrap_or(0.0);
    let max_price = price_filter
        .get("maxPrice")
        .and_then(value_to_f64)
        .unwrap_or(0.0);
    Ok((tick_size, min_price, max_price))
}

/// Round price to Binance tickSize (price must satisfy price % tickSize == 0).
pub(super) fn adjust_price_to_tick(price: f64, tick_size: f64) -> f64 {
    if tick_size <= 0.0 {
        return price;
    }
    (price / tick_size).round() * tick_size
}

/// Format price for Binance API with decimal places matching tickSize (avoids PRICE_FILTER/PERCENT_PRICE format issues).
pub(super) fn fmt_price_for_binance(price: f64, tick_size: f64) -> String {
    if tick_size <= 0.0 {
        return fmt_num(price);
    }
    let tick_str = format!("{:.10}", tick_size)
        .trim_end_matches('0')
        .to_string();
    let decimals = tick_str
        .find('.')
        .map(|i| (tick_str.len().saturating_sub(i).saturating_sub(1)).min(8))
        .unwrap_or(0);
    format!("{:.prec$}", price, prec = decimals)
}

pub(super) fn normalize_binance_order_qty(
    client: &Client,
    cfg: &RootConfig,
    symbol: &str,
    raw_qty: f64,
) -> Result<f64, String> {
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

pub(super) fn effective_order_qty_for_preview(
    client: &Client,
    cfg: &RootConfig,
    trade: &TradeInput,
) -> Result<f64, String> {
    let base_qty = resolve_base_qty(client, cfg, trade)?;
    if trade.exchange == "binance" {
        let use_quote_order_qty = trade.order_type == "market" && trade.quote_qty_usd.is_some();
        if !use_quote_order_qty {
            return normalize_binance_order_qty(client, cfg, &trade.symbol, base_qty);
        }
    }
    Ok(base_qty)
}

pub(super) fn submit_binance_order(
    client: &Client,
    cfg: &RootConfig,
    trade: &TradeInput,
) -> Result<OrderEvent, String> {
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
    if matches!(
        trade.order_type.as_str(),
        "limit" | "stop_loss_limit" | "take_profit_limit"
    ) {
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
            Some(Ok((tick, _, _))) => {
                fmt_price_for_binance(adjust_price_to_tick(limit_price, *tick), *tick)
            }
            _ => fmt_num(limit_price),
        };
        params.push(("price", price_str));
    }
    if trade.order_type == "limit_maker" {
        let limit_price = trade
            .price
            .ok_or_else(|| tr("crypto.err.price_required_for_limit"))?;
        let price_str = match &price_filter_opt {
            Some(Ok((tick, _, _))) => {
                fmt_price_for_binance(adjust_price_to_tick(limit_price, *tick), *tick)
            }
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
        .or_else(|| {
            v.get("orderId")
                .and_then(|x| x.as_str())
                .map(str::to_string)
        })
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

pub(super) fn submit_okx_order(
    client: &Client,
    cfg: &RootConfig,
    trade: &TradeInput,
) -> Result<OrderEvent, String> {
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
    let v = okx_request(
        client,
        cfg,
        Method::POST,
        "/api/v5/trade/order",
        None,
        Some(body),
    )?;
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
            data.get("sMsg")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown")
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
