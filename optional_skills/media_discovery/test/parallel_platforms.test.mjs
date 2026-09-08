import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { handleRequest, normalizedConfig } from "../src/main.mjs";
import { beginRun, configurePlatforms, readState, readRecords, heartbeat, requestStop,
  finishRun, commitPageRecords, beginBackgroundWorker, finishBackgroundWorker } from "../src/storage.mjs";
import { parallelPlatformLimit } from "../src/run_leases.mjs";

async function fixture(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "media-discovery-parallel-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const context = { skill_storage: { storage_kind: "directory", directory_path: root } };
  return { root, call: (args, runtime) => handleRequest({ request_id: "parallel-test", args, context }, runtime) };
}

function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}

async function eventually(check) {
  const deadline = Date.now() + 5000;
  while (Date.now() < deadline) {
    if (await check()) return;
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  assert.fail("condition did not become true");
}

const record = (platform, suffix = "1", kind = "video") => ({
  platform, kind, dedup_key: `${platform}:${suffix}`, image_sequence: 1,
  title: "fixture", platform_text: "caption", discovered_at: "2026-09-09T00:00:00Z",
});

test("different one-shot platforms overlap; duplicate platform is rejected and stop is local", { timeout: 10000 }, async t => {
  const { root, call } = await fixture(t);
  const gate = deferred();
  const started = new Set();
  const stopped = {};
  const runtime = { parallelLimit: 3, collectPlatform: async ({ platform, onPage, shouldStop }) => {
    started.add(platform);
    await gate.promise;
    stopped[platform] = await shouldStop();
    await onPage({ records: [record(platform, "image1", "image"), record(platform, "image2", "image")], temporaryPaths: [] });
  } };
  const jobs = ["douyin", "xiaohongshu"].map(platform => call({ action: "run_once", platform }, runtime));
  try {
    await eventually(() => started.size === 2);
    const state = await readState(root);
    assert.equal(Object.keys(state.active_runs).length, 2);
    assert.deepEqual([...state.active_run.platforms].sort(), ["douyin", "xiaohongshu"]);
    const duplicate = await call({ action: "run_once", platform: "douyin" }, runtime);
    assert.equal(duplicate.extra.error_code, "run_already_active");
    const stop = await call({ action: "stop_current", platform: "douyin" });
    assert.equal(stop.extra.drain_run_ids.length, 1);
    const unrelated = await call({ action: "stop_current", platform: "kuaishou" });
    assert.equal(unrelated.extra.side_effect_applied, false);
  } finally { gate.resolve(); }
  const results = await Promise.all(jobs);
  assert.deepEqual(stopped, { douyin: true, xiaohongshu: false });
  assert.deepEqual(results.map(result => result.extra.state), ["stopped_after_current_item", "completed_batch"]);
  const records = await readRecords(root);
  assert.deepEqual(records.map(item => item.global_sequence), [1, 2, 3, 4]);
  for (const platform of started) assert.equal(new Set(records.filter(item => item.platform === platform).map(item => item.post_sequence)).size, 1);
  assert.equal((await readState(root)).active_run, null);
});

test("multi-platform one-shot uses independent quotas and awaits every partial result", { timeout: 10000 }, async t => {
  const { call } = await fixture(t);
  const gate = deferred();
  const started = new Set();
  const job = call({ action: "run_once", platforms: ["douyin", "xiaohongshu", "kuaishou"], max_items_per_run: 2 }, {
    parallelLimit: 3,
    collectPlatform: async ({ platform, limit, onPage }) => {
      assert.equal(limit, 2);
      started.add(platform);
      await gate.promise;
      if (platform === "douyin") throw new Error("selector_drift");
      for (const suffix of ["1", "2"]) await onPage({ records: [record(platform, suffix)], temporaryPaths: [] });
    },
  });
  try { await eventually(() => started.size === 3); } finally { gate.resolve(); }
  const result = await job;
  assert.equal(result.status, "error");
  assert.equal(result.extra.error_code, "partial_collection_failed");
  assert.equal(result.extra.runs.length, 3);
  assert.equal(result.extra.counts.items, 4);
  assert.equal(result.extra.side_effect_applied, true);
});

test("manual login waits on one platform while another completes", { timeout: 10000 }, async t => {
  const { call, root } = await fixture(t);
  await call({ action: "enable", platforms: ["douyin", "xiaohongshu"], max_items_per_run: 1, confirm: true });
  const login = deferred();
  let waiting = false, peerFinished = false, attempts = 0;
  const job = call({ action: "run_enabled_once" }, {
    parallelLimit: 2, maxContinuousCycles: 2,
    waitForInteractiveLogin: async () => { waiting = true; await login.promise; return { ready: true }; },
    collectPlatform: async ({ platform, onPage }) => {
      if (platform === "douyin" && attempts++ === 0) throw new Error("login_required");
      await onPage({ records: [record(platform)], temporaryPaths: [] });
      if (platform === "xiaohongshu") peerFinished = true;
    },
  });
  try {
    await eventually(() => waiting && peerFinished);
    assert.equal((await readRecords(root)).length, 1);
  } finally { login.resolve(); }
  const result = await job;
  assert.equal(result.status, "ok");
  assert.equal(result.extra.background_worker.counts.items, 2);
});

test("enable joins a live coordinator and does not open another worker", { timeout: 10000 }, async t => {
  const { call } = await fixture(t);
  await call({ action: "enable", platform: "douyin", confirm: true });
  const hold = deferred();
  const started = new Set();
  const job = call({ action: "run_enabled_once" }, {
    parallelLimit: 2, maxContinuousCycles: 2,
    collectPlatform: async ({ platform, onPage }) => {
      started.add(platform);
      if (platform === "douyin") await hold.promise;
      await onPage({ records: [record(platform)], temporaryPaths: [] });
    },
  });
  try {
    await eventually(() => started.has("douyin"));
    const enabled = await call({ action: "enable", platform: "xiaohongshu", confirm: true });
    assert.equal(enabled.status, "ok");
    assert.equal(enabled.extra.background_worker_active, true);
    assert.equal(enabled.extra.background_start_spec.capability, "media_discovery.run_enabled_once");
    const joined = await call({ action: "run_enabled_once" });
    assert.equal(joined.extra.state, "already_running");
    assert.equal(joined.extra.background_worker.worker_id, enabled.extra.background_worker.worker_id);
    await eventually(() => started.has("xiaohongshu"));
    const duplicate = await call({ action: "enable", platform: "douyin", confirm: true });
    assert.equal(duplicate.status, "error");
  } finally { hold.resolve(); }
  assert.equal((await job).extra.background_worker.counts.items, 2);
});

test("capacity is enforced across callers, and released by graceful completion", async t => {
  const { root } = await fixture(t);
  await configurePlatforms(root, ["douyin", "xiaohongshu"], {
    douyin: normalizedConfig({}), xiaohongshu: normalizedConfig({}),
  });
  const options = { parallel_limit: 1 };
  const { run } = await beginRun(root, ["douyin"], options);
  await assert.rejects(beginRun(root, ["xiaohongshu"], options), /collection_capacity_busy/);
  await finishRun(root, run, "completed_batch");
  assert.ok((await beginRun(root, ["xiaohongshu"], options)).run);
});

test("concurrent commits preserve unique sequence, deduplication, and complete CSV rows", async t => {
  const { root } = await fixture(t);
  const proposals = Array.from({ length: 30 }, (_, index) => [record(index % 2 ? "douyin" : "xiaohongshu", String(index))]);
  await Promise.all([...proposals, ...proposals].map(records => commitPageRecords(root, records)));
  const records = await readRecords(root);
  assert.equal(records.length, 30);
  assert.deepEqual(records.map(item => item.global_sequence), Array.from({ length: 30 }, (_, index) => index + 1));
  assert.equal((await fs.readFile(path.join(root, "exports", "videos.csv"), "utf8")).trim().split("\n").length, 31);
});

test("lost leases stop their browser and cannot commit into a replacement run", async t => {
  const { root } = await fixture(t);
  await configurePlatforms(root, ["douyin"], { douyin: normalizedConfig({}) });
  const worker = await beginBackgroundWorker(root);
  const { run } = await beginRun(root, ["douyin"], { worker_id: worker.worker_id });
  await finishBackgroundWorker(root, worker.worker_id, "stopped");
  assert.equal(await heartbeat(root, run.run_id, {}), true);
  await assert.rejects(commitPageRecords(root, [record("douyin")], run.run_id), /run_lease_lost/);
  await assert.rejects(beginRun(root, ["douyin"], { worker_id: worker.worker_id }), /worker_lease_lost/);
  assert.equal((await readRecords(root)).length, 0);
});

test("a stale worker cannot be migrated while it is still heartbeating", async t => {
  const { root } = await fixture(t);
  const legacy = { schema_version: 1, platforms: {}, active_run: {
    run_id: "old", platforms: ["douyin"], heartbeat_at: new Date().toISOString(),
  }, background_worker: null, runs: [] };
  await fs.writeFile(path.join(root, "state.json"), JSON.stringify(legacy));
  await assert.rejects(readState(root), /storage_upgrade_requires_idle/);
  assert.deepEqual(JSON.parse(await fs.readFile(path.join(root, "state.json"), "utf8")), legacy);
});

test("fatal platform failure does not cancel another platform's current post", { timeout: 10000 }, async t => {
  const { call } = await fixture(t);
  await call({ action: "enable", platforms: ["douyin", "xiaohongshu"], confirm: true });
  const result = await call({ action: "run_enabled_once" }, {
    parallelLimit: 2, maxContinuousCycles: 2,
    collectPlatform: async ({ platform, onPage, shouldStop }) => {
      if (platform === "douyin") throw new Error("source_url_invalid");
      await new Promise(resolve => setTimeout(resolve, 100));
      assert.equal(await shouldStop(), false);
      await onPage({ records: [record(platform)], temporaryPaths: [] });
    },
  });
  assert.equal(result.status, "error");
  assert.equal(result.extra.platform_outcomes.douyin.lifecycle_state, "failed");
  assert.equal(result.extra.platform_outcomes.xiaohongshu.counts.items, 1);
});

test("manual login retry only consumes the remaining per-platform quota", async t => {
  const { call } = await fixture(t);
  const limits = [];
  const result = await call({ action: "run_once", platform: "douyin", max_items_per_run: 2 }, {
    waitForInteractiveLogin: async () => ({ ready: true }),
    collectPlatform: async ({ platform, limit, onPage }) => {
      limits.push(limit);
      await onPage({ records: [record(platform, String(limits.length))], temporaryPaths: [] });
      if (limits.length === 1) throw new Error("login_required");
    },
  });
  assert.deepEqual(limits, [2, 1]);
  assert.equal(result.extra.run.counts.items, 2);
});

test("memory limits respect both host and process constraints", t => {
  let hostGiB = 16, constrainedGiB = 0;
  t.mock.method(os, "totalmem", () => hostGiB * 1024 ** 3);
  if (typeof process.constrainedMemory === "function") {
    t.mock.method(process, "constrainedMemory", () => constrainedGiB * 1024 ** 3);
    constrainedGiB = 2;
    assert.equal(parallelPlatformLimit(), 1);
    constrainedGiB = 4;
    assert.equal(parallelPlatformLimit(), 2);
    constrainedGiB = 0;
  }
  for (const [size, expected] of [[2, 1], [4, 2], [8, 3], [16, 3]]) {
    hostGiB = size;
    assert.equal(parallelPlatformLimit(), expected);
  }
});

test("one-shot queues every platform when memory capacity is one", async t => {
  const { call } = await fixture(t);
  let active = 0, peak = 0;
  const result = await call({ action: "run_once", platforms: ["douyin", "xiaohongshu", "kuaishou"] }, {
    parallelLimit: 1,
    collectPlatform: async ({ platform, onPage }) => {
      active += 1;
      peak = Math.max(peak, active);
      try { await onPage({ records: [record(platform)], temporaryPaths: [] }); }
      finally { active -= 1; }
    },
  });
  assert.equal(result.status, "ok");
  assert.equal(result.extra.runs.length, 3);
  assert.equal(result.extra.counts.items, 3);
  assert.equal(peak, 1);
});

test("disabling one platform drains only its batch and keeps the coordinator alive", { timeout: 10000 }, async t => {
  const { call, root } = await fixture(t);
  await call({ action: "enable", platforms: ["douyin", "xiaohongshu"], confirm: true });
  const holds = { douyin: deferred(), xiaohongshu: deferred() };
  const started = new Set();
  const worker = call({ action: "run_enabled_once" }, {
    parallelLimit: 2,
    collectPlatform: async ({ platform, onPage, shouldStop }) => {
      started.add(platform);
      await holds[platform].promise;
      assert.equal(await shouldStop(), true);
      await onPage({ records: [record(platform)], temporaryPaths: [] });
    },
  });
  try {
    await eventually(() => started.size === 2);
    const stopped = await call({ action: "disable", platform: "douyin" });
    assert.equal(stopped.extra.drain_run_ids.length, 1);
    holds.douyin.resolve();
    await eventually(async () => (await readRecords(root)).length === 1);
    const state = await readState(root);
    assert.ok(state.background_worker);
    assert.equal(state.platforms.xiaohongshu.enabled, true);
    assert.equal(Object.values(state.active_runs).find(run => run.platforms.includes("xiaohongshu")).stop_requested_at, undefined);
  } finally {
    await call({ action: "disable" });
    holds.douyin.resolve();
    holds.xiaohongshu.resolve();
  }
  assert.equal((await worker).status, "ok");
  const final = await readState(root);
  assert.equal(final.background_worker, null);
  assert.equal(Object.keys(final.active_runs).length, 0);
  assert.equal((await readRecords(root)).length, 2);
});

test("an idle coordinator cannot exit after another platform is enabled", async t => {
  const { call, root } = await fixture(t);
  await call({ action: "enable", platform: "douyin", confirm: true });
  const worker = await beginBackgroundWorker(root);
  await call({ action: "disable", platform: "douyin" });
  await call({ action: "enable", platform: "xiaohongshu", confirm: true });
  assert.equal(await finishBackgroundWorker(root, worker.worker_id, "stopped", {}, { only_when_disabled: true }), null);
  assert.equal((await readState(root)).background_worker.worker_id, worker.worker_id);
  await call({ action: "disable", platform: "xiaohongshu" });
  assert.ok(await finishBackgroundWorker(root, worker.worker_id, "stopped", {}, { only_when_disabled: true }));
});
