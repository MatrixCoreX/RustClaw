use super::*;

pub(super) fn handle_trade_preview(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let trade = parse_trade_input(obj, cfg)?;
    let checks = risk_checks(client, cfg, &trade, false)?;
    let preview_qty = effective_order_qty_for_preview(client, cfg, &trade)?;
    let notional = estimate_notional_usd(
        client,
        cfg,
        &TradeInput {
            qty: preview_qty,
            ..trade.clone()
        },
    )?;
    // When user specified a USDT spend amount, Binance uses quoteOrderQty and fills
    // the actual coin qty at market; the displayed qty is only an estimate.
    let (qty_label, quote_part) = if let Some(q) = trade.quote_qty_usd {
        ("est_qty", format!(" quote_usd={:.4}", q))
    } else {
        ("qty", String::new())
    };
    // Include order_type and price in the summary line for human-readable preview output.
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

pub(super) fn handle_trade_submit(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let trade = parse_trade_input(obj, cfg)?;
    let checks = risk_checks(client, cfg, &trade, true)?;
    let event = match trade.exchange.as_str() {
        "binance" => submit_binance_order(client, cfg, &trade)?,
        "okx" => submit_okx_order(client, cfg, &trade)?,
        other => {
            return Err(tr_with(
                "crypto.err.unsupported_execution_exchange",
                &[("exchange", other)],
            ))
        }
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

pub(super) fn build_trade_submitted_text(event: &OrderEvent) -> String {
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
            let price_part = event
                .avg_fill_price
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
            let price_str = event
                .price
                .map(|p| format!(" price={}", fmt_num(p)))
                .unwrap_or_default();
            let stop_str = event
                .reason
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
            let filled_str = event
                .executed_qty
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
        ),
    }
}

pub(super) fn handle_order_status(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg)?;
    match exchange.as_str() {
        "binance" => handle_order_status_binance(client, cfg, obj),
        "okx" => handle_order_status_okx(client, cfg, obj),
        _ => Err(tr_with(
            "crypto.err.unsupported_exchange_for_order_status",
            &[("exchange", &exchange)],
        )),
    }
}

pub(super) fn handle_cancel_order(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg)?;
    match exchange.as_str() {
        "binance" => handle_cancel_order_binance(client, cfg, obj),
        "okx" => handle_cancel_order_okx(client, cfg, obj),
        _ => Err(tr_with(
            "crypto.err.unsupported_exchange_for_cancel_order",
            &[("exchange", &exchange)],
        )),
    }
}

pub(super) fn handle_positions(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg)?;
    match exchange.as_str() {
        "binance" => handle_positions_binance(client, cfg)
            .map_err(|err| crypto_account_access_error("binance", err)),
        "okx" => {
            handle_positions_okx(client, cfg).map_err(|err| crypto_account_access_error("okx", err))
        }
        _ => Err(tr_with(
            "crypto.err.unsupported_exchange_for_positions",
            &[("exchange", &exchange)],
        )),
    }
}

pub(super) fn handle_open_orders(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg)?;
    match exchange.as_str() {
        "binance" => {
            ensure_binance_config(cfg)
                .map_err(|err| crypto_account_access_error("binance", err))?;
            let symbol = obj
                .get("symbol")
                .and_then(|v| v.as_str())
                .map(normalize_symbol);
            let mut params = Vec::<(&str, String)>::new();
            if let Some(s) = &symbol {
                params.push(("symbol", s.clone()));
            }
            let v =
                binance_signed_request(client, cfg, Method::GET, "/api/v3/openOrders", &mut params)
                    .map_err(|err| crypto_account_access_error("binance", err))?;
            let arr = v.as_array().cloned().unwrap_or_default();
            let mut lines = Vec::new();
            for order in &arr {
                let sym = order.get("symbol").and_then(|x| x.as_str()).unwrap_or("");
                let oid = order
                    .get("orderId")
                    .map(|x| x.to_string())
                    .unwrap_or_default();
                let side = order.get("side").and_then(|x| x.as_str()).unwrap_or("");
                let otype = order.get("type").and_then(|x| x.as_str()).unwrap_or("");
                let price = order.get("price").and_then(|x| x.as_str()).unwrap_or("0");
                let orig_qty = order.get("origQty").and_then(|x| x.as_str()).unwrap_or("0");
                let exec_qty = order
                    .get("executedQty")
                    .and_then(|x| x.as_str())
                    .unwrap_or("0");
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
                    &[
                        ("exchange", "binance"),
                        ("symbol_suffix", symbol_suffix.as_str()),
                    ],
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
            Ok((
                summary,
                json!({"action":"open_orders","exchange":"binance","orders":arr}),
            ))
        }
        "okx" => {
            ensure_okx_config(cfg).map_err(|err| crypto_account_access_error("okx", err))?;
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
            let v = okx_request(
                client,
                cfg,
                Method::GET,
                "/api/v5/trade/orders-pending",
                Some(&q),
                None,
            )
            .map_err(|err| crypto_account_access_error("okx", err))?;
            let arr = v
                .get("data")
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default();
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
                    &[
                        ("exchange", "okx"),
                        ("symbol_suffix", symbol_suffix.as_str()),
                    ],
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
            Ok((
                summary,
                json!({"action":"open_orders","exchange":"okx","orders":arr}),
            ))
        }
        _ => Err(format!("open_orders: unsupported exchange={exchange}")),
    }
}

