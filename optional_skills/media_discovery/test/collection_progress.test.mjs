import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { collectOrderedPages, completedDiscoveryPosts, optionalLimit } from "../src/collection_progress.mjs";
import { normalizedConfig, handleRequest } from "../src/main.mjs";
import { stopsCollection } from "../src/browser_flow_control.mjs";
import { beginRun, finishRun, configurePlatforms, readRecords } from "../src/storage.mjs";

const config = { source_mode: "topics", topics: ["财经"], platform: "douyin" };
const record = n => ({ kind: "video", platform: "douyin", item_id: `douyin:${n}`,
  dedup_key: `video-${n}`, source_mode: "topics", search_keyword: "财经",
  video_page_url: `https://www.douyin.com/video/${n}`, title: `post ${n}` });

async function storage(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "discovery-limits-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  return { root, context: { skill_storage: { storage_kind: "directory", directory_path: root } } };
}

function traversal(overrides = {}) {
  let page = 1;
  const collected = [];
  return { collected, args: {
    readCandidates: async () => [page], identity: value => value,
    collect: async value => collected.push(value), scroll: async () => { page += 1; },
    shouldStop: async () => false, limit: 300, stopsCollection,
    ...overrides,
  } };
}

test("large requested counts and optional budgets have no business ceiling; pacing stays bounded", () => {
  const normalized = normalizedConfig({ ...config, max_items_per_run: 300 });
  assert.equal(normalized.max_items_per_run, 300);
  for (const field of ["max_run_minutes", "max_scrolls_per_source", "max_images_per_post"]) {
    assert.equal(normalized[field], 0);
    assert.equal(normalizedConfig({ ...config, [field]: 100000 })[field], 100000);
  }
  assert.equal(optionalLimit(0), Infinity);
  assert.equal(optionalLimit(undefined), Infinity);
  assert.equal(normalizedConfig({ ...config, max_items_per_run: 0 }).max_items_per_run, 0);
  assert.equal(normalizedConfig({ ...config, max_run_minutes: 30 }).max_items_per_run, 0);
  for (const value of [-1, 1.2, Infinity, Number.MAX_SAFE_INTEGER + 1]) {
    assert.throws(() => normalizedConfig({ ...config, max_items_per_run: value }), /invalid_args/);
  }
  assert.throws(() => normalizedConfig({ ...config, pacing_min_delay_ms: 0 }), /invalid_args/);
  assert.throws(() => normalizedConfig({ ...config, rest_min_seconds: 0 }), /invalid_args/);
});

test("a healthy traversal crosses 100 scrolls and stops at exactly 300 posts in order", async () => {
  const { args, collected } = traversal();
  const result = await collectOrderedPages(args);
  assert.equal(result.handled, 300);
  assert.equal(result.scrolls, 299);
  assert.equal(result.stop_reason, "target_reached");
  assert.deepEqual(collected, Array.from({ length: 300 }, (_, i) => i + 1));
});

test("collect can skip an already-open identity without consuming the target", async () => {
  const collected = [];
  const result = await collectOrderedPages({
    readCandidates: async () => [1, 2, 3],
    identity: (value) => value,
    collect: async (value) => {
      if (value === 2) return { skipped: true };
      collected.push(value);
      return undefined;
    },
    scroll: async () => {},
    shouldStop: async () => false,
    limit: 10,
    maxScrolls: 0,
    stopsCollection,
  });
  assert.deepEqual(collected, [1, 3]);
  assert.equal(result.handled, 2);
  assert.equal(result.skipped, 1);
});

test("background replay skips saved identities and continues after the old first 100", async () => {
  const { args, collected } = traversal({ completed: new Set(Array.from({ length: 100 }, (_, i) => i + 1)) });
  const result = await collectOrderedPages(args);
  assert.equal(result.handled, 300);
  assert.equal(result.skipped, 100);
  assert.deepEqual(collected, Array.from({ length: 300 }, (_, i) => i + 101));
});

test("unchanged results stop a batch rather than spin forever", async () => {
  const { args } = traversal({ readCandidates: async () => [1, 2, 3] });
  const result = await collectOrderedPages(args);
  assert.equal(result.handled, 3);
  assert.equal(result.stop_reason, "no_new_results");
  assert.equal(result.scrolls, 3);
});

