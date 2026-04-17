import test from "node:test";
import assert from "node:assert/strict";

import { formatDateOnlyHuman } from "./date-format.ts";

test("formats unix seconds as YYYY-MM-DD", () => {
  assert.equal(formatDateOnlyHuman("1775098855", "zh-CN"), "2026-04-02");
});

test("formats iso datetime as YYYY-MM-DD", () => {
  assert.equal(formatDateOnlyHuman("2026-04-02T03:00:55Z", "en-US"), "2026-04-02");
});

test("keeps invalid values unchanged", () => {
  assert.equal(formatDateOnlyHuman("not-a-date", "zh-CN"), "not-a-date");
});
