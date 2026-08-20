import assert from "node:assert/strict";
import test from "node:test";

import {
  formatBancorAssetAmountForDisplay,
  formatBancorIntegerAmount,
  formatBancorTradeHistoryAmount,
} from "./bancor-amount-display";

test("BANCOR reserve display rounds decimal amounts to an integer without floating point", () => {
  assert.equal(formatBancorIntegerAmount("25802835.65290000"), "25802836");
  assert.equal(formatBancorIntegerAmount("10000.00000000"), "10000");
  assert.equal(formatBancorIntegerAmount("invalid"), "invalid");
});

test("BANCOR trade display uses integers for AIC while preserving USD precision", () => {
  assert.equal(formatBancorAssetAmountForDisplay("33222036.72780000", "AIC"), "33222037");
  assert.equal(formatBancorAssetAmountForDisplay("0.29888786", "USD"), "0.29888786");
});

test("BANCOR trade history keeps meaningful precision without trailing zeros", () => {
  assert.equal(formatBancorTradeHistoryAmount("900191.74840000"), "900191.7484");
  assert.equal(formatBancorTradeHistoryAmount("1200.00000000"), "1200");
  assert.equal(formatBancorTradeHistoryAmount("0.03340000"), "0.0334");
  assert.equal(formatBancorTradeHistoryAmount("invalid"), "invalid");
});
