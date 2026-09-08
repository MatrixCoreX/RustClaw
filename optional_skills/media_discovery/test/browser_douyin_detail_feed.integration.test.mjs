import assert from "node:assert/strict";
import fs from "node:fs/promises";
import test from "node:test";

import { chromium } from "playwright";

import {
  activeDouyinFeedEntry,
  advanceDouyinDetailFeed,
  openDouyinRecommendationDetail,
} from "../src/browser.mjs";

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
  throw new Error("browser executable is required for the explicit Douyin detail-feed integration test");
}

test("Douyin recommendations open one detail item and then advance item by item", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const browser = await chromium.launch({
    executablePath: await browserExecutable(),
    headless: true,
  });
  t.after(() => browser.close());
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  await context.route("https://www.douyin.com/**", async (route) => {
    const pathname = new URL(route.request().url()).pathname;
    if (pathname === "/video/111") {
      await route.fulfill({
        contentType: "text/html",
        body: `<!doctype html>
          <style>html,body{margin:0} article{height:900px;width:100vw} video{width:640px;height:640px}</style>
          <article data-aweme-id="111"><video></video></article>
          <article data-aweme-id="222"><video></video></article>`,
      });
      return;
    }
    await route.fulfill({
      contentType: "text/html",
      body: `<!doctype html>
        <style>html,body{margin:0} article{height:800px;width:100vw}</style>
        <article data-aweme-id="111"><a href="https://www.douyin.com/video/111">open</a></article>`,
    });
  });
  const page = await context.newPage();
  await page.goto("https://www.douyin.com/");

  const first = await activeDouyinFeedEntry(page);
  assert.equal(first?.itemId, "111");

  const opened = await openDouyinRecommendationDetail(page, {
    pacing_min_delay_ms: 200,
    pacing_max_delay_ms: 200,
  });
  assert.equal(new URL(opened.page.url()).pathname, "/video/111");
  assert.equal(opened.entry?.itemId, "111");

  const next = await advanceDouyinDetailFeed(opened.page, "111", {
    pacing_min_delay_ms: 200,
    pacing_max_delay_ms: 200,
  });
  assert.equal(next?.itemId, "222");
});
