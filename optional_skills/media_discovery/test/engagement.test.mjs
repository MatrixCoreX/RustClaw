import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { evaluateEngagementFilters, metricFromDisplay, normalizeEngagementFilters } from "../src/engagement.mjs";
import { weiboPostMetrics } from "../src/browser_engagement.mjs";
import { handleRequest, normalizedConfig } from "../src/main.mjs";
import { readRecords } from "../src/storage.mjs";
import { publishCollectionPage } from "../src/collection_progress.mjs";

const condition = (metric, operator, value) => ({ metric, operator, value });
const snapshot = (platform, values) => ({ platform, metrics: Object.fromEntries(
  Object.entries(values).map(([name, value]) => [name, typeof value === "string" ? { display: value } : { display: String(value), value }]),
) });

test("strict boundaries, zero, and every supported counter use numeric comparisons", () => {
  for (const metric of ["views", "reads", "likes", "comments", "favorites", "shares", "reposts", "coins", "danmaku"]) {
    for (const [operator, value, matches] of [["gt", 100, false], ["gte", 100, true], ["lt", 100, false], ["lte", 100, true], ["eq", 100, true]]) {
      assert.equal(evaluateEngagementFilters(snapshot("weibo", { [metric]: 100 }), { all: [condition(metric, operator, value)] }).matched, matches);
    }
  }
  assert.equal(evaluateEngagementFilters(snapshot("weibo", { reposts: 0 }), { all: [condition("reposts", "eq", 0)] }).matched, true);
  assert.equal(evaluateEngagementFilters(snapshot("weibo", {}), { all: [condition("reposts", "eq", 0)] }).reason, "metric_unavailable");
});

test("global rules and each platform's own all/any thresholds remain independent", () => {
  const filters = normalizeEngagementFilters({ all: [condition("comments", "gte", 10)], platforms: {
    douyin: { all: [condition("likes", "gt", 10000)] },
    xiaohongshu: { all: [condition("favorites", "gte", 500)] },
    weibo: { all: [condition("reposts", "gt", 200)], any: [condition("likes", "gt", 1000), condition("reads", "gte", 20000)] },
  } });
  assert.equal(evaluateEngagementFilters(snapshot("douyin", { comments: 10, likes: 10001 }), filters).matched, true);
  assert.equal(evaluateEngagementFilters(snapshot("xiaohongshu", { comments: 10, favorites: 500, likes: 0 }), filters).matched, true);
  assert.equal(evaluateEngagementFilters(snapshot("weibo", { comments: 10, reposts: 201, likes: 0, reads: 20000 }), filters).matched, true);
  assert.equal(evaluateEngagementFilters(snapshot("weibo", { comments: 9, reposts: 201, likes: 1001 }), filters).matched, false);
  assert.equal(evaluateEngagementFilters(snapshot("weibo", { comments: 10, shares: 10000, likes: 1001 }), filters).reason, "metric_unavailable");
});

test("abbreviated displays never invent exact values or accept an ambiguous boundary", () => {
  for (const display of ["12千", "1.2萬", "1.2w"]) {
    assert.ok(metricFromDisplay(display));
    assert.equal(evaluateEngagementFilters(snapshot("douyin", { likes: display }), { all: [condition("likes", "gt", 10000)] }).matched, true);
    assert.equal(evaluateEngagementFilters(snapshot("douyin", { likes: display }), { all: [condition("likes", "eq", 12000)] }).matched, false);
  }
  assert.deepEqual(metricFromDisplay("1.2万"), { display: "1.2万" });
  assert.deepEqual(metricFromDisplay("12,004"), { display: "12,004", value: 12004 });
  assert.deepEqual(metricFromDisplay("12,004 views"), { display: "12,004 views", value: 12004 });
  assert.equal(evaluateEngagementFilters(snapshot("tiktok", { views: "12K views" }), { all: [condition("views", "gt", 10000)] }).matched, true);
  assert.equal(metricFromDisplay("follow 9999"), null);
  const data = snapshot("douyin", { likes: "1.2万" });
  assert.equal(evaluateEngagementFilters(data, { all: [condition("likes", "gt", 10000)] }).matched, true);
  assert.equal(evaluateEngagementFilters(data, { all: [condition("likes", "gt", 12000)] }).reason, "metric_precision_insufficient");
  assert.equal(evaluateEngagementFilters(data, { all: [condition("likes", "gte", 14000)] }).reason, "threshold_not_met");
  assert.equal(evaluateEngagementFilters(snapshot("tiktok", { views: "10K+" }), { all: [condition("views", "gte", 10000)] }).matched, true);
  assert.equal(evaluateEngagementFilters(snapshot("tiktok", { views: "10K+" }), { all: [condition("views", "lt", 20000)] }).matched, false);
});

test("invalid filters fail explicitly instead of weakening the user's conditions", () => {
  for (const filters of [null, [], { all: {} }, { all: [condition("unknown", "gt", 1)] },
    { all: [condition("likes", "gt", -1)] }, { all: [condition("likes", "gt", "1000")] },
    { all: [condition("likes", "contains", 1)] }, { platforms: { unknown: {} } }, { minimum_likes: 100 }]) {
    assert.throws(() => normalizeEngagementFilters(filters), /invalid_args/);
  }
});

test("all browser adapters preserve skip decisions so a rejected post never consumes the accepted quota", async () => {
  const skipped = { skipped: true, records: [] };
  assert.deepEqual(await publishCollectionPage(async () => {}, skipped), { skipped: true });
  assert.deepEqual(await publishCollectionPage(async () => ({ skipped: true }), { records: [{}] }), { skipped: true });
  assert.equal(await publishCollectionPage(async () => {}, { records: [{}] }), undefined);
});

