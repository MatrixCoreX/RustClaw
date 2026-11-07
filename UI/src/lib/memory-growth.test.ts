import assert from "node:assert/strict";
import test from "node:test";
import { capacityPolicyKey, parseMemoryCapacity, type MemoryCapacity } from "./memory-capacity";
import { manualGrowthLabel, measurementLabel } from "./memory-capacity-display";
import { report, settings, t } from "./memory-capacity.fixture";

const parse = (data: unknown) => parseMemoryCapacity(data, capacityPolicyKey(settings()));
function growth(admin = false): MemoryCapacity {
  const value = report();
  value.manual_growth = { schema_version: 1, scope: "memory_domain_assigned_btree_content_growth",
    decision: "available", configured_limit_bytes: admin ? { status: "measured", bytes: "1024" } : { status: "admin_only" } };
  if (admin) value.measurements.memory_domain_assigned_btree_bytes = { status: "measured", bytes: "512" };
  return value;
}

test("an absent manual observation on an older node is never displayed as available", () => {
  const read = parse(report());
  assert.ok(read);
  const label = manualGrowthLabel(read.manual_growth?.decision, t);
  assert.equal(read.manual_growth, undefined);
  assert.match(label, /尚不可用/);
});

test("manual observation validates its complete versioned contract", () => {
  for (const manual of [null, {}, { ...growth().manual_growth, schema_version: 2 },
    { ...growth().manual_growth, scope: "future" }, { ...growth().manual_growth, decision: "future" },
    { ...growth().manual_growth, configured_limit_bytes: { status: "measured", bytes: "0" } }]) {
    const value = growth();
    Object.assign(value, { manual_growth: manual });
    assert.equal(parse(value), null);
  }
});

test("background quota exhaustion and missing quota do not contradict a manual storage observation", () => {
  for (const kind of ["missing", "spent"] as const) {
    const value = growth();
    if (kind === "missing") {
      value.capacity_decision = value.quota.decision = "principal_quota_missing";
      value.quota.counters = null;
    } else {
      value.quota.counters!.max_logical_microunits = "0";
      value.capacity_decision = value.quota.decision = "principal_background_cost_exhausted";
    }
    assert.equal(parse(value)?.manual_growth?.decision, "available");
  }
});

test("whole database pressure may pause background while manual domain budget remains available", () => {
  const value = growth(true);
  value.pressure.state = "explicit_write_blocked";
  value.capacity_decision = "storage_paused";
  const read = parse(value);
  assert.equal(read?.capacity_decision, "storage_paused");
  assert.equal(read?.manual_growth?.decision, "available");
});

test("measured manual budget decisions must agree with actual domain allocation", () => {
  const value = growth(true);
  value.measurements.memory_domain_assigned_btree_bytes = { status: "measured", bytes: "1024" };
  assert.equal(parse(value), null);
  value.manual_growth!.decision = "domain_budget_reached";
  assert.equal(parse(value)?.manual_growth?.decision, "domain_budget_reached");
  value.measurements.memory_domain_assigned_btree_bytes = { status: "measured", bytes: "512" };
  assert.equal(parse(value), null);
});

test("manual budget roundtrips full u64 decimals without floating point coercion", () => {
  const value = growth(true);
  value.manual_growth!.configured_limit_bytes = { status: "measured", bytes: "18446744073709551615" };
  value.measurements.memory_domain_assigned_btree_bytes = { status: "measured", bytes: "18446744073709551614" };
  const read = parse(value);
  const manual = read?.manual_growth;
  assert.ok(manual);
  assert.equal(measurementLabel(manual.configured_limit_bytes, "en-US", t), "18,446,744,073,709,551,615 B");
});

test("missing or unknown storage observations cannot carry an available manual decision", () => {
  for (const state of ["missing", "unknown"] as const) {
    const value = growth();
    value.pressure.state = state;
    value.capacity_decision = state === "missing" ? "storage_state_missing" : "storage_state_unknown";
    assert.equal(parse(value), null);
    value.manual_growth!.decision = state === "missing" ? "storage_state_missing" : "storage_state_unknown";
    value.manual_growth!.configured_limit_bytes = { status: "unavailable", reason: "not_sampled" };
    assert.ok(parse(value));
  }
});

test("missing and damaged policy have distinct localized recovery labels and no fabricated limit", () => {
  const labels = new Set<string>();
  for (const decision of ["storage_policy_missing", "storage_policy_invalid", "storage_policy_unavailable"] as const) {
    const value = growth(true);
    value.manual_growth!.decision = decision;
    assert.equal(parse(value), null);
    value.manual_growth!.configured_limit_bytes = { status: "unavailable", reason: "not_sampled" };
    assert.equal(parse(value)?.manual_growth?.decision, decision);
    labels.add(manualGrowthLabel(decision, t));
    assert.match(manualGrowthLabel(decision, (_zh, en) => en), /configuration/);
  }
  assert.equal(labels.size, 3);
});

test("manual projection remains bound to the selected owner revision and conversation", () => {
  for (const replacement of [settings("B"), settings("A", 2), settings("A", 1, "other")]) {
    assert.equal(parse({ ...growth(), settings: replacement }), null);
  }
});

test("manual projection rejects malformed limits and contradictory stop provenance", () => {
  for (const bytes of ["-1", "1e4", "001", 1024, null]) {
    const value = growth(true);
    Object.assign(value.manual_growth!.configured_limit_bytes, { bytes });
    assert.equal(parse(value), null);
  }
  const value = growth();
  value.manual_growth!.decision = "explicit_stop_unbound";
  assert.equal(parse(value), null);
  value.pressure.state = "explicit_write_blocked";
  value.capacity_decision = "storage_paused";
  assert.equal(parse(value)?.manual_growth?.decision, "explicit_stop_unbound");
});
