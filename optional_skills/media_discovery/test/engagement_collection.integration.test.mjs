import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { chromium } from "playwright";
import { captureEngagementMetrics, collectPlatform } from "../src/browser.mjs";
import { observeEngagementResponses } from "../src/browser_engagement.mjs";
import { handleRequest } from "../src/main.mjs";
import { readRecords } from "../src/storage.mjs";

const enabled = process.env.MEDIA_DISCOVERY_BROWSER_TEST === "1";

test("Toutiao filters before preview capture, keeps searching, and persists only matching posts", { skip: !enabled }, async t => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "discovery-filter-browser-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const original = chromium.launchPersistentContext.bind(chromium);
  t.mock.method(chromium, "launchPersistentContext", async (profile, options) => {
    const context = await original(profile, options);
    await context.route("**/*", route => {
      if (!route.request().isNavigationRequest()) return route.abort();
      const id = new URL(route.request().url()).pathname.split("/").filter(Boolean).at(-1);
      const n = Number(id) - 7000000;
      const count = [100, 101, undefined, 102][n - 1];
      const data = { itemId: id, title: `Article ${n}`, content: `<p>Caption for article ${n}</p>`, publishTime: 1767225600,
        itemCell: { itemCounter: { diggCount: count, readCount: 30000 + n, commentCount: n, shareCount: 12, repinCount: 8, forwardCount: 2 } },
        detailFeedList: [{ itemId: "other", itemCounter: { diggCount: 99999999 } }] };
      return route.fulfill({ contentType: "text/html", body: `<style>article{width:500px;height:330px;background:#dce8ef;padding:30px}h1{color:#146c43}</style>
        <script id="RENDER_DATA">${encodeURIComponent(JSON.stringify({ data }))}</script>
        <article class="article-content"><h1>Article ${n}</h1><p>Caption ${n}</p></article>` });
    });
    return context;
  });
  const result = await handleRequest({ args: { action: "run_once", platform: "toutiao", source_mode: "seed_urls",
    seed_urls: [1, 2, 3, 4].map(n => `https://www.toutiao.com/article/${7000000 + n}/`), max_items_per_run: 2,
    browser_mode: "silent", pacing_min_delay_ms: 200, pacing_max_delay_ms: 200,
    engagement_filters: { platforms: { toutiao: { all: [{ metric: "likes", operator: "gt", value: 100 }] } } } },
    context: { skill_storage: { storage_kind: "directory", directory_path: root } } }, { collectPlatform });
  assert.equal(result.status, "ok", JSON.stringify(result));
  assert.equal(result.extra.run.counts.items, 2);
  assert.equal(result.extra.run.counts.filtered, 2);
  const records = await readRecords(root);
  assert.deepEqual(records.map(r => r.item_id), ["toutiao:7000002", "toutiao:7000004"]);
  assert.equal(records[0].engagement.metrics.reads.value, 30002);
  assert.equal(records[0].engagement.metrics.shares.value, 12);
  assert.equal(records[0].engagement.metrics.reposts.value, 2);
  assert.equal(records[0].engagement.metrics.views, undefined);
  const previews = await fs.readdir(path.join(root, "exports/images"));
  assert.equal(previews.length, 2);
  assert.ok(previews.every(name => !name.includes("7000001") && !name.includes("7000003")));
});

test("Weibo observes the exact browser post response and preserves separate reads, plays, and reposts", { skip: !enabled }, async t => {
  const browser = await chromium.launch({ executablePath: process.env.MEDIA_DISCOVERY_CHROME_BIN || "/usr/bin/google-chrome", headless: true });
  t.after(() => browser.close());
  const context = await browser.newContext();
  const stop = observeEngagementResponses(context, "weibo");
  t.after(stop);
  const page = await context.newPage();
  const id = "5351516240087575";
  await page.route("**/*", route => {
    if (route.request().url().includes("/ajax/statuses/show")) return route.fulfill({ json: {
      idstr: id, reads_count: 1178453, attitudes_count: 1230, comments_count: 56, reposts_count: 62,
      page_info: { media_info: { duration: 167, online_users: "104万次观看", online_users_number: 1047463 } },
      secret: "must-not-be-retained",
    } });
    return route.fulfill({ contentType: "text/html", body: `<article><footer><span class="woo-like-count">1.2千</span></footer></article>` });
  });
  await page.goto(`https://weibo.com/detail/${id}`);
  await page.evaluate(id => fetch(`/ajax/statuses/show?id=${id}`).then(r => r.json()), id);
  const engagement = await captureEngagementMetrics(page.locator("article"), "weibo", new Date().toISOString());
  assert.equal(engagement.metrics.reads.value, 1178453);
  assert.equal(engagement.metrics.views.value, 1047463);
  assert.equal(engagement.metrics.views.display, "104万次观看");
  assert.equal(engagement.metrics.likes.value, 1230);
  assert.equal(engagement.metrics.reposts.value, 62);
  assert.equal(engagement.metrics.shares, undefined);
  assert.ok(!JSON.stringify(engagement).includes("must-not-be-retained"));
  await page.goto("https://weibo.com/detail/5351516240087576");
  assert.equal((await captureEngagementMetrics(page, "weibo", new Date().toISOString())).metrics.reads, undefined);
});

test("structured platform counters bind to the current post and cannot borrow recommendation metrics", { skip: !enabled }, async t => {
  const browser = await chromium.launch({ executablePath: process.env.MEDIA_DISCOVERY_CHROME_BIN || "/usr/bin/google-chrome", headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  await page.setContent(`<script>window.__INITIAL_STATE__={note:{noteDetailMap:{note123:{note:{noteId:"note123",interactInfo:{likedCount:"10001",commentCount:"35",collectedCount:"500"}}}}}};</script><main></main>`);
  const metrics = (await captureEngagementMetrics(page, "xiaohongshu", "2026-10-20T00:00:00Z", "xiaohongshu:note123")).metrics;
  assert.equal(metrics.likes.value, 10001);
  assert.equal(metrics.comments.value, 35);
  assert.equal(metrics.favorites.value, 500);
  assert.deepEqual((await captureEngagementMetrics(page, "xiaohongshu", "2026-10-20T00:00:00Z", "xiaohongshu:other")).metrics, {});
  await page.setContent(`<script>window.__INITIAL_STATE__={videoData:{bvid:"BV1test",aid:12345,stat:{view:20000,coin:50,danmaku:101}}};</script>`);
  assert.equal((await captureEngagementMetrics(page, "bilibili", "2026-10-20T00:00:00Z", "bilibili:av12345")).metrics.coins.value, 50);
  assert.deepEqual((await captureEngagementMetrics(page, "bilibili", "2026-10-20T00:00:00Z", "bilibili:av99999")).metrics, {});
});
