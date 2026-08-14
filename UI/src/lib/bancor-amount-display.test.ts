import assert from "node:assert/strict";
import test from "node:test";

import {
  formatBancorAssetAmountForDisplay,
  formatBancorIntegerAmount,
} from "./bancor-amount-display";

test("BANCOR reserve display rounds decimal amounts to an integer without floating point", () => {
  assert.equal(formatBancorIntegerAmount("25802835.65290000"), "25802836");
  assert.equal(formatBancorIntegerAmount("10000.00000000"), "10000");
  assert.equal(formatBancorIntegerAmount("invalid"), "invalid");
});

test("BANCOR trade display uses integers for POINT while preserving USD precision", () => {
  assert.equal(formatBancorAssetAmountForDisplay("33222036.72780000", "POINT"), "33222037");
  assert.equal(formatBancorAssetAmountForDisplay("0.29888786", "USD"), "0.29888786");
});
