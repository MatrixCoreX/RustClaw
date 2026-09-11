import { test } from "node:test";
import assert from "node:assert/strict";
import { amountUnits, displayUnits, movementSign } from "../frontend/wallet/amounts";

test("asset amounts preserve all eight decimals without floating point rounding", () => {
  assert.equal(amountUnits("0.00000001"), "1");
  assert.equal(amountUnits("92233720368.54775807"), "9223372036854775807");
  assert.equal(displayUnits("9223372036854775807"), "92233720368.54775807");
  for (const value of [
    "0",
    "-1",
    "1e8",
    "0.123456789",
    "92233720368.54775808",
    "Infinity",
  ])
    assert.equal(amountUnits(value), null);
  assert.equal(displayUnits(undefined), "—");
  assert.equal(displayUnits("0"), "0.00000000");
});

test("history marks each trade asset movement, not two positive receipts", () => {
  assert.equal(movementSign("bancor_buy", "USD"), "-");
  assert.equal(movementSign("bancor_buy", "AIC"), "+");
  assert.equal(movementSign("bancor_sell", "USD"), "+");
  assert.equal(movementSign("bancor_sell", "AIC"), "-");
  assert.equal(movementSign("transfer_in", "USD"), "+");
  assert.equal(movementSign("transfer_out", "AIC"), "-");
});
