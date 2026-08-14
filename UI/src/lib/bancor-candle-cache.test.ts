import assert from "node:assert/strict";
import test from "node:test";

import type { NniBancorCandle, NniBancorCandlesResponse } from "../types/api";
import {
  BANCOR_CANDLE_REQUEST_MAX_CANDLES,
  calculateBancorCandleRefreshLimit,
  isBancorCandleResponse,
  mergeBancorCandleResponses,
} from "./bancor-candle-cache";

function candle(bucketStart: number, close: string): NniBancorCandle {
  return {
    bucket_start_unix: bucketStart,
    bucket_end_unix: bucketStart + 300,
    open: close,
    high: close,
    low: close,
    close,
    point_volume_units: "0",
    point_volume: "0.00000000",
    usd_volume_units: "0",
    usd_volume: "0.00000000",
    trade_count: 0,
    has_trades: false,
  };
}

function response(candles: NniBancorCandle[], overrides: Partial<NniBancorCandlesResponse> = {}): NniBancorCandlesResponse {
  return {
    schema_version: 1,
    status: "bancor_candles",
    market_id: "point-usd-v1",
    interval_seconds: 300,
    start_time_unix: candles[0]?.bucket_start_unix ?? 0,
    end_time_unix: candles.at(-1)?.bucket_end_unix ?? 0,
    price_scale: 1_000_000_000_000,
    price_decimal_places: 12,
    market_version: 10,
    market_created_at_unix: 1_800_000_000,
    price_kind: "execution_average_usd_per_point",
    candles,
    ...overrides,
  };
}

test("BANCOR candle refresh requests only the live edge while cache is current", () => {
  const cached = response([candle(1_800_000_000, "0.000100000000")]);
  assert.equal(calculateBancorCandleRefreshLimit(cached, 300, 1_800_000_120), 2);
  assert.equal(calculateBancorCandleRefreshLimit(cached, 300, 1_800_001_200), 5);
  assert.equal(calculateBancorCandleRefreshLimit(null, 300, 1_800_000_120), 300);
  assert.equal(calculateBancorCandleRefreshLimit(cached, 300, 1_900_000_000), 300);
  assert.equal(BANCOR_CANDLE_REQUEST_MAX_CANDLES, 300);
});

test("BANCOR merged history is not truncated to one server page", () => {
  const older = response(Array.from({ length: 300 }, (_value, index) => (
    candle(1_799_910_000 + index * 300, "0.000100000000")
  )));
  const newer = response(Array.from({ length: 300 }, (_value, index) => (
    candle(1_800_000_000 + index * 300, "0.000101000000")
  )), { market_version: 11 });
  assert.equal(mergeBancorCandleResponses(older, newer).candles.length, 600);
});

test("BANCOR candle responses require a stable price kind and market series identity", () => {
  const valid = response([candle(1_800_000_000, "0.000100000000")]);
  assert.equal(isBancorCandleResponse(valid, 300), true);
  for (const field of ["market_version", "market_created_at_unix", "price_kind"] as const) {
    const invalid = { ...valid } as Record<string, unknown>;
    delete invalid[field];
    assert.equal(isBancorCandleResponse(invalid, 300), false, `${field} must be required`);
  }
  assert.equal(isBancorCandleResponse({ ...valid, status: "ok" }, 300), false);
  assert.equal(isBancorCandleResponse({ ...valid, price_kind: "post_trade_marginal_usd_per_point" }, 300), false);
  const missingTradeState = { ...valid, candles: [{ ...valid.candles[0] }] } as Record<string, unknown>;
  delete (missingTradeState.candles as Array<Record<string, unknown>>)[0].has_trades;
  assert.equal(isBancorCandleResponse(missingTradeState, 300), false);
});

test("BANCOR incremental candle responses replace live buckets and preserve history", () => {
  const cached = response([
    candle(1_799_999_400, "0.000099000000"),
    candle(1_799_999_700, "0.000100000000"),
    candle(1_800_000_000, "0.000101000000"),
  ]);
  const incoming = response([
    candle(1_800_000_000, "0.000102000000"),
    candle(1_800_000_300, "0.000103000000"),
  ], { market_version: 11 });
  const merged = mergeBancorCandleResponses(cached, incoming, 300);
  assert.deepEqual(merged.candles.map((value) => value.bucket_start_unix), [
    1_799_999_400,
    1_799_999_700,
    1_800_000_000,
    1_800_000_300,
  ]);
  assert.equal(merged.candles[2].close, "0.000102000000");
  assert.equal(merged.start_time_unix, 1_799_999_400);
});

test("BANCOR cache is discarded when the server market series is reset", () => {
  const cached = response([candle(1_800_000_000, "0.000101000000")], {
    market_version: 20,
    market_created_at_unix: 1_800_000_000,
  });
  const reset = response([candle(1_900_000_000, "0.000100000000")], {
    market_version: 1,
    market_created_at_unix: 1_900_000_000,
  });
  const merged = mergeBancorCandleResponses(cached, reset);
  assert.deepEqual(merged.candles, reset.candles);
});

test("BANCOR stale responses cannot overwrite newer live buckets", () => {
  const cached = response([
    candle(1_800_000_000, "0.000101000000"),
    candle(1_800_000_300, "0.000102000000"),
  ], { market_version: 20 });
  const stale = response([
    candle(1_799_999_700, "0.000100000000"),
    candle(1_800_000_000, "0.000099000000"),
  ], { market_version: 19 });
  const merged = mergeBancorCandleResponses(cached, stale);
  assert.equal(merged.market_version, 20);
  assert.equal(merged.candles.find((value) => value.bucket_start_unix === 1_800_000_000)?.close, "0.000101000000");
  assert.equal(merged.start_time_unix, 1_799_999_700);
  assert.equal(merged.end_time_unix, 1_800_000_600);
});

test("BANCOR cache retains only the configured newest buckets", () => {
  const cached = response([
    candle(1_800_000_000, "0.000100000000"),
    candle(1_800_000_300, "0.000101000000"),
  ]);
  const incoming = response([
    candle(1_800_000_600, "0.000102000000"),
    candle(1_800_000_900, "0.000103000000"),
  ], { market_version: 12 });
  const merged = mergeBancorCandleResponses(cached, incoming, 3);
  assert.deepEqual(merged.candles.map((value) => value.bucket_start_unix), [
    1_800_000_300,
    1_800_000_600,
    1_800_000_900,
  ]);
});
