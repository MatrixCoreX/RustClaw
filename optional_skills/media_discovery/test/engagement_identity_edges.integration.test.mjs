import assert from "node:assert/strict";
import test from "node:test";
import { chromium } from "playwright";
import { captureEngagementMetrics, observeEngagementResponses } from "../src/browser_engagement.mjs";

const enabled = process.env.MEDIA_DISCOVERY_BROWSER_TEST === "1";
const capturedAt = "2026-10-20T00:00:00Z";
async function browserPage(t, platform) {
  const browser = await chromium.launch({ executablePath: process.env.MEDIA_DISCOVERY_CHROME_BIN || "/usr/bin/google-chrome", headless: true });
  t.after(() => browser.close());
  const context = await browser.newContext();
  t.after(observeEngagementResponses(context, platform));
  return context.newPage();
}

test("Bilibili accepts matching aid-only state without requiring a bvid", { skip: !enabled }, async t => {
  const page = await browserPage(t, "bilibili");
  await page.setContent(`<script>window.__INITIAL_STATE__={videoData:{aid:12345,stat:{view:20000,coin:50}}};</script>`);
  const result = await captureEngagementMetrics(page, "bilibili", capturedAt, "bilibili:av12345");
  assert.equal(result.metrics.coins.value, 50);
  assert.equal(result.metrics.views.value, 20000);
});

test("Bilibili still accepts a matching bvid without aid, but rejects other identities", { skip: !enabled }, async t => {
  const page = await browserPage(t, "bilibili");
  await page.setContent(`<script>window.__INITIAL_STATE__={videoData:{bvid:"BV1234567890",stat:{view:20000,coin:50}}};</script>`);
  const result = await captureEngagementMetrics(page, "bilibili", capturedAt, "bilibili:BV1234567890");
  assert.equal(result.metrics.coins.value, 50);
  for (const requested of ["bilibili:BV0987654321", "bilibili:av12345"]) {
    assert.deepEqual((await captureEngagementMetrics(page, "bilibili", capturedAt, requested)).metrics, {});
  }
});

test("Bilibili rejects unrelated aid-only state instead of borrowing its counters", { skip: !enabled }, async t => {
  const page = await browserPage(t, "bilibili");
  await page.setContent(`<script>window.__INITIAL_STATE__={videoData:{aid:12345,stat:{view:20000,coin:50}}};</script>`);
  for (const requested of ["bilibili:av99999", "bilibili:BV1234567890"]) {
    const result = await captureEngagementMetrics(page, "bilibili", capturedAt, requested);
    assert.deepEqual(result.metrics, {}, requested);
    assert.equal(result.availability.coins.status, "not_observed");
  }
});

test("Bilibili does not trust counters when state and capture both lack a post identity", { skip: !enabled }, async t => {
  const page = await browserPage(t, "bilibili");
  await page.setContent(`<script>window.__INITIAL_STATE__={videoData:{stat:{view:20000,coin:50}}};</script>`);
  assert.deepEqual((await captureEngagementMetrics(page, "bilibili", capturedAt)).metrics, {});
});

async function weiboResponse(t, payload) {
  const page = await browserPage(t, "weibo");
  const id = "5351516240087575";
  await page.route("**/*", route => route.request().url().includes("/ajax/statuses/show")
    ? route.fulfill({ json: payload })
    : route.fulfill({ contentType: "text/html", body: "<article>Current post</article>" }));
  await page.goto(`https://weibo.com/detail/${id}`);
  await page.evaluate(id => fetch(`/ajax/statuses/show?id=${id}`).then(response => response.json()), id);
  return captureEngagementMetrics(page.locator("article"), "weibo", capturedAt, `weibo:${id}`);
}

test("Weibo preserves explicit missing counters only for a response with the matching post ID", { skip: !enabled }, async t => {
  const result = await weiboResponse(t, { idstr: "5351516240087575", attitudes_count: null, comments_count: 0 });
  assert.equal(result.availability.likes.status, "not_provided");
  assert.equal(result.availability.likes.retryable, false);
  assert.equal(result.metrics.comments.value, 0);
});

test("Weibo mismatched response cannot turn an unobserved metric into a definitive absent metric", { skip: !enabled }, async t => {
  const result = await weiboResponse(t, { idstr: "5351516240087576", attitudes_count: null, comments_count: 99999 });
  assert.deepEqual(result.metrics, {});
  assert.equal(result.availability.likes.status, "not_observed");
  assert.equal(result.availability.likes.retryable, true);
});

test("Weibo ignores a mismatched response without erasing a previous valid observation", { skip: !enabled }, async t => {
  const page = await browserPage(t, "weibo");
  const id = "5351516240087575";
  let payload = { idstr: id, attitudes_count: 41 };
  await page.route("**/*", route => route.request().url().includes("/ajax/statuses/show")
    ? route.fulfill({ json: payload })
    : route.fulfill({ contentType: "text/html", body: "<article>Current post</article>" }));
  await page.goto(`https://weibo.com/detail/${id}`);
  const capture = () => captureEngagementMetrics(page.locator("article"), "weibo", capturedAt, `weibo:${id}`);
  const fetchPost = () => page.evaluate(id => fetch(`/ajax/statuses/show?id=${id}`).then(response => response.json()), id);
  await fetchPost();
  assert.equal((await capture()).metrics.likes.value, 41);
  for (const invalid of [{ idstr: "5351516240087576", attitudes_count: null },
    { attitudes_count: null }, { idstr: "", attitudes_count: null }, null, []]) {
    payload = invalid;
    await fetchPost();
    const result = await capture();
    assert.equal(result.metrics.likes?.value, 41, JSON.stringify(payload));
    assert.equal(result.availability.likes.status, "available");
  }
});
