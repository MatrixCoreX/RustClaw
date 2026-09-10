use super::*;

#[test]
fn pool_funding_moves_ohlc_without_fabricating_a_trade() {
    let value = json!({
        "schema_version": 1, "status": "bancor_candles", "market_id": "aic-usd-v1",
        "market_version": 2, "market_created_at_unix": 1800000000,
        "price_kind": "pool_marginal_usd_per_aic", "interval_seconds": 60,
        "price_scale": 1000000000000_u64, "price_decimal_places": 12,
        "start_time_unix": 1800000000, "end_time_unix": 1800000060,
        "candles": [{"bucket_start_unix": 1800000000,"bucket_end_unix": 1800000060,
            "open": "0.000020000000", "low": "0.000020000000",
            "high": "0.000020020000", "close": "0.000020020000",
            "trade_count": 0, "has_trades": false,
            "aic_volume_units": "0", "usd_volume_units": "0",
            "aic_volume": "0.00000000", "usd_volume": "0.00000000",
            "liquidity_event_count": 1, "liquidity_usd_units": "100000000", "liquidity_usd": "1.00000000"}]
    });
    assert_eq!(validate_bancor_candles_response(&value, 60, 10), Ok(()));
    for field in [
        "liquidity_event_count",
        "liquidity_usd_units",
        "liquidity_usd",
    ] {
        let mut missing = value.clone();
        missing["candles"][0].as_object_mut().unwrap().remove(field);
        assert!(validate_bancor_candles_response(&missing, 60, 10).is_err());
    }
    let mut inconsistent = value.clone();
    inconsistent["candles"][0]["liquidity_event_count"] = json!(0);
    assert!(validate_bancor_candles_response(&inconsistent, 60, 10).is_err());
    let mut wrong_basis = value;
    wrong_basis["price_kind"] = json!("execution_average_usd_per_aic");
    assert!(validate_bancor_candles_response(&wrong_basis, 60, 10).is_err());
}
