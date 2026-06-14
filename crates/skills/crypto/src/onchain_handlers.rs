use super::*;

pub(super) fn handle_onchain(
    client: &Client,
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
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
            Ok((
                text,
                json!({"action":"onchain","chain":"ethereum","stats":data}),
            ))
        }
        _ => Err(tr("crypto.err.unsupported_chain")),
    }
}

pub(super) fn handle_eth_address_onchain(
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
        .or_else(|| {
            cfg.crypto
                .eth_token_contracts
                .get(&token.to_ascii_uppercase())
        })
        .ok_or_else(|| format!("token contract not configured for ethereum token: {token}"))?;
    let decimals = cfg
        .crypto
        .eth_token_decimals
        .get(&token)
        .or_else(|| {
            cfg.crypto
                .eth_token_decimals
                .get(&token.to_ascii_uppercase())
        })
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
