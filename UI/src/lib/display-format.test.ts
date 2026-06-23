import test from "node:test";
import assert from "node:assert/strict";

import { formatBytes, formatDuration, sleep, toLocalTime } from "./display-format.ts";

test("formats byte counts for compact dashboard cards", () => {
  assert.equal(formatBytes(null), "--");
  assert.equal(formatBytes(512), "512 B");
  assert.equal(formatBytes(1536), "1.50 KB");
  assert.equal(formatBytes(5 * 1024 * 1024), "5.00 MB");
});

test("formats task and uptime durations", () => {
  assert.equal(formatDuration(undefined), "--");
  assert.equal(formatDuration(7), "7s");
  assert.equal(formatDuration(67), "1m 7s");
  assert.equal(formatDuration(3661), "1h 1m 1s");
  assert.equal(formatDuration(90061), "1d 1h 1m");
});

test("formats local time without throwing", () => {
  assert.equal(typeof toLocalTime(1782197321075), "string");
});

test("sleep resolves after a short timeout", async () => {
  const before = Date.now();
  await sleep(1);
  assert.ok(Date.now() >= before);
});
