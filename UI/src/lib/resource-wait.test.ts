import assert from "node:assert/strict";
import test from "node:test";
import { resourceWaitDetail } from "./resource-wait";

test("storage waits explain the affected disk and numeric recovery threshold", () => {
  const value = { schema_version: 1, reason_code: "storage_pressure", volumes: [{ role: "system", state: "critical", available_bytes: 1024 ** 3, critical_reserve_bytes: 2 * 1024 ** 3, shortfall_bytes: 1024 ** 3 + 1 }] };
  const en = resourceWaitDetail(value, "en")!;
  assert.match(en, /System disk/);
  assert.match(en, /1.*GB/);
  assert.match(en, /more than 2.*GB/);
  assert.match(en, /continue automatically/);
  assert.match(resourceWaitDetail(value, "zh")!, /系统盘/);
  assert.match(resourceWaitDetail(value, "zh")!, /需要高于/);
});

test("failed probes never invent zero available space", () => {
  const text = resourceWaitDetail({ schema_version: 1, volumes: [{ role: "database", state: "unavailable", available_bytes: null }] }, "en")!;
  assert.match(text, /unavailable/);
  assert.doesNotMatch(text, /0 B|0 GB/);
});

test("memory and concurrency waits identify their actual resource", () => {
  assert.match(resourceWaitDetail({ schema_version: 1, reason_code: "memory_unavailable", memory: { available_bytes: 1024 ** 3, required_bytes: 2 * 1024 ** 3 } }, "en")!, /memory.*needs/);
  assert.match(resourceWaitDetail({ schema_version: 1, reason_code: "browser_slot_unavailable" }, "en")!, /Browser task slots/);
  assert.match(resourceWaitDetail({ schema_version: 1, reason_code: "provider_slot_unavailable" }, "en")!, /Model request slots/);
});

test("unknown protocols and arbitrary reason text stay out of presentation", () => {
  assert.equal(resourceWaitDetail({ schema_version: 7, reason_code: "memory_unavailable" }, "en"), null);
  assert.equal(resourceWaitDetail({ schema_version: 1, reason_code: "private arbitrary fixture" }, "en"), null);
});
