import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { ENGAGEMENT_METRICS, evaluateEngagementFilters } from "../src/engagement.mjs";
import { handleRequest } from "../src/main.mjs";
import { parseSkillRequest } from "../src/request.mjs";
import { resultPage } from "../src/results.mjs";
import { readRecords } from "../src/storage.mjs";

const compare = (actual, operator, expected) => ({
  gt: actual > expected, gte: actual >= expected, lt: actual < expected,
  lte: actual <= expected, eq: actual === expected,
})[operator];
const post = (platform, id, value = 11) => ({
  platform, item_id: `${platform}:${id}`, dedup_key: `${platform}:${id}:video`, kind: "video",
  engagement: { platform, metrics: { likes: { display: String(value), value }, coins: { display: "20", value: 20 } } },
});
async function fixture(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "discovery-extended-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  return { root, user_id: 7, chat_id: 8, context: { skill_storage: { storage_kind: "directory", directory_path: root } } };
}

test("all exact metric operators preserve zero and safe-integer boundary comparisons", () => {
  const values = [0, 1, 99, 100, 101, 9999, 10000, Number.MAX_SAFE_INTEGER - 1, Number.MAX_SAFE_INTEGER];
  for (const metric of ENGAGEMENT_METRICS) for (const actual of values) for (const value of values) {
    for (const operator of ["gt", "gte", "lt", "lte", "eq"]) {
      const result = evaluateEngagementFilters({ platform: "weibo", metrics: { [metric]: { display: String(actual), value: actual } } },
        { all: [{ metric, operator, value }] });
      assert.equal(result.matched, compare(actual, operator, value), JSON.stringify({ metric, actual, operator, value }));
    }
  }
});

test("global and platform AND/OR groups agree with an independent deterministic truth table", () => {
  let seed = 0x281019;
  const random = maximum => {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    return seed % maximum;
  };
  const condition = () => ({ metric: ENGAGEMENT_METRICS[random(ENGAGEMENT_METRICS.length)],
    operator: ["gt", "gte", "lt", "lte", "eq"][random(5)], value: random(5) });
  const group = () => ({ all: Array.from({ length: random(4) }, condition), any: Array.from({ length: random(4) }, condition) });
  for (let trial = 0; trial < 2000; trial++) {
    const metrics = Object.fromEntries(ENGAGEMENT_METRICS.filter(() => random(3) !== 0).map(metric => [metric, { value: random(5) }]));
    const global = group(), local = group();
    const accepts = rule => metrics[rule.metric] != null && compare(metrics[rule.metric].value, rule.operator, rule.value);
    const satisfies = value => value.all.every(accepts) && (!value.any.length || value.any.some(accepts));
    const expected = satisfies(global) && satisfies(local);
    const actual = evaluateEngagementFilters({ platform: "weibo", metrics },
      { ...global, platforms: { weibo: local, toutiao: { all: [{ metric: "likes", operator: "gt", value: 99999 }] } } });
    assert.equal(actual.matched, expected, `trial ${trial}: ${JSON.stringify({ metrics, global, local })}`);
  }
});

test("gallery pagination groups posts, preserves platform identity and never omits or repeats a post", () => {
  const records = [], expected = [];
  for (const platform of ["weibo", "toutiao"]) for (let id = 0; id < 45; id++) {
    const count = 1 + id % 13;
    const record = post(platform, id);
    expected.push({ platform, item_id: record.item_id, record_count: count });
    for (let image = 0; image < count; image++) records.push({ ...record, kind: "image", global_sequence: records.length + 1 });
  }
  for (const limit of [1, 2, 7, 20, 50, 1000]) {
    const seen = [];
    let offset = 0;
    for (;;) {
      const page = resultPage(records, { offset, limit });
      assert.equal(page.result_page.total, expected.length);
      assert.equal(page.result_page.returned, page.results.length);
      seen.push(...page.results.map(({ platform, item_id, record_count }) => ({ platform, item_id, record_count })));
      if (!page.result_page.has_more) break;
      assert.ok(page.result_page.next_offset > offset);
      offset = page.result_page.next_offset;
    }
    assert.deepEqual(seen, expected, `limit=${limit}`);
  }
  assert.deepEqual(resultPage(records, { offset: 1000 }).results, []);
});

