import test from "node:test";
import assert from "node:assert/strict";

import { formatDateOnlyHuman, formatDateTimeHuman, formatUnixDateTime } from "./date-format.ts";

test("formats unix seconds as YYYY-MM-DD", () => {
  assert.equal(formatDateOnlyHuman("1775098855", "zh-CN"), "2026-04-02");
});

test("formats iso datetime as YYYY-MM-DD", () => {
  assert.equal(formatDateOnlyHuman("2026-04-02T03:00:55Z", "en-US"), "2026-04-02");
});

test("keeps invalid values unchanged", () => {
  assert.equal(formatDateOnlyHuman("not-a-date", "zh-CN"), "not-a-date");
});

test("formats full datetime values", () => {
  assert.match(formatDateTimeHuman("2026-04-02T03:00:55Z", "en-US"), /2026/);
  assert.equal(formatDateTimeHuman(null, "en-US"), "--");
  assert.equal(formatDateTimeHuman("not-a-date", "en-US"), "not-a-date");
});

test("formats unix datetime values", () => {
  assert.match(formatUnixDateTime(1775098855, "en-US"), /2026/);
  assert.equal(formatUnixDateTime(0, "en-US"), "--");
});