pub(super) fn handle_cancel_all_orders(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg)?;
    match exchange.as_str() {
        "binance" => {
            ensure_binance_config(cfg)?;
            let symbol = obj
                .get("symbol")
                .and_then(|v| v.as_str())
                .ok_or("cancel_all_orders on binance requires symbol")?;
            let sym = normalize_symbol(symbol);
            let mut params = vec![("symbol", sym.clone())];
            let v = binance_signed_request(
                client,
                cfg,
                Method::DELETE,
                "/api/v3/openOrders",
                &mut params,
            )?;
            let arr = v.as_array().cloned().unwrap_or_default();
            let text = tr_with(
                "crypto.msg.cancel_all_orders_done",
                &[
                    ("exchange", "binance"),
                    ("sym", sym.as_str()),
                    ("count", &arr.len().to_string()),
                ],
            );
            Ok((
                text,
                json!({"action":"cancel_all_orders","exchange":"binance","symbol":sym,"cancelled":arr}),
            ))
        }
        "okx" => {
            ensure_okx_config(cfg)?;
            let symbol = obj
                .get("symbol")
                .and_then(|v| v.as_str())
                .map(normalize_symbol);
            // First fetch open orders, then cancel them in batch
            let mut q_parts = vec!["instType=SPOT".to_string()];
            if let Some(s) = &symbol {
                q_parts.push(format!("instId={}", encode(&to_okx_inst_id(s))));
            }
            let q = q_parts.join("&");
            let pending = okx_request(
                client,
                cfg,
                Method::GET,
                "/api/v5/trade/orders-pending",
                Some(&q),
                None,
            )?;
            let arr = pending
                .get("data")
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default();
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
            let v = okx_request(
                client,
                cfg,
                Method::POST,
                "/api/v5/trade/cancel-batch-orders",
                None,
                Some(body),
            )?;
            let cancelled = v
                .get("data")
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default();
            let sym_info = symbol.as_deref().unwrap_or("all");
            let text = tr_with(
                "crypto.msg.cancel_all_orders_done",
                &[
                    ("exchange", "okx"),
                    ("sym", sym_info),
                    ("count", &cancelled.len().to_string()),
                ],
            );
            Ok((
                text,
                json!({"action":"cancel_all_orders","exchange":"okx","cancelled":cancelled}),
            ))
        }
        _ => Err(format!(
            "cancel_all_orders: unsupported exchange={exchange}"
        )),
    }
}

