import assert from "node:assert/strict";
import fs from "node:fs/promises";
import test from "node:test";

import { chromium } from "playwright";

import { captureEngagementMetrics } from "../src/browser.mjs";

const RUN_BROWSER_TEST = process.env.MEDIA_DISCOVERY_BROWSER_TEST === "1";

async function browserExecutable() {
  const configured = process.env.MEDIA_DISCOVERY_CHROME_BIN?.trim();
  const candidates = configured
    ? [configured]
    : process.platform === "darwin"
      ? [
          "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
          "/Applications/Chromium.app/Contents/MacOS/Chromium",
          "/opt/homebrew/bin/chromium",
          "/usr/local/bin/chromium",
        ]
      : [
          "/usr/bin/chromium",
          "/usr/bin/chromium-browser",
          "/usr/bin/google-chrome",
          "/usr/bin/google-chrome-stable",
          "/snap/bin/chromium",
        ];
  for (const candidate of candidates) {
    if (await fs.access(candidate).then(() => true).catch(() => false)) return candidate;
  }
  throw new Error("browser executable is required for the explicit engagement integration test");
}

async function withPage(t, content) {
  const browser = await chromium.launch({ executablePath: await browserExecutable(), headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  await page.setContent(content);
  return page;
}

test("captures Douyin machine-identified engagement without localized phrase matching", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, `
    <main data-aweme-id="123">
      <span data-e2e="video-play-count" data-count="12004">12,004</span>
      <span data-e2e="video-player-digg">1.2万</span>
      <span data-e2e="feed-comment-icon">318</span>
      <span data-e2e="video-player-collect">46</span>
      <span data-e2e="video-player-share">12</span>
      <p>likes 999999 and shares 888888 are ordinary post text</p>
    </main>
  `);
  const capturedAt = "2026-09-07T01:02:03.000Z";
  const engagement = await captureEngagementMetrics(page.locator("main"), "douyin", capturedAt);

  assert.equal(engagement.captured_at, capturedAt);
  assert.deepEqual(engagement.metrics.views, { display: "12004", value: 12004 });
  assert.deepEqual(engagement.metrics.likes, { display: "1.2万" });
  assert.deepEqual(engagement.metrics.comments, { display: "318", value: 318 });
  assert.deepEqual(engagement.metrics.favorites, { display: "46", value: 46 });
  assert.deepEqual(engagement.metrics.shares, { display: "12", value: 12 });
});

test("captures only the engagement controls exposed by Xiaohongshu", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, `
    <main>
      <div class="like-wrapper"><span class="count">86</span></div>
      <div class="collect-wrapper"><span class="count">2,401</span></div>
      <div class="share-wrapper"><span class="count">unavailable</span></div>
    </main>
  `);
  const engagement = await captureEngagementMetrics(
    page.locator("main"),
    "xiaohongshu",
    "2026-09-07T01:02:03.000Z",
  );

  assert.deepEqual(engagement.metrics.likes, { display: "86", value: 86 });
  assert.deepEqual(engagement.metrics.favorites, { display: "2,401", value: 2401 });
  assert.equal(engagement.metrics.shares, undefined);
  assert.equal(engagement.metrics.comments, undefined);
  assert.equal(engagement.metrics.views, undefined);
});

test("captures Kuaishou likes from the card's structural control", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, `
    <article class="video-card">
      <div class="video-info-content"><i class="like-icon"></i><span class="info-text">12,004</span></div>
      <p>shares 999999 is ordinary post text</p>
    </article>
  `);
  const engagement = await captureEngagementMetrics(
    page.locator("article"),
    "kuaishou",
    "2026-09-07T01:02:03.000Z",
  );

  assert.deepEqual(engagement.metrics.likes, { display: "12,004", value: 12004 });
  assert.equal(engagement.metrics.shares, undefined);
});

test("keeps available comment, favorite and share counts, including zero, without hidden or comment controls", {
  skip: !RUN_BROWSER_TEST,
}, async t => {
  const page = await withPage(t, `<article>
    <span data-testid="comment-count" style="display:none">9999</span>
    <div data-comment-id="123"><span data-testid="comment-count">9998</span></div>
    <div class="interactive-item comment-item"><span class="item-count">24</span></div>
    <div class="interactive-item collect-item"><span class="item-count">0</span></div>
    <div class="interactive-item share-item"><span class="item-count">17</span></div>
    <div data-testid="play-count">--</div>
  </article>`);
  const result = await captureEngagementMetrics(page, "kuaishou", "2026-09-09T00:00:00Z");
  assert.deepEqual(result.metrics, {
    comments: { display: "24", value: 24 },
    favorites: { display: "0", value: 0 },
    shares: { display: "17", value: 17 },
  });
  await page.setContent('<span data-testid="comment-count">24</span><span data-testid="collect-count">0</span><span data-testid="share-count">17</span>');
  const xhs = await captureEngagementMetrics(page, "xiaohongshu", "2026-09-09T00:00:00Z");
  assert.deepEqual(xhs.metrics, result.metrics);
});
