import assert from "node:assert/strict";
import test from "node:test";

import {
  BANCOR_DEFAULT_CANDLE_INTERVAL_SECONDS,
  BANCOR_DEFAULT_SLIPPAGE_BPS,
  BANCOR_MARKET_TRADE_LIMIT,
  BANCOR_SUCCESS_MESSAGE_DURATION_MS,
  BANCOR_TRADE_PAGE_SIZE,
  BANCOR_MAX_SLIPPAGE_BPS,
  adjustBancorInputAmount,
  buildBancorAccountPath,
  buildBancorCandlesPath,
  calculateBancorEstimatedOutput,
  calculateBancorInputFee,
  formatBancorApiError,
  hasEarlierBancorCandles,
  parseBancorSlippagePercent,
  projectBancorCandlesForInterval,
  validateBancorTradeInput,
} from "./useBancorRuntime";

test("BANCOR opens on the five-minute view by default", () => {
  assert.equal(BANCOR_DEFAULT_CANDLE_INTERVAL_SECONDS, 300);
  assert.equal(BANCOR_MARKET_TRADE_LIMIT, 100);
  assert.equal(BANCOR_TRADE_PAGE_SIZE, 10);
  assert.equal(BANCOR_SUCCESS_MESSAGE_DURATION_MS, 5_000);
});

test("BANCOR account history requests ten trades per page", () => {
  assert.equal(buildBancorAccountPath(3), "/v1/nni/bancor/account?page=3&per_page=10");
  assert.equal(buildBancorAccountPath(-5), "/v1/nni/bancor/account?page=1&per_page=10");
});

test("BANCOR interval projection never labels old-period candles as the new period", () => {
  const fiveMinutes = { interval_seconds: 300 } as never;
  const oneHour = { interval_seconds: 3_600 } as never;
  assert.equal(projectBancorCandlesForInterval(fiveMinutes, 3_600, null), null);
  assert.equal(projectBancorCandlesForInterval(fiveMinutes, 3_600, oneHour), oneHour);
  assert.equal(projectBancorCandlesForInterval(fiveMinutes, 300, null), fiveMinutes);
});

test("BANCOR historical candle requests use an end cursor and never exceed one server page", () => {
  assert.equal(
    buildBancorCandlesPath(60, 5_000, 1_800_000_059),
    "/v1/nni/bancor/candles?interval_seconds=60&limit=300&end_time_unix=1800000059",
  );
  assert.equal(
    buildBancorCandlesPath(300, 2),
    "/v1/nni/bancor/candles?interval_seconds=300&limit=2",
  );
});

test("BANCOR history availability stops once a candle reaches market creation", () => {
  const base = {
    interval_seconds: 300,
    market_created_at_unix: 1_800_000_125,
  };
  assert.equal(hasEarlierBancorCandles({
    ...base,
    candles: [{ bucket_start_unix: 1_800_000_000 }],
  } as never), false);
  assert.equal(hasEarlierBancorCandles({
    ...base,
    candles: [{ bucket_start_unix: 1_800_000_300 }],
  } as never), true);
});

const zh = (value: string) => value;

test("BANCOR amount boundary errors are beginner-readable", () => {
  for (const code of ["nni_bancor_amount_invalid", "nni_bancor_input_amount_invalid"]) {
    const message = formatBancorApiError(code, zh, "fallback");
    assert.match(message, /必须大于 0/);
    assert.match(message, /不能超过/);
    assert.doesNotMatch(message, /nni_bancor/);
  }
});

test("BANCOR zero-output errors explain how to recover", () => {
  for (const code of [
    "nni_bancor_trade_below_minimum",
    "nni_bancor_input_after_fee_too_small",
    "nni_bancor_output_too_small",
  ]) {
    const message = formatBancorApiError(code, zh, "fallback");
    assert.match(message, /不能为 0\.0000/);
    assert.match(message, /增加交易金额/);
    assert.doesNotMatch(message, /nni_bancor/);
  }
});

