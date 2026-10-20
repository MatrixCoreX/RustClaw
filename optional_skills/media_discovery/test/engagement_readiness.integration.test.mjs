import assert from "node:assert/strict";
import test from "node:test";
import { chromium } from "playwright";
import { captureEngagementMetrics, observeEngagementResponses } from "../src/browser_engagement.mjs";
const enabled = process.env.MEDIA_DISCOVERY_BROWSER_TEST === "1";
async function browserPage(t) {
  const browser = await chromium.launch({ executablePath: process.env.MEDIA_DISCOVERY_CHROME_BIN || "/usr/bin/google-chrome", headless: true });
  t.after(() => browser.close());
  const context = await browser.newContext();
  return { context, page: await context.newPage() };
}

test("wait for the current Weibo response, not recommendations, without reloading the page", { skip: !enabled }, async t => {
  const { context, page } = await browserPage(t);
  t.after(observeEngagementResponses(context, "weibo"));
  let navigations = 0;
  const id = "5351516240087575";
  await context.route("**/*", route => {
    const url = new URL(route.request().url());
    if (url.pathname === "/ajax/statuses/show") return route.fulfill({ json: { idstr: url.searchParams.get("id"), attitudes_count: url.searchParams.get("id") === id ? 1234 : 999999 } });
    navigations++;
    return route.fulfill({ contentType: "text/html", body: "<main>Post</main>" });
  });
  await page.goto(`https://weibo.com/detail/${id}`);
  const waitForResponse = page.waitForResponse.bind(page);
  t.mock.method(page, "waitForResponse", async (...args) => {
    const waiting = waitForResponse(...args);
    await page.evaluate(async id => {
      await fetch('/ajax/statuses/show?id=5351516240087576');
      await fetch(`/ajax/statuses/show?id=${id}`);
    }, id);
    return waiting;
  });
  const result = await captureEngagementMetrics(page, "weibo", new Date().toISOString(), `weibo:${id}`, {
    config: { engagement_filters: { all: [{ metric: "likes", operator: "gte", value: 100 }] } }, readinessTimeoutMs: 2000,
  });
  assert.equal(result.metrics.likes.value, 1234);
  assert.equal(result.observation.rechecked, true);
  assert.equal(result.availability.likes.status, "available");
  assert.equal(navigations, 1);
});

test("a late DOM counter can satisfy a filter after its source becomes visible", { skip: !enabled }, async t => {
  const { page } = await browserPage(t);
  await page.setContent("<main></main>");
  const scope = { page: () => page, locator: selector => {
    const locator = page.locator(selector);
    if (!selector.includes(",")) return locator;
    return { first: () => ({ waitFor: async options => {
      const waiting = locator.first().waitFor(options);
      await page.evaluate(() => {
        const el = document.createElement("strong");
        el.setAttribute("data-e2e", "like-count"); el.textContent = "1001";
        document.querySelector("main").append(el);
      });
      return waiting;
    } }) };
  } };
  const result = await captureEngagementMetrics(scope, "tiktok", new Date().toISOString(), "tiktok:123", {
    config: { engagement_filters: { all: [{ metric: "likes", operator: "gt", value: 1000 }] } }, readinessTimeoutMs: 2000,
  });
  assert.equal(result.metrics.likes.value, 1001);
  assert.equal(result.observation.rechecked, true);
});

test("an expired observation window means not observed, never zero or platform absence", { skip: !enabled }, async t => {
  const { page } = await browserPage(t);
  await page.setContent("<main></main>");
  const result = await captureEngagementMetrics(page, "tiktok", new Date().toISOString(), "tiktok:123", {
    config: { engagement_filters: { all: [{ metric: "likes", operator: "gte", value: 0 }] } }, readinessTimeoutMs: 50,
  });
  assert.equal(result.metrics.likes, undefined);
  assert.equal(result.availability.likes.status, "not_observed");
  assert.equal(result.observation.readiness, "not_observed_before_io_deadline");
});

test("an explicitly absent source value is distinguished from late loading and preserves a real zero", { skip: !enabled }, async t => {
  const { page } = await browserPage(t);
  await page.setContent(`<script>window.__UNIVERSAL_DATA_FOR_REHYDRATION__={__DEFAULT_SCOPE__:{'webapp.video-detail':{itemInfo:{itemStruct:{id:'123',stats:{diggCount:null,commentCount:0}}}}}};</script>`);
  const result = await captureEngagementMetrics(page, "tiktok", new Date().toISOString(), "tiktok:123", {
    config: { engagement_filters: { all: [{ metric: "likes", operator: "gte", value: 0 }] } }, readinessTimeoutMs: 50,
  });
  assert.equal(result.metrics.comments.value, 0);
  assert.equal(result.availability.likes.status, "not_provided");
  assert.equal(result.availability.likes.retryable, false);
  assert.equal(result.observation, undefined);
});
