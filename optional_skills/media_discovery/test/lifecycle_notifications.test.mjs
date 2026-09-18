import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { handleRequest } from "../src/main.mjs";
import { createBackgroundProgressReporter, backgroundReportIntervalMs } from "../src/progress.mjs";

async function fixture(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "discovery-notifications-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const context = { skill_storage: { storage_kind: "directory", directory_path: root } };
  const frames = [];
  return { frames, request: args => ({ request_id: "notification-test", args, context }),
    runtime: { writeProgress: frame => frames.push(frame) } };
}

test("parallel finite collection notifies once at start, final response preserves results", async t => {
  const { frames, request, runtime } = await fixture(t);
  const result = await handleRequest(request({ action: "run_once", platforms: ["douyin", "kuaishou"],
    max_items_per_run: 300, max_run_minutes: 30 }), { ...runtime, parallelLimit: 2,
    collectPlatform: async ({ platform, onPage }) => {
      assert.equal(frames.length, 1);
      await onPage({ records: [{ kind: "video", platform, dedup_key: platform }], temporaryPaths: [] });
    } });
  assert.equal(result.status, "ok", JSON.stringify(result));
  assert.equal(result.extra.counts.items, 2);
  assert.equal(frames.length, 1); // Completion is delivered by the normal model finalizer, not a duplicate progress notice.
  assert.equal(frames[0].params.requested_items, 300);
  assert.equal(frames[0].params.max_run_minutes, 30);
  assert.equal(frames[0].params.continuous, false);
  assert.equal(frames[0].params.stop_capability, "media_discovery.stop_current");
  assert.equal(frames[0].params.notification_renderer, "model");
  assert.deepEqual(frames[0].params.platforms, ["douyin", "kuaishou"]);
  assert.equal("text" in frames[0], false);
});

test("time-only collection has no implicit five-post target", async t => {
  const { frames, request, runtime } = await fixture(t);
  const result = await handleRequest(request({ action: "run_once", platform: "douyin", max_run_minutes: 15 }),
    { ...runtime, collectPlatform: async ({ limit, onPage }) => {
      assert.equal(limit, Infinity);
      for (let n = 0; n < 7; n++) await onPage({ records: [{ kind: "video", platform: "douyin", dedup_key: String(n) }], temporaryPaths: [] });
      return { stop_reason: "no_new_results" };
    } });
  assert.equal(result.extra.run.counts.items, 7);
  assert.equal(result.extra.run.collection_outcome.target_reached, false);
  assert.equal(frames[0].params.requested_items, 0);
  assert.equal(frames[0].params.max_run_minutes, 15);
});

test("continuous worker sends one start with stop guidance, not one per internal batch", async t => {
  const { frames, request, runtime } = await fixture(t);
  await handleRequest(request({ action: "enable", platform: "douyin", confirm: true }), runtime);
  assert.equal(frames.length, 0);
  let batches = 0, clock = Date.now();
  const result = await handleRequest(request({ action: "run_enabled_once" }), { ...runtime,
    now: () => clock, sleep: async ms => { clock += ms; },
    collectPlatform: async ({ onPage }) => {
      assert.equal(frames.length, 1);
      batches++;
      await onPage({ records: [{ kind: "video", platform: "douyin", dedup_key: String(batches) }], temporaryPaths: [] });
      if (batches === 3) await handleRequest(request({ action: "disable", platform: "douyin" }));
    } });
  assert.equal(result.status, "ok", JSON.stringify(result));
  assert.equal(batches, 3);
  assert.equal(result.extra.state, "stopped");
  assert.equal(result.extra.background_worker.counts.items, 3);
  assert.equal(frames.length, 1);
  assert.equal(frames[0].params.continuous, true);
  assert.equal(frames[0].params.stop_capability, "media_discovery.disable");
  assert.equal(frames[0].params.requested_items, 0);
});

test("rejected startup emits no started notice and execution failure preserves the error", async t => {
  const { frames, request, runtime } = await fixture(t);
  const rejected = await handleRequest(request({ action: "run_once", platform: "invalid" }), runtime);
  assert.equal(rejected.status, "error");
  assert.equal(frames.length, 0);
  const failed = await handleRequest(request({ action: "run_once", platform: "douyin" }), {
    ...runtime, collectPlatform: async () => { throw new Error("browser_missing"); } });
  assert.equal(frames.length, 1);
  assert.equal(failed.status, "error");
  assert.equal(failed.extra.error_code, "browser_missing");
});

test("start and periodic heartbeats share one sequence without repeating start", t => {
  let now = 0;
  const frames = [];
  const reporter = createBackgroundProgressReporter({ requestId: "sequence", run: { run_id: "run", platforms: ["douyin"] },
    counts: {}, writeFrame: frame => frames.push(frame), now: () => now });
  t.after(() => reporter.stop());
  assert.equal(reporter.start(), true);
  assert.equal(reporter.start(), false);
  now += backgroundReportIntervalMs;
  reporter.emitIfDue();
  assert.deepEqual(frames.map(frame => frame.sequence), [1, 2]);
  assert.deepEqual(frames.map(frame => frame.kind), ["progress", "heartbeat"]);
});