pub(super) fn handle_trade_history(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let exchange = resolve_exchange(obj.get("exchange").and_then(|v| v.as_str()), cfg)?;
    let limit = obj
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20)
        .clamp(1, 500);
    match exchange.as_str() {
        "binance" => {
            ensure_binance_config(cfg)
                .map_err(|err| crypto_account_access_error("binance", err))?;
            let symbol = obj
                .get("symbol")
                .and_then(|v| v.as_str())
                .ok_or("trade_history on binance requires symbol")?;
            let sym = normalize_symbol(symbol);
            let mut params = vec![("symbol", sym.clone()), ("limit", limit.to_string())];
            let v =
                binance_signed_request(client, cfg, Method::GET, "/api/v3/myTrades", &mut params)
                    .map_err(|err| crypto_account_access_error("binance", err))?;
            let arr = v.as_array().cloned().unwrap_or_default();
            let mut lines = Vec::new();
            for trade in &arr {
                let side = if trade
                    .get("isBuyer")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false)
                {
                    "buy"
                } else {
                    "sell"
                };
                let price = trade.get("price").and_then(|x| x.as_str()).unwrap_or("0");
                let qty = trade.get("qty").and_then(|x| x.as_str()).unwrap_or("0");
                let quote_qty = trade
                    .get("quoteQty")
                    .and_then(|x| x.as_str())
                    .unwrap_or("0");
                let commission = trade
                    .get("commission")
                    .and_then(|x| x.as_str())
                    .unwrap_or("0");
                let comm_asset = trade
                    .get("commissionAsset")
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                let tid = trade.get("id").map(|x| x.to_string()).unwrap_or_default();
                lines.push(format!(
                    "{sym} {side} qty={qty} quoteQty={quote_qty} price={price} fee={commission}{comm_asset} id={tid}"
                ));
            }
            let summary = if lines.is_empty() {
                format!("trade_history binance {sym}: none")
            } else {
                format!(
                    "trade_history binance {sym} count={}\n{}",
                    arr.len(),
                    lines.join("\n")
                )
            };
            Ok((
                summary,
                json!({"action":"trade_history","exchange":"binance","symbol":sym,"trades":arr}),
            ))
        }
        "okx" => {
            ensure_okx_config(cfg).map_err(|err| crypto_account_access_error("okx", err))?;
            let symbol = obj
                .get("symbol")
                .and_then(|v| v.as_str())
                .map(normalize_symbol);
            let mut q_parts = vec!["instType=SPOT".to_string(), format!("limit={limit}")];
            if let Some(s) = &symbol {
                q_parts.push(format!("instId={}", encode(&to_okx_inst_id(s))));
            }
            let q = q_parts.join("&");
            let v = okx_request(
                client,
                cfg,
                Method::GET,
                "/api/v5/trade/fills",
                Some(&q),
                None,
            )
            .map_err(|err| crypto_account_access_error("okx", err))?;
            let arr = v
                .get("data")
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default();
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
                format!(
                    "trade_history okx {sym_info} count={}\n{}",
                    arr.len(),
                    lines.join("\n")
                )
            };
            Ok((
                summary,
                json!({"action":"trade_history","exchange":"okx","trades":arr}),
            ))
        }
        _ => Err(format!("trade_history: unsupported exchange={exchange}")),
    }
}

pub(super) fn handle_order_status_binance(
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
    let v = binance_signed_request(client, cfg, Method::GET, "/api/v3/order", &mut params)
        .map_err(|err| crypto_account_access_error("binance", err))?;
    let status = v
        .get("status")
        .and_then(|x| x.as_str())
        .unwrap_or("UNKNOWN");
    let id_text = order_id.or(client_order_id).unwrap_or("unknown");
    let sym = normalize_symbol(symbol);
    let text = tr_with(
        "crypto.msg.order_status_summary",
        &[
            ("symbol", sym.as_str()),
            ("id_text", id_text),
            ("status", status),
        ],
    );
    Ok((
        text,
        json!({"action":"order_status","exchange":"binance","order":v}),
    ))
}

pub(super) fn handle_cancel_order_binance(
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
    Ok((
        text,
        json!({"action":"cancel_order","exchange":"binance","order":v}),
    ))
}

pub(super) fn handle_positions_binance(
    client: &Client,
    cfg: &RootConfig,
) -> Result<(String, Value), String> {
    let mut params = Vec::<(&str, String)>::new();
    let v = binance_signed_request(client, cfg, Method::GET, "/api/v3/account", &mut params)
        .map_err(|err| crypto_account_access_error("binance", err))?;
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
                &[
                    ("asset", asset),
                    ("free", free_s.as_str()),
                    ("locked", locked_s.as_str()),
                ],
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

pub(super) fn handle_order_status_okx(
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
    let v = okx_request(
        client,
        cfg,
        Method::GET,
        "/api/v5/trade/order",
        Some(&q),
        None,
    )
    .map_err(|err| crypto_account_access_error("okx", err))?;
    let data = v
        .get("data")
        .and_then(|x| x.as_array())
        .and_then(|x| x.first())
        .cloned()
        .unwrap_or(Value::Null);
    let state = data
        .get("state")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown");
    let id_text = order_id.or(client_order_id).unwrap_or("unknown");
    let sym = normalize_symbol(symbol);
    let text = tr_with(
        "crypto.msg.order_status_summary",
        &[
            ("symbol", sym.as_str()),
            ("id_text", id_text),
            ("status", state),
        ],
    );
    Ok((
        text,
        json!({"action":"order_status","exchange":"okx","order":data}),
    ))
}

pub(super) fn handle_cancel_order_okx(
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
    Ok((
        text,
        json!({"action":"cancel_order","exchange":"okx","order":v}),
    ))
}

pub(super) fn handle_positions_okx(
    client: &Client,
    cfg: &RootConfig,
) -> Result<(String, Value), String> {
    let v = okx_request(
        client,
        cfg,
        Method::GET,
        "/api/v5/account/balance",
        None,
        None,
    )
    .map_err(|err| crypto_account_access_error("okx", err))?;
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
                &[
                    ("ccy", ccy),
                    ("eq", eq_s.as_str()),
                    ("avail", avail_s.as_str()),
                ],
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
