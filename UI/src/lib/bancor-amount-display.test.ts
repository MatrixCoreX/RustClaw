import assert from "node:assert/strict";
import test from "node:test";

import {
  formatBancorAssetAmountForDisplay,
  formatBancorBalanceAmount,
  formatBancorBalanceHoverAmount,
  formatBancorIntegerAmount,
  formatBancorTradeHistoryAmount,
} from "./bancor-amount-display";

test("BANCOR reserve display rounds decimal amounts to an integer without floating point", () => {
  assert.equal(formatBancorIntegerAmount("25802835.65290000"), "25802836");
  assert.equal(formatBancorIntegerAmount("10000.00000000"), "10000");
  assert.equal(formatBancorIntegerAmount("invalid"), "invalid");
});

test("BANCOR trade display preserves settlement precision for both assets", () => {
  assert.equal(formatBancorAssetAmountForDisplay("33222036.72780000", "AIC"), "33222036.72780000");
  assert.equal(formatBancorAssetAmountForDisplay("0.29888786", "USD"), "0.29888786");
});

test("BANCOR trade history keeps meaningful precision without trailing zeros", () => {
  assert.equal(formatBancorTradeHistoryAmount("900191.74840000"), "900191.7484");
  assert.equal(formatBancorTradeHistoryAmount("1200.00000000"), "1200");
  assert.equal(formatBancorTradeHistoryAmount("0.03340000"), "0.0334");
  assert.equal(formatBancorTradeHistoryAmount("invalid"), "invalid");
});

test("BANCOR balances truncate to two decimals without floating point", () => {
  assert.equal(formatBancorBalanceAmount("40931009.34474085"), "40931009.34");
  assert.equal(formatBancorBalanceAmount("5700.00000000"), "5700.00");
  assert.equal(formatBancorBalanceAmount("9.99900000"), "9.99");
  assert.equal(formatBancorBalanceAmount("-9.99900000"), "-9.99");
  assert.equal(formatBancorBalanceAmount("invalid"), "invalid");
});

test("BANCOR balance hover values preserve up to eight decimals without trailing zeros", () => {
  assert.equal(formatBancorBalanceHoverAmount("40931009.34474085"), "40931009.34474085");
  assert.equal(formatBancorBalanceHoverAmount("5700.00000000"), "5700");
  assert.equal(formatBancorBalanceHoverAmount("9.99900000"), "9.999");
  assert.equal(formatBancorBalanceHoverAmount("1.123456789"), "1.12345678");
  assert.equal(formatBancorBalanceHoverAmount("invalid"), "invalid");
});
