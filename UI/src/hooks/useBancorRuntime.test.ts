import assert from "node:assert/strict";
import test from "node:test";

import {
  BANCOR_DEFAULT_CANDLE_INTERVAL_SECONDS,
  calculateBancorInputFee,
  formatBancorApiError,
  validateBancorTradeInput,
} from "./useBancorRuntime";

test("BANCOR opens on the five-minute view by default", () => {
  assert.equal(BANCOR_DEFAULT_CANDLE_INTERVAL_SECONDS, 300);
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

test("BANCOR per-trade maximum has a distinct recovery message", () => {
  const message = formatBancorApiError("nni_bancor_trade_above_maximum", zh, "fallback");
  assert.match(message, /超过单笔安全上限/);
  assert.match(message, /减少交易金额/);
});

const market = {
  fee_bps: 50,
  point_reserve_units: "1000000000000",
  usd_reserve_units: "100000000",
} as never;

const account = {
  point_balance_units: "100000",
  usd_balance_units: "50000",
} as never;

test("BANCOR frontend preflight checks the input asset balance", () => {
  assert.equal(
    validateBancorTradeInput({ side: "sell", inputAmount: "10.0001", market, account }),
    "nni_bancor_insufficient_point_balance",
  );
  assert.equal(
    validateBancorTradeInput({ side: "buy", inputAmount: "5.0001", market, account }),
    "nni_bancor_insufficient_usd_balance",
  );
  assert.equal(validateBancorTradeInput({ side: "sell", inputAmount: "10.0000", market, account }), null);
  assert.equal(validateBancorTradeInput({ side: "buy", inputAmount: "5.0000", market, account }), null);
});

test("BANCOR frontend preflight rejects zero-settlement amounts", () => {
  assert.equal(
    validateBancorTradeInput({ side: "buy", inputAmount: "0.0001", market, account }),
    "nni_bancor_input_after_fee_too_small",
  );
  assert.equal(
    validateBancorTradeInput({ side: "sell", inputAmount: "0.0002", market, account }),
    "nni_bancor_output_too_small",
  );
  assert.equal(
    validateBancorTradeInput({ side: "buy", inputAmount: "1.0000", market, account: null }),
    "nni_bancor_account_required",
  );
});

test("BANCOR frontend fee preview uses the backend integer rounding rule", () => {
  assert.equal(calculateBancorInputFee("100.0000", 50), "0.5000");
  assert.equal(calculateBancorInputFee("0.0001", 50), "0.0001");
  assert.equal(calculateBancorInputFee("1.0000", 0), "0.0000");
  assert.equal(calculateBancorInputFee("0", 50), null);
});