test("preview explicitly exposes nested platform thresholds to the model observation contract", async () => {
  const filters = { platforms: { douyin: { all: [condition("likes", "gt", 10000)] },
    weibo: { all: [condition("reposts", "gte", 20)], any: [condition("comments", "gt", 100)] } } };
  const result = await handleRequest({ args: { action: "preview_enable", platforms: ["douyin", "weibo"],
    topics: ["财经"], source_mode: "topics", engagement_filters: filters } });
  assert.equal(result.status, "ok");
  assert.equal(result.extra.model_observation.side_effect_applied, false);
  for (const platform of ["douyin", "weibo"]) {
    assert.deepEqual(result.extra.model_observation.platform_configs[platform], result.extra.platform_configs[platform]);
    assert.deepEqual(result.extra.model_observation.platform_configs[platform].engagement_filters, filters);
  }
});

test("Weibo responses keep reads and reposts distinct and reject another post", () => {
  const data = { idstr: "5351516240087575", reads_count: 1178453, reposts_count: 62, comments_count: 56, attitudes_count: 1230, secret: "not-a-metric" };
  const metrics = weiboPostMetrics(data, data.idstr);
  assert.equal(metrics.reads.value, 1178453);
  assert.equal(metrics.reposts.value, 62);
  assert.equal(metrics.shares, undefined);
  assert.equal(metrics.views, undefined);
  assert.equal(metrics.secret, undefined);
  assert.deepEqual(weiboPostMetrics(data, "other"), {});
});

test("Weibo requires a nonempty matching identity before interpreting a response", () => {
  const id = "5351516240087575";
  for (const post of [null, [], {}, { idstr: "" }, { idstr: "other", mid: id }]) {
    assert.deepEqual(weiboPostMetrics(post, id), {});
  }
  for (const expected of [undefined, null, ""]) {
    assert.deepEqual(weiboPostMetrics({ attitudes_count: 99 }, expected), {});
  }
  for (const key of ["idstr", "mid", "id"]) {
    assert.equal(weiboPostMetrics({ [key]: id, attitudes_count: 0 }, id).likes.value, 0);
  }
});

test("rejected, unknown, and uncertain posts are not persisted or counted as matches", async t => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "media-discovery-filter-storage-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const filters = { all: [condition("likes", "gt", 12000)] };
  const request = { args: { action: "run_once", platform: "weibo", max_items_per_run: 2, engagement_filters: filters },
    context: { skill_storage: { storage_kind: "directory", directory_path: root } } };
  const result = await handleRequest(request, { collectPlatform: async ({ onPage, config }) => {
    assert.deepEqual(config.engagement_filters, filters);
    for (const [index, metrics] of [ { likes: 12000 }, {}, { likes: "1.2万" }, { likes: 12001, reposts: 33 }, { likes: 20000, reads: 45678 } ].entries()) {
      await onPage({ temporaryPaths: [], records: [{ kind: "video", platform: "weibo", item_id: `weibo:${index}`,
        dedup_key: `weibo:${index}:video`, video_page_url: `https://weibo.com/detail/${index}`,
        engagement: snapshot("weibo", metrics) }] });
    }
    return { stop_reason: "target_reached" };
  } });
  assert.equal(result.status, "ok");
  assert.equal(result.extra.run.counts.items, 2);
  assert.equal(result.extra.run.counts.examined, 5);
  assert.equal(result.extra.run.counts.filtered, 3);
  assert.equal(result.extra.run.counts.filter_unavailable, 1);
  assert.equal(result.extra.run.counts.filter_uncertain, 1);
  const records = await readRecords(root);
  assert.deepEqual(records.map(record => record.item_id), ["weibo:3", "weibo:4"]);
  assert.equal(records[0].engagement.metrics.reposts.value, 33);
  assert.equal(records[1].engagement.metrics.reads.value, 45678);
  const csv = await fs.readFile(path.join(root, "exports/videos.csv"), "utf8");
  assert.ok(csv.includes('"reads","reposts","coins","danmaku"'));
  assert.ok(csv.includes('"45678"'));
  assert.ok(!csv.includes('"1.2万"'));
});

test("continuous configuration persists platform-specific filters across restart", async t => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "media-discovery-filter-enable-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const filters = { platforms: { weibo: { all: [condition("reposts", "gte", 200)] } } };
  const context = { skill_storage: { storage_kind: "directory", directory_path: root } };
  const result = await handleRequest({ args: { action: "enable", platform: "weibo", confirm: true, engagement_filters: filters }, context });
  assert.equal(result.status, "ok");
  const state = JSON.parse(await fs.readFile(path.join(root, "state.json"), "utf8"));
  assert.deepEqual(state.platforms.weibo.config.engagement_filters, normalizedConfig({ engagement_filters: filters }).engagement_filters);
  const reopened = await handleRequest({ args: { action: "status" }, context });
  assert.deepEqual(reopened.extra.platforms.weibo.config.engagement_filters, filters);
});

test("a completed source with no qualifying posts is a valid empty result, never an invented match", async t => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "media-discovery-empty-filter-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const result = await handleRequest({ args: { action: "run_once", platform: "weibo",
    engagement_filters: { all: [condition("likes", "gt", 100)] } },
    context: { skill_storage: { storage_kind: "directory", directory_path: root } } }, {
    collectPlatform: async ({ onPage }) => {
      assert.deepEqual(await onPage({ records: [{ engagement: snapshot("weibo", { likes: 100 }) }] }), { skipped: true });
      return { handled: 0, stop_reason: "source_exhausted" };
    },
  });
  assert.equal(result.status, "ok");
  assert.equal(result.extra.run.counts.items, 0);
  assert.equal(result.extra.run.counts.filtered, 1);
  assert.equal(result.extra.run.counts.failures, 0);
  assert.deepEqual(await readRecords(root), []);
});
