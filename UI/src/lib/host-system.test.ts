import test from "node:test";
import assert from "node:assert/strict";

import {
  hostCapacityUsedPercent,
  hostSummaryIsPartial,
  hostSummaryIsStale,
  hostSystemTitle,
} from "./host-system.ts";
import type { HostSystemSummary } from "../types/api.ts";

const summary: HostSystemSummary = {
  schema_version: 4,
  collected_at_ts: 1_000,
  os: {
    family: "linux",
    name: "Ubuntu 24.04 LTS",
    version: "24.04",
    kernel: "6.8.0",
  },
  architecture: "aarch64",
  deployment: "container",
  memory: {
    total_bytes: 8_000,
    available_bytes: 2_000,
    available_ratio: 0.25,
  },
  storage: {
    total_bytes: 10_000,
    available_bytes: 4_000,
    available_ratio: 0.4,
  },
  uptime_seconds: 500,
  runtime_resources: {
    pressure_state: "normal",
    reserved_memory_bytes: 0,
    active_leases: 0,
    reserved_cpu_cores: 0,
    reserved_network_slots: 0,
    reserved_provider_slots: 0,
    reserved_browser_slots: 0,
    active_heavy_leases: 0,
    waiting_tasks: 0,
    resource_waiting_tasks: 0,
    recent_waiting_reason_code: null,
    recent_admission_refusal_reason: null,
    recent_admission_refusal_at_epoch: null,
    swap_used_bytes: 0,
    cgroup_version: 2,
    process_memory_measurement: "pss_plus_swap_pss",
    process_count: 4,
    process_memory_current_bytes: 512,
    process_memory_peak_bytes: 768,
    process_memory_warning: false,
    process_memory_roles_current_bytes: { core: 400, web_gateway: 112 },
    process_memory_roles_peak_bytes: { core: 600, web_gateway: 168 },
  },
  unavailable_fields: [],
};

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
