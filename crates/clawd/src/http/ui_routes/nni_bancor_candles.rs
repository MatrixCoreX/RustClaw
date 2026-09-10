fn validate_bancor_candles_response(
    data: &Value,
    expected_interval_seconds: u64,
    expected_limit: usize,
) -> Result<(), &'static str> {
    let object = data
        .as_object()
        .ok_or("nni_bancor_candles_contract_invalid")?;
    if object.get("schema_version").and_then(Value::as_u64) != Some(1)
        || object.get("status").and_then(Value::as_str) != Some("bancor_candles")
        || object
            .get("market_id")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        || object
            .get("market_version")
            .and_then(Value::as_u64)
            .is_none()
        || object
            .get("market_created_at_unix")
            .and_then(Value::as_i64)
            .is_none_or(|value| value < 0)
        || object.get("price_kind").and_then(Value::as_str) != Some(NNI_BANCOR_CANDLE_PRICE_KIND)
        || object.get("interval_seconds").and_then(Value::as_u64) != Some(expected_interval_seconds)
        || object.get("price_scale").and_then(Value::as_u64) != Some(1_000_000_000_000)
        || object.get("price_decimal_places").and_then(Value::as_u64) != Some(12)
    {
        return Err("nni_bancor_candles_contract_invalid");
    }
    let range_start = object
        .get("start_time_unix")
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0)
        .ok_or("nni_bancor_candles_contract_invalid")?;
    let range_end = object
        .get("end_time_unix")
        .and_then(Value::as_i64)
        .filter(|value| *value >= range_start)
        .ok_or("nni_bancor_candles_contract_invalid")?;
    let candles = object
        .get("candles")
        .and_then(Value::as_array)
        .ok_or("nni_bancor_candles_contract_invalid")?;
    if candles.len() > expected_limit {
        return Err("nni_bancor_candles_contract_invalid");
    }
    let interval_seconds = i64::try_from(expected_interval_seconds)
        .map_err(|_| "nni_bancor_candles_contract_invalid")?;
    let mut previous_end = None;
    for candle in candles {
        let candle = candle
            .as_object()
            .ok_or("nni_bancor_candles_contract_invalid")?;
        let bucket_start = candle
            .get("bucket_start_unix")
            .and_then(Value::as_i64)
            .filter(|value| *value >= range_start)
            .ok_or("nni_bancor_candles_contract_invalid")?;
        let bucket_end = candle
            .get("bucket_end_unix")
            .and_then(Value::as_i64)
            .filter(|value| *value > bucket_start && *value <= range_end)
            .ok_or("nni_bancor_candles_contract_invalid")?;
        let bucket_span = bucket_end - bucket_start;
        let span_is_valid = if expected_interval_seconds == 31_536_000 {
            (31_536_000..=31_622_400).contains(&bucket_span)
        } else {
            bucket_span == interval_seconds
        };
        if !span_is_valid || previous_end.is_some_and(|value| bucket_start < value) {
            return Err("nni_bancor_candles_contract_invalid");
        }
        previous_end = Some(bucket_end);

        let mut prices = [0.0_f64; 4];
        for (index, field) in ["open", "high", "low", "close"].iter().enumerate() {
            prices[index] = candle
                .get(*field)
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|value| value.is_finite() && *value > 0.0)
                .ok_or("nni_bancor_candles_contract_invalid")?;
        }
        let [open, high, low, close] = prices;
        if high < open.max(close) || low > open.min(close) || low > high {
            return Err("nni_bancor_candles_contract_invalid");
        }
        for field in ["aic_volume_units", "usd_volume_units"] {
            if candle
                .get(field)
                .and_then(Value::as_str)
                .is_none_or(|value| {
                    value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit())
                })
            {
                return Err("nni_bancor_candles_contract_invalid");
            }
        }
        for field in ["aic_volume", "usd_volume"] {
            if candle
                .get(field)
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<f64>().ok())
                .is_none_or(|value| !value.is_finite() || value < 0.0)
            {
                return Err("nni_bancor_candles_contract_invalid");
            }
        }
        let trade_count = candle
            .get("trade_count")
            .and_then(Value::as_u64)
            .ok_or("nni_bancor_candles_contract_invalid")?;
        let has_trades = candle
            .get("has_trades")
            .and_then(Value::as_bool)
            .ok_or("nni_bancor_candles_contract_invalid")?;
        if has_trades != (trade_count > 0) {
            return Err("nni_bancor_candles_contract_invalid");
        }
        let liquidity_count = candle
            .get("liquidity_event_count")
            .and_then(Value::as_u64)
            .ok_or("nni_bancor_candles_contract_invalid")?;
        let liquidity_units = candle
            .get("liquidity_usd_units")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
            .ok_or("nni_bancor_candles_contract_invalid")?;
        let liquidity_amount = candle
            .get("liquidity_usd")
            .and_then(Value::as_str)
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v >= 0.0)
            .ok_or("nni_bancor_candles_contract_invalid")?;
        if (liquidity_count == 0) != liquidity_units.bytes().all(|b| b == b'0')
            || (liquidity_count == 0) != (liquidity_amount == 0.0)
        {
            return Err("nni_bancor_candles_contract_invalid");
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "nni_bancor_candles_tests.rs"]
mod pool_candle_tests;