test("interleaved adjacent i64 owners can page only their own runs and reads do not mutate storage", async t => {
  const request = await fixture(t);
  const owners = ["9223372036854775806", "9223372036854775807"];
  const runs = [];
  for (let batch = 0; batch < 4; batch++) {
    const owner = owners[batch % owners.length];
    const actor = parseSkillRequest(`{"user_id":${owner},"chat_id":8}`);
    const result = await handleRequest({ ...request, ...actor, args: { action: "run_once", platform: "weibo", max_items_per_run: 3 } }, {
      collectPlatform: async ({ onPage }) => {
        for (let n = 0; n < 3; n++) await onPage({ records: [post("weibo", `${batch}-${n}`, batch * 10 + n)] });
        return { stop_reason: "target_reached" };
      },
    });
    assert.equal(result.status, "ok", JSON.stringify(result));
    runs.push({ owner, run_id: result.extra.run.run_id, batch });
  }
  const snapshot = async () => {
    const files = ["state.json"];
    for (const directory of ["records", "exports"]) {
      const entries = await fs.readdir(path.join(request.root, directory), { withFileTypes: true });
      files.push(...entries.filter(entry => entry.isFile()).map(entry => `${directory}/${entry.name}`));
    }
    return Object.fromEntries(await Promise.all(files.sort().map(async file => [file, await fs.readFile(path.join(request.root, file))])));
  };
  const before = await snapshot();
  for (const { owner, run_id, batch } of runs) for (const reader of owners) {
    const result = await handleRequest({ ...request, user_id: reader, args: { action: "get_run_results", run_id, limit: 1, offset: 1 } });
    if (owner === reader) {
      assert.equal(result.status, "ok");
      assert.equal(result.extra.results[0].item_id, `weibo:${batch}-1`);
      assert.equal(result.extra.results[0].engagement.metrics.likes.value, batch * 10 + 1);
      assert.equal(result.extra.result_page.total, 3);
      assert.equal(result.extra.side_effect_applied, false);
    } else {
      assert.equal(result.extra.error_code, "result_run_not_found");
      assert.equal(result.extra.results, undefined);
    }
  }
  assert.deepEqual(await snapshot(), before);
  assert.equal((await readRecords(request.root)).length, 12);
});

test("an unsupported mandatory filter skips only that platform, not a supported platform", async t => {
  const request = await fixture(t);
  const calls = [];
  const result = await handleRequest({ ...request, args: { action: "run_once", platforms: ["weibo", "bilibili"],
    max_items_per_run: 1, engagement_filters: { all: [{ metric: "coins", operator: "gte", value: 10 }] } } }, {
    parallelLimit: 2,
    collectPlatform: async ({ platform, onPage }) => {
      calls.push(platform);
      await onPage({ records: [post(platform, 1)] });
      return { stop_reason: "target_reached" };
    },
  });
  assert.equal(result.status, "ok", JSON.stringify(result));
  assert.deepEqual(calls, ["bilibili"]);
  assert.equal(result.extra.runs.find(run => run.platforms.includes("weibo")).collection_outcome.stop_reason, "filter_not_evaluable");
  assert.deepEqual(result.extra.results.map(record => record.platform), ["bilibili"]);
  assert.equal(result.extra.results[0].engagement.metrics.coins.value, 20);
  assert.equal((await readRecords(request.root)).length, 1);
});

test("one platform failing after commit does not discard either platform's committed results", async t => {
  const request = await fixture(t);
  const calls = [];
  const result = await handleRequest({ ...request, args: { action: "run_once", platforms: ["weibo", "toutiao"], max_items_per_run: 1 } }, {
    parallelLimit: 2,
    collectPlatform: async ({ platform, onPage }) => {
      calls.push(platform);
      await onPage({ records: [post(platform, 1, platform === "weibo" ? 11 : 22)] });
      if (platform === "weibo") throw new Error("source_unavailable");
      return { stop_reason: "target_reached" };
    },
  });
  assert.equal(result.status, "error");
  assert.equal(result.extra.error_code, "partial_collection_failed");
  assert.deepEqual(calls.sort(), ["toutiao", "weibo"]);
  assert.equal(result.extra.results.length, 2, JSON.stringify(result));
  assert.deepEqual(Object.fromEntries(result.extra.model_observation.results.map(record => [record.platform, record.engagement.metrics.likes.value])),
    { weibo: 11, toutiao: 22 });
  assert.equal((await readRecords(request.root)).length, 2);
});
