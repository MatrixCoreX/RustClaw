import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { chromium } from "playwright";
import { browserCapability, waitForPlatformFeed } from "../src/browser.mjs";
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