test("BANCOR accepts configurable slippage up to fifty percent", () => {
  assert.equal(BANCOR_DEFAULT_SLIPPAGE_BPS, 50);
  assert.equal(BANCOR_MAX_SLIPPAGE_BPS, 5_000);
  assert.equal(parseBancorSlippagePercent("0"), 0);
  assert.equal(parseBancorSlippagePercent("0.50"), 50);
  assert.equal(parseBancorSlippagePercent("5"), 500);
  assert.equal(parseBancorSlippagePercent("50.00"), 5_000);
  assert.equal(parseBancorSlippagePercent("50.01"), null);
  assert.equal(parseBancorSlippagePercent("1.001"), null);
  assert.equal(parseBancorSlippagePercent("-1"), null);
});

const market = {
  fee_bps: 50,
  point_reserve_units: "10000000000000000",
  usd_reserve_units: "1000000000000",
} as never;

const account = {
  point_balance_units: "1000000000",
  usd_balance_units: "500000000",
} as never;

test("BANCOR frontend preflight checks the input asset balance", () => {
  assert.equal(
    validateBancorTradeInput({ side: "sell", inputAmount: "10.00010000", market, account }),
    "nni_bancor_insufficient_point_balance",
  );
  assert.equal(
    validateBancorTradeInput({ side: "buy", inputAmount: "5.00010000", market, account }),
    "nni_bancor_insufficient_usd_balance",
  );
  assert.equal(validateBancorTradeInput({ side: "sell", inputAmount: "10.00000000", market, account }), null);
  assert.equal(validateBancorTradeInput({ side: "buy", inputAmount: "5.00000000", market, account }), null);
});

test("BANCOR frontend preflight rejects zero-settlement amounts", () => {
  assert.equal(
    validateBancorTradeInput({ side: "buy", inputAmount: "0.00000001", market, account }),
    "nni_bancor_input_after_fee_too_small",
  );
  assert.equal(
    validateBancorTradeInput({ side: "sell", inputAmount: "0.00000002", market, account }),
    "nni_bancor_output_too_small",
  );
  assert.equal(
    validateBancorTradeInput({ side: "buy", inputAmount: "1.00000000", market, account: null }),
    "nni_bancor_account_required",
  );
});

test("BANCOR frontend fee preview uses the backend integer rounding rule", () => {
  assert.equal(calculateBancorInputFee("100.00000000", 50), "0.50000000");
  assert.equal(calculateBancorInputFee("0.00010000", 50), "0.00000050");
  assert.equal(calculateBancorInputFee("1.00000000", 0), "0.00000000");
  assert.equal(calculateBancorInputFee("0", 50), null);
});

test("BANCOR swap preview follows the same integer reserve formula as the backend", () => {
  assert.equal(calculateBancorEstimatedOutput({ side: "buy", inputAmount: "1.00000000", market }), "9949.01007349");
  assert.equal(calculateBancorEstimatedOutput({ side: "sell", inputAmount: "100.00000000", market }), "0.00994999");
  assert.equal(calculateBancorEstimatedOutput({ side: "buy", inputAmount: "0.00000001", market }), null);
});

test("BANCOR amount shortcuts reduce precisely and step by one whole unit", () => {
  assert.equal(adjustBancorInputAmount("100.00000000", "decrease_25_percent"), "75.00000000");
  assert.equal(adjustBancorInputAmount("100.00000000", "decrease_50_percent"), "50.00000000");
  assert.equal(adjustBancorInputAmount("2.00000000", "decrement"), "1.00000000");
  assert.equal(adjustBancorInputAmount("1.00000000", "decrement"), "1.00000000");
  assert.equal(adjustBancorInputAmount("1.00000000", "increment"), "2.00000000");
  assert.equal(adjustBancorInputAmount("", "increment"), "1.00000000");
  assert.equal(adjustBancorInputAmount("0.00010000", "decrement"), "0.00010000");
  assert.equal(adjustBancorInputAmount("0.00010000", "decrease_50_percent"), "0.00005000");
  assert.equal(adjustBancorInputAmount("invalid", "increment"), null);
  assert.equal(adjustBancorInputAmount("92233720368.00000000", "increment"), null);
});
