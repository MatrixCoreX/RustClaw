import assert from "node:assert/strict";
import test from "node:test";
import { capacityPolicyKey, capacitySelection, parseMemoryCapacity } from "./memory-capacity";
import { capacityLabel, measurementLabel } from "./memory-capacity-display";
import { report, settings, t } from "./memory-capacity.fixture";

const parse = (data: unknown) => parseMemoryCapacity(data, capacityPolicyKey(settings()));

test("capacity schema preserves exact decimal measurements above JavaScript safe integers", () => {
  const value = report();
  value.quota.counters!.max_bytes = "9223372036854775807";
  value.measurements.own_source_transcript_payload_bytes = { status: "measured", bytes: "18446744073709551615" };
  const read = parse(value);
  assert.equal(read?.quota.counters?.max_bytes, "9223372036854775807");
  assert.equal(measurementLabel(read!.measurements.own_source_transcript_payload_bytes, "en-US", t), "18,446,744,073,709,551,615 B");
});

test("missing measurements and corrupt schema never become zero or available", () => {
  for (const change of [
    { schema_version: 2 }, { sampled_at_ts: -1 }, { capacity_decision: "future" }, { measurements: {} },
    { quota: null }, { pressure: { state: "normal" } },
  ]) assert.equal(parse({ ...report(), ...change }), null);
  for (const value of [null, [], {}, 0, "text"]) assert.equal(parse(value), null);
});

test("measured byte strings reject negative fractional lossy and noncanonical values", () => {
  for (const bytes of [-1, 123, "-1", "1.5", "1e10", "001", "", null]) {
    const value = report();
    Object.assign(value.measurements.own_source_transcript_payload_bytes, { bytes });
    assert.equal(parse(value), null);
  }
});

test("capacity parser rejects counter decisions that contradict source quotas", () => {
  const value = report();
  value.quota.counters!.used_rows = "100";
  assert.equal(parse(value), null);
  value.quota.decision = value.capacity_decision = "principal_rows_exhausted";
  assert.equal(parse(value)?.capacity_decision, "principal_rows_exhausted");
});

test("unknown and missing pressure override available quota without losing requested settings", () => {
  for (const [state, decision] of [["missing", "storage_state_missing"], ["unknown", "storage_state_unknown"]] as const) {
    const value = report();
    value.pressure.state = state;
    assert.equal(parse(value), null);
    value.capacity_decision = decision;
    assert.equal(parse(value)?.settings.generate_memory, true);
  }
});

test("missing quota is explicit and cannot carry fabricated zero counters", () => {
  const value = report();
  value.quota.decision = value.capacity_decision = "principal_quota_missing";
  assert.equal(parse(value), null);
  value.quota.counters = null;
  assert.equal(parse(value)?.quota.counters, null);
});

test("capacity response binds all owner conversation policy and effective-setting fields", () => {
  for (const replacement of [settings("B"), settings("A", 2), settings("A", 1, "chat"),
    { ...settings(), generate_memory: false }, { ...settings(), managed_deny_reason: "managed" }]) {
    assert.equal(parse({ ...report(), settings: replacement }), null);
  }
});

test("capacity selection waits for matching conversation settings and encodes query context", () => {
  assert.equal(capacitySelection(settings(), "conversation", "chat"), null);
  assert.equal(capacitySelection(settings("A", 1, "old"), "conversation", "new"), null);
  assert.equal(capacitySelection(null, "principal", null), null);
  const selected = capacitySelection(settings("A", 1, "chat & 你好"), "conversation", "chat & 你好")!;
  assert.equal(new URL(selected.path, "https://fixture.invalid").searchParams.get("conversation_id"), "chat & 你好");
});

test("measurement labels distinguish actual zero missing and administrator-only data", () => {
  assert.equal(measurementLabel({ status: "measured", bytes: "0" }, "en-US", t), "0 B");
  assert.equal(measurementLabel({ status: "unavailable", reason: "in_memory_database" }, "en-US", t), "暂不可测");
  assert.equal(measurementLabel({ status: "admin_only" }, "en-US", t), "仅管理员可见");
  assert.equal(capacityLabel("available", t), "容量检查未阻止自动生成");
});
