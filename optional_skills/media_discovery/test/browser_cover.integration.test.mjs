import assert from "node:assert/strict";
import fs from "node:fs/promises";
import test from "node:test";

import { chromium } from "playwright";

import { renderedVideoCover } from "../src/browser.mjs";

const RUN_BROWSER_TEST = process.env.MEDIA_DISCOVERY_BROWSER_TEST === "1";

async function browserExecutable() {
  const candidates = process.env.MEDIA_DISCOVERY_CHROME_BIN?.trim()
    ? [process.env.MEDIA_DISCOVERY_CHROME_BIN.trim()]
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
  throw new Error("browser executable is required for the explicit cover integration test");
}

async function withPage(t, content) {
  const browser = await chromium.launch({ executablePath: await browserExecutable(), headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ viewport: { width: 900, height: 700 } });
  await page.setContent(content);
  return page;
}

test("video cover selection prefers an unobscured rendered frame", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, '<main><video style="width:640px;height:360px;background:#222"></video></main>');
  const cover = await renderedVideoCover(page, "douyin");
  assert.equal(cover?.source, "rendered_video_frame");
  assert.equal(await cover?.locator.evaluate((node) => node.tagName), "VIDEO");
});

test("video cover selection refuses a media element hidden by an overlay", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, `
    <main style="position:relative;width:640px;height:360px">
      <video style="width:640px;height:360px;background:#222"></video>
      <section style="position:absolute;inset:0;z-index:2;background:white">overlay</section>
    </main>
  `);
  assert.equal(await renderedVideoCover(page.locator("main"), "douyin"), null);
});

test("platform poster controls provide the cover when no video frame is available", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, `
    <main data-e2e="video-cover" style="width:640px;height:360px">
      <img alt="" style="width:640px;height:360px" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='640' height='360'%3E%3Crect width='640' height='360' fill='navy'/%3E%3C/svg%3E">
    </main>
  `);
  const cover = await renderedVideoCover(page, "douyin");
  assert.equal(cover?.source, "rendered_poster_image");
});