test("unavailable posts do not consume the target, access barriers stop without further browsing", async () => {
  let errors = 0;
  const { args } = traversal({ collect: async n => { if (n === 1) throw new Error("source_unavailable"); },
    onFailure: async () => { errors += 1; } });
  assert.equal((await collectOrderedPages(args)).handled, 300);
  assert.equal(errors, 1);
  let scrolls = 0;
  const blocked = traversal({ collect: async () => { throw new Error("rate_limited"); },
    scroll: async () => { scrolls += 1; } });
  await assert.rejects(collectOrderedPages(blocked.args), /rate_limited/);
  assert.equal(scrolls, 0);
});

test("cancellation and explicitly requested scroll budgets remain effective", async () => {
  let handled = 0;
  const cancelled = traversal({ collect: async () => { handled += 1; }, shouldStop: async () => handled >= 7 });
  assert.equal((await collectOrderedPages(cancelled.args)).stop_reason, "collection_stopped");
  assert.equal(handled, 7);
  const bounded = traversal({ maxScrolls: 2 });
  assert.deepEqual(await collectOrderedPages(bounded.args), {
    handled: 3, skipped: 0, failures: 0, scrolls: 2, stop_reason: "requested_scroll_limit",
  });
});

test("only complete posts for the current platform/search are continuation evidence", () => {
  const gallery = [1, 2].map(n => ({ ...record(2), kind: "image", image_url: `https://images.example/${n}` }));
  gallery[1].collection_truncated = true;
  assert.deepEqual([...completedDiscoveryPosts([record(1), ...gallery,
    { ...record(3), search_keyword: "other" }, { ...record(4), platform: "xiaohongshu" }], "douyin", config)], ["douyin:1"]);
});

test("one-shot 300 flows through normalization, storage, receipt, and ordered CSV", async t => {
  const { root, context } = await storage(t);
  const result = await handleRequest({ args: { ...config, action: "run_once", max_items_per_run: 300 }, context }, {
    collectPlatform: async ({ limit, config: actual, onPage, shouldStop }) => {
      assert.equal(limit, 300);
      assert.equal(actual.max_run_minutes, 0);
      assert.equal(await shouldStop(), false);
      for (let n = 1; n <= limit; n += 1) await onPage({ records: [record(n)], temporaryPaths: [] });
      return { handled: limit, stop_reason: "target_reached" };
    },
  });
  assert.equal(result.status, "ok", JSON.stringify(result));
  assert.equal(result.extra.run.counts.items, 300);
  assert.equal(result.extra.run.collection_outcome.stop_reason, "target_reached");
  assert.deepEqual((await readRecords(root)).map(r => r.item_id), Array.from({ length: 300 }, (_, i) => `douyin:${i + 1}`));
  assert.ok((await fs.readFile(path.join(root, "exports/videos.csv"), "utf8")).includes('post 300'));
});

test("background cycles reconstruct continuation from committed records", async t => {
  const { context } = await storage(t);
  await handleRequest({ args: { ...config, action: "enable", confirm: true, max_items_per_run: 2 }, context });
  let clock = Date.now();
  const collected = [];
  const result = await handleRequest({ args: { action: "run_enabled_once" }, context }, {
    maxContinuousCycles: 3, now: () => clock, sleep: async ms => { clock += ms + 500000; },
    collectPlatform: async ({ completedPosts, onPage, limit }) => {
      let handled = 0;
      for (let n = 1; n <= 6 && handled < limit; n += 1) {
        if (completedPosts.has(`douyin:${n}`)) continue;
        await onPage({ records: [record(n)], temporaryPaths: [] });
        handled += 1; collected.push(n);
      }
      return { handled, stop_reason: "target_reached" };
    },
  });
  assert.equal(result.status, "ok", JSON.stringify(result));
  assert.deepEqual(collected, [1, 2, 3, 4, 5, 6]);
});

test("run history retains more than 100 batches and supports larger result pages", async t => {
  const { root, context } = await storage(t);
  await configurePlatforms(root, ["douyin"], { douyin: normalizedConfig(config) });
  for (let n = 0; n < 105; n += 1) {
    const { run } = await beginRun(root, ["douyin"]);
    await finishRun(root, run, "completed_batch");
  }
  const result = await handleRequest({ args: { action: "list_runs", limit: 300 }, context });
  assert.equal(result.extra.total, 105);
  assert.equal(result.extra.runs.length, 105);
});
