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
  persistBancorCandleInterval,
  projectBancorCandlesForInterval,
  readBancorCandleInterval,
  validateBancorTradeInput,
} from "./useBancorRuntime";

test("BANCOR opens on the one-hour view by default", () => {
  assert.equal(BANCOR_DEFAULT_CANDLE_INTERVAL_SECONDS, 3_600);
  assert.equal(BANCOR_MARKET_TRADE_LIMIT, 100);
  assert.equal(BANCOR_TRADE_PAGE_SIZE, 10);
  assert.equal(BANCOR_SUCCESS_MESSAGE_DURATION_MS, 5_000);
});

test("BANCOR persists a supported candle interval and rejects damaged preferences", () => {
  const values = new Map<string, string>();
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
  };

  assert.equal(readBancorCandleInterval(storage), 3_600);
  persistBancorCandleInterval(storage, 86_400);
  assert.equal(readBancorCandleInterval(storage), 86_400);

  persistBancorCandleInterval(storage, 123);
  assert.equal(readBancorCandleInterval(storage), 86_400);
  values.set([...values.keys()][0], "damaged");
  assert.equal(readBancorCandleInterval(storage), 3_600);
});

test("BANCOR account history requests ten trades per page", () => {
  assert.equal(buildBancorAccountPath(3), "/v1/nni/bancor/account?page=3&per_page=10");
  assert.equal(buildBancorAccountPath(-5), "/v1/nni/bancor/account?page=1&per_page=10");
  assert.equal(
    buildBancorAccountPath(2, "/v1/nni/assets"),
    "/v1/nni/assets/account?page=2&per_page=10",
  );
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

test("BANCOR minimum errors state only the concrete lower bound", () => {
  assert.equal(
    formatBancorApiError(
      "nni_bancor_trade_below_minimum",
      zh,
      "fallback",
      { amount: "0.0001", asset: "USD" },
    ),
    "金额不能小于 0.0001 USD。",
  );
});

test("BANCOR zero-output errors explain how to recover", () => {
  for (const code of ["nni_bancor_input_after_fee_too_small", "nni_bancor_output_too_small"]) {
    const message = formatBancorApiError(code, zh, "fallback");
    assert.match(message, /增加交易金额/);
    assert.doesNotMatch(message, /nni_bancor/);
  }
});

test("BANCOR missing asset owner error directs beginners to the NNI page", () => {
  const message = formatBancorApiError("nni_asset_owner_required", zh, "fallback");
  assert.match(message, /NNI 页面/);
  assert.match(message, /生成并绑定资产账号/);
  assert.doesNotMatch(message, /nni_asset_owner_required/);
});

test("BANCOR revoked device authorization explains how to bind the device again", () => {
  const message = formatBancorApiError("nni_asset_device_not_authorized", zh, "fallback");
  assert.match(message, /当前设备/);
  assert.match(message, /重新绑定资产账号/);
  assert.doesNotMatch(message, /nni_asset_device_not_authorized/);
});

test("BANCOR formats NNI device admission errors for users", () => {
  for (const code of [
    "nni_public_key_whitelist_empty",
    "public_key_whitelist_empty",
    "nni_pubkey_not_allowlisted",
    "nni_public_key_not_allowlisted",
    "public_key_not_allowlisted",
  ]) {
    assert.equal(
      formatBancorApiError(code, zh, "fallback"),
      "当前硬件签名方式暂时无法读取账户。可以改用资产密钥签名交易。",
    );
  }
});

test("BANCOR formats Core business rate limits instead of exposing machine codes", () => {
  const chinese = formatBancorApiError("nni_rate_limit_bancor_private", zh, "fallback");
  const english = formatBancorApiError(
    "nni_rate_limit_bancor_private",
    (_zh, en) => en,
    "fallback",
  );
  assert.equal(chinese, "账户与交易请求过于频繁，请稍后再试。");
  assert.equal(english, "Account and trading requests are too frequent. Try again shortly.");
  assert.doesNotMatch(chinese, /nni_rate_limit/);
  assert.doesNotMatch(english, /nni_rate_limit/);
});

test("BANCOR accepts configurable slippage up to fifty percent", () => {
  assert.equal(BANCOR_DEFAULT_SLIPPAGE_BPS, 300);
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
  min_trade_usd: "0.00000200",
  min_trade_usd_units: "200",
  min_trade_aic: "0.00010052",
  min_trade_aic_units: "10052",
  minimum_fee_units: "1",
  minimum_output_units: "1",
  aic_reserve_units: "10000000000000000",
  usd_reserve_units: "1000000000000",
} as never;

const account = {
  aic_balance_units: "1000000000",
  usd_balance_units: "500000000",
} as never;

test("BANCOR frontend preflight checks the input asset balance", () => {
  assert.equal(
    validateBancorTradeInput({ side: "sell", inputAmount: "10.00010000", market, account }),
    "nni_bancor_insufficient_aic_balance",
  );
  assert.equal(
    validateBancorTradeInput({ side: "buy", inputAmount: "5.00010000", market, account }),
    "nni_bancor_insufficient_usd_balance",
  );
  assert.equal(validateBancorTradeInput({ side: "sell", inputAmount: "10.00000000", market, account }), null);
  assert.equal(validateBancorTradeInput({ side: "buy", inputAmount: "5.00000000", market, account }), null);
});

test("BANCOR frontend preflight enforces the market minimum before settlement math", () => {
  assert.equal(
    validateBancorTradeInput({ side: "buy", inputAmount: "0.00000001", market, account }),
    "nni_bancor_trade_below_minimum",
  );
  assert.equal(
    validateBancorTradeInput({ side: "buy", inputAmount: "0.00000199", market, account }),
    "nni_bancor_trade_below_minimum",
  );
  assert.equal(
    validateBancorTradeInput({ side: "sell", inputAmount: "0.00010051", market, account }),
    "nni_bancor_trade_below_minimum",
  );
  assert.equal(validateBancorTradeInput({ side: "buy", inputAmount: "0.00000200", market, account }), null);
  assert.equal(validateBancorTradeInput({ side: "sell", inputAmount: "0.00010052", market, account }), null);
  assert.equal(
    validateBancorTradeInput({ side: "buy", inputAmount: "1.00000000", market, account: null }),
    "nni_bancor_account_required",
  );
});

test("BANCOR leaves minimum validation to the backend during a rolling schema upgrade", () => {
  const marketWithoutDynamicMinimums = {
    ...(market as unknown as Record<string, unknown>),
    min_trade_usd: undefined,
    min_trade_usd_units: undefined,
    min_trade_aic: undefined,
    min_trade_aic_units: undefined,
  } as never;
  assert.equal(
    validateBancorTradeInput({
      side: "buy",
      inputAmount: "1.00000000",
      market: marketWithoutDynamicMinimums,
      account,
    }),
    null,
  );
  assert.equal(
    validateBancorTradeInput({
      side: "sell",
      inputAmount: "1.00000000",
      market: marketWithoutDynamicMinimums,
      account,
    }),
    null,
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
