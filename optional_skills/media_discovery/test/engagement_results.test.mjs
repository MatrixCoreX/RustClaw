import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { metricAvailability, requestedMetricSupport, recordFilterDiagnostics } from "../src/engagement_availability.mjs";
import { supportedEngagementMetrics } from "../src/browser_engagement.mjs";
import { evaluateEngagementFilters } from "../src/engagement.mjs";
import { handleRequest } from "../src/main.mjs";
import { readRecords } from "../src/storage.mjs";

const condition = (metric, value = 1) => ({ metric, operator: "gte", value });
const metrics = { likes: { display: "123", value: 123 }, shares: { display: "4", value: 4 }, reads: { display: "12345", value: 12345 } };
async function fixture(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "discovery-results-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  return { root, user_id: 7, chat_id: 8, context: { skill_storage: { storage_kind: "directory", directory_path: root } } };
}
const post = (platform, n, engagement = { platform, metrics }) => ({ kind: "video", platform,
  item_id: `${platform}:${n}`, dedup_key: `${platform}:${n}:video`, title: `Post ${n}`,
  platform_text: `Caption ${n}`, video_page_url: `https://example.invalid/${n}`, engagement });

test("missing data never implies platform-wide absence, and content applicability is explicit", () => {
  const data = metricAvailability("toutiao", { likes: { display: "0", value: 0 }, shares: { display: "1.2万" } },
    supportedEngagementMetrics("toutiao"), "article", { comments: "post_state:commentCount" });
  assert.equal(data.likes.status, "available");
  assert.equal(data.likes.precision, "exact");
  assert.equal(data.shares.precision, "approximate");
  assert.equal(data.views.status, "not_applicable");
  assert.equal(data.comments.status, "not_provided");
  assert.equal(data.comments.retryable, false);
  assert.equal(data.reads.status, "not_observed");
  assert.equal(data.reads.retryable, true);
  assert.equal(data.coins.status, "adapter_unsupported");
});

test("unsupported OR alternatives do not block a supported matching alternative", () => {
  const support = supportedEngagementMetrics("weibo");
  assert.equal(requestedMetricSupport({ any: [condition("coins"), condition("likes")] }, "weibo", support).evaluable, true);
  assert.equal(requestedMetricSupport({ all: [condition("coins"), condition("likes")] }, "weibo", support).evaluable, false);
  const engagement = { platform: "weibo", metrics: { likes: metrics.likes },
    availability: metricAvailability("weibo", { likes: metrics.likes }, support) };
  const decision = evaluateEngagementFilters(engagement, { any: [condition("coins"), condition("likes")] });
  assert.equal(decision.matched, true);
  const report = [];
  recordFilterDiagnostics(report, "weibo", decision);
  assert.deepEqual(report, [{ platform: "weibo", metric: "coins", status: "adapter_unsupported", posts: 1, retryable: false, blocks_match: false }]);
});

test("preview explains adapter coverage without changing the requested thresholds", async () => {
  const filters = { all: [condition("coins")] };
  const result = await handleRequest({ args: { action: "preview_enable", platform: "weibo", engagement_filters: filters } });
  assert.equal(result.status, "ok");
  assert.equal(result.extra.metric_support.weibo.evaluable, false);
  assert.equal(result.extra.metric_support.weibo.platform_public_availability, "not_inferred");
  assert.deepEqual(result.extra.model_observation.metric_support, result.extra.metric_support);
  assert.deepEqual(result.extra.config.engagement_filters, filters);
});

test("run replies contain saved values, and an owned read query paginates without a new browser", async t => {
  const request = await fixture(t);
  let browserCalls = 0;
  const result = await handleRequest({ ...request, args: { action: "run_once", platform: "toutiao", max_items_per_run: 35 } }, {
    collectPlatform: async ({ onPage }) => {
      browserCalls++;
      for (let n = 0; n < 35; n++) await onPage({ records: [post("toutiao", n)], temporaryPaths: [] });
      return { stop_reason: "target_reached" };
    },
  });
  assert.equal(result.status, "ok", JSON.stringify(result));
  assert.equal(result.extra.results.length, 20);
  assert.equal(result.extra.result_page.total, 35);
  assert.equal(result.extra.result_page.has_more, true);
  assert.equal(result.extra.model_observation.results[0].engagement.metrics.reads.value, 12345);
  assert.ok(!JSON.stringify(result.extra.model_observation).includes("exports/"));
  const query = result.extra.result_query;
  const tail = await handleRequest({ ...request, args: { action: "get_run_results", ...query.args } });
  assert.equal(tail.status, "ok");
  assert.equal(tail.extra.results.length, 15);
  assert.equal(tail.extra.result_page.has_more, false);
  assert.equal(tail.extra.side_effect_applied, false);
  assert.equal(tail.extra.collection_outcome.stop_reason, "target_reached");
  assert.equal(tail.extra.capture_summary.records_saved, 35);
  const all = await handleRequest({ ...request, args: { action: "get_run_results", run_id: result.extra.run.run_id, limit: 1000 } });
  assert.equal(all.extra.results.length, 35);
  assert.equal(browserCalls, 1);
  const records = await readRecords(request.root);
  assert.ok(records.every(record => record.owner_user_id === "7" && record.run_id === result.extra.run.run_id));
  for (const user_id of [9, null, Number.MAX_SAFE_INTEGER + 1]) {
    const denied = await handleRequest({ ...request, user_id, args: { action: "get_run_results", run_id: result.extra.run.run_id } });
    assert.equal(denied.status, "error");
    assert.ok(["result_run_not_found", "result_actor_required"].includes(denied.extra.error_code));
    assert.equal(denied.extra.results, undefined);
  }
  for (const args of [{ run_id: "../../state.json" }, { run_id: result.extra.run.run_id, limit: 0 }, { run_id: result.extra.run.run_id, offset: -1 }]) {
    assert.equal((await handleRequest({ ...request, args: { action: "get_run_results", ...args } })).extra.error_code, "invalid_args");
  }
});

