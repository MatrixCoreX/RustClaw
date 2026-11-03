import test from "node:test";
import assert from "node:assert/strict";

import {
  hostCapacityUsedPercent,
  hostSummaryIsPartial,
  hostSummaryIsStale,
  hostSystemTitle,
  hostStorageNeedsAttention,
} from "./host-system.ts";
import { hostSystemSummaryFixture as summary } from "../../test/fixtures/host-system";

test("formats a bounded operating system title without duplicate versions", () => {
  assert.equal(hostSystemTitle(summary), "Ubuntu 24.04 LTS");
  assert.equal(
    hostSystemTitle({
      ...summary,
      os: { ...summary.os, name: "Ubuntu", version: "24.04" },
    }),
    "Ubuntu 24.04",
  );
});

test("computes clamped resource usage percentages", () => {
  assert.equal(hostCapacityUsedPercent(summary.memory), 75);
  assert.equal(
    hostCapacityUsedPercent({ total_bytes: 100, available_bytes: 150, available_ratio: 1 }),
    0,
  );
  assert.equal(
    hostCapacityUsedPercent({ total_bytes: null, available_bytes: null, available_ratio: null }),
    null,
  );
});

test("detects stale and partial summaries", () => {
  assert.equal(hostSummaryIsStale(summary, 1_299), false);
  assert.equal(hostSummaryIsStale(summary, 1_301), true);
  assert.equal(hostSummaryIsPartial(summary), false);
  assert.equal(
    hostSummaryIsPartial({
      ...summary,
      unavailable_fields: [{ field: "memory.available_bytes", code: "memory_available_unavailable" }],
    }),
    true,
  );
});

test("low storage is a display hint, and invalid measurements are not usable percentages", () => {
  assert.equal(hostStorageNeedsAttention({ total_bytes: 100_000, available_bytes: 1_000, available_ratio: 0.01 }), true);
  assert.equal(hostStorageNeedsAttention(summary.storage), false);
  assert.equal(hostStorageNeedsAttention({ total_bytes: 100_000, available_bytes: 5_400, available_ratio: 0.054 }), false);
  assert.equal(hostStorageNeedsAttention({ total_bytes: 100_000, available_bytes: 5_000, available_ratio: 0.05 }), true);
  assert.equal(hostStorageNeedsAttention({ total_bytes: null, available_bytes: null, available_ratio: null }), false);
  assert.equal(hostCapacityUsedPercent({ total_bytes: Number.NaN, available_bytes: 1, available_ratio: null }), null);
  assert.equal(hostCapacityUsedPercent({ total_bytes: 100, available_bytes: Number.NaN, available_ratio: null }), null);
});
