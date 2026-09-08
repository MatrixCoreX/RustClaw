import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { chromium } from "playwright";

import {
  activeDouyinFeedEntry,
  advanceDouyinDetailFeed,
  openDouyinRecommendationDetail,
} from "../src/browser.mjs";
import { browserStageError, recordBrowserFailure } from "../src/browser_diagnostics.mjs";

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

test("client-launch cards open their HTTPS detail and advance through the player control", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const browser = await chromium.launch({ executablePath: await browserExecutable(), headless: true });
  t.after(() => browser.close());
  const context = await browser.newContext();
  let externalClicks = 0;
  await context.exposeFunction("recordExternalClick", () => { externalClicks += 1; });
  await context.route("https://www.douyin.com/**", async (route) => {
    const url = new URL(route.request().url());
    const detail = /^\/video\/\d+$/u.test(url.pathname);
    await route.fulfill({ contentType: "text/html", body: detail
      ? `<!doctype html><div data-e2e="video-detail"><video style="width:640px;height:360px"></video>
        <button data-e2e="video-switch-next-arrow" onclick="history.pushState({},'', '/video/222')">next</button></div>`
      : `<!doctype html><div data-aweme-id="111" style="height:500px;width:500px" onclick="recordExternalClick()">
        <a href="snssdk1128://aweme/detail/111">client</a>
        <a href="https://attacker.invalid/video/111">external</a></div>` });
  });
  const page = await context.newPage();
  await page.goto("https://www.douyin.com/jingxuan");
  const config = { pacing_min_delay_ms: 200, pacing_max_delay_ms: 200 };
  const opened = await openDouyinRecommendationDetail(page, config);
  assert.equal(opened.page, page);
  assert.equal(page.url(), "https://www.douyin.com/video/111");
  assert.equal(opened.entry.itemId, "111");
  assert.equal(externalClicks, 0);
  assert.equal((await advanceDouyinDetailFeed(page, "111", config)).itemId, "222");
});

test("a late verification page is classified during feed wait and stop cancels that wait", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const browser = await chromium.launch({ executablePath: await browserExecutable(), headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  await page.route("https://www.douyin.com/**", route => route.fulfill({ contentType: "text/html", body:
    new URL(route.request().url()).pathname === "/" ?
      `<!doctype html><body><script>setTimeout(()=>{const f=document.createElement('iframe');
      f.src='/verifycenter/captcha/v2';document.body.appendChild(f)},250)</script></body>` : "<!doctype html><body></body>" }));
  await page.goto("https://www.douyin.com/");
  await assert.rejects(openDouyinRecommendationDetail(page, { browser_mode: "silent" }),
    error => error.message === "challenge_required" && error.discovery_stage === "feed_ready");
  await assert.rejects(openDouyinRecommendationDetail(page, { browser_mode: "visible" }, async () => true),
    error => error.message === "collection_stopped");
});

test("failure diagnostics retain the stage and structural counts without page text or URL secrets", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "media-discovery-diagnostics-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const browser = await chromium.launch({ executablePath: await browserExecutable(), headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  await page.route("https://www.douyin.com/**", route => route.fulfill({ contentType: "text/html", body:
    '<div data-aweme-id="111">private-fixture-text</div><input type="password" value="private-fixture-password">' }));
  await page.goto("https://www.douyin.com/jingxuan?secret=private-fixture-secret");
  const error = browserStageError("selector_drift", "recommendation_ready");
  await recordBrowserFailure(page, { root, runId: "run_fixture", platform: "douyin", stage: "collect_items", error });
  const raw = await fs.readFile(path.join(root, "diagnostics/run_fixture/douyin.json"), "utf8");
  const saved = JSON.parse(raw);
  assert.equal(saved.stage, "recommendation_ready");
  assert.equal(saved.document.recommendation_cards, 1);
  assert.equal(saved.document.login_inputs, 1);
  assert.equal(saved.document.pathname, "/jingxuan");
  assert.equal(raw.includes("private-fixture"), false);
});