test("partial browser failure retains already committed metric values in the model observation", async t => {
  const request = await fixture(t);
  const result = await handleRequest({ ...request, args: { action: "run_once", platform: "toutiao" } }, {
    collectPlatform: async ({ onPage }) => {
      await onPage({ records: [post("toutiao", 1)], temporaryPaths: [] });
      throw new Error("source_unavailable");
    },
  });
  assert.equal(result.status, "error");
  assert.equal(result.extra.model_observation.error_code, "source_unavailable");
  assert.equal(result.extra.results[0].engagement.metrics.likes.value, 123);
});

test("filtered posts retain aggregate reasons only, not rejected captions or metric values", async t => {
  const request = await fixture(t);
  const result = await handleRequest({ ...request, args: { action: "run_once", platform: "toutiao",
    engagement_filters: { all: [condition("views")] } } }, {
    collectPlatform: async ({ onPage }) => {
      const engagement = { platform: "toutiao", metrics,
        availability: metricAvailability("toutiao", metrics, supportedEngagementMetrics("toutiao"), "article") };
      await onPage({ records: [post("toutiao", 1, engagement)], temporaryPaths: [] });
      return { stop_reason: "source_exhausted" };
    },
  });
  assert.equal(result.status, "ok");
  assert.deepEqual(result.extra.filter_report, result.extra.run.filter_report);
  assert.equal(result.extra.model_observation.filter_report[0].status, "not_applicable");
  assert.equal(result.extra.model_observation.results.length, 0);
  assert.ok(!JSON.stringify(result.extra.run.filter_report).includes("Caption"));
  assert.equal((await readRecords(request.root)).length, 0);
});

test("unsupported mandatory background filters pause instead of repeatedly opening browsers", async t => {
  const request = await fixture(t);
  await handleRequest({ ...request, args: { action: "enable", confirm: true, platform: "weibo",
    engagement_filters: { all: [condition("coins")] } } });
  let browserCalls = 0;
  await handleRequest({ ...request, args: { action: "run_enabled_once" } }, {
    maxContinuousCycles: 1, sleep: async () => {}, collectPlatform: async () => { browserCalls++; },
  });
  assert.equal(browserCalls, 0);
  const state = JSON.parse(await fs.readFile(path.join(request.root, "state.json"), "utf8"));
  assert.equal(state.platforms.weibo.paused, true);
  assert.equal(state.runs[0].error_code, "engagement_filter_unavailable");
  assert.equal(state.runs[0].filter_report[0].status, "adapter_unsupported");
});

test("an impossible mandatory one-shot filter reports unsupported metrics without pointless browsing", async t => {
  const request = await fixture(t);
  const result = await handleRequest({ ...request, args: { action: "run_once", platform: "weibo",
    engagement_filters: { all: [condition("favorites", 0)] } } }, { collectPlatform: async () => {
    assert.fail("unsupported mandatory metrics must not launch a browser");
  } });
  assert.equal(result.status, "ok");
  assert.equal(result.extra.run.collection_outcome.stop_reason, "filter_not_evaluable");
  assert.equal(result.extra.run.counts.items, 0);
  assert.equal(result.extra.run.counts.examined, 0);
  assert.equal(result.extra.run.counts.filtered, 0);
  assert.equal(result.extra.model_observation.counts.filter_unavailable, 0);
  assert.deepEqual(result.extra.filter_report, [{ platform: "weibo", metric: "favorites", status: "adapter_unsupported", posts: 0, blocks_match: true, retryable: false }]);
});
