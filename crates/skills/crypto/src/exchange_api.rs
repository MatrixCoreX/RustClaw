use super::*;

pub(super) fn binance_signed_request(
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

pub(super) fn okx_request(
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
    let prehash = format!(
        "{}{}{}{}",
        ts,
        method.as_str().to_ascii_uppercase(),
        req_path,
        body_text
    );
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
    let resp = req
        .send()
        .map_err(|err| format!("okx request failed: {err}"))?;
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

pub(super) fn ensure_binance_config(cfg: &RootConfig) -> Result<(), String> {
    ensure_binance_config_for_action(cfg, "")
}

pub(super) fn ensure_binance_config_for_action(
    cfg: &RootConfig,
    action: &str,
) -> Result<(), String> {
    if !cfg.binance.enabled {
        return Err(crypto_config_error(
            "binance",
            action,
            CRYPTO_CREDENTIAL_NOT_BOUND_ERROR_KIND,
            "crypto.err.binance_not_bound",
        ));
    }
    if is_placeholder(&cfg.binance.api_key) || is_placeholder(&cfg.binance.api_secret) {
        return Err(crypto_config_error(
            "binance",
            action,
            CRYPTO_CREDENTIAL_INCOMPLETE_ERROR_KIND,
            "crypto.err.binance_credentials_incomplete",
        ));
    }
    Ok(())
}

pub(super) fn ensure_okx_config(cfg: &RootConfig) -> Result<(), String> {
    ensure_okx_config_for_action(cfg, "")
}

pub(super) fn ensure_okx_config_for_action(cfg: &RootConfig, action: &str) -> Result<(), String> {
    if !cfg.okx.enabled {
        return Err(crypto_config_error(
            "okx",
            action,
            CRYPTO_CREDENTIAL_NOT_BOUND_ERROR_KIND,
            "crypto.err.okx_not_bound",
        ));
    }
    if is_placeholder(&cfg.okx.api_key)
        || is_placeholder(&cfg.okx.api_secret)
        || is_placeholder(&cfg.okx.passphrase)
    {
        return Err(crypto_config_error(
            "okx",
            action,
            CRYPTO_CREDENTIAL_INCOMPLETE_ERROR_KIND,
            "crypto.err.okx_credentials_incomplete",
        ));
    }
    Ok(())
}
