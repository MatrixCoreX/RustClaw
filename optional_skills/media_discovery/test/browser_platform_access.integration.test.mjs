import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { chromium } from "playwright";
import { browserCapability, collectXiaohongshuHomeFeed, currentPlatformAccessError, screenshotLocator, waitForPlatformFeed } from "../src/browser.mjs";
import { recordBrowserFailure } from "../src/browser_diagnostics.mjs";

const enabled = process.env.MEDIA_DISCOVERY_BROWSER_TEST === "1";

async function browserPage(t, html, url) {
  const browser = await chromium.launch({ executablePath: (await browserCapability()).chromium_executable, headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  await page.route("**/*", route => route.fulfill({ contentType: "text/html", body: html }));
  await page.goto(url);
  return page;
}

test("late Xiaohongshu network restriction is not reported as selector drift", { skip: !enabled }, async t => {
  const page = await browserPage(t, `<script>setTimeout(()=>history.replaceState({},'',
    '/website-login/error?error_code=300012&secret=not-for-diagnostics'),100)</script>`,
  "https://www.xiaohongshu.com/explore");
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "media-discovery-access-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  let failure;
  await assert.rejects(waitForPlatformFeed(page, "xiaohongshu", { browser_mode: "silent" }, async () => false, 3000), error => {
    failure = error;
    return error.message === "network_access_restricted" && error.discovery_stage === "feed_ready";
  });
  const diagnostic = await recordBrowserFailure(page, { root, runId: "test", platform: "xiaohongshu", error: failure });
  assert.equal(diagnostic.document.platform_error_code, "300012");
  assert.equal(JSON.stringify(diagnostic).includes("not-for-diagnostics"), false);
});

test("feed readiness accepts delayed valid Kuaishou links and supports cancellation", { skip: !enabled }, async t => {
  const page = await browserPage(t, `<div class="video-card"><a href="/short-video/"></a></div>
    <script>setTimeout(()=>document.querySelector('a').href='/short-video/3x6up9yztvdsi7w',150)</script>`,
  "https://www.kuaishou.com/brilliant");
  await waitForPlatformFeed(page, "kuaishou", { browser_mode: "silent" }, async () => false, 3000);
  await assert.rejects(waitForPlatformFeed(page, "kuaishou", {}, async () => true), { message: "collection_stopped" });
});

test("missing feed DOM produces a bounded selector failure", { skip: !enabled }, async t => {
  const page = await browserPage(t, "<!doctype html><body></body>", "https://www.kuaishou.com/brilliant");
  await assert.rejects(waitForPlatformFeed(page, "kuaishou", {}, async () => false, 100), { message: "selector_drift" });
});

test("Xiaohongshu modal with text/number inputs is a login barrier, not a ready feed", { skip: !enabled }, async t => {
  const page = await browserPage(t, `<section class="note-item" data-note-id="fixture"></section>
    <div class="reds-modal reds-modal-open login-modal"><div class="login-container">
    <input type="text"><input type="number"></div></div>`, "https://www.xiaohongshu.com/explore");
  assert.equal(await currentPlatformAccessError(page, "xiaohongshu"), "login_required");
  await assert.rejects(waitForPlatformFeed(page, "xiaohongshu", {}, async () => false, 500), { message: "login_required" });
  await page.locator(".login-modal").evaluate(node => { node.style.display = "none"; });
  assert.equal(await currentPlatformAccessError(page, "xiaohongshu"), null);
});

for (const late of [false, true]) {
  test(`screenshot rejects a login modal ${late ? "during" : "before"} capture`, { skip: !enabled }, async t => {
    const page = await browserPage(t, `<div id="media" style="width:300px;height:300px;background:#126dad"></div>`,
      "https://www.xiaohongshu.com/explore");
    const root = await fs.mkdtemp(path.join(os.tmpdir(), "capture-barrier-"));
    t.after(() => fs.rm(root, { recursive: true, force: true }));
    const file = path.join(root, "image.png");
    const addModal = () => page.evaluate(() => {
      const modal = document.createElement("div");
      modal.className = "reds-modal reds-modal-open login-modal";
      modal.innerHTML = '<div class="login-container"><input type="text"></div>';
      modal.style.cssText = "position:fixed;inset:0;background:white;z-index:99";
      document.body.append(modal);
    });
    const locator = page.locator("#media");
    if (late) {
      const screenshot = locator.screenshot.bind(locator);
      locator.screenshot = async options => { await addModal(); return screenshot(options); };
    } else await addModal();
    await assert.rejects(screenshotLocator(locator, file, "xiaohongshu"), { message: "login_required" });
    assert.deepEqual(await fs.readdir(root), []);
  });
}

test("screenshot rejects partial unknown occlusion even with a clear center", { skip: !enabled }, async t => {
  const page = await browserPage(t, `<div id="media" style="width:300px;height:300px;background:#126dad"></div>
    <aside style="position:fixed;left:0;top:0;width:95px;height:95px;background:white;z-index:99"></aside>`,
    "https://www.xiaohongshu.com/explore");
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "capture-occlusion-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  await assert.rejects(screenshotLocator(page.locator("#media"), path.join(root, "image.png"), "xiaohongshu"),
    { message: "screenshot_obscured" });
  assert.deepEqual(await fs.readdir(root), []);
});

test("visible feed resumes after manual login and stops during verification", { skip: !enabled }, async t => {
  const page = await browserPage(t, `<section class="note-item" data-note-id="fixture"></section>
    <div class="login-modal reds-modal-open"><div class="login-container"><input type="text"></div></div>`,
    "https://www.xiaohongshu.com/explore");
  let stopChecks = 0;
  await assert.rejects(waitForPlatformFeed(page, "xiaohongshu", { browser_mode: "visible" },
    async () => ++stopChecks > 1, 3000), error =>
    error.message === "collection_stopped" && error.discovery_stage === "manual_verification");
  await page.evaluate(() => setTimeout(() => document.querySelector(".login-modal").remove(), 100));
  await waitForPlatformFeed(page, "xiaohongshu", { browser_mode: "visible" }, async () => false, 5000);
});

test("unobscured media screenshots persist as a PNG", { skip: !enabled }, async t => {
  const page = await browserPage(t, '<div id="media" style="width:300px;height:300px;background:#126dad"></div>',
    "https://www.xiaohongshu.com/explore");
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "capture-valid-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const file = path.join(root, "image.png");
  await screenshotLocator(page.locator("#media"), file, "xiaohongshu");
  const image = await fs.readFile(file);
  assert.equal(image.subarray(1, 4).toString(), "PNG");
  assert.ok(image.length >= 512);
  assert.deepEqual(await fs.readdir(root), ["image.png"]);
});

test("Xiaohongshu capture retains item identity when feed cards reorder", { skip: !enabled }, async t => {
  const image = `data:image/svg+xml;base64,${Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="300" height="300"><rect width="300" height="300" fill="#126dad"/></svg>').toString('base64')}`;
  const page = await browserPage(t, ["first", "second"].map(id =>
    `<section class="note-item" data-note-id="${id}" style="display:inline-block;width:300px">
    <img src="${image}" width="300" height="300"><span class="title">${id}</span></section>`).join(""),
    "https://www.xiaohongshu.com/explore");
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "capture-identity-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const locate = page.locator.bind(page);
  page.locator = (selector, ...args) => {
    const locator = locate(selector, ...args);
    if (selector === "section.note-item[data-note-id]") {
      const evaluateAll = locator.evaluateAll.bind(locator);
      locator.evaluateAll = async (...params) => {
        const result = await evaluateAll(...params);
        if (Array.isArray(result)) await page.evaluate(() => document.body.append(document.querySelector('[data-note-id="first"]')));
        return result;
      };
    }
    return locator;
  };
  const results = [];
  await collectXiaohongshuHomeFeed(page, root, "identity-fixture", { browser_mode: "silent",
    pacing_min_delay_ms: 1, pacing_max_delay_ms: 1 }, { source_mode: "home_feed", url: page.url() },
  1, async () => false, async result => results.push(result));
  assert.equal(results.length, 1);
  assert.equal(results[0].records[0].item_id, "xiaohongshu:first");
  assert.equal(results[0].records[0].title, "first");
});
